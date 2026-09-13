/**
 * 3D 舞台（T18 / REQ-032；PRD UI-043/UI-044；architecture §3「3D 模块懒加载」、§5.4）。
 *
 * 本文件是**唯一**直接依赖 three / R3F 的入口，由 `ViewerPanel` 通过 `lazy()` 动态
 * 加载：资料库与其它页面的首屏包不含 three（构建产物按 chunk 拆分，见实现记录）。
 *
 * 职责与不变量（QA 按此复核）：
 * - **asset-root 局部坐标**：`gltf.scene` 即 asset-root；锚点、相机位姿都用它的
 *   局部坐标读写（`asset-root.ts`），显示居中/缩放只作用于**外层 display group**。
 * - **fit / reset 语义明确**：
 *   * `fit()`：按当前视口比例重新取景（保持当前观察方向），模型始终完整可见；
 *   * `reset()`：回到模型加载时的初始取景（正面方向 + 当时的距离）。
 * - **上下文丢失/恢复**：监听 canvas 的 `webglcontextlost` / `webglcontextrestored`；
 *   丢失期间禁用交互、显示状态（由面板渲染），恢复后 three 重建 GL 资源并继续渲染；
 *   「立即重建」= 重新挂载 Canvas（换一个新上下文），不是刷新页面。
 * - **资源释放**：模型卸载/切换时 dispose geometry/material/texture（账本见
 *   `resources.ts`），旧资源与旧热点都不会串进新模型。
 */

import { Canvas, useFrame, useThree } from "@react-three/fiber";
import { useCallback, useEffect, useRef, useState } from "react";
import { Group, Raycaster, Vector2, Vector3, type PerspectiveCamera } from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";

import { createAssetRoot, type AssetRoot } from "./asset-root";
import {
  clearViewerBridgeHandlers,
  installViewerBridge,
  type ViewerAnchorProjection,
  type ViewerModelInfo,
} from "./bridge";
import {
  applyCameraPose,
  computeFit,
  readCameraPose,
  type Bounds,
  type CameraPose,
  type FitResult,
  type Vec3,
} from "./coordinates";
import { ViewerError, loadGlbModel, summarizeScene, type LoadedModel } from "./glb";
import { viewerResourceStats } from "./resources";
import type { WebglContextState } from "./webgl";

export interface ViewerHotspotView {
  readonly id: string;
  readonly partId: string;
  readonly positionLocal: Vec3;
}

export interface ViewerStageModel {
  readonly assetId: string;
  readonly revisionId: string;
  readonly sha256: string;
}

export interface ViewerStageApi {
  /** 回到初始取景（加载时记录的相机位姿）。 */
  reset(): void;
  /** 按当前视口比例重新取景（保持当前方向）。 */
  fit(): void;
  /** 立即重建：重新挂载 Canvas（获取新的 WebGL 上下文）。 */
  rebuild(): void;
  /** 当前相机位姿（asset-root 局部坐标；未加载时 null）。 */
  pose(): CameraPose | null;
  /** 套用一份 asset-root 局部位姿（步骤视角「回到该视角」；UI-050）。 */
  applyPose(pose: CameraPose): void;
  /** 把某个 asset-root 局部点移到视口中心（部件列表 ↔ 3D 热点联动的「居中」）。 */
  focusLocal(local: Vec3): void;
}

/** 一次拾取（人工直接拾取建点）。 */
export interface ViewerPickResult {
  /** asset-root 局部坐标（保存锚点用这一份）。 */
  readonly local: Vec3;
  /** 同一命中点的世界坐标（投影/调试用）。 */
  readonly world: Vec3;
  /** 命中点到视口的屏幕像素坐标（测试可观察证据）。 */
  readonly screen: { x: number; y: number };
}

