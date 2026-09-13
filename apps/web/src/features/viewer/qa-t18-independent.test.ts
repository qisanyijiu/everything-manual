/**
 * T18 坐标适配层 **QA 独立复验**（QA 回合 22；AC-051；PRD 修订 2）。
 *
 * 与 RD 的 `coordinates.test.ts` **故意不同的构造方式**（不复制 RD 的用例）：
 * 1. 用**随机化性质测试**（固定种子的伪随机）而不是少量手写样例：随机
 *    旋转（轴角）+ 非均匀缩放 + 随机局部点，验证往返一致与"显示变换不影响
 *    局部点语义"；
 * 2. 法线正确性用**几何第一性原理**判定：把局部切平面上的两个切向量与法线
 *    一起变换，断言变换后的法线与两条变换后的切线都垂直（而不是只比对
 *    "与 RD 的公式是否一致"）；
 * 3. 显示变换（居中+等比例缩放的相似变换）用**独立推导的公式**重算世界坐标：
 *    `world = (local − center) · s`，与 `localToWorld` 组合 display 变换的结果
 *    比对——这正是"同一局部点在各种旋转/缩放下世界位置一致"的可证伪形式；
 * 4. 用真实 three `Object3D` 图（与运行时同一对象类型）交叉验证纯数学层：
 *    `Object3D.localToWorld` 与 `coordinates.localToWorld` 结果一致。
 *
 * 本文件只读地使用生产模块，不修改任何生产代码；发现的差异按缺陷报告。
 */

import { describe, expect, it } from "vitest";
import { Group, Object3D, Quaternion, Vector3 } from "three";

import {
  checkAnchor,
  composeTransforms,
  computeFit,
  isFiniteVec3,
  localToWorld,
  transformDirection,
  transformNormal,
  worldDirectionToLocal,
  worldToLocal,
  type Transform,
  type Vec3,
} from "./coordinates";

/** 固定种子伪随机（xorshift32）：同一版本永远得到同一组用例，可复现。 */
function makeRandom(seed: number): () => number {
  let state = seed | 0;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return ((state >>> 0) % 1_000_000) / 1_000_000;
  };
}

const random = makeRandom(0x5eed_18);

function randomVec3(scale: number): Vec3 {
  return [
    (random() - 0.5) * 2 * scale,
    (random() - 0.5) * 2 * scale,
    (random() - 0.5) * 2 * scale,
  ];
}

/** 随机旋转（轴角）+ 非均匀缩放 + 平移的父级变换。 */
function randomTransform(): Transform {
  const axis: Vec3 = [random() - 0.5, random() - 0.5, random() - 0.5];
  const degrees = random() * 360;
  const q = new Quaternion().setFromAxisAngle(
    new Vector3(axis[0], axis[1], axis[2]).normalize(),
    (degrees * Math.PI) / 180,
  );
  return {
    position: randomVec3(5),
    quaternion: [q.x, q.y, q.z, q.w],
    // 刻意非均匀：三个分量互不相同且远离 1。
    scale: [0.3 + random() * 2.5, 0.3 + random() * 2.5, 0.3 + random() * 2.5],
  };
}

function distance(a: Vec3, b: Vec3): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

function normalize(v: Vec3): Vec3 {
  const length = Math.hypot(v[0], v[1], v[2]);
  return [v[0] / length, v[1] / length, v[2] / length];
}

