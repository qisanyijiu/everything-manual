/**
 * 坐标适配层测试（T18 / PRD AC-051；architecture §5.4；contracts §2）。
 *
 * 覆盖（QA 按此复核 AC-051 的 `coordinates.test.ts` 部分）：
 * 1. **不对称模型 + asset-root 有旋转/非均匀缩放**时，同一局部点在不同显示
 *    旋转/缩放下的世界位置换算一致（保存的局部坐标不随显示变换改变）；
 * 2. **非均匀缩放下的法线**必须用逆转置：变换后法线仍与切线垂直，而"当普通
 *    方向量变换"的朴素做法不垂直（固化差异，防止实现退化成朴素做法）；
 * 3. 局部/世界往返一致（`localToWorld` ↔ `worldToLocal`，含 three 对象图）；
 * 4. 有限性校验与锚点状态（T19 复用：NaN/Infinity 拒绝、stale 判定）；
 * 5. `computeFit` 的居中/缩放语义（显示变换只在外层，不改 asset-root 自身）。
 *
 * 真实 three 对象图参与测试（`createAssetRoot` 用的是 `Object3D.worldToLocal`），
 * 因此这里也验证"读写同源"这条硬约束，而不只是我们自己实现的数学。
 */

import { describe, expect, it } from "vitest";
import { BufferGeometry, Euler, Group, Mesh, MeshStandardMaterial, Quaternion, Vector3 } from "three";

import { createAssetRoot } from "./asset-root";
import {
  applyCameraPose,
  checkAnchor,
  composeTransforms,
  computeFit,
  isFiniteVec3,
  localToWorld,
  makeAnchor,
  quatFromEulerXYZ,
  readCameraPose,
  transformDirection,
  transformNormal,
  worldDirectionToLocal,
  worldToLocal,
  type Transform,
  type Vec3,
} from "./coordinates";

/** 不对称模型的 asset-root 局部采样点（与 fixture 生成器同一组坐标，见 §AC-051）。 */
const LOCAL_POINTS: readonly Vec3[] = [
  [-1.0, -0.5, -0.3],
  [1.05, 0.5, -0.3],
  [1.1, 0.65, 0.4],
];

/** asset-root 自身的变换（旋转 + 非均匀缩放 + 平移）——AC-051 的"不对称"来源。 */
const ASSET_ROOT_TRANSFORM: Transform = {
  position: [0.3, -0.2, 0.15],
  quaternion: quatFromEulerXYZ([0.35, -0.8, 0.25]),
  scale: [0.8, 1.7, 0.45],
};

/** 外层显示变换（fit 的居中/缩放所在层；本项目约定外层为等比例缩放）。 */
const DISPLAY_TRANSFORMS: readonly Transform[] = [
  { position: [0, 0, 0], quaternion: [0, 0, 0, 1], scale: [1, 1, 1] },
  {
    position: [1.5, -2, 0.75],
    quaternion: quatFromEulerXYZ([0, Math.PI / 6, 0]),
    scale: [2.5, 2.5, 2.5],
  },
  {
    position: [-3, 1, 2],
    quaternion: quatFromEulerXYZ([0.4, 1.1, -0.3]),
    scale: [0.35, 0.35, 0.35],
  },
];

function expectVec3Close(actual: Vec3, expected: Vec3, digits = 10): void {
  expect(actual[0]).toBeCloseTo(expected[0], digits);
  expect(actual[1]).toBeCloseTo(expected[1], digits);
  expect(actual[2]).toBeCloseTo(expected[2], digits);
}

/** 构造 three 对象图：display → assetRoot → child(旋转+非均匀缩放) → mesh。 */
function buildSceneGraph(display: Transform): { displayGroup: Group; assetRoot: Group } {
  const displayGroup = new Group();
  displayGroup.position.set(...display.position);
  displayGroup.quaternion.copy(
    new Quaternion(...display.quaternion),
  );
  displayGroup.scale.set(...display.scale);

  const assetRoot = new Group();
  assetRoot.position.set(...ASSET_ROOT_TRANSFORM.position);
  assetRoot.quaternion.copy(new Quaternion(...ASSET_ROOT_TRANSFORM.quaternion));
  assetRoot.scale.set(...ASSET_ROOT_TRANSFORM.scale);

  const mesh = new Mesh(new BufferGeometry(), new MeshStandardMaterial());
  assetRoot.add(mesh);
  displayGroup.add(assetRoot);
  displayGroup.updateMatrixWorld(true);
  return { displayGroup, assetRoot };
}

