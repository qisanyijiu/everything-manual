/** One protected PDF, one canvas. Page selection never silently substitutes another page. */
import { useEffect, useId, useRef, useState } from "react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import { fetchAssetContent } from "../../api/endpoints";
import { describeError } from "../../api/client";
import { openPdfDocument, MAX_RENDER_SCALE } from "../import/pdf/vendor";
import { textItemsToText } from "../import/pdf/prepare";

export interface OriginalDocumentPanelProps {
  readonly assetId: string | null;
  readonly pageNumber: number;
  readonly onPageChange: (pageNumber: number) => void;
  readonly onPageCount?: (pageCount: number) => void;
  readonly fromEvidence?: boolean;
}
type PanelState =
  | { phase: "idle" | "loading" }
  | { phase: "ready"; total: number }
  | { phase: "error"; message: string; requestId: string | null };

export function OriginalDocumentPanel({ assetId, pageNumber, onPageChange, onPageCount, fromEvidence = false }: OriginalDocumentPanelProps) {
  const [state, setState] = useState<PanelState>({ phase: "idle" });
  const [pageText, setPageText] = useState("");
  const [renderedKey, setRenderedKey] = useState<string | null>(null);
  const [input, setInput] = useState(String(pageNumber));
  const [inputError, setInputError] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const renderingKey = `${assetId ?? "none"}/${pageNumber}/${attempt}`;
  const rendered = renderedKey === renderingKey;
  const inputId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const pdfRef = useRef<PDFDocumentProxy | null>(null);
  const countCallback = useRef(onPageCount);
  countCallback.current = onPageCount;
  const total = state.phase === "ready" ? state.total : null;
  const validPage = total !== null && Number.isSafeInteger(pageNumber) && pageNumber >= 1 && pageNumber <= total;

  useEffect(() => {
    setInput(Number.isFinite(pageNumber) ? String(pageNumber) : "");
    setInputError(false);
  }, [assetId, pageNumber]);

  useEffect(() => {
    let cancelled = false;
    const controller = new AbortController();
    let opened: PDFDocumentProxy | null = null;
    pdfRef.current = null;
    setPageText("");
    setRenderedKey(null);
    setState({ phase: assetId === null ? "idle" : "loading" });
    if (assetId !== null) void (async () => {
      try {
        const { bytes } = await fetchAssetContent(assetId, { signal: controller.signal });
        if (cancelled) return;
        opened = await openPdfDocument(new Uint8Array(bytes));
        if (cancelled) { await opened.loadingTask.destroy(); return; }
        pdfRef.current = opened;
        countCallback.current?.(opened.numPages);
        setState({ phase: "ready", total: opened.numPages });
      } catch (error) {
        if (!cancelled) setState({ phase: "error", ...describeError(error) });
      }
    })();
    return () => {
      cancelled = true;
      controller.abort();
      pdfRef.current = null;
      if (opened !== null) void opened.loadingTask.destroy();
    };
  }, [assetId, attempt]);

  useEffect(() => {
    const pdf = pdfRef.current;
    const canvas = canvasRef.current;
    setPageText("");
    setRenderedKey(null);
    if (!validPage || pdf === null || canvas === null) return;
    const context = canvas.getContext("2d", { alpha: false });
    if (context === null) {
      setState({ phase: "error", message: "浏览器不支持 Canvas 2D：无法显示原文页", requestId: null });
      return;
    }
    let cancelled = false;
    let task: { cancel: () => void } | null = null;
    void (async () => {
      const page = await pdf.getPage(pageNumber);
      try {
        if (cancelled) return;
        const base = page.getViewport({ scale: 1 });
        const maxWidth = Math.max(canvas.parentElement?.clientWidth ?? 600, 1);
        const viewport = page.getViewport({ scale: Math.min(MAX_RENDER_SCALE, maxWidth / Math.max(base.width, 1)) });
        const ratio = Math.min(window.devicePixelRatio || 1, 2);
        canvas.width = Math.max(1, Math.ceil(viewport.width * ratio));
        canvas.height = Math.max(1, Math.ceil(viewport.height * ratio));
        canvas.style.width = `${viewport.width}px`;
        context.setTransform(ratio, 0, 0, ratio, 0, 0);
        context.fillStyle = "#ffffff";
        context.fillRect(0, 0, viewport.width, viewport.height);
        const rendering = page.render({ canvasContext: context, canvas: null, viewport });
        task = rendering;
        await rendering.promise;
        const text = await page.getTextContent();
        if (!cancelled) {
          setPageText(textItemsToText(text.items as readonly { str?: string; hasEOL?: boolean }[]));
          setRenderedKey(renderingKey);
        }
      } finally { page.cleanup(); }
    })().catch((error: unknown) => {
      if (!cancelled) setState({ phase: "error", ...describeError(error) });
    });
    return () => {
      cancelled = true;
      task?.cancel();
      canvas.width = 0;
      canvas.height = 0;
    };
  }, [renderingKey, validPage, pageNumber, total]);

  const errorText = `请输入 1 至 ${total ?? "…"} 的整数页码`;
  const jump = () => {
    const number = Number(input);
    if (!/^\d+$/.test(input.trim()) || !Number.isSafeInteger(number) || total === null || number < 1 || number > total) {
      setInputError(true);
      inputRef.current?.focus();
      return;
    }
    setInputError(false);
    onPageChange(number);
  };
  return (
    <section className="original-panel" aria-label="原文（PDF）">
      <p className="page-note">PDF 实际页码{total === null ? " · 正在读取页数" : ` · 共 ${total} 页`}</p>
      <div className="original-panel__nav" role="group" aria-label="原文翻页">
        <button type="button" disabled={!validPage || pageNumber <= 1} onClick={() => onPageChange(pageNumber - 1)}>上一页</button>
        <span data-testid="original-page-label">{validPage ? `第 ${pageNumber} / ${total} 页` : total === null ? "正在读取页数" : "页码不可用"}</span>
        <button type="button" disabled={!validPage || pageNumber >= (total ?? 0)} onClick={() => onPageChange(pageNumber + 1)}>下一页</button>
      </div>
      <form className="original-panel__jump" onSubmit={(event) => { event.preventDefault(); jump(); }}>
        <label htmlFor={inputId}>页码</label>
        <input ref={inputRef} id={inputId} inputMode="numeric" value={input} disabled={total === null}
          aria-invalid={inputError} aria-describedby={inputError ? `${inputId}-error` : undefined}
          onChange={(event) => { setInput(event.target.value); setInputError(false); }} />
        <button type="submit" disabled={total === null}>跳转</button>
        {inputError && <p id={`${inputId}-error`} role="alert">{errorText}</p>}
      </form>
      {assetId === null && <p className="original-panel__message" data-testid="original-empty">该物品还没有绑定说明书原件。</p>}
      {state.phase === "loading" && <p className="original-panel__message" role="status">正在加载原文…</p>}
      {total !== null && !validPage && <p role="alert" className="original-panel__message">{fromEvidence ? "此出处页码超出原件范围" : "此页码不可用"}。{errorText}。</p>}
      {state.phase === "error" && <div className="original-panel__message" role="alert" data-testid="original-error">
        <p>原文加载失败：{state.message}</p>
        {state.requestId !== null && <p>诊断请求 ID：<code>{state.requestId}</code></p>}
        <button type="button" onClick={() => setAttempt((value) => value + 1)}>重新加载原文</button>
      </div>}
      {validPage && !rendered && <p role="status">正在绘制第 {pageNumber} 页…</p>}
      <canvas ref={canvasRef} hidden={!validPage || !rendered} className="original-panel__canvas" data-testid="original-canvas" role="img" aria-label={`原 PDF 第 ${pageNumber} 页`} />
      {validPage && rendered && <details className="original-panel__text" data-testid="original-text">
        <summary>本页文字（PDF 文字层）</summary>
        {pageText !== "" ? <pre>{pageText}</pre> : <p>本页没有可读取的文字层，可查看上方页图。</p>}
      </details>}
    </section>
  );
}
export default OriginalDocumentPanel;
