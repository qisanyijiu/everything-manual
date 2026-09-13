/**
 * 坐标适配层：asset-root 局部坐标 ↔ 世界坐标（T18 / REQ-032、REQ-033 的读取语义；
 * PRD §4 AC-051；architecture §5.4；contracts §2「Hotspot / CameraPose」）。
 *
 * 术语与不变量（T19 复用，变更必须先改本节）：
 * - **asset-root**：GLB 导入后的根对象（`GLTFLoader` 返回的场景根 Group）。它的
 *   局部坐标系就是**唯一权威的锚点空间**：`positionLocal` / `CameraPose.*Local`
 *   全部都写在 asset-root 局部坐标里。
 * - **world**：Three.js 场景根坐标（相机与射线检测所在空间）。
 * - **显示居中/缩放放在外层 group**：适配模型用的平移/缩放在 asset-root **外层**
 *   的 display group 上，不写进 asset-root 自身变换，也不烘焙进顶点。因此
 *   "同一局部点"在显示变换改变时仍落在模型表面的同一处，保存下来的局部坐标不变。
 * - **锚点身份只来自不可变版本**：`modelRevisionId + modelSha256`（见 `Anchor`）。
 *   **不得**用运行时 `mesh.uuid`、three 对象引用或易变节点名作唯一锚点——
 *   重新加载/重新生成后这些都会变（architecture §5.4）。
 * - **法线**：非均匀缩放下方向量与法线的变换不同。方向（切线等）用线性部分
 *   `R·S`；法线必须用逆转置 `(R·S)⁻ᵀ`（本文件 `transformNormal`）。把法线当
 *   普通方向量变换会在非均匀缩放下产生错误光照与错误几何判断。
 *
 * 本文件是**纯数学**（不 import three）：可在 Node/Vitest 直接验证，也避免把
 * three 拉进首屏包。与 three 对象图的桥接见 `asset-root.ts`。
 */

/** 三维向量（JSON 友好的元组形式，与合同的 `positionLocal` 数值序列一致）。 */
export type Vec3 = readonly [number, number, number];

/** 四元数 `[x, y, z, w]`（与 three 的 `Quaternion` 顺序一致）。 */
export type Quat = readonly [number, number, number, number];

/**
 * 一个对象相对其父级的变换（与 three 的 `Object3D` 约定一致：
 * `world = T · R · S`，即先缩放、再旋转、最后平移）。
 */
export interface Transform {
  readonly position: Vec3;
  readonly quaternion: Quat;
  readonly scale: Vec3;
}

/** 轴对齐包围盒（局部坐标；来自 GLB 几何或服务端 `bounds` 摘要）。 */
export interface Bounds {
  readonly min: Vec3;
  readonly max: Vec3;
}

// ---------------------------------------------------------------------------
// 有限性校验（T19：anchor 数值必须有限，禁止 NaN/Infinity）
// ---------------------------------------------------------------------------

export function isFiniteVec3(value: unknown): value is Vec3 {
  return (
    Array.isArray(value) &&
    value.length === 3 &&
    value.every((item) => typeof item === "number" && Number.isFinite(item))
  );
}

export function isFiniteQuat(value: unknown): value is Quat {
  return (
    Array.isArray(value) &&
    value.length === 4 &&
    value.every((item) => typeof item === "number" && Number.isFinite(item))
  );
}

/** 变换可用于计算（避免把 NaN/Infinity 静默带进保存的锚点）。 */
export function isFiniteTransform(transform: Transform): boolean {
  return (
    isFiniteVec3(transform.position) &&
    isFiniteQuat(transform.quaternion) &&
    isFiniteVec3(transform.scale) &&
    transform.scale.every((component) => component !== 0)
  );
}

// ---------------------------------------------------------------------------
// 四元数 / 线性代数（只实现本层需要的部分，保持无依赖）
// ---------------------------------------------------------------------------

/** 四元数乘法 `a · b`（先施加 b，再施加 a）。 */
export function multiplyQuat(a: Quat, b: Quat): Quat {
  const [ax, ay, az, aw] = a;
  const [bx, by, bz, bw] = b;
  return [
    aw * bx + ax * bw + ay * bz - az * by,
    aw * by - ax * bz + ay * bw + az * bx,
    aw * bz + ax * by - ay * bx + az * bw,
    aw * bw - ax * bx - ay * by - az * bz,
  ];
}

