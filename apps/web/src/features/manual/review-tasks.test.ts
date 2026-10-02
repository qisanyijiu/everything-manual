import { describe, expect, it } from "vitest";
import { hotspotViews, publishChecklist, type PublishChecklistInput } from "./review-state";
import { nextReviewTask, reviewTasks, taskDestination, taskForPublishIssue } from "./review-tasks";
const model = { revisionId: "m1", sha256: "a".repeat(64) };
const input = (): { -readonly [K in keyof PublishChecklistInput]: PublishChecklistInput[K] } => ({ model, modelReview: null, entityReviews: {}, hotspots: [],
  parts: [{ id: "p1", name: "盖板", description: "", evidence: [], reviewStatus: "needs_review" }],
  steps: [{ id: "s1", title: "开盖", orderedActions: [], evidence: [], partIds: ["p1"], safetyNotes: [], reviewStatus: "needs_review" }],
  specs: [{ id: "sp1", label: "供电" }] });
describe("PC02B server-fact task mapping", () => {
  it("keeps stable grouped identities, true completion and optional poses outside publish rules", () => {
    const value = input();
    const before = reviewTasks(value, {});
    expect(before.filter(t => t.group === "facts" && !t.done)).toHaveLength(3);
    value.entityReviews = { p1: { reviewStatus: "confirmed", userEdited: null, textOnly: true, editedAt: null, editedBy: null } };
    const after = reviewTasks(value, {});
    expect(after.find(t => t.key === "fact-p1")?.done).toBe(true);
    expect(after.find(t => t.key === "binding-p1")?.done).toBe(true);
    expect(after.find(t => t.key === "pose-s1")).toMatchObject({ done: false, required: false });
    expect(publishChecklist(value).items.some(t => t.code.includes("pose"))).toBe(false);
    expect(nextReviewTask(after, "fact-p1")?.key).toBe("fact-s1");
    expect(nextReviewTask(after, "missing")).toBeNull();
  });
  it("stale targets the same hotspot; phone points at explanation and does not generate patches", () => {
    const value = input();
    value.hotspots = hotspotViews([{ id: "old", partId: "p1", status: "stale", anchor: { modelRevisionId: "old", modelSha256: "b".repeat(64), positionLocal: [1,2,3] } }], model);
    const task = reviewTasks(value, {}).find(t => t.key === "binding-p1")!;
    expect(task).toMatchObject({ action: "rebind", hotspotId: "old", done: false });
    expect(taskDestination(task, true)).toEqual({ panelId: "parts", focusId: "geometry-note-old" });
    expect(taskDestination(task, false)).toEqual({ panelId: "parts", focusId: "rebind-old" });
  });
  it("known issue must match trusted entity kind and id; unknown/no target never invents navigation", () => {
    const tasks = reviewTasks(input(), {});
    expect(taskForPublishIssue({ code: "knowledgeUnreviewed", entityId: "p1", entityKind: "part", message: "review" }, tasks)?.key).toBe("fact-p1");
    for (const issue of [
      { code: "futureCode", entityId: "p1", entityKind: "part" },
      { code: "knowledgeUnreviewed", entityId: "p1", entityKind: "spec" },
      { code: "knowledgeUnreviewed", entityId: "missing", entityKind: "part" },
      { code: "knowledgeUnreviewed", entityId: null, entityKind: "part" },
    ]) expect(taskForPublishIssue({ ...issue, message: "safe error" }, tasks)).toBeNull();
  });
  it("old model declarations do not complete either current independent declaration", () => {
    const value = input(); value.modelReview = { modelRevisionId: "old", modelSha256: model.sha256, loaded: true, userConfirmed: true, loadedAt: 1, userConfirmedAt: 1, checkedAt: 1 };
    expect(reviewTasks(value, {}).filter(t => t.group === "model").every(t => !t.done)).toBe(true);
  });
});
