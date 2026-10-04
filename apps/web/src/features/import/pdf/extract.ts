/**
 * 在浏览器内从说明书 PDF 拆出视图候选图（ADR-044；与 `preparePages` 共用同一份 PDF.js 文档）。
 *
 * 每页：
 * 1. operator list → 内嵌位图的页面空间包围盒（`imageBoxesFromOperators`）；
 * 2. 渲染一张不含文字的灰度预览，墨迹掩膜降采样 + 膨胀 → 连通区域（矢量线稿也能找到）；
 * 3. 合并、去重、按 `plausibleFigure` 过滤；按面积降序最多取 `perPage` 个；
 * 4. 以高分辨率渲染整页，裁剪每个区域为 JPEG。
 *
 * 结果只是"候选"：由调用方上传为 `photo` 资产并登记 `view-candidates`（服务端可做 AI 建议视图）。
 */

import { OPS, type PDFDocumentProxy } from "pdfjs-dist";

import {
  DEFAULT_FILTER,
  connectedRegions,
  dedupeBoxes,
  dilate,
  imageBoxesFromOperators,
  pad,
  plausibleFigure,
  toPixels,
  type Box,
  type Matrix,
} from "./figures";

export interface FigureCandidate {
  readonly pageNumber: number;
  readonly source: "embedded" | "region";
  readonly box: Box;
  readonly blob: Blob;
  readonly width: number;
  readonly height: number;
}

export interface ExtractOptions {
  readonly pdf: PDFDocumentProxy;
  readonly pageNumbers: readonly number[];
  /** 每页最多候选数（防止表格页拆出一堆碎片）。 */
  readonly perPage?: number;
  /** 全部最多候选数。 */
  readonly maxTotal?: number;
  readonly signal?: AbortSignal;
  readonly onProgress?: (page: number, found: number) => void;
}

const MASK_CELL = 4; // 掩膜降采样：每格 4×4 像素
const INK_THRESHOLD = 200; // 灰度 < 200 视为墨迹
const CROP_LONG_EDGE = 1600;

function toBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => (blob ? resolve(blob) : reject(new Error("裁剪图编码失败"))), "image/jpeg", 0.92);
  });
}

async function renderPage(page: Awaited<ReturnType<PDFDocumentProxy["getPage"]>>, scale: number, withText: boolean) {
  const viewport = page.getViewport({ scale });
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.ceil(viewport.width));
  canvas.height = Math.max(1, Math.ceil(viewport.height));
  const context = canvas.getContext("2d", { alpha: false, willReadFrequently: !withText });
  if (context === null) {
    throw new Error("浏览器不支持 Canvas 2D");
  }
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, canvas.width, canvas.height);
  // 不含文字的版本：屏蔽文字绘制，只留下图形（线稿 + 位图），让区域查找不被文字段落干扰。
  const textLayerOff = !withText;
  const restore: (() => void)[] = [];
  if (textLayerOff) {
    const proto = context as unknown as { fillText: unknown; strokeText: unknown };
    const fillText = proto.fillText;
    const strokeText = proto.strokeText;
    proto.fillText = () => undefined;
    proto.strokeText = () => undefined;
    restore.push(() => {
      proto.fillText = fillText;
      proto.strokeText = strokeText;
    });
  }
  try {
    await page.render({ canvasContext: context, canvas: null, viewport }).promise;
  } finally {
    restore.forEach((fn) => fn());
  }
  return { canvas, context, viewport };
}

function inkRegions(context: CanvasRenderingContext2D, width: number, height: number, textBoxes: readonly Box[]): Box[] {
  const pixels = context.getImageData(0, 0, width, height).data;
  const mw = Math.ceil(width / MASK_CELL);
  const mh = Math.ceil(height / MASK_CELL);
  const mask = new Uint8Array(mw * mh);
  for (let y = 0; y < height; y += 2) {
    for (let x = 0; x < width; x += 2) {
      const i = (y * width + x) * 4;
      const gray = 0.299 * (pixels[i] ?? 255) + 0.587 * (pixels[i + 1] ?? 255) + 0.114 * (pixels[i + 2] ?? 255);
      if (gray < INK_THRESHOLD) {
        mask[Math.floor(y / MASK_CELL) * mw + Math.floor(x / MASK_CELL)] = 1;
      }
    }
  }
  // 擦掉文字层所在的格子：段落文字不应被当成"图形"（图注、编号也一起去掉，线稿本身保留）。
  for (const t of textBoxes) {
    const x0 = Math.max(0, Math.floor(t.x / MASK_CELL));
    const y0 = Math.max(0, Math.floor(t.y / MASK_CELL));
    const x1 = Math.min(mw - 1, Math.ceil((t.x + t.width) / MASK_CELL));
    const y1 = Math.min(mh - 1, Math.ceil((t.y + t.height) / MASK_CELL));
    for (let y = y0; y <= y1; y += 1) {
      mask.fill(0, y * mw + x0, y * mw + x1 + 1);
    }
  }
  return connectedRegions(dilate(mask, mw, mh, 3), mw, mh).map((b) => ({
    x: b.x * MASK_CELL,
    y: b.y * MASK_CELL,
    width: b.width * MASK_CELL,
    height: b.height * MASK_CELL,
  }));
}

