/**
 * 组装离线 3D 阅读器单文件 HTML（release manifest + GLB 字节 → 可双击打开的 .html）。
 *
 * - 知识读取复用在线阅读器的发布版本修订层，保留已冻结的人工更正；
 * - 只放**锚点仍属于发布模型版本**的热点（与 `ReleaseReaderPage` 的读取侧防线一致）；
 * - 所有内嵌文本都经过转义：JSON 里的 `<` 改写为 `<`，防止 `</script>` 截断；
 *   标题等进 HTML 的文字用 `escapeHtml`。离线脚本写 DOM 时只用 textContent。
 */

import { checkAnchor } from "../viewer/coordinates";
import { readInteractive } from "../viewer/interactive-view";
import { readReleaseKnowledge } from "../manual/release-view";
import {
  readDraftHotspots,
  readDraftModel,
  type DraftEvidence,
} from "../viewer/draft-view";
import {
  STANDALONE_MODEL_ELEMENT_ID,
  STANDALONE_PAYLOAD_ELEMENT_ID,
  STANDALONE_PAYLOAD_VERSION,
  type StandaloneEvidence,
  type StandalonePayload,
} from "./payload";
import { STANDALONE_STYLES } from "./styles";

export interface StandaloneItemInfo {
  readonly name: string;
  readonly brand: string | null;
  readonly model: string | null;
}

export interface StandaloneReleaseInfo {
  readonly id: string;
  readonly manifest: unknown;
}

export class StandaloneExportError extends Error {}

function toEvidence(evidence: readonly DraftEvidence[]): StandaloneEvidence[] {
  return evidence.map((item) => ({ pageNumber: item.pageNumber, quote: item.quote }));
}

/** 从 release manifest 整理离线载荷（纯函数；不含模型字节）。 */
export function buildStandalonePayload(
  item: StandaloneItemInfo,
  release: StandaloneReleaseInfo,
): StandalonePayload {
  const manifest = (release.manifest ?? {}) as { knowledge?: unknown; review?: unknown; publishedAt?: unknown };
  const knowledge = manifest.knowledge;
  const reviewed = readReleaseKnowledge(knowledge, manifest.review);
  const model = readDraftModel(knowledge);
  if (model === null) {
    throw new StandaloneExportError("该发布版本没有可用的 3D 模型，无法生成离线 3D 页面。");
  }
  const hotspots = readDraftHotspots(knowledge).flatMap((hotspot) =>
    hotspot.anchor !== null && checkAnchor(hotspot.anchor, model).usable
      ? [{ id: hotspot.id, partId: hotspot.partId, positionLocal: hotspot.anchor.positionLocal }]
      : [],
  );
  const subtitle = [item.brand, item.model].filter((value) => value !== null && value !== "");
  return {
    schemaVersion: STANDALONE_PAYLOAD_VERSION,
    title: item.name,
    subtitle: subtitle.length > 0 ? subtitle.join(" · ") : "交互式说明书",
    releaseId: release.id,
    publishedAt: typeof manifest.publishedAt === "string" ? manifest.publishedAt : null,
    model: { sha256: model.sha256 },
    parts: reviewed.parts.map((part) => ({
      id: part.id,
      name: part.name,
      description: part.description,
      evidence: toEvidence(part.evidence),
    })),
    steps: reviewed.steps.map((step) => ({
      id: step.id,
      title: step.title,
      orderedActions: step.orderedActions,
      safetyNotes: step.safetyNotes,
      partIds: step.partIds,
      evidence: toEvidence(step.evidence),
    })),
    specs: reviewed.specs.map((spec) => ({
      id: spec.id,
      label: spec.label,
      value: spec.value,
      evidence: toEvidence(spec.evidence),
    })),
    hotspots,
    interactive: (() => {
      const view = readInteractive(knowledge, model);
      return view === null
        ? null
        : {
            nodeNames: view.partsModel.nodeNames,
            bindings: view.bindings.map((binding) => ({ partId: binding.partId, nodes: binding.nodes })),
            actions: view.actions,
            poses: view.poses,
          };
    })(),
  };
}