/** 某个局部点在"完整图"里的世界坐标（display ∘ assetRoot）。 */
function worldPointOf(display: Transform, localPoint: Vec3): Vec3 {
  return localToWorld(composeTransforms(display, ASSET_ROOT_TRANSFORM), localPoint);
}

describe("坐标适配层：同一局部点在不同显示变换下一致（AC-051）", () => {
  it("不对称模型的同一局部点：各种显示旋转/缩放下的世界位置换算一致", () => {
    const worlds: Vec3[] = [];
    for (const display of DISPLAY_TRANSFORMS) {
      const { assetRoot } = buildSceneGraph(display);
      const adapter = createAssetRoot(assetRoot, { revisionId: "rev-1", sha256: "sha-1" });
      for (const localPoint of LOCAL_POINTS) {
        const world = adapter.toWorld(localPoint);
        worlds.push(world);
        // 1) 同一局部点在该显示变换下的世界位置，反解必须回到同一局部点。
        expectVec3Close(adapter.toLocal(world), localPoint, 9);
        // 2) 与纯数学实现（同一约定）一致。
        expectVec3Close(world, worldPointOf(display, localPoint), 9);
      }
    }
    // 3) 反向保护：显示变换确实改变了世界位置（否则上面的"一致"没有意义）。
    const first = buildSceneGraph(DISPLAY_TRANSFORMS[0] as Transform);
    const third = buildSceneGraph(DISPLAY_TRANSFORMS[2] as Transform);
    const a = createAssetRoot(first.assetRoot, { revisionId: "rev-1", sha256: "sha-1" }).toWorld(
      LOCAL_POINTS[0] as Vec3,
    );
    const b = createAssetRoot(third.assetRoot, { revisionId: "rev-1", sha256: "sha-1" }).toWorld(
      LOCAL_POINTS[0] as Vec3,
    );
    expect(Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])).toBeGreaterThan(0.1);
  });

  it("保存下来的锚点与显示变换无关：不同显示下从世界点反解得到同一局部坐标", () => {
    const stored: Vec3[] = [];
    for (const display of DISPLAY_TRANSFORMS) {
      const { assetRoot } = buildSceneGraph(display);
      const adapter = createAssetRoot(assetRoot, { revisionId: "rev-1", sha256: "sha-1" });
      // 在同一台设备上"点选"模型表面的同一处：世界点由完整图（display∘root）给出。
      const world = worldPointOf(display, LOCAL_POINTS[1] as Vec3);
      stored.push(adapter.toLocal(world));
    }
    for (const point of stored) {
      expectVec3Close(point, LOCAL_POINTS[1] as Vec3, 9);
    }
    // 三个显示变换下保存出来的锚点一致（浮点误差量级内逐分量相等）：
    // 这才是"可持久化的锚点"——存下来的局部坐标不随显示变换漂移。
    expectVec3Close(stored[0] as Vec3, stored[1] as Vec3, 9);
    expectVec3Close(stored[1] as Vec3, stored[2] as Vec3, 9);
  });

  it("asset-root 非均匀缩放下的往返一致（localToWorld ↔ worldToLocal）", () => {
    for (const localPoint of LOCAL_POINTS) {
      const world = localToWorld(ASSET_ROOT_TRANSFORM, localPoint);
      expectVec3Close(worldToLocal(ASSET_ROOT_TRANSFORM, world), localPoint, 9);
    }
  });
});

