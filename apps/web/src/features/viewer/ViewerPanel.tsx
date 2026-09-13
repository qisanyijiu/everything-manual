/**
 * 3D 阅读器面板（T18 / REQ-032；PRD UI-043/UI-044/UI-045；§6.1.3 中栏）。
 *
 * 与 3D 运行时的边界：本文件**不 import three**，只做
 *  1) 模型字节获取（真实 `GET /assets/{id}/content`，带进度）；
 *  2) WebGL 可用性探测（不可用时直接给出可读原因 + 文字阅读入口，不挂载 Canvas）；
 *  3) 工具栏（复位视角 / 适配模型 / 立即重建）与状态行（`role="status"`）；
 *  4) 通过 `lazy()` 加载真正的 3D 模块（`ViewerStage`，three 在独立 chunk）。
 *
 * 状态机（面板状态行与 e2e 断言按此）：
 *   noModel / unavailable → fetching(进度) → ready(交给 ViewerStage 解析/渲染)
 *   ready 期间：ok ⇄ lost(「3D 显示已中断，正在重建…」+ 「立即重建」，交互禁用)
 *   lost 点击「立即重建」→ restoring(同上文案；新 Canvas 挂载后由舞台上报 ok 收敛回
 *   ok；重建前捕获的相机位姿由新舞台套用) —— BUG-007：两条恢复路径都必须清除
 *   `unavailableTimer` 并把交互还回可用态，不得停在 lost/不可用文案。
 *   lost/restoring 超过 `CONTEXT_UNAVAILABLE_AFTER_MS` 仍未收到 ok → 「浏览器 3D 上下文不可用」
 *   失败（HTTP/哈希/解析）→ 可读原因 + 「重试加载」
 *
 * 「文本与 PDF 阅读不依赖 WebGL 成功」：本面板的任何失败都不影响左栏部件/步骤与
 * 右栏原文（它们不经过 WebGL）；面板只在自身区域内显示失败。
 */

import { Suspense, lazy, useCallback, useEffect, useRef, useState } from "react";

import { fetchAssetContent } from "../../api/endpoints";
import { describeError } from "../../api/client";
import {
  CONTEXT_LOST_MESSAGE,
  CONTEXT_RESTORED_MESSAGE,
  WEBGL_UNAVAILABLE_MESSAGE,
  probeWebgl,
  type WebglContextState,
} from "./webgl";
import type { CameraPose } from "./coordinates";
import type { ViewerModelInfo } from "./bridge";
import type { ViewerHotspotView, ViewerPickResult, ViewerStageApi } from "./ViewerStage";

/** three 所在 chunk：只在真正要渲染模型时下载（资料库首屏不含它）。 */
const ViewerStage = lazy(() =>
  import("./ViewerStage").then((module) => ({ default: module.ViewerStage })),
);

/** 丢失后多久仍没有 restored 就判定"浏览器上下文不可用"（UI-044 的连续失败路径）。 */
const CONTEXT_UNAVAILABLE_AFTER_MS = 8_000;
/** 「3D 显示已恢复」提示保留时间。 */
const RESTORED_NOTICE_MS = 4_000;

export interface ViewerPanelModel {
  readonly assetId: string;
  readonly revisionId: string;
  readonly sha256: string;
}

export interface ViewerPanelProps {
  /** 可用模型（草稿知识里的 `model`）；null = 没有可用模型（空态，不是错误）。 */
  readonly model: ViewerPanelModel | null;
  readonly hotspots: readonly ViewerHotspotView[];
  /** 文字阅读入口（3D 不可用/失败时把焦点交给文字面板；UI-044/UI-045）。 */
  readonly onUseTextPath?: () => void;
  readonly onModelReady?: (info: ViewerModelInfo) => void;
  readonly onPoseChange?: (pose: CameraPose | null) => void;
  /** 拾取模式（校准；UI-047）。 */
  readonly pickMode?: boolean;
  /** 选中的热点 id（与部件列表联动；UI-046）。 */
  readonly selectedHotspotId?: string | null;
  /** 人工直接拾取回调（仅 pickMode 下由舞台触发）。 */
  readonly onPick?: (pick: ViewerPickResult) => void;
  readonly onHotspotSelect?: (hotspotId: string) => void;
  /**
   * 舞台 API 的外部引用（校准工作区用它取/套用步骤视角、把热点居中）。
   * 传入时由调用方持有生命周期；不传则面板内部自持（只读用法）。
   */
  readonly apiRef?: { current: ViewerStageApi | null };
}