export interface ViewerStageProps {
  readonly buffer: ArrayBuffer;
  readonly model: ViewerStageModel;
  readonly hotspots: readonly ViewerHotspotView[];
  readonly onModelReady?: (info: ViewerModelInfo) => void;
  readonly onLoadError?: (error: { code: string; message: string }) => void;
  readonly onContextState?: (state: WebglContextState) => void;
  readonly onPoseChange?: (pose: CameraPose | null) => void;
  /** 拾取模式（校准）：点击模型表面 = 建点；关闭时点击只做选中/旋转（UI-047/UI-048）。 */
  readonly pickMode?: boolean;
  /** 当前选中的热点 id（与左栏部件列表联动；UI-046）。 */
  readonly selectedHotspotId?: string | null;
  /** 人工直接拾取回调（局部点 + 世界点 + 屏幕坐标；仅 pickMode 下触发）。 */
  readonly onPick?: (pick: ViewerPickResult) => void;
  /** 点选 3D 热点标记（选中对应部件；只读模式也可用）。 */
  readonly onHotspotSelect?: (hotspotId: string) => void;
  readonly apiRef: { current: ViewerStageApi | null };
  /** 观察方向（世界坐标；默认正面）。reset 会回到同一方向。 */
  readonly initialDirection?: Vec3;
  /**
   * 「立即重建」前的相机位姿（asset-root 局部坐标；BUG-007 修复）：
   * 新挂载的舞台在模型就绪后**消费一次**（置回 null）并套用该位姿，使手动重建与
   * 自动 restored 分支一样保留当前视角；为 null 时按初始取景（保持旧行为）。
   */
  readonly restorePoseRef?: { current: CameraPose | null };
}

const FIT_MARGIN = 1.35;
const DEFAULT_FOV = 40;
const HOTSPOT_RADIUS = 0.035;
const HOTSPOT_COLOR = "#b45309";
const HOTSPOT_SELECTED_COLOR = "#1d4ed8";
const DEFAULT_DIRECTION: Vec3 = [0, 0, 1];

/** 点击判定阈值：位移超过它（或按住超过它）就不是"点击"，只旋转相机（UI-048）。 */
const CLICK_MAX_MOVE_PX = 6;
const CLICK_MAX_DURATION_MS = 600;
/** 点选 3D 热点标记的屏幕容差（像素）。 */
const HOTSPOT_CLICK_RADIUS_PX = 18;

/** 舞台外壳：持有 Canvas 的 generation（「立即重建」= 换 generation 重新挂载）。 */
export function ViewerStage(props: ViewerStageProps) {
  const [generation, setGeneration] = useState(0);

  const rebuild = useCallback(() => {
    setGeneration((value) => value + 1);
  }, []);

  return (
    <Canvas
      key={generation}
      dpr={[1, 2]}
      frameloop="always"
      camera={{ fov: DEFAULT_FOV, near: 0.01, far: 100, position: [0, 0, 3] }}
      gl={{ antialias: true, powerPreference: "high-performance" }}
      onCreated={({ gl }) => {
        const canvas = gl.domElement;
        canvas.setAttribute("role", "img");
        canvas.setAttribute(
          "aria-label",
          "3D 模型视口：在画布上拖动可旋转、滚轮可缩放；等效操作见上方按钮",
        );
        canvas.setAttribute("data-testid", "viewer-canvas");
      }}
    >
      <StageScene {...props} rebuild={rebuild} />
    </Canvas>
  );
}

interface StageSceneProps extends ViewerStageProps {
  readonly rebuild: () => void;
}

