/**
 * GLB 加载/校验/释放的单元测试（T18 / REQ-032；contracts §7、AC-050 的释放条款）。
 *
 * 覆盖：
 * 1. 自包含检查：外链 buffer/image、required extension、非 GLB、声明长度不符都被拒绝
 *    （浏览器侧纵深防御：即使服务端已校验，阅读器也不请求外部地址）；
 * 2. 哈希核对：`sha256Hex` 与 node:crypto 的独立实现对拍（合同 `sha256` 字段同口径）；
 * 3. 资源账本：`trackModelResources` 的 created/alive/disposed 一一对应（"卸载后
 *    geometry/material/texture 被释放"的可断言证据）。
 */

import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";

import { afterEach, describe, expect, it } from "vitest";
import { BufferGeometry, Group, Mesh, MeshStandardMaterial, Texture } from "three";

import { ViewerError, assertSelfContainedGlb, readGlbChunks, sha256Hex } from "./glb";
import { resetViewerResources, trackModelResources, viewerResourceStats } from "./resources";

/**
 * fixture 目录定位：jsdom 环境下 `import.meta.url` 不是 file: 协议，
 * 因此按仓库布局从 cwd 解析（vitest 的根 = `apps/web`，与 npm script 一致）。
 */
const FIXTURE_DIR = (() => {
  const candidates = [
    path.join(process.cwd(), "tests", "e2e", "fixtures"),
    path.join(process.cwd(), "apps", "web", "tests", "e2e", "fixtures"),
  ];
  const found = candidates.find((candidate) =>
    existsSync(path.join(candidate, "viewer-asymmetric.glb")),
  );
  if (found === undefined) {
    throw new Error(`找不到阅读器 fixture 目录（已尝试：${candidates.join(", ")}）`);
  }
  return found;
})();

function readFixture(name: string): ArrayBuffer {
  const bytes = readFileSync(path.join(FIXTURE_DIR, name));
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer;
}

function fixtureBuffer(): ArrayBuffer {
  return readFixture("viewer-asymmetric.glb");
}

/** 构造一个最小 GLB（JSON chunk 可自定义），用于负例。 */
function synthGlb(json: Record<string, unknown>): ArrayBuffer {
  const jsonText = JSON.stringify(json);
  const jsonPadding = (4 - (jsonText.length % 4)) % 4;
  const jsonBytes = new TextEncoder().encode(jsonText + " ".repeat(jsonPadding));
  const bin = new Uint8Array(8);
  const total = 12 + 8 + jsonBytes.length + 8 + bin.length;
  const buffer = new ArrayBuffer(total);
  const view = new DataView(buffer);
  const bytes = new Uint8Array(buffer);
  view.setUint32(0, 0x46546c67, true);
  view.setUint32(4, 2, true);
  view.setUint32(8, total, true);
  view.setUint32(12, jsonBytes.length, true);
  view.setUint32(16, 0x4e4f534a, true);
  bytes.set(jsonBytes, 20);
  const binHeader = 20 + jsonBytes.length;
  view.setUint32(binHeader, bin.length, true);
  view.setUint32(binHeader + 4, 0x004e4942, true);
  return buffer;
}

afterEach(() => {
  resetViewerResources();
});

