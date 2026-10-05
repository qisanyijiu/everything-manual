import { describe, expect, it } from "vitest";

import {
  describeHotspotMarker,
  hotspotMarkerSpriteSize,
  paintHotspotMarker,
  type MarkerCanvasContext,
} from "./hotspot-markers";

type Call =
  | { op: "arc"; x: number; y: number; r: number }
  | { op: "fill"; style: string }
  | { op: "stroke"; style: string; dash: number[]; lineWidth: number }
  | { op: "fillText"; text: string; style: string }
  | { op: "clearRect" };

function recordingContext(): { ctx: MarkerCanvasContext; calls: Call[] } {
  const calls: Call[] = [];
  let dash: number[] = [];
  const ctx: MarkerCanvasContext = {
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 0,
    font: "",
    textAlign: "",
    textBaseline: "",
    clearRect: () => calls.push({ op: "clearRect" }),
    beginPath: () => undefined,
    arc: (x, y, r) => calls.push({ op: "arc", x, y, r }),
    fill: () => calls.push({ op: "fill", style: String(ctx.fillStyle) }),
    stroke: () =>
      calls.push({ op: "stroke", style: String(ctx.strokeStyle), dash: [...dash], lineWidth: ctx.lineWidth }),
    fillText: (text) => calls.push({ op: "fillText", text, style: String(ctx.fillStyle) }),
    setLineDash: (segments) => {
      dash = [...segments];
    },
  };
  return { ctx, calls };
}

describe("热点标记视觉语法（VS-03 / AC-VS-005）", () => {
  it("已确认为实心圆：填充 accent、无虚线、无问号、无数字", () => {
    const style = describeHotspotMarker("confirmed", false);
    expect(style).toEqual({
      status: "confirmed",
      selected: false,
      shape: "solid",
      glyph: null,
      outerRing: false,
    });
    const { ctx, calls } = recordingContext();
    paintHotspotMarker(ctx, style);
    expect(calls.some((call) => call.op === "fill")).toBe(true);
    expect(calls.some((call) => call.op === "fillText")).toBe(false);
    expect(
      calls.some((call) => call.op === "stroke" && call.dash.some((value) => value > 0)),
    ).toBe(false);
  });

  it("候选为虚线空心环 + 问号，与已确认明显区分", () => {
    const style = describeHotspotMarker("candidate", false);
    expect(style).toEqual({
      status: "candidate",
      selected: false,
      shape: "dashed-ring",
      glyph: "?",
      outerRing: false,
    });
    const { ctx, calls } = recordingContext();
    paintHotspotMarker(ctx, style);
    const dashed = calls.find(
      (call) => call.op === "stroke" && call.dash.some((value) => value > 0),
    );
    expect(dashed, "候选必须有虚线描边").toBeTruthy();
    expect(calls.some((call) => call.op === "fillText" && call.text === "?")).toBe(true);
  });

  it("选中在两种形态上都加外环（3px 焦点环 + 纸色隔离带）", () => {
    for (const status of ["confirmed", "candidate"] as const) {
      const style = describeHotspotMarker(status, true);
      expect(style.outerRing).toBe(true);
      const { ctx, calls } = recordingContext();
      paintHotspotMarker(ctx, style);
      const strokes = calls.filter((call) => call.op === "stroke" && call.dash.length === 0);
      expect(strokes.length, `${status} 选中应有两段外环`).toBeGreaterThanOrEqual(2);
      expect(strokes.some((call) => call.op === "stroke" && call.lineWidth >= 5)).toBe(true);
    }
  });

  it("未知/缺失状态不得被当成候选（画成已确认，不冒充待复核）", () => {
    expect(describeHotspotMarker(undefined, false).status).toBe("confirmed");
    expect(describeHotspotMarker("unbound", false).status).toBe("confirmed");
    expect(describeHotspotMarker("stale", false).status).toBe("confirmed");
  });

  it("标记世界尺寸随模型最大边长等比（屏幕占比稳定）", () => {
    const small = hotspotMarkerSpriteSize(1);
    const large = hotspotMarkerSpriteSize(4);
    expect(large).toBeCloseTo(small * 4, 5);
    expect(hotspotMarkerSpriteSize(0)).toBeGreaterThan(0);
    expect(hotspotMarkerSpriteSize(Number.NaN)).toBeGreaterThan(0);
  });
});
