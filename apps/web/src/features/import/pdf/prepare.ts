/**
 * 逐页准备流水线（T09 / REQ-014；架构 §5.1、PRD §6.2 UI-014/UI-015/UI-017）。
 *
 * 每页顺序执行：提取文字 → 渲染页图（白底 JPEG）→ 上传两个资产 → `PUT` 页记录。
 * 硬约束（QA 按此复核）：
 * - **每次只渲染一页**：只有一张 canvas（不进 DOM），逐页复用；100 页不会同时存在
 *   100 张 canvas（AC-062 / PRD §5.5）。
 * - **页图规格**：长边 ≤2000px、白底 JPEG；`viewport` 用渲染时的实际尺寸与旋转角，
 *   坐标原点即旋转后 viewport 的左上角（不经 transform 伪造字符框）。
 * - **页文字为空（扫描页）**：不上传文字资产（`textAssetId = null`），页图照传。
 * - **单页失败不拖垮整轮**：该页记入 `failedPages` 并继续下一页（UI-014）。
 * - **取消/切换**：`AbortSignal` 触发时取消进行中的 render task 与上传，
 *   释放 canvas 与 PDF 文档；已完成的页保留在服务端（UI-015）。
 * - **worker 转移 ArrayBuffer**：`pdf` 由调用方打开（打开阶段的错误在那里分类），
 *   本函数负责在结束时 `destroy()`；调用方每次运行都重新读取原件字节。
 */

import type { PDFDocumentProxy } from "pdfjs-dist";

import { putPreparationPage, uploadPageAsset } from "../api";
import { isCancelled, PrepareCancelledError } from "./errors";
import { MAX_PAGE_IMAGE_LONG_EDGE, MAX_RENDER_SCALE } from "./vendor";

/** 进度：真实可数的进度（第 n / N 页、已完成页数），不产生线性总百分比。 */
export interface PageProgress {
  /** 正在处理的页号（1-based）；轮次之间为 null。 */
  readonly currentPage: number | null;
  readonly totalPages: number;
  readonly completedPages: number;
  /** 本轮开始时间（毫秒时间戳），用于"已用时"显示。 */
  readonly startedAt: number;
}

export interface PageFailure {
  readonly pageNumber: number;
  readonly message: string;
}

export interface PrepareOptions {
  readonly itemId: string;
  readonly preparationId: string;
  /** 已打开的 PDF 文档（由调用方 `openPdfDocument` 打开并做拒绝分类）。 */
  readonly pdf: PDFDocumentProxy;
  /** 本轮要处理的页号（升序、1-based）。续传时只包含缺失页。 */
  readonly pageNumbers: readonly number[];
  readonly totalPages: number;
  readonly onProgress?: (progress: PageProgress) => void;
  readonly onPageDone?: (pageNumber: number) => void;
  readonly onPageFailed?: (failure: PageFailure) => void;
  readonly signal?: AbortSignal;
}

export interface PrepareSummary {
  readonly completedPages: number;
  readonly failedPages: readonly number[];
  readonly startedAt: number;
}

/** 页文字提取结果（结构化文本；POS 变换只用于版面，不用来伪造字符框）。 */
interface TextItemLike {
  readonly str?: string;
  readonly hasEOL?: boolean;
}

/**
 * 把 PDF.js 的文字项拼成页文字。
 *
 * 规则：按 PDF.js 给出的阅读顺序拼接 `str`；`hasEOL` 处换行，其余项之间补空格。
 * 不推断字符坐标，也不做"OCR 式"补救（ADR-003：不做假装精确的 transform）。
 */
export function textItemsToText(items: readonly TextItemLike[]): string {
  let out = "";
  for (const item of items) {
    if (typeof item.str === "string") {
      out += item.str;
    }
    if (item.hasEOL === true) {
      out += "\n";
    } else {
      out += " ";
    }
  }
  return out
    .split("\n")
    .map((line) => line.replace(/\s+$/u, ""))
    .join("\n")
    .trim();
}

/** 渲染缩放：长边不超过 2000px，且不把小页面无限放大。 */
export function renderScaleFor(width: number, height: number): number {
  const longEdge = Math.max(width, height);
  if (longEdge <= 0) {
    return 1;
  }
  return Math.min(MAX_RENDER_SCALE, MAX_PAGE_IMAGE_LONG_EDGE / longEdge);
}

/** 页资产文件名（仅元数据；不含路径语义）。 */
export function pageAssetFilename(pageNumber: number, extension: string): string {
  return `page-${String(pageNumber).padStart(4, "0")}.${extension}`;
}

/**
 * 处理一批页并返回汇总。
 *
 * 抛出：`PrepareCancelledError`（取消）、PDF 打开阶段的原样错误（由调用方分类）。
 */
