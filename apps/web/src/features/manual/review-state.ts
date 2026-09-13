/**
 * 校准工作区的**纯逻辑层**（T19；PRD AC-052–AC-056、UI-046–UI-056）。
 *
 * 设计原则：
 * 1. **服务端是唯一权威**：这里的"发布预检"只是把服务端的发布不变量镜像成可读的
 *    待办清单（UI-054「不满足时按钮禁用且原因列表常驻」），最终以 422 的
 *    `details.issues[]` 为准（两处判据必须一致；服务端代码在
 *    `crates/server/src/releases/invariants.rs`）。
 * 2. **「事实确认」与「几何校准」严格分开**（ADR-005 / PRD §6.3.2）：
 *    文字确认（`entity` 级 confirmed/needs_review 与人工修订）与几何校准
 *    （热点绑定、步骤视角）是不同的动作、不同的状态与不同的文案，绝不共用一个
 *    无定语的"确认"。
 * 3. **不静默复用**：stale 热点永不进入"可用热点"集合；发布预检把它列为待处理项。
 */

import type { Anchor, Vec3 } from "../viewer/coordinates";
import type {
  DraftHotspotView,
  DraftPart,
  DraftStep,
  EntityReviewView,
  ModelReviewView,
} from "../viewer/draft-view";
import { isEntityReviewed } from "../viewer/draft-view";

/** 模型身份（当前草稿选中的不可变版本）。 */
export interface ModelIdentity {
  readonly revisionId: string;
  readonly sha256: string;
}

export interface HotspotView {
  readonly id: string;
  readonly partId: string;
  readonly status: "unbound" | "candidate" | "confirmed" | "stale";
  readonly anchor: Anchor | null;
  /** 锚点是否与当前模型 revision+sha 完全一致且数值有限。 */
  readonly usable: boolean;
}

/** 归一化热点状态（服务端未识别取值按 unbound 处理，不猜成 confirmed）。 */
export function hotspotViews(
  hotspots: readonly DraftHotspotView[],
  model: ModelIdentity | null,
): HotspotView[] {
  return hotspots.map((hotspot) => {
    const status =
      hotspot.status === "confirmed" ||
      hotspot.status === "candidate" ||
      hotspot.status === "stale" ||
      hotspot.status === "unbound"
        ? hotspot.status
        : "unbound";
    const usable =
      model !== null &&
      hotspot.anchor !== null &&
      hotspot.anchor.modelRevisionId === model.revisionId &&
      hotspot.anchor.modelSha256 === model.sha256 &&
      status !== "unbound" &&
      status !== "stale";
    return { id: hotspot.id, partId: hotspot.partId, status, anchor: hotspot.anchor, usable };
  });
}

/** 可用于 3D 显示的热点（stale/unbound/不匹配一律不显示为有效热点）。 */
export function usableHotspots(
  views: readonly HotspotView[],
): { id: string; partId: string; positionLocal: Vec3 }[] {
  return views
    .filter((hotspot) => hotspot.usable && hotspot.anchor !== null)
    .map((hotspot) => ({
      id: hotspot.id,
      partId: hotspot.partId,
      positionLocal: (hotspot.anchor as Anchor).positionLocal,
    }));
}

export interface PartHotspotSummary {
  readonly confirmed: number;
  readonly stale: number;
  readonly unbound: number;
}

export function summarizePartHotspots(
  views: readonly HotspotView[],
  partId: string,
): PartHotspotSummary {
  const of = (predicate: (hotspot: HotspotView) => boolean): number =>
    views.filter((hotspot) => hotspot.partId === partId && predicate(hotspot)).length;
  return {
    confirmed: of((hotspot) => hotspot.usable && hotspot.status === "confirmed"),
    stale: of((hotspot) => hotspot.status === "stale" || (!hotspot.usable && hotspot.anchor !== null)),
    unbound: of((hotspot) => hotspot.status === "unbound" || hotspot.anchor === null),
  };
}

// ---------------------------------------------------------------------------
// 发布预检（镜像服务端不变量；422 details.issues 是最终判据）
// ---------------------------------------------------------------------------

