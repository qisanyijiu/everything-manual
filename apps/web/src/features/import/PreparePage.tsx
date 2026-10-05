import { usePageWork } from "../shell/work-protection";
import { useQueryClient } from "@tanstack/react-query";
import { workflowKeys, useItemSummaries } from "../library/workflow";
/** PDF preparation is explicit; all persisted progress comes from document-scoped discovery. */
import { useEffect, useRef, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router";
import { describeError, isApiError } from "../../api/client";
import { Skeleton } from "../../components/Skeleton";
import { PageLayout } from "../shell/PageLayout";
import { useItemDetail } from "../library/items";
import { useReaderDocuments } from "../viewer/reader-documents";
import { WizardNav, WizardSteps } from "./WizardSteps";
import { completePreparation, createOrResumePreparation, fetchAssetBytes, getPreparation, type DocumentDto, type PreparationDetail } from "./api";
import { rememberPreparationId } from "./preparation-pointer";
import { CLIENT_DERIVED_NOTICE, matchesDocument, PreparationDiscovery, preparationProgress, usePreparationDiscovery } from "./preparation-discovery";
import { classifyPdfError, isCancelled, PrepareCancelledError, tooManyPagesRejection, type PdfRejection } from "./pdf/errors";
import { isPreparationConflict, missingPageNumbers, preparePages, type PageFailure } from "./pdf/prepare";
import { MAX_PDF_PAGES, openPdfDocument } from "./pdf/vendor";
import { describeCompleteFailure } from "./messages";

interface RunState {
  phase: "idle" | "starting" | "preparing" | "stopping" | "sealing";
  preparationId: string | null;
  total: number | null;
  pages: number[];
  current: number | null;
  failures: PageFailure[];
  rejection: PdfRejection | null;
  stopped: boolean;
}
const INITIAL: RunState = { phase: "idle", preparationId: null, total: null, pages: [], current: null, failures: [], rejection: null, stopped: false };
const CONFLICT = "记录已更新，请重新读取进度";

export function PreparePage() {
  const { itemId = "" } = useParams();
  const [search, setSearch] = useSearchParams();
  const item = useItemDetail(itemId || null);
  const documents = useReaderDocuments(itemId || null);
  const [busy, setBusy] = useState(false);
  const all = documents.data?.documents ?? [];
  const summary = useItemSummaries([itemId], search.get("documentId"));
  const selectedId = search.get("documentId") ?? summary.data?.[0]?.documentId ?? null;
  const document = all.find((entry) => entry.id === selectedId) ?? null;
  return <PageLayout><section className="page prepare-page" aria-labelledby="prepare-title">
    <WizardSteps currentSegment="import/prepare" itemId={itemId} />
    <p className="eyebrow">DOCUMENT PREPARATION</p>
    <h1 id="prepare-title">准备说明书资料</h1>
    <p className="page__lead">{item.data?.data.name ?? "物品"}：把原件整理为可核对的逐页资料。此步骤不调用生成服务。</p>
    {summary.isError && <p role="alert">处理状态暂不可用。<button type="button" onClick={() => void summary.refetch()}>重新读取处理状态</button></p>}
    {(documents.isPending || summary.isPending) && <Skeleton label="正在读取原件…" rows={3} />}
    {documents.error && <div className="error-panel" role="alert"><p>原件清单读取失败：{describeError(documents.error).message}</p><button type="button" onClick={() => void documents.refetch()}>重新读取原件</button></div>}
    {!documents.isPending && !documents.error && all.length === 0 && <p>还没有说明书原件。<Link to={`/items/${itemId}/import/document`}>先去绑定原件</Link></p>}
    {all.length > 0 && <div className="field"><label htmlFor="prepare-document">所选原件</label>
      <select id="prepare-document" className="field__input" value={document?.id ?? ""} disabled={busy} onChange={(event) => setSearch({ documentId: event.target.value })}>
        {!document && <option value="">请选择当前物品的原件</option>}
        {all.map((entry) => <option key={entry.id} value={entry.id}>{entry.title}</option>)}
      </select>{busy && <p className="field__hint">请先停止当前准备，再切换原件。</p>}
    </div>}
    {document && <DocumentPreparation key={`${itemId}/${document.id}/${document.sourceSha256}`} itemId={itemId} document={document} requestedId={search.get("preparationId")} onBusy={setBusy} />}
    {!document && selectedId !== null && !documents.isPending && <p role="alert">之前的记录不适用于当前原件，请重新选择。</p>}
  </section></PageLayout>;
}

function DocumentPreparation({ itemId, document, requestedId, onBusy }: {
  itemId: string; document: DocumentDto; requestedId: string | null; onBusy: (value: boolean) => void;
}) {
  const discovery = usePreparationDiscovery(itemId, document, requestedId);
  const [run, setRun] = useState<RunState>(INITIAL);
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const abort = useRef<AbortController | null>(null);
  const mounted = useRef(true);
  const queryClient = useQueryClient();
  const running = useRef(false);
  const busy = run.phase !== "idle";
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; abort.current?.abort(); onBusy(false); };
  }, [onBusy]);
  useEffect(() => { onBusy(busy); }, [busy, onBusy]);
  usePageWork({ active: busy, message: "离开将停止本页尚未完成的资料准备。已上传及已保存的页保留；返回后读取进度，仅补缺页。在途请求可能已经完成。", discard: () => abort.current?.abort() });
  const selected = discovery.selected;
  const usable = selected?.readiness.compatible === true;
  const ready = usable && selected?.preparation.state === "ready";
  const runMatches = run.preparationId === selected?.preparation.id;
  const total = selected?.preparation.pageCount ?? (runMatches ? run.total : null);
  const pages = busy && runMatches ? run.pages : selected?.readiness.completedPages ?? [];
  const missing = total === null ? [] : missingPageNumbers(total, pages);
  const uncertain = discovery.loading || !!discovery.error || discovery.invalidSelection;

  function verify(resource: PreparationDetail): void {
    const candidate = { preparation: resource.detail, readiness: resource.detail.readiness };
    if (!matchesDocument(candidate, document)) throw new Error("之前的记录不适用于当前原件");
    if (!candidate.readiness.compatible) throw new Error(candidate.readiness.explanation ?? "记录无法复用，请重新准备。");
  }
  function publish(resource: PreparationDetail) {
    if (!mounted.current) return;
    discovery.update(resource);
    rememberPreparationId(itemId, resource.detail.id);
    setRun((previous) => ({ ...previous, preparationId: resource.detail.id, pages: resource.detail.readiness.completedPages, total: resource.detail.pageCount ?? previous.total }));
  }
  async function reread() {
    setError(null);
    try {
      await discovery.refresh();
      setConflict(false);
    } catch (failure) { setError(describeError(failure).message); }
  }
  async function start(createNew = false, onlyPage?: number) {
    if (running.current || uncertain || conflict || ready || (!createNew && selected && !usable)) return;
    running.current = true;
    const controller = new AbortController();
    abort.current = controller;
    setError(null);
    setRun({ ...INITIAL, phase: "starting", preparationId: createNew ? null : selected?.preparation.id ?? null });
    let preparationId = createNew ? null : selected?.preparation.id ?? null;
    let pdf: Awaited<ReturnType<typeof openPdfDocument>> | null = null;
    let stopped = false;
    try {
      // Recheck before reading/rendering the PDF. A remotely sealed record never enters the pipeline.
      let resource = preparationId === null ? null : await getPreparation(preparationId, controller.signal);
      if (resource) {
        verify(resource); publish(resource);
        if (resource.detail.state === "ready") return;
      }
      const bytes = await fetchAssetBytes(document.sourceAssetId, controller.signal);
      try { pdf = await openPdfDocument(bytes); }
      catch (failure) { if (!controller.signal.aborted) setRun((previous) => ({ ...previous, rejection: classifyPdfError(failure) })); return; }
      if (controller.signal.aborted) throw new PrepareCancelledError();
      if (pdf.numPages > MAX_PDF_PAGES) {
        setRun((previous) => ({ ...previous, rejection: tooManyPagesRejection(pdf!.numPages) }));
        return;
      }
      if (preparationId === null) {
        const created = await createOrResumePreparation(document.id, document.sourceSha256, createNew);
        preparationId = created.preparation.id;
        if (controller.signal.aborted) throw new PrepareCancelledError();
        resource = await getPreparation(preparationId, controller.signal);
        verify(resource); publish(resource);
      }
      if (controller.signal.aborted) throw new PrepareCancelledError();
      if (!resource || resource.detail.state === "ready") return;
      const pageCount = pdf.numPages;
      if (resource.detail.readiness.completedPages.some((number) => number > pageCount)) throw new Error("保存页码超出当前 PDF 总页数，请重新准备；旧记录保留。");
      const uploaded = resource.detail.readiness.completedPages;
      const remaining = missingPageNumbers(pageCount, uploaded).filter((number) => onlyPage === undefined || number === onlyPage);
      setRun({ ...INITIAL, preparationId, phase: "preparing", total: pageCount, pages: uploaded });
      if (remaining.length === 0) return;
      const pipelinePdf = pdf;
      pdf = null; // Pipeline owns cleanup, including cancellation and conflicts.
      await preparePages({ itemId, preparationId, pdf: pipelinePdf, pageNumbers: remaining, totalPages: pageCount, signal: controller.signal,
        onProgress: (progress) => { if (mounted.current) setRun((previous) => ({ ...previous, current: progress.currentPage })); },
        onPageDone: (number) => { if (mounted.current) setRun((previous) => ({ ...previous, pages: [...new Set([...previous.pages, number])].sort((a,b) => a-b) })); },
        onPageFailed: (failure) => { if (mounted.current) setRun((previous) => ({ ...previous, failures: [...previous.failures, failure] })); },
      });
    } catch (failure) {
      if (isCancelled(failure) || controller.signal.aborted) stopped = true;
      else if (mounted.current) {
        if (isPreparationConflict(failure)) { setConflict(true); setError(CONFLICT); }
        else {
          setError(describeError(failure).message);
        }
      }
    } finally {
      await pdf?.loadingTask.destroy().catch(() => undefined);
      stopped ||= controller.signal.aborted;
      // A PUT already in flight may settle after Stop. Wait for it, then use the persisted facts.
      if (preparationId !== null && mounted.current) {
        try { publish(await getPreparation(preparationId)); }
        catch (failure) { setError(`准备记录读取失败：${describeError(failure).message}`); setConflict(true); }
      }
      if (mounted.current) {
        await discovery.query.refetch();
        setRun((previous) => ({ ...previous, phase: "idle", current: null, stopped }));
      }
      running.current = false;
      abort.current = null;
    }
  }
  async function seal() {
    if (running.current || !selected || ready || uncertain || conflict || total === null || missing.length > 0) return;
    running.current = true;
    const controller = new AbortController(); abort.current = controller;
    setRun((previous) => ({ ...previous, phase: "sealing" }));
    setError(null);
    try {
      const current = await getPreparation(selected.preparation.id, controller.signal);
      verify(current);
      if (current.detail.state === "ready") { publish(current); return; }
      if (controller.signal.aborted) throw new PrepareCancelledError();
      await completePreparation(current.detail.id, total, current.etag);
      await queryClient.invalidateQueries({ queryKey: workflowKeys.root });
      publish(await getPreparation(current.detail.id));
      await discovery.query.refetch();
    } catch (failure) {
      if (isPreparationConflict(failure)) { setConflict(true); setError(CONFLICT); }
      else setError([describeError(failure).message, ...(isApiError(failure) ? describeCompleteFailure(failure.details) : [])].join("；"));
    } finally { running.current = false; abort.current = null; if (mounted.current) setRun((previous) => ({ ...previous, phase: "idle" })); }
  }
  return <>
    <PreparationDiscovery state={{ ...discovery, choose: (candidateId) => { setRun(INITIAL); setError(null); setConflict(false); discovery.choose(candidateId); } }} document={document} disabled={busy} />
    <p className="field__hint">准备需要保持本标签页打开；关闭标签页会中断准备，重新进入只补齐未完成的页。</p>
    {selected && !uncertain && <section className="prepare-progress" aria-label="当前准备进度">
      <h2>当前记录 · {document.title}</h2><p><code>{selected.preparation.id}</code></p>
      <p role="status" data-testid={ready ? "prepare-sealed" : "prepare-progress"}>{ready ? preparationProgress(selected) : total === null ? preparationProgress(selected) : `已完成 ${pages.length}/${total} 页`}</p>
      {busy && run.current !== null && <p role="status">正在处理第 {run.current} / {run.total} 页</p>}
      {total !== null && !ready && <progress aria-label="已完成页数" max={total} value={pages.length} />}
      {ready && <p>{CLIENT_DERIVED_NOTICE}</p>}
    </section>}
    {run.stopped && <p role="status">已停止，已完成页已保留。可稍后继续准备。</p>}
    {run.rejection && <div className="error-panel" role="alert"><h2>无法准备此 PDF</h2><p>{run.rejection.message}</p></div>}
    {error && <div className="error-panel" role="alert"><p>{error}</p><button type="button" disabled={busy} onClick={() => void reread()}>重新读取进度</button></div>}
    {runMatches && run.failures.length > 0 && <ul className="prepare-failures">{run.failures.map((failure) => <li key={failure.pageNumber}>第 {failure.pageNumber} 页：{failure.message} <button type="button" disabled={busy || conflict || ready || uncertain} onClick={() => void start(false, failure.pageNumber)}>重试第 {failure.pageNumber} 页</button></li>)}</ul>}
    <div className="prepare-actions">
      {busy ? <><button type="button" data-testid="prepare-cancel" disabled={run.phase === "stopping" || run.phase === "sealing"} onClick={() => { setRun((previous) => ({ ...previous, phase: "stopping" })); abort.current?.abort(); }}>{run.phase === "stopping" ? "正在停止并读取进度…" : run.phase === "sealing" ? "正在封存…" : "停止准备"}</button><p>停止后保留已完成页，可稍后继续。</p></>
        : !ready && !uncertain && <>
          {(selected === null && discovery.entries.length === 0) || usable ? <button type="button" className="button-primary" data-testid="prepare-start" disabled={conflict} onClick={() => void start()}>{selected ? "继续准备，仅补齐缺页" : "开始准备"}</button> : <><p>重新准备会新建记录，旧记录保留。</p><button type="button" data-testid="prepare-restart" disabled={conflict} onClick={() => void start(true)}>重新准备</button></>}
          {usable && total !== null && missing.length === 0 && <button type="button" className="button-primary" data-testid="prepare-seal" disabled={conflict} onClick={() => void seal()}>封存资料</button>}
        </>}
    </div>
    {!ready && <p className="field__hint">{CLIENT_DERIVED_NOTICE}</p>}
    <WizardNav currentSegment="import/prepare" itemId={itemId} nextDisabled={!ready || busy || uncertain} nextDisabledReason="请先补齐页资料并明确封存。" nextHref={`/items/${itemId}/import/confirm?documentId=${encodeURIComponent(document.id)}&preparationId=${encodeURIComponent(selected?.preparation.id ?? "")}`} />
  </>;
}
