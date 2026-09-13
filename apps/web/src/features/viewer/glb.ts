/**
 * GLB 加载与释放（T18 / REQ-032；contracts §7、architecture §5.4）。
 *
 * 浏览器侧的职责（服务端 T13 已做过一遍结构校验，这里是**纵深防御**）：
 * 1. **自包含检查**：GLB 的 buffer / image 不得带 `uri`（含 `data:` 之外的任何
 *    外链），`extensionsRequired` 必须为空。带外链的模型在浏览器里会去请求外部
 *    地址——既违反"模型只作资料资产、不执行脚本"的边界，也让渲染依赖第三方网络。
 *    客户端直接拒绝并给出可读原因，不尝试加载。
 * 2. **哈希核对**：草稿记录的 `modelSha256` 与实际字节比对；不一致即失败（不渲染
 *    "看起来像但其实是另一个版本"的模型）。
 * 3. **解析与释放**：用 `GLTFLoader.parse` 解析自包含 GLB；释放时按账本
 *    （`resources.ts`）逐个 `dispose()` geometry/material/texture。
 *
 * 注意：`GLTFLoader.parse` 对 data URI 与 blob URL 是允许的，因此第 1 步必须在
 * 解析前完成，不能依赖加载器抛错。
 */

import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import type { Group } from "three";

import { trackModelResources, type TrackedModel } from "./resources";

export type ViewerErrorCode =
  | "model_http_error"
  | "model_not_glb"
  | "model_external_resource"
  | "model_required_extension"
  | "model_hash_mismatch"
  | "model_parse_failed"
  | "model_empty";

/** 观看器可读错误：`code` 稳定（测试与文案按它分支），`message` 面向用户。 */
export class ViewerError extends Error {
  readonly code: ViewerErrorCode;

  constructor(code: ViewerErrorCode, message: string) {
    super(message);
    this.name = "ViewerError";
    this.code = code;
  }
}

interface GltfJson {
  asset?: unknown;
  buffers?: { uri?: unknown }[];
  images?: { uri?: unknown }[];
  extensionsRequired?: unknown;
  meshes?: unknown;
  /** 仅用于测试断言"节点带有旋转/非均匀缩放"；不参与运行逻辑。 */
  nodes?: { rotation?: unknown; scale?: unknown }[];
}

interface GlbChunks {
  readonly json: GltfJson;
  readonly binLength: number;
}

/** 读取 GLB 容器（magic/version/length/chunk 布局），返回 JSON 与 BIN 长度。 */
export function readGlbChunks(buffer: ArrayBuffer): GlbChunks {
  if (buffer.byteLength < 20) {
    throw new ViewerError("model_not_glb", "模型文件过小，不是有效的 GLB");
  }
  const view = new DataView(buffer);
  const magic = view.getUint32(0, true);
  const version = view.getUint32(4, true);
  const declared = view.getUint32(8, true);
  // glTF magic 的四个 ASCII 字节 "glTF" 小端读法是 0x46546c67。
  if (magic !== 0x46546c67) {
    throw new ViewerError("model_not_glb", "模型不是 GLB（缺少 glTF magic）");
  }
  if (version !== 2) {
    throw new ViewerError("model_not_glb", `不支持的 GLB 版本：${version}（只支持 glTF 2.0）`);
  }
  if (declared !== buffer.byteLength) {
    throw new ViewerError(
      "model_not_glb",
      `GLB 声明长度（${declared}）与实际字节数（${buffer.byteLength}）不一致：疑似截断或损坏`,
    );
  }

  let offset = 12;
  let json: GltfJson | null = null;
  let binLength = 0;
  let index = 0;
  while (offset + 8 <= buffer.byteLength) {
    const chunkLength = view.getUint32(offset, true);
    const chunkType = view.getUint32(offset + 4, true);
    const start = offset + 8;
    const end = start + chunkLength;
    if (end > buffer.byteLength) {
      throw new ViewerError("model_not_glb", "GLB chunk 长度越过文件末尾：文件已损坏");
    }
    const bytes = new Uint8Array(buffer, start, chunkLength);
    if (index === 0) {
      if (chunkType !== 0x4e4f534a) {
        throw new ViewerError("model_not_glb", "GLB 的首个 chunk 必须是 JSON");
      }
      let text: string;
      try {
        text = new TextDecoder().decode(bytes);
      } catch {
        throw new ViewerError("model_not_glb", "GLB 的 JSON chunk 无法解码为 UTF-8");
      }
      try {
        json = JSON.parse(text) as GltfJson;
      } catch {
        throw new ViewerError("model_not_glb", "GLB 的 JSON chunk 不是合法 JSON");
      }
    } else {
      if (chunkType !== 0x004e4942) {
        throw new ViewerError("model_not_glb", "GLB 中出现未知 chunk 类型（只接受 JSON + BIN）");
      }
      binLength += chunkLength;
    }
    offset = end;
    index += 1;
  }
  if (json === null) {
    throw new ViewerError("model_not_glb", "GLB 缺少 JSON chunk");
  }
  return { json, binLength };
}

/**
 * 自包含检查（解析前）：拒绝外链 buffer/image 与未支持的 required extension。
 * 与 T13 服务端 `glb::inspect_glb_file` 的判定口径一致（浏览器侧只做必要子集）。
 */
