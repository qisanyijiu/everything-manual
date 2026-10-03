import { useCallback, useEffect, useRef, useState } from "react";
import { describeError, requestData } from "../../api/client";
import { API_PREFIX, getEstimate, type JobCreateRequest, type JobDetailDto, type QuoteDto } from "../../api/endpoints";
import { useItemSummaries } from "../library/workflow";

export interface SavedSubmission { quoteId: string; key: string; body: JobCreateRequest }
const storageKey = (itemId: string) => `manual:submission:${itemId}`;
const record = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const identifier = (value: unknown): value is string => typeof value === "string" && /^[A-Za-z0-9._:-]{1,128}$/.test(value);
const amount = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
function readHint(itemId: string): SavedSubmission | null {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(storageKey(itemId)) ?? "null");
    if (!record(value) || !identifier(value.quoteId) || !identifier(value.key) || !record(value.body)) return null;
    const body = value.body;
    if (body.quoteId !== value.quoteId || !identifier(body.preparationId) || !Array.isArray(body.photoIds) || body.photoIds.length < 2 || body.photoIds.length > 4 || !body.photoIds.every(identifier) || new Set(body.photoIds).size !== body.photoIds.length || !record(body.limits)) return null;
    const limits = body.limits;
    if (!amount(limits.tripoCreditMinor) || !amount(limits.manualAiUsdMicros)) return null;
    // Copy only the bounded fields we understand; never let an old/corrupt browser object
    // become a render-time number or a replay request. Server reads remain authoritative.
    return { quoteId: value.quoteId, key: value.key, body: {
      quoteId: value.quoteId, preparationId: body.preparationId, photoIds: [...body.photoIds],
      limits: { tripoCreditMinor: limits.tripoCreditMinor, manualAiUsdMicros: limits.manualAiUsdMicros },
    } };
  } catch { return null; }
}
function writeHint(itemId: string, value: SavedSubmission | null) {
  try { if (value) localStorage.setItem(storageKey(itemId), JSON.stringify(value)); else localStorage.removeItem(storageKey(itemId)); } catch { /* Server discovery remains authoritative when browser storage is unavailable. */ }
}

/** A cancelled parent can still own a running remote purchase. Fail closed on incomplete facts. */
export function completedJobRequoteProblem(job: JobDetailDto, jobId: string, itemId: string): string | null {
  const terminal = (status: string) => ["succeeded", "failed", "cancelled"].includes(status);
  const blocked = "旧任务仍有未完成或待核对的状态，请先到任务详情处理；尚未重新报价。";
  // A finished draft can retain an unknown Manual AI ledger entry after an
  // explicitly authorized replacement. Its usage is still unknown, but the
  // frozen authorized upper bound is zero. This only permits a new quote; a
  // new job still needs explicit confirmation.
  const completedFreeManualAi = job.status === "succeeded" && !!job.draftId
    && Array.isArray(job.stages) && job.stages.length > 0
    && job.stages.every((stage) => stage.status === "succeeded");
  const resolvedReservation = (entry: JobDetailDto["reservations"][number]) =>
    ["reserved", "settled", "released"].includes(entry.state)
    || (completedFreeManualAi && entry.state === "unknown" && entry.provider === "manual_ai"
      && entry.currency === "usdMicros" && entry.reservedMinor === 0);
  if (job.id !== jobId || job.item?.id !== itemId || !terminal(job.status)
    || !Array.isArray(job.stages) || job.stages.length === 0 || !job.stages.every((stage) => terminal(stage.status))
    || !Array.isArray(job.attempts) || !Array.isArray(job.reservations) || job.reservations.length === 0
    || !job.reservations.every(resolvedReservation)) return blocked;
  for (const attempt of job.attempts) {
    const stage = job.stages.find((entry) => entry.id === attempt.stageId);
    if (!stage || !["failed", "accepted"].includes(attempt.submitState)) return blocked;
    if (attempt.submitState === "accepted" || attempt.remoteTaskId) {
      if (stage.stageKind === "tripo_submit") {
        // Submit success only proves acceptance. Poll failure alone can mean a failed GET.
        const completedRemote = attempt.remoteTaskId && job.stages.some((entry) => {
          const usage = entry.usage;
          return entry.stageKind === "tripo_poll" && record(usage)
            && usage.remoteTaskId === attempt.remoteTaskId
            && typeof usage.normalizedStatus === "string"
            && ["success", "failed", "cancelled", "banned", "expired"].includes(usage.normalizedStatus);
        });
        if (!completedRemote) return blocked;
      } else if (stage.stageKind !== "manual_extract" || !["succeeded", "failed"].includes(stage.status)) return blocked;
    }
  }
  return null;
}