describe("GLB 自包含检查（contracts §7）", () => {
  it("仓库 fixture 通过检查并给出 JSON/BIN 结构", () => {
    const buffer = fixtureBuffer();
    const chunks = assertSelfContainedGlb(buffer);
    expect(chunks.binLength).toBeGreaterThan(0);
    expect(readGlbChunks(buffer).json).toBeDefined();
    // 不对称 fixture 的关键特征：asset-root 下的节点带旋转 + 非均匀缩放。
    const node = chunks.json.nodes?.[0];
    expect(node?.rotation).toBeInstanceOf(Array);
    expect(node?.scale).toBeInstanceOf(Array);
    const scale = node?.scale as number[];
    expect(new Set(scale).size).toBeGreaterThan(1);
  });

  it("外链 buffer 被拒绝（model_external_resource）", () => {
    const buffer = synthGlb({
      asset: { version: "2.0" },
      buffers: [{ byteLength: 4, uri: "https://example.invalid/model.bin" }],
    });
    expect(() => assertSelfContainedGlb(buffer)).toThrowError(ViewerError);
    try {
      assertSelfContainedGlb(buffer);
    } catch (error) {
      expect((error as ViewerError).code).toBe("model_external_resource");
    }
  });

  it("外链 image 被拒绝（不请求外部地址）", () => {
    const buffer = synthGlb({
      asset: { version: "2.0" },
      buffers: [{ byteLength: 4 }],
      images: [{ uri: "textures/color.png" }],
    });
    try {
      assertSelfContainedGlb(buffer);
      throw new Error("应当抛出");
    } catch (error) {
      expect((error as ViewerError).code).toBe("model_external_resource");
    }
  });

  it("required extension 被拒绝（首版不支持任何必需扩展）", () => {
    const buffer = synthGlb({
      asset: { version: "2.0" },
      buffers: [{ byteLength: 4 }],
      extensionsRequired: ["KHR_draco_mesh_compression"],
    });
    try {
      assertSelfContainedGlb(buffer);
      throw new Error("应当抛出");
    } catch (error) {
      expect((error as ViewerError).code).toBe("model_required_extension");
    }
  });

  it("data: URI 的 buffer 也按外链处理（阅读器不接受任何 uri 形态）", () => {
    const buffer = synthGlb({
      asset: { version: "2.0" },
      buffers: [{ byteLength: 4, uri: "data:application/octet-stream;base64,AAAA" }],
    });
    try {
      assertSelfContainedGlb(buffer);
      throw new Error("应当抛出");
    } catch (error) {
      expect((error as ViewerError).code).toBe("model_external_resource");
    }
  });

  it("非 GLB（magic 不符）与声明长度不符都被拒绝", () => {
    const notGlb = new ArrayBuffer(32);
    try {
      assertSelfContainedGlb(notGlb);
      throw new Error("应当抛出");
    } catch (error) {
      expect((error as ViewerError).code).toBe("model_not_glb");
    }

    const truncated = fixtureBuffer();
    const view = new DataView(truncated);
    view.setUint32(8, view.getUint32(8, true) + 4, true); // 声明长度 > 实际长度
    try {
      assertSelfContainedGlb(truncated);
      throw new Error("应当抛出");
    } catch (error) {
      expect((error as ViewerError).code).toBe("model_not_glb");
    }
  });
});

describe("模型内容哈希（与合同 sha256 同口径）", () => {
  it("sha256Hex 与 node:crypto 的独立实现一致", async () => {
    const buffer = fixtureBuffer();
    const expected = createHash("sha256").update(Buffer.from(buffer)).digest("hex");
    await expect(sha256Hex(buffer)).resolves.toBe(expected);
    expect(expected).toMatch(/^[0-9a-f]{64}$/);
  });

  it("两个 fixture 的哈希不同（换模型测试依赖这一点）", async () => {
    const first = await sha256Hex(fixtureBuffer());
    const second = await sha256Hex(readFixture("viewer-asymmetric-b.glb"));
    expect(first).not.toBe(second);
  });
});

describe("资源账本：卸载后 geometry/material/texture 被释放", () => {
  it("created/disposed/alive 一一对应（含共享材质与嵌套节点）", () => {
    resetViewerResources();
    const texture = new Texture();
    const material = new MeshStandardMaterial({ map: texture });
    const sharedMaterial = new MeshStandardMaterial();
    const geometry = new BufferGeometry();
    const root = new Group();
    const first = new Mesh(geometry, material);
    const second = new Mesh(new BufferGeometry(), [sharedMaterial, material]);
    root.add(first, second);

    const tracked = trackModelResources(root);
    expect(tracked.counts).toEqual({ geometries: 2, materials: 2, textures: 1 });
    const before = viewerResourceStats();
    expect(before.modelsLoaded).toBe(1);
    expect(before.modelsAlive).toBe(1);
    expect(before.geometries.alive).toBe(2);
    expect(before.materials.alive).toBe(2);
    expect(before.textures.alive).toBe(1);

    tracked.dispose();
    const after = viewerResourceStats();
    expect(after.geometries).toEqual({ created: 2, disposed: 2, alive: 0 });
    expect(after.materials).toEqual({ created: 2, disposed: 2, alive: 0 });
    expect(after.textures).toEqual({ created: 1, disposed: 1, alive: 0 });
    expect(after.modelsAlive).toBe(0);

    // 幂等：重复 dispose 不会把计数减成负数（换模型/卸载路径可能重复触发）。
    tracked.dispose();
    expect(viewerResourceStats().geometries.alive).toBe(0);
  });
});
