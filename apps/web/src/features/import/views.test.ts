import { describe, expect, it } from "vitest";

import type { PhotoDto } from "../../api/endpoints";
import { generationGaps, tripoPhotoIds, photosByView } from "./views";

function photo(id: string, view: string): PhotoDto {
  return {
    id,
    itemId: "item-1",
    assetId: `asset-${id}`,
    view,
    revision: 1,
    createdAt: "2026-09-12T00:00:00Z",
    updatedAt: "2026-09-12T00:00:00Z",
  };
}

describe("多视图照片选择", () => {
  it("按槽位顺序（front→left→back→right）取 id，detail 不进入", () => {
    const photos = [
      photo("p-right", "right"),
      photo("p-detail", "detail"),
      photo("p-front", "front"),
      photo("p-left", "left"),
    ];
    expect(tripoPhotoIds(photos)).toEqual(["p-front", "p-left", "p-right"]);
  });

  it("photosByView 每视图保留第一条（服务端保证同视图唯一）", () => {
    const map = photosByView([photo("p1", "front"), photo("p2", "front")]);
    expect(map.get("front")?.id).toBe("p1");
  });
});

describe("生成前置缺项（词表与服务端 details.items[].code 对齐）", () => {
  it("缺 front 与缺侧面分别列出，并给出修复链接", () => {
    const gaps = generationGaps({
      itemId: "item-1",
      preparationState: "ready",
      photos: [photo("p-detail", "detail")],
      generationCapability: true,
    });
    expect(gaps.map((gap) => gap.code)).toEqual(["missingFrontView", "missingSideView"]);
    expect(gaps[0]?.actionHref).toBe("/items/item-1/import/views");
  });

  it("准备未封存（含完全没有准备记录）报 preparationNotReady", () => {
    for (const state of [null, "preparing"]) {
      const gaps = generationGaps({
        itemId: "item-1",
        preparationState: state,
        photos: [photo("p-front", "front"), photo("p-left", "left")],
        generationCapability: true,
      });
      expect(gaps.map((gap) => gap.code)).toEqual(["preparationNotReady"]);
    }
  });

  it("能力未就绪（未配置 Provider/价格目录）给出设置页入口", () => {
    const gaps = generationGaps({
      itemId: "item-1",
      preparationState: "ready",
      photos: [photo("p-front", "front"), photo("p-left", "left")],
      generationCapability: false,
    });
    expect(gaps.map((gap) => gap.code)).toEqual(["generationUnavailable"]);
    expect(gaps[0]?.actionHref).toBe("/settings");
  });

  it("资料齐全且能力可用时没有缺项（front + left 满足侧面要求）", () => {
    const gaps = generationGaps({
      itemId: "item-1",
      preparationState: "ready",
      photos: [photo("p-front", "front"), photo("p-left", "left"), photo("p-detail", "detail")],
      generationCapability: true,
    });
    expect(gaps).toEqual([]);
  });
});
