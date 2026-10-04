/**
 * 物品的生成历史（路由 `/items/:itemId/generations`）。
 *
 * 服务端从不删除生成产物：每个任务一份冻结快照、一份草稿与它自己的 GLB，发布版本不可变。
 * 本页把它们按时间倒序列出（`GET /jobs?itemId=` 游标分页 + `GET /items/{id}/releases`），
 * 每一次生成都能回看结果、进入复核，或打开由它发布出的版本。
 */

import { useMemo } from "react";
import { Link, useParams } from "react-router";
import { useQuery } from "@tanstack/react-query";

import { describeError } from "../../api/client";
import { getItem, listReleases, type ReleaseDto } from "../../api/endpoints";
import { EmptyState } from "../../components/EmptyState";
import { Skeleton } from "../../components/Skeleton";
import { formatLocalDateTime } from "../../lib/format";
import { itemKeys } from "../library/items";
import { costStateLabel, providerLabel } from "./CostBreakdown";
import { useJobList } from "./jobs";
import { jobStatusMeta } from "./status";

export function GenerationHistoryPage() {
  const itemId = useParams().itemId ?? "";
  const itemQuery = useQuery({
    queryKey: itemKeys.detail(itemId),
    queryFn: () => getItem(itemId),
    enabled: itemId !== "",
  });
  const releasesQuery = useQuery({
    queryKey: ["releases", itemId],
    queryFn: () => listReleases(itemId),
    enabled: itemId !== "",
  });
  const jobsQuery = useJobList(null, itemId);
  const jobs = useMemo(() => jobsQuery.data?.pages.flatMap((page) => page.jobs) ?? [], [jobsQuery.data]);

  /** draftId → 由该草稿发布出的版本（一个草稿可多次发布）。 */
  const releasesByDraft = useMemo(() => {
    const map = new Map<string, ReleaseDto[]>();
    for (const release of releasesQuery.data ?? []) {
      const list = map.get(release.draftId) ?? [];
      list.push(release);
      map.set(release.draftId, list);
    }
    return map;
  }, [releasesQuery.data]);

  const item = itemQuery.data?.data ?? null;

  return (
    <div className="page generation-history" data-testid="generation-history">
      <header className="page__header">
        <div>
          <p className="eyebrow">GENERATION HISTORY</p>
          <h1>生成历史{item !== null && ` · ${item.name}`}</h1>
          <p className="field__hint">每次生成的 3D 模型与提取结果都会永久保留，可随时回看、复核或重新发布。</p>
        </div>
        <div className="page__header-actions">
          <Link className="button" to={`/items/${itemId}`}>返回物品概览</Link>
        </div>
      </header>

      {jobsQuery.isPending ? (
        <Skeleton rows={4} label="正在读取生成历史" />
      ) : jobsQuery.isError ? (
        <p className="field__error" role="alert">
          读取失败：{describeError(jobsQuery.error).message}
        </p>
      ) : jobs.length === 0 ? (
        <EmptyState
          title="还没有生成记录"
          description="完成导入向导并确认生成后，每一次结果都会出现在这里。"
          action={<Link className="button-primary" to={`/items/${itemId}/import/confirm`}>去报价与生成</Link>}
        />
      ) : (
        <ol className="history-list">
          {jobs.map((job, index) => {
            const meta = jobStatusMeta(job.status);
            const releases = job.draftId === null || job.draftId === undefined ? [] : (releasesByDraft.get(job.draftId) ?? []);
            return (
              <li key={job.id} className="history-card" data-testid="history-entry">
                <div className="history-card__head">
                  <h2>
                    第 {jobs.length - index} 次生成
                    <span className={`status-label status-label--${meta.category}`}>{meta.label}</span>
                  </h2>
                  <p className="field__hint">{formatLocalDateTime(job.createdAt)}</p>
                </div>
                <ul className="history-card__costs">
                  {job.reservations.map((reservation) => (
                    <li key={reservation.provider}>
                      {providerLabel(reservation.provider)}：{reservation.reservedDisplay}（{costStateLabel(reservation.state)}）
                    </li>
                  ))}
                </ul>
                {releases.length > 0 && (
                  <p className="history-card__releases">
                    已发布：
                    {releases.map((release) => (
                      <Link key={release.id} to={`/items/${itemId}/releases/${release.id}`}>
                        {formatLocalDateTime(release.createdAt)}
                      </Link>
                    ))}
                  </p>
                )}
                <div className="history-card__actions">
                  <Link className="button-primary" to={`/jobs/${job.id}/result`}>查看结果</Link>
                  {job.draftId !== null && job.draftId !== undefined && (
                    <Link className="button" to={`/items/${itemId}/drafts/${job.draftId}/review`}>复核 / 发布</Link>
                  )}
                  <Link className="button" to={`/jobs/${job.id}`}>任务详情</Link>
                </div>
              </li>
            );
          })}
        </ol>
      )}
      {jobsQuery.hasNextPage && (
        <button type="button" className="button" onClick={() => void jobsQuery.fetchNextPage()} disabled={jobsQuery.isFetchingNextPage}>
          {jobsQuery.isFetchingNextPage ? "正在加载…" : "加载更早的生成"}
        </button>
      )}
    </div>
  );
}

export default GenerationHistoryPage;
