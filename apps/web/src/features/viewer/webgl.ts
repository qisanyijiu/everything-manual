/**
 * WebGL 可用性探测与上下文状态（T18 / REQ-032；PRD UI-044）。
 *
 * 为什么先探测再挂 canvas：R3F/three 在拿不到 WebGL 上下文时会在挂载阶段抛错，
 * 那只给用户一个通用错误边界；探测后我们可以给出**可读原因 + 文字阅读入口**
 * （UI-044「失败：显示「浏览器 3D 上下文不可用」+ 文字阅读入口」），并且**不**
 * 影响同一页面的文字/PDF 阅读（UI-045）。
 *
 * 本模块不 import three（探测只用原生 canvas API），可被非 3D 代码安全引用。
 */

export type WebglAvailability = "webgl2" | "webgl1" | "unavailable";

let cachedAvailability: WebglAvailability | null = null;

/**
 * 探测 WebGL 可用性（每份文档只探测一次并缓存）。
 *
 * 缓存理由：探测要创建一个临时（并立即释放）的 WebGL 上下文，而面板在换模型时
 * 会重复走这条路径；同一份文档内结果不会变化，重复探测只会浪费上下文名额。
 */
export function probeWebgl(): WebglAvailability {
  cachedAvailability ??= probeWebglOnce();
  return cachedAvailability;
}

/** 探测 WebGL 可用性：用一个临时 canvas 尝试取上下文，随后立即丢弃它。 */
function probeWebglOnce(): WebglAvailability {
  if (typeof document === "undefined") {
    return "unavailable";
  }
  let canvas: HTMLCanvasElement | null = null;
  try {
    canvas = document.createElement("canvas");
    const gl2 = canvas.getContext("webgl2");
    if (gl2 !== null) {
      return "webgl2";
    }
    const gl1 = canvas.getContext("webgl");
    if (gl1 !== null) {
      return "webgl1";
    }
  } catch {
    // jsdom 等环境没有 WebGL 实现：按"不可用"处理，不把异常抛给页面。
    return "unavailable";
  } finally {
    if (canvas !== null) {
      // 释放探测用的上下文（部分实现需要显式 loseContext 才能回收）。
      try {
        const context = canvas.getContext("webgl2") ?? canvas.getContext("webgl");
        const extension = (context as WebGLRenderingContext | null)?.getExtension(
          "WEBGL_lose_context",
        );
        extension?.loseContext();
      } catch {
        // 释放失败不影响判定。
      }
    }
  }
  return "unavailable";
}

/** WebGL 上下文生命周期（面板状态行与测试断言按这组取值）。 */
export type WebglContextState = "ok" | "lost" | "restoring" | "unavailable";

export const WEBGL_UNAVAILABLE_MESSAGE =
  "浏览器 3D 上下文不可用：此环境没有可用的 WebGL，模型无法显示。文字与 PDF 阅读不受影响。";

export const CONTEXT_LOST_MESSAGE = "3D 显示已中断，正在重建…";

export const CONTEXT_RESTORED_MESSAGE = "3D 显示已恢复。";