export async function extractFigures(options: ExtractOptions): Promise<FigureCandidate[]> {
  const perPage = options.perPage ?? 4;
  const maxTotal = options.maxTotal ?? 24;
  const out: FigureCandidate[] = [];
  const opCodes = {
    save: OPS.save,
    restore: OPS.restore,
    transform: OPS.transform,
    paintImageXObject: OPS.paintImageXObject,
    paintInlineImageXObject: OPS.paintInlineImageXObject,
    paintImageMaskXObject: OPS.paintImageMaskXObject,
  };
  for (const pageNumber of options.pageNumbers) {
    if (options.signal?.aborted === true || out.length >= maxTotal) {
      break;
    }
    const page = await options.pdf.getPage(pageNumber);
    try {
      const base = page.getViewport({ scale: 1 });
      // 1) 区域查找用的低分辨率、不含文字渲染（长边约 1000px）。
      const probeScale = Math.min(2, 1000 / Math.max(base.width, base.height));
      const probe = await renderPage(page, probeScale, false);
      const W = probe.canvas.width;
      const H = probe.canvas.height;
      const embedded = imageBoxesFromOperators(await page.getOperatorList(), opCodes).map((b) => ({
        box: toPixels(b, probe.viewport.transform as unknown as Matrix),
        source: "embedded" as const,
      }));
      const text = await page.getTextContent();
      const textBoxes = text.items.flatMap((raw) => {
        const item = raw as { str?: string; transform?: number[]; width?: number; height?: number };
        if (!item.str || !item.str.trim() || !item.transform) {
          return [];
        }
        const [, , , d, e, f] = item.transform as [number, number, number, number, number, number];
        const h = Math.abs(item.height || d || 0);
        return [toPixels({ x: e, y: f - h * 0.25, width: item.width ?? 0, height: h * 1.3 }, probe.viewport.transform as unknown as Matrix)];
      });
      const regions = inkRegions(probe.context, W, H, textBoxes).map((box) => ({ box, source: "region" as const }));
      const filter = { pageWidth: W, pageHeight: H, ...DEFAULT_FILTER, minSide: Math.max(48, Math.round(DEFAULT_FILTER.minSide * (W / 1000))) };
      // 覆盖大半页的内嵌位图（整页扫描/截图）不直接当候选：交给区域查找在图内找具体的图形。
      const pageArea = W * H;
      const usableEmbedded = embedded.filter((c) => c.box.width * c.box.height < 0.6 * pageArea);
      const all = [...usableEmbedded, ...regions].filter((c) => plausibleFigure(c.box, filter));
      const keptBoxes = dedupeBoxes(all.map((c) => c.box)).slice(0, perPage);
      const picked = keptBoxes.map((box) => all.find((c) => c.box === box) ?? { box, source: "region" as const });
      probe.canvas.width = 0;
      if (picked.length === 0) {
        options.onProgress?.(pageNumber, 0);
        continue;
      }
      // 2) 高分辨率含文字渲染（与页面所见一致），按比例裁剪。
      const hiScale = Math.min(4, CROP_LONG_EDGE / Math.max(...picked.map((p) => Math.max(p.box.width, p.box.height) / probeScale)));
      const hi = await renderPage(page, Math.max(probeScale, hiScale), true);
      const k = hi.canvas.width / W;
      for (const candidate of picked) {
        const b = pad({ x: candidate.box.x * k, y: candidate.box.y * k, width: candidate.box.width * k, height: candidate.box.height * k }, 12 * k, hi.canvas.width, hi.canvas.height);
        const crop = document.createElement("canvas");
        crop.width = Math.max(1, Math.round(b.width));
        crop.height = Math.max(1, Math.round(b.height));
        const cctx = crop.getContext("2d", { alpha: false });
        if (cctx === null) {
          continue;
        }
        cctx.fillStyle = "#ffffff";
        cctx.fillRect(0, 0, crop.width, crop.height);
        cctx.drawImage(hi.canvas, b.x, b.y, b.width, b.height, 0, 0, crop.width, crop.height);
        out.push({ pageNumber, source: candidate.source, box: b, blob: await toBlob(crop), width: crop.width, height: crop.height });
        crop.width = 0;
        if (out.length >= maxTotal) {
          break;
        }
      }
      hi.canvas.width = 0;
      options.onProgress?.(pageNumber, picked.length);
    } finally {
      page.cleanup();
    }
  }
  return out;
}
