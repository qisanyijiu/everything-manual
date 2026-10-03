import { describe, expect, it } from "vitest";
import { readReleaseKnowledge } from "./release-view";

const evidence = [{ documentId: "doc", pageNumber: 2, quote: "供应商引文" }];
const knowledge = {
  knowledge: {
    parts: [{ id: "p", name: "原部件", description: "原说明", evidence }],
    steps: [{ id: "s", title: "原步骤", orderedActions: ["原操作"], partIds: ["p"], safetyNotes: ["原安全提示"], evidence }],
    specs: [{ id: "v", label: "原规格", value: "原数值", evidence }],
  },
  model: { revisionId: "model", sha256: "hash", assetId: "asset", validationState: "validated" },
  hotspots: [{ id: "h", partId: "p", status: "confirmed", anchor: { modelRevisionId: "model", modelSha256: "hash", positionLocal: [1, 2, 3] } }],
};

function freeze(value: unknown) {
  if (typeof value !== "object" || value === null) return;
  Object.freeze(value);
  for (const entry of Object.values(value)) freeze(entry);
}

describe("frozen release text overlay", () => {
  it("uses all six fields while preserving source, references, safety and the input bytes", () => {
    const review = { entities: {
      p: { userEdited: { name: "人工部件", description: "人工说明" }, textOnly: true },
      s: { userEdited: { title: "人工步骤", orderedActions: ["操作一", "操作二"] } },
      v: { userEdited: { label: "人工规格", value: "人工数值" } },
    } };
    const manifest = { knowledge, review };
    const before = JSON.stringify(manifest);
    freeze(manifest);
    const view = readReleaseKnowledge(knowledge, review);
    expect(view.parts[0]).toMatchObject({ id: "p", name: "人工部件", description: "人工说明", evidence, hasUserEdit: true });
    expect(view.steps[0]).toMatchObject({ id: "s", title: "人工步骤", orderedActions: ["操作一", "操作二"], partIds: ["p"], safetyNotes: ["原安全提示"], evidence, hasUserEdit: true });
    expect(view.specs[0]).toMatchObject({ id: "v", label: "人工规格", value: "人工数值", evidence, hasUserEdit: true });
    expect(view.parts[0]?.original.name).toBe("原部件");
    expect(view.steps[0]?.original.orderedActions).toEqual(["原操作"]);
    expect(view.specs[0]?.original.value).toBe("原数值");
    expect(view.parts[0]?.evidence).toBe(view.parts[0]?.original.evidence);
    expect(view.steps[0]?.partIds).toBe(view.steps[0]?.original.partIds);
    expect(view.review.entities.p?.textOnly).toBe(true);
    expect(JSON.stringify(manifest)).toBe(before);
  });

  it("falls back field by field, including an entity with no edit", () => {
    const view = readReleaseKnowledge(knowledge, { entities: {
      p: { userEdited: { name: "人工部件" } },
      s: { userEdited: { orderedActions: ["人工操作"] } },
      v: { reviewStatus: "confirmed" },
    } });
    expect(view.parts[0]).toMatchObject({ name: "人工部件", description: "原说明" });
    expect(view.steps[0]).toMatchObject({ title: "原步骤", orderedActions: ["人工操作"] });
    expect(view.specs[0]).toMatchObject({ label: "原规格", value: "原数值", hasUserEdit: false });
  });

  it("preserves explicitly empty text and actions instead of restoring original text", () => {
    const view = readReleaseKnowledge(knowledge, { entities: {
      p: { userEdited: { name: "", description: "" } },
      s: { userEdited: { title: "", orderedActions: [] } },
      v: { userEdited: { label: "", value: "" } },
    } });
    expect(view.parts[0]).toMatchObject({ name: "", description: "", hasUserEdit: true });
    expect(view.steps[0]).toMatchObject({ title: "", orderedActions: [], hasUserEdit: true });
    expect(view.specs[0]).toMatchObject({ label: "", value: "", hasUserEdit: true });
  });

  it("ignores unknown IDs, other entity kinds and malformed fields without changing linkage", () => {
    const view = readReleaseKnowledge(knowledge, { entities: {
      missing: { userEdited: { name: "未知" } },
      p: { userEdited: { title: "错误类型", name: 7, id: "changed", evidence: [] } },
      s: { userEdited: { name: "错误类型", title: null, orderedActions: "invalid", partIds: [], safetyNotes: [42] } },
      v: { userEdited: { description: "错误类型", label: false, value: null } },
    } });
    expect(view.parts).toHaveLength(1);
    expect(view.parts[0]).toMatchObject({ id: "p", name: "原部件", evidence, hasUserEdit: false });
    expect(view.steps[0]).toMatchObject({ title: "原步骤", orderedActions: ["原操作"], partIds: ["p"], safetyNotes: ["原安全提示"], hasUserEdit: false });
    expect(view.specs[0]).toMatchObject({ label: "原规格", value: "原数值", hasUserEdit: false });
  });

  it("keeps older releases without a review and absent knowledge readable", () => {
    expect(readReleaseKnowledge(knowledge, undefined).parts[0]).toMatchObject({ name: "原部件", hasUserEdit: false });
    expect(readReleaseKnowledge(undefined, undefined)).toEqual({ parts: [], steps: [], specs: [], review: { entities: {}, modelReview: null } });
  });

  it("applies a safety-only revision including explicit removal, retaining the original warnings and source", () => {
    const before = JSON.stringify(knowledge);
    for (const safetyNotes of [["红灯亮起之后等待至少四秒"], []]) {
      const view = readReleaseKnowledge(knowledge, { entities: { s: { userEdited: { safetyNotes } } } });
      expect(view.steps[0]).toMatchObject({ title: "原步骤", orderedActions: ["原操作"], safetyNotes, hasUserEdit: true, partIds: ["p"], evidence });
      expect(view.steps[0]?.original.safetyNotes).toEqual(["原安全提示"]);
    }
    expect(JSON.stringify(knowledge)).toBe(before);
  });

  it.each([null, "wrong", ["valid", 42], [null]])("does not turn malformed safety notes into a warning deletion: %j", (safetyNotes) => {
    const view = readReleaseKnowledge(knowledge, { entities: { s: { userEdited: { safetyNotes } } } });
    expect(view.steps[0]).toMatchObject({ safetyNotes: ["原安全提示"], hasUserEdit: false });
  });
});
