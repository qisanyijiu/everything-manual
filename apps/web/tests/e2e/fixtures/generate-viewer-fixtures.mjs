#!/usr/bin/env node
/**
 * 阅读器（T18）测试模型生成器：现场构造 glTF 2.0 二进制（GLB）测试资产。
 *
 * 来源与许可（与 tests/fixtures/README.md 同一约定）：
 * - 只按公开规范（glTF 2.0 GLB 容器、PNG）写字节，没有拷贝任何厂商模型或贴图；
 * - 几何、贴图、法线全部由本脚本计算，虚构形状；许可与仓库源码相同；
 * - 输出逐字节确定（无随机数、无时间戳），哈希可复现、可写进断言。
 *
 * 为什么需要这些模型（PRD §4 AC-051 / REQ-040）：
 * - `viewer-asymmetric.glb`：**不对称**形状 + 节点带**旋转与非均匀缩放**
 *   （测试"同一局部点在不同旋转/缩放下一致"与"法线必须用逆转置"）；
 *   含内嵌 PNG 贴图（测试贴图释放）。
 * - `viewer-asymmetric-b.glb`：第二个不对称模型（换模型测试：旧资源/旧热点不串入）。
 * - `--large`：约 100k 三角面模型（性能测量用；按 REQ-040 的默认预算）。
 *
 * 用法：
 *   node generate-viewer-fixtures.mjs                # 生成两个小模型
 *   node generate-viewer-fixtures.mjs --large         # 追加生成 100k 面模型
 *   node generate-viewer-fixtures.mjs --out <dir>     # 指定输出目录（默认本目录）
 */

import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { crc32, deflateSync } from "node:zlib";

const HERE = path.dirname(new URL(import.meta.url).pathname);

// ---------------------------------------------------------------------------
// PNG（真彩 RGB，8 bit，无压缩直存块）：只为实现最小可解码贴图
// ---------------------------------------------------------------------------

