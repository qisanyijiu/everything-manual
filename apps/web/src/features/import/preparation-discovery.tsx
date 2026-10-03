import { useState } from "react";
import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { describeError, isApiError } from "../../api/client";
import { formatLocalDateTime } from "../../lib/format";
import { getPreparation, listPreparations, type DocumentDto, type PreparationCandidate, type PreparationDetail } from "./api";
import { recallPreparationId } from "./preparation-pointer";

export const CLIENT_DERIVED_NOTICE = "页图由本机浏览器生成（clientDerived）；哈希只证明字节一致，不证明其确实来自原 PDF，原件保留可复核。";

export function matchesDocument(candidate: PreparationCandidate, document: DocumentDto): boolean {
  return candidate.preparation.documentId === document.id && candidate.preparation.sourceSha256 === document.sourceSha256;
}

/** Both preparation and confirmation consume this server-verified, document-scoped selection. */
export function usePreparationDiscovery(itemId: string, document: DocumentDto | null, requestedId: string | null = null) {
  const queryClient = useQueryClient();
  const scope = `${itemId}/${document?.id ?? ""}/${document?.sourceSha256 ?? ""}`;
  const [choice, setChoice] = useState<{ scope: string; id: string } | null>(null);
  const query = useInfiniteQuery({
    queryKey: ["preparation-discovery", itemId, document?.id, document?.sourceSha256],
    enabled: document !== null,
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) => listPreparations(document?.id ?? "", pageParam, signal),
    getNextPageParam: (last) => last.nextCursor,
    refetchOnWindowFocus: false,
    retry: false,
  });
  const recommended = query.data?.pages[0]?.recommended ?? null;
  const chosenId = choice?.scope === scope ? choice.id : requestedId ?? query.data?.pages[0]?.recommendedPreparationId ?? null;
  const detail = useQuery({
    queryKey: ["preparation-selection", scope, chosenId],
    queryFn: ({ signal }) => getPreparation(chosenId ?? "", signal),
    enabled: document !== null && chosenId !== null && query.isSuccess,
    retry: false,
    refetchOnWindowFocus: false,
  });
  const value = detail.data?.detail;
  const selected = value && document && value.id === chosenId && matchesDocument({ preparation: value, readiness: value.readiness }, document)
    ? { preparation: value, readiness: value.readiness } : null;
  const cached = recallPreparationId(itemId);
  const hint = useQuery({
    queryKey: ["preparation-hint", scope, cached],
    queryFn: ({ signal }) => getPreparation(cached ?? "", signal),
    enabled: document !== null && cached !== null,
    retry: false,
    refetchOnWindowFocus: false,
  });
  const invalidSelection = chosenId !== null && detail.isSuccess && selected === null;
  const invalidHint = cached !== null && ((isApiError(hint.error) && hint.error.status === 404) || (hint.data && document &&
    (!matchesDocument({ preparation: hint.data.detail, readiness: hint.data.detail.readiness }, document) || !hint.data.detail.readiness.compatible)));
  const entries = [...new Map((query.data?.pages.flatMap((page) => page.data) ?? []).map((entry) => [entry.preparation.id, entry])).values()];
  return {
    query, detail, entries, recommended, selected, chosenId,
    loading: document !== null && (query.isPending || (chosenId !== null && detail.isPending)),
    error: query.error ?? detail.error,
    invalidHint: Boolean(invalidHint), invalidSelection,
    update: (resource: PreparationDetail) => {
      queryClient.setQueryData(["preparation-selection", scope, resource.detail.id], resource);
      setChoice({ scope, id: resource.detail.id });
    },
    choose: (candidateId: string) => setChoice({ scope, id: candidateId }),
    refresh: async () => {
      const result = await query.refetch();
      if (result.error) throw result.error;
      if (chosenId !== null) {
        const reread = await detail.refetch();
        if (reread.error) throw reread.error;
      }
    },
  };
}

