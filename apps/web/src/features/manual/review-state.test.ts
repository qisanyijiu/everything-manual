/**
 * 校准工作区纯逻辑的单测（T19；AC-052/AC-054/AC-056 的前端侧）。
 *
 * 覆盖：热点状态归一化（stale/unbound/不匹配都不可用）、发布预检与服务端不变量
 * 同判据（未确认知识/缺热点/仅文本条目/模型复核双声明与版本匹配）、请求体构造
 * （人工直接拾取 = confirmed + 非空 anchor；重新绑定沿用同一热点 id）。
 */

import { describe, expect, it } from "vitest";

import type { DraftPart, DraftStep } from "../viewer/draft-view";
import type { EntityReviewView, ModelReviewView } from "../viewer/draft-view";
import {
  hotspotPickUpsert,
  hotspotRebindUpsert,
  hotspotRemove,
  hotspotViews,
  publishChecklist,
  summarizePartHotspots,
  textOnlyPatch,
  usableHotspots,
} from "./review-state";

const MODEL = { revisionId: "rev-1", sha256: "a".repeat(64) };

function part(id: string, name: string): DraftPart {
  return { id, name, description: "", reviewStatus: "needs_review", evidence: [] };
}

function step(id: string, title: string): DraftStep {
  return {
    id,
    title,
    orderedActions: [],
    partIds: [],
    safetyNotes: [],
    reviewStatus: "needs_review",
    evidence: [],
  };
}

function reviewEntry(overrides: Partial<EntityReviewView>): EntityReviewView {
  return {
    reviewStatus: null,
    userEdited: null,
    textOnly: false,
    editedAt: null,
    editedBy: null,
    ...overrides,
  };
}

function modelReview(overrides: Partial<ModelReviewView>): ModelReviewView {
  return {
    loaded: true,
    userConfirmed: true,
    checkedAt: 1,
    loadedAt: 1,
    userConfirmedAt: 1,
    modelRevisionId: MODEL.revisionId,
    modelSha256: MODEL.sha256,
    ...overrides,
  };
}

describe("热点视图（stale 不冒充有效热点）", () => {
  it("只有匹配当前模型 revision+sha 的 confirmed/candidate 才可用", () => {
    const views = hotspotViews(
      [
        {
          id: "h1",
          partId: "p1",
          status: "confirmed",
          anchor: { modelRevisionId: MODEL.revisionId, modelSha256: MODEL.sha256, positionLocal: [0.1, 0.2, 0.3] },
        },
        {
          id: "h2",
          partId: "p1",
          status: "confirmed",
          anchor: { modelRevisionId: "rev-0", modelSha256: "b".repeat(64), positionLocal: [0, 0, 0] },
        },
        { id: "h3", partId: "p2", status: "unbound", anchor: null },
        {
          id: "h4",
          partId: "p2",
          status: "stale",
          anchor: { modelRevisionId: "rev-0", modelSha256: "b".repeat(64), positionLocal: [1, 1, 1] },
        },
      ],
      MODEL,
    );
    expect(views.map((hotspot) => hotspot.usable)).toEqual([true, false, false, false]);
    expect(usableHotspots(views).map((hotspot) => hotspot.id)).toEqual(["h1"]);
    // [0,0,0] 只有在与当前模型匹配时才可能是 confirmed 的坐标（服务端拒绝占位）。
    expect(usableHotspots(views)[0]?.positionLocal).toEqual([0.1, 0.2, 0.3]);
  });

  it("部件热点统计区分已确认/失效/未绑定", () => {
    const views = hotspotViews(
      [
        {
          id: "h1",
          partId: "p1",
          status: "confirmed",
          anchor: { modelRevisionId: MODEL.revisionId, modelSha256: MODEL.sha256, positionLocal: [0, 1, 0] },
        },
        {
          id: "h2",
          partId: "p1",
          status: "stale",
          anchor: { modelRevisionId: "old", modelSha256: "c".repeat(64), positionLocal: [0, 0, 0] },
        },
        { id: "h3", partId: "p1", status: "unbound", anchor: null },
      ],
      MODEL,
    );
    expect(summarizePartHotspots(views, "p1")).toEqual({ confirmed: 1, stale: 1, unbound: 1 });
  });
});

