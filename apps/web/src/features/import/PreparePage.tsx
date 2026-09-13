/**
 * 向导第 4 步：PDF 逐页准备与续传（T09 / REQ-014、REQ-015；PRD §6.2 UI-014–UI-018）。
 *
 * 交互要点（QA 按此复核）：
 * - 进度是「第 n / N 页」与已完成页数（`role="progressbar"` 的 `aria-valuenow` 是已完成页数），
 *   **不出现与真实页数无关的百分比或预计剩余时间**（PRD §6.3.2 禁用措辞）。
 * - 常驻文案：准备需要保持本标签页打开；关闭标签页会中断准备，重新进入只补齐未完成的页。
 *   进行中注册 `beforeunload` 离开确认（UI-015）。
 * - 断线续传：进入时先用 `GET /preparations/{id}` 的服务端状态，只渲染并上传缺失页（UI-017）；
 *   服务端记录是事实来源，本地不缓存"已完成"结论。
 * - 加密与超页数 PDF 在打开阶段拒绝（UI-016）：**不创建页记录、不创建准备记录、不进 job**。
 * - 封存（UI-018）需显式点击；缺页/资产不符的 422 逐条列出并提供「继续补齐」；
 *   成功显示「准备完成（ready）」且禁用写入控件；常驻 `clientDerived` 说明。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useParams } from "react-router";

import { describeError } from "../../api/client";
import { PageLayout } from "../shell/PageLayout";
import { Skeleton } from "../../components/Skeleton";
import {
  forgetPreparationId,
  recallPreparationId,
  rememberPreparationId,
} from "./preparation-pointer";
import { WizardNav, WizardSteps } from "./WizardSteps";
import { useNotify } from "../../components/notifications";
import {
  completePreparation,
  createOrResumePreparation,
  fetchAssetBytes,
  getPreparation,
  type DocumentDto,
} from "./api";
import { describeCompleteFailure } from "./messages";
import {
  classifyPdfError,
  isCancelled,
  tooManyPagesRejection,
  type PdfRejection,
} from "./pdf/errors";
import {
  missingPageNumbers,
  preparePages,
  type PageFailure,
  type PageProgress,
} from "./pdf/prepare";
import { MAX_PDF_PAGES, openPdfDocument } from "./pdf/vendor";
// 物品与 document 的读取复用 T07/T08 已交付的 Query 钩子（同一份服务端事实，避免第二套缓存）。
import { useItemDetail, useItemDocuments } from "../library/items";

/** 常驻提示（UI-015）：准备期间必须保持页面打开。 */
const KEEP_OPEN_NOTICE =
  "准备需要保持本标签页打开；关闭标签页会中断准备，重新进入只补齐未完成的页。";

/** 封存说明（UI-018）：哈希只证明字节一致。 */
const CLIENT_DERIVED_NOTICE =
  "页图由本机浏览器生成（clientDerived）；哈希只证明字节一致，不证明其确实来自原 PDF，原件保留可复核。";

type Phase = "idle" | "starting" | "preparing" | "sealed";

interface RunState {
  readonly phase: Phase;
  readonly totalPages: number | null;
  readonly uploaded: readonly number[];
  readonly currentPage: number | null;
  readonly completedPages: number;
  readonly startedAt: number | null;
  readonly failures: readonly PageFailure[];
  readonly rejection: PdfRejection | null;
  /** 上一轮是"续传"（服务端已有页）时为 true，用于「已完成 n / N 页，继续补齐」文案。 */
  readonly resumed: boolean;
}

const INITIAL_STATE: RunState = {
  phase: "idle",
  totalPages: null,
  uploaded: [],
  currentPage: null,
  completedPages: 0,
  startedAt: null,
  failures: [],
  rejection: null,
  resumed: false,
};