describe("非均匀缩放下的法线处理（AC-051）", () => {
  const transform: Transform = {
    position: [0.4, -0.3, 0.2],
    quaternion: quatFromEulerXYZ([0.3, -0.7, 0.2]),
    // 明显非均匀，且法线/切线都不是缩放轴对齐的（否则"朴素做法"碰巧也垂直）。
    scale: [0.5, 3, 1],
  };
  const SQRT_HALF = Math.SQRT1_2;
  const normalLocal: Vec3 = [SQRT_HALF, SQRT_HALF, 0];
  const tangentLocal: Vec3 = [SQRT_HALF, -SQRT_HALF, 0];

  it("同一个平面上的法线与切线在局部空间垂直（测试前提）", () => {
    const dot =
      normalLocal[0] * tangentLocal[0] +
      normalLocal[1] * tangentLocal[1] +
      normalLocal[2] * tangentLocal[2];
    expect(dot).toBeCloseTo(0, 12);
  });

  it("法线用逆转置：变换后仍与切线垂直", () => {
    const normalWorld = transformNormal(transform, normalLocal);
    const tangentWorld = transformDirection(transform, tangentLocal);
    const dot =
      normalWorld[0] * tangentWorld[0] +
      normalWorld[1] * tangentWorld[1] +
      normalWorld[2] * tangentWorld[2];
    expect(Math.abs(dot)).toBeLessThan(1e-9);
    // 单位长度（法线用于光照与几何判断，必须归一化）。
    expect(Math.hypot(normalWorld[0], normalWorld[1], normalWorld[2])).toBeCloseTo(1, 9);
  });

  it("把法线当普通方向量变换会破坏垂直性（固化'为什么不能那样做'）", () => {
    const naive = transformDirection(transform, normalLocal);
    const length = Math.hypot(naive[0], naive[1], naive[2]) || 1;
    const naiveUnit: Vec3 = [naive[0] / length, naive[1] / length, naive[2] / length];
    const tangentWorld = transformDirection(transform, tangentLocal);
    const dot =
      naiveUnit[0] * tangentWorld[0] +
      naiveUnit[1] * tangentWorld[1] +
      naiveUnit[2] * tangentWorld[2];
    expect(Math.abs(dot)).toBeGreaterThan(0.1);
  });

  it("世界方向 → 局部分向与局部 → 世界方向互逆（相机 up 用前者）", () => {
    const worldDirection: Vec3 = [0.2, 0.9, -0.35];
    const local = worldDirectionToLocal(transform, worldDirection);
    expectVec3Close(transformDirection(transform, local), worldDirection, 9);
  });
});

describe("有限性校验与锚点状态（T19 复用）", () => {
  it("NaN/Infinity 不是有限局部点", () => {
    expect(isFiniteVec3([0, 0, 0])).toBe(true);
    expect(isFiniteVec3([Number.NaN, 0, 0])).toBe(false);
    expect(isFiniteVec3([Number.POSITIVE_INFINITY, 0, 0])).toBe(false);
    expect(isFiniteVec3([0, 0])).toBe(false);
    expect(isFiniteVec3("0,0,0")).toBe(false);
  });

  it("makeAnchor 拒绝非有限数值（不产生 [0,0,0] 之类的占位）", () => {
    expect(makeAnchor({ revisionId: "r1", sha256: "s1" }, [Number.NaN, 1, 2])).toBeNull();
    const anchor = makeAnchor({ revisionId: "r1", sha256: "s1" }, [1, 2, 3]);
    expect(anchor).not.toBeNull();
    expect(anchor?.positionLocal).toEqual([1, 2, 3]);
  });

  it("checkAnchor：版本或哈希变化即 stale（不得当有效热点显示）", () => {
    const anchor = makeAnchor({ revisionId: "r1", sha256: "s1" }, [1, 2, 3]);
    expect(anchor).not.toBeNull();
    expect(checkAnchor(anchor, { revisionId: "r1", sha256: "s1" })).toEqual({
      usable: true,
      reason: "current",
    });
    expect(checkAnchor(anchor, { revisionId: "r2", sha256: "s1" })).toEqual({
      usable: false,
      reason: "staleRevision",
    });
    expect(checkAnchor(anchor, { revisionId: "r1", sha256: "s2" })).toEqual({
      usable: false,
      reason: "staleSha",
    });
    expect(checkAnchor(null, { revisionId: "r1", sha256: "s1" })).toEqual({
      usable: false,
      reason: "invalid",
    });
  });

  it("相机位姿读写同源：readCameraPose → applyCameraPose 回到同一世界量", () => {
    const camera = {
      position: [1.2, 2.4, 3.6] as Vec3,
      up: [0, 1, 0] as Vec3,
      fov: 40,
    };
    const target: Vec3 = [0.1, -0.2, 0.3];
    const pose = readCameraPose(ASSET_ROOT_TRANSFORM, camera, target);
    const applied = applyCameraPose(ASSET_ROOT_TRANSFORM, pose);
    expectVec3Close(applied.position, camera.position, 9);
    expectVec3Close(applied.target, target, 9);
    expectVec3Close(applied.up, camera.up, 9);
    expect(applied.fov).toBe(40);
  });
});