/** 四元数共轭；单位四元数下等于逆（只处理单位四元数，不在此做数值归一）。 */
export function conjugateQuat(q: Quat): Quat {
  return [-q[0], -q[1], -q[2], q[3]];
}

/** 用单位四元数旋转向量（不含缩放与平移）。 */
export function rotateByQuat(q: Quat, v: Vec3): Vec3 {
  const [x, y, z, w] = q;
  const [vx, vy, vz] = v;
  // t = 2 · (q_vec × v)；v' = v + w·t + q_vec × t
  const tx = 2 * (y * vz - z * vy);
  const ty = 2 * (z * vx - x * vz);
  const tz = 2 * (x * vy - y * vx);
  return [
    vx + w * tx + (y * tz - z * ty),
    vy + w * ty + (z * tx - x * tz),
    vz + w * tz + (x * ty - y * tx),
  ];
}

/** 绕轴角构造四元数（轴不必归一；`degrees` 为角度制）。 */
export function quatFromAxisAngle(axis: Vec3, degrees: number): Quat {
  const [ax, ay, az] = axis;
  const length = Math.hypot(ax, ay, az);
  if (length === 0) {
    throw new Error("轴长度为零：无法构造四元数");
  }
  const radians = (degrees * Math.PI) / 180;
  const half = radians / 2;
  const s = Math.sin(half) / length;
  return [ax * s, ay * s, az * s, Math.cos(half)];
}

/** 欧拉角（弧度，three 的 XYZ 顺序）→ 四元数。用于测试与 T19 的预设视角。 */
export function quatFromEulerXYZ(euler: Vec3): Quat {
  const [x, y, z] = euler;
  const c1 = Math.cos(x / 2);
  const c2 = Math.cos(y / 2);
  const c3 = Math.cos(z / 2);
  const s1 = Math.sin(x / 2);
  const s2 = Math.sin(y / 2);
  const s3 = Math.sin(z / 2);
  return [
    s1 * c2 * c3 + c1 * s2 * s3,
    c1 * s2 * c3 - s1 * c2 * s3,
    c1 * c2 * s3 + s1 * s2 * c3,
    c1 * c2 * c3 - s1 * s2 * s3,
  ];
}

/** 组合变换 `parent · child`（与 three 的父子矩阵一致）。 */
export function composeTransforms(parent: Transform, child: Transform): Transform {
  const scaledChildPosition: Vec3 = [
    child.position[0] * parent.scale[0],
    child.position[1] * parent.scale[1],
    child.position[2] * parent.scale[2],
  ];
  const rotated = rotateByQuat(parent.quaternion, scaledChildPosition);
  return {
    position: [
      parent.position[0] + rotated[0],
      parent.position[1] + rotated[1],
      parent.position[2] + rotated[2],
    ],
    quaternion: multiplyQuat(parent.quaternion, child.quaternion),
    // 父级非均匀缩放会剪切子级：调用方只允许把**等比例缩放**的外层显示变换
    // （`ViewerStage` 的 display group）与本函数组合，因此逐分量相乘与 three 一致。
    scale: [
      parent.scale[0] * child.scale[0],
      parent.scale[1] * child.scale[1],
      parent.scale[2] * child.scale[2],
    ],
  };
}

// ---------------------------------------------------------------------------
// 点 / 方向 / 法线的变换
// ---------------------------------------------------------------------------

/** 局部点 → 世界点：`T · R · S · p`。 */
export function localToWorld(transform: Transform, point: Vec3): Vec3 {
  const [sx, sy, sz] = transform.scale;
  const [px, py, pz] = point;
  const rotated = rotateByQuat(transform.quaternion, [px * sx, py * sy, pz * sz]);
  const [tx, ty, tz] = transform.position;
  return [rotated[0] + tx, rotated[1] + ty, rotated[2] + tz];
}

/** 世界点 → 局部点：`S⁻¹ · Rᵀ · (w - T)`（保存锚点用这一方向）。 */
export function worldToLocal(transform: Transform, world: Vec3): Vec3 {
  const [tx, ty, tz] = transform.position;
  const relative = rotateByQuat(conjugateQuat(transform.quaternion), [
    world[0] - tx,
    world[1] - ty,
    world[2] - tz,
  ]);
  const [sx, sy, sz] = transform.scale;
  return [relative[0] / sx, relative[1] / sy, relative[2] / sz];
}