export type PreparationDiscoveryState = ReturnType<typeof usePreparationDiscovery>;

export function preparationProgress(candidate: PreparationCandidate, knownTotal?: number | null): string {
  const count = candidate.readiness.completedPageCount;
  const total = candidate.preparation.pageCount ?? knownTotal;
  if (!candidate.readiness.compatible) return candidate.readiness.explanation ?? "记录无法复用，请重新准备。";
  if (candidate.preparation.state === "ready") return `准备完成 · ${total} 页`;
  return total == null ? `已完成 ${count} 页，总页数待读取原件` : `已完成 ${count}/${total} 页`;
}

export function PreparationDiscovery({ state, document, disabled = false }: {
  state: PreparationDiscoveryState; document: DocumentDto; disabled?: boolean;
}) {
  const [retryError, setRetryError] = useState<string | null>(null);
  const retry = async () => {
    setRetryError(null);
    try { await state.refresh(); } catch (error) { setRetryError(describeError(error).message); }
  };
  const row = (candidate: PreparationCandidate, recommended: boolean) => (
    <label className="preparation-record" key={candidate.preparation.id}>
      <input type="radio" name="preparation-record" value={candidate.preparation.id}
        checked={state.chosenId === candidate.preparation.id}
        disabled={disabled || !candidate.readiness.compatible || !matchesDocument(candidate, document)}
        onChange={() => state.choose(candidate.preparation.id)} />
      <span>
        <strong>{recommended ? (candidate.preparation.state === "ready" ? "可直接使用的准备结果" : "可继续的准备记录") : "准备记录"}{recommended && " · 推荐"}</strong>
        <span>{document.title} · {formatLocalDateTime(candidate.preparation.updatedAt)}</span>
        <span>{preparationProgress(candidate)}</span>
        <span>{candidate.readiness.compatible ? "兼容当前原件 · v1" : "不可复用"}</span>
        <code>{candidate.preparation.id}</code>
      </span>
    </label>
  );
  return <section className="preparation-discovery" aria-label="已保存的准备记录">
    {state.invalidHint && <p className="status-note">之前的记录不适用于当前原件；以下以服务端读取结果为准。</p>}
    {state.loading && <p role="status">正在查找已保存的准备记录…</p>}
    {(state.error || retryError || state.invalidSelection) && <div className="error-panel" role="alert">
      <h2>准备记录读取失败</h2>
      <p>{state.invalidSelection ? "之前的记录不适用于当前原件" : retryError ?? describeError(state.error).message}</p>
      <button type="button" disabled={disabled || state.query.isFetching || state.detail.isFetching} onClick={() => void retry()}>重新读取</button>
      {(state.invalidSelection || state.detail.error) && state.recommended && state.recommended.preparation.id !== state.chosenId && <button type="button" onClick={() => state.choose(state.recommended!.preparation.id)}>使用当前原件的推荐记录</button>}
    </div>}
    {/* Keep the radio group mounted while reading the selected detail, so native
        arrow navigation and the expanded details survive the asynchronous read. */}
    {!state.query.isPending && !state.query.error && <>
      {state.recommended && row(state.recommended, true)}
      {state.entries.length === 0 && <p>当前原件尚无准备记录。点击开始后才会读取并制作页资料。</p>}
      {state.entries.length > 0 && !state.recommended && <p role="status">已有记录无法复用。重新准备会新建记录，旧记录保留。</p>}
      {state.entries.length > 0 && <details>
        <summary>其他准备记录（已加载 {state.entries.length} 条）</summary>
        {state.entries.filter((entry) => entry.preparation.id !== state.recommended?.preparation.id).map((entry) => row(entry, false))}
        {state.query.hasNextPage && <button type="button" disabled={disabled || state.query.isFetchingNextPage} onClick={() => void state.query.fetchNextPage()}>加载更多准备记录</button>}
      </details>}
    </>}
  </section>;
}
