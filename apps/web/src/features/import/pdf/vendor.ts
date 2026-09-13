/**
 * PDF.js 运行时装配（T09 / REQ-014；架构 §3「PDF」、§5.1、§7）。
 *
 * 硬约束（QA 按此复核）：
 * - **主包与 worker 必须同版本**：worker 通过 Vite 的 `?url` 显式导入
 *   （`pdfjs-dist/build/pdf.worker.min.mjs?url`），与 `import ... from "pdfjs-dist"`
 *   来自同一个 `node_modules/pdfjs-dist`；不手工拼接可能被 hash 改名的路径。
 * - **CMaps / standard fonts / WASM / ICC 一律本地加载、不访问 CDN**：URL 指向
 *   `/vendor/pdfjs/<dir>/`，由 `vite.pdfjs-vendor.ts` 从同一 pdfjs-dist 版本复制
 *   （dev 直接服务 node_modules，build 复制进 dist，随 rust-embed 进二进制）。
 * - **worker 的 ArrayBuffer 可能被转移**：`openPdfDocument` 每次调用都要求调用方
 *   传入一份新读取的字节（`File.arrayBuffer()` / 重新 fetch 原件），不复用同一份。
 *
 * 路径用 `new URL(..., window.location.origin)` 归一成绝对地址：PDF.js 在 worker 内
 * 也会用这些 URL 取 CMap/字体/WASM，绝对地址不受 worker 相对路径解析影响。
 */

import { GlobalWorkerOptions, getDocument, type PDFDocumentProxy } from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

import { PDFJS_VENDOR_PREFIX } from "./vendor-path";

/** 本地 PDF.js 资源根（与构建插件的前缀同源，见 `vite.pdfjs-vendor.ts`）。 */
export const PDFJS_VENDOR_BASE = new URL(
  `${import.meta.env.BASE_URL.replace(/\/$/, "")}${PDFJS_VENDOR_PREFIX}`,
  window.location.origin,
).href;

GlobalWorkerOptions.workerSrc = workerUrl;

/** 传给 `getDocument` 的资源地址（全部本地；不出现 CDN 域名）。 */
export function pdfjsAssetUrls(): {
  cMapUrl: string;
  cMapPacked: true;
  standardFontDataUrl: string;
  wasmUrl: string;
  iccUrl: string;
} {
  return {
    cMapUrl: `${PDFJS_VENDOR_BASE}cmaps/`,
    cMapPacked: true,
    standardFontDataUrl: `${PDFJS_VENDOR_BASE}standard_fonts/`,
    wasmUrl: `${PDFJS_VENDOR_BASE}wasm/`,
    iccUrl: `${PDFJS_VENDOR_BASE}iccs/`,
  };
}

/**
 * 打开一份 PDF 字节。
 *
 * `data` 会被 PDF.js 转移给 worker（`Transferable`），调用方**不得**在调用后再读它；
 * 需要再次打开时重新读取原 File / 原件内容。
 */
export function openPdfDocument(data: Uint8Array): Promise<PDFDocumentProxy> {
  return getDocument({
    data,
    ...pdfjsAssetUrls(),
    // 不请求远端：资源全部来自本地 vendor 目录（离线检查的语义之一）。
    useSystemFonts: false,
    disableAutoFetch: true,
    disableStream: true,
  }).promise;
}

/** 页图长边上限（架构 §5.1：≤2000px；与服务端 `MAX_PAGE_IMAGE_LONG_EDGE` 一致）。 */
export const MAX_PAGE_IMAGE_LONG_EDGE = 2000;

/** 原 PDF 页数上限（架构 §5.1：≤100 页；与服务端 `MAX_PDF_PAGES` 一致）。 */
export const MAX_PDF_PAGES = 100;

/** 页图放大上限：小页面不无限放大（避免无意义的体积与耗时）。 */
export const MAX_RENDER_SCALE = 2;