/**
 * 局部**方向**（切线、位移差等）→ 世界方向：线性部分 `R · S`，不含平移。
 * 法线不要用本函数（见 `transformNormal`）。
 */
export function transformDirection(transform: Transform, direction: Vec3): Vec3 {
  const [sx, sy, sz] = transform.scale;
  return rotateByQuat(transform.quaternion, [
    direction[0] * sx,
    direction[1] * sy,
    direction[2] * sz,
  ]);
}

/**
 * 世界方向 → 局部方向：线性部分的逆 `S⁻¹ · Rᵀ`（`transformDirection` 的反向）。
 * 相机 `up` 在世界坐标给出，写进 `CameraPose.upLocal` 时必须走这一方向。
 */
export function worldDirectionToLocal(transform: Transform, direction: Vec3): Vec3 {
  const rotated = rotateByQuat(conjugateQuat(transform.quaternion), direction);
  const [sx, sy, sz] = transform.scale;
  return [rotated[0] / sx, rotated[1] / sy, rotated[2] / sz];
}

/**
 * 局部**法线** → 世界法线：线性部分的逆转置 `(R·S)⁻ᵀ = R·S⁻¹`，再归一化。
 *
 * 为什么不能直接用 `transformDirection`：非均匀缩放下法线与切线不再同变换，
 * 用 `R·S` 会让变换后的法线与表面不垂直（测试 `coordinates.test.ts` 固化了
 * 这一点，包含"朴素做法不垂直"的反例断言）。
 */
export function transformNormal(transform: Transform, normal: Vec3): Vec3 {
  const [sx, sy, sz] = transform.scale;
  const scaled: Vec3 = [normal[0] / sx, normal[1] / sy, normal[2] / sz];
  const rotated = rotateByQuat(transform.quaternion, scaled);
  const length = Math.hypot(rotated[0], rotated[1], rotated[2]);
  if (length === 0) {
    return [0, 0, 0];
  }
  return [rotated[0] / length, rotated[1] / length, rotated[2] / length];
}

// ---------------------------------------------------------------------------
// 适配（fit）与相机位姿（CameraPose；T19 保存视角时复用同一语义）
// ---------------------------------------------------------------------------

/** 相机位姿（全部相对同一 asset-root；contracts §2 `CameraPose`）。 */
export interface CameraPose {
  readonly positionLocal: Vec3;
  readonly targetLocal: Vec3;
  readonly upLocal: Vec3;
  readonly fov: number;
}

/** 适配参数：把模型放正、缩放到单位球、相机拉到能看全的距离。 */
export interface FitResult {
  /** 外层显示 group 的缩放（等比例；不写进 asset-root）。 */
  readonly displayScale: number;
  /** 外层显示 group 的平移（居中；不写进 asset-root）。 */
  readonly displayOffset: Vec3;
  /** 相机到模型中心的距离（世界单位，外层缩放之后）。 */
  readonly distance: number;
  /** 相机看向的点（世界坐标；等于显示变换后的模型中心）。 */
  readonly target: Vec3;
  /** 初始相机位置（世界坐标）。 */
  readonly cameraPosition: Vec3;
}

export const DEFAULT_FIT_MARGIN = 1.35;

/**
 * 由局部包围盒与相机视锥计算适配参数。
 *
 * 约定：显示缩放把模型最大半径归一到 1（长边 = 2 世界单位），相机看向模型中心；
 * `distance = max(distanceForHeight, distanceForWidth) · margin`，因此各种视口
 * 比例下都完整可见（fit 的定义，`reset` 回到同一参数）。
 */