function StageScene({
  buffer,
  model,
  hotspots,
  onModelReady,
  onLoadError,
  onContextState,
  onPoseChange,
  pickMode,
  selectedHotspotId,
  onPick,
  onHotspotSelect,
  apiRef,
  rebuild,
  initialDirection,
  restorePoseRef,
}: StageSceneProps) {
  const gl = useThree((state) => state.gl);
  // Canvas 的 `camera` prop 固定了透视相机（fov/near/far），这里显式收窄类型：
  // 位姿（CameraPose）需要 fov，正交相机没有该字段。
  const camera = useThree((state) => state.camera) as PerspectiveCamera;
  const size = useThree((state) => state.size);

  const [loaded, setLoaded] = useState<LoadedModel | null>(null);
  const [modelInfo, setModelInfo] = useState<ViewerModelInfo | null>(null);

  const displayRef = useRef<Group | null>(null);
  const assetRootRef = useRef<AssetRoot | null>(null);
  const controlsRef = useRef<OrbitControls | null>(null);
  const fitRef = useRef<FitResult | null>(null);
  const boundsRef = useRef<Bounds | null>(null);
  const initialPoseRef = useRef<{ position: Vector3; target: Vector3 } | null>(null);
  const contextLostRef = useRef(false);
  const framesRef = useRef(0);
  const directionRef = useRef<Vec3>(initialDirection ?? DEFAULT_DIRECTION);
  const hotspotsRef = useRef<readonly ViewerHotspotView[]>(hotspots);
  hotspotsRef.current = hotspots;
  const pickModeRef = useRef(pickMode === true);
  pickModeRef.current = pickMode === true;
  const lastPickRef = useRef<ViewerPickResult | null>(null);

  const reportPose = useCallback(() => {
    const root = assetRootRef.current;
    const controls = controlsRef.current;
    if (root === null || controls === null) {
      onPoseChange?.(null);
      return;
    }
    onPoseChange?.(
      readCameraPose(
        root.transform(),
        {
          position: [camera.position.x, camera.position.y, camera.position.z],
          up: [camera.up.x, camera.up.y, camera.up.z],
          fov: camera.fov,
        },
        [controls.target.x, controls.target.y, controls.target.z],
      ),
    );
  }, [camera, onPoseChange]);

  // --- 模型加载与释放（换模型/卸载都走同一条释放路径） ---------------------------
  useEffect(() => {
    let cancelled = false;
    let current: LoadedModel | null = null;
    const run = async (): Promise<void> => {
      try {
        const result = await loadGlbModel(buffer, { expectedSha256: model.sha256 });
        if (cancelled) {
          result.dispose();
          return;
        }
        current = result;
        const root = createAssetRoot(result.scene, {
          revisionId: model.revisionId,
          sha256: model.sha256,
        });
        assetRootRef.current = root;
        boundsRef.current = root.localBounds();
        const summary = summarizeScene(result.scene);
        const info: ViewerModelInfo = {
          assetId: model.assetId,
          revisionId: model.revisionId,
          sha256: model.sha256,
          triangles: summary.triangles,
          objects: summary.objects,
          textures: summary.textures,
          bounds: boundsRef.current,
        };
        setLoaded(result);
        setModelInfo(info);
        onModelReady?.(info);
      } catch (error) {
        if (cancelled) {
          return;
        }
        const viewerError =
          error instanceof ViewerError
            ? error
            : new ViewerError(
                "model_parse_failed",
                error instanceof Error ? error.message : String(error),
              );
        onLoadError?.({ code: viewerError.code, message: viewerError.message });
      }
    };
    void run();
    return () => {
      cancelled = true;
      assetRootRef.current = null;
      boundsRef.current = null;
      fitRef.current = null;
      initialPoseRef.current = null;
      setLoaded(null);
      setModelInfo(null);
      current?.dispose();
      onPoseChange?.(null);
    };
    // 只有"哪一份模型"参与依赖；回调由调用方保持稳定或用 ref 传递。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [buffer, model.assetId, model.revisionId, model.sha256]);

  // --- OrbitControls（旋转/缩放；丢失期间禁用） ---------------------------------
  useEffect(() => {
    const controls = new OrbitControls(camera, gl.domElement);
    // 减少动效：不使用阻尼/惯性（相机动作即时完成，无自动动画）。
    controls.enableDamping = false;
    controls.rotateSpeed = 0.85;
    controls.zoomSpeed = 0.9;
    controlsRef.current = controls;
    const onChange = (): void => reportPose();
    controls.addEventListener("change", onChange);
    reportPose();
    return () => {
      controls.removeEventListener("change", onChange);
      controls.dispose();
      controlsRef.current = null;
    };
  }, [camera, gl, reportPose]);

  // --- 适配（fit）与复位（reset） -------------------------------------------------
  const applyFit = useCallback(
    (options: { keepDirection: boolean }) => {
      const controls = controlsRef.current;
      const display = displayRef.current;
      const bounds = boundsRef.current;
      if (controls === null || display === null || bounds === null) {
        return;
      }
      const aspect = size.height > 0 ? size.width / size.height : 1;
      const direction = options.keepDirection
        ? directionFrom(controls.target, camera.position)
        : directionRef.current;
      const fit = computeFit(bounds, {
        fovDeg: camera.fov,
        aspect,
        margin: FIT_MARGIN,
        direction,
      });
      fitRef.current = fit;
      display.position.set(fit.displayOffset[0], fit.displayOffset[1], fit.displayOffset[2]);
      display.scale.setScalar(fit.displayScale);
      camera.up.set(0, 1, 0);
      camera.position.set(
        fit.cameraPosition[0],
        fit.cameraPosition[1],
        fit.cameraPosition[2],
      );
      controls.target.set(fit.target[0], fit.target[1], fit.target[2]);
      controls.update();
      reportPose();
    },
    [camera, reportPose, size.height, size.width],
  );

  /**
   * 套用一份 asset-root 局部位姿（读取用同一对变换的反向：`applyCameraPose`）。
   * 「立即重建」路径用它保留重建前的视角（BUG-007；与自动 restored 分支的
   * "相机对象不变 ⇒ 位姿保留"等价）。
   */
  const applyPose = useCallback(
    (pose: CameraPose) => {
      const root = assetRootRef.current;
      const controls = controlsRef.current;
      if (root === null || controls === null) {
        return;
      }
      const applied = applyCameraPose(root.transform(), pose);
      // `up` 是方向量：显示变换是等比例缩放，长度会变，归一化后再写回相机。
      const upLength = Math.hypot(applied.up[0], applied.up[1], applied.up[2]);
      if (upLength > 0) {
        camera.up.set(applied.up[0] / upLength, applied.up[1] / upLength, applied.up[2] / upLength);
      }
      camera.position.set(applied.position[0], applied.position[1], applied.position[2]);
      controls.target.set(applied.target[0], applied.target[1], applied.target[2]);
      controls.update();
      reportPose();
    },
    [camera, reportPose],
  );

  // 首次适配：模型就绪之后（display group 已挂载）。
  useEffect(() => {
    if (loaded === null) {
      return;
    }
    applyFit({ keepDirection: false });
    const controls = controlsRef.current;
    // reset 的语义是"回到初始取景"：先记录默认取景，再（可选的）套用重建前位姿，
    // 因此「复位视角」在任何情况下都回到默认正面取景。
    initialPoseRef.current =
      controls === null
        ? null
        : { position: camera.position.clone(), target: controls.target.clone() };
    // 手动重建：消费一次重建前捕获的位姿（与自动恢复分支一致的"位姿保留"）。
    const pending = restorePoseRef?.current ?? null;
    if (pending !== null && restorePoseRef !== undefined) {
      restorePoseRef.current = null;
      applyPose(pending);
    }
  }, [loaded, applyFit, applyPose, camera, restorePoseRef]);

  // 视口尺寸变化：重新取景（保持方向），避免模型被裁切。
  useEffect(() => {
    if (loaded === null) {
      return;
    }
    applyFit({ keepDirection: true });
  }, [size.width, size.height, loaded, applyFit]);

  // --- 舞台 API（面板工具栏调用） ------------------------------------------------
  useEffect(() => {
    apiRef.current = {
      reset: () => {
        const controls = controlsRef.current;
        const initial = initialPoseRef.current;
        const fit = fitRef.current;
        const display = displayRef.current;
        if (controls === null || initial === null || fit === null || display === null) {
          return;
        }
        display.position.set(fit.displayOffset[0], fit.displayOffset[1], fit.displayOffset[2]);
        display.scale.setScalar(fit.displayScale);
        camera.up.set(0, 1, 0);
        camera.position.copy(initial.position);
        controls.target.copy(initial.target);
        controls.update();
        reportPose();
      },
      fit: () => applyFit({ keepDirection: true }),
      applyPose: (pose: CameraPose) => applyPose(pose),
      focusLocal: (local: Vec3) => {
        const root = assetRootRef.current;
        const controls = controlsRef.current;
        if (root === null || controls === null) {
          return;
        }
        // 把该局部点的世界位置移到视口中心：只平移观察目标与相机（保持方向与距离），
        // 因此"居中"不会改变用户当前的理解角度。
        const world = root.toWorld(local);
        const offsetX = camera.position.x - controls.target.x;
        const offsetY = camera.position.y - controls.target.y;
        const offsetZ = camera.position.z - controls.target.z;
        controls.target.set(world[0], world[1], world[2]);
        camera.position.set(world[0] + offsetX, world[1] + offsetY, world[2] + offsetZ);
        controls.update();
        reportPose();
      },
      rebuild,
      pose: () => {
        const root = assetRootRef.current;
        const controls = controlsRef.current;
        if (root === null || controls === null) {
          return null;
        }
        return readCameraPose(
          root.transform(),
          {
            position: [camera.position.x, camera.position.y, camera.position.z],
            up: [camera.up.x, camera.up.y, camera.up.z],
            fov: camera.fov,
          },
          [controls.target.x, controls.target.y, controls.target.z],
        );
      },
    };
    return () => {
      apiRef.current = null;
    };
  }, [apiRef, applyFit, applyPose, camera, rebuild, reportPose]);

  // --- 拾取与点选（UI-046/UI-047/UI-048） -----------------------------------------
  //
  // 判定点击与拖动（位移/时长阈值）：拖动只改变相机（OrbitControls），绝不建点。
  // raycast 只包含模型 mesh（asset-root 子树）并排除热点自身（热点标记的
  // `raycast` 已置空）。命中点用 asset-root 的 worldToLocal 保存（architecture §5.4）。
  useEffect(() => {
    const canvas = gl.domElement;
    const raycaster = createRaycaster();
    const pointer = new Vector2();
    let pressed: { x: number; y: number; time: number } | null = null;

    const groundPosition = (event: PointerEvent) => {
      const rect = canvas.getBoundingClientRect();
      const screenX = event.clientX - rect.left;
      const screenY = event.clientY - rect.top;
      return { rect, screenX, screenY };
    };

    const toNdc = (rect: DOMRect, screenX: number, screenY: number) => {
      pointer.set(
        (screenX / Math.max(rect.width, 1)) * 2 - 1,
        -(screenY / Math.max(rect.height, 1)) * 2 + 1,
      );
      return pointer;
    };

    const projectLocal = (local: Vec3): { x: number; y: number } | null => {
      const root = assetRootRef.current;
      if (root === null) {
        return null;
      }
      const world = root.toWorld(local);
      const projected = new Vector3(world[0], world[1], world[2]).project(camera);
      const rect = canvas.getBoundingClientRect();
      return {
        x: ((projected.x + 1) / 2) * rect.width,
        y: ((1 - projected.y) / 2) * rect.height,
      };
    };

    const pickHotspotAt = (screenX: number, screenY: number): string | null => {
      let best: { id: string; distance: number } | null = null;
      for (const hotspot of hotspotsRef.current) {
        const projected = projectLocal(hotspot.positionLocal);
        if (projected === null) {
          continue;
        }
        const distance = Math.hypot(projected.x - screenX, projected.y - screenY);
        if (distance <= HOTSPOT_CLICK_RADIUS_PX && (best === null || distance < best.distance)) {
          best = { id: hotspot.id, distance };
        }
      }
      return best?.id ?? null;
    };

    const surfaceHit = (event: PointerEvent): ViewerPickResult | null => {
      const root = assetRootRef.current;
      if (root === null) {
        return null;
      }
      const { rect, screenX, screenY } = groundPosition(event);
      raycaster.setFromCamera(toNdc(rect, screenX, screenY), camera);
      const intersections = raycaster.intersectObject(root.object, true);
      for (const intersection of intersections) {
        const point = intersection.point;
        const world: Vec3 = [point.x, point.y, point.z];
        const local = root.toLocal(world);
        return { local, world, screen: { x: screenX, y: screenY } };
      }
      return null;
    };

    const handleDown = (event: PointerEvent): void => {
      if (event.button !== 0) {
        return;
      }
      pressed = { x: event.clientX, y: event.clientY, time: performance.now() };
    };
    const handleUp = (event: PointerEvent): void => {
      const start = pressed;
      pressed = null;
      if (start === null || event.button !== 0) {
        return;
      }
      const moved = Math.hypot(event.clientX - start.x, event.clientY - start.y);
      const duration = performance.now() - start.time;
      if (moved > CLICK_MAX_MOVE_PX || duration > CLICK_MAX_DURATION_MS) {
        // 拖动或长按：只旋转相机，不建点、不选中（UI-048）。
        return;
      }
      const { rect, screenX, screenY } = groundPosition(event);
      const hotspotId = pickHotspotAt(screenX, screenY);
      if (hotspotId !== null) {
        onHotspotSelect?.(hotspotId);
        return;
      }
      if (!pickModeRef.current) {
        return;
      }
      const hit = surfaceHit(event);
      if (hit === null) {
        return;
      }
      lastPickRef.current = hit;
      onPick?.(hit);
      void rect;
    };

    canvas.addEventListener("pointerdown", handleDown);
    canvas.addEventListener("pointerup", handleUp);
    return () => {
      canvas.removeEventListener("pointerdown", handleDown);
      canvas.removeEventListener("pointerup", handleUp);
    };
  }, [camera, gl, onHotspotSelect, onPick]);

  // --- 上下文丢失/恢复 -----------------------------------------------------------
  useEffect(() => {
    const canvas = gl.domElement;
    const handleLost = (event: Event): void => {
      // 必须 preventDefault：否则浏览器不会再派发 restored（UI-044 的「不能只
      // 提供刷新按钮」正是要在这条路径上重建）。
      event.preventDefault();
      contextLostRef.current = true;
      if (controlsRef.current !== null) {
        controlsRef.current.enabled = false;
      }
      onContextState?.("lost");
    };
    const handleRestored = (): void => {
      contextLostRef.current = false;
      if (controlsRef.current !== null) {
        controlsRef.current.enabled = true;
      }
      onContextState?.("ok");
    };
    canvas.addEventListener("webglcontextlost", handleLost);
    canvas.addEventListener("webglcontextrestored", handleRestored);
    // 本次挂载的上下文已经创建成功（R3F 在拿到 GL 后才挂载场景），向面板上报一次
    // `ok`：首次挂载是无操作；「立即重建」换新 Canvas 时，这一步把面板从
    // 「正在重建…」收敛回可用态（BUG-007 的根因是此处缺上报）。
    contextLostRef.current = false;
    onContextState?.("ok");
    return () => {
      canvas.removeEventListener("webglcontextlost", handleLost);
      canvas.removeEventListener("webglcontextrestored", handleRestored);
    };
  }, [gl, onContextState]);

  // --- 只读可观测桥 --------------------------------------------------------------
  useEffect(() => {
    installViewerBridge({
      frames: () => framesRef.current,
      contextState: () => (contextLostRef.current ? "lost" : "ok"),
      model: () => modelInfo,
      cameraPose: () => apiRef.current?.pose() ?? null,
      viewport: () => ({
        width: size.width,
        height: size.height,
        aspect: size.height > 0 ? size.width / size.height : 1,
        pixelRatio: gl.getPixelRatio(),
      }),
      anchors: (): ViewerAnchorProjection[] => {
        const root = assetRootRef.current;
        if (root === null) {
          return [];
        }
        return hotspotsRef.current.map((hotspot) => ({
          id: hotspot.id,
          partId: hotspot.partId,
          local: hotspot.positionLocal,
          world: root.toWorld(hotspot.positionLocal),
        }));
      },
      roundTrip: (local) => {
        const root = assetRootRef.current;
        if (root === null) {
          return null;
        }
        const world = root.toWorld(local);
        const back = root.toLocal(world);
        const error = Math.hypot(back[0] - local[0], back[1] - local[1], back[2] - local[2]);
        return { local, world, back, error };
      },
      localBounds: () => boundsRef.current,
      project: (local: Vec3) => {
        const root = assetRootRef.current;
        if (root === null) {
          return null;
        }
        const world = root.toWorld(local);
        const projected = new Vector3(world[0], world[1], world[2]).project(camera);
        const rect = gl.domElement.getBoundingClientRect();
        return {
          ndc: [projected.x, projected.y, projected.z] as [number, number, number],
          screen: [
            ((projected.x + 1) / 2) * rect.width,
            ((1 - projected.y) / 2) * rect.height,
          ] as [number, number],
          visible: projected.z <= 1,
        };
      },
      lastPick: () => lastPickRef.current,
      picking: () => pickModeRef.current,
      stats: () => viewerResourceStats(),
    });
    return () => {
      clearViewerBridgeHandlers();
    };
  }, [apiRef, camera, gl, modelInfo, size.height, size.width]);

  return (
    <>
      <FrameTicker
        contextLostRef={contextLostRef}
        onFrame={() => {
          framesRef.current += 1;
        }}
      />
      <ambientLight intensity={1.4} />
      <directionalLight position={[3, 5, 4]} intensity={1.6} />
      <directionalLight position={[-4, -2, -3]} intensity={0.6} />
      <group ref={displayRef} name="em-display-group">
        {loaded !== null && (
          <primitive object={loaded.scene}>
            <HotspotMarkers hotspots={hotspots} selectedHotspotId={selectedHotspotId ?? null} />
          </primitive>
        )}
      </group>
    </>
  );
}

/** 帧计数：上下文丢失期间不计数（用于断言"恢复后继续渲染"）。 */
function FrameTicker({
  contextLostRef,
  onFrame,
}: {
  contextLostRef: { current: boolean };
  onFrame: () => void;
}) {
  useFrame(() => {
    if (!contextLostRef.current) {
      onFrame();
    }
  });
  return null;
}

/**
 * 热点标记（T19：渲染**可用热点**；stale/unbound 由调用方过滤，不在此图）。
 *
 * 作为 asset-root 的子对象渲染：位置就是 anchor 的 asset-root 局部坐标，与保存
 * 时同一坐标空间，因此外层显示变换（fit 的平移/缩放）与相机变化都不会让标记
 * 偏离模型表面。`raycast` 置空：标记不参与射线拾取——点选标记由**屏幕投影距离**
 * 判定（`pickHotspotAt`），raycast 只命中模型 mesh（architecture §5.4）。
 * 选中的热点放大并换色（与左栏部件列表的选中状态一致；UI-046）。
 */
function HotspotMarkers({
  hotspots,
  selectedHotspotId,
}: {
  hotspots: readonly ViewerHotspotView[];
  selectedHotspotId: string | null;
}) {
  return (
    <>
      {hotspots.map((hotspot) => {
        const selected = hotspot.id === selectedHotspotId;
        return (
          <mesh
            key={hotspot.id}
            position={[
              hotspot.positionLocal[0],
              hotspot.positionLocal[1],
              hotspot.positionLocal[2],
            ]}
            userData={{ emHotspot: true, hotspotId: hotspot.id, partId: hotspot.partId }}
            raycast={() => null}
          >
            <sphereGeometry args={[selected ? HOTSPOT_RADIUS * 1.5 : HOTSPOT_RADIUS, 16, 12]} />
            <meshBasicMaterial color={selected ? HOTSPOT_SELECTED_COLOR : HOTSPOT_COLOR} />
          </mesh>
        );
      })}
    </>
  );
}

function createRaycaster(): Raycaster {
  return new Raycaster();
}

/** 由目标点与相机位置求单位观察方向；退化时回落到正面。 */
function directionFrom(target: Vector3, position: Vector3): Vec3 {
  const x = position.x - target.x;
  const y = position.y - target.y;
  const z = position.z - target.z;
  const length = Math.hypot(x, y, z);
  if (length < 1e-6) {
    return [0, 0, 1];
  }
  return [x / length, y / length, z / length];
}
