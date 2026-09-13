/**
 * 任务详情（PRD §6.1.2 `/jobs/:jobId`；§6.2 UI-027、UI-031–UI-039）。
 *
 * - 主栏：阶段明细与**按阶段的恢复入口**（重试/对账；见 `JobStageList`）；
 * - 右栏：费用区块（credits/USD 分列）与任务级操作（取消）；
 * - 轮询：终态停止、页面不可见降频（见 `jobs.ts`）；网络错误不误报业务失败（UI-033）；
 * - 412 冲突（If-Match 过期）走统一刷新提示（UI-008），不自动覆盖；
 * - 取消前写明后果：不保证供应商撤单、已提交阶段保留查询与账务（UI-036）。
 */

import { useState } from "react";
import { Link, useParams } from "react-router";

import { describeError, isApiError } from "../../api/client";
import { ConflictNotice } from "../../components/ConflictNotice";
import { Skeleton } from "../../components/Skeleton";
import { useNotify } from "../../components/notifications";
import { formatLocalDateTime } from "../../lib/format";
import { Drawer } from "../shell/Drawer";
import { PageLayout } from "../shell/PageLayout";
import { CostBreakdown } from "./CostBreakdown";
import { JobStageList } from "./JobStageList";
import { readCurrentRevision, useCancelJob, useJobDetail } from "./jobs";
import { jobStatusMeta, retryDeniedHint, stageKindLabel } from "./status";