function pngFromPixels(width, height, pixels) {
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const chunk = (type, data) => {
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length, 0);
    const typeAndData = Buffer.concat([Buffer.from(type, "ascii"), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(typeAndData) >>> 0, 0);
    return Buffer.concat([length, typeAndData, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // color type: truecolor
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;
  const raw = Buffer.alloc(height * (1 + width * 3));
  for (let y = 0; y < height; y += 1) {
    const rowStart = y * (1 + width * 3);
    raw[rowStart] = 0; // filter: none
    for (let x = 0; x < width; x += 1) {
      const [r, g, b] = pixels(x, y);
      raw[rowStart + 1 + x * 3] = r;
      raw[rowStart + 2 + x * 3] = g;
      raw[rowStart + 3 + x * 3] = b;
    }
  }
  const idat = deflateSync(raw);
  return Buffer.concat([
    signature,
    chunk("IHDR", ihdr),
    chunk("IDAT", idat),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/** 16×16 贴图：左半偏红、右半偏蓝（贴图是否被正确采样/释放可分辨）。 */
function checkerTexture() {
  return pngFromPixels(16, 16, (x, y) => {
    const left = x < 8;
    const bright = (x + y) % 2 === 0;
    if (left) {
      return bright ? [200, 90, 70] : [150, 60, 45];
    }
    return bright ? [70, 110, 190] : [45, 80, 150];
  });
}

// ---------------------------------------------------------------------------
// GLB 容器
// ---------------------------------------------------------------------------

const COMPONENT_FLOAT = 5126;
const COMPONENT_USHORT = 5123;
const TARGET_ARRAY_BUFFER = 34962;
const TARGET_ELEMENT_ARRAY_BUFFER = 34963;

function padTo4(buffer, fill) {
  const remainder = buffer.length % 4;
  if (remainder === 0) {
    return buffer;
  }
  return Buffer.concat([buffer, Buffer.alloc(4 - remainder, fill)]);
}

/** 把"顶点属性 + 索引 + 贴图"打成自包含 GLB（BIN chunk + bufferView 内嵌贴图）。 */
function buildGlb({ positions, normals, uvs, indices, node, texturePng, meshName }) {
  const chunks = [];
  let offset = 0;
  const bufferViews = [];
  const accessors = [];

  const pushAccessor = (data, { componentType, type, target, count, min, max }) => {
    const bytes = padTo4(Buffer.from(data.buffer, data.byteOffset, data.byteLength), 0);
    bufferViews.push({ buffer: 0, byteOffset: offset, byteLength: data.byteLength, target });
    chunks.push(bytes);
    offset += bytes.length;
    accessors.push({
      bufferView: bufferViews.length - 1,
      componentType,
      count,
      type,
      ...(min !== undefined ? { min, max } : {}),
    });
  };

  // POSITION 的 min/max 是 glTF 的**必填**字段，逐分量给出。
  pushAccessor(new Float32Array(positions), {
    componentType: COMPONENT_FLOAT,
    type: "VEC3",
    target: TARGET_ARRAY_BUFFER,
    count: positions.length / 3,
    min: componentMinMax(positions, 3, 0),
    max: componentMinMax(positions, 3, 1),
  });
  pushAccessor(new Float32Array(normals), {
    componentType: COMPONENT_FLOAT,
    type: "VEC3",
    target: TARGET_ARRAY_BUFFER,
    count: normals.length / 3,
  });
  pushAccessor(new Float32Array(uvs), {
    componentType: COMPONENT_FLOAT,
    type: "VEC2",
    target: TARGET_ARRAY_BUFFER,
    count: uvs.length / 2,
  });
  pushAccessor(new Uint16Array(indices), {
    componentType: COMPONENT_USHORT,
    type: "SCALAR",
    target: TARGET_ELEMENT_ARRAY_BUFFER,
    count: indices.length,
  });

  // 贴图 bytes 追加到 BIN 末尾（独立 bufferView，无 uri = 内嵌）。
  const png = padTo4(texturePng, 0);
  bufferViews.push({ buffer: 0, byteOffset: offset, byteLength: texturePng.length });
  const imageView = bufferViews.length - 1;
  chunks.push(png);
  offset += png.length;

  const bin = Buffer.concat(chunks);
  const json = {
    asset: { version: "2.0", generator: "everything-manual viewer fixtures" },
    scene: 0,
    scenes: [{ nodes: [0] }],
    nodes: [
      {
        name: "asset-root-child",
        mesh: 0,
        ...(node.translation !== undefined ? { translation: node.translation } : {}),
        ...(node.rotation !== undefined ? { rotation: node.rotation } : {}),
        ...(node.scale !== undefined ? { scale: node.scale } : {}),
      },
    ],
    meshes: [
      {
        name: meshName,
        primitives: [
          {
            attributes: { POSITION: 0, NORMAL: 1, TEXCOORD_0: 2 },
            indices: 3,
            material: 0,
          },
        ],
      },
    ],
    materials: [
      {
        name: "fixture-material",
        pbrMetallicRoughness: {
          baseColorTexture: { index: 0 },
          metallicFactor: 0,
          roughnessFactor: 0.85,
        },
      },
    ],
    textures: [{ sampler: 0, source: 0 }],
    samplers: [{ magFilter: 9729, minFilter: 9729, wrapS: 33071, wrapT: 33071 }],
    images: [{ bufferView: imageView, mimeType: "image/png" }],
    buffers: [{ byteLength: bin.length }],
    bufferViews,
    accessors,
  };

  const jsonChunk = padTo4(Buffer.from(JSON.stringify(json), "utf8"), 0x20);
  const binChunk = padTo4(bin, 0);
  const header = Buffer.alloc(12);
  header.writeUInt32LE(0x46546c67, 0); // "glTF"
  header.writeUInt32LE(2, 4);
  header.writeUInt32LE(12 + 8 + jsonChunk.length + 8 + binChunk.length, 8);
  const jsonHeader = Buffer.alloc(8);
  jsonHeader.writeUInt32LE(jsonChunk.length, 0);
  jsonHeader.writeUInt32LE(0x4e4f534a, 4); // JSON
  const binHeader = Buffer.alloc(8);
  binHeader.writeUInt32LE(binChunk.length, 0);
  binHeader.writeUInt32LE(0x004e4942, 4); // BIN
  return {
    bytes: Buffer.concat([header, jsonHeader, jsonChunk, binHeader, binChunk]),
    indexCount: indices.length,
  };
}

function componentMinMax(values, components, which) {
  const out = new Array(components).fill(which === 0 ? Infinity : -Infinity);
  for (let i = 0; i < values.length; i += components) {
    for (let c = 0; c < components; c += 1) {
      const value = values[i + c];
      if (which === 0) {
        out[c] = Math.min(out[c], value);
      } else {
        out[c] = Math.max(out[c], value);
      }
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// 几何
// ---------------------------------------------------------------------------

/** 8 顶点六面体（不规则、非对称）→ 非索引三角形；返回顶点数组与扁平法线。 */
function hexahedron(corners) {
  const faces = [
    [0, 1, 2, 3], // 后
    [4, 5, 6, 7], // 前
    [0, 1, 5, 4], // 下
    [3, 2, 6, 7], // 上
    [0, 3, 7, 4], // 左
    [1, 2, 6, 5], // 右
  ];
  const positions = [];
  const normals = [];
  const uvs = [];
  const indices = [];
  const pushVertex = (corner, normal, u, v) => {
    positions.push(corners[corner][0], corners[corner][1], corners[corner][2]);
    normals.push(normal[0], normal[1], normal[2]);
    uvs.push(u, v);
    return positions.length / 3 - 1;
  };
  for (const [a, b, c, d] of faces) {
    const normal = faceNormal(corners[a], corners[b], corners[c]);
    const i0 = pushVertex(a, normal, 0, 0);
    const i1 = pushVertex(b, normal, 1, 0);
    const i2 = pushVertex(c, normal, 1, 1);
    const i3 = pushVertex(d, normal, 0, 1);
    indices.push(i0, i1, i2, i0, i2, i3);
  }
  return { positions, normals, uvs, indices };
}

function faceNormal(a, b, c) {
  const ux = b[0] - a[0];
  const uy = b[1] - a[1];
  const uz = b[2] - a[2];
  const vx = c[0] - a[0];
  const vy = c[1] - a[1];
  const vz = c[2] - a[2];
  const nx = uy * vz - uz * vy;
  const ny = uz * vx - ux * vz;
  const nz = ux * vy - uy * vx;
  const length = Math.hypot(nx, ny, nz) || 1;
  return [nx / length, ny / length, nz / length];
}

/** 绕轴角 → 四元数（与 three 的 Quaternion 顺序一致：x, y, z, w）。 */
function quatFromEuler(x, y, z) {
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

function applyNodeToPoint(node, point) {
  const scale = node.scale ?? [1, 1, 1];
  const rotation = node.rotation ?? [0, 0, 0, 1];
  const translation = node.translation ?? [0, 0, 0];
  const [sx, sy, sz] = [point[0] * scale[0], point[1] * scale[1], point[2] * scale[2]];
  const [x, y, z, w] = rotation;
  const tx = 2 * (y * sz - z * sy);
  const ty = 2 * (z * sx - x * sz);
  const tz = 2 * (x * sy - y * sx);
  return [
    sx + w * tx + (y * tz - z * ty) + translation[0],
    sy + w * ty + (z * tx - x * tz) + translation[1],
    sz + w * tz + (x * ty - y * tx) + translation[2],
  ];
}

const MODEL_A = {
  file: "viewer-asymmetric.glb",
  meshName: "asymmetric-a",
  corners: [
    [-1.0, -0.5, -0.3],
    [1.2, -0.5, -0.35],
    [1.05, 0.5, -0.3],
    [-0.9, 0.55, -0.3],
    [-0.8, -0.6, 0.32],
    [0.95, -0.45, 0.25],
    [1.1, 0.65, 0.4],
    [-1.0, 0.4, 0.28],
  ],
  node: {
    translation: [0.3, -0.2, 0.15],
    rotation: quatFromEuler(0.35, -0.8, 0.25),
    // 非均匀缩放（asset-root 下的节点）：法线必须用逆转置才正确。
    scale: [0.8, 1.7, 0.45],
  },
};

const MODEL_B = {
  file: "viewer-asymmetric-b.glb",
  meshName: "asymmetric-b",
  corners: [
    [-0.6, -0.9, -0.5],
    [1.4, -0.7, -0.2],
    [0.7, 0.3, -0.6],
    [-0.5, 0.9, -0.1],
    [-0.7, -0.8, 0.6],
    [0.9, -0.4, 0.5],
    [1.2, 0.5, 0.2],
    [-0.3, 0.6, 0.7],
  ],
  node: {
    translation: [-0.4, 0.3, -0.1],
    rotation: quatFromEuler(-0.5, 0.6, -0.2),
    scale: [1.6, 0.5, 0.9],
  },
};

/** 100k 面地形（性能测量用）：解析法线、Uint16 索引。 */
function largeTerrain({ segments = 224, size = 4 } = {}) {
  const n = segments;
  const positions = [];
  const normals = [];
  const uvs = [];
  const height = (x, y) =>
    0.35 * Math.sin(2.1 * x) * Math.cos(1.7 * y) + 0.18 * Math.sin(3.3 * x + 1.1 * y);
  const gradient = (x, y) => [
    0.35 * 2.1 * Math.cos(2.1 * x) * Math.cos(1.7 * y) + 0.18 * 3.3 * Math.cos(3.3 * x + 1.1 * y),
    -0.35 * 1.7 * Math.sin(2.1 * x) * Math.sin(1.7 * y) + 0.18 * 1.1 * Math.cos(3.3 * x + 1.1 * y),
  ];
  for (let iy = 0; iy <= n; iy += 1) {
    for (let ix = 0; ix <= n; ix += 1) {
      const x = (ix / n - 0.5) * size;
      const y = (iy / n - 0.5) * size;
      const z = height(x, y);
      positions.push(x, y, z);
      const [dzdx, dzdy] = gradient(x, y);
      const nx = -dzdx;
      const ny = -dzdy;
      const nz = 1;
      const length = Math.hypot(nx, ny, nz);
      normals.push(nx / length, ny / length, nz / length);
      uvs.push(ix / n, iy / n);
    }
  }
  const indices = [];
  for (let iy = 0; iy < n; iy += 1) {
    for (let ix = 0; ix < n; ix += 1) {
      const a = iy * (n + 1) + ix;
      const b = a + 1;
      const c = a + (n + 1);
      const d = c + 1;
      indices.push(a, c, b, b, c, d);
    }
  }
  return { positions, normals, uvs, indices };
}

// ---------------------------------------------------------------------------
// 主流程
// ---------------------------------------------------------------------------

function main() {
  const args = process.argv.slice(2);
  const outIndex = args.indexOf("--out");
  const outDir = outIndex >= 0 ? path.resolve(args[outIndex + 1]) : HERE;
  const withLarge = args.includes("--large");
  mkdirSync(outDir, { recursive: true });

  const written = [];
  for (const model of [MODEL_A, MODEL_B]) {
    const geometry = hexahedron(model.corners);
    const glb = buildGlb({
      ...geometry,
      node: model.node,
      texturePng: checkerTexture(),
      meshName: model.meshName,
    });
    const target = path.join(outDir, model.file);
    writeFileSync(target, glb.bytes);
    // 供 e2e 使用的表面采样点：写在 asset-root 局部坐标（= 节点变换之后）。
    const points = [model.corners[0], model.corners[2], model.corners[6]].map((corner) =>
      applyNodeToPoint(model.node, corner),
    );
    const pointsFile = path.join(outDir, model.file.replace(/\.glb$/, ".points.json"));
    writeFileSync(
      pointsFile,
      `${JSON.stringify({ assetRootLocalPoints: points }, null, 2)}\n`,
    );
    written.push(
      `${model.file}: ${glb.bytes.length} bytes, ${geometry.indices.length / 3} triangles`,
    );
    written.push(`${path.basename(pointsFile)}: ${points.length} asset-root 局部采样点`);
  }

  if (withLarge) {
    const geometry = largeTerrain();
    const glb = buildGlb({
      ...geometry,
      node: { translation: [0, 0, 0] },
      texturePng: checkerTexture(),
      meshName: "terrain-100k",
    });
    const target = path.join(outDir, "viewer-large.glb");
    writeFileSync(target, glb.bytes);
    written.push(
      `viewer-large.glb: ${glb.bytes.length} bytes, ${geometry.indices.length / 3} triangles`,
    );
  }

  process.stdout.write(`输出目录：${outDir}\n${written.join("\n")}\n`);
}

main();