export function PreparePage() {
  const { itemId } = useParams();
  const id = itemId ?? "";
  const itemQuery = useItemDetail(id === "" ? null : id);
  const documentsQuery = useItemDocuments(id === "" ? null : id);
  const notify = useNotify();

  const [documentId, setDocumentId] = useState<string | null>(null);
  const [preparationId, setPreparationId] = useState<string | null>(null);
  const [etag, setEtag] = useState<string | null>(null);
  const [state, setState] = useState<RunState>(INITIAL_STATE);
  const [sealError, setSealError] = useState<string | null>(null);
  const [sealing, setSealing] = useState(false);
  const [now, setNow] = useState(() => Date.now());

  const abortRef = useRef<AbortController | null>(null);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      abortRef.current?.abort();
    };
  }, []);

  // 进行中：注册离开确认（刷新/关闭标签页），并每秒更新"已用时"。
  const preparing = state.phase === "preparing";
  useEffect(() => {
    if (!preparing) {
      return;
    }
    const handler = (event: BeforeUnloadEvent): void => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", handler);
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => {
      window.removeEventListener("beforeunload", handler);
      window.clearInterval(timer);
    };
  }, [preparing]);

  const documents: readonly DocumentDto[] = documentsQuery.data?.documents ?? [];
  const selectedDocument =
    documents.find((document) => document.id === documentId) ?? documents[0] ?? null;

  // 刷新/重新进入：用服务端查询恢复"已完成哪些页"（不重传已完成页，UI-017）。
  const selectedId = selectedDocument?.id ?? null;
  useEffect(() => {
    if (selectedId === null) {
      return;
    }
    const stored = recallPreparationId(id);
    if (stored === null) {
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const detail = await getPreparation(stored);
        if (cancelled || !mountedRef.current) {
          return;
        }
        setPreparationId(stored);
        setEtag(detail.etag);
        setState((previous) => ({
          ...previous,
          totalPages: detail.detail.pageCount ?? previous.totalPages,
          uploaded: detail.detail.pages.map((page) => page.pageNumber),
          phase: detail.detail.state === "ready" ? "sealed" : previous.phase,
        }));
      } catch {
        forgetPreparationId(id);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [id, selectedId]);

  const refreshPreparation = useCallback(async (preparation: string): Promise<number[]> => {
    const detail = await getPreparation(preparation);
    if (mountedRef.current) {
      setEtag(detail.etag);
      setState((previous) => ({
        ...previous,
        totalPages: detail.detail.pageCount ?? previous.totalPages,
        uploaded: detail.detail.pages.map((page) => page.pageNumber),
        phase: detail.detail.state === "ready" ? "sealed" : previous.phase,
      }));
    }
    return detail.detail.pages.map((page) => page.pageNumber);
  }, []);

  /** 开始/继续准备：先解析 PDF 并做拒绝判定，再创建/复用服务端记录并只补缺失页。 */
  const start = useCallback(async (): Promise<void> => {
    if (selectedDocument === null) {
      return;
    }
    setSealError(null);
    setState({ ...INITIAL_STATE, phase: "starting" });
    const controller = new AbortController();
    abortRef.current = controller;
    let pdf: Awaited<ReturnType<typeof openPdfDocument>> | null = null;
    try {
      // 每次运行都重新读取原件字节：worker 可能已转移上一次的 ArrayBuffer。
      const bytes = await fetchAssetBytes(selectedDocument.sourceAssetId, controller.signal);
      pdf = await openPdfDocument(bytes);
      if (pdf.numPages > MAX_PDF_PAGES) {
        const rejection = tooManyPagesRejection(pdf.numPages);
        await pdf.loadingTask.destroy().catch(() => undefined);
        pdf = null;
        setState({ ...INITIAL_STATE, rejection });
        return;
      }
      const { preparation } = await createOrResumePreparation(
        selectedDocument.id,
        selectedDocument.sourceSha256,
      );
      if (mountedRef.current) {
        setPreparationId(preparation.id);
      }
      rememberPreparationId(id, preparation.id);
      const uploaded = await refreshPreparation(preparation.id);
      const totalPages = pdf.numPages;
      const missing = missingPageNumbers(totalPages, uploaded);
      if (missing.length === 0) {
        setState((previous) => ({
          ...previous,
          phase: "idle",
          totalPages,
          uploaded,
          resumed: uploaded.length > 0,
        }));
        return;
      }
      setState({
        ...INITIAL_STATE,
        phase: "preparing",
        totalPages,
        uploaded,
        startedAt: Date.now(),
        resumed: uploaded.length > 0,
      });
      await preparePages({
        itemId: id,
        preparationId: preparation.id,
        pdf,
        pageNumbers: missing,
        totalPages,
        signal: controller.signal,
        onProgress: (progress: PageProgress) => {
          if (!mountedRef.current) {
            return;
          }
          setState((previous) => ({
            ...previous,
            currentPage: progress.currentPage,
            completedPages: progress.completedPages,
            startedAt: progress.startedAt,
          }));
        },
        onPageFailed: (failure) => {
          if (!mountedRef.current) {
            return;
          }
          setState((previous) => ({ ...previous, failures: [...previous.failures, failure] }));
        },
      });
      pdf = null; // preparePages 已 destroy。
      const finalPages = await refreshPreparation(preparation.id);
      setState((previous) => ({
        ...previous,
        phase: "idle",
        uploaded: finalPages,
        currentPage: null,
        resumed: false,
      }));
    } catch (error) {
      if (pdf !== null) {
        // `preparePages` 在 finally 里负责销毁；cancel 路径可能已销毁过，
        // 二次调用是 no-op（PDF.js 用 `_transport?.destroy()` + 置空），这里再兜一层。
        await pdf.loadingTask.destroy().catch(() => undefined);
      }
      if (isCancelled(error) || controller.signal.aborted) {
        if (mountedRef.current) {
          setState((previous) => ({ ...previous, phase: "idle", currentPage: null }));
          if (preparationId !== null) {
            await refreshPreparation(preparationId).catch(() => undefined);
          }
        }
        return;
      }
      const rejection = classifyPdfError(error);
      if (mountedRef.current) {
        setState({ ...INITIAL_STATE, rejection });
      }
    } finally {
      abortRef.current = null;
    }
  }, [id, preparationId, refreshPreparation, selectedDocument]);

  /** 取消：销毁 render task 与上传，保留已完成页（UI-015）。 */
  const cancel = useCallback((): void => {
    abortRef.current?.abort();
    setState((previous) => ({ ...previous, phase: "idle", currentPage: null }));
  }, []);

  /** 重试单页（UI-014：失败页不阻塞其它页）。 */
  const retryPage = useCallback(
    async (pageNumber: number): Promise<void> => {
      if (selectedDocument === null || preparationId === null || state.totalPages === null) {
        return;
      }
      const controller = new AbortController();
      abortRef.current = controller;
      const pdf = await openPdfDocument(
        await fetchAssetBytes(selectedDocument.sourceAssetId, controller.signal),
      );
      try {
        setState((previous) => ({
          ...previous,
          phase: "preparing",
          failures: previous.failures.filter((failure) => failure.pageNumber !== pageNumber),
          startedAt: Date.now(),
        }));
        await preparePages({
          itemId: id,
          preparationId,
          pdf,
          pageNumbers: [pageNumber],
          totalPages: state.totalPages,
          signal: controller.signal,
          onProgress: (progress) => {
            if (mountedRef.current) {
              setState((previous) => ({
                ...previous,
                currentPage: progress.currentPage,
                completedPages: progress.completedPages,
              }));
            }
          },
          onPageFailed: (failure) => {
            if (mountedRef.current) {
              setState((previous) => ({ ...previous, failures: [...previous.failures, failure] }));
            }
          },
        });
      } catch (error) {
        if (!isCancelled(error) && mountedRef.current) {
          setState((previous) => ({
            ...previous,
            failures: [...previous.failures, { pageNumber, message: describeError(error).message }],
          }));
        }
      } finally {
        abortRef.current = null;
        const refreshed = await refreshPreparation(preparationId).catch(() => null);
        if (mountedRef.current) {
          setState((previous) => ({
            ...previous,
            phase: "idle",
            currentPage: null,
            uploaded: refreshed ?? previous.uploaded,
          }));
        }
      }
    },
    [id, preparationId, refreshPreparation, selectedDocument, state.totalPages],
  );

  /** 封存：If-Match + pageCount；缺项 422 逐条列出（UI-018）。 */
  const seal = useCallback(async (): Promise<void> => {
    if (preparationId === null || state.totalPages === null || selectedDocument === null) {
      return;
    }
    setSealing(true);
    setSealError(null);
    try {
      const latest = await getPreparation(preparationId);
      const sealed = await completePreparation(
        preparationId,
        state.totalPages,
        latest.etag ?? etag,
      );
      if (mountedRef.current) {
        setEtag(`"r${sealed.revision}"`);
        setState((previous) => ({ ...previous, phase: "sealed" }));
        notify("准备已封存（ready）");
      }
    } catch (error) {
      const info = describeError(error);
      const details = (error as { details?: unknown } | null)?.details;
      const lines = describeCompleteFailure(details);
      setSealError(lines.length > 0 ? `${info.message}（${lines.join("；")}）` : info.message);
    } finally {
      if (mountedRef.current) {
        setSealing(false);
      }
    }
  }, [etag, notify, preparationId, selectedDocument, state.totalPages]);

  const item = itemQuery.data?.data;
  const elapsedSeconds =
    state.startedAt === null ? 0 : Math.max(0, Math.round((now - state.startedAt) / 1000));
  const allUploaded =
    state.totalPages !== null && state.uploaded.length >= state.totalPages;

  return (
    <PageLayout>
      <section className="page">
        <WizardSteps currentSegment="import/prepare" itemId={id} />
        <h1>资料准备</h1>
        <p className="page__lead">
          {item?.name ?? "物品"}：浏览器逐页提取页文字并渲染页图，上传到本机服务端后封存。
        </p>

        <p className="notice-inline" role="note">
          {KEEP_OPEN_NOTICE}
        </p>

        {(itemQuery.isPending || documentsQuery.isPending) && (
          <Skeleton label="正在读取资料…" rows={3} />
        )}

        {(itemQuery.error !== null || documentsQuery.error !== null) && (
          <div className="error-panel" role="alert">
            <h2>无法读取准备资料</h2>
            <p>{describeError(itemQuery.error ?? documentsQuery.error).message}</p>
            {describeError(itemQuery.error ?? documentsQuery.error).requestId !== null && (
              <p className="error-panel__meta">
                请求 ID：{describeError(itemQuery.error ?? documentsQuery.error).requestId}
              </p>
            )}
          </div>
        )}

        {!itemQuery.isPending && !documentsQuery.isPending && documents.length === 0 && (
          <div className="empty-state">
            <p>这件物品还没有绑定说明书原件。</p>
            <p>
              <Link to={`/items/${id}/import/document`}>先绑定说明书原件</Link>
            </p>
          </div>
        )}

        {!itemQuery.isPending && !documentsQuery.isPending && documents.length > 0 && (
          <div className="prepare">
            {documents.length > 1 && (
              <div className="field">
                <label htmlFor="prepare-document">说明书原件</label>
                <select
                  id="prepare-document"
                  value={selectedDocument?.id ?? ""}
                  onChange={(event) => {
                    setDocumentId(event.target.value);
                    setPreparationId(null);
                    setState(INITIAL_STATE);
                  }}
                  disabled={preparing}
                >
                  {documents.map((document) => (
                    <option key={document.id} value={document.id}>
                      {document.title}
                    </option>
                  ))}
                </select>
              </div>
            )}

            {state.rejection !== null && (
              <div className="error-panel" role="alert">
                <h2>无法开始准备</h2>
                <p>{state.rejection.message}</p>
                <p className="error-panel__meta">
                  没有创建页记录，也没有任何收费请求；请换一份可用的 PDF 后重试。
                </p>
              </div>
            )}

            {state.totalPages !== null && state.phase !== "sealed" && (
              <div className="prepare__progress">
                <p aria-live="polite" data-testid="prepare-status">
                  {state.phase === "preparing"
                    ? `第 ${state.currentPage ?? "-"} / ${state.totalPages} 页`
                    : state.uploaded.length > 0
                      ? `已完成 ${state.uploaded.length} / ${state.totalPages} 页`
                      : `共 ${state.totalPages} 页，尚未开始`}
                </p>
                <p className="prepare__meta">
                  已完成页数：{state.uploaded.length} / {state.totalPages}
                  {state.phase === "preparing" && `；已用时 ${elapsedSeconds} 秒`}
                </p>
                <div
                  className="progressbar"
                  role="progressbar"
                  aria-label="已完成的页数"
                  aria-valuenow={state.uploaded.length}
                  aria-valuemin={0}
                  aria-valuemax={state.totalPages}
                >
                  <span
                    className="progressbar__fill"
                    style={{
                      width: `${Math.round((state.uploaded.length / state.totalPages) * 100)}%`,
                    }}
                  />
                </div>
              </div>
            )}

            {state.totalPages === null && state.uploaded.length > 0 && (
              <p data-testid="prepare-resume-hint">
                已完成 {state.uploaded.length} 页，继续补齐（总页数在开始准备后确认）。
              </p>
            )}

            {state.failures.length > 0 && (
              <ul className="prepare__failures" data-testid="prepare-failures">
                {state.failures.map((failure) => (
                  <li key={failure.pageNumber}>
                    <span>
                      第 {failure.pageNumber} 页失败：{failure.message}
                    </span>
                    <button type="button" onClick={() => void retryPage(failure.pageNumber)}>
                      重试本页
                    </button>
                  </li>
                ))}
              </ul>
            )}

            <div className="prepare__actions">
              <button
                type="button"
                data-testid="prepare-start"
                onClick={() => void start()}
                disabled={preparing || state.phase === "starting" || state.phase === "sealed"}
              >
                {state.phase === "starting"
                  ? "正在读取 PDF…"
                  : allUploaded && state.phase !== "sealed"
                    ? "重新检查缺失页"
                    : state.uploaded.length > 0 && state.phase !== "sealed"
                      ? "继续准备"
                      : "开始准备"}
              </button>
              <button
                type="button"
                data-testid="prepare-cancel"
                onClick={cancel}
                disabled={!preparing}
              >
                取消
              </button>
              <button
                type="button"
                data-testid="prepare-seal"
                onClick={() => void seal()}
                disabled={!allUploaded || sealing || state.phase === "sealed"}
              >
                {sealing ? "正在封存…" : "封存资料"}
              </button>
            </div>

            {sealError !== null && (
              <div className="error-panel" role="alert">
                <p>{sealError}</p>
                <button type="button" onClick={() => void start()}>
                  继续补齐
                </button>
              </div>
            )}

            {state.phase === "sealed" && (
              <div className="prepare__sealed" role="status" data-testid="prepare-sealed">
                <h2>准备完成（ready）</h2>
                <p>页图与页文字已封存（{state.uploaded.length} 页）。</p>
                <p className="error-panel__meta">{CLIENT_DERIVED_NOTICE}</p>
              </div>
            )}

            <p className="prepare__note">{CLIENT_DERIVED_NOTICE}</p>
          </div>
        )}

        <WizardNav
          currentSegment="import/prepare"
          itemId={id}
          nextDisabled={state.phase !== "sealed"}
          nextDisabledReason="资料准备尚未封存（ready）：请先完成全部页并点击「封存资料」。"
        />
      </section>
    </PageLayout>
  );
}