export function assertSelfContainedGlb(buffer: ArrayBuffer): GlbChunks {
  const chunks = readGlbChunks(buffer);
  const { json } = chunks;
  const externalBuffers = (json.buffers ?? []).filter(
    (entry) => entry !== null && typeof entry === "object" && typeof entry.uri === "string",
  );
  const externalImages = (json.images ?? []).filter(
    (entry) => entry !== null && typeof entry === "object" && typeof entry.uri === "string",
  );
  if (externalBuffers.length > 0 || externalImages.length > 0) {
    throw new ViewerError(
      "model_external_resource",
      "模型引用了外部资源（buffer/image 的 uri）：阅读器只接受自包含 GLB，不请求外部地址",
    );
  }
  const required = json.extensionsRequired;
  if (Array.isArray(required) && required.length > 0) {
    throw new ViewerError(
      "model_required_extension",
      `模型要求未支持的 glTF 扩展：${required.map(String).join(", ")}`,
    );
  }
  return chunks;
}

/** `ArrayBuffer` → sha256 十六进制（WebCrypto；与合同 `sha256` 字段同口径）。 */
export async function sha256Hex(buffer: ArrayBuffer): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", buffer);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

export interface LoadedModel {
  /** 解析出的场景根（asset-root）：局部坐标即它的局部坐标。 */
  readonly scene: Group;
  /** 资源账本（释放时使用；`dispose()` 幂等）。 */
  readonly resources: TrackedModel;
  /** 释放：geometry/material/texture。再次调用是空操作。 */
  dispose(): void;
}

export interface LoadModelOptions {
  /** 期望的内容哈希（来自草稿的模型版本事实）；缺省时跳过核对。 */
  readonly expectedSha256?: string | null;
}

/**
 * 校验并解析 GLB。
 *
 * 抛出 `ViewerError`（自包含/哈希/解析失败）——调用方据 `code` 展示可读错误。
 * 解析成功即记账（`trackModelResources`），调用方必须在卸载/换模型时调用
 * `dispose()`，否则账本的 `alive` 会持续增长（e2e 会断言这一点）。
 */
export async function loadGlbModel(buffer: ArrayBuffer, options: LoadModelOptions = {}): Promise<LoadedModel> {
  assertSelfContainedGlb(buffer);

  const expected = options.expectedSha256 ?? null;
  if (expected !== null && expected !== "") {
    const actual = await sha256Hex(buffer);
    if (actual !== expected.toLowerCase()) {
      throw new ViewerError(
        "model_hash_mismatch",
        `模型内容哈希与记录不一致（期望 ${expected.slice(0, 12)}…，实际 ${actual.slice(0, 12)}…）：拒绝渲染，请重新生成或核对资料`,
      );
    }
  }

  const loader = new GLTFLoader();
  let scene: Group;
  try {
    const gltf = await new Promise<{ scene: Group }>((resolve, reject) => {
      loader.parse(buffer, "", (result) => resolve(result as { scene: Group }), (error) => {
        reject(error instanceof Error ? error : new Error(String(error)));
      });
    });
    scene = gltf.scene;
  } catch (error) {
    throw new ViewerError(
      "model_parse_failed",
      `模型解析失败：${error instanceof Error ? error.message : String(error)}`,
    );
  }

  let hasMesh = false;
  scene.traverse((object) => {
    const geometry = (object as unknown as { geometry?: { isBufferGeometry?: boolean } }).geometry;
    if (geometry !== undefined && geometry.isBufferGeometry === true) {
      hasMesh = true;
    }
  });
  if (!hasMesh) {
    throw new ViewerError("model_empty", "模型不含可渲染几何：没有可显示的网格");
  }

  const resources = trackModelResources(scene);
  return {
    scene,
    resources,
    dispose(): void {
      resources.dispose();
    },
  };
}

/** 只读的可渲染统计（模型面数/贴图数；面板与 QA 展示用）。 */
export interface ModelStats {
  readonly triangles: number;
  readonly objects: number;
  readonly textures: number;
}

export function summarizeScene(scene: Group): ModelStats {
  let triangles = 0;
  let objects = 0;
  const textures = new Set<unknown>();
  scene.traverse((object) => {
    const mesh = object as unknown as {
      geometry?: { isBufferGeometry?: boolean; index?: { count: number } | null; attributes?: { position?: { count: number } } };
      material?: unknown;
    };
    objects += 1;
    const geometry = mesh.geometry;
    if (geometry?.isBufferGeometry === true) {
      const index = geometry.index;
      const position = geometry.attributes?.position;
      if (index != null) {
        triangles += Math.floor(index.count / 3);
      } else if (position !== undefined) {
        triangles += Math.floor(position.count / 3);
      }
    }
    const material = mesh.material as Record<string, unknown> | undefined;
    if (material !== undefined && material !== null && typeof material === "object") {
      for (const value of Object.values(material)) {
        const texture = value as { isTexture?: boolean } | null;
        if (texture !== null && typeof texture === "object" && texture.isTexture === true) {
          textures.add(texture);
        }
      }
    }
  });
  return { triangles, objects, textures: textures.size };
}
