/**
 * 阶段明细与**按阶段的恢复入口**（PRD §6.2 UI-031、UI-034、UI-035、UI-037、UI-038）。
 *
 * 界面规则：
 * - 逐个展示阶段种类/批次、状态、尝试次数、下次运行时间、错误摘要与结果产物；
 *   **不使用百分比或预计剩余时间**（§6.3.2）；
 * - `needs_input`：列出服务端给出的可行动缺项（`needsInput[]`），每条按稳定 `code`
 *   指向可补齐的向导步骤；是否可重试**只看服务端 `retry` 字段**（T15 P3①：
 *   被 `budgetNotHolding` 拒绝时不得渲染重试按钮，改为显示原因与"重新报价/新建任务"）；
 * - `submission_unknown`：**不渲染重试**，渲染对账面板（三种动作；同步链路不提供
 *   `attachRemoteTask`——依据服务端 `submissionStyle`）；
 * - 重试使用 `If-Match` + 每次操作一个 `Idempotency-Key`（重放不产生第二个 attempt）。
 */

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";

import { describeError, isApiError } from "../../api/client";
import type {
  JobAttemptDto,
  JobStageDto,
  ReconcileRequestDto,
  ReservationDto,
} from "../../api/endpoints";
import { ConflictNotice } from "../../components/ConflictNotice";
import { readReason } from "../../components/form";
import { useNotify } from "../../components/notifications";
import { Drawer } from "../shell/Drawer";
import { jobKeys, readCurrentRevision, useRetryJob, useReconcileJob } from "./jobs";
import { formatLocalDateTime } from "../../lib/format";
import { CREDIT_MINOR_SCALE, USD_MICROS_SCALE, minorToInputString, parseMinorInput } from "../import/money";
import { retryDeniedHint, stageKindLabel, stageStatusLabel } from "./status";

export interface JobStageListProps {
  readonly itemId: string;
  readonly jobId: string;
  readonly jobStatus: string;
  readonly stages: readonly JobStageDto[];
  readonly attempts: readonly JobAttemptDto[];
  readonly reservations: readonly ReservationDto[];
  /** 任务详情 GET 的 ETag（缺它不能做 If-Match 动作：显示为不可用原因）。 */
  readonly ifMatch: string | null;
}

