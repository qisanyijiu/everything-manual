/**
 * 草稿知识的**防御式读取**（T18；T19 在同一形状上做校验与写入）。
 *
 * 为什么单独一层：`DraftDto.knowledge` 在 OpenAPI 里是 `unknown`（版本化 JSON 聚合，
 * schema 归 `manual_draft_v1` 管理），前端不能假设字段一定存在——部分成功的草稿
 * 只有外壳、没有知识分支。读取器对缺字段/类型不符**返回空结果**，不抛错、不猜。
 *
 * 读取字段名与 contracts §2 一致（camelCase）：
 * `model.{revisionId, sha256, assetId}`、`knowledge.{parts, steps}`、
 * 实体上的 `reviewStatus`/`evidence[]`/`orderedActions[]`/`partIds[]`/`safetyNotes[]`、
 * evidence 的 `pageNumber`（1-based）与 `documentId`；
 * hotspot 形态：`hotspots[].{id, partId, status, anchor.{modelRevisionId, modelSha256, positionLocal}}`。
 */

import { isFiniteVec3, type Anchor, type CameraPose, type Vec3 } from "./coordinates";

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : null;
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function asStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

/** 模型分支产物引用（`knowledge.model`；validated 才由服务端写入）。 */
export interface DraftModelRef {
  readonly revisionId: string;
  readonly sha256: string;
  readonly assetId: string;
  readonly validationState: string;
}

/**
 * 读取可用的模型版本。
 *
 * 只接受 `validationState === "validated"`（UI-043「禁用：模型未通过校验时不进入 3D
 * 视图」的读取侧防线；服务端本就不会把 rejected 版本写进草稿，这里再挡一次，
 * 避免将来上游变化时把未通过校验的模型渲染出来）。
 */
export function readDraftModel(knowledge: unknown): DraftModelRef | null {
  const shell = asRecord(knowledge);
  const model = asRecord(shell?.model);
  if (model === null) {
    return null;
  }
  const revisionId = asString(model.revisionId);
  const sha256 = asString(model.sha256);
  const assetId = asString(model.assetId);
  const validationState = asString(model.validationState) ?? "";
  if (revisionId === null || sha256 === null || assetId === null || revisionId === "" || assetId === "") {
    return null;
  }
  if (validationState !== "validated") {
    return null;
  }
  return { revisionId, sha256, assetId, validationState };
}

export interface DraftEvidence {
  readonly pageNumber: number;
  readonly documentId: string | null;
  readonly quote: string | null;
}

export interface DraftPart {
  readonly id: string;
  readonly name: string;
  readonly description: string;
  readonly reviewStatus: string | null;
  readonly evidence: readonly DraftEvidence[];
}

export interface DraftStep {
  readonly id: string;
  readonly title: string;
  readonly orderedActions: readonly string[];
  readonly partIds: readonly string[];
  readonly safetyNotes: readonly string[];
  readonly reviewStatus: string | null;
  readonly evidence: readonly DraftEvidence[];
}

export interface DraftHotspotView {
  readonly id: string;
  readonly partId: string;
  readonly status: string;
  readonly anchor: Anchor | null;
}

function readEvidence(value: unknown): DraftEvidence[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const out: DraftEvidence[] = [];
  for (const entry of value) {
    const record = asRecord(entry);
    if (record === null) {
      continue;
    }
    const pageNumber = record.pageNumber;
    if (typeof pageNumber !== "number" || !Number.isInteger(pageNumber) || pageNumber < 1) {
      continue;
    }
    out.push({
      pageNumber,
      documentId: asString(record.documentId),
      quote: asString(record.quote),
    });
  }
  return out;
}

function readReviewStatus(record: Record<string, unknown>): string | null {
  return asString(record.reviewStatus);
}

/** 草稿知识里的合并结果（`knowledge.knowledge`）。 */
function mergedKnowledge(knowledge: unknown): Record<string, unknown> | null {
  const shell = asRecord(knowledge);
  return asRecord(shell?.knowledge);
}

export function readDraftParts(knowledge: unknown): DraftPart[] {
  const merged = mergedKnowledge(knowledge);
  const parts = merged?.parts;
  if (!Array.isArray(parts)) {
    return [];
  }
  const out: DraftPart[] = [];
  for (const entry of parts) {
    const record = asRecord(entry);
    const id = asString(record?.id);
    const name = asString(record?.name);
    if (record === null || id === null || name === null) {
      continue;
    }
    out.push({
      id,
      name,
      description: asString(record.description) ?? "",
      reviewStatus: readReviewStatus(record),
      evidence: readEvidence(record.evidence),
    });
  }
  return out;
}

