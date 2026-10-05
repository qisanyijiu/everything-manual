/**
 * 交互层运行时（ADR-042）：分件节点的高亮、动作与姿势动画。
 *
 * 约定（与服务端 `drafts::interactive` 同源）：
 * - 变换步相对节点**初始**位姿；一个动作/姿势的全部步骤按顺序叠加到初始位姿上；
 * - `translate.vector`、`rotate.pivot/axis` 都在 asset-root 局部坐标中（与热点同一空间）；
 * - 姿势之间切换时从初始位姿重新计算（不累积误差）；动作在当前姿势之上叠加；
 * - 只做展示用刚体变换，不声称机械结构真实（界面需如实说明）。
 *
 * 本模块只依赖 three（不依赖 React），在线查看器与离线 HTML 共用。
 */

import { Color, Matrix4, Mesh, Quaternion, Vector3, type Material, type Object3D } from "three";

import type { ModelActionView, ModelPoseView, TransformStepView } from "./interactive-view";
import { inspectPartNodes } from "./part-inspection";
import type { ViewerPartProjection } from "./bridge";

/** 一组变换步在进度 `t ∈ [0,1]` 时对每个节点产生的局部矩阵（相对初始位姿，左乘）。 */
export function stepMatrices(steps: readonly TransformStepView[], t: number): Map<string, Matrix4> {
  const out = new Map<string, Matrix4>();
  for (const step of steps) {
    const m = new Matrix4();
    if (step.kind === "translate" && step.vector) {
      m.makeTranslation(step.vector[0] * t, step.vector[1] * t, step.vector[2] * t);
    } else if (step.kind === "rotate" && step.axis && step.pivot && typeof step.angleDeg === "number") {
      const axis = new Vector3(...step.axis).normalize();
      const pivot = new Vector3(...step.pivot);
      const q = new Quaternion().setFromAxisAngle(axis, ((step.angleDeg * Math.PI) / 180) * t);
      m.makeTranslation(pivot.x, pivot.y, pivot.z)
        .multiply(new Matrix4().makeRotationFromQuaternion(q))
        .multiply(new Matrix4().makeTranslation(-pivot.x, -pivot.y, -pivot.z));
    } else {
      continue;
    }
    for (const node of step.nodes) {
      const prev = out.get(node) ?? new Matrix4();
      out.set(node, m.clone().multiply(prev));
    }
  }
  return out;
}

const ease = (t: number): number => (t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2);

/**
 * 节点动画控制器：记录每个具名节点的初始矩阵；每帧把"姿势 × 激活动作"合成到初始位姿上。
 * 只修改分件节点自身的变换（asset-root 不动，热点坐标系不变）。
 */
export class PartAnimator {
  private readonly nodes = new Map<string, Object3D>();
  private readonly base = new Map<string, Matrix4>();
  private pose: { steps: readonly TransformStepView[]; from: number; to: number; start: number; duration: number; prevSteps: readonly TransformStepView[] } | null = null;
  private readonly actions = new Map<string, { action: ModelActionView; from: number; to: number; start: number }>();
  private readonly highlighted = new Map<Mesh, Material | Material[]>();
  private readonly reducedMotion: boolean;
  private active = false;
  /** 跟随节点运动的标记（热点）：记录初始位置与所跟随的节点。 */
  private readonly followers = new Map<Object3D, { node: string; base: Vector3 }>();
  private readonly root: Object3D;
  private readonly partOffsets = new Map<string, Vector3>();
  private readonly expansions = new Map<string, { from: number; to: number; start: number }>();

  constructor(root: Object3D, reducedMotion = false) {
    this.root = root;
    this.reducedMotion = reducedMotion;
    root.traverse((object) => {
      if (object.name !== "" && !this.nodes.has(object.name)) {
        this.nodes.set(object.name, object);
        object.updateMatrix();
        this.base.set(object.name, object.matrix.clone());
      }
    });
  }

  /** 登记可独立观察的几何节点，不要求它已绑定说明书部件。 */
  configureParts(names: readonly string[]): void {
    this.partOffsets.clear();
    for (const node of inspectPartNodes(this.root, names)) this.partOffsets.set(node.name, node.offset);
  }

  get expandedNodeNames(): readonly string[] {
    return [...this.expansions].filter(([, state]) => state.to === 1).map(([name]) => name);
  }