/** 离线页应内嵌的 GLB：有交互层时用分件模型，否则用模型本身。 */
export function standaloneModelAssetId(manifest: unknown): string | null {
  const knowledge = (manifest as { knowledge?: unknown } | null)?.knowledge;
  const model = readDraftModel(knowledge);
  if (model === null) {
    return null;
  }
  return readInteractive(knowledge, model)?.partsModel.assetId ?? model.assetId;
}

export function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

/** JSON 内嵌进 `<script>`：转义 `<`、U+2028/U+2029，避免截断与解析差异。 */
export function serializeForScript(value: unknown): string {
  return JSON.stringify(value)
    .replaceAll("<", "\\u003c")
    .replaceAll("\u2028", "\\u2028")
    .replaceAll("\u2029", "\\u2029");
}

export function bytesToBase64(bytes: Uint8Array): string {
  const chunkSize = 0x8000;
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return btoa(binary);
}

/** 脚本文本内嵌：只需防住 `</script`（大小写不敏感）。 */
function escapeScriptText(code: string): string {
  return code.replace(/<\/(script)/gi, "<\\/$1");
}

export function buildStandaloneHtml(
  payload: StandalonePayload,
  modelBytes: Uint8Array,
  viewerScript: string,
): string {
  const title = escapeHtml(`${payload.title} · 交互式说明书`);
  return `<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src blob: data:; connect-src blob: data:; worker-src blob:" />
<title>${title}</title>
<style>${STANDALONE_STYLES}</style>
</head>
<body>
<header class="top">
  <div>
    <p class="eyebrow">THE INTERACTIVE MANUAL</p>
    <h1 id="em-title"></h1>
    <p id="em-subtitle" class="subtitle"></p>
    <details class="version-details">
      <summary>版本信息</summary>
      <p id="em-release"></p>
    </details>
  </div>
  <p class="meta">离线版 · 由万物说明书导出</p>
</header>
<main class="layout">
  <section class="panel" aria-labelledby="em-parts-heading">
    <h2 id="em-parts-heading">部件</h2>
    <ul id="em-parts" class="parts"></ul>
  </section>
  <section class="stage" aria-label="3D 模型">
    <div class="stage__bar"><button id="em-reset" type="button">复位视角</button></div>
    <div id="em-canvas" class="stage__canvas"></div>
    <div id="em-interactions" class="interactions"></div>
    <p id="em-status" class="status" role="status">正在加载 3D 模型…</p>
  </section>
  <section class="panel" aria-labelledby="em-steps-heading">
    <h2 id="em-steps-heading">步骤</h2>
    <div class="stepper">
      <button id="em-step-prev" type="button">上一步</button>
      <span id="em-step-counter"></span>
      <button id="em-step-next" type="button">下一步</button>
    </div>
    <div id="em-step" class="step" aria-live="polite"></div>
    <section id="em-specs-section">
      <h2>规格</h2>
      <dl id="em-specs" class="specs"></dl>
    </section>
  </section>
</main>
<script id="${STANDALONE_PAYLOAD_ELEMENT_ID}" type="application/json">${serializeForScript(payload)}</script>
<script id="${STANDALONE_MODEL_ELEMENT_ID}" type="application/octet-stream">${bytesToBase64(modelBytes)}</script>
<script>${escapeScriptText(viewerScript)}</script>
</body>
</html>
`;
}

/** 安全的下载文件名（去掉路径与控制字符）。 */
export function standaloneFileName(item: StandaloneItemInfo): string {
  const base = [item.name, item.model]
    .filter((value): value is string => value !== null && value !== "")
    .join("-")
    .replace(/[\\/:*?"<>|]+/g, "_")
    .replace(/\p{Cc}+/gu, "")
    .trim();
  return `${base === "" ? "manual" : base}-3d.html`;
}