describe("适配（fit）语义：显示变换只在外层（AC-051/UI-043）", () => {
  const bounds = { min: [-1.2, -0.6, -0.4] as Vec3, max: [1.4, 0.7, 0.5] as Vec3 };

  it("居中 + 等比例缩放：模型中心落到世界原点，最大半径归一到 1", () => {
    const fit = computeFit(bounds, { fovDeg: 40, aspect: 16 / 9 });
    const center: Vec3 = [
      (bounds.min[0] + bounds.max[0]) / 2,
      (bounds.min[1] + bounds.max[1]) / 2,
      (bounds.min[2] + bounds.max[2]) / 2,
    ];
    const display: Transform = {
      position: fit.displayOffset,
      quaternion: [0, 0, 0, 1],
      scale: [fit.displayScale, fit.displayScale, fit.displayScale],
    };
    expectVec3Close(localToWorld(display, center), [0, 0, 0], 9);
    const corner: Vec3 = [bounds.max[0], bounds.max[1], bounds.max[2]];
    const cornerWorld = localToWorld(display, corner);
    const radius = Math.hypot(cornerWorld[0], cornerWorld[1], cornerWorld[2]);
    // 包围盒半径 = 对角线的一半；缩放后应位于 0.5..1 之间（归一化到单位球量级）。
    expect(radius).toBeGreaterThan(0.4);
    expect(radius).toBeLessThanOrEqual(1.0 + 1e-9);
  });

  it("窄视口需要更远距离（不裁切），且相机看向模型中心", () => {
    const wide = computeFit(bounds, { fovDeg: 40, aspect: 16 / 9 });
    const narrow = computeFit(bounds, { fovDeg: 40, aspect: 3 / 4 });
    expect(narrow.distance).toBeGreaterThan(wide.distance);
    expectVec3Close(wide.target, [0, 0, 0], 9);
    expect(Math.hypot(...wide.cameraPosition)).toBeCloseTo(wide.distance, 9);
  });

  it("非有限包围盒被拒绝（不产生 NaN 相机参数）", () => {
    expect(() =>
      computeFit({ min: [Number.NaN, 0, 0], max: [1, 1, 1] }, { fovDeg: 40, aspect: 1 }),
    ).toThrow();
  });
});

describe("显示变换不修改 asset-root 自身（architecture §5.4）", () => {
  it("给 display group 设变换后，asset-root 的局部变换保持原样", () => {
    const display = DISPLAY_TRANSFORMS[1] as Transform;
    const { displayGroup, assetRoot } = buildSceneGraph(display);
    const displayEuler = new Euler().setFromQuaternion(new Quaternion(...display.quaternion));
    displayGroup.rotation.copy(displayEuler);
    assetRoot.updateMatrixWorld(true);
    // asset-root 的局部变换 = 模型自身的变换（未被显示变换污染）。
    const localPosition = new Vector3();
    const localQuaternion = new Quaternion();
    const localScale = new Vector3();
    assetRoot.matrix.decompose(localPosition, localQuaternion, localScale);
    expect([localPosition.x, localPosition.y, localPosition.z]).toEqual([
      ...ASSET_ROOT_TRANSFORM.position,
    ]);
    expect(localScale.x).toBeCloseTo(ASSET_ROOT_TRANSFORM.scale[0], 12);
  });
});