describe("发布预检（镜像服务端发布不变量）", () => {
  it("未确认知识 / 缺热点 / 模型复核未完成逐条列出且不可发布", () => {
    const checklist = publishChecklist({
      parts: [part("p1", "后盖")],
      steps: [step("s1", "取下后盖")],
      specs: [],
      hotspots: [],
      entityReviews: {},
      modelReview: null,
      model: MODEL,
    });
    expect(checklist.ready).toBe(false);
    const codes = checklist.items.map((item) => item.code);
    expect(codes).toContain("knowledgeUnreviewed");
    expect(codes).toContain("hotspotMissing");
    expect(codes).toContain("modelReviewMissing");
    expect(checklist.counts.unreviewed).toBe(2);
    expect(checklist.counts.missingHotspots).toBe(1);
  });

  it("仅文本条目（已确认）不需要热点，但保留计数", () => {
    const checklist = publishChecklist({
      parts: [part("p1", "后盖")],
      steps: [],
      specs: [],
      hotspots: [],
      entityReviews: { p1: reviewEntry({ reviewStatus: "confirmed", textOnly: true }) },
      modelReview: modelReview({}),
      model: MODEL,
    });
    expect(checklist.ready).toBe(true);
    expect(checklist.counts.textOnlyParts).toBe(1);
    expect(checklist.counts.missingHotspots).toBe(0);
  });

  it("模型复核必须双声明且与当前模型 revision+sha 匹配", () => {
    const base = {
      parts: [] as DraftPart[],
      steps: [] as DraftStep[],
      specs: [],
      hotspots: [],
      entityReviews: {},
      model: MODEL,
    };
    expect(
      publishChecklist({ ...base, modelReview: modelReview({ loaded: false }) }).ready,
    ).toBe(false);
    expect(
      publishChecklist({ ...base, modelReview: modelReview({ userConfirmed: false }) }).ready,
    ).toBe(false);
    const mismatched = publishChecklist({
      ...base,
      modelReview: modelReview({ modelRevisionId: "rev-2", modelSha256: "d".repeat(64) }),
    });
    expect(mismatched.ready).toBe(false);
    expect(mismatched.items[0]?.code).toBe("modelReviewModelMismatch");
    expect(publishChecklist({ ...base, modelReview: modelReview({}) }).ready).toBe(true);
  });

  it("stale 热点不算 confirmed，发布预检把它计入待处理", () => {
    const views = hotspotViews(
      [
        {
          id: "h-old",
          partId: "p1",
          status: "stale",
          anchor: { modelRevisionId: "old", modelSha256: "e".repeat(64), positionLocal: [0, 0, 0] },
        },
      ],
      MODEL,
    );
    const checklist = publishChecklist({
      parts: [part("p1", "后盖")],
      steps: [],
      specs: [],
      hotspots: views,
      entityReviews: { p1: reviewEntry({ reviewStatus: "confirmed" }) },
      modelReview: modelReview({}),
      model: MODEL,
    });
    expect(checklist.ready).toBe(false);
    expect(checklist.counts.missingHotspots).toBe(1);
    expect(checklist.counts.staleHotspots).toBe(1);
  });
});

describe("请求体构造（与服务端 PATCH 同形）", () => {
  it("人工直接拾取 = confirmed + 非空 anchor（不写 [0,0,0] 占位）", () => {
    const upsert = hotspotPickUpsert("p1", MODEL, [0.25, -0.5, 0.75]);
    expect(upsert.status).toBe("confirmed");
    expect(upsert.anchor).toEqual({
      modelRevisionId: "rev-1",
      modelSha256: "a".repeat(64),
      positionLocal: [0.25, -0.5, 0.75],
    });
    expect(Object.keys(upsert)).not.toContain("id");
  });

  it("重新绑定沿用同一热点 id（stale → confirmed）", () => {
    const upsert = hotspotRebindUpsert("h1", "p1", MODEL, [0, 0.5, 0]);
    expect(upsert.id).toBe("h1");
    expect(upsert.status).toBe("confirmed");
  });

  it("解绑与仅文本条目的请求体形状", () => {
    expect(hotspotRemove("h1")).toEqual({ upsert: [], remove: ["h1"] });
    expect(textOnlyPatch("p1")).toEqual({ p1: { textOnly: true, reviewStatus: "confirmed" } });
  });
});