export function readDraftSteps(knowledge: unknown): DraftStep[] {
  const merged = mergedKnowledge(knowledge);
  const steps = merged?.steps;
  if (!Array.isArray(steps)) {
    return [];
  }
  const out: DraftStep[] = [];
  for (const entry of steps) {
    const record = asRecord(entry);
    const id = asString(record?.id);
    const title = asString(record?.title);
    if (record === null || id === null || title === null) {
      continue;
    }
    out.push({
      id,
      title,
      orderedActions: asStringArray(record.orderedActions),
      partIds: asStringArray(record.partIds),
      safetyNotes: asStringArray(record.safetyNotes),
      reviewStatus: readReviewStatus(record),
      evidence: readEvidence(record.evidence),
    });
  }
  return out;
}

export interface DraftSpec {
  readonly id: string;
  readonly label: string;
  readonly value: string;
  readonly reviewStatus: string | null;
  readonly evidence: readonly DraftEvidence[];
}

/** 规格读取（与部件/步骤同一宽容策略）。 */
export function readDraftSpecs(knowledge: unknown): DraftSpec[] {
  const merged = mergedKnowledge(knowledge);
  const specs = merged?.specs;
  if (!Array.isArray(specs)) {
    return [];
  }
  const out: DraftSpec[] = [];
  for (const entry of specs) {
    const record = asRecord(entry);
    const id = asString(record?.id);
    const label = asString(record?.label);
    if (record === null || id === null || label === null) {
      continue;
    }
    out.push({
      id,
      label,
      value: asString(record.value) ?? "",
      reviewStatus: readReviewStatus(record),
      evidence: readEvidence(record.evidence),
    });
  }
  return out;
}

/** 热点读取（T19 落库形状以 contracts §2 为准；本读取器按字段名宽容读取）。 */
export function readDraftHotspots(knowledge: unknown): DraftHotspotView[] {
  const shell = asRecord(knowledge);
  const hotspots = shell?.hotspots;
  if (!Array.isArray(hotspots)) {
    return [];
  }
  const out: DraftHotspotView[] = [];
  for (const entry of hotspots) {
    const record = asRecord(entry);
    const id = asString(record?.id);
    if (record === null || id === null) {
      continue;
    }
    const anchor = asRecord(record.anchor);
    const revisionId = asString(anchor?.modelRevisionId);
    const sha256 = asString(anchor?.modelSha256);
    const positionLocal = anchor?.positionLocal;
    const usable =
      revisionId !== null &&
      sha256 !== null &&
      isFiniteVec3(positionLocal) &&
      revisionId !== "" &&
      sha256 !== "";
    out.push({
      id,
      partId: asString(record.partId) ?? "",
      status: asString(record.status) ?? "unbound",
      anchor: usable
        ? { modelRevisionId: revisionId, modelSha256: sha256, positionLocal: positionLocal as Vec3 }
        : null,
    });
  }
  return out;
}

/**
 * 步骤视角读取（`knowledge.stepPoses`；T19）。
 * 键 = 步骤 id，值 = `CameraPose`（相对同一 asset-root）。坏数据返回空表。
 */
export function readDraftStepPoses(knowledge: unknown): Record<string, CameraPose> {
  const shell = asRecord(knowledge);
  const poses = asRecord(shell?.stepPoses);
  if (poses === null) {
    return {};
  }
  const out: Record<string, CameraPose> = {};
  for (const [stepId, value] of Object.entries(poses)) {
    const record = asRecord(value);
    const fov = record?.fov;
    const positionLocal = record?.positionLocal;
    const targetLocal = record?.targetLocal;
    const upLocal = record?.upLocal;
    if (
      record === null ||
      typeof fov !== "number" ||
      !Number.isFinite(fov) ||
      !isFiniteVec3(positionLocal) ||
      !isFiniteVec3(targetLocal) ||
      !isFiniteVec3(upLocal)
    ) {
      continue;
    }
    out[stepId] = { positionLocal, targetLocal, upLocal, fov };
  }
  return out;
}

/** 实体级复核覆盖（`review.entities[<entityId>]`；T19）。 */
export interface EntityReviewView {
  readonly reviewStatus: string | null;
  readonly userEdited: {
    readonly name?: string;
    readonly description?: string;
    readonly title?: string;
    readonly orderedActions?: readonly string[];
    readonly label?: string;
    readonly value?: string;
  } | null;
  readonly textOnly: boolean;
  readonly editedAt: number | null;
  readonly editedBy: string | null;
}