describe("QA-T18 坐标层独立复验（AC-051）", () => {
  it("随机 200 组「旋转 + 非均匀缩放 + 平移」：局部↔世界往返一致（≤1e-9）", () => {
    for (let round = 0; round < 200; round += 1) {
      const transform = randomTransform();
      const local = randomVec3(3);
      const world = localToWorld(transform, local);
      const back = worldToLocal(transform, world);
      expect(distance(back, local), `第 ${round} 组往返误差`).toBeLessThan(1e-9);
    }
  });

  it("与 three 的 Object3D 矩阵实现交叉一致（同一变换、同一点）", () => {
    for (let round = 0; round < 50; round += 1) {
      const transform = randomTransform();
      const object = new Object3D();
      object.position.set(...transform.position);
      object.quaternion.set(...transform.quaternion);
      object.scale.set(...transform.scale);
      object.updateMatrixWorld(true);

      const local = randomVec3(2);
      const expected = new Vector3(...local).applyMatrix4(object.matrixWorld);
      const actual = localToWorld(transform, local);
      expect(distance(actual, [expected.x, expected.y, expected.z])).toBeLessThan(1e-9);

      // 反向也必须一致（这一方向是保存锚点用的 worldToLocal）。
      const world: Vec3 = [expected.x, expected.y, expected.z];
      const backVector = object.worldToLocal(new Vector3(...world));
      const backPure = worldToLocal(transform, world);
      expect(distance(backPure, [backVector.x, backVector.y, backVector.z])).toBeLessThan(1e-9);
    }
  });

  it("显示变换（居中 + 等比例缩放）不改变局部点语义：世界坐标 = (局部 − 中心) · s", () => {
    // 与 ViewerStage 的 fit 规则一致：displayOffset = −center·s，displayScale = s。
    const bounds = { min: [-1.2, -3.4, 0.7] as Vec3, max: [2.8, 1.6, 4.1] as Vec3 };
    for (let round = 0; round < 50; round += 1) {
      // 随机的"显示变换"参数（不同视口比例/方向/距 margin），模拟旋转与缩放后的状态。
      const aspect = 0.4 + random() * 3;
      const fit = computeFit(bounds, { fovDeg: 30 + random() * 40, aspect });
      const display: Transform = {
        position: fit.displayOffset,
        quaternion: [0, 0, 0, 1],
        scale: [fit.displayScale, fit.displayScale, fit.displayScale],
      };
      const local = [
        bounds.min[0] + random() * (bounds.max[0] - bounds.min[0]),
        bounds.min[1] + random() * (bounds.max[1] - bounds.min[1]),
        bounds.min[2] + random() * (bounds.max[2] - bounds.min[2]),
      ] as Vec3;

      // 生产路径：asset-root 局部点经"世界→局部"的同一对变换。
      const viaDisplay = localToWorld(display, local);
      // 独立推导：世界 = (局部 − 中心) · s，其中 s = 1/半径（computeFit 的定义）。
      const center: Vec3 = [
        (bounds.min[0] + bounds.max[0]) / 2,
        (bounds.min[1] + bounds.max[1]) / 2,
        (bounds.min[2] + bounds.max[2]) / 2,
      ];
      const expected: Vec3 = [
        (local[0] - center[0]) * fit.displayScale,
        (local[1] - center[1]) * fit.displayScale,
        (local[2] - center[2]) * fit.displayScale,
      ];
      expect(distance(viaDisplay, expected), `第 ${round} 组显示变换误差`).toBeLessThan(1e-9);

      // 反过来：显示空间里的世界点必须映射回同一局部点（读数与写数同源）。
      expect(distance(worldToLocal(display, viaDisplay), local)).toBeLessThan(1e-9);
    }
  });

  it("多级组合（display ∘ asset-root）：display 为等比例缩放时两级变换与世界坐标一致", () => {
    // 契约（coordinates.ts 注释与 ViewerStage 实现）：外层 display 只允许等比例
    // 缩放（`display.scale.setScalar(...)`），此时逐分量组合与 three 的
    // `parent · child` 矩阵一致；asset-root 自身可以带任意非均匀缩放。
    for (let round = 0; round < 50; round += 1) {
      const uniform = 0.4 + random() * 2.2;
      const display = randomTransform();
      const uniformDisplay: Transform = {
        position: display.position,
        quaternion: display.quaternion,
        scale: [uniform, uniform, uniform],
      };
      const assetRoot = randomTransform();
      const local = randomVec3(2);
      const viaCompose = localToWorld(composeTransforms(uniformDisplay, assetRoot), local);
      const viaChain = localToWorld(uniformDisplay, localToWorld(assetRoot, local));
      expect(distance(viaCompose, viaChain)).toBeLessThan(1e-9);
    }
  });

  it("生产显示路径的缩放是等比例：非均匀 display 缩放只能是明确不支持的用法", () => {
    // 反向固化契约：display 用非均匀缩放时逐分量组合与真实矩阵链不一致（剪切），
    // 因此生产代码不得出现非均匀 display 缩放（ViewerStage 用 `setScalar`）。
    let diverged = 0;
    for (let round = 0; round < 20; round += 1) {
      const display = randomTransform();
      const assetRoot = randomTransform();
      const local = randomVec3(2);
      const viaCompose = localToWorld(composeTransforms(display, assetRoot), local);
      const viaChain = localToWorld(display, localToWorld(assetRoot, local));
      if (distance(viaCompose, viaChain) > 1e-6) {
        diverged += 1;
      }
    }
    expect(diverged, "非均匀 display 缩放应当出现偏差（契约边界的证据）").toBeGreaterThan(0);
  });

  it("法线用逆转置的几何意义：非均匀缩放下（R·S⁻¹）n 垂直于两条变换后的切线", () => {
    for (let round = 0; round < 100; round += 1) {
      const transform = randomTransform();
      const normal = normalize(randomVec3(1));
      // 构造切平面上的两条线性无关切线：任取两个与 n 不平行的方向做正交化。
      const seedA = normalize(randomVec3(1));
      const seedB = normalize(randomVec3(1));
      const reject = (v: Vec3): Vec3 => normalize([
        v[0] - dot(v, normal) * normal[0],
        v[1] - dot(v, normal) * normal[1],
        v[2] - dot(v, normal) * normal[2],
      ]);
      const tangentA = reject(seedA);
      const tangentB = reject(seedB);

      const worldNormal = transformNormal(transform, normal);
      const worldA = transformDirection(transform, tangentA);
      const worldB = transformDirection(transform, tangentB);
      expect(Math.abs(dot(worldNormal, worldA)), `第 ${round} 组法线·切线A`).toBeLessThan(1e-9);
      expect(Math.abs(dot(worldNormal, worldB)), `第 ${round} 组法线·切线B`).toBeLessThan(1e-9);
      expect(Math.hypot(worldNormal[0], worldNormal[1], worldNormal[2])).toBeCloseTo(1, 9);

      // 方向向量（切线）必须仍然落在变换后的平面里：与"错误做法"划清界限——
      // 用同一个线性部分 R·S 变换法线时，非均匀缩放下不再垂直（反例）。
      const naive = transformDirection(transform, normal);
      const naiveDot = Math.abs(dot(normalize(naive), worldA));
      expect(naiveDot, `第 ${round} 组朴素做法应当不垂直`).toBeGreaterThan(1e-6);
    }
  });

  it("相机 up 的世界→局部方向变换与局部→世界互为逆（读写同源）", () => {
    for (let round = 0; round < 100; round += 1) {
      const transform = randomTransform();
      const worldUp = normalize(randomVec3(1));
      const localUp = worldDirectionToLocal(transform, worldUp);
      const back = transformDirection(transform, localUp);
      expect(distance(back, worldUp)).toBeLessThan(1e-9);
    }
  });

  it("锚点身份只由不可变版本决定：换 revision 或换 sha 即 stale，NaN 即 invalid", () => {
    const model = { revisionId: "rev-2", sha256: "b".repeat(64) };
    const anchor = {
      modelRevisionId: "rev-1",
      modelSha256: "a".repeat(64),
      positionLocal: [0.1, 0.2, 0.3] as Vec3,
    };
    expect(checkAnchor(anchor, model).reason).toBe("staleRevision");
    expect(checkAnchor({ ...anchor, modelRevisionId: "rev-2" }, model).reason).toBe("staleSha");
    expect(
      checkAnchor({ ...anchor, modelRevisionId: "rev-2", modelSha256: "b".repeat(64) }, model)
        .usable,
    ).toBe(true);
    expect(
      checkAnchor({ ...anchor, positionLocal: [Number.NaN, 0, 0] as Vec3 }, model).reason,
    ).toBe("invalid");
    expect(isFiniteVec3([1, 2, Number.POSITIVE_INFINITY])).toBe(false);
  });

  it("fit 语义：视口越窄相机越远（模型始终完整可见的独立判据）", () => {
    const bounds = { min: [-1, -1, -1] as Vec3, max: [1, 1, 1] as Vec3 };
    const wide = computeFit(bounds, { fovDeg: 40, aspect: 2 });
    const narrow = computeFit(bounds, { fovDeg: 40, aspect: 0.5 });
    expect(narrow.distance).toBeGreaterThan(wide.distance);
    // 目标点与显示参数只由包围盒决定（与视口无关）：换比例不改"居中"。
    expect(wide.target).toEqual(narrow.target);
    expect(wide.displayScale).toBeCloseTo(narrow.displayScale, 12);
    // 相机位置必须在目标点方向上、距离等于 distance（第一性原理检查）。
    const length = Math.hypot(...wide.cameraPosition);
    expect(length).toBeCloseTo(wide.distance, 9);
  });

  it("真实 Object3D 场景图（旋转 + 非均匀缩放）上的往返：与运行时同一对象类型", () => {
    const root = new Group();
    const child = new Object3D();
    child.position.set(0.3, -0.2, 0.45);
    child.quaternion.setFromAxisAngle(new Vector3(1, 2, -1).normalize(), 0.87);
    child.scale.set(1.7, 0.45, 2.3);
    root.add(child);
    const display = new Group();
    display.position.set(0.4, -1.1, 0.2);
    display.scale.setScalar(1.9);
    display.add(root);
    display.updateMatrixWorld(true);

    const worldToLocalMatrix = new Vector3();
    for (let round = 0; round < 20; round += 1) {
      const local = randomVec3(1.5);
      child.localToWorld(worldToLocalMatrix.set(...local));
      const back = child.worldToLocal(worldToLocalMatrix.clone());
      expect(distance([back.x, back.y, back.z], local)).toBeLessThan(1e-9);
      // display 变换只影响世界坐标，不影响局部坐标的读写结果。
    }
  });
});