export function computeFit(
  bounds: Bounds,
  options: { fovDeg: number; aspect: number; margin?: number; direction?: Vec3 },
): FitResult {
  if (!isFiniteVec3(bounds.min) || !isFiniteVec3(bounds.max)) {
    throw new Error("包围盒不是有限数值：无法计算适配参数");
  }
  const center: Vec3 = [
    (bounds.min[0] + bounds.max[0]) / 2,
    (bounds.min[1] + bounds.max[1]) / 2,
    (bounds.min[2] + bounds.max[2]) / 2,
  ];
  const extent: Vec3 = [
    bounds.max[0] - bounds.min[0],
    bounds.max[1] - bounds.min[1],
    bounds.max[2] - bounds.min[2],
  ];
  const radius = Math.max(Math.hypot(extent[0], extent[1], extent[2]) / 2, 1e-6);
  const displayScale = 1 / radius;
  // 外层显示 group：先平移（-center）再缩放（displayScale），
  // 使模型中心落到世界原点、最大半径变 1。
  const displayOffset: Vec3 = [
    -center[0] * displayScale,
    -center[1] * displayScale,
    -center[2] * displayScale,
  ];
  const margin = options.margin ?? DEFAULT_FIT_MARGIN;
  const fovRad = (options.fovDeg * Math.PI) / 180;
  const halfHeight = Math.tan(fovRad / 2);
  const halfWidth = halfHeight * Math.max(options.aspect, 1e-6);
  const distance = (margin * Math.max(1 / halfHeight, 1 / halfWidth));
  const direction = options.direction ?? [0, 0, 1];
  const [dx, dy, dz] = direction;
  const length = Math.hypot(dx, dy, dz);
  if (length === 0) {
    throw new Error("方向向量长度为零：无法计算相机位置");
  }
  const unit: Vec3 = [dx / length, dy / length, dz / length];
  return {
    displayScale,
    displayOffset,
    distance,
    target: [0, 0, 0],
    cameraPosition: [unit[0] * distance, unit[1] * distance, unit[2] * distance],
  };
}

/**
 * 相机位姿（世界 → asset-root 局部）。
 *
 * `camera.up` 与 `position` 一样是**世界量**（相机挂在场景根下），因此两者都用
 * "世界 → 局部"的同一方向转换，不能混用 `transformDirection`（那是反向）。
 */
export function readCameraPose(
  assetRoot: Transform,
  camera: { position: Vec3; up: Vec3; fov: number },
  targetWorld: Vec3,
): CameraPose {
  return {
    positionLocal: worldToLocal(assetRoot, camera.position),
    targetLocal: worldToLocal(assetRoot, targetWorld),
    upLocal: worldDirectionToLocal(assetRoot, camera.up),
    fov: camera.fov,
  };
}

/** 局部位姿 → 世界（相机、控制器目标与 up）。读取与写入必须用同一对变换。 */
export function applyCameraPose(
  assetRoot: Transform,
  pose: CameraPose,
): { position: Vec3; target: Vec3; up: Vec3; fov: number } {
  return {
    position: localToWorld(assetRoot, pose.positionLocal),
    target: localToWorld(assetRoot, pose.targetLocal),
    up: transformDirection(assetRoot, pose.upLocal),
    fov: pose.fov,
  };
}

// ---------------------------------------------------------------------------
// 锚点（T19 复用；本卡只读消费）
// ---------------------------------------------------------------------------

/**
 * 热点/视角锚点的**可保存形态**：绑定不可变模型版本 + 内容哈希 + asset-root 局部点。
 * 用 `modelRevisionId + modelSha256` 而不是运行时对象引用作身份（architecture §5.4）。
 */
export interface Anchor {
  readonly modelRevisionId: string;
  readonly modelSha256: string;
  readonly positionLocal: Vec3;
}

export function makeAnchor(model: {
  revisionId: string;
  sha256: string;
}, positionLocal: Vec3): Anchor | null {
  if (!isFiniteVec3(positionLocal)) {
    return null;
  }
  return {
    modelRevisionId: model.revisionId,
    modelSha256: model.sha256,
    positionLocal,
  };
}

export interface AnchorStatus {
  readonly usable: boolean;
  readonly reason: "current" | "staleRevision" | "staleSha" | "invalid";
}

/**
 * 锚点是否仍可用于当前模型：版本与哈希都必须匹配，数值必须有限。
 * 不匹配即 stale（旧锚点可以保留解释，但不得当有效热点显示）。
 */
export function checkAnchor(
  anchor: Anchor | null,
  model: { revisionId: string; sha256: string },
): AnchorStatus {
  if (anchor === null) {
    return { usable: false, reason: "invalid" };
  }
  if (!isFiniteVec3(anchor.positionLocal)) {
    return { usable: false, reason: "invalid" };
  }
  if (anchor.modelRevisionId !== model.revisionId) {
    return { usable: false, reason: "staleRevision" };
  }
  if (anchor.modelSha256 !== model.sha256) {
    return { usable: false, reason: "staleSha" };
  }
  return { usable: true, reason: "current" };
}
