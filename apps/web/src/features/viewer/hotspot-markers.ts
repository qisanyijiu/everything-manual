/**
 * 热点标记的视觉语法（设计冻结：`design/vs-01/reader-specimen.html` R-S4 与
 * `component-states.md` §6；视觉 PRD 修订 1 / UI 修订 1）。
 *
 * 冻结语义——候选与已确认**不得视觉混同**：
 * - 已确认（confirmed）：实心圆（`--color-accent` 实底）；
 * - 候选（candidate）：虚线空心环 + 问号（surface 底 + accent 虚线描边 + accent 问号）；
 * - 选中（selected）：在上述形态外再加一圈 3px 焦点环（不闪烁）；
 * - 无热点：模型区不画标记（调用方过滤，不在本模块）。
 *
 * 标记不承载编号：产品数据里没有"热点编号"实体，禁止伪造。列表侧的状态牌
 * （「热点 N」/「候选（待复核）」/「无热点」）才是文字载体。
 *
 * 取值来自 tokens（theme.css 中同一数值）；本模块不依赖 React / three，画布绘制
 * 与 three 贴图创建分离，便于在 jsdom 下对"视觉语法"做纯函数断言。
 */

/** 仅区分两种可显示状态；其余（unbound/stale/未知）由调用方过滤，不画在模型上。 */
export type HotspotMarkerStatus = "candidate" | "confirmed";

export interface HotspotMarkerStyle {
  readonly status: HotspotMarkerStatus;
  readonly selected: boolean;
  /** 已确认 = 实心圆；候选 = 虚线空心环。 */
  readonly shape: "solid" | "dashed-ring";
  /** 候选标记中心的问题号；已确认为 null（没有真实编号不伪造数字）。 */
  readonly glyph: "?" | null;
  /** 选中时额外绘制的外圈（3px 焦点环 + 纸色隔离带）。 */
  readonly outerRing: boolean;
}

/** 归一化热点状态：只有明确的 "candidate" 才画候选；其余可显示状态按已确认。 */
export function normalizeHotspotStatus(status: string | undefined): HotspotMarkerStatus {
  return status === "candidate" ? "candidate" : "confirmed";
}

/** 纯函数：热点 → 标记视觉语法（供 3D 标记、测试与离线阅读器共用同一判据）。 */
export function describeHotspotMarker(
  status: string | undefined,
  selected: boolean,
): HotspotMarkerStyle {
  const normalized = normalizeHotspotStatus(status);
  return {
    status: normalized,
    selected,
    shape: normalized === "candidate" ? "dashed-ring" : "solid",
    glyph: normalized === "candidate" ? "?" : null,
    outerRing: selected,
  };
}

/** token 取色（theme.css 同名取值；离线 HTML 内联同一份）。 */
export const HOTSPOT_MARKER_COLORS = {
  confirmedFill: "#A63F21", // --color-accent
  candidateFill: "#FFFEF9", // --color-surface
  candidateStroke: "#A63F21", // --color-accent
  candidateGlyph: "#A63F21", // --color-accent
  selectedRing: "#A63F21", // --focus-ring-color
  ringGap: "#F4F1E8", // --color-paper（外环与标记之间的隔离带）
} as const;

/** 标记贴图的画布边长（2 的幂，便于上传）。 */
export const HOTSPOT_MARKER_TEXTURE_SIZE = 128;

/** 实心圆/虚线环直径占贴图边长的比例（其余留给选中外环的绘制空间）。 */
const DISC_RADIUS_RATIO = 0.3125; // 40 / 128

/**
 * 标记贴图的世界尺寸：让圆的直径约为模型最大边长的 `discFraction`，
 * 使标记屏幕占比在不同模型尺寸下保持一致（模型尺寸来自 asset-root 局部包围盒）。
 */
export function hotspotMarkerSpriteSize(maxDimension: number, discFraction = 0.06): number {
  const safe = Number.isFinite(maxDimension) && maxDimension > 0 ? maxDimension : 1;
  return (safe * discFraction) / (DISC_RADIUS_RATIO * 2);
}

/** 画布上下文的最小接口（jsdom 无 2D 上下文；测试用桩对象即可断言语法）。 */
export interface MarkerCanvasContext {
  clearRect(x: number, y: number, w: number, h: number): void;
  beginPath(): void;
  arc(x: number, y: number, r: number, start: number, end: number): void;
  fill(): void;
  stroke(): void;
  fillText(text: string, x: number, y: number): void;
  setLineDash(segments: number[]): void;
  // 与真实 CanvasRenderingContext2D 兼容：仅使用纯字符串取色。
  fillStyle: string | CanvasGradient | CanvasPattern;
  strokeStyle: string | CanvasGradient | CanvasPattern;
  lineWidth: number;
  font: string;
  textAlign: string;
  textBaseline: string;
}

/**
 * 把标记语法画进 2D 上下文（画布贴图）。纯绘制，不触碰 DOM/three。
 * 坐标为贴图像素；中心 `size / 2`。
 */
export function paintHotspotMarker(
  ctx: MarkerCanvasContext,
  style: HotspotMarkerStyle,
  size = HOTSPOT_MARKER_TEXTURE_SIZE,
): void {
  const center = size / 2;
  const radius = size * DISC_RADIUS_RATIO;
  ctx.clearRect(0, 0, size, size);

  // 选中外圈：先画纸色隔离带再画焦点环，使外环与标记本体可分辨。
  if (style.outerRing) {
    ctx.beginPath();
    ctx.arc(center, center, radius + size * 0.11, 0, Math.PI * 2);
    ctx.strokeStyle = HOTSPOT_MARKER_COLORS.ringGap;
    ctx.lineWidth = size * 0.09;
    ctx.setLineDash([]);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(center, center, radius + size * 0.15, 0, Math.PI * 2);
    ctx.strokeStyle = HOTSPOT_MARKER_COLORS.selectedRing;
    ctx.lineWidth = size * 0.047; // ≈ 3px @ 64px 显示尺寸
    ctx.setLineDash([]);
    ctx.stroke();
  }

  if (style.shape === "dashed-ring") {
    // 候选：surface 实底 + accent 虚线环 + 问号。
    ctx.beginPath();
    ctx.arc(center, center, radius, 0, Math.PI * 2);
    ctx.fillStyle = HOTSPOT_MARKER_COLORS.candidateFill;
    ctx.fill();
    ctx.beginPath();
    ctx.arc(center, center, radius - size * 0.02, 0, Math.PI * 2);
    ctx.strokeStyle = HOTSPOT_MARKER_COLORS.candidateStroke;
    ctx.lineWidth = size * 0.055;
    ctx.setLineDash([size * 0.1, size * 0.07]);
    ctx.stroke();
    ctx.setLineDash([]);
    if (style.glyph !== null) {
      ctx.fillStyle = HOTSPOT_MARKER_COLORS.candidateGlyph;
      ctx.font = `600 ${Math.round(size * 0.42)}px sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(style.glyph, center, center + size * 0.015);
    }
    return;
  }

  // 已确认：实心圆（无数字——没有真实编号不伪造）。
  ctx.setLineDash([]);
  ctx.beginPath();
  ctx.arc(center, center, radius, 0, Math.PI * 2);
  ctx.fillStyle = HOTSPOT_MARKER_COLORS.confirmedFill;
  ctx.fill();
}
