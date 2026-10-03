/**
 * 发布面板（T19；PRD UI-054/UI-055/UI-056；REQ-035）。
 *
 * 交互合同（QA 按此复核）：
 * - **发布是显式动作**：界面不存在任何自动/后台/定时的发布入口（ADR-005）；
 * - **不满足时按钮禁用且原因列表常驻**（UI-054）：左侧清单是服务端发布不变量的
 *   镜像（`review-state.publishChecklist`），每条带"去处理"定位；
 * - **服务端 422 的 `details.issues[]` 逐条呈现**（UI-054）：这是最终判据，
 *   与本地清单同屏对照（两者不一致时以服务端为准，界面不会隐藏服务端明细）；
 * - **412 恢复**（UI-056）：显示当前 revision + 刷新入口，不自动合并、不覆盖；
 * - **成功后常驻不可变提示**（UI-055）：版本号、发布时间、模型 revision、manifest
 *   摘要与「已发布版本不可再修改」，并给「打开阅读器」入口。
 */

import { useState } from "react";
import { Link } from "react-router";
import { useMutation } from "@tanstack/react-query";

import { describeError, isApiError } from "../../api/client";
import { publishDraft, type PublishIssueDto, type ReleaseDto } from "../../api/endpoints";
import { REVIEW_GROUPS, taskForPublishIssue, type ReviewTask } from "./review-tasks";
import type { PublishChecklist } from "./review-state";

export interface PublishPanelProps {
  readonly tasks: readonly ReviewTask[];
  readonly onChooseTask: (task: ReviewTask) => void;
  readonly busy: boolean;
  readonly readSerial: number;
  readonly checklist: PublishChecklist;
  readonly etag: string | null;
  readonly itemId: string;
  readonly draftId: string;
  readonly narrow: boolean;
  readonly onPublished: (releaseId: string, replayed: boolean) => void;
  readonly onIssues: (message: string) => void;
  readonly onRefetchRequested: () => Promise<boolean>;
}

interface PublishFailure {
  readonly requestId: string | null;
  readonly readSerial: number;
  readonly kind: "issues" | "conflict" | "error";
  readonly message: string;
  readonly issues: readonly PublishIssueDto[];
  readonly currentRevision: number | null;
}

function readIssues(details: unknown): PublishIssueDto[] {
  if (typeof details !== "object" || details === null) {
    return [];
  }
  const issues = (details as { issues?: unknown }).issues;
  if (!Array.isArray(issues)) {
    return [];
  }
  return issues.filter(
    (issue): issue is PublishIssueDto =>
      typeof issue === "object" &&
      issue !== null &&
      typeof (issue as PublishIssueDto).code === "string" &&
      typeof (issue as PublishIssueDto).message === "string",
  );
}

