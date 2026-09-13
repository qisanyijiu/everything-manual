/**
 * asset-root 与 Three.js 对象图的桥接（T18；architecture §5.4）。
 *
 * 设计要点（QA 与 T19 按此复核）：
 * - **局部坐标 = GLB 根对象的局部坐标**；保存锚点时调用 root 的 `worldToLocal`，
 *   读取热点时调用同一个 root 的 `localToWorld`——读写同源，不使用"近似"变换。
 * - **显示居中/缩放在外层 group**：本模块从不修改传入对象自身的变换；调用方
 *   （`ViewerCanvas`）把 root 挂在 display group 下，由外层承担 fit 的平移与缩放。
 *   因此切换显示变换不会改变"同一局部点"的含义。
 * - **身份来自不可变版本**：`revisionId + sha256`（绝不使用 `mesh.uuid` / 节点名）。
 * - **法线**：用 `transformNormal`（逆转置），非均匀缩放下与切线保持垂直。
 */

import { Box3, Quaternion, Vector3, type Object3D } from "three";

import {
  isFiniteVec3,
  transformNormal,
  type Bounds,
  type Transform,
  type Vec3,
} from "./coordinates";

/** 模型版本的稳定身份（不可变；不用运行时对象引用）。 */
export interface ModelIdentity {
  readonly revisionId: string;
  readonly sha256: string;
}

export interface AssetRoot {
  /** 根对象（GLB 场景根）。调用方只读取，不修改其变换。 */
  readonly object: Object3D;
  readonly identity: ModelIdentity;
  /** 当前 asset-root 的世界变换快照（每次调用重新读取 `matrixWorld`）。 */
  transform(): Transform;
  /** 世界点 → asset-root 局部点（保存锚点用这一方向）。 */
  toLocal(worldPoint: Vec3): Vec3;
  /** 局部点 → 世界点（读取热点/位姿用这一方向）。 */
  toWorld(localPoint: Vec3): Vec3;
  /** 局部法线 → 世界法线（逆转置；非均匀缩放正确）。 */
  normalToWorld(localNormal: Vec3): Vec3;
  /** 模型在 asset-root 局部坐标下的轴对齐包围盒。 */
  localBounds(): Bounds;
}

/** 由 Object3D 的 `matrixWorld` 反解出 `Transform`（与 three 的 T·R·S 约定一致）。 */
export function readTransform(object: Object3D): Transform {
  const position = new Vector3();
  const quaternion = new Quaternion();
  const scale = new Vector3();
  object.matrixWorld.decompose(position, quaternion, scale);
  return {
    position: [position.x, position.y, position.z],
    quaternion: [quaternion.x, quaternion.y, quaternion.z, quaternion.w],
    scale: [scale.x, scale.y, scale.z],
  };
}

/**
 * 创建 asset-root 适配器。
 *
 * 前提：`object` 已加入场景图（`matrixWorld` 由渲染循环维护）；每次读写都先
 * `updateWorldMatrix(true, false)` 刷新自身与祖先链，避免用到上一帧的矩阵。
 */
export function createAssetRoot(object: Object3D, identity: ModelIdentity): AssetRoot {
  const scratch = new Vector3();
  const corner = new Vector3();
  const box = new Box3();

  const refresh = (): void => {
    object.updateWorldMatrix(true, false);
  };

  return {
    object,
    identity,
    transform(): Transform {
      refresh();
      return readTransform(object);
    },
    toLocal(worldPoint: Vec3): Vec3 {
      if (!isFiniteVec3(worldPoint)) {
        throw new Error("世界坐标不是有限数值：拒绝转换");
      }
      refresh();
      scratch.set(worldPoint[0], worldPoint[1], worldPoint[2]);
      const local = object.worldToLocal(scratch);
      return [local.x, local.y, local.z];
    },
    toWorld(localPoint: Vec3): Vec3 {
      if (!isFiniteVec3(localPoint)) {
        throw new Error("局部坐标不是有限数值：拒绝转换");
      }
      refresh();
      scratch.set(localPoint[0], localPoint[1], localPoint[2]);
      const world = object.localToWorld(scratch);
      return [world.x, world.y, world.z];
    },
    normalToWorld(localNormal: Vec3): Vec3 {
      refresh();
      return transformNormal(readTransform(object), localNormal);
    },
    localBounds(): Bounds {
      refresh();
      box.setFromObject(object);
      if (box.isEmpty()) {
        return { min: [0, 0, 0], max: [0, 0, 0] };
      }
      const min: [number, number, number] = [Infinity, Infinity, Infinity];
      const max: [number, number, number] = [-Infinity, -Infinity, -Infinity];
      for (const x of [box.min.x, box.max.x]) {
        for (const y of [box.min.y, box.max.y]) {
          for (const z of [box.min.z, box.max.z]) {
            corner.set(x, y, z);
            const local = object.worldToLocal(corner);
            min[0] = Math.min(min[0], local.x);
            min[1] = Math.min(min[1], local.y);
            min[2] = Math.min(min[2], local.z);
            max[0] = Math.max(max[0], local.x);
            max[1] = Math.max(max[1], local.y);
            max[2] = Math.max(max[2], local.z);
          }
        }
      }
      return { min, max };
    },
  };
}