/** Only nonsecret operation identity/body is a hint. No POST, confirmation or new key here. */
export function useQuoteRecovery(itemId: string, requestedQuoteId: string | null) {
  const summary = useItemSummaries([itemId]);
  const [phase, setPhase] = useState<"discovering" | "checking" | "verified" | "unknown">("discovering");
  const [quote, setQuote] = useState<QuoteDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [operation, setOperation] = useState<SavedSubmission | null>(() => readHint(itemId));
  const operationRef = useRef(operation);
  const initialized = useRef(false);
  const generation = useRef(0);
  const completedJobCheck = useRef<{ jobId: string; generation: number } | null>(null);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const remember = useCallback((value: SavedSubmission | null) => {
    operationRef.current = value; setOperation(value); writeHint(itemId, value);
  }, [itemId]);
  const read = useCallback(async (quoteId: string, clearRejected = false) => {
    initialized.current = true;
    const serial = ++generation.current;
    setPhase("checking"); setError(null);
    try {
      const resource = await getEstimate(itemId, quoteId);
      if (!mounted.current || serial !== generation.current) return;
      setQuote(resource.data);
      // GET consumption takes precedence even for expired quotes, pending configuration or unknown jobs.
      if (resource.data.consumedJobId || clearRejected) remember(null);
      setPhase("verified");
    } catch (failure) {
      if (!mounted.current || serial !== generation.current) return;
      setError(describeError(failure).message); setPhase("unknown");
    }
  }, [itemId, remember]);
  useEffect(() => {
    if (initialized.current) return;
    const hintId = operationRef.current?.quoteId ?? requestedQuoteId;
    if (hintId) { void read(hintId); return; }
    if (summary.isPending) return;
    if (summary.isError) { setPhase("unknown"); setError(describeError(summary.error).message); return; }
    const saved = summary.data?.[0]?.latestQuoteId;
    if (saved) { void read(saved); return; }
    initialized.current = true; setPhase("verified");
  }, [read, requestedQuoteId, summary.data, summary.isPending, summary.isError, summary.error]);
  useEffect(() => {
    const synchronize = () => {
      const saved = readHint(itemId);
      if (saved && saved.key !== operationRef.current?.key) {
        operationRef.current = saved; setOperation(saved); void read(saved.quoteId);
      }
    };
    const onStorage = (event: StorageEvent) => { if (event.key === storageKey(itemId)) synchronize(); };
    window.addEventListener("storage", onStorage); window.addEventListener("focus", synchronize);
    return () => { window.removeEventListener("storage", onStorage); window.removeEventListener("focus", synchronize); };
  }, [itemId, read]);
  const previousRequested = useRef(requestedQuoteId);
  useEffect(() => {
    if (previousRequested.current === requestedQuoteId) return;
    previousRequested.current = requestedQuoteId;
    // Explicit history/link navigation is a read, but cannot bypass an unresolved operation.
    if (requestedQuoteId && requestedQuoteId !== quote?.id && !operationRef.current) void read(requestedQuoteId);
  }, [requestedQuoteId, quote?.id, read]);
  const adopt = useCallback((value: QuoteDto) => {
    initialized.current = true; generation.current += 1;
    setQuote(value); setError(null); setPhase("verified");
  }, []);
  const retry = useCallback(() => {
    const id = operationRef.current?.quoteId ?? quote?.id ?? requestedQuoteId ?? summary.data?.[0]?.latestQuoteId;
    if (id) void read(id);
    else { initialized.current = false; setPhase("discovering"); void summary.refetch(); }
  }, [quote?.id, read, requestedQuoteId, summary]);
  const hasPendingOperation = useCallback(() => operationRef.current !== null || readHint(itemId) !== null, [itemId]);
  const verifyCompletedJob = useCallback(async (jobId: string) => {
    completedJobCheck.current = null;
    if (hasPendingOperation()) throw new Error("仍有提交结果待核对，不能重新报价。");
    const serial = generation.current;
    const resource = await requestData<JobDetailDto>(`${API_PREFIX}/jobs/${encodeURIComponent(jobId)}`, { cache: "no-store" });
    if (!mounted.current || generation.current !== serial || hasPendingOperation()) throw new Error("提交或报价状态已改变，请先核对原操作；尚未重新报价。");
    const problem = completedJobRequoteProblem(resource.data, jobId, itemId);
    if (problem) throw new Error(problem);
    completedJobCheck.current = { jobId, generation: serial };
  }, [hasPendingOperation, itemId]);
  const consumeCompletedJobCheck = useCallback((jobId: string) => {
    const checked = completedJobCheck.current;
    completedJobCheck.current = null;
    return !!checked && checked.jobId === jobId && checked.generation === generation.current && !hasPendingOperation();
  }, [hasPendingOperation]);
  return { phase, quote, error, operation, remember, read, adopt, retry, summary, hasPendingOperation, verifyCompletedJob, consumeCompletedJobCheck };
}