/** 幂等键随机段（`crypto.randomUUID` 不可用时退化为时间戳；只用于本次操作去重）。 */
function randomKey(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

/** 缺项代码 → 可补齐的向导步骤（界面侧映射；不改变服务端判定）。 */
function missingAction(code: string, itemId: string): { href: string; label: string } | null {
  const base = `/items/${itemId}`;
  if (code.includes("front") || code.includes("view") || code.includes("photo")) {
    return { href: `${base}/import/views`, label: "去补齐照片视图" };
  }
  if (code.includes("page") || code.includes("preparation")) {
    return { href: `${base}/import/prepare`, label: "去检查 PDF 准备" };
  }
  if (code.includes("document")) {
    return { href: `${base}/import/document`, label: "去检查说明书原件" };
  }
  if (code.includes("budget") || code.includes("price")) {
    return { href: "/settings", label: "查看服务与价格配置状态" };
  }
  return { href: base, label: "打开物品" };
}

export function JobStageList(props: JobStageListProps) {
  const { stages } = props;
  return (
    <section className="job-stages" aria-labelledby="job-stages-title" data-testid="job-stages">
      <h2 id="job-stages-title" className="summary-panel__title">
        阶段明细
      </h2>
      <p className="field__hint">
        阶段按执行顺序列出（批次阶段各自一行）：这里不显示线性百分比——各阶段的状态与
        结果才是可核对的事实。
      </p>
      <ol className="job-stages__list">
        {stages.map((stage) => (
          <StageRow key={stage.id} stage={stage} {...props} />
        ))}
      </ol>
    </section>
  );
}

function StageRow({
  stage,
  itemId,
  jobId,
  jobStatus,
  attempts,
  reservations,
  ifMatch,
}: JobStageListProps & { readonly stage: JobStageDto }) {
  const notify = useNotify();
  const queryClient = useQueryClient();
  const retryMutation = useRetryJob();
  const [retryError, setRetryError] = useState<string | null>(null);
  const [conflictRevision, setConflictRevision] = useState<number | null>(null);
  const attemptsForStage = attempts.filter((attempt) => attempt.stageId === stage.id);

  async function retry(): Promise<void> {
    if (ifMatch === null) {
      return;
    }
    setRetryError(null);
    setConflictRevision(null);
    try {
      const result = await retryMutation.mutateAsync({
        jobId,
        stageId: stage.id,
        ifMatch,
        // 一次用户操作一个幂等键；重复点击被按钮禁用拦住，正确性仍由服务端幂等保证。
        idempotencyKey: `retry-${stage.id}-${randomKey()}`,
      });
      notify(
        `已重新排队阶段「${stageKindLabel(stage.stageKind, stage.batchIndex)}」：` +
          `已完成部分不会被覆盖（一并重排的已完成下游阶段：${result.requeuedDependents}）。`,
      );
    } catch (error) {
      if (isApiError(error) && error.status === 412) {
        // UI-008：显示当前版本，刷新后按最新版本重试，不自动覆盖。
        setConflictRevision(readCurrentRevision(error.details));
        return;
      }
      const info = describeError(error);
      setRetryError(info.message);
      notify(`重试被拒绝：${info.message}`, { kind: "alert", requestId: info.requestId });
    }
  }

  const unknown = stage.status === "submission_unknown";
  const blocked = stage.status === "needs_input" || stage.status === "failed";

  return (
    <li className="job-stage" data-testid="job-stage" data-stage-kind={stage.stageKind}>
      <div className="job-stage__head">
        <h3 className="job-stage__title">
          {stageKindLabel(stage.stageKind, stage.batchIndex)}
          <span className="job-stage__status" data-testid="job-stage-status">
            {stageStatusLabel(stage.status)}
          </span>
        </h3>
        <p className="job-stage__meta">
          {stage.pageSet !== null && stage.pageSet !== undefined && stage.pageSet.length > 0 && (
            <>页范围：第 {stage.pageSet.join("、")} 页；</>
          )}
          尝试 {stage.attemptCount} 次
          {stage.pollCount > 0 && <>；已查询 {stage.pollCount} 次</>}
          {stage.nextRunAt != null && <>；下次运行约 {formatLocalDateTime(stage.nextRunAt)}</>}
          {"；更新于 "}
          {formatLocalDateTime(stage.updatedAt)}
        </p>
      </div>

      {stage.lastError !== null && stage.lastError !== undefined && (
        <p className="job-stage__error" role="note" data-testid="job-stage-error">
          最近一次错误摘要：{stage.lastError}
        </p>
      )}

      {stage.stageKind === "manual_extract" && stage.knowledgeProduced === false && (
        <p className="job-stage__note">
          该批未产出正式知识（拒答/截断/格式错/校验失败）：不自动重试、不重复付费。
        </p>
      )}

      {stage.needsInput.length > 0 && (
        <ul className="missing-list__items" role="alert" data-testid="job-stage-missing">
          {stage.needsInput.map((item) => {
            const action = missingAction(item.code, itemId);
            return (
              <li key={`${stage.id}-${item.code}`}>
                <span className="missing-list__message">{item.message}</span>
                {action !== null && <Link to={action.href}>{action.label}</Link>}
              </li>
            );
          })}
        </ul>
      )}

      {conflictRevision !== null && (
        <ConflictNotice
          currentRevision={conflictRevision}
          refreshing={false}
          onRefresh={() => {
            setConflictRevision(null);
            void queryClient.invalidateQueries({ queryKey: jobKeys.detail(jobId) });
          }}
        />
      )}

      {blocked && <RetryEntry stage={stage} ifMatch={ifMatch} retrying={retryMutation.isPending} error={retryError} onRetry={retry} />}

      {unknown && (
        <ReconcilePanel
          jobId={jobId}
          stage={stage}
          jobStatus={jobStatus}
          attempts={attemptsForStage}
          reservations={reservations}
          ifMatch={ifMatch}
        />
      )}
    </li>
  );
}

/** 重试入口：只在服务端 `retry.allowed=true` 时渲染按钮（否则照实显示原因）。 */
function RetryEntry({
  stage,
  ifMatch,
  retrying,
  error,
  onRetry,
}: {
  readonly stage: JobStageDto;
  readonly ifMatch: string | null;
  readonly retrying: boolean;
  readonly error: string | null;
  readonly onRetry: () => Promise<void>;
}) {
  if (ifMatch === null) {
    return (
      <p className="job-stage__note" data-testid="stage-retry-unavailable">
        暂时无法重试：没有读到任务的当前版本（刷新后重试）。
      </p>
    );
  }
  if (!stage.retry.allowed) {
    return (
      <div className="job-stage__recovery" data-testid="stage-retry-denied">
        <p className="job-stage__note">该阶段当前不提供重试入口。</p>
        <p className="field__warning" data-testid="stage-retry-reason">
          {stage.retry.message ?? retryDeniedHint(stage.retry.reason ?? null)}
        </p>
        <p className="field__hint">{retryDeniedHint(stage.retry.reason ?? null)}</p>
      </div>
    );
  }
  return (
    <div className="job-stage__recovery">
      <button
        type="button"
        className="button-secondary"
        data-testid="stage-retry-button"
        disabled={retrying}
        aria-busy={retrying}
        onClick={() => void onRetry()}
      >
        {retrying ? "正在重新排队…" : "重试该阶段"}
      </button>
      <p className="field__hint">
        只重跑该阶段：已完成的其他阶段成果保留，不改变模型/质量预设；重试需要该分支仍有预算背书。
      </p>
      {error !== null && (
        <p className="field__error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

/**
 * 对账面板（UI-034/UI-035）：只对 `submission_unknown` 渲染。
 *
 * - 三个动作的后果必须写明（不保证远端撤单、可能重复收费、admin 声明不等于供应商证明）；
 * - `attachRemoteTask` 只在服务端 `submissionStyle === "asyncRemoteTask"` 时提供；
 * - `authorizeReplacement` 走二次确认对话框（危险动作）。
 */
function ReconcilePanel({
  jobId,
  stage,
  jobStatus,
  attempts,
  reservations,
  ifMatch,
}: {
  readonly jobId: string;
  readonly stage: JobStageDto;
  readonly jobStatus: string;
  readonly attempts: readonly JobAttemptDto[];
  readonly reservations: readonly ReservationDto[];
  readonly ifMatch: string | null;
}) {
  const notify = useNotify();
  const reconcileMutation = useReconcileJob();
  const latestAttempt = attempts[attempts.length - 1] ?? null;
  const attachAvailable = stage.submissionStyle === "asyncRemoteTask";
  const actions: Array<{ value: "recordNoTask" | "attachRemoteTask" | "authorizeReplacement"; label: string; hint: string }> = [
    {
      value: "recordNoTask",
      label: "记录「账户中未找到该任务」",
      hint: "需要填写核查证据；这是你的声明，本应用不会代为伪造供应商「不存在」证明；未决预留不自动释放。",
    },
    ...(attachAvailable
      ? [
          {
            value: "attachRemoteTask" as const,
            label: "附加账户中查到的远端任务",
            hint: "服务端会用当前 Tripo 凭据查询验证该任务可访问；验证失败不修改任何记录（不猜测类型）。",
          },
        ]
      : []),
    {
      value: "authorizeReplacement",
      label: "授权替代提交（可能重复收费）",
      hint: "会产生新的付费请求；旧提交的未决账务与预留保留。需要再次预算确认。",
    },
  ];
  const queryClient = useQueryClient();
  const [action, setAction] = useState(actions[0]?.value ?? "recordNoTask");
  const [remoteTaskId, setRemoteTaskId] = useState("");
  const [evidence, setEvidence] = useState("");
  const [acknowledgeMatches, setAcknowledgeMatches] = useState(false);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [conflictRevision, setConflictRevision] = useState<number | null>(null);

  const branchProvider = stage.stageKind === "manual_extract" ? "manual_ai" : "tripo";
  const reservation = reservations.find((entry) => entry.provider === branchProvider) ?? null;
  const upperMinor = reservation?.reservedMinor ?? 0;
  const scale = branchProvider === "tripo" ? CREDIT_MINOR_SCALE : USD_MICROS_SCALE;
  const [limitInput, setLimitInput] = useState(() => minorToInputString(upperMinor, scale));
  const limitParsed = parseMinorInput(limitInput, scale);

  async function submit(action_: "recordNoTask" | "attachRemoteTask" | "authorizeReplacement"): Promise<void> {
    if (ifMatch === null) {
      return;
    }
    setError(null);
    setConflictRevision(null);
    try {
      const authorizedMinor = limitParsed.ok ? limitParsed.minor : upperMinor;
      let request: ReconcileRequestDto;
      if (action_ === "recordNoTask") {
        request = { action: "recordNoTask", stageId: stage.id, evidence };
      } else if (action_ === "attachRemoteTask") {
        request = {
          action: "attachRemoteTask",
          stageId: stage.id,
          remoteTaskId,
          acknowledgeMatches: true,
        };
      } else {
        request = {
          action: "authorizeReplacement",
          stageId: stage.id,
          acknowledgeDuplicateRisk: true,
          limits:
            branchProvider === "tripo"
              ? { tripoCreditMinor: authorizedMinor }
              : { manualAiUsdMicros: authorizedMinor },
        };
      }
      const result = await reconcileMutation.mutateAsync({ jobId, request, ifMatch });
      notify(`对账动作已完成：${result.notice}`);
      setConfirmOpen(false);
    } catch (error_) {
      setConfirmOpen(false);
      if (isApiError(error_) && error_.status === 412) {
        // UI-008：显示服务端当前版本，刷新后按最新版本重新提交，不自动覆盖。
        setConflictRevision(readCurrentRevision(error_.details));
        return;
      }
      const info = describeError(error_);
      const reason = isApiError(error_) ? readReason(error_.details) : null;
      setError(reason === null ? info.message : `${info.message}（原因：${reason}）`);
    }
  }

  const disabled = ifMatch === null || reconcileMutation.isPending;

  return (
    <div className="reconcile-panel" data-testid="reconcile-panel">
      <h4>付费提交结果未知：需要先对账</h4>
      <p className="field__warning">
        该分支的后续购买已暂停；预留保留为未决（等待对账），不会自动释放。
      </p>
      {latestAttempt !== null && (
        <p className="field__hint" data-testid="reconcile-attempt">
          最近一次提交：{latestAttempt.submitState}
          {latestAttempt.remoteTaskId !== null && <>；远端任务 ID <code>{latestAttempt.remoteTaskId}</code></>}
          {latestAttempt.responseId !== null && <>；同步响应 ID <code>{latestAttempt.responseId}</code></>}
          ；提交于 {formatLocalDateTime(latestAttempt.startedAt)}
          {latestAttempt.lastError !== null && <>；错误摘要：{latestAttempt.lastError}</>}
        </p>
      )}
      {jobStatus === "cancelled" && (
        <p className="field__hint">
          任务已取消：对账只记录事实与账务（attachRemoteTask/recordNoTask），不再自动推进；
          替代提交会被拒绝。
        </p>
      )}

      <fieldset className="reconcile-panel__actions">
        <legend>选择对账动作</legend>
        {actions.map((option) => (
          <div key={option.value} className="checkbox-row">
            <input
              type="radio"
              id={`reconcile-${stage.id}-${option.value}`}
              name={`reconcile-${stage.id}`}
              value={option.value}
              checked={action === option.value}
              disabled={disabled}
              onChange={() => {
                setAction(option.value);
                setError(null);
              }}
            />
            <label htmlFor={`reconcile-${stage.id}-${option.value}`}>{option.label}</label>
            <p className="field__hint">{option.hint}</p>
          </div>
        ))}
      </fieldset>

      {action === "recordNoTask" && (
        <div className="field">
          <label className="field__label" htmlFor={`evidence-${stage.id}`}>
            核查证据（必填）
          </label>
          <textarea
            id={`evidence-${stage.id}`}
            className="field__input"
            rows={3}
            value={evidence}
            disabled={disabled}
            aria-describedby={`evidence-hint-${stage.id}`}
            onChange={(event) => setEvidence(event.target.value)}
          />
          <p className="field__hint" id={`evidence-hint-${stage.id}`}>
            例如查询时间、账户内任务列表的核对结果。这是你的核查声明，不是供应商出具的证明。
          </p>
        </div>
      )}

      {action === "attachRemoteTask" && (
        <div className="field">
          <label className="field__label" htmlFor={`remote-task-${stage.id}`}>
            远端任务 ID（account 中查到，原样填写）
          </label>
          <input
            id={`remote-task-${stage.id}`}
            className="field__input"
            type="text"
            value={remoteTaskId}
            disabled={disabled}
            onChange={(event) => setRemoteTaskId(event.target.value)}
          />
          <div className="checkbox-row">
            <input
              id={`ack-${stage.id}`}
              type="checkbox"
              checked={acknowledgeMatches}
              disabled={disabled}
              onChange={(event) => setAcknowledgeMatches(event.target.checked)}
            />
            <label htmlFor={`ack-${stage.id}`}>我确认该任务与本次未决提交对应（二次确认）</label>
          </div>
          <p className="field__hint">
            服务端会查询验证该 ID 能否被当前账户访问；验证失败会明确失败且不修改任何记录。
          </p>
        </div>
      )}

      {action === "authorizeReplacement" && (
        <div className="field">
          <label className="field__label" htmlFor={`limit-${stage.id}`}>
            再次预算确认（{branchProvider === "tripo" ? "Tripo credits" : "说明书 AI USD"}，必须覆盖冻结上界）
          </label>
          <input
            id={`limit-${stage.id}`}
            className="field__input"
            type="text"
            inputMode="decimal"
            value={limitInput}
            disabled={disabled}
            aria-invalid={limitParsed.ok ? undefined : true}
            onChange={(event) => setLimitInput(event.target.value)}
          />
          {!limitParsed.ok && <p className="field__error">{limitParsed.message}</p>}
          <p className="field__hint">
            冻结的保守上界：{reservation?.reservedDisplay ?? "（未读到预留）"}；低于上界会被服务端拒绝。
          </p>
        </div>
      )}

      {conflictRevision !== null && (
        <ConflictNotice
          currentRevision={conflictRevision}
          refreshing={false}
          description="该任务已被其他操作更新：对账使用 If-Match（任务版本）；你填写的对账内容会保留。"
          onRefresh={() => {
            setConflictRevision(null);
            void queryClient.invalidateQueries({ queryKey: jobKeys.detail(jobId) });
          }}
        />
      )}

      {error !== null && (
        <p className="field__error" role="alert" data-testid="reconcile-error">
          {error}
        </p>
      )}

      <div className="reconcile-panel__submit">
        {action === "authorizeReplacement" ? (
          <button
            type="button"
            className="button-danger"
            data-testid="reconcile-submit"
            disabled={disabled}
            aria-busy={reconcileMutation.isPending}
            onClick={() => setConfirmOpen(true)}
          >
            授权替代提交…
          </button>
        ) : (
          <button
            type="button"
            className="button-primary"
            data-testid="reconcile-submit"
            disabled={disabled || (action === "recordNoTask" && evidence.trim() === "") || (action === "attachRemoteTask" && (remoteTaskId.trim() === "" || !acknowledgeMatches))}
            aria-busy={reconcileMutation.isPending}
            onClick={() => void submit(action)}
          >
            {reconcileMutation.isPending ? "提交中…" : "提交对账动作"}
          </button>
        )}
      </div>

      <Drawer open={confirmOpen} onClose={() => setConfirmOpen(false)} title="确认授权替代提交">
        <p className="field__warning">
          这会创建<strong>新的付费请求</strong>：旧提交结果未知，可能产生<strong>重复收费</strong>；
          旧 attempt 的未决账务与预留会保留，不自动释放。
        </p>
        <p className="field__hint">
          本次授权上限：{limitParsed.ok ? limitParsed.minor : "（数值无效）"}
          （{branchProvider === "tripo" ? "credits 最小单位" : "USD 最小单位"}）。
        </p>
        <div className="reconcile-panel__submit">
          <button
            type="button"
            className="button-danger"
            data-testid="reconcile-confirm-replacement"
            disabled={disabled || !limitParsed.ok}
            onClick={() => void submit("authorizeReplacement")}
          >
            确认授权（承担重复收费风险）
          </button>
          <button type="button" onClick={() => setConfirmOpen(false)}>
            返回
          </button>
        </div>
      </Drawer>
    </div>
  );
}
