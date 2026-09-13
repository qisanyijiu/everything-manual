/**
 * 任务中心列表（PRD §6.1.2 `/jobs`；§6.2 UI-029、UI-030、UI-033）。
 *
 * - 游标分页（`cursor` 由 URL 承载，§6.1.1）；排序由服务端给定（U-03）；
 * - 可见页轮询约 2 秒 / 页面不可见约 15 秒 / **终态行不会让页面继续轮询**（UI-029）；
 * - 每行显示物品名称·型号、整体状态（文本标签）、阶段计数摘要（**不是百分比**）、
 *   分列费用预留；终态行给出草稿/详情的下一步；
 * - 网络错误保留上次数据并标注「数据可能已过期」，**不把网络问题写成任务失败**（UI-033）。
 */

import { useState } from "react";
import { Link, useSearchParams } from "react-router";

import { describeError, isApiError } from "../../api/client";
import { EmptyState } from "../../components/EmptyState";
import { Skeleton } from "../../components/Skeleton";
import { formatLocalDateTime } from "../../lib/format";
import { PageLayout } from "../shell/PageLayout";
import { costStateLabel, providerLabel } from "./CostBreakdown";
import { useJobList } from "./jobs";
import { jobStatusMeta, type JobStatusCategory } from "./status";

/**
 * 状态筛选（PRD §6.1.2 的「状态筛选侧栏」）。
 *
 * 服务端 `GET /jobs` 当前只支持 `itemId` 过滤（contracts §3），因此这里是**对已加载
 * 页面的客户端筛选**，界面必须如实说明"只筛选已加载的任务"——不冒充服务端全量过滤。
 */
const FILTERS: Array<{ id: string; label: string; categories: JobStatusCategory[] }> = [
  { id: "all", label: "全部", categories: [] },
  { id: "active", label: "进行中", categories: ["queue", "local", "provider"] },
  { id: "manual", label: "等待人工", categories: ["manual"] },
  { id: "unknown", label: "等待对账", categories: ["unknown"] },
  { id: "done", label: "已结束", categories: ["done"] },
  { id: "failed", label: "失败", categories: ["failed"] },
];

