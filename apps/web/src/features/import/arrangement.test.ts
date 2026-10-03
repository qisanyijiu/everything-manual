import { EMPTY_SLOTS, autoFill, drop, removeCard, sameArrangement, slotsPayload, type Slots } from "./arrangement";

const a = { assetId: "a", candidateId: "ca" };
const b = { assetId: "b", candidateId: "cb" };
const c = { assetId: "c", candidateId: null };

describe("drop", () => {
  it("托盘 → 空槽", () => {
    expect(drop(EMPTY_SLOTS, a, { kind: "slot", view: "front" }).front).toEqual(a);
  });

  it("槽 → 已占槽：两者交换", () => {
    const s: Slots = { ...EMPTY_SLOTS, front: a, left: b };
    const next = drop(s, a, { kind: "slot", view: "left" });
    expect(next.front).toEqual(b);
    expect(next.left).toEqual(a);
  });

  it("托盘 → 已占槽：替换，被替换的卡片回到托盘", () => {
    const s: Slots = { ...EMPTY_SLOTS, front: a };
    const next = drop(s, c, { kind: "slot", view: "front" });
    expect(next.front).toEqual(c);
    expect(Object.values(next).filter(Boolean)).toHaveLength(1);
  });

  it("槽 → 托盘：清空该槽；拖到原槽不变", () => {
    const s: Slots = { ...EMPTY_SLOTS, back: b };
    expect(drop(s, b, { kind: "tray" }).back).toBeNull();
    expect(drop(s, b, { kind: "slot", view: "back" })).toBe(s);
  });
});

describe("autoFill / removeCard / payload", () => {
  const candidates = [
    { id: "ca", assetId: "a", suggestedView: "front", confidence: 0.6 },
    { id: "cb", assetId: "b", suggestedView: "front", confidence: 0.9 },
    { id: "cc", assetId: "c", suggestedView: "back", confidence: 0.4 },
    { id: "cd", assetId: "d", suggestedView: "left", confidence: 0.8 },
  ];

  it("每个空槽取置信度最高的候选；低于阈值不填；已占槽不动", () => {
    const filled = autoFill(EMPTY_SLOTS, candidates);
    expect(filled.front?.assetId).toBe("b");
    expect(filled.left?.assetId).toBe("d");
    expect(filled.back).toBeNull();
    const kept = autoFill({ ...EMPTY_SLOTS, front: { assetId: "x", candidateId: null } }, candidates);
    expect(kept.front?.assetId).toBe("x");
  });

  it("removeCard 清空所在槽位；payload 只含资产 id", () => {
    const s: Slots = { ...EMPTY_SLOTS, front: a, detail: c };
    const next = removeCard(s, "a");
    expect(slotsPayload(next)).toEqual({ front: null, left: null, back: null, right: null, detail: "c" });
    expect(sameArrangement(s, next)).toBe(false);
    expect(sameArrangement(s, { ...s })).toBe(true);
  });
});
