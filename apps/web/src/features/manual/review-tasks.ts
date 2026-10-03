import type { PublishIssueDto } from "../../api/endpoints";
import { isEntityReviewed, type DraftStep } from "../viewer/draft-view";
import type { PublishChecklistInput } from "./review-state";

export type ReviewTaskGroup = "facts" | "model" | "geometry";
export interface ReviewTask {
  readonly key: string;
  readonly group: ReviewTaskGroup;
  readonly title: string;
  readonly problem: string;
  readonly done: boolean;
  readonly required: boolean;
  readonly entityId: string | null;
  readonly entityKind: "part" | "step" | "spec" | "model";
  readonly action: "fact" | "modelLoaded" | "modelConfirmed" | "binding" | "rebind" | "pose";
  readonly hotspotId?: string;
}
export const REVIEW_GROUPS: readonly { id: ReviewTaskGroup; label: string }[] = [
  { id: "facts", label: "文字事实" }, { id: "model", label: "模型核对" }, { id: "geometry", label: "热点与视角" },
];

/** Task rows explain existing release rules; optional poses/stale leftovers add no publish requirement. */
export function reviewTasks(input: PublishChecklistInput, poses: Readonly<Record<string, unknown>>): ReviewTask[] {
  const tasks: ReviewTask[] = [];
  const entities = [
    ...input.parts.map(part => ({ id: part.id, title: part.name, kind: "part" as const })),
    ...input.steps.map(step => ({ id: step.id, title: step.title, kind: "step" as const })),
    ...input.specs.map(spec => ({ id: spec.id, title: spec.label, kind: "spec" as const })),
  ];
  for (const entity of entities) {
    const reviewed = input.entityReviews[entity.id];
    const done = isEntityReviewed(reviewed);
    tasks.push({ key: `fact-${entity.id}`, group: "facts", title: entity.title, entityId: entity.id, entityKind: entity.kind,
      action: "fact", done, required: true,
      problem: done ? reviewed?.userEdited != null ? "已保存人工修订" : "已确认事实" : "尚未确认或修订文字事实" });
  }
  const model = input.model;
  const matches = model !== null && input.modelReview?.modelRevisionId === model.revisionId && input.modelReview.modelSha256 === model.sha256;
  for (const [action, label, done] of [
    ["modelLoaded", "已在浏览器成功打开此模型", matches && input.modelReview?.loaded === true],
    ["modelConfirmed", "我已核对模型与资料一致", matches && input.modelReview?.userConfirmed === true],
  ] as const) {
    tasks.push({ key: action, group: "model", title: "当前模型", entityId: model?.revisionId ?? null, entityKind: "model", action,
      required: true, done, problem: model === null ? "当前没有可用模型，需先完成模型生成" : `${done ? "已声明" : "待声明"}：${label}` });
  }
  for (const part of input.parts) {
    const existing = input.hotspots.filter(h => h.partId === part.id);
    const stale = existing.filter(h => h.status === "stale" || (!h.usable && h.anchor !== null));
    const done = input.entityReviews[part.id]?.textOnly === true || existing.some(h => h.status === "confirmed" && h.usable);
    const first = !done ? stale[0] : undefined;
    tasks.push({ key: `binding-${part.id}`, group: "geometry", title: part.name, entityId: part.id, entityKind: "part", required: true, done,
      action: first ? "rebind" : "binding", ...(first ? { hotspotId: first.id } : {}),
      problem: done ? input.entityReviews[part.id]?.textOnly ? "已明确标记为仅文本条目" : "已有当前模型的有效热点" : first ? "热点已失效，需要在当前模型上重新绑定" : "缺少当前模型的有效热点" });
    for (const hotspot of existing.filter(h => h.id !== first?.id && h.status !== "unbound")) {
      const required = !hotspot.usable && hotspot.status !== "stale"; // matches the server's requires_anchor rule
      tasks.push({ key: `rebind-${hotspot.id}`, group: "geometry", title: part.name, entityId: part.id, entityKind: "part", hotspotId: hotspot.id,
        action: "rebind", done: hotspot.usable, required, problem: hotspot.usable ? "绑定与当前模型一致" : required ? "热点声明与模型不一致，需要重新绑定" : "旧绑定已失效（可选处理，不作为有效热点）" });
    }
  }
  for (const step of input.steps as readonly DraftStep[]) tasks.push({ key: `pose-${step.id}`, group: "geometry", title: step.title,
    entityId: step.id, entityKind: "step", action: "pose", required: false, done: poses[step.id] !== undefined,
    problem: poses[step.id] !== undefined ? "已保存步骤视角（可选）" : "尚未设置步骤视角（可选，不影响发布）" });
  return tasks;
}

/** Reject unknown codes/identities. Do not turn a server string into a DOM selector or arbitrary URL. */
export function taskForPublishIssue(issue: PublishIssueDto, tasks: readonly ReviewTask[]): ReviewTask | null {
  if (!issue.entityId) return null;
  if (["knowledgeUnreviewed", "evidencePageMissing", "stepPartReferenceMissing"].includes(issue.code)) {
    return tasks.find(task => task.action === "fact" && task.entityId === issue.entityId && task.entityKind === issue.entityKind) ?? null;
  }
  if (["modelReviewMissing", "modelReviewIncomplete", "modelReviewModelMismatch"].includes(issue.code) && issue.entityKind === "model") {
    const matching = tasks.filter(task => task.group === "model" && task.entityId === issue.entityId);
    return matching.find(task => !task.done) ?? matching[0] ?? null;
  }
  if (issue.code === "hotspotMissing" && issue.entityKind === "part") return tasks.find(task => task.key === `binding-${issue.entityId}`) ?? null;
  if (issue.code === "hotspotNotMatchingModel" && issue.entityKind === "hotspot") return tasks.find(task => task.hotspotId === issue.entityId) ?? null;
  return null;
}

export function taskDestination(task: ReviewTask, narrow: boolean) {
  const geometry = task.action === "binding" || task.action === "rebind" || task.action === "pose";
  const panelId = task.action === "binding" || task.action === "rebind" ? "parts" : "steps";
  const focusId = geometry && narrow ? `geometry-note-${task.action === "pose" ? task.entityId : (task.done ? task.entityId : task.hotspotId ?? task.entityId)}`
    : task.action === "fact" ? `review-fact-${task.entityId}`
    : task.action === "binding" ? task.done ? `part-select-${task.entityId}` : `bind-${task.entityId}`
    : task.action === "rebind" ? task.done ? `part-select-${task.entityId}` : `rebind-${task.hotspotId}`
    : task.action === "pose" ? `pose-${task.entityId}` : `review-${task.action}`;
  return { panelId, focusId };
}

export function nextReviewTask(tasks: readonly ReviewTask[], currentKey: string): ReviewTask | null {
  const index = tasks.findIndex(task => task.key === currentKey);
  if (index < 0) return null;
  return [...tasks.slice(index + 1), ...tasks.slice(0, index)].find(task => !task.done) ?? null;
}
