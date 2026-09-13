/**
 * 阅读器的**只读可观测桥**（`window.__EM_VIEWER__`；T18）。
 *
 * 用途（为什么不是"测试专用后门"）：
 * - QA / Playwright / 人工走查需要**可观察证据**：当前相机位姿、已渲染帧数、
 *   夹在模型上的锚点世界坐标、资源账本、上下文状态。这些量无法从 DOM 稳定读出
 *   （截图不能证明"锚点没漂"、"资源被释放"）。
 * - 桥只暴露**读操作**：没有修改相机、没有写锚点、没有强制丢失上下文。测试要
 *   模拟上下文丢失用浏览器的 `WEBGL_lose_context` 扩展（真实路径），不经过本桥。
 * - 不暴露任何凭据、资产字节、token：只有几何/位姿/计数的投影。
 *
 * 生命周期：随 3D 模块（chunk）加载而出现；卸载后仍可读资源账本（用于断言"释放"），
 * 与组件相关的字段返回 null。
 */

import { viewerResourceStats, type ViewerResourceStats } from "./resources";
import type { Bounds, CameraPose, Vec3 } from "./coordinates";
import type { WebglContextState } from "./webgl";

export interface ViewerModelInfo {
  readonly assetId: string;
  readonly revisionId: string;
  readonly sha256: string;
  readonly triangles: number;
  readonly objects: number;
  readonly textures: number;
  readonly bounds: Bounds;
}

export interface ViewerAnchorProjection {
  readonly id: string;
  readonly partId: string;
  /** asset-root 局部坐标（保存下来的那一个）。 */
  readonly local: Vec3;
  /** 同一局部点在当前显示变换下的世界坐标（用于断言"不漂移"）。 */
  readonly world: Vec3;
}

/** 单个局部点的屏幕投影（拾取验证与"热点居中"的可观察证据）。 */
export interface ViewerProjection {
  /** NDC 坐标（[-1,1]，z 供可见性判断）。 */
  readonly ndc: [number, number, number];
  /** 相对 canvas 的屏幕像素坐标。 */
  readonly screen: [number, number];
  /** 是否在相机前方/视锥内（z <= 1）。 */
  readonly visible: boolean;
}

/** 最近一次人工拾取（局部点 + 世界点 + 屏幕坐标）。 */
export interface ViewerPickProjection {
  readonly local: Vec3;
  readonly world: Vec3;
  readonly screen: { readonly x: number; readonly y: number };
}

export interface RoundTripResult {
  readonly local: Vec3;
  readonly world: Vec3;
  readonly back: Vec3;
  /** `|back - local|`；测试断言它足够小（浮点误差量级）。 */
  readonly error: number;
}

/** 视口尺寸（`CameraPose` 不含视口；T19 保存视角/回放时需要同一份取值）。 */
export interface ViewerViewport {
  readonly width: number;
  readonly height: number;
  readonly aspect: number;
  readonly pixelRatio: number;
}

export interface ViewerBridge {
  readonly version: 1;
  frames(): number;
  contextState(): WebglContextState;
  model(): ViewerModelInfo | null;
  cameraPose(): CameraPose | null;
  viewport(): ViewerViewport | null;
  anchors(): readonly ViewerAnchorProjection[];
  roundTrip(local: Vec3): RoundTripResult | null;
  localBounds(): Bounds | null;
  /** asset-root 局部点 → 屏幕像素（T19：点击位置正确性的可观察证据）。 */
  project(local: Vec3): ViewerProjection | null;
  /** 最近一次人工拾取（未拾取过为 null）。 */
  lastPick(): ViewerPickProjection | null;
  /** 是否处于拾取模式。 */
  picking(): boolean;
  stats(): ViewerResourceStats & { frames: number };
}

interface ViewerBridgeHandlers {
  frames: () => number;
  contextState: () => WebglContextState;
  model: () => ViewerModelInfo | null;
  cameraPose: () => CameraPose | null;
  viewport: () => ViewerViewport | null;
  anchors: () => readonly ViewerAnchorProjection[];
  roundTrip: (local: Vec3) => RoundTripResult | null;
  localBounds: () => Bounds | null;
  project: (local: Vec3) => ViewerProjection | null;
  lastPick: () => ViewerPickProjection | null;
  picking: () => boolean;
  stats: () => ViewerResourceStats;
}

declare global {
  interface Window {
    /** 见 `bridge.ts`：只读可观测桥（无副作用）。 */
    __EM_VIEWER__?: ViewerBridge;
  }
}

let handlers: ViewerBridgeHandlers | null = null;

function currentBridge(): ViewerBridge {
  return {
    version: 1,
    frames: () => handlers?.frames() ?? 0,
    contextState: () => handlers?.contextState() ?? "unavailable",
    model: () => handlers?.model() ?? null,
    cameraPose: () => handlers?.cameraPose() ?? null,
    viewport: () => handlers?.viewport() ?? null,
    anchors: () => handlers?.anchors() ?? [],
    roundTrip: (local) => handlers?.roundTrip(local) ?? null,
    localBounds: () => handlers?.localBounds() ?? null,
    project: (local) => handlers?.project(local) ?? null,
    lastPick: () => handlers?.lastPick() ?? null,
    picking: () => handlers?.picking() ?? false,
    stats: () => {
      // 账本在模块作用域：3D 组件卸载后仍可读（"卸载后资源被释放"的可观察证据）。
      const stats: ViewerResourceStats = handlers?.stats() ?? viewerResourceStats();
      return { ...stats, frames: handlers?.frames() ?? 0 };
    },
  };
}

/** 挂载 3D 场景时注册读取器（同一时刻只有一个场景）。 */
export function installViewerBridge(next: ViewerBridgeHandlers): void {
  handlers = next;
  if (typeof window !== "undefined") {
    window.__EM_VIEWER__ = currentBridge();
  }
}

/** 卸载 3D 场景：清空与组件相关的读取器，保留 `stats()`（账本在模块作用域）。 */
export function clearViewerBridgeHandlers(): void {
  handlers = null;
}
