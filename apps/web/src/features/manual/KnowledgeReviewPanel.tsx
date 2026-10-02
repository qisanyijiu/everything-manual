/**
 * 知识确认与修订面板（T19；PRD UI-051/UI-052/UI-053；REQ-034）。
 *
 * **「事实确认」与「几何校准」分开**（§6.3.2）：
 * - 本面板只做**文字事实**的确认/修订（实体级 `confirmed`/`needs_review`、
 *   人工修订 `userEdited`、modelReview 的两个**用户声明**）；
 * - 热点绑定与步骤视角在部件列表/步骤面板（几何校准），不在这里；
 * - 文案不使用无定语的"确认"：动作用「确认事实」「取消确认」，
 *   modelReview 用「已在浏览器成功打开此模型」「我已核对模型与资料一致」。
 *
 * **供应商事实快照只读**（UI-051）：AI 提取的原文没有输入控件，只提供
 * 「复制为本地修订」；人工修订进入覆盖层并保留原文本对照与出处。
 *
 * **modelReview 是用户声明**（contracts §2）：`checkedAt` 由服务器赋值；
 * 界面注明"这是你的复核声明，不是服务端 GPU 测试结论"；换模型后记录被清空。
 */


import type { EntityReviewView, ModelReviewView } from "../viewer/draft-view";
import type { ModelIdentity } from "./review-state";

interface PartLike {
  readonly id: string;
  readonly name: string;
  readonly description: string;
  readonly evidence: readonly { pageNumber: number; quote: string | null }[];
}
interface StepLike {
  readonly id: string;
  readonly title: string;
  readonly orderedActions: readonly string[];
  readonly evidence: readonly { pageNumber: number; quote: string | null }[];
}
interface SpecLike {
  readonly id: string;
  readonly label: string;
  readonly value: string;
  readonly evidence: readonly { pageNumber: number; quote: string | null }[];
}

export type EditValues = { name: string; description: string; title: string; actions: string; label: string; value: string };
export type EditFields = { name?: string; description?: string; title?: string; orderedActions?: string[]; label?: string; value?: string };

export interface KnowledgeReviewPanelProps {
  readonly editing: string | null;
  readonly buffers: Readonly<Record<string, EditValues>>;
  readonly busy: boolean;
  readonly saveStatus?: string;
  readonly onEditing: (id: string | null) => void;
  readonly onBuffer: (id: string, values: EditValues | null) => void;
  readonly parts: readonly PartLike[];
  readonly steps: readonly StepLike[];
  readonly specs: readonly SpecLike[];
  readonly entityReviews: Readonly<Record<string, EntityReviewView>>;
  readonly modelReview: ModelReviewView | null;
  readonly model: ModelIdentity | null;
  /** 本会话内模型是否真的在浏览器打开过（loaded 按钮的门槛）。 */
  readonly modelLoaded: boolean;
  readonly narrow: boolean;
  readonly onDeclareModelReady: () => void;
  readonly onDeclareModelConfirmed: () => void;
  readonly onSetEntityReview: (entityId: string, decision: "confirmed" | "needs_review") => void;
  readonly onSaveEntityEdit: (
    entityId: string,
    fields: {
      name?: string;
      description?: string;
      title?: string;
      orderedActions?: string[];
      label?: string;
      value?: string;
    },
  ) => Promise<boolean>;
}

type Kind = "part" | "step" | "spec";

