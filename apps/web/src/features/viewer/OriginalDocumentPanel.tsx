/**
 * 原文（PDF）阅读面板（T18 / REQ-032 的"PDF 阅读不依赖 WebGL"；PRD UI-045/UI-057 的读取侧）。
 *
 * 关键约束（QA 按此复核）：
 * - **不经过 WebGL**：页面用 PDF.js 渲染到 2D canvas；3D 失败/不可用时本面板完全照常。
 * - **本地资源、无 CDN**：复用 T09 的 `openPdfDocument`（worker/CMaps/standard fonts/WASM
 *   全部来自本地 vendor 目录，见 `features/import/pdf/vendor.ts`）。
 * - **一次只渲染一页**：切换页/卸载时取消 render task、销毁 canvas 与 PDF 文档
 *   （与 PRD §5.5「100 页不同时铺满 canvas」同一原则）。
 * - **页码 1-based**：显示与跳转都用 `pageNumber`（contracts §1），不出现 0-based 页码。
 */

import { useEffect, useRef, useState } from "react";

import { fetchAssetContent } from "../../api/endpoints";
import { describeError } from "../../api/client";
import { openPdfDocument, MAX_RENDER_SCALE } from "../import/pdf/vendor";
import { textItemsToText } from "../import/pdf/prepare";
import type { PDFDocumentProxy } from "pdfjs-dist";

export interface OriginalDocumentPanelProps {
  /** 原件资产（`document.sourceAssetId`）；null = 该物品还没有绑定说明书。 */
  readonly assetId: string | null;
  /** 当前页（1-based，由页面状态驱动）。 */
  readonly pageNumber: number;
  readonly onPageChange: (pageNumber: number) => void;
  readonly onPageCount?: (pageCount: number) => void;
}

type PanelState =
  | { readonly phase: "idle" }
  | { readonly phase: "loading" }
  | { readonly phase: "ready" }
  | { readonly phase: "error"; readonly message: string };

export function OriginalDocumentPanel({
  assetId,
  pageNumber,
  onPageChange,
  onPageCount,
}: OriginalDocumentPanelProps) {
  const [state, setState] = useState<PanelState>({ phase: "idle" });
  const [pageText, setPageText] = useState<string>("");
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const pdfRef = useRef<PDFDocumentProxy | null>(null);
  const pageCountRef = useRef<number>(0);
  const renderTaskRef = useRef<{ cancel: () => void } | null>(null);

  // 打开原件（每次进入都重新读取字节：PDF.js 会转移 ArrayBuffer，不能复用）。
  useEffect(() => {
    if (assetId === null) {
      setState({ phase: "idle" });
      return;
    }
    let cancelled = false;
    setState({ phase: "loading" });
    const controller = new AbortController();
    const canvasAtStart = canvasRef.current;
    const run = async (): Promise<void> => {
      try {
        const { bytes } = await fetchAssetContent(assetId, { signal: controller.signal });
        const pdf = await openPdfDocument(new Uint8Array(bytes));
        if (cancelled) {
          await pdf.loadingTask.destroy();
          return;
        }
        pdfRef.current = pdf;
        pageCountRef.current = pdf.numPages;
        onPageCount?.(pdf.numPages);
        setState({ phase: "ready" });
      } catch (error) {
        if (!cancelled) {
          setState({ phase: "error", message: describeError(error).message });
        }
      }
    };
    void run();
    return () => {
      cancelled = true;
      controller.abort();
      renderTaskRef.current?.cancel();
      renderTaskRef.current = null;
      const pdf = pdfRef.current;
      pdfRef.current = null;
      pageCountRef.current = 0;
      if (pdf !== null) {
        void pdf.loadingTask.destroy();
      }
      if (canvasAtStart !== null) {
        canvasAtStart.width = 0;
        canvasAtStart.height = 0;
      }
    };
  }, [assetId, onPageCount]);

  // 渲染当前页（只渲染一页；切页时取消上一个任务）。
  useEffect(() => {
    const pdf = pdfRef.current;
    const canvas = canvasRef.current;
    if (state.phase !== "ready" || pdf === null || canvas === null) {
      return;
    }
    const context = canvas.getContext("2d", { alpha: false });
    if (context === null) {
      setState({ phase: "error", message: "浏览器不支持 Canvas 2D：无法显示原文页" });
      return;
    }
    let cancelled = false;
    const run = async (): Promise<void> => {
      try {
        renderTaskRef.current?.cancel();
        const page = await pdf.getPage(Math.min(Math.max(pageNumber, 1), pdf.numPages));
        if (cancelled) {
          page.cleanup();
          return;
        }
        const base = page.getViewport({ scale: 1 });
        const maxWidth = Math.max(canvas.parentElement?.clientWidth ?? 600, 240);
        const scale = Math.max(
          0.2,
          Math.min(MAX_RENDER_SCALE, maxWidth / Math.max(base.width, 1)),
        );
        const viewport = page.getViewport({ scale });
        const ratio = Math.min(window.devicePixelRatio || 1, 2);
        canvas.width = Math.max(1, Math.ceil(viewport.width * ratio));
        canvas.height = Math.max(1, Math.ceil(viewport.height * ratio));
        canvas.style.width = `${Math.ceil(viewport.width)}px`;
        context.setTransform(ratio, 0, 0, ratio, 0, 0);
        context.fillStyle = "#ffffff";
        context.fillRect(0, 0, viewport.width, viewport.height);
        const task = page.render({ canvasContext: context, canvas: null, viewport });
        renderTaskRef.current = task;
        await task.promise;
        renderTaskRef.current = null;
        const textContent = await page.getTextContent();
        if (!cancelled) {
          setPageText(
            textItemsToText(
              textContent.items as readonly { str?: string; hasEOL?: boolean }[],
            ),
          );
        }
        page.cleanup();
      } catch (error) {
        if (!cancelled) {
          setState({ phase: "error", message: describeError(error).message });
        }
      }
    };
    void run();
    return () => {
      cancelled = true;
      renderTaskRef.current?.cancel();
      renderTaskRef.current = null;
    };
  }, [state.phase, pageNumber]);

  const total = pageCountRef.current;

  return (
    <section className="original-panel" aria-label="原文（PDF）">
      <div className="original-panel__nav" role="group" aria-label="原文翻页">
        <button
          type="button"
          disabled={pageNumber <= 1}
          onClick={() => onPageChange(Math.max(1, pageNumber - 1))}
        >
          上一页
        </button>
        <span data-testid="original-page-label">
          第 {pageNumber} / {total > 0 ? total : "?"} 页
        </span>
        <button
          type="button"
          disabled={total > 0 && pageNumber >= total}
          onClick={() => onPageChange(pageNumber + 1)}
        >
          下一页
        </button>
      </div>
      {assetId === null && (
        <p className="original-panel__message" data-testid="original-empty">
          该物品还没有绑定说明书原件。
        </p>
      )}
      {state.phase === "loading" && (
        <p className="original-panel__message" role="status">
          正在加载原文…
        </p>
      )}
      {state.phase === "error" && (
        <p className="original-panel__message" role="alert" data-testid="original-error">
          原文加载失败：{state.message}
        </p>
      )}
      <canvas
        ref={canvasRef}
        className="original-panel__canvas"
        data-testid="original-canvas"
        role="img"
        aria-label={`原 PDF 第 ${pageNumber} 页`}
      />
      {pageText !== "" && (
        <details className="original-panel__text" data-testid="original-text">
          <summary>本页文字（PDF 文字层）</summary>
          <pre>{pageText}</pre>
        </details>
      )}
    </section>
  );
}

export default OriginalDocumentPanel;