export async function preparePages(options: PrepareOptions): Promise<PrepareSummary> {
  const startedAt = Date.now();
  const signal = options.signal;
  const ensureNotCancelled = (): void => {
    if (signal?.aborted === true) {
      throw new PrepareCancelledError();
    }
  };

  const pdf = options.pdf;
  // 单张复用 canvas（不进 DOM）：每次只渲染一页，渲染前重置尺寸即等于"销毁上一张"。
  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d", { alpha: false });
  if (context === null) {
    await pdf.loadingTask.destroy();
    throw new Error("浏览器不支持 Canvas 2D：无法渲染页图");
  }

  let renderTask: { cancel: () => void } | null = null;
  const onAbort = (): void => {
    renderTask?.cancel();
  };
  signal?.addEventListener("abort", onAbort);

  const failedPages: number[] = [];
  let completedPages = 0;
  try {
    for (const pageNumber of options.pageNumbers) {
      ensureNotCancelled();
      options.onProgress?.({
        currentPage: pageNumber,
        totalPages: options.totalPages,
        completedPages,
        startedAt,
      });
      try {
        const page = await pdf.getPage(pageNumber);
        try {
          ensureNotCancelled();
          // 旋转后的 viewport（PDF.js 已把页面 /Rotate 计入 width/height 与 rotation）：
          // 页图坐标原点即该 viewport 左上角。
          const baseViewport = page.getViewport({ scale: 1 });
          const viewport = page.getViewport({
            scale: renderScaleFor(baseViewport.width, baseViewport.height),
          });
          canvas.width = Math.max(1, Math.ceil(viewport.width));
          canvas.height = Math.max(1, Math.ceil(viewport.height));
          // 白底：PDF 页本身是透明的，白底是页图合同的一部分（架构 §5.1）。
          context.setTransform(1, 0, 0, 1, 0, 0);
          context.fillStyle = "#ffffff";
          context.fillRect(0, 0, canvas.width, canvas.height);

          // v6 的 RenderParameters：显式用 canvasContext 渲染时 canvas 必须为 null
          // （文档：`canvas` 默认取 canvasContext 关联的 canvas；置 null 表示只用 context）。
          const task = page.render({ canvasContext: context, canvas: null, viewport });
          renderTask = task;
          await task.promise;
          renderTask = null;
          ensureNotCancelled();

          const jpeg = await canvasToBlob(canvas, "image/jpeg", 0.9);
          const textContent = await page.getTextContent();
          const text = textItemsToText(textContent.items as readonly TextItemLike[]);

          ensureNotCancelled();
          const imageAsset = await uploadPageAsset(
            options.itemId,
            "pageImage",
            jpeg,
            pageAssetFilename(pageNumber, "jpg"),
            signal ? { signal } : {},
          );
          let textAssetId: string | null = null;
          if (text !== "") {
            const textAsset = await uploadPageAsset(
              options.itemId,
              "pageText",
              new Blob([text], { type: "text/plain;charset=utf-8" }),
              pageAssetFilename(pageNumber, "txt"),
              signal ? { signal } : {},
            );
            textAssetId = textAsset.id;
          }
          ensureNotCancelled();
          await putPreparationPage(options.preparationId, pageNumber, {
            textAssetId,
            imageAssetId: imageAsset.id,
            viewport: {
              width: canvas.width,
              height: canvas.height,
              rotation: viewport.rotation,
            },
          });
        } finally {
          page.cleanup();
        }
        completedPages += 1;
        options.onPageDone?.(pageNumber);
      } catch (error) {
        if (isCancelled(error) || signal?.aborted === true) {
          throw new PrepareCancelledError();
        }
        failedPages.push(pageNumber);
        options.onPageFailed?.({
          pageNumber,
          message: error instanceof Error ? error.message : String(error),
        });
      }
      options.onProgress?.({
        currentPage: null,
        totalPages: options.totalPages,
        completedPages,
        startedAt,
      });
    }
  } finally {
    signal?.removeEventListener("abort", onAbort);
    renderTask?.cancel();
    // 销毁 canvas：置零尺寸并丢弃引用（UI-015：取消/切换时不保留旧帧）。
    canvas.width = 0;
    canvas.height = 0;
    await pdf.loadingTask.destroy();
  }

  return { completedPages, failedPages, startedAt };
}

/** `canvas.toBlob` 的 Promise 包装（JPEG 编码失败视为该页失败）。 */
function canvasToBlob(
  canvas: HTMLCanvasElement,
  type: string,
  quality: number,
): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => {
        if (blob === null) {
          reject(new Error("页图编码失败（canvas.toBlob 返回 null）"));
          return;
        }
        resolve(blob);
      },
      type,
      quality,
    );
  });
}

/** 计算需要补齐的页号（1..totalPages 中不在 `uploaded` 内的页）。 */
export function missingPageNumbers(
  totalPages: number,
  uploaded: readonly number[],
): number[] {
  const present = new Set(uploaded);
  const missing: number[] = [];
  for (let page = 1; page <= totalPages; page += 1) {
    if (!present.has(page)) {
      missing.push(page);
    }
  }
  return missing;
}