export function PublishPanel({
  checklist, tasks, onChooseTask, busy, readSerial,
  etag,
  itemId,
  draftId,
  narrow,
  onPublished,
  onIssues,
  onRefetchRequested,
}: PublishPanelProps) {
  const [storedFailure, setFailure] = useState<PublishFailure | null>(null);
  const failure = storedFailure?.kind === "issues" || storedFailure?.readSerial === readSerial ? storedFailure : null;
  const rereadIssues = failure?.kind === "issues" && failure.readSerial !== readSerial;
  const serverBlocked = (failure?.kind === "issues" && !rereadIssues) || failure?.kind === "conflict";
  const [onlyPending, setOnlyPending] = useState(true);
  const [published, setPublished] = useState<ReleaseDto | null>(null);
  const [idempotencyKey, setIdempotencyKey] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: (input: { ifMatch: string; key: string }) =>
      publishDraft(itemId, draftId, input.ifMatch, input.key),
    onSuccess: (result) => {
      setFailure(null);
      setPublished(result.release);
      setIdempotencyKey(null);
      onPublished(result.release.id, result.replayed);
    },
    onError: (error: unknown) => {
      if (isApiError(error) && error.status === 412) {
        const details = error.details as { currentRevision?: unknown } | null;
        const currentRevision =
          typeof details?.currentRevision === "number" ? details.currentRevision : null;
        setFailure({
          kind: "conflict", requestId: error.requestId, readSerial,
          message: `该内容已被其他操作更新（当前 r${currentRevision ?? "?"}）：请刷新后重试`,
          issues: [],
          currentRevision,
        });
        return;
      }
      if (isApiError(error) && error.status === 422) {
        const issues = readIssues(error.details);
        const message =
          issues.length > 0
            ? error.message
            : "发布条件不满足：请按服务端返回的字段明细修正后重试";
        setFailure({ kind: "issues", message: error.message || message, issues, currentRevision: null, requestId: error.requestId, readSerial });
        onIssues(message);
        return;
      }
      setFailure({ kind: "error", requestId: describeError(error).requestId, readSerial, message: describeError(error).message, issues: [], currentRevision: null });
      onIssues(describeError(error).message);
    },
  });

  const pending = !checklist.ready;
  const disabledReason =
    etag === null
      ? "草稿尚未加载完成"
      : pending
        ? "发布条件未满足：请按下方列表逐条处理（服务端仍会再次校验）"
        : null;

  const submit = () => {
    if (etag === null || pending || busy || mutation.isPending || serverBlocked) {
      return;
    }
    // 幂等键一次操作生成并复用：重复点击/断线重试返回同一发布版本。
    const key = idempotencyKey ?? `publish-${draftId}-${Date.now()}`;
    setIdempotencyKey(key);
    mutation.mutate({ ifMatch: etag, key });
  };

  return (
    <section aria-label="发布" className="publish-panel" data-testid="publish-panel">
      <h2 id="publish-heading" tabIndex={-1}>发布不可变版本</h2>
      <p className="page-note">
        发布是显式动作：服务端校验全部发布不变量后才生成不可变版本；界面不存在自动发布路径。
      </p>
      <ul className="entity-list" data-testid="publish-checklist">
        <li>
          未确认知识：{checklist.counts.unreviewed}（部件/步骤/规格各自需要「确认事实」或人工修订）
        </li>
        <li>缺 confirmed 热点部件：{checklist.counts.missingHotspots}</li>
        <li>
          已失效热点：{checklist.counts.staleHotspots}
          {checklist.counts.staleHotspots > 0 && "（需在新模型上重新绑定，不能强制确认）"}
        </li>
        <li>
          模型复核：
          {checklist.modelReview.loaded ? "已声明打开" : "未声明打开"}、
          {checklist.modelReview.userConfirmed ? "已声明核对一致" : "未声明核对一致"}
          {!checklist.modelReview.matches && "（与当前模型不一致）"}
        </li>
        {checklist.counts.textOnlyParts > 0 && (
          <li>仅文本条目部件：{checklist.counts.textOnlyParts}（保留在发布内容并明显标识）</li>
        )}
      </ul>
      <section className="review-tasks" aria-label="发布待办" data-testid="review-tasks">
        <label className="review-filter"><input type="checkbox" checked={onlyPending} onChange={event => setOnlyPending(event.target.checked)} />只看未完成</label>
        {REVIEW_GROUPS.map(group => {
          const entries = tasks.filter(task => task.group === group.id);
          const visible = entries.filter(task => !onlyPending || !task.done);
          return <section key={group.id} aria-labelledby={`review-group-${group.id}`}>
            <h3 id={`review-group-${group.id}`} tabIndex={-1}>{group.label} · 未完成 {entries.filter(task => !task.done).length}</h3>
            {visible.length === 0 ? <p>本组已处理完成。</p> : <ul className="entity-list">
              {visible.map(task => <li key={task.key} data-testid={`todo-${task.key}`}>
                <strong>{task.title}</strong><p>{task.problem}</p>
                {!task.required && <span className="status-label">可选，不影响发布</span>}
                <button type="button" disabled={busy} aria-label={`去处理 · ${task.title} · ${task.problem}`} onClick={() => onChooseTask(task)}>去处理</button>
              </li>)}
            </ul>}
          </section>;
        })}
      </section>
      {pending && <div className="publish-issues" data-testid="publish-pending"><h3>发布条件尚未满足</h3><ul>{checklist.items.map((item,index) => <li key={`${item.code}-${index}`}>{item.label}</li>)}</ul></div>}
      <div className="row-actions">
        <button
          type="button"
          disabled={etag === null || pending || busy || mutation.isPending || serverBlocked}
          title={disabledReason ?? undefined}
          onClick={submit}
        >
          {rereadIssues ? "重新检查并发布（通过后生成不可变版本）" : "发布（生成不可变版本）"}
        </button>
        {narrow && (
          <span className="page-note" data-testid="narrow-publish-note">
            窄屏可以完成文字确认与发布；热点绑定与视角保存需要在 ≥768px 窗口完成。
          </span>
        )}
      </div>

      {failure !== null && failure.kind === "conflict" && (
        <div className="page-error" role="alert" data-testid="publish-conflict">
          <p>{failure.message}</p>
          {failure.requestId !== null && <p>诊断 ID：{failure.requestId}</p>}
          <button type="button" disabled={busy} onClick={() => void onRefetchRequested()}>核对最新版本</button>
        </div>
      )}
      {failure !== null && failure.kind === "issues" && (
        <div className="page-error" role="alert" data-testid="publish-issues">
          <p>{failure.message}</p>
          {rereadIssues && <p role="status">已读取最新草稿，服务端问题尚未确认解决；再次提交仍需服务器校验，通过后才会发布。</p>}
          <ul>
            {failure.issues.map((issue, index) => { const task = taskForPublishIssue(issue, tasks); return (
              <li key={`${issue.code}-${issue.entityId ?? index}`}>
                <span className="status-label">{issue.code}</span> {issue.message}
                {task !== null && <button type="button" disabled={busy} onClick={() => onChooseTask(task)} aria-label={`去处理 · ${task.title} · ${issue.message}`}>去处理</button>}
              </li>
            ); })}
          </ul>
          {failure.requestId !== null && <p>诊断 ID：{failure.requestId}</p>}
          <button type="button" disabled={busy} onClick={() => void onRefetchRequested()}>重新读取检查结果</button>
        </div>
      )}
      {failure !== null && failure.kind === "error" && (
        <div className="page-error" role="alert" data-testid="publish-error">
          <p>{failure.message}</p>
        </div>
      )}

      {published !== null && (
        <div className="notice-panel" data-testid="publish-success">
          <p>
            已发布版本 {published.id}（不可再修改）：之后对草稿的修改不会改变该版本。
          </p>
          <ul>
            <li>来源草稿 revision：r{published.draftRevision}</li>
            <li>模型 revision：{published.modelRevisionId}</li>
            <li>发布时间：{new Date(published.createdAt).toLocaleString("zh-CN", { hour12: false })}</li>
            <li>manifest 资产：{published.manifestAssetId}</li>
          </ul>
          <p>
            <Link to={`/items/${itemId}/releases/${published.id}`}>打开阅读器</Link>
            {" · "}
            <Link to={`/items/${itemId}/releases`}>查看版本列表</Link>
          </p>
        </div>
      )}
    </section>
  );
}