export interface PublishChecklistItem {
  readonly code: string;
  readonly label: string;
  /** 可定位的实体 id（部件/步骤/规格/热点）；模型与输入级问题为 null。 */
  readonly entityId: string | null;
}

export interface PublishCounts {
  /** 未确认且无人工修订的实体数（部件+步骤+规格）。 */
  readonly unreviewed: number;
  /** 缺 confirmed 热点（且未标「仅文本条目」）的部件数。 */
  readonly missingHotspots: number;
  readonly staleHotspots: number;
  readonly textOnlyParts: number;
}

export interface ModelReviewState {
  readonly present: boolean;
  readonly loaded: boolean;
  readonly userConfirmed: boolean;
  readonly matches: boolean;
}

export interface PublishChecklist {
  readonly ready: boolean;
  readonly items: readonly PublishChecklistItem[];
  readonly counts: PublishCounts;
  readonly modelReview: ModelReviewState;
}

export interface PublishChecklistInput {
  readonly parts: readonly DraftPart[];
  readonly steps: readonly DraftStep[];
  readonly specs: readonly { id: string; label: string }[];
  readonly hotspots: readonly HotspotView[];
  readonly entityReviews: Readonly<Record<string, EntityReviewView>>;
  readonly modelReview: ModelReviewView | null;
  readonly model: ModelIdentity | null;
}

/**
 * 计算发布预检（与服务端 `check_publish_invariants` 同判据；顺序稳定：
 * 模型 → 复核 → 知识 → 热点）。
 */
export function publishChecklist(input: PublishChecklistInput): PublishChecklist {
  const items: PublishChecklistItem[] = [];
  const reviews = input.entityReviews;

  if (input.model === null) {
    items.push({ code: "modelMissing", label: "草稿没有可用模型（模型分支未完成）", entityId: null });
  }

  const review = input.modelReview;
  const matches =
    review !== null &&
    input.model !== null &&
    review.modelRevisionId === input.model.revisionId &&
    review.modelSha256 === input.model.sha256;
  if (input.model !== null) {
    if (review === null) {
      items.push({
        code: "modelReviewMissing",
        label: "缺少模型复核：请先打开模型，再完成两个声明",
        entityId: null,
      });
    } else if (!matches) {
      items.push({
        code: "modelReviewModelMismatch",
        label: "模型复核声明的版本与当前模型不一致：换模型后必须重新复核",
        entityId: null,
      });
    } else {
      if (!review.loaded) {
        items.push({
          code: "modelReviewIncomplete",
          label: "模型复核未完成：需要声明「已在浏览器成功打开此模型」",
          entityId: null,
        });
      }
      if (!review.userConfirmed) {
        items.push({
          code: "modelReviewIncomplete",
          label: "模型复核未完成：需要声明「我已核对模型与资料一致」",
          entityId: null,
        });
      }
    }
  }

  let unreviewed = 0;
  for (const part of input.parts) {
    if (!isEntityReviewed(reviews[part.id])) {
      unreviewed += 1;
      items.push({
        code: "knowledgeUnreviewed",
        label: `部件「${part.name}」尚未确认或修订`,
        entityId: part.id,
      });
    }
  }
  for (const step of input.steps) {
    if (!isEntityReviewed(reviews[step.id])) {
      unreviewed += 1;
      items.push({
        code: "knowledgeUnreviewed",
        label: `步骤「${step.title}」尚未确认或修订`,
        entityId: step.id,
      });
    }
  }
  for (const spec of input.specs) {
    if (!isEntityReviewed(reviews[spec.id])) {
      unreviewed += 1;
      items.push({
        code: "knowledgeUnreviewed",
        label: `规格「${spec.label}」尚未确认或修订`,
        entityId: spec.id,
      });
    }
  }

  let missingHotspots = 0;
  let textOnlyParts = 0;
  for (const part of input.parts) {
    if (reviews[part.id]?.textOnly === true) {
      textOnlyParts += 1;
      continue;
    }
    const confirmed = input.hotspots.some(
      (hotspot) => hotspot.partId === part.id && hotspot.usable && hotspot.status === "confirmed",
    );
    if (!confirmed) {
      missingHotspots += 1;
      items.push({
        code: "hotspotMissing",
        label: `部件「${part.name}」还没有 confirmed 热点（点选绑定，或确认后标记「仅文本条目」）`,
        entityId: part.id,
      });
    }
  }

  const staleHotspots = input.hotspots.filter(
    (hotspot) => hotspot.status === "stale" || (!hotspot.usable && hotspot.anchor !== null),
  ).length;
  for (const hotspot of input.hotspots) {
    if (hotspot.status !== "stale" && hotspot.anchor !== null && !hotspot.usable) {
      items.push({
        code: "hotspotNotMatchingModel",
        label: `热点绑定与当前模型不符：请重新绑定（${hotspot.partId}）`,
        entityId: hotspot.partId,
      });
    }
  }

  return {
    ready: items.length === 0,
    items,
    counts: { unreviewed, missingHotspots, staleHotspots, textOnlyParts },
    modelReview: {
      present: review !== null,
      loaded: review?.loaded === true,
      userConfirmed: review?.userConfirmed === true,
      matches,
    },
  };
}

