/**
 * 离线 3D 阅读器的数据载荷（导出 HTML 内嵌的 JSON；SPA 写入、离线脚本读取）。
 *
 * 设计要点：
 * - **只放已发布版本的不可变内容**：知识来自 release manifest，模型字节来自
 *   manifest 指向的 GLB；不含会话、密钥、服务端 URL 与内部资产 ID 以外的东西；
 * - **离线脚本不再做语义校验**：载荷由 SPA 用 `draft-view` 的同一套读取函数整理，
 *   离线侧只按本文件的类型渲染（文字一律走 textContent，不拼 HTML）；
 * - 坐标沿用合同语义：`positionLocal` 是 GLB 场景根（asset-root）的局部坐标。
 */

/** 载荷 schema 版本（离线脚本据此拒绝无法理解的文件）。 */
export const STANDALONE_PAYLOAD_VERSION = "em_standalone_viewer_v1";

/** 内嵌 `<script type="application/json">` 的元素 ID。 */
export const STANDALONE_PAYLOAD_ELEMENT_ID = "em-payload";

/** 内嵌 GLB（base64）的元素 ID：与 JSON 分开，避免超大字符串进 JSON.parse。 */
export const STANDALONE_MODEL_ELEMENT_ID = "em-model";

export type Vec3Tuple = readonly [number, number, number];

export interface StandaloneEvidence {
  readonly pageNumber: number;
  readonly quote: string | null;
}

export interface StandalonePart {
  readonly id: string;
  readonly name: string;
  readonly description: string;
  readonly evidence: readonly StandaloneEvidence[];
}

export interface StandaloneStep {
  readonly id: string;
  readonly title: string;
  readonly orderedActions: readonly string[];
  readonly safetyNotes: readonly string[];
  readonly partIds: readonly string[];
  readonly evidence: readonly StandaloneEvidence[];
}

export interface StandaloneSpec {
  readonly id: string;
  readonly label: string;
  readonly value: string;
  readonly evidence: readonly StandaloneEvidence[];
}

export interface StandaloneHotspot {
  readonly id: string;
  readonly partId: string;
  readonly positionLocal: Vec3Tuple;
}

export interface StandalonePayload {
  readonly schemaVersion: typeof STANDALONE_PAYLOAD_VERSION;
  readonly title: string;
  readonly subtitle: string;
  readonly releaseId: string;
  readonly publishedAt: string | null;
  readonly model: {
    readonly sha256: string;
  };
  readonly parts: readonly StandalonePart[];
  readonly steps: readonly StandaloneStep[];
  readonly specs: readonly StandaloneSpec[];
  readonly hotspots: readonly StandaloneHotspot[];
  /** 交互层（ADR-042）：有值时内嵌的 GLB 是分件模型（与热点同一坐标系）。 */
  readonly interactive: {
    readonly bindings: readonly { readonly partId: string; readonly nodes: readonly string[] }[];
    readonly actions: readonly import("../viewer/interactive-view").ModelActionView[];
    readonly poses: readonly import("../viewer/interactive-view").ModelPoseView[];
  } | null;
}
