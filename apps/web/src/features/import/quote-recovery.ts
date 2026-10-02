import { useCallback, useEffect, useRef, useState } from "react";
import { describeError } from "../../api/client";
import { getEstimate, type JobCreateRequest, type QuoteDto } from "../../api/endpoints";
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
  return { phase, quote, error, operation, remember, read, adopt, retry, summary };
}