// ---------------------------------------------------------------------------
// 请求体构造（与服务端 `DraftPatchRequest` 同形）
// ---------------------------------------------------------------------------

/** 人工直接拾取：新建热点并立即 confirmed（anchor 用 asset-root 局部坐标）。 */
export function hotspotPickUpsert(
  partId: string,
  model: ModelIdentity,
  positionLocal: Vec3,
): {
  partId: string;
  status: "confirmed";
  anchor: { modelRevisionId: string; modelSha256: string; positionLocal: number[] };
} {
  return {
    partId,
    status: "confirmed",
    anchor: {
      modelRevisionId: model.revisionId,
      modelSha256: model.sha256,
      positionLocal: [...positionLocal],
    },
  };
}

/** 重新绑定：用同一热点 id 提交匹配当前模型的 anchor（stale → confirmed）。 */
export function hotspotRebindUpsert(
  hotspotId: string,
  partId: string,
  model: ModelIdentity,
  positionLocal: Vec3,
): {
  id: string;
  partId: string;
  status: "confirmed";
  anchor: { modelRevisionId: string; modelSha256: string; positionLocal: number[] };
} {
  return { id: hotspotId, ...hotspotPickUpsert(partId, model, positionLocal) };
}

/** 解绑（移除热点）：部件回到"未绑定"，需要重新绑定或标记仅文本条目。 */
export function hotspotRemove(hotspotId: string): { upsert: []; remove: string[] } {
  return { upsert: [], remove: [hotspotId] };
}

export type EntityKind = "part" | "step" | "spec";

/** 实体确认/取消确认（事实确认，不是几何校准）。 */
export function entityReviewPatch(
  kind: EntityKind,
  entityId: string,
  decision: "confirmed" | "needs_review",
): Record<string, { reviewStatus: "confirmed" | "needs_review" }> {
  void kind;
  return { [entityId]: { reviewStatus: decision } };
}

/** 人工修订（userEdited 覆盖层；原快照不变，UI 显示原文本对照）。 */
export function entityEditPatch(
  entityId: string,
  fields: {
    name?: string;
    description?: string;
    title?: string;
    orderedActions?: string[];
    label?: string;
    value?: string;
  },
  options: { confirm: boolean },
): Record<
  string,
  {
    reviewStatus?: "confirmed";
    userEdited: typeof fields;
  }
> {
  return {
    [entityId]: {
      ...(options.confirm ? { reviewStatus: "confirmed" as const } : {}),
      userEdited: fields,
    },
  };
}

/** 「仅文本条目」（必须同时确认或已有修订记录）。 */
export function textOnlyPatch(entityId: string): Record<string, { textOnly: true; reviewStatus: "confirmed" }> {
  return { [entityId]: { textOnly: true, reviewStatus: "confirmed" } };
}
