import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import { getItemSummaries, type ItemSummaryDto } from "../../api/endpoints";
import { formatLocalDateTime } from "../../lib/format";

export const workflowKeys = { root: ["item-workflow"] as const };

/** One bounded batch per 100 identities; never fetch full details per row. */
export function useItemSummaries(itemIds: readonly string[], documentId: string | null = null) {
  const ids = [...new Set(itemIds.filter(Boolean))].sort();
  return useQuery({
    queryKey: [...workflowKeys.root, ids, documentId],
    enabled: ids.length > 0,
    retry: false,
    staleTime: 0,
    refetchOnMount: "always",
    queryFn: async () => {
      const results: ItemSummaryDto[] = [];
      for (let offset = 0; offset < ids.length; offset += 100) {
        const response = await getItemSummaries(ids.slice(offset, offset + 100), documentId);
        results.push(...response.data);
      }
      return results;
    },
  });
}

export function workflowAction(summary: ItemSummaryDto): { label: string; status: string; href: string } {
  const item = `/items/${encodeURIComponent(summary.itemId)}`;
  const target = encodeURIComponent(summary.targetId ?? "");
  const context = new URLSearchParams();
  if (summary.documentId) context.set("documentId", summary.documentId);
  if (summary.preparationId) context.set("preparationId", summary.preparationId);
  if (summary.action === "confirm" && summary.latestQuoteId) context.set("quoteId", summary.latestQuoteId);
  const suffix = context.size > 0 ? `?${context}` : "";
  switch (summary.action) {
    case "handleJob": return { label: "处理任务", status: "任务需要处理", href: `/jobs/${target}` };
    case "viewJob": return { label: "查看进度", status: "任务进行中", href: `/jobs/${target}` };
    case "reviewDraft": return { label: "继续复核", status: "草稿待复核", href: `${item}/drafts/${target}/review` };
    case "readRelease": return { label: "阅读说明书", status: "已有发布版", href: `${item}/releases/${target}` };
    case "addDocument": return { label: "补齐资料", status: "待绑定说明书", href: `${item}/import/document${suffix}` };
    case "addViews": return { label: "补齐资料", status: "待补齐正面和侧面视图", href: `${item}/import/views${suffix}` };
    case "prepare": return { label: "补齐资料", status: "待完成说明书准备", href: `${item}/import/prepare${suffix}` };
    case "confirm": return { label: "预算与确认", status: "资料已就绪", href: `${item}/import/confirm${suffix}` };
  }
}

export function WorkflowActions({ itemId, summary, unavailable, loading }: { itemId: string; summary?: ItemSummaryDto; unavailable: boolean; loading: boolean }) {
  const action = summary && !unavailable ? workflowAction(summary) : null;
  return <div className="workflow-actions">
    <span className="workflow-actions__status">{unavailable ? "处理状态暂不可用" : action?.status ?? "正在读取处理状态…"}</span>
    {!unavailable && summary?.latestReleaseId && summary.latestReleaseDraftRevision != null && <span className="workflow-actions__version">已发布 · 草稿 r{summary.latestReleaseDraftRevision}{summary.latestReleaseCreatedAt && ` · ${formatLocalDateTime(summary.latestReleaseCreatedAt)}`}</span>}
    {action ? <Link className="button-primary" to={action.href}>{action.label}</Link> : <Link to={`/items/${itemId}`}>查看物品</Link>}
    {!loading && !unavailable && summary?.latestReleaseId && summary.action !== "readRelease" && <Link to={`/items/${itemId}/releases/${summary.latestReleaseId}`}>阅读已发布版</Link>}
    <Link to={`/items/${itemId}/releases`}>历史版本</Link>
  </div>;
}