  /** 只读实际变换投影，由开发构建的可观测桥调用。 */
  inspectParts(): readonly ViewerPartProjection[] {
    this.root.updateWorldMatrix(true, false);
    const rootInverse = this.root.matrixWorld.clone().invert();
    return [...this.partOffsets.keys()].flatMap((name) => {
      const object = this.nodes.get(name);
      if (!object) return [];
      object.updateWorldMatrix(true, false);
      return [{ name, matrixRoot: rootInverse.clone().multiply(object.matrixWorld).toArray(), expanded: this.expansions.get(name)?.to === 1 }];
    });
  }

  /** 外观观察层与语义动作独立；空列表只复原观察展开。 */
  setExpandedNodes(names: readonly string[], now: number): void {
    const wanted = new Set(names);
    for (const name of this.partOffsets.keys()) {
      const current = this.expansions.get(name);
      const to = wanted.has(name) ? 1 : 0;
      if ((current?.to ?? 0) !== to) {
        this.expansions.set(name, { from: current ? this.expansionProgress(current, now) : 0, to, start: now });
      }
    }
    this.active = true;
  }

  private expansionProgress(state: { from: number; to: number; start: number }, now: number): number {
    const progress = this.reducedMotion ? 1 : Math.max(0, Math.min(1, (now - state.start) / 400));
    return state.from + (state.to - state.from) * ease(progress);
  }

  /**
   * 让热点标记跟随部件运动：`partNode` 给出部件 → 节点。标记对象需带 `userData.partId`。
   * 重复调用会替换跟随关系（热点集合变化时由调用方重新登记）。
   */
  attachMarkers(root: Object3D, partNode: ReadonlyMap<string, string>): void {
    for (const [object, follower] of this.followers) {
      object.position.copy(follower.base);
    }
    this.followers.clear();
    root.traverse((object) => {
      const partId = object.userData?.partId;
      const node = typeof partId === "string" ? partNode.get(partId) : undefined;
      if (object.userData?.emHotspot === true && node !== undefined) {
        this.followers.set(object, { node, base: object.position.clone() });
      }
    });
    this.active = true;
  }

  hasNode(name: string): boolean {
    return this.nodes.has(name);
  }

  /** 是否仍有动画在播放（调用方据此决定是否继续请求帧）。 */
  get animating(): boolean {
    return this.active;
  }

  setPose(pose: ModelPoseView | null, now: number): void {
    const prevSteps = this.pose?.steps ?? [];
    this.pose = {
      steps: pose?.steps ?? [],
      prevSteps,
      from: 0,
      to: 1,
      start: now,
      duration: this.reducedMotion ? 0 : (pose?.durationMs ?? 400),
    };
    this.active = true;
  }

  /** 触发动作：toggle 在 0/1 之间切换；pulse 做 0→1→0。返回切换后的状态（true = 处于目标状态）。 */
  trigger(action: ModelActionView, now: number): boolean {
    const current = this.actions.get(action.id);
    const at = current ? this.progressOf(current, now) : 0;
    const to = action.mode === "pulse" ? 1 : at > 0.5 ? 0 : 1;
    this.actions.set(action.id, { action, from: at, to, start: now });
    this.active = true;
    return to === 1 && action.mode === "toggle";
  }

  resetActions(now: number): void {
    for (const [id, state] of this.actions) {
      this.actions.set(id, { ...state, from: this.progressOf(state, now), to: 0, start: now });
    }
    this.active = true;
  }

  private progressOf(state: { action: ModelActionView; from: number; to: number; start: number }, now: number): number {
    const duration = this.reducedMotion ? 0 : Math.max(1, state.action.durationMs);
    const raw = duration === 0 ? 1 : Math.min(1, (now - state.start) / duration);
    if (state.action.mode === "pulse" && state.to === 1) {
      const tri = raw < 0.5 ? raw * 2 : (1 - raw) * 2;
      return ease(Math.max(0, tri));
    }
    return state.from + (state.to - state.from) * ease(raw);
  }