export function JobDetailPage() {
  const { jobId } = useParams();
  const id = jobId ?? "";
  const notify = useNotify();
  const jobQuery = useJobDetail(id === "" ? null : id);
  const cancelMutation = useCancelJob();
  const [cancelOpen, setCancelOpen] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [conflictRevision, setConflictRevision] = useState<number | null>(null);

  const resource = jobQuery.data;
  const detail = resource?.data ?? null;
  const ifMatch = resource?.etag ?? null;
  const networkError = jobQuery.isError && !isApiError(jobQuery.error);
  const meta = detail === null ? null : jobStatusMeta(detail.status);

  async function cancel(): Promise<void> {
    if (ifMatch === null) {
      return;
    }
    setActionError(null);
    setConflictRevision(null);
    try {
      const result = await cancelMutation.mutateAsync({ jobId: id, ifMatch });
      setCancelOpen(false);
      notify(`取消完成：已停止未提交阶段 ${result.stagesCancelled} 个。${result.notice}`);
    } catch (error) {
      setCancelOpen(false);
      const info = describeError(error);
      if (isApiError(error) && error.status === 412) {
        setConflictRevision(readCurrentRevision(error.details));
        return;
      }
      setActionError(info.message);
      notify(`取消失败：${info.message}`, { kind: "alert", requestId: info.requestId });
    }
  }

  const actionPanel = detail === null ? null : (
    <div className="summary-panel" data-testid="job-actions">
      <h2 className="summary-panel__title">操作</h2>
      {meta !== null && (
        <p className="job-detail__next" data-testid="job-next-step">
          下一步：{meta.nextStep}
        </p>
      )}
      {detail.status !== "succeeded" &&
      detail.status !== "failed" &&
      detail.status !== "cancelled" ? (
        <>
          <button
            type="button"
            className="button-secondary"
            data-testid="cancel-button"
            disabled={ifMatch === null || cancelMutation.isPending}
            onClick={() => setCancelOpen(true)}
          >
            取消任务…
          </button>
          <p className="field__hint">
            取消只停止本地的后续推进：已提交给供应商的付费操作不会被撤销（不保证供应商撤单）。
          </p>
        </>
      ) : (
        <p className="field__hint" data-testid="cancel-not-needed">
          任务已结束（{meta?.label}）：没有可取消的推进。
        </p>
      )}
      {detail.stages.some((stage) => stage.status === "needs_input") && (
        <p className="field__hint" data-testid="recovery-summary">
          有阶段等待人工补齐：可在下方对应阶段查看缺项与可用恢复动作（是否可重试由服务端判定）。
        </p>
      )}
      {detail.stages.some((stage) => stage.status === "submission_unknown") && (
        <p className="field__warning" data-testid="reconcile-summary">
          有阶段的付费提交结果未知：已暂停该分支后续购买，请先对账（不提供盲目重试）。
        </p>
      )}
      {detail.draftId !== null && detail.draftId !== undefined && (
        <p className="field__hint">
          <Link to={`/items/${detail.item.id}/drafts/${detail.draftId}/review`}>
            打开草稿（待复核）
          </Link>
          ：生成完成不等于已发布。
        </p>
      )}
    </div>
  );

  return (
    <PageLayout
      aside={{
        id: "job-cost",
        label: "费用与操作",
        content: (
          <>
            {detail !== null && (
              <CostBreakdown reservations={detail.reservations} budgetNotice={detail.budgetNotice} />
            )}
            {actionPanel}
          </>
        ),
      }}
    >
      <section className="page job-detail" aria-labelledby="job-detail-title">
        <h1 id="job-detail-title">
          {detail === null ? "任务详情" : `${detail.item.name}${detail.item.model !== "" ? ` · ${detail.item.model}` : ""}`}
        </h1>

        {networkError && (
          <p className="notice-inline" role="status" data-testid="network-notice">
            网络连接异常，正在自动重试（本地状态未变；下面显示的是最后一次读到的服务端状态）。
          </p>
        )}

        {jobQuery.isPending && <Skeleton label="正在读取任务详情…" rows={5} />}

        {jobQuery.isError && detail === null && (
          <div className="error-panel" role="alert" data-testid="job-load-error">
            <h2>{networkError ? "无法连接服务" : "无法读取任务详情"}</h2>
            <p>{describeError(jobQuery.error).message}</p>
            {networkError && (
              <p className="error-panel__meta">
                这是网络/进程问题，不是任务失败：任务状态以服务端数据库为准。
              </p>
            )}
            <div className="error-panel__actions">
              <button type="button" onClick={() => void jobQuery.refetch()}>
                重试
              </button>
              <Link to="/jobs">返回任务中心</Link>
            </div>
          </div>
        )}

        {detail !== null && (
          <>
            <div className="job-detail__status" data-testid="job-detail-status">
              <span className={`status-tag status-tag--${meta?.category ?? "failed"}`}>
                {meta?.label ?? detail.status}
              </span>
              <p className="job-detail__meta">
                任务 <code>{detail.id}</code>；创建于 {formatLocalDateTime(detail.createdAt)}；
                更新于 {formatLocalDateTime(detail.updatedAt)}；版本 r{detail.revision}。
              </p>
              <p className="field__hint">
                状态为服务端事实（刷新或重启后从这里恢复）；202 只表示任务已入队。
              </p>
            </div>

            {conflictRevision !== null && (
              <ConflictNotice
                currentRevision={conflictRevision}
                refreshing={jobQuery.isFetching}
                description="取消/重试/对账使用任务的版本（If-Match）：其他操作已更新了该任务；你填写的表单内容会保留。"
                onRefresh={() => {
                  setConflictRevision(null);
                  void jobQuery.refetch();
                }}
              />
            )}

            {actionError !== null && (
              <p className="field__error" role="alert" data-testid="job-action-error">
                {actionError}
              </p>
            )}

            {detail.stages.some((stage) => stage.status === "failed") && (
              <div className="policy-note" role="note" data-testid="failed-summary">
                <h2>失败阶段的错误摘要</h2>
                <ul>
                  {detail.stages
                    .filter((stage) => stage.status === "failed")
                    .map((stage) => (
                      <li key={stage.id}>
                        {stageKindLabel(stage.stageKind, stage.batchIndex)}：
                        {stage.lastError ?? "（服务端未给出错误摘要）"}
                      </li>
                    ))}
                </ul>
                <p className="field__hint">
                  应用不会自动降质量、换模型或增加处理阶段；失败阶段的可用恢复动作见下方阶段明细。
                </p>
              </div>
            )}

            <JobStageList
              itemId={detail.item.id}
              jobId={detail.id}
              jobStatus={detail.status}
              stages={detail.stages}
              attempts={detail.attempts}
              reservations={detail.reservations}
              ifMatch={ifMatch}
            />
          </>
        )}

        <Drawer
          open={cancelOpen}
          onClose={() => setCancelOpen(false)}
          title="确认取消任务"
        >
          <p className="field__warning" data-testid="cancel-consequences">
            取消只停止本地的后续推进：<strong>已提交给供应商的付费操作不会被撤销</strong>
            （不保证供应商撤单）；已提交阶段保留查询与账务记录，未决预留不自动释放。
          </p>
          <p className="field__hint">
            取消后本任务不再发起任何新的付费步骤；已有的成果与账务记录保留可查。
          </p>
          <div className="reconcile-panel__submit">
            <button
              type="button"
              className="button-danger"
              data-testid="cancel-confirm"
              disabled={cancelMutation.isPending}
              onClick={() => void cancel()}
            >
              {cancelMutation.isPending ? "正在取消…" : "确认取消（不撤销远端付费）"}
            </button>
            <button type="button" onClick={() => setCancelOpen(false)}>
              返回
            </button>
          </div>
        </Drawer>
      </section>
    </PageLayout>
  );
}

/** 供 UI-031 文案核对复用的导出（阶段缺项提示的界面侧补充）。 */
export { retryDeniedHint };