type LoadState =
  | { readonly phase: "noModel" }
  | { readonly phase: "unavailable"; readonly message: string }
  | { readonly phase: "fetching"; readonly received: number; readonly total: number | null }
  | { readonly phase: "ready"; readonly buffer: ArrayBuffer }
  | { readonly phase: "error"; readonly message: string };

export function ViewerPanel({
  model,
  hotspots,
  onUseTextPath,
  onModelReady,
  onPoseChange,
  pickMode,
  selectedHotspotId,
  onPick,
  onHotspotSelect,
  apiRef,
}: ViewerPanelProps) {
  const [state, setState] = useState<LoadState>({ phase: "noModel" });
  const [attempt, setAttempt] = useState(0);
  const [contextState, setContextState] = useState<WebglContextState>("ok");
  const [contextUnusable, setContextUnusable] = useState(false);
  const [restoredNotice, setRestoredNotice] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [stageReady, setStageReady] = useState(false);
  const internalApi = useRef<ViewerStageApi | null>(null);
  const stageApi = apiRef ?? internalApi;
  const previousContext = useRef<WebglContextState>("ok");
  const unavailableTimer = useRef<number | null>(null);
  const restoredTimer = useRef<number | null>(null);
  /** 「立即重建」前捕获的相机位姿（asset-root 局部坐标）；新舞台就绪后套用并清空。 */
  const restorePoseRef = useRef<CameraPose | null>(null);

  const clearTimers = useCallback(() => {
    if (unavailableTimer.current !== null) {
      window.clearTimeout(unavailableTimer.current);
      unavailableTimer.current = null;
    }
    if (restoredTimer.current !== null) {
      window.clearTimeout(restoredTimer.current);
      restoredTimer.current = null;
    }
  }, []);

  useEffect(() => clearTimers, [clearTimers]);

  /**
   * 启动/重置「多久没收到 ok 就判上下文不可用」的兜底计时。
   *
   * 生命周期（BUG-007 的修复点）：进入 lost 或点击「立即重建」时启动；**只要舞台
   * 上报 ok 就立即清除**——否则已恢复的显示会在 8 秒后被误报为「浏览器 3D 上下文
   * 不可用」，且该误报会把工具栏按钮永久锁死。
   */
  const startUnavailableTimer = useCallback(() => {
    if (unavailableTimer.current !== null) {
      window.clearTimeout(unavailableTimer.current);
    }
    unavailableTimer.current = window.setTimeout(() => {
      unavailableTimer.current = null;
      setContextUnusable(true);
    }, CONTEXT_UNAVAILABLE_AFTER_MS);
  }, []);

  // --- 模型字节获取（WebGL 不可用时不下载浪费） ---------------------------------
  //
  // 依赖**模型身份**（assetId/revisionId/sha256）而不是对象引用：草稿的其它字段
  // （热点、复核、视角）更新时模型对象会被重新构造，若按对象引用重取字节，3D 场景
  // 会在每次校准写入后整体重挂载（丢失选中/拾取状态）。身份相同 = 同一份字节。
  const modelAssetId = model?.assetId ?? null;
  const modelSha256 = model?.sha256 ?? null;
  useEffect(() => {
    if (modelAssetId === null || modelSha256 === null) {
      setState({ phase: "noModel" });
      return;
    }
    if (probeWebgl() === "unavailable") {
      setState({ phase: "unavailable", message: WEBGL_UNAVAILABLE_MESSAGE });
      return;
    }
    const controller = new AbortController();
    setLoadError(null);
    setStageReady(false);
    setState({ phase: "fetching", received: 0, total: null });
    fetchAssetContent(modelAssetId, {
      signal: controller.signal,
      onProgress: (received, total) => {
        setState((current) =>
          current.phase === "fetching" ? { phase: "fetching", received, total } : current,
        );
      },
    })
      .then((response) => {
        if (!controller.signal.aborted) {
          setState({ phase: "ready", buffer: response.bytes });
        }
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) {
          return;
        }
        setState({ phase: "error", message: describeError(error).message });
      });
    return () => {
      controller.abort();
    };
  }, [modelAssetId, modelSha256, attempt]);

  // 3D 模块解析失败（哈希不符/外链资源/解析失败）→ 面板错误态（可重试）。
  const handleLoadError = useCallback((error: { code: string; message: string }) => {
    setLoadError(error.message);
    setStageReady(false);
  }, []);

  const handleContextState = useCallback(
    (next: WebglContextState) => {
      const previous = previousContext.current;
      previousContext.current = next;
      setContextState(next);
      if (next === "lost") {
        setContextUnusable(false);
        startUnavailableTimer();
        return;
      }
      // ok（含「立即重建」换新上下文后的首个上报）：清除 8 秒兜底计时——
      // 上下文已可用，再做"不可用"判定就是误报（BUG-007）。
      if (unavailableTimer.current !== null) {
        window.clearTimeout(unavailableTimer.current);
        unavailableTimer.current = null;
      }
      if (next === "ok") {
        setContextUnusable(false);
      }
      if (previous === "lost" && next === "ok") {
        // 自动 restored 分支：保留短暂的「3D 显示已恢复。」提示（手动重建分支直接
        // 回到就绪文案，不多一次提示，避免把可用态延后）。
        setRestoredNotice(true);
        if (restoredTimer.current !== null) {
          window.clearTimeout(restoredTimer.current);
        }
        restoredTimer.current = window.setTimeout(() => {
          restoredTimer.current = null;
          setRestoredNotice(false);
        }, RESTORED_NOTICE_MS);
      }
    },
    [startUnavailableTimer],
  );

  const retry = useCallback(() => {
    setAttempt((value) => value + 1);
  }, []);

  /**
   * 「立即重建」：重新挂载 Canvas（新上下文），不是刷新页面（UI-044）。
   *
   * 面板侧的动作（BUG-007）：捕获当前相机位姿 → 清除计时与不可用标记 → 进入
   * `restoring`（交互保持禁用、状态行仍显示"正在重建…"，直到新舞台上报 ok）→
   * 为新一次重建启动兜底计时（重建失败仍会给出"上下文不可用"，不会静默卡住）。
   */
  const rebuild = useCallback(() => {
    const api = stageApi.current;
    restorePoseRef.current = api?.pose() ?? null;
    clearTimers();
    setContextUnusable(false);
    setRestoredNotice(false);
    setStageReady(false);
    previousContext.current = "restoring";
    setContextState("restoring");
    startUnavailableTimer();
    api?.rebuild();
    // `stageApi` 是稳定的 ref（外部传入或内部自持）；不作为依赖是刻意的：
    // 重建不应因 ref 对象身份变化而重新创建回调（BUG-007 的 timer 生命周期已固定）。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clearTimers, startUnavailableTimer]);

  const ready = state.phase === "ready" && loadError === null;
  const lost = contextState === "lost";
  /** 「立即重建」已发起、新舞台尚未上报 ok：交互同样禁用（UI-044「重建中禁用交互」）。 */
  const restoring = contextState === "restoring";
  const interactive = ready && stageReady && !lost && !restoring;
  const status = statusText({
    state,
    contextState,
    contextUnusable,
    restoredNotice,
    loadError,
    stageReady,
  });

  return (
    <section className="viewer-panel" aria-label="3D 模型阅读器">
      <div className="viewer-panel__toolbar" role="group" aria-label="模型视角操作">
        <button type="button" disabled={!interactive} onClick={() => stageApi.current?.reset()}>
          复位视角
        </button>
        <button type="button" disabled={!interactive} onClick={() => stageApi.current?.fit()}>
          适配模型
        </button>
        {(lost || contextUnusable) && (
          <button type="button" onClick={rebuild}>
            立即重建
          </button>
        )}
      </div>

      <div className="viewer-panel__stage" data-testid="viewer-stage">
        {state.phase === "noModel" && (
          <p className="viewer-panel__message" data-testid="viewer-empty">
            该草稿没有可用的模型版本（模型分支未完成或未通过校验）；部件、步骤与原文仍可阅读。
          </p>
        )}
        {state.phase === "unavailable" && (
          <div className="viewer-panel__message" data-testid="viewer-unavailable">
            <p>{state.message}</p>
            {onUseTextPath !== undefined && (
              <button type="button" onClick={onUseTextPath}>
                改用文字阅读
              </button>
            )}
          </div>
        )}
        {state.phase === "fetching" && (
          <p className="viewer-panel__message" role="status" data-testid="viewer-loading">
            正在加载模型…
            {state.total !== null && state.total > 0
              ? `（${Math.min(100, Math.round((state.received / state.total) * 100))}%）`
              : `（已接收 ${state.received} 字节）`}
          </p>
        )}
        {state.phase === "error" && (
          <div className="viewer-panel__message" data-testid="viewer-error">
            <p role="alert">模型加载失败：{state.message}</p>
            <button type="button" onClick={retry}>
              重试加载
            </button>
            {onUseTextPath !== undefined && (
              <button type="button" onClick={onUseTextPath}>
                改用文字阅读
              </button>
            )}
          </div>
        )}
        {state.phase === "ready" && loadError !== null && (
          <div className="viewer-panel__message" data-testid="viewer-error">
            <p role="alert">模型无法显示：{loadError}</p>
            <button type="button" onClick={retry}>
              重试加载
            </button>
            {onUseTextPath !== undefined && (
              <button type="button" onClick={onUseTextPath}>
                改用文字阅读
              </button>
            )}
          </div>
        )}
        {state.phase === "ready" && loadError === null && (
          <Suspense
            fallback={
              <p className="viewer-panel__message" role="status" data-testid="viewer-loading">
                正在加载 3D 模块…
              </p>
            }
          >
            <ViewerStage
              buffer={state.buffer}
              model={model as ViewerPanelModel}
              hotspots={hotspots}
              apiRef={stageApi}
              restorePoseRef={restorePoseRef}
              onModelReady={(info) => {
                setStageReady(true);
                onModelReady?.(info);
              }}
              onLoadError={handleLoadError}
              onContextState={handleContextState}
              onPoseChange={onPoseChange}
              pickMode={pickMode}
              selectedHotspotId={selectedHotspotId}
              onPick={onPick}
              onHotspotSelect={onHotspotSelect}
            />
          </Suspense>
        )}
      </div>

      <p className="viewer-panel__status" role="status" data-testid="viewer-status">
        {status}
      </p>
      <p className="viewer-panel__notice">
        模型为外观资产，不表示机械结构；拖动旋转、滚轮缩放，键盘可用上方按钮完成等效操作。
      </p>
    </section>
  );
}

function statusText({
  state,
  contextState,
  contextUnusable,
  restoredNotice,
  loadError,
  stageReady,
}: {
  state: LoadState;
  contextState: WebglContextState;
  contextUnusable: boolean;
  restoredNotice: boolean;
  loadError: string | null;
  stageReady: boolean;
}): string {
  if (contextState === "lost" || contextState === "restoring") {
    // restoring 与 lost 同一文案：重建期间不得谎报可用，也不提前报"不可用"。
    return contextUnusable ? "浏览器 3D 上下文不可用：请使用文字阅读。" : CONTEXT_LOST_MESSAGE;
  }
  if (restoredNotice) {
    return CONTEXT_RESTORED_MESSAGE;
  }
  switch (state.phase) {
    case "noModel":
      return "没有可显示的模型（仅文字与原文可用）。";
    case "unavailable":
      return "3D 不可用：请使用文字阅读。";
    case "fetching":
      return "正在加载模型…";
    case "error":
      return "模型加载失败。";
    case "ready":
      if (loadError !== null) {
        return "模型无法显示。";
      }
      return stageReady ? "模型已加载，可旋转/缩放。" : "正在准备 3D 场景…";
    default:
      return "";
  }
}