export function KnowledgeReviewPanel({
  parts,
  steps,
  specs,
  entityReviews,
  modelReview,
  model,
  modelLoaded,
  narrow,
  onDeclareModelReady,
  onDeclareModelConfirmed,
  onSetEntityReview,
  onSaveEntityEdit, editing, buffers, busy, saveStatus, onEditing, onBuffer,
}: KnowledgeReviewPanelProps) {
  const entries: { kind: Kind; entity: PartLike | StepLike | SpecLike }[] = [
    ...parts.map((part) => ({ kind: "part" as const, entity: part })),
    ...steps.map((step) => ({ kind: "step" as const, entity: step })),
    ...specs.map((spec) => ({ kind: "spec" as const, entity: spec })),
  ];
  const loadedDeclared = modelReview !== null && matchesModel(modelReview, model) && modelReview.loaded === true;
  const confirmedDeclared = modelReview !== null && matchesModel(modelReview, model) && modelReview.userConfirmed === true;

  return (
    <section aria-label="知识确认与修订" data-testid="knowledge-panel">
      <h2>文字事实确认与修订</h2>
      <p className="page-note">
        这里只处理文字事实（AI 提取内容）。热点绑定与步骤视角属于几何校准，在部件列表与步骤面板中完成。
      </p>
      <section aria-label="模型复核" className="notice-panel" data-testid="model-review-panel">
        <h3>模型复核（两个独立声明）</h3>
        <p>
          这是你的复核声明，不是服务端 GPU 测试结论；两个动作都记录声明时间，换模型后清空。
        </p>
        <div className="row-actions">
          <button
            type="button"
            id="review-modelLoaded"
            disabled={busy || model === null || loadedDeclared || !modelLoaded}
            title={model === null ? "草稿没有可用模型" : undefined}
            onClick={onDeclareModelReady}
          >
            已在浏览器成功打开此模型
          </button>
          <button
            type="button"
            id="review-modelConfirmed"
            disabled={busy || !loadedDeclared || confirmedDeclared}
            onClick={onDeclareModelConfirmed}
          >
            我已核对模型与资料一致
          </button>
        </div>
        <ul className="entity-list" data-testid="model-review-state">
          <li>
            打开模型：{loadedDeclared ? `已声明（${formatTime(modelReview?.loadedAt ?? null)}）` : "未声明"}
          </li>
          <li>
            核对一致：{confirmedDeclared ? `已声明（${formatTime(modelReview?.userConfirmedAt ?? null)}）` : "未声明"}
          </li>
          {modelReview !== null && !matchesModel(modelReview, model) && (
            <li role="alert">该声明属于旧模型版本：换模型后需要重新打开并重新核对。</li>
          )}
          {modelReview !== null && model !== null && matchesModel(modelReview, model) && (
            <li>声明绑定模型：{modelReview.modelRevisionId.slice(0, 8)}…（哈希与当前模型一致）</li>
          )}
        </ul>
      </section>

      <ul className="entity-list" data-testid="knowledge-entities">
        {entries.map(({ kind, entity }) => {
          const review = entityReviews[entity.id];
          const reviewed = review !== undefined && (review.reviewStatus === "confirmed" || review.userEdited !== null);
          const original = originalText(kind, entity);
          const edited = review?.userEdited ?? null;
          return (
            <li key={entity.id} id={`review-entity-${entity.id}`} tabIndex={-1} data-testid={`knowledge-${entity.id}`}>
              <div className="knowledge-head">
                <span className="status-label" data-testid={`knowledge-status-${entity.id}`}>
                  {review?.userEdited != null
                    ? `已修订（人工${review.editedAt !== null ? `，${formatTime(review.editedAt)}` : ""}）`
                    : review?.reviewStatus === "confirmed"
                      ? "已确认（人工）"
                      : "待复核"}
                </span>
                {kind === "part" && review?.textOnly === true && (
                  <span className="status-label">仅文本条目（发布时保留并标识）</span>
                )}
              </div>
              <p className="original-text">{original}</p>
              {edited !== null && (
                <p className="edited-text" data-testid={`knowledge-edited-${entity.id}`}>
                  人工修订：{editedText(kind, edited)}
                </p>
              )}
              {evidenceLine(entity.evidence) !== null && (
                <p className="page-note">
                  {evidenceLine(entity.evidence)}
                  {evidenceLine(entity.evidence)!.includes("bbox") ? "" : ""}
                </p>
              )}
              <div className="row-actions">
                <button
                  type="button"
                  id={`review-fact-${entity.id}`}
                  disabled={busy || (reviewed && review?.reviewStatus === "confirmed")}
                  onClick={() => onSetEntityReview(entity.id, "confirmed")}
                >
                  确认事实
                </button>
                <button
                  type="button"
                  disabled={busy || review?.reviewStatus !== "confirmed"}
                  onClick={() => onSetEntityReview(entity.id, "needs_review")}
                >
                  取消确认
                </button>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => onEditing(editing === entity.id ? null : entity.id)}
                >
                  {editing === entity.id ? "收起修订" : "复制为本地修订"}
                </button>
              </div>
              {editing === entity.id && (
                <EditForm
                  kind={kind}
                  entity={entity}
                  values={buffers[entity.id]}
                  busy={busy} status={saveStatus}
                  onChange={(values) => onBuffer(entity.id, values)}
                  onCancel={() => { onBuffer(entity.id, null); onEditing(null); }}
                  onSave={async (fields) => {
                    if (await onSaveEntityEdit(entity.id, fields)) { onBuffer(entity.id, null); onEditing(null); }
                  }}
                />
              )}
            </li>
          );
        })}
      </ul>
      {narrow && (
        <p className="page-note">窄屏说明：文字事实确认与发布在窄屏仍可用（几何校准除外）。</p>
      )}
    </section>
  );
}

function matchesModel(review: ModelReviewView, model: ModelIdentity | null): boolean {
  if (model === null) {
    return false;
  }
  return review.modelRevisionId === model.revisionId && review.modelSha256 === model.sha256;
}

