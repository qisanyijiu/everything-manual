/**
 * 草稿知识防御式读取的单元测试（T18）。
 *
 * 契约（QA 按此复核）：`DraftDto.knowledge` 是 `manual_draft_v1` 的 JSON 外壳，
 * 前端**不得**假设字段一定存在或类型一定正确；缺字段/类型不符返回空结果，不抛错、
 * 不猜测。热点读取按 contracts §2 的字段名（`anchor.positionLocal` 等）宽容读取。
 */

import { describe, expect, it } from "vitest";

import {
  readDraftHotspots,
  readDraftMissing,
  readDraftModel,
  readDraftParts,
  readDraftSteps,
} from "./draft-view";

const VALID_MODEL = {
  revisionId: "rev-1",
  sha256: "a".repeat(64),
  validationState: "validated",
  assetId: "asset-1",
  bounds: null,
};

function shellWith(extra: Record<string, unknown>): Record<string, unknown> {
  return {
    schemaVersion: "manual_draft_v1",
    completeness: "complete",
    model: VALID_MODEL,
    knowledge: {
      schemaVersion: "manual_extract_v1",
      parts: [
        {
          id: "p1",
          name: "外壳",
          description: "说明",
          reviewStatus: "needs_review",
          evidence: [{ documentId: "d1", preparationId: "prep-1", pageNumber: 2, quote: "q" }],
        },
      ],
      steps: [
        {
          id: "s1",
          title: "取下外壳",
          orderedActions: ["拧松螺丝", "取下外壳"],
          partIds: ["p1"],
          safetyNotes: ["断电"],
          reviewStatus: "needs_review",
          evidence: [
            { documentId: "d1", preparationId: "prep-1", pageNumber: 3, quote: null },
            { documentId: "d1", preparationId: "prep-1", pageNumber: "3" },
          ],
        },
      ],
    },
    missing: [{ code: "model_branch_incomplete", message: "模型分支未完成" }],
    ...extra,
  };
}

describe("草稿模型分支读取（UI-043：未通过校验不进入 3D）", () => {
  it("validated 版本可读，并带出校验状态", () => {
    const model = readDraftModel(shellWith({}));
    expect(model).toEqual({
      revisionId: "rev-1",
      sha256: "a".repeat(64),
      assetId: "asset-1",
      validationState: "validated",
    });
  });

  it("rejected/未知校验状态不返回可用模型（不渲染未校验模型）", () => {
    const rejected = shellWith({
      model: { ...VALID_MODEL, validationState: "rejected" },
    });
    expect(readDraftModel(rejected)).toBeNull();
    const unknown = shellWith({ model: { ...VALID_MODEL, validationState: undefined } });
    expect(readDraftModel(unknown)).toBeNull();
  });

  it("缺字段或类型不符时返回 null，而不是半成品引用", () => {
    expect(readDraftModel(null)).toBeNull();
    expect(readDraftModel("知识外壳")).toBeNull();
    expect(readDraftModel(shellWith({ model: null }))).toBeNull();
    expect(readDraftModel(shellWith({ model: { ...VALID_MODEL, assetId: "" } }))).toBeNull();
  });
});

describe("部件/步骤读取（宽容：坏条目跳过，不抛错）", () => {
  it("读取出部件与步骤的可读字段（含 1-based 页码出处）", () => {
    const knowledge = shellWith({});
    const parts = readDraftParts(knowledge);
    expect(parts).toHaveLength(1);
    expect(parts[0]?.name).toBe("外壳");
    expect(parts[0]?.evidence[0]?.pageNumber).toBe(2);

    const steps = readDraftSteps(knowledge);
    expect(steps).toHaveLength(1);
    expect(steps[0]?.orderedActions).toEqual(["拧松螺丝", "取下外壳"]);
    expect(steps[0]?.safetyNotes).toEqual(["断电"]);
    // 页码必须是 1-based 整数：非整数/缺失的出处被丢弃（不伪造页码）。
    expect(steps[0]?.evidence.map((evidence) => evidence.pageNumber)).toEqual([3]);
  });

  it("损坏或缺失的知识结构返回空数组", () => {
    expect(readDraftParts(null)).toEqual([]);
    expect(readDraftParts({ knowledge: "不是对象" })).toEqual([]);
    expect(readDraftSteps({ knowledge: { steps: [{ id: "s", title: null }] } })).toEqual([]);
    expect(readDraftParts({ knowledge: { parts: [null, 42, { id: "x" }] } })).toEqual([]);
  });
});

describe("热点读取（contracts §2 字段名）", () => {
  it("有限数值的锚点可读；非有限数值退化为 anchor=null（不显示为有效热点）", () => {
    const knowledge = shellWith({
      hotspots: [
        {
          id: "h1",
          partId: "p1",
          status: "confirmed",
          anchor: { modelRevisionId: "rev-1", modelSha256: "s", positionLocal: [1, 2, 3] },
        },
        {
          id: "h2",
          partId: "p1",
          status: "candidate",
          anchor: { modelRevisionId: "rev-1", modelSha256: "s", positionLocal: [null, 2, 3] },
        },
        { id: "h3", partId: "p1", status: "unbound", anchor: null },
      ],
    });
    const hotspots = readDraftHotspots(knowledge);
    expect(hotspots).toHaveLength(3);
    expect(hotspots[0]?.anchor?.positionLocal).toEqual([1, 2, 3]);
    expect(hotspots[1]?.anchor).toBeNull();
    expect(hotspots[2]?.anchor).toBeNull();
    expect(hotspots[2]?.status).toBe("unbound");
  });

  it("缺失 hotspots 字段时返回空数组（T15 草稿尚无热点）", () => {
    expect(readDraftHotspots(shellWith({}))).toEqual([]);
  });
});

describe("缺项读取", () => {
  it("读出 code/message；坏条目跳过", () => {
    const missing = readDraftMissing(
      shellWith({ missing: [{ code: "a", message: "缺 A" }, { code: 42 }, null] }),
    );
    expect(missing).toEqual([{ code: "a", message: "缺 A" }]);
  });
});
