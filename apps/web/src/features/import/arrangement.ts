/**
 * 视图排列状态（ADR-044）——拖拽/删除/交换的纯逻辑，便于单测。
 *
 * 状态：5 个槽位各放一个"卡片"（照片资产 id），外加候选托盘。卡片来源可以是候选或已登记的照片；
 * 一张卡片同一时刻只在一个位置（某槽位或托盘）。保存时只提交槽位 → 资产 id。
 */

import { VIEW_ORDER, type ViewSlot } from "./views";

export interface CardRef {
  readonly assetId: string;
  /** 候选 id（来自 PDF 拆图）；手动上传或既有照片为 null。 */
  readonly candidateId: string | null;
}

export type Slots = Readonly<Record<ViewSlot, CardRef | null>>;

export const EMPTY_SLOTS: Slots = { front: null, left: null, back: null, right: null, detail: null };

export type DropTarget = { readonly kind: "slot"; readonly view: ViewSlot } | { readonly kind: "tray" };

export function slotOf(slots: Slots, assetId: string): ViewSlot | null {
  return VIEW_ORDER.find((view) => slots[view]?.assetId === assetId) ?? null;
}

/**
 * 把卡片放到目标：
 * - 放到空槽：从原槽（若有）移出；
 * - 放到已占槽：与原槽交换（从托盘来则把被替换的卡片退回托盘）；
 * - 放回托盘：清空它所在的槽。
 */
export function drop(slots: Slots, card: CardRef, target: DropTarget): Slots {
  const from = slotOf(slots, card.assetId);
  const next: Record<ViewSlot, CardRef | null> = { ...slots };
  if (target.kind === "tray") {
    if (from !== null) {
      next[from] = null;
    }
    return next;
  }
  if (from === target.view) {
    return slots;
  }
  const occupant = next[target.view];
  next[target.view] = card;
  if (from !== null) {
    next[from] = occupant;
  }
  return next;
}

/** 删除卡片：从槽位移出（候选本身是否删除由调用方决定）。 */
export function removeCard(slots: Slots, assetId: string): Slots {
  const from = slotOf(slots, assetId);
  if (from === null) {
    return slots;
  }
  return { ...slots, [from]: null };
}

/**
 * 按建议视图自动填充空槽：每个视图取置信度最高的候选；已被占用的槽、已在槽中的卡片不动。
 * 只是初始建议，用户随后可拖拽修正。
 */
export function autoFill(
  slots: Slots,
  candidates: readonly { id: string; assetId: string; suggestedView?: string | null; confidence?: number | null; pageNumber?: number | null }[],
  minConfidence = 0.5,
): Slots {
  const next: Record<ViewSlot, CardRef | null> = { ...slots };
  for (const view of VIEW_ORDER) {
    if (next[view] !== null) {
      continue;
    }
    const best = candidates
      .filter((c) => c.suggestedView === view && (c.confidence ?? 0) >= minConfidence && slotOf(next, c.assetId) === null)
      .sort((a, b) => (b.confidence ?? 0) - (a.confidence ?? 0))[0];
    if (best !== undefined) {
      next[view] = { assetId: best.assetId, candidateId: best.id };
    }
  }
  // 生成要求至少一张侧面视图（left/back/right）。说明书常把所有产品图都画成正面/三分之四视角，
  // 分类结果全是 front：此时用其余置信度最高的产品图补一个侧面槽（优先不同页），用户可再拖拽修正。
  const sides: readonly ViewSlot[] = ["left", "right", "back"];
  if (sides.every((view) => next[view] === null)) {
    const frontPage = candidates.find((c) => c.assetId === next.front?.assetId)?.pageNumber ?? null;
    const spare = candidates
      .filter((c) => c.suggestedView != null && c.suggestedView !== "detail" && (c.confidence ?? 0) >= minConfidence && slotOf(next, c.assetId) === null)
      .sort((a, b) => Number(a.pageNumber === frontPage) - Number(b.pageNumber === frontPage) || (b.confidence ?? 0) - (a.confidence ?? 0))[0];
    if (spare !== undefined) {
      next.right = { assetId: spare.assetId, candidateId: spare.id };
    }
  }
  return next;
}

export function slotsPayload(slots: Slots): Record<ViewSlot, string | null> {
  return Object.fromEntries(VIEW_ORDER.map((view) => [view, slots[view]?.assetId ?? null])) as Record<ViewSlot, string | null>;
}

export function sameArrangement(a: Slots, b: Slots): boolean {
  return VIEW_ORDER.every((view) => (a[view]?.assetId ?? null) === (b[view]?.assetId ?? null));
}