  /** 每帧调用：合成并写回节点矩阵。 */
  update(now: number): void {
    if (!this.active) {
      return;
    }
    let running = false;
    const composed = new Map<string, Matrix4>();
    const merge = (mats: Map<string, Matrix4>): void => {
      for (const [name, m] of mats) {
        composed.set(name, m.clone().multiply(composed.get(name) ?? new Matrix4()));
      }
    };
    if (this.pose) {
      const raw = this.pose.duration === 0 ? 1 : Math.min(1, (now - this.pose.start) / this.pose.duration);
      if (raw < 1) {
        running = true;
      }
      const t = ease(raw);
      // 旧姿势淡出、新姿势淡入（两者都相对初始位姿，线性混合足够用于展示）。
      merge(stepMatrices(this.pose.prevSteps, 1 - t));
      merge(stepMatrices(this.pose.steps, t));
    }
    for (const [id, state] of this.actions) {
      const duration = this.reducedMotion ? 0 : Math.max(1, state.action.durationMs);
      if (duration > 0 && now - state.start < duration) {
        running = true;
      }
      const t = this.progressOf(state, now);
      if (t <= 0 && state.to === 0 && !running) {
        this.actions.delete(id);
        continue;
      }
      merge(stepMatrices(state.action.steps, t));
    }
    const offsets = new Map([...this.partOffsets.keys()].map((name) => [name, new Vector3()]));
    for (const [name, state] of this.expansions) {
      if (!this.reducedMotion && now - state.start < 400) running = true;
      const offset = this.partOffsets.get(name);
      if (offset) offsets.set(name, offset.clone().multiplyScalar(this.expansionProgress(state, now)));
    }
    for (const [name, object] of this.nodes) {
      const base = this.base.get(name);
      if (!base) {
        continue;
      }
      const delta = composed.get(name);
      const next = delta ? delta.clone().multiply(base) : base.clone();
      next.decompose(object.position, object.quaternion, object.scale);
      const offset = offsets.get(name);
      if (offset && object.parent) {
        // 具名节点也可能嵌套。减掉祖先位移，避免子分件被整体展开两次。
        const independent = offset.clone();
        for (let parent: Object3D | null = object.parent; parent && parent !== this.root; parent = parent.parent) {
          const parentOffset = offsets.get(parent.name);
          if (parentOffset) { independent.sub(parentOffset); break; }
        }
        object.parent.updateWorldMatrix(true, false);
        this.root.updateWorldMatrix(true, false);
        const rootToParent = object.parent.matrixWorld.clone().invert().multiply(this.root.matrixWorld);
        object.position.add(independent.applyMatrix4(rootToParent).sub(new Vector3().applyMatrix4(rootToParent)));
      }
    }
    for (const [object, follower] of this.followers) {
      const delta = composed.get(follower.node);
      object.position.copy(follower.base);
      if (delta) {
        object.position.applyMatrix4(delta);
      }
      const offset = offsets.get(follower.node);
      if (offset) object.position.add(offset);
    }
    this.active = running;
  }

  /**
   * 高亮一组节点（替换为发光材质副本；再次调用会先恢复）。
   * 默认取色 = `--color-accent`（选中语义与应用列表/热点标记一致，VS-03）。
   */
  highlight(names: readonly string[], color = "#A63F21"): void {
    for (const [mesh, material] of this.highlighted) {
      const tinted = mesh.material;
      mesh.material = material;
      (Array.isArray(tinted) ? tinted : [tinted]).forEach((copy) => copy.dispose());
    }
    this.highlighted.clear();
    for (const name of names) {
      this.nodes.get(name)?.traverse((object) => {
        const mesh = object as Mesh;
        if (!mesh.isMesh) {
          return;
        }
        if (this.highlighted.has(mesh)) return;
        const original = mesh.material;
        const tint = (material: Material): Material => {
          const copy = material.clone() as Material & { emissive?: Color; emissiveIntensity?: number };
          if (copy.emissive) {
            copy.emissive = new Color(color);
            copy.emissiveIntensity = 0.55;
          }
          return copy;
        };
        mesh.material = Array.isArray(original) ? original.map(tint) : tint(original);
        this.highlighted.set(mesh, original);
      });
    }
  }

  dispose(): void {
    for (const [mesh, original] of this.highlighted) {
      const tinted = mesh.material;
      mesh.material = original;
      (Array.isArray(tinted) ? tinted : [tinted]).forEach((material) => material.dispose());
    }
    this.highlighted.clear();
  }
}