export function JobsListPage() {
  const [searchParams] = useSearchParams();
  const startCursor = searchParams.get("cursor");
  /** 物品过滤：`?itemId=`（资料库行内「查看任务」入口；由服务端过滤，游标与过滤条件绑定）。 */
  const itemId = searchParams.get("itemId");
  const jobsQuery = useJobList(startCursor, itemId);
  const [filter, setFilter] = useState("all");

  const pages = jobsQuery.data?.pages ?? [];
  const allJobs = pages.flatMap((page) => page.jobs);
  const selected = FILTERS.find((entry) => entry.id === filter) ?? FILTERS[0]!;
  const jobs =
    selected.categories.length === 0
      ? allJobs
      : allJobs.filter((job) => selected.categories.includes(jobStatusMeta(job.status).category));
  const loadMoreCursor =
    pages.length > 0 ? (pages[pages.length - 1]?.nextCursor ?? null) : null;
  const networkError = jobsQuery.isError && !isApiError(jobsQuery.error);
  const lastUpdated = jobsQuery.dataUpdatedAt;

  const filterPanel = (
    <div className="summary-panel" data-testid="job-filter-panel">
      <h2 className="summary-panel__title">状态筛选</h2>
      <div role="group" aria-label="按状态筛选任务" className="job-filter">
        {FILTERS.map((entry) => (
          <label key={entry.id} className="job-filter__option">
            <input
              type="radio"
              name="job-status-filter"
              value={entry.id}
              checked={filter === entry.id}
              onChange={() => setFilter(entry.id)}
            />
            {entry.label}
          </label>
        ))}
      </div>
      <p className="field__hint">
        筛选只作用于<strong>已加载</strong>的任务（服务端列表接口当前不支持按状态过滤）；
        更早的任务需要先「加载更早的任务」。
      </p>
    </div>
  );

  return (
    <PageLayout aside={{ id: "job-filter", label: "状态筛选", content: filterPanel }}>
    <section className="page jobs-page" aria-labelledby="jobs-title">
      <h1 id="jobs-title">任务中心</h1>
      <p className="page__lead">
        任务阶段、费用与恢复入口都以此页与服务端为准；生成完成不等于已发布，草稿需要人工复核。
      </p>

      {itemId !== null && (
        <p className="notice-inline" role="status" data-testid="jobs-item-scope">
          仅显示该物品的任务（服务端过滤）。<Link to="/jobs">查看全部任务</Link>
        </p>
      )}

      {!jobsQuery.isPending && allJobs.length > 0 && jobs.length === 0 && (
        <p className="empty-note" data-testid="job-filter-empty">
          当前筛选下没有已加载的任务：可切换筛选或加载更早的任务。
        </p>
      )}

      {networkError && (
        <p className="notice-inline" role="status" data-testid="network-notice">
          网络连接异常，正在自动重试（本地状态未变；下面显示的是最后一次读到的服务端状态）。
        </p>
      )}

      {jobsQuery.isPending && <Skeleton label="正在读取任务…" rows={4} />}

      {jobsQuery.isError && jobs.length === 0 && isApiError(jobsQuery.error) && (
        <div className="error-panel" role="alert" data-testid="jobs-load-error">
          <h2>无法读取任务列表</h2>
          <p>{describeError(jobsQuery.error).message}</p>
          <div className="error-panel__actions">
            <button type="button" onClick={() => void jobsQuery.refetch()}>
              重试
            </button>
          </div>
        </div>
      )}

      {jobsQuery.isError && jobs.length === 0 && networkError && (
        <div className="error-panel" role="alert" data-testid="jobs-network-error">
          <h2>无法连接服务</h2>
          <p>{describeError(jobsQuery.error).message}</p>
          <p className="error-panel__meta">
            这是网络/进程问题，不是任务失败：任务状态仍以服务端数据库为准，恢复连接后会自动更新。
          </p>
        </div>
      )}

      {!jobsQuery.isPending && jobs.length === 0 && !jobsQuery.isError && (
        <EmptyState
          title="还没有任务"
          description="完成资料准备与预算确认后即可创建任务；任务的阶段与费用会在这里显示。"
          action={<Link to="/items/new">去新建物品</Link>}
        />
      )}

      {jobs.length > 0 && (
        <>
          <p className="jobs-page__meta" data-testid="jobs-updated-at">
            最后更新时间：{lastUpdated === 0 ? "（尚未取到）" : formatLocalDateTime(new Date(lastUpdated).toISOString())}
            {networkError && "（网络异常，数据可能已过期）"}
          </p>
          <ul className="jobs-list">
            {jobs.map((job) => {
              const meta = jobStatusMeta(job.status);
              return (
                <li key={job.id} className="jobs-list__row" data-testid="job-list-row">
                  <div className="jobs-list__head">
                    <h2 className="jobs-list__title">
                      <Link to={`/jobs/${job.id}`}>
                        {job.itemName}
                        {job.itemModel !== "" && ` · ${job.itemModel}`}
                      </Link>
                    </h2>
                    <span className={`status-tag status-tag--${meta.category}`} data-testid="job-list-status">
                      {meta.label}
                    </span>
                  </div>
                  <p className="jobs-list__meta">
                    创建于 {formatLocalDateTime(job.createdAt)}；更新于{" "}
                    {formatLocalDateTime(job.updatedAt)}
                  </p>
                  <p className="jobs-list__stages" data-testid="job-list-stage-summary">
                    阶段：共 {job.stageSummary.total}（已完成 {job.stageSummary.succeeded}、进行中{" "}
                    {job.stageSummary.active}、缺项 {job.stageSummary.blocked}、待对账{" "}
                    {job.stageSummary.unknown}、失败 {job.stageSummary.failed}、已取消{" "}
                    {job.stageSummary.cancelled}）
                  </p>
                  <ul className="jobs-list__costs" data-testid="job-list-costs">
                    {job.reservations.map((entry) => (
                      <li key={`${job.id}-${entry.provider}-${entry.state}`}>
                        {providerLabel(entry.provider)}：{costStateLabel(entry.state)}{" "}
                        <span data-testid={`job-list-cost-${entry.provider}-${entry.state}`}>
                          {entry.reservedDisplay}
                        </span>
                      </li>
                    ))}
                    {job.reservations.length === 0 && <li>无费用记录（未产生付费提交）</li>}
                  </ul>
                  <div className="jobs-list__actions">
                    <Link to={`/jobs/${job.id}`}>查看阶段与恢复入口</Link>
                    {job.draftId !== null && job.draftId !== undefined && (
                      <Link to={`/items/${job.itemId}/drafts/${job.draftId}/review`}>
                        打开草稿（待复核）
                      </Link>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
          {loadMoreCursor !== null && (
            <p className="library-page__more">
              <Link
                to={`/jobs?${new URLSearchParams({
                  ...(itemId === null ? {} : { itemId }),
                  cursor: loadMoreCursor,
                }).toString()}`}
                data-testid="jobs-more"
              >
                加载更早的任务
              </Link>
            </p>
          )}
          {startCursor !== null && (
            <p className="library-page__more">
              <Link to={itemId === null ? "/jobs" : `/jobs?itemId=${encodeURIComponent(itemId)}`}>
                返回最新任务
              </Link>
            </p>
          )}
        </>
      )}
    </section>
    </PageLayout>
  );
}