const EMPTY_ENTITY_REVIEW: EntityReviewView = {
  reviewStatus: null,
  userEdited: null,
  textOnly: false,
  editedAt: null,
  editedBy: null,
};

/** 该实体是否已复核（confirmed 或有人工修订记录；与服务端发布不变量同判据）。 */
export function isEntityReviewed(entry: EntityReviewView | undefined): boolean {
  if (entry === undefined) {
    return false;
  }
  return entry.reviewStatus === "confirmed" || entry.userEdited !== null;
}

export interface ModelReviewView {
  readonly loaded: boolean;
  readonly userConfirmed: boolean;
  readonly checkedAt: number | null;
  readonly loadedAt: number | null;
  readonly userConfirmedAt: number | null;
  readonly modelRevisionId: string;
  readonly modelSha256: string;
}

export interface DraftReviewView {
  readonly entities: Readonly<Record<string, EntityReviewView>>;
  readonly modelReview: ModelReviewView | null;
}

function readUserEdited(value: unknown): EntityReviewView["userEdited"] {
  const record = asRecord(value);
  if (record === null) {
    return null;
  }
  const asOptional = (candidate: unknown): string | undefined =>
    typeof candidate === "string" ? candidate : undefined;
  const actions = Array.isArray(record.orderedActions)
    ? record.orderedActions.filter((item): item is string => typeof item === "string")
    : undefined;
  const name = asOptional(record.name);
  const description = asOptional(record.description);
  const title = asOptional(record.title);
  const label = asOptional(record.label);
  const fieldValue = asOptional(record.value);
  const result = {
    ...(name === undefined ? {} : { name }),
    ...(description === undefined ? {} : { description }),
    ...(title === undefined ? {} : { title }),
    ...(actions === undefined ? {} : { orderedActions: actions }),
    ...(label === undefined ? {} : { label }),
    ...(fieldValue === undefined ? {} : { value: fieldValue }),
  };
  return Object.keys(result).length === 0 ? null : result;
}

/** 读取整块复核覆盖层（`DraftDto.review`；缺字段/坏结构返回空，不猜）。 */
export function readDraftReview(review: unknown): DraftReviewView {
  const record = asRecord(review);
  const entitiesRecord = asRecord(record?.entities);
  const entities: Record<string, EntityReviewView> = {};
  if (entitiesRecord !== null) {
    for (const [id, value] of Object.entries(entitiesRecord)) {
      const entry = asRecord(value);
      if (entry === null) {
        continue;
      }
      entities[id] = {
        reviewStatus: asString(entry.reviewStatus),
        userEdited: readUserEdited(entry.userEdited),
        textOnly: entry.textOnly === true,
        editedAt: typeof entry.editedAt === "number" ? entry.editedAt : null,
        editedBy: asString(entry.editedBy),
      };
    }
  }
  const modelReviewRecord = asRecord(record?.modelReview);
  const modelReview: ModelReviewView | null =
    modelReviewRecord === null
      ? null
      : {
          loaded: modelReviewRecord.loaded === true,
          userConfirmed: modelReviewRecord.userConfirmed === true,
          checkedAt:
            typeof modelReviewRecord.checkedAt === "number" ? modelReviewRecord.checkedAt : null,
          loadedAt: typeof modelReviewRecord.loadedAt === "number" ? modelReviewRecord.loadedAt : null,
          userConfirmedAt:
            typeof modelReviewRecord.userConfirmedAt === "number"
              ? modelReviewRecord.userConfirmedAt
              : null,
          modelRevisionId: asString(modelReviewRecord.modelRevisionId) ?? "",
          modelSha256: asString(modelReviewRecord.modelSha256) ?? "",
        };
  return { entities, modelReview };
}

export { EMPTY_ENTITY_REVIEW };

export interface DraftMissingEntry {
  readonly code: string;
  readonly message: string;
}

export function readDraftMissing(knowledge: unknown): DraftMissingEntry[] {
  const shell = asRecord(knowledge);
  const missing = shell?.missing;
  if (!Array.isArray(missing)) {
    return [];
  }
  const out: DraftMissingEntry[] = [];
  for (const entry of missing) {
    const record = asRecord(entry);
    const code = asString(record?.code);
    if (code === null) {
      continue;
    }
    out.push({ code, message: asString(record?.message) ?? "" });
  }
  return out;
}