function originalText(kind: Kind, entity: PartLike | StepLike | SpecLike): string {
  if (kind === "part") {
    const part = entity as PartLike;
    return `${part.name}：${part.description}`;
  }
  if (kind === "step") {
    const step = entity as StepLike;
    return `${step.title}：${step.orderedActions.join(" → ")}`;
  }
  const spec = entity as SpecLike;
  return `${spec.label}：${spec.value}`;
}

function editedText(
  kind: Kind,
  edited: NonNullable<EntityReviewView["userEdited"]>,
): string {
  if (kind === "part") {
    return `${edited.name ?? ""}${edited.description !== undefined ? `：${edited.description}` : ""}`;
  }
  if (kind === "step") {
    return `${edited.title ?? ""}${
      edited.orderedActions !== undefined ? `：${edited.orderedActions.join(" → ")}` : ""
    }`;
  }
  return `${edited.label ?? ""}${edited.value !== undefined ? `：${edited.value}` : ""}`;
}

/** 出处行（1-based 页码；bbox 无法可靠提供时为 null，界面不绘制猜测框）。 */
function evidenceLine(
  evidence: readonly { pageNumber: number; quote: string | null }[],
): string | null {
  if (evidence.length === 0) {
    return null;
  }
  const pages = evidence.map((entry) => `第 ${entry.pageNumber} 页`).join("、");
  const quote = evidence.find((entry) => entry.quote !== null)?.quote ?? null;
  return `出处：${pages}${quote !== null ? `（引文：${quote}）` : ""}（无 bbox 时只显示页码与文字，不绘制猜测框）`;
}

function formatTime(millis: number | null): string {
  if (millis === null) {
    return "时间未知";
  }
  return new Date(millis).toLocaleString("zh-CN", { hour12: false });
}

/** 人工修订表单（只允许该实体类型支持的字段；服务端再次校验）。 */
function EditForm({
  kind,
  entity,
  onSave, onCancel, values, onChange, busy, status,
}: {
  values: EditValues | undefined;
  status?: string;
  busy: boolean;
  onChange: (values: EditValues) => void;
  kind: Kind;
  entity: PartLike | StepLike | SpecLike;
  onSave: (fields: {
    name?: string;
    description?: string;
    title?: string;
    orderedActions?: string[];
    label?: string;
    value?: string;
  }) => Promise<void>;
  onCancel: () => void;
}) {
  const part = kind === "part" ? (entity as PartLike) : null;
  const step = kind === "step" ? (entity as StepLike) : null;
  const spec = kind === "spec" ? (entity as SpecLike) : null;
  const current: EditValues = values ?? { name: part?.name ?? "", description: part?.description ?? "",
    title: step?.title ?? "", actions: (step?.orderedActions ?? []).join("\n"), label: spec?.label ?? "", value: spec?.value ?? "" };
  const { name, description, title, actions, label, value } = current;
  const change = (field: keyof EditValues, text: string) => onChange({ ...current, [field]: text });

  return (
    <div className="edit-form">
      {part !== null && (
        <>
          <label>
            部件名
            <input disabled={busy} value={name} maxLength={120} onChange={(event) => change("name", event.target.value)} />
          </label>
          <label>
            说明
            <textarea
              disabled={busy}
              value={description}
              maxLength={1200}
              rows={3}
              onChange={(event) => change("description", event.target.value)}
            />
          </label>
        </>
      )}
      {step !== null && (
        <>
          <label>
            步骤标题
            <input disabled={busy} value={title} maxLength={200} onChange={(event) => change("title", event.target.value)} />
          </label>
          <label>
            操作（每行一步）
            <textarea
              disabled={busy}
              value={actions}
              rows={3}
              onChange={(event) => change("actions", event.target.value)}
            />
          </label>
        </>
      )}
      {spec !== null && (
        <>
          <label>
            规格名
            <input disabled={busy} value={label} maxLength={120} onChange={(event) => change("label", event.target.value)} />
          </label>
          <label>
            规格值
            <input disabled={busy} value={value} maxLength={600} onChange={(event) => change("value", event.target.value)} />
          </label>
        </>
      )}
      <p role="status" aria-live="polite">{status ?? (values ? "有未保存修改" : "")}</p>
      <div className="row-actions">
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            if (part !== null) {
              void onSave({ name, description });
            } else if (step !== null) {
              void onSave({
                title,
                orderedActions: actions
                  .split("\n")
                  .map((line) => line.trim())
                  .filter((line) => line !== ""),
              });
            } else {
              void onSave({ label, value });
            }
          }}
        >
          保存人工修订（并确认事实）
        </button>
        <button type="button" disabled={busy} onClick={onCancel}>
          取消
        </button>
      </div>
      <p className="page-note">原文本与出处保留在上方；保存不会修改供应商事实快照。</p>
    </div>
  );
}
