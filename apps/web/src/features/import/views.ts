/**
 * 视图槽位与"生成前置缺项"的纯逻辑（PRD §6.2 UI-012/UI-013；REQ-013/REQ-016/REQ-020）。
 *
 * 缺项清单与**服务端 estimate 的 `details.items[].code` 使用同一套词表**
 * （`missingFrontView`/`missingSideView`/`preparationNotReady`），
 * 这样界面提示与 422 明细相互对应，不会出现两套说法。
 */

import type { PhotoDto } from "../../api/endpoints";

/** 槽位顺序（front → left → back → right → detail）。 */
export const VIEW_ORDER = ["front", "left", "back", "right", "detail"] as const;

export type ViewSlot = (typeof VIEW_ORDER)[number];

export const VIEW_LABELS: Record<ViewSlot, string> = {
  front: "正面",
  left: "左侧",
  back: "背面",
  right: "右侧",
  detail: "特写",
};

/** 进入多视图生成的前四个槽位（detail 只用于理解与核对，不进入 Tripo 请求体）。 */
export const TRIPO_VIEWS: readonly Exclude<ViewSlot, "detail">[] = ["front", "left", "back", "right"];

/** 侧视图：至少其中之一（REQ-020 的"front + left/back/right 之一"）。 */
export const SIDE_VIEWS: readonly ViewSlot[] = ["left", "back", "right"];

/**
 * 按槽位顺序取出进入生成的 `photoId`（不含 detail）。
 *
 * 顺序与 T11 快照的槽位顺序（front→left→back→right）一致；
 * 先 front 再侧视图，服务端按集合校验，不依赖前端排序。
 */
export function tripoPhotoIds(photos: readonly PhotoDto[]): string[] {
  const ids: string[] = [];
  for (const view of TRIPO_VIEWS) {
    const photo = photos.find((candidate) => candidate.view === view);
    if (photo !== undefined) {
      ids.push(photo.id);
    }
  }
  return ids;
}

export function photosByView(photos: readonly PhotoDto[]): Map<string, PhotoDto> {
  const map = new Map<string, PhotoDto>();
  for (const photo of photos) {
    if (!map.has(photo.view)) {
      map.set(photo.view, photo);
    }
  }
  return map;
}

export interface MissingItem {
  readonly code: string;
  readonly message: string;
  readonly actionHref: string | null;
  readonly actionLabel: string | null;
}

export interface GenerationGapsInput {
  readonly itemId: string;
  /** preparation 的 state；null = 还没有准备记录（指针缺失或未创建）。 */
  readonly preparationState: string | null;
  readonly photos: readonly PhotoDto[];
  /** `GET /settings/status` 的 `capabilities.generation`；null = 尚未读取。 */
  readonly generationCapability: boolean | null;
}

/**
 * 生成前缺项（顺序即修复顺序）：准备 → 视图 → 配置。
 *
 * 前端只做"提前提示"，权威校验仍在服务端（estimate / jobs 各自返回 422/409 明细；
 * 前端不据此判断远端成功，也不替代服务端校验）。
 */
export function generationGaps(input: GenerationGapsInput): MissingItem[] {
  const gaps: MissingItem[] = [];
  const { itemId } = input;

  if (input.preparationState === null) {
    gaps.push({
      code: "preparationNotReady",
      message: "还没有可用的资料准备记录：请先完成第 4 步的准备并封存（ready）。",
      actionHref: `/items/${itemId}/import/prepare`,
      actionLabel: "去准备",
    });
  } else if (input.preparationState !== "ready") {
    gaps.push({
      code: "preparationNotReady",
      message: `资料准备尚未封存（当前状态：${input.preparationState}）。`,
      actionHref: `/items/${itemId}/import/prepare`,
      actionLabel: "继续准备",
    });
  }

  const views = new Set(input.photos.map((photo) => photo.view));
  if (!views.has("front")) {
    gaps.push({
      code: "missingFrontView",
      message: "缺少 front（正面）视图照片。",
      actionHref: `/items/${itemId}/import/views`,
      actionLabel: "去补充视图",
    });
  }
  if (!SIDE_VIEWS.some((view) => views.has(view))) {
    gaps.push({
      code: "missingSideView",
      message: "缺少侧面视图：left/back/right 至少需要一张。",
      actionHref: `/items/${itemId}/import/views`,
      actionLabel: "去补充视图",
    });
  }

  if (input.generationCapability === false) {
    gaps.push({
      code: "generationUnavailable",
      message:
        "生成能力未就绪：服务端未配置 Tripo／说明书 AI 密钥或价格目录（设置页只显示状态，密钥由部署者配置）。",
      actionHref: "/settings",
      actionLabel: "查看服务状态",
    });
  }

  return gaps;
}
