/**
 * QA 回合 24 · T19 独立验收（QA 自证；AC-052/AC-053/AC-054/AC-055/AC-056/AC-057 与卡内边界）。
 *
 * 与 RD 用例（`manual-review.spec.ts`）保持独立：
 * - **自带 fixture**：模型 CDN 的字节可在用例中途切换（两轮真实流水线产出不同 sha 的
 *   新模型），用于 AC-053「重新生成模型 → 旧绑定 stale、旧发布版仍指向旧模型」的真实
 *   链路复现（RD 用例不具备该能力，Rust 侧的同名断言不构成本文件的证据）；
 * - **自带观察口径**：真实 HTTP 合同 + 真实资产字节哈希（sha256 现场计算）+
 *   阅读器只读桥（`window.__EM_VIEWER__`）；不复用 RD 的断言、截图与数据。
 * - **零真实外网**：浏览器路由只做源站改写（Vite 端口 → 本机测试后端）+ 外部请求阻断；
 *   `route.fulfill` 正常响应计数必须恒为 0（守卫 + 断言）。
 * - 全部造数经公开 HTTP 合同；后端为**测试构建**（`--features job-failpoints`）+
 *   显式配置才放行"明文 http + 回环"的模型下载；全链路零付费、零外网。
 *
 * 覆盖矩阵（命令 ↔ AC）：
 * - QA19-1：AC-052（有限数值、人工直接拾取 confirmed、unbound 不占位由 QA19-5 覆盖、
 *   拖动不建点、raycast 不命中热点标记本身）；AC-057（1-based 跳页一致性）；
 * - QA19-2：AC-053（真实两轮流水线 stale、旧 sha 被拒、旧发布版字节可读、换模型清空
 *   modelReview、UI 重新绑定）；
 * - QA19-3：AC-055/AC-056（发布不变量阶梯 428/422/412/409、幂等重放同一 release、
 *   发布后修改 draft 不改变 release 字节与哈希、无自动发布）；
 * - QA19-4：卡内边界（窄屏 <768px 禁用几何校准/视角保存并解释、文字确认与发布保留、
 *   §6.3.2 禁用措辞、无自动发布入口、键盘焦点）；
 * - QA19-5：AC-054 + AC-052 的 API 侧（热点状态机全量、供应商快照只读、userEdited
 *   保留出处、modelReview 服务器赋值与客户端不可自证、evidence 1-based 且 bbox=null）。
 */

import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import path from "node:path";

import {
  expect,
  request as playwrightRequest,
  test,
  type APIRequestContext,
  type Page,
  type Route,
} from "@playwright/test";

import {
  apiLogin,
  fetchAsset,
  fetchPhotos,
  loginViaUi,
  seedItemWithDocument,
  seedPhoto,
  seedReadyPreparation,
} from "./helpers";
import {
  BACKEND_PASSWORD,
  JOB_LIMITS,
  MODEL_PRESET,
  TestBackend,
  buildTestServerBinary,
  type LocalFixture,
} from "./job-recovery-harness";
import { E2E_WEB_PORT, REPO_ROOT, fixturePath } from "./runtime";

test.describe.configure({ timeout: 300_000 });

const WEB_BASE = `http://127.0.0.1:${E2E_WEB_PORT}`;
const FIXTURE_TASK_ID = "qa19-fixture-task-0001";
const PAGE_TEXT = "第 1 页：QA19 样例页文字（后盖与电池）。";
const QA_DIR = path.join(REPO_ROOT, "artifacts", "web-mvp", "t19-qa");

function sha256Hex(bytes: Buffer): string {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        reject(new Error("无法取得空闲端口"));
        return;
      }
      const port = address.port;
      server.close(() => resolve(port));
    });
  });
}

/** 第二个模型版本：翻转 BIN 起始的一个顶点浮点高位字节（结构不变、字节不同）。 */
function variantModelBytes(): Buffer {
  const bytes = Buffer.from(fs.readFileSync(fixturePath("sample-model.glb")));
  const marker = bytes.indexOf(Buffer.from("BIN\0", "latin1"));
  expect(marker, "GLB 必须包含 BIN chunk").toBeGreaterThan(0);
  const dataStart = marker + 4;
  bytes[dataStart + 3] = (bytes[dataStart + 3] ?? 0) ^ 0x01;
  return bytes;
}

/** 说明书 AI 的结构化结果（引用第 1 页；两个部件用于部件↔热点联动）。 */
function qaKnowledge(): unknown {
  return {
    schemaVersion: "manual_extract_v1",
    parts: [
      {
        id: "p-1",
        name: "后盖",
        description: "固定机身背部的盖板",
        evidence: [{ pageNumber: 1, quote: PAGE_TEXT }],
      },
      {
        id: "p-2",
        name: "电池",
        description: "可拆卸电池仓",
        evidence: [{ pageNumber: 1, quote: null }],
      },
    ],
    steps: [
      {
        id: "s-1",
        title: "取下后盖",
        orderedActions: ["松开固定件", "取下后盖"],
        partIds: ["p-1"],
        evidence: [{ pageNumber: 1, quote: PAGE_TEXT }],
        safetyNotes: ["操作前断电。"],
      },
      {
        id: "s-2",
        title: "取出电池",
        orderedActions: ["断开连接器", "取出电池"],
        partIds: ["p-2"],
        evidence: [{ pageNumber: 1, quote: PAGE_TEXT }],
        safetyNotes: ["避免金属工具短路。"],
      },
    ],
    specs: [{ id: "sp-1", label: "供电", value: "DC 12 V / 2.5 A", evidence: [{ pageNumber: 1, quote: null }] }],
    uncertainties: [],
  };
}

/**
 * QA 自有 fixture：Tripo v3 + 说明书 AI + **可切换字节**的模型 CDN。
 * 缺脚本路由返回 501（不返回通用成功，与 T05 约定一致）。
 */
class QaFixture {
  readonly counts = { upload: 0, submit: 0, task: 0, manual: 0, cdn: 0 };
  /** 模型 CDN 当前提供的字节（用例中途可切换以模拟"重新生成"）。 */
  modelBytes: Buffer = fs.readFileSync(fixturePath("sample-model.glb"));
  private server: http.Server | null = null;
  port = 0;

  get base(): string {
    return `http://127.0.0.1:${this.port}`;
  }

  async start(): Promise<void> {
    this.port = await freePort();
    this.server = http.createServer((request, response) => {
      void this.handle(request, response);
    });
    await new Promise<void>((resolve) => this.server?.listen(this.port, "127.0.0.1", resolve));
  }

  async stop(): Promise<void> {
    if (this.server === null) {
      return;
    }
    await new Promise<void>((resolve) => this.server?.close(() => resolve()));
    this.server = null;
  }

  private json(response: http.ServerResponse, status: number, body: unknown): void {
    const payload = JSON.stringify(body);
    response.writeHead(status, {
      "content-type": "application/json",
      "content-length": Buffer.byteLength(payload),
    });
    response.end(payload);
  }

  private async handle(request: http.IncomingMessage, response: http.ServerResponse): Promise<void> {
    const url = new URL(request.url ?? "/", `http://127.0.0.1:${this.port}`);
    for await (const _chunk of request) {
      // 消费请求体（fixture 不解析上传内容，与 T17 设施一致）。
    }
    const method = request.method ?? "GET";
    if (method === "POST" && url.pathname === "/v3/files") {
      this.counts.upload += 1;
      this.json(response, 200, { code: 0, data: { file_token: `qa19-token-${this.counts.upload}` } });
      return;
    }
    if (method === "POST" && url.pathname === "/v3/generation/multiview-to-model") {
      this.counts.submit += 1;
      this.json(response, 200, { code: 0, data: { task_id: FIXTURE_TASK_ID } });
      return;
    }
    if (method === "GET" && url.pathname.startsWith("/v3/tasks/")) {
      this.counts.task += 1;
      this.json(response, 200, {
        code: 0,
        data: {
          task_id: FIXTURE_TASK_ID,
          status: "success",
          progress: 100,
          credits_consumed: 30,
          output: {
            model_url: `${this.base}/cdn/model.glb`,
            rendered_image_url: "https://cdn.example.invalid/preview.png",
          },
        },
      });
      return;
    }
    if (method === "GET" && url.pathname === "/cdn/model.glb") {
      this.counts.cdn += 1;
      const body = this.modelBytes;
      response.writeHead(200, { "content-type": "model/gltf-binary", "content-length": body.length });
      response.end(body);
      return;
    }
    if (method === "POST" && url.pathname === "/v1/responses") {
      this.counts.manual += 1;
      this.json(response, 200, {
        id: `resp_qa19_${this.counts.manual}`,
        object: "response",
        status: "completed",
        model: "gpt-5-mini",
        output: [
          {
            type: "message",
            role: "assistant",
            content: [{ type: "output_text", annotations: [], text: JSON.stringify(qaKnowledge()) }],
          },
        ],
        usage: { input_tokens: 1000, output_tokens: 200, total_tokens: 1200 },
      });
      return;
    }
    this.json(response, 501, { error: { message: `fixture has no route for ${method} ${url.pathname}` } });
  }
}

// ---------------------------------------------------------------------------
// 数据类型（只声明本文件用到的字段）
// ---------------------------------------------------------------------------

interface DraftModelInfo {
  revisionId: string;
  sha256: string;
  assetId: string;
  validationState: string;
}
interface AnchorView {
  modelRevisionId: string;
  modelSha256: string;
  positionLocal: number[];
}
interface HotspotView {
  id: string;
  partId: string;
  status: string;
  anchor: AnchorView | null;
}
interface EvidenceView {
  pageNumber: number;
  quote: string | null;
  bbox?: number[] | null;
  derived?: boolean;
}
interface PartView {
  id: string;
  name: string;
  evidence: EvidenceView[];
}
interface StepView {
  id: string;
  title: string;
  partIds: string[];
  evidence: EvidenceView[];
}
interface SpecView {
  id: string;
  label: string;
  value: string;
  evidence: EvidenceView[];
}
interface EntityReviewView {
  reviewStatus?: string | null;
  userEdited?: Record<string, unknown> | null;
  textOnly?: boolean;
  editedAt?: number | null;
  editedBy?: string | null;
}
interface ModelReviewView {
  loaded?: boolean;
  userConfirmed?: boolean;
  checkedAt?: number | null;
  loadedAt?: number | null;
  userConfirmedAt?: number | null;
  modelRevisionId?: string;
  modelSha256?: string;
}
interface DraftData {
  id: string;
  revision: number;
  status: string;
  knowledge: {
    model?: DraftModelInfo | null;
    knowledge?: { parts?: PartView[]; steps?: StepView[]; specs?: SpecView[] } | null;
    hotspots?: HotspotView[];
    missing?: unknown[];
  };
  review?: {
    entities?: Record<string, EntityReviewView>;
    modelReview?: ModelReviewView | null;
  } | null;
}
interface DraftView {
  etag: string;
  data: DraftData;
}
interface ReleaseDetail {
  id: string;
  draftId: string;
  draftRevision: number;
  modelRevisionId: string;
  manifestAssetId: string;
  manifestSha256: string;
  manifest: { model?: { assetId?: string; sha256?: string } } & Record<string, unknown>;
}

interface Api {
  context: APIRequestContext;
  csrf: string;
}

interface CallResult<T> {
  status: number;
  json: T;
  etag: string | null;
  headers: Record<string, string>;
}

async function apiCall<T = unknown>(
  api: Api,
  options: {
    method: "GET" | "POST" | "PATCH";
    path: string;
    data?: unknown;
    ifMatch?: string | null;
    idempotencyKey?: string | null;
  },
): Promise<CallResult<T>> {
  const headers: Record<string, string> = { "x-csrf-token": api.csrf };
  if (options.ifMatch !== undefined && options.ifMatch !== null) {
    headers["if-match"] = options.ifMatch;
  }
  if (options.idempotencyKey !== undefined && options.idempotencyKey !== null) {
    headers["idempotency-key"] = options.idempotencyKey;
  }
  const response = await api.context.fetch(`${backend.base}${options.path}`, {
    method: options.method,
    headers,
    data: options.data as never,
  });
  const text = await response.text();
  return {
    status: response.status(),
    json: (text === "" ? null : JSON.parse(text)) as T,
    etag: response.headers()["etag"] ?? null,
    headers: response.headers(),
  };
}

async function freshApi(): Promise<Api> {
  const context = await playwrightRequest.newContext();
  const csrf = await apiLogin(context, backend.base, BACKEND_PASSWORD);
  return { context, csrf };
}

async function getDraft(api: Api, itemId: string, draftId: string): Promise<DraftView> {
  const result = await apiCall<{ data: DraftData }>(api, {
    method: "GET",
    path: `/api/v1/items/${itemId}/drafts/${draftId}`,
  });
  expect(result.status, JSON.stringify(result.json)).toBe(200);
  expect(result.etag, "草稿必须带 ETag").not.toBeNull();
  return { etag: result.etag ?? "", data: result.json.data };
}

async function patchDraft(
  api: Api,
  itemId: string,
  draftId: string,
  etag: string,
  body: unknown,
): Promise<CallResult<{ data?: DraftData; error?: unknown }>> {
  return apiCall<{ data?: DraftData; error?: unknown }>(api, {
    method: "PATCH",
    path: `/api/v1/items/${itemId}/drafts/${draftId}`,
    data: body,
    ifMatch: etag,
  });
}

async function patchDraftOk(
  api: Api,
  itemId: string,
  draftId: string,
  view: DraftView,
  body: unknown,
): Promise<DraftView> {
  const result = await patchDraft(api, itemId, draftId, view.etag, body);
  expect(result.status, JSON.stringify(result.json)).toBe(200);
  return { etag: result.etag ?? "", data: (result.json.data ?? null) as DraftData };
}

async function publishDraft(
  api: Api,
  itemId: string,
  draftId: string,
  options: { ifMatch?: string | null; idempotencyKey?: string | null },
): Promise<CallResult<{ data?: { id: string; draftRevision: number; draftRevisionAfterPublish?: number | null }; error?: { code?: string; details?: unknown } }>> {
  return apiCall(api, {
    method: "POST",
    path: `/api/v1/items/${itemId}/drafts/${draftId}/publish`,
    ifMatch: options.ifMatch ?? null,
    idempotencyKey: options.idempotencyKey ?? null,
  });
}

async function releaseDetail(api: Api, itemId: string, releaseId: string): Promise<ReleaseDetail> {
  const result = await apiCall<{ data: ReleaseDetail }>(api, {
    method: "GET",
    path: `/api/v1/items/${itemId}/releases/${releaseId}`,
  });
  expect(result.status, JSON.stringify(result.json)).toBe(200);
  return result.json.data;
}

async function listReleases(api: Api, itemId: string): Promise<{ id: string }[]> {
  const result = await apiCall<{ data: { id: string }[] }>(api, {
    method: "GET",
    path: `/api/v1/items/${itemId}/releases`,
  });
  expect(result.status, JSON.stringify(result.json)).toBe(200);
  return result.json.data;
}

// ---------------------------------------------------------------------------
// 造数与真实流水线
// ---------------------------------------------------------------------------

interface ReadyInputs {
  itemId: string;
  documentId: string;
  preparationId: string;
  photoIds: string[];
}

async function seedReadyItem(api: Api, name: string): Promise<ReadyInputs> {
  const seed = await seedItemWithDocument(
    api.context,
    backend.base,
    BACKEND_PASSWORD,
    "sample-manual-text.pdf",
    name,
  );
  // `seedItemWithDocument` 内部重新登录（换 cookie）：CSRF 与会话绑定，必须重取。
  api.csrf = await apiLogin(api.context, backend.base, BACKEND_PASSWORD);
  const preparationId = await seedReadyPreparation(api.context, backend.base, api.csrf, seed);
  await seedPhoto(api.context, backend.base, api.csrf, seed.itemId, "front", "sample-photo-front.jpg");
  await seedPhoto(api.context, backend.base, api.csrf, seed.itemId, "left", "sample-photo-left.png");
  // 照片登记后 cookie/CSRF 不变；照片列表用服务端事实。
  const photos = await fetchPhotos(api.context, backend.base, seed.itemId);
  return {
    itemId: seed.itemId,
    documentId: seed.documentId,
    preparationId,
    photoIds: photos.map((photo) => photo.id),
  };
}

async function createJobOnItem(
  api: Api,
  inputs: ReadyInputs,
  key: string,
): Promise<string> {
  const estimate = await apiCall<{ data: { id: string } }>(api, {
    method: "POST",
    path: `/api/v1/items/${inputs.itemId}/estimates`,
    data: { preparationId: inputs.preparationId, photoIds: inputs.photoIds, modelPreset: MODEL_PRESET },
  });
  expect(estimate.status, JSON.stringify(estimate.json)).toBe(201);
  const quoteId = estimate.json.data.id;
  const confirmed = await apiCall(api, {
    method: "POST",
    path: `/api/v1/items/${inputs.itemId}/estimates/${quoteId}/confirm`,
  });
  expect(confirmed.status, JSON.stringify(confirmed.json)).toBe(200);
  const job = await apiCall<{ data: { id: string } }>(api, {
    method: "POST",
    path: `/api/v1/items/${inputs.itemId}/jobs`,
    data: {
      quoteId,
      preparationId: inputs.preparationId,
      photoIds: inputs.photoIds,
      limits: JOB_LIMITS,
    },
    idempotencyKey: key,
  });
  expect(job.status, JSON.stringify(job.json)).toBe(202);
  return job.json.data.id;
}

/** 轮询任务直到草稿产出（真实执行器在服务进程内推进）。 */
async function waitForDraft(
  api: Api,
  jobId: string,
): Promise<{ draftId: string; jobStatus: string }> {
  const deadline = Date.now() + 180_000;
  let last = "";
  while (Date.now() < deadline) {
    const job = await apiCall<{ data: { status: string; draftId: string | null } }>(api, {
      method: "GET",
      path: `/api/v1/jobs/${jobId}`,
    });
    expect(job.status, JSON.stringify(job.json)).toBe(200);
    last = job.json.data.status;
    if ((last === "succeeded" || last === "needs_input") && job.json.data.draftId !== null) {
      return { draftId: job.json.data.draftId, jobStatus: last };
    }
    if (last === "failed" || last === "cancelled") {
      throw new Error(`任务终态为 ${last}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`等待草稿超时（最后状态 ${last}）`);
}

function modelIdentity(knowledge: DraftData["knowledge"]): DraftModelInfo {
  const model = knowledge.model ?? null;
  expect(model, "草稿必须有模型分支产物").not.toBeNull();
  const info = model as DraftModelInfo;
  expect(info.validationState, "模型必须 validated").toBe("validated");
  return info;
}

function partsOf(draft: DraftData): PartView[] {
  return draft.knowledge.knowledge?.parts ?? [];
}

function hotspotsOf(draft: DraftData): HotspotView[] {
  return draft.knowledge.hotspots ?? [];
}

/** 通过公开合同把草稿推到"除 modelReview 外"全部满足（供发布阶梯用例）。 */
async function makeEntitiesAndHotspotsReady(
  api: Api,
  itemId: string,
  draftId: string,
): Promise<DraftView> {
  let view = await getDraft(api, itemId, draftId);
  const model = modelIdentity(view.data.knowledge);
  const merged = view.data.knowledge.knowledge ?? {};
  const entities: Record<string, unknown> = {};
  for (const part of merged.parts ?? []) {
    entities[part.id] = { reviewStatus: "confirmed" };
  }
  for (const step of merged.steps ?? []) {
    entities[step.id] = { reviewStatus: "confirmed" };
  }
  for (const spec of merged.specs ?? []) {
    entities[spec.id] = { reviewStatus: "confirmed" };
  }
  view = await patchDraftOk(api, itemId, draftId, view, { entities });
  const upsert = (merged.parts ?? []).map((part, index) => ({
    partId: part.id,
    status: "confirmed",
    anchor: {
      modelRevisionId: model.revisionId,
      modelSha256: model.sha256,
      positionLocal: [0.4 - index * 0.2, 0.3, 1.0],
    },
  }));
  return patchDraftOk(api, itemId, draftId, view, { hotspots: { upsert } });
}

// ---------------------------------------------------------------------------
// 浏览器：真实链路路由与阅读器观察
// ---------------------------------------------------------------------------

interface Routing {
  external: string[];
  assetContent: string[];
  publishRequests: string[];
  fulfilledOk: number;
}

async function installRealRouting(page: Page): Promise<Routing> {
  const routing: Routing = { external: [], assetContent: [], publishRequests: [], fulfilledOk: 0 };
  await page.route("**/*", async (rawRoute: Route) => {
    const route: Route = Object.create(rawRoute, {
      fulfill: {
        value: async (...args: Parameters<Route["fulfill"]>) => {
          routing.fulfilledOk += 1;
          return rawRoute.fulfill(...args);
        },
      },
    });
    const request = route.request();
    const url = new URL(request.url());
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      await route.continue();
      return;
    }
    if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") {
      routing.external.push(url.href);
      await route.abort();
      return;
    }
    if (url.pathname.startsWith("/api/v1/assets/")) {
      routing.assetContent.push(url.pathname);
    }
    if (url.pathname.endsWith("/publish")) {
      routing.publishRequests.push(url.pathname);
    }
    let target = request.url();
    if (url.href.startsWith(`${WEB_BASE}/api/v1`)) {
      target = `${backend.base}${url.pathname}${url.search}`;
    }
    await route.continue({ url: target });
  });
  return routing;
}

async function openReview(page: Page, itemId: string, draftId: string): Promise<Routing> {
  const routing = await installRealRouting(page);
  await page.goto(`${WEB_BASE}/`);
  await loginViaUi(page, WEB_BASE, BACKEND_PASSWORD);
  await page.goto(`${WEB_BASE}/items/${itemId}/drafts/${draftId}/review`);
  await waitForViewer(page);
  return routing;
}

async function waitForViewer(page: Page): Promise<void> {
  await expect(page.getByTestId("viewer-canvas")).toBeVisible();
  await expect
    .poll(async () => page.evaluate(() => window.__EM_VIEWER__?.stats().modelsAlive ?? 0), {
      timeout: 60_000,
    })
    .toBe(1);
  await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
}

async function projectLocal(
  page: Page,
  local: readonly [number, number, number],
): Promise<{ screen: [number, number]; visible: boolean }> {
  const projection = await page.evaluate(
    (point) => window.__EM_VIEWER__?.project(point as [number, number, number]) ?? null,
    [local[0], local[1], local[2]],
  );
  expect(projection, "桥必须提供 project()").not.toBeNull();
  return { screen: projection?.screen ?? [0, 0], visible: projection?.visible ?? false };
}

async function viewerBounds(
  page: Page,
): Promise<{ min: readonly number[]; max: readonly number[] } | null> {
  return page.evaluate(() => {
    const bounds = window.__EM_VIEWER__?.localBounds() ?? null;
    return bounds === null ? null : { min: [...bounds.min], max: [...bounds.max] };
  });
}

async function canvasBox(page: Page): Promise<{ x: number; y: number; width: number; height: number }> {
  const box = await page.getByTestId("viewer-canvas").boundingBox();
  expect(box, "3D 视口必须可见").not.toBeNull();
  return box as { x: number; y: number; width: number; height: number };
}

/**
 * 画布相对像素坐标 → 视口绝对坐标。
 * 断言过的元素可能把页面滚到别处（例如 stale 区块在左栏底部），画布会滚出视口；
 * 直接用旧 boundingBox 点击会落到页面其它位置。这里先滚动再换算。
 */
async function canvasViewportPoint(page: Page, screen: [number, number]): Promise<[number, number]> {
  const canvas = page.getByTestId("viewer-canvas");
  await canvas.scrollIntoViewIfNeeded();
  let box = await canvasBox(page);
  const viewport = page.viewportSize() ?? { width: 1280, height: 720 };
  const absolute: [number, number] = [box.x + screen[0], box.y + screen[1]];
  if (
    absolute[0] < 2 ||
    absolute[1] < 2 ||
    absolute[0] > viewport.width - 2 ||
    absolute[1] > viewport.height - 2
  ) {
    await page.evaluate((delta) => {
      window.scrollTo(0, Math.max(0, window.scrollY + delta));
    }, absolute[1] - viewport.height / 2);
    await page.waitForTimeout(120);
    box = await canvasBox(page);
    absolute[0] = box.x + screen[0];
    absolute[1] = box.y + screen[1];
  }
  expect(
    absolute[0] >= 2 && absolute[1] >= 2 && absolute[0] <= viewport.width - 2 && absolute[1] <= viewport.height - 2,
    `画布点击点必须在视口内：${JSON.stringify(absolute)}（viewport ${viewport.width}x${viewport.height}）`,
  ).toBe(true);
  return absolute;
}

async function clickCanvasAt(page: Page, screen: [number, number]): Promise<void> {
  const point = await canvasViewportPoint(page, screen);
  await page.mouse.move(point[0], point[1]);
  await page.mouse.down();
  await page.mouse.up();
}

async function dragCanvas(page: Page, dx: number, dy: number): Promise<void> {
  const box = await canvasBox(page);
  const center = await canvasViewportPoint(page, [box.width / 2, box.height / 2]);
  await page.mouse.move(center[0], center[1]);
  await page.mouse.down();
  await page.mouse.move(center[0] + dx, center[1] + dy, { steps: 12 });
  await page.mouse.up();
}

async function lastPickLocal(page: Page): Promise<number[] | null> {
  return page.evaluate(() => {
    const pick = window.__EM_VIEWER__?.lastPick() ?? null;
    return pick === null ? null : [...pick.local];
  });
}

async function viewerPose(
  page: Page,
): Promise<{ positionLocal: number[]; targetLocal: number[] } | null> {
  return page.evaluate(() => {
    const pose = window.__EM_VIEWER__?.cameraPose() ?? null;
    return pose === null
      ? null
      : { positionLocal: [...pose.positionLocal], targetLocal: [...pose.targetLocal] };
  });
}

async function viewerAnchors(
  page: Page,
): Promise<{ id: string; partId: string; local: number[] }[]> {
  return page.evaluate(() =>
    (window.__EM_VIEWER__?.anchors() ?? []).map((anchor) => ({
      id: anchor.id,
      partId: anchor.partId,
      local: [...anchor.local],
    })),
  );
}

/** 热点标记（局部半径 0.035）在屏幕上的近似半径。 */
async function markerScreenRadius(page: Page, local: readonly [number, number, number]): Promise<number> {
  const center = (await projectLocal(page, local)).screen;
  const r = 0.035;
  let best = 0;
  for (const delta of [
    [r, 0, 0],
    [0, r, 0],
    [0, 0, r],
  ] as const) {
    const point = (await projectLocal(page, [local[0] + delta[0], local[1] + delta[1], local[2] + delta[2]]))
      .screen;
    best = Math.max(best, Math.hypot(point[0] - center[0], point[1] - center[1]));
  }
  return best;
}

function writeEvidence(name: string, value: unknown): void {
  fs.mkdirSync(QA_DIR, { recursive: true });
  fs.writeFileSync(path.join(QA_DIR, name), `${JSON.stringify(value, null, 2)}\n`);
}

function shot(page: Page, name: string): Promise<Buffer> {
  const dir = path.join(QA_DIR, "screenshots");
  fs.mkdirSync(dir, { recursive: true });
  return page.screenshot({ path: path.join(dir, `${name}.png`) });
}

// ---------------------------------------------------------------------------

let fixture: QaFixture;
let backend: TestBackend;

test.beforeAll(async () => {
  test.setTimeout(900_000);
  buildTestServerBinary();
  fixture = new QaFixture();
  await fixture.start();
  backend = new TestBackend("t19qa");
  await backend.start(fixture as unknown as LocalFixture);
});

test.afterAll(async () => {
  await backend?.cleanup(fixture as unknown as LocalFixture);
});

// ---------------------------------------------------------------------------
// QA19-1 · AC-052：真实浏览器拾取绑定、拖动不建点、raycast 不命中热点标记
// ---------------------------------------------------------------------------

test("QA19-1 拾取绑定：有限数值 anchor、拖动不建点、raycast 只命中模型 mesh（AC-052）", async ({
  page,
}) => {
  const api = await freshApi();
  const inputs = await seedReadyItem(api, "QA19 拾取物品");
  const jobId = await createJobOnItem(api, inputs, "qa19-pick-job");
  const { draftId, jobStatus } = await waitForDraft(api, jobId);
  expect(jobStatus, "fixture 全链路必须成功").toBe("succeeded");

  const draft = await getDraft(api, inputs.itemId, draftId);
  const model = modelIdentity(draft.data.knowledge);
  const parts = partsOf(draft.data);
  expect(parts.length, "fixture 必须产出 ≥2 个部件").toBeGreaterThanOrEqual(2);
  // 真实链路锚定：草稿模型 sha256 == QA fixture 现场提供的真实字节哈希。
  const bytesA = fs.readFileSync(fixturePath("sample-model.glb"));
  expect(model.sha256).toBe(sha256Hex(bytesA));

  const routing = await openReview(page, inputs.itemId, draftId);
  expect(routing.fulfilledOk, "真实链路用例不得伪造正常响应").toBe(0);
  expect(routing.publishRequests, "页面加载不得自动发布").toEqual([]);

  const partA = parts[0] as PartView;
  const partB = parts[1] as PartView;
  await expect(page.getByTestId(`part-hotspot-${partA.id}`)).toContainText("未绑定");
  await expect(page.getByTestId(`part-hotspot-${partB.id}`)).toContainText("未绑定");

  // 拖动旋转（在拾取模式之外）不得建点。
  await dragCanvas(page, 130, 80);
  await expect
    .poll(async () => hotspotsOf((await getDraft(api, inputs.itemId, draftId)).data).length, {
      timeout: 10_000,
    })
    .toBe(0);

  // 进入 partA 的拾取绑定；拖动仍是旋转（不建点、不丢模式）。
  await page.getByTestId(`part-row-${partA.id}`).getByRole("button", { name: "绑定热点" }).click();
  await expect(page.getByTestId("pick-mode-active")).toBeVisible();
  const poseBefore = await viewerPose(page);
  await dragCanvas(page, 120, 70);
  await page.waitForTimeout(300);
  expect(hotspotsOf((await getDraft(api, inputs.itemId, draftId)).data).length, "拖动不得建点").toBe(0);
  const poseAfter = await viewerPose(page);
  const moved =
    Math.abs((poseAfter?.positionLocal[0] ?? 0) - (poseBefore?.positionLocal[0] ?? 0)) +
    Math.abs((poseAfter?.positionLocal[1] ?? 0) - (poseBefore?.positionLocal[1] ?? 0)) +
    Math.abs((poseAfter?.positionLocal[2] ?? 0) - (poseBefore?.positionLocal[2] ?? 0));
  expect(moved, "拖动必须真的旋转相机").toBeGreaterThan(0);
  await expect(page.getByTestId("pick-mode-active")).toBeVisible();

  // 拾取：点向"包围盒中心"的投影（射线命中朝向相机的表面）。
  const bounds = await viewerBounds(page);
  expect(bounds, "桥必须提供 localBounds()").not.toBeNull();
  const centerLocal: [number, number, number] = [0, 1, 2].map(
    (axis) => ((bounds?.min[axis] ?? 0) + (bounds?.max[axis] ?? 0)) / 2,
  ) as [number, number, number];
  const centerProjection = await projectLocal(page, centerLocal);
  expect(centerProjection.visible, "模型中心必须在视口内").toBe(true);
  await clickCanvasAt(page, centerProjection.screen);
  let picked: number[] | null = null;
  await expect
    .poll(
      async () => {
        picked = await lastPickLocal(page);
        return picked !== null;
      },
      { timeout: 15_000 },
    )
    .toBe(true);
  for (const value of picked ?? []) {
    expect(Number.isFinite(value), `拾取点必须是有限数值：${JSON.stringify(picked)}`).toBe(true);
  }
  // 独立一致性：拾取点投影回屏幕必须落在点击像素上（≤2px）。
  const reprojection = await projectLocal(page, [
    (picked ?? [0])[0] ?? 0,
    (picked ?? [0, 0])[1] ?? 0,
    (picked ?? [0, 0, 0])[2] ?? 0,
  ]);
  expect(Math.abs(reprojection.screen[0] - centerProjection.screen[0])).toBeLessThan(2);
  expect(Math.abs(reprojection.screen[1] - centerProjection.screen[1])).toBeLessThan(2);

  // 落库：confirmed + 身份匹配 + 与拾取点逐轴一致（人工直接拾取 unbound → confirmed）。
  await expect
    .poll(async () => hotspotsOf((await getDraft(api, inputs.itemId, draftId)).data).length, {
      timeout: 15_000,
    })
    .toBe(1);
  const first = hotspotsOf((await getDraft(api, inputs.itemId, draftId)).data)[0] as HotspotView;
  expect(first.status).toBe("confirmed");
  expect(first.partId).toBe(partA.id);
  expect(first.anchor?.modelRevisionId).toBe(model.revisionId);
  expect(first.anchor?.modelSha256).toBe(model.sha256);
  const anchorLocal = first.anchor?.positionLocal ?? [];
  expect(anchorLocal).toHaveLength(3);
  for (let axis = 0; axis < 3; axis += 1) {
    expect(
      Math.abs((anchorLocal[axis] ?? Number.NaN) - ((picked ?? [])[axis] ?? Number.NaN)),
      `落库 anchor 必须等于拾取点（轴 ${axis}）`,
    ).toBeLessThan(1e-9);
  }
  await expect(page.getByTestId(`part-hotspot-${partA.id}`)).toContainText("热点已确认");

  // ---- raycast 只命中模型 mesh（不命中热点标记自身） --------------------------
  // 放大到标记在屏幕上足够大；然后在**标记球体覆盖范围内、但超出点选半径（18px）**
  // 的位置点击 partB 的绑定：若 raycast 把标记球体算进去，命中点会落在表面之外
  // （球体半径 0.035 局部单位，最大外向偏移 ~0.03）；正确实现应命中模型表面。
  const anchorPoint: [number, number, number] = [
    anchorLocal[0] ?? 0,
    anchorLocal[1] ?? 0,
    anchorLocal[2] ?? 0,
  ];
  let markerRadius = await markerScreenRadius(page, anchorPoint);
  const radiusTrace: number[] = [markerRadius];
  const wheelPoint = await canvasViewportPoint(page, [ (await canvasBox(page)).width / 2, (await canvasBox(page)).height / 2 ]);
  await page.mouse.move(wheelPoint[0], wheelPoint[1]);
  for (let attempt = 0; attempt < 20 && markerRadius < 45; attempt += 1) {
    await page.mouse.wheel(0, -500);
    await page.waitForTimeout(120);
    markerRadius = await markerScreenRadius(page, anchorPoint);
    radiusTrace.push(markerRadius);
  }
  expect(
    markerRadius,
    `放大后标记屏幕半径必须 > 45px（测试前置条件）；实测轨迹 ${JSON.stringify(radiusTrace)}`,
  ).toBeGreaterThan(45);
  await page.getByTestId(`part-row-${partB.id}`).getByRole("button", { name: "绑定热点" }).click();
  await expect(page.getByTestId("pick-mode-active")).toBeVisible();
  // 长按（原地 >600ms）：只算旋转、不建点，且不退出拾取模式（UI-048 的时长阈值）。
  const pressPoint = await canvasViewportPoint(page, centerProjection.screen);
  await page.mouse.move(pressPoint[0], pressPoint[1]);
  await page.mouse.down();
  await page.waitForTimeout(750);
  await page.mouse.up();
  await page.waitForTimeout(300);
  expect(
    hotspotsOf((await getDraft(api, inputs.itemId, draftId)).data).length,
    "长按（>600ms）不得建点",
  ).toBe(1);
  await expect(page.getByTestId("pick-mode-active")).toBeVisible();
  const markerCenter = (await projectLocal(page, [
    anchorLocal[0] ?? 0,
    anchorLocal[1] ?? 0,
    anchorLocal[2] ?? 0,
  ])).screen;
  const offsetPx = Math.max(24, Math.round(markerRadius * 0.5));
  expect(offsetPx, "点击点必须在标记覆盖内（< 0.9R）且超出 18px 点选半径").toBeLessThan(
    markerRadius * 0.9,
  );
  await clickCanvasAt(page, [markerCenter[0] + offsetPx, markerCenter[1]]);
  let pick2: number[] | null = null;
  await expect
    .poll(
      async () => {
        pick2 = await lastPickLocal(page);
        return pick2 !== null && pick2.some((value, index) => value !== (picked ?? [])[index]);
      },
      { timeout: 15_000 },
    )
    .toBe(true);
  const boundsNow = await viewerBounds(page);
  const pickLocal = pick2 as unknown as number[];
  for (const value of pickLocal) {
    expect(Number.isFinite(value)).toBe(true);
  }
  for (let axis = 0; axis < 3; axis += 1) {
    expect(
      (pickLocal[axis] ?? 0) >= (boundsNow?.min[axis] ?? 0) - 0.002 &&
        (pickLocal[axis] ?? 0) <= (boundsNow?.max[axis] ?? 0) + 0.002,
      `拾取点必须落在模型包围盒内（axis ${axis}：${pickLocal[axis]}）；越界即说明 raycast 命中了热点标记几何`,
    ).toBe(true);
  }
  const onFace = [0, 1, 2].some(
    (axis) =>
      Math.abs((pickLocal[axis] ?? 0) - (boundsNow?.min[axis] ?? 0)) < 0.01 ||
      Math.abs((pickLocal[axis] ?? 0) - (boundsNow?.max[axis] ?? 0)) < 0.01,
  );
  expect(onFace, "拾取点必须贴在模型表面上（至少一个轴触面）").toBe(true);
  await expect
    .poll(async () => hotspotsOf((await getDraft(api, inputs.itemId, draftId)).data).length, {
      timeout: 15_000,
    })
    .toBe(2);
  const all = hotspotsOf((await getDraft(api, inputs.itemId, draftId)).data);
  for (const hotspot of all) {
    for (const value of hotspot.anchor?.positionLocal ?? []) {
      expect(Number.isFinite(value)).toBe(true);
    }
    expect(hotspot.status).toBe("confirmed");
  }

  // 点选已有热点（标记中心 ≤18px）→ 选中而不是新建（标记不是 raycast 目标）。
  await clickCanvasAt(page, markerCenter);
  await expect
    .poll(async () =>
      page.evaluate((partId) => {
        const row = document.querySelector(`[data-testid="part-row-${partId}"]`);
        return row?.querySelector('[aria-current="true"]') !== null;
      }, partA.id),
    )
    .toBe(true);
  await page.waitForTimeout(300);
  expect(hotspotsOf((await getDraft(api, inputs.itemId, draftId)).data).length, "点选标记不得新建热点").toBe(2);

  // AC-057（1-based 跳页一致）：步骤出处按钮的页码与原文页签一致。
  const pageButton = page
    .getByTestId("steps-panel")
    .getByRole("button", { name: /^第 \d+ 页$/ })
    .first();
  await expect(pageButton).toBeVisible();
  const evidencePage = Number(/第 (\d+) 页/.exec((await pageButton.textContent()) ?? "")?.[1] ?? "0");
  expect(evidencePage, "出处页码必须 ≥1（1-based）").toBeGreaterThanOrEqual(1);
  await pageButton.click();
  await expect(page.getByTestId("original-page-label")).toContainText(`第 ${evidencePage} /`);

  await shot(page, "qa19-1-pick-and-raycast");
  writeEvidence("qa19-1-pick.json", {
    itemId: inputs.itemId,
    draftId,
    model: { revisionId: model.revisionId, sha256: model.sha256 },
    picked: picked,
    secondPick: pick2,
    markerRadius,
    markerOffsetPx: offsetPx,
    hotspots: all.map((hotspot) => ({ id: hotspot.id, partId: hotspot.partId, status: hotspot.status })),
    routing: { fulfilledOk: routing.fulfilledOk, external: routing.external, publishRequests: routing.publishRequests },
  });
  expect(routing.fulfilledOk, "全程不得伪造正常响应").toBe(0);
  expect(routing.external, "全程零真实外网").toEqual([]);
  expect(routing.publishRequests, "全程不得自动发布").toEqual([]);
  await api.context.dispose();
});

// ---------------------------------------------------------------------------
// QA19-2 · AC-053：重新生成模型 → 旧绑定 stale、旧 sha 被拒、旧发布版仍指向旧模型
// ---------------------------------------------------------------------------

test("QA19-2 重新生成模型：旧绑定 stale（真实链路）、旧 sha 被拒、旧发布版字节不变（AC-053）", async ({
  page,
}) => {
  const api = await freshApi();
  const inputs = await seedReadyItem(api, "QA19 stale 物品");
  const bytesA = fs.readFileSync(fixturePath("sample-model.glb"));
  const shaA = sha256Hex(bytesA);

  // 第一轮（模型 A）：真实流水线 → 草稿 1 → 发布版本 1。
  const job1 = await createJobOnItem(api, inputs, "qa19-stale-job-1");
  const draft1Info = await waitForDraft(api, job1);
  expect(draft1Info.jobStatus).toBe("succeeded");
  const draft1 = await getDraft(api, inputs.itemId, draft1Info.draftId);
  const modelA = modelIdentity(draft1.data.knowledge);
  expect(modelA.sha256, "第一轮模型必须等于 QA fixture 当时的真实字节").toBe(shaA);
  const ready1 = await makeEntitiesAndHotspotsReady(api, inputs.itemId, draft1Info.draftId);
  const view1 = await patchDraftOk(api, inputs.itemId, draft1Info.draftId, ready1, {
    modelReview: { loaded: true, userConfirmed: true },
  });
  const hotspots1 = hotspotsOf(view1.data);
  expect(hotspots1.length).toBeGreaterThan(0);
  expect(hotspots1.every((hotspot) => hotspot.status === "confirmed")).toBe(true);
  const hotspot1Id = (hotspots1[0] as HotspotView).id;

  const published1 = await publishDraft(api, inputs.itemId, draft1Info.draftId, {
    ifMatch: view1.etag,
    idempotencyKey: "qa19-stale-publish-1",
  });
  expect(published1.status, JSON.stringify(published1.json)).toBe(201);
  const release1Id = published1.json.data?.id ?? "";
  const detail1Before = await releaseDetail(api, inputs.itemId, release1Id);
  expect(detail1Before.modelRevisionId).toBe(modelA.revisionId);
  expect(detail1Before.manifest.model?.sha256).toBe(modelA.sha256);
  const manifestBytes1Before = (
    await fetchAsset(api.context, backend.base, detail1Before.manifestAssetId)
  ).bytes;
  expect(sha256Hex(manifestBytes1Before)).toBe(detail1Before.manifestSha256);
  const modelAssetId1 = detail1Before.manifest.model?.assetId ?? "";
  const modelBytes1Before = (await fetchAsset(api.context, backend.base, modelAssetId1)).bytes;
  expect(sha256Hex(modelBytes1Before), "旧发布版指向旧模型真实字节").toBe(shaA);

  // 第二轮（模型 B）：切换 CDN 字节 → 同一物品的新任务 → 新草稿。
  fixture.modelBytes = variantModelBytes();
  const shaB = sha256Hex(fixture.modelBytes);
  expect(shaB).not.toBe(shaA);
  const job2 = await createJobOnItem(api, inputs, "qa19-stale-job-2");
  const draft2Info = await waitForDraft(api, job2);
  expect(draft2Info.jobStatus).toBe("succeeded");
  expect(draft2Info.draftId, "新任务必须产出新草稿").not.toBe(draft1Info.draftId);
  const draft2 = await getDraft(api, inputs.itemId, draft2Info.draftId);
  const modelB = modelIdentity(draft2.data.knowledge);
  expect(modelB.sha256, "第二轮模型必须等于切换后的真实字节").toBe(shaB);
  expect(modelB.revisionId).not.toBe(modelA.revisionId);

  // 旧绑定继承为 stale（anchor 保留作解释，不是有效热点）。
  const carried = hotspotsOf(draft2.data).find((hotspot) => hotspot.id === hotspot1Id);
  expect(carried, "新草稿必须继承旧热点（stale）").toBeTruthy();
  expect((carried as HotspotView).status).toBe("stale");
  expect((carried as HotspotView).anchor?.modelRevisionId).toBe(modelA.revisionId);
  expect((carried as HotspotView).anchor?.modelSha256).toBe(modelA.sha256);
  // 换模型清空 modelReview（用户声明必须重新做出）。
  expect(draft2.data.review?.modelReview ?? null, "换模型必须清空 modelReview").toBeNull();

  // API 拒绝旧 sha 的 confirmed 提交。
  const rejected = await patchDraft(api, inputs.itemId, draft2Info.draftId, draft2.etag, {
    hotspots: {
      upsert: [
        {
          id: hotspot1Id,
          partId: (carried as HotspotView).partId,
          status: "confirmed",
          anchor: {
            modelRevisionId: modelA.revisionId,
            modelSha256: modelA.sha256,
            positionLocal: [0.5, 0.5, 0.5],
          },
        },
      ],
    },
  });
  expect(rejected.status, JSON.stringify(rejected.json)).toBe(422);
  expect(JSON.stringify(rejected.json)).toContain("旧模型版本");

  // 存在 stale 热点时发布被拒（AC-056 的"stale 热点"子句）：stale 绑定不得满足
  // "每个交互部件至少一个 confirmed 热点"，422 明细对该部件报 hotspotMissing，且不产生 release。
  // （"confirmed/candidate 冒充" 的 hotspotNotMatchingModel 分支需要篡改注入，
  //  由 RD `publishing.rs::publish_rejects_tampered_references_and_mismatched_hotspots` 覆盖。）
  const blocked = await publishDraft(api, inputs.itemId, draft2Info.draftId, {
    ifMatch: draft2.etag,
    idempotencyKey: "qa19-stale-publish-blocked",
  });
  expect(blocked.status, JSON.stringify(blocked.json)).toBe(422);
  const blockedIssues =
    (blocked.json.error?.details as
      | { issues?: { code: string; entityId?: string }[] }
      | undefined)?.issues ?? [];
  const stalePartIssue = blockedIssues.find(
    (issue) => issue.code === "hotspotMissing" && issue.entityId === (carried as HotspotView).partId,
  );
  expect(
    stalePartIssue,
    `stale 绑定必须使该部件报 hotspotMissing（不得当有效热点用）：${JSON.stringify(blockedIssues)}`,
  ).toBeTruthy();
  expect(await listReleases(api, inputs.itemId), "被拒发布不得产生 release").toHaveLength(1);

  // 浏览器：stale 单独区块、不显示为有效热点、发布预检计入、可重新绑定。
  const routing = await openReview(page, inputs.itemId, draft2Info.draftId);
  await expect(page.getByTestId("stale-hotspots")).toBeVisible();
  await expect(page.getByTestId("stale-hotspots")).toContainText("已失效热点（旧模型版本）");
  await expect(page.getByTestId("stale-hotspots")).toContainText("不再作为有效热点显示");
  await expect(page.getByTestId(`part-hotspot-${(carried as HotspotView).partId}`)).toContainText(
    "个已失效",
  );
  await expect(page.getByTestId("publish-checklist")).toContainText(
    `已失效热点：${hotspots1.length}`,
  );
  expect(
    (await viewerAnchors(page)).length,
    "stale 绑定不得作为有效热点显示在 3D（桥的 anchors 为空）",
  ).toBe(0);
  await shot(page, "qa19-2-stale-block");

  // UI 重新绑定：点击「在新模型上重新绑定」→ 拾取 → 状态回 confirmed（新 sha）。
  await page
    .getByTestId("stale-hotspots")
    .getByRole("button", { name: "在新模型上重新绑定" })
    .first()
    .click();
  await expect(page.getByTestId("pick-hint")).toContainText("重新绑定");
  const bounds = await viewerBounds(page);
  const centerLocal: [number, number, number] = [0, 1, 2].map(
    (axis) => ((bounds?.min[axis] ?? 0) + (bounds?.max[axis] ?? 0)) / 2,
  ) as [number, number, number];
  const projection = await projectLocal(page, centerLocal);
  await clickCanvasAt(page, projection.screen);
  await expect
    .poll(
      async () => {
        const current = await getDraft(api, inputs.itemId, draft2Info.draftId);
        const rebound = hotspotsOf(current.data).find((hotspot) => hotspot.id === hotspot1Id);
        return rebound?.status ?? "missing";
      },
      { timeout: 20_000 },
    )
    .toBe("confirmed");
  const rebound = hotspotsOf((await getDraft(api, inputs.itemId, draft2Info.draftId)).data).find(
    (hotspot) => hotspot.id === hotspot1Id,
  ) as HotspotView;
  expect(rebound.anchor?.modelSha256, "重新绑定必须使用新模型 sha").toBe(modelB.sha256);
  expect(rebound.anchor?.modelRevisionId).toBe(modelB.revisionId);
  await expect
    .poll(async () => (await viewerAnchors(page)).length, { timeout: 20_000 })
    .toBe(1);

  // 旧发布版继续指向旧模型且可读（字节与哈希不变）。
  const detail1After = await releaseDetail(api, inputs.itemId, release1Id);
  expect(detail1After.manifestSha256).toBe(detail1Before.manifestSha256);
  const manifestBytes1After = (
    await fetchAsset(api.context, backend.base, detail1After.manifestAssetId)
  ).bytes;
  expect(manifestBytes1After.equals(manifestBytes1Before), "release 字节必须不变").toBe(true);
  const modelBytes1After = (await fetchAsset(api.context, backend.base, modelAssetId1)).bytes;
  expect(sha256Hex(modelBytes1After)).toBe(shaA);

  await shot(page, "qa19-2-rebound");
  writeEvidence("qa19-2-stale.json", {
    itemId: inputs.itemId,
    draft1Id: draft1Info.draftId,
    draft2Id: draft2Info.draftId,
    release1Id,
    modelA: { revisionId: modelA.revisionId, sha256: modelA.sha256 },
    modelB: { revisionId: modelB.revisionId, sha256: modelB.sha256 },
    carried: { id: carried?.id, status: carried?.status, anchor: carried?.anchor },
    rebound: { status: rebound.status, anchor: rebound.anchor },
    manifestSha256: detail1Before.manifestSha256,
    routing: { fulfilledOk: routing.fulfilledOk, external: routing.external },
  });
  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
  // 全链路零付费外部调用之外的观察：fixture 计数只反映本机链路。
  expect(fixture.counts.submit, "两次生成 = 两次付费提交（fixture 本机）").toBeGreaterThanOrEqual(2);
  await api.context.dispose();
});

// ---------------------------------------------------------------------------
// QA19-3 · AC-055/AC-056：发布不变量阶梯、428/422/412/409、幂等重放、发布后独立
// ---------------------------------------------------------------------------

test("QA19-3 发布不变量阶梯、幂等重放与发布后 draft 独立性（AC-055/AC-056）", async () => {
  const api = await freshApi();
  const inputs = await seedReadyItem(api, "QA19 发布物品");
  const jobId = await createJobOnItem(api, inputs, "qa19-publish-job");
  const { draftId, jobStatus } = await waitForDraft(api, jobId);
  expect(jobStatus).toBe("succeeded");
  const draft = await getDraft(api, inputs.itemId, draftId);
  modelIdentity(draft.data.knowledge);

  // 缺 If-Match → 428；缺 Idempotency-Key → 422（字段级）。
  const noIfMatch = await publishDraft(api, inputs.itemId, draftId, {
    idempotencyKey: "qa19-pub-ladder",
  });
  expect(noIfMatch.status, JSON.stringify(noIfMatch.json)).toBe(428);
  const noKey = await publishDraft(api, inputs.itemId, draftId, { ifMatch: draft.etag });
  expect(noKey.status, JSON.stringify(noKey.json)).toBe(422);
  expect(JSON.stringify(noKey.json)).toContain("idempotencyKey");

  // 未确认知识 / 缺 confirmed 热点 / modelReview 未声明 → 422 明细，逐条列出。
  const incomplete = await publishDraft(api, inputs.itemId, draftId, {
    ifMatch: draft.etag,
    idempotencyKey: "qa19-pub-ladder",
  });
  expect(incomplete.status, JSON.stringify(incomplete.json)).toBe(422);
  const issues = (
    incomplete.json.error?.details as { issues?: { code: string; entityKind: string; message: string }[] } | undefined
  )?.issues ?? [];
  const codes = issues.map((issue) => issue.code);
  for (const code of ["knowledgeUnreviewed", "hotspotMissing", "modelReviewMissing"]) {
    expect(codes, `422 明细必须包含 ${code}：${JSON.stringify(issues)}`).toContain(code);
  }
  for (const issue of issues) {
    expect(issue.entityKind, `issue 必须有 entityKind：${JSON.stringify(issue)}`).not.toBe("");
    expect(issue.message.length).toBeGreaterThan(0);
  }
  expect(await listReleases(api, inputs.itemId), "不满足不变量时不得产生 release").toEqual([]);

  // 除 modelReview 外全部满足 → 仍 422（modelReviewMissing）。
  let view = await makeEntitiesAndHotspotsReady(api, inputs.itemId, draftId);
  const onlyModelReview = await publishDraft(api, inputs.itemId, draftId, {
    ifMatch: view.etag,
    idempotencyKey: "qa19-pub-ladder",
  });
  expect(onlyModelReview.status).toBe(422);
  expect(JSON.stringify(onlyModelReview.json)).toContain("modelReviewMissing");

  // loaded 单真 → 422（modelReviewIncomplete）；双真 → 201（只有双真才可发布）。
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    modelReview: { loaded: true, userConfirmed: false },
  });
  const loadedOnly = await publishDraft(api, inputs.itemId, draftId, {
    ifMatch: view.etag,
    idempotencyKey: "qa19-pub-ladder",
  });
  expect(loadedOnly.status).toBe(422);
  expect(JSON.stringify(loadedOnly.json)).toContain("modelReviewIncomplete");

  // 并发/陈旧 If-Match → 412 + currentRevision。
  const staleRevision = view.etag;
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    modelReview: { loaded: true, userConfirmed: true },
  });
  const stale = await publishDraft(api, inputs.itemId, draftId, {
    ifMatch: staleRevision,
    idempotencyKey: "qa19-pub-ladder",
  });
  expect(stale.status, JSON.stringify(stale.json)).toBe(412);
  expect(JSON.stringify(stale.json)).toContain("currentRevision");

  // 发布 → 201（同时记录发布前 revision，用于并发与幂等断言）。
  const revisionBeforePublish = view.data.revision;
  const published = await publishDraft(api, inputs.itemId, draftId, {
    ifMatch: view.etag,
    idempotencyKey: "qa19-pub-ladder",
  });
  expect(published.status, JSON.stringify(published.json)).toBe(201);
  const releaseId = published.json.data?.id ?? "";
  expect(releaseId).not.toBe("");
  expect(published.json.data?.draftRevision).toBe(revisionBeforePublish);
  expect(published.json.data?.draftRevisionAfterPublish).toBe(revisionBeforePublish + 1);

  const detailBefore = await releaseDetail(api, inputs.itemId, releaseId);
  expect(detailBefore.manifest.draftRevision).toBe(revisionBeforePublish);
  const manifestBytesBefore = (
    await fetchAsset(api.context, backend.base, detailBefore.manifestAssetId)
  ).bytes;
  expect(sha256Hex(manifestBytesBefore)).toBe(detailBefore.manifestSha256);

  // 幂等重放：同 key 同 body（旧 If-Match 也走重放路径）→ 同一 release + 标记头。
  const replay = await publishDraft(api, inputs.itemId, draftId, {
    ifMatch: view.etag,
    idempotencyKey: "qa19-pub-ladder",
  });
  expect(replay.status, JSON.stringify(replay.json)).toBe(201);
  expect(replay.headers["x-idempotent-replay"]).toBe("true");
  expect(replay.json.data?.id).toBe(releaseId);
  // 同 key 不同 body（用新 revision）→ 409。
  const conflict = await publishDraft(api, inputs.itemId, draftId, {
    ifMatch: `"r${revisionBeforePublish + 1}"`,
    idempotencyKey: "qa19-pub-ladder",
  });
  expect(conflict.status, JSON.stringify(conflict.json)).toBe(409);
  expect(await listReleases(api, inputs.itemId)).toHaveLength(1);

  // 发布后修改草稿：release 字节与哈希不变（重读 manifest 资产）。
  const after = await getDraft(api, inputs.itemId, draftId);
  expect(after.data.revision).toBe(revisionBeforePublish + 1);
  const firstPartId = (partsOf(after.data)[0] as PartView).id;
  const edited = await patchDraftOk(api, inputs.itemId, draftId, after, {
    entities: { [firstPartId]: { reviewStatus: "needs_review" } },
  });
  expect(edited.data.revision).toBeGreaterThan(revisionBeforePublish + 1);
  const detailAfter = await releaseDetail(api, inputs.itemId, releaseId);
  expect(detailAfter.manifestSha256, "发布后修改草稿不得改变 release 哈希").toBe(
    detailBefore.manifestSha256,
  );
  const manifestBytesAfter = (
    await fetchAsset(api.context, backend.base, detailAfter.manifestAssetId)
  ).bytes;
  expect(manifestBytesAfter.equals(manifestBytesBefore), "release 字节必须逐字节不变").toBe(true);
  expect(detailAfter.draftRevision, "release 仍记录发布时的 draftRevision").toBe(
    revisionBeforePublish,
  );
  expect(await listReleases(api, inputs.itemId), "修改草稿不得自动发布新版本").toHaveLength(1);

  writeEvidence("qa19-3-publish.json", {
    itemId: inputs.itemId,
    draftId,
    releaseId,
    revisionBeforePublish,
    manifestSha256: detailBefore.manifestSha256,
    issuesCodes: codes,
    replayedId: replay.json.data?.id,
  });
  await api.context.dispose();
});

// ---------------------------------------------------------------------------
// QA19-4 · 卡内边界：窄屏禁用几何校准/视角保存、文字确认与发布保留、禁用措辞、无自动发布
// ---------------------------------------------------------------------------

test("QA19-4 窄屏禁用几何校准（含视角保存）并解释；文字确认与发布保留；禁用措辞（AC-060 前端侧/§6.3.2）", async ({
  page,
}) => {
  const api = await freshApi();
  const inputs = await seedReadyItem(api, "QA19 窄屏物品");
  const jobId = await createJobOnItem(api, inputs, "qa19-narrow-job");
  const { draftId, jobStatus } = await waitForDraft(api, jobId);
  expect(jobStatus).toBe("succeeded");
  const draft = await getDraft(api, inputs.itemId, draftId);
  const parts = partsOf(draft.data);
  expect(parts.length).toBeGreaterThan(0);
  const partId = (parts[0] as PartView).id;

  await page.setViewportSize({ width: 700, height: 900 });
  const routing = await openReview(page, inputs.itemId, draftId);
  expect(routing.publishRequests, "页面加载不得自动发布").toEqual([]);

  // 几何校准（拾取/绑定）禁用 + 解释性提示。
  await expect(page.getByTestId("pick-mode-toggle")).toBeDisabled();
  await expect(page.getByTestId("pick-hint")).toContainText("≥768px");
  await expect(page.getByTestId("pick-hint")).toContainText("只读热点");
  await page.getByRole("button", { name: "部件与热点" }).click();
  await expect(page.getByTestId(`part-row-${partId}`).getByRole("button", { name: "绑定热点" })).toBeDisabled();
  await expect(page.getByTestId(`part-hotspot-${partId}`)).toContainText("未绑定");
  // 键盘可达 + 可见 focus（非 3D 核心操作；部件列表是热点的文字替代路径）。
  const partButton = page.getByTestId(`part-row-${partId}`).getByRole("button").first();
  // 键盘模态：焦点环由 `:focus-visible` 提供（只对键盘意图显示）。
  await partButton.focus();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");
  await expect(partButton).toBeFocused();
  const outline = await partButton.evaluate((element) => getComputedStyle(element).outlineStyle);
  expect(outline, "键盘焦点必须有可见 focus 环").not.toBe("none");
  await page.keyboard.press("Enter");
  await expect(partButton).toHaveAttribute("aria-current", "true");
  await page.keyboard.press("Escape");
  // 视角保存属于校准：窄屏禁用 + 解释。
  await page.getByRole("button", { name: "步骤与原文" }).click();
  const savePose = page.getByRole("button", { name: "保存当前视角" }).first();
  await expect(savePose).toBeDisabled();
  expect(await savePose.getAttribute("title")).toContain("≥768px");

  // 文字事实确认可用（抽屉内），且发布入口保留但禁用（缺 confirmed 热点）。
  const confirmButton = page.getByTestId(`knowledge-${partId}`).getByRole("button", { name: "确认事实" });
  await expect(confirmButton).toBeEnabled();
  await confirmButton.click();
  await expect(page.getByTestId(`knowledge-status-${partId}`)).toContainText("已确认");
  await expect(page.getByTestId("publish-panel")).toBeVisible();
  await expect(page.getByTestId("narrow-publish-note")).toContainText("≥768px");
  await expect(page.getByTestId("publish-checklist")).toContainText("缺 confirmed 热点部件");
  await expect(page.getByRole("button", { name: "发布（生成不可变版本）" })).toBeDisabled();

  // 禁用措辞清单（§6.3.2）：只允许出现在"否定语境"里。
  const bodyText = (await page.locator("body").textContent()) ?? "";
  const forbidden = ["已自动校准", "总进度", "已证明页图来自原 PDF", "重试不会重复收费", "离线可用", "零费用"];
  const hits: { phrase: string; context: string }[] = [];
  for (const phrase of forbidden) {
    let index = bodyText.indexOf(phrase);
    while (index >= 0) {
      const context = bodyText.slice(Math.max(0, index - 14), index + phrase.length + 14);
      if (!/不存在|不提供|不会|不得|不是|无|未/.test(context)) {
        hits.push({ phrase, context });
      }
      index = bodyText.indexOf(phrase, index + phrase.length);
    }
  }
  expect(hits, `禁用措辞不得以正向断言出现：${JSON.stringify(hits)}`).toEqual([]);
  // 无自动/一键发布与控制入口（角色级断言，避免被说明文字误伤）。
  expect(await page.getByRole("button", { name: /自动发布|一键发布|自动校准|一键发布到线上/ }).count()).toBe(0);
  expect(await page.getByRole("link", { name: /自动发布|一键发布|自动校准/ }).count()).toBe(0);

  // 窄屏仍可读原文（1-based 页签，加载完成后显示总页数）。
  await expect
    .poll(async () => (await page.getByTestId("original-page-label").textContent()) ?? "")
    .toMatch(/第 \d+ \/ \d+ 页/);

  // 「事实确认」与「几何校准」文案区分：不存在无定语的裸「确认」按钮；
  // 文字动作用「确认事实」，几何动作用「绑定热点/拾取/视角」字样。
  expect(await page.getByRole("button", { name: "确认", exact: true }).count()).toBe(0);
  await expect(page.getByTestId("pick-hint")).toContainText("校准");

  // 文案瑕疵留证（非阻断观察）：modelReview 面板文案里的 Markdown 粗体标记被原样渲染。
  const modelReviewText = (await page.getByTestId("model-review-panel").textContent()) ?? "";

  await shot(page, "qa19-4-narrow");

  // 关闭抽屉（覆盖层会拦截主栏点击），再走发布流程。
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("steps-panel")).toBeHidden();

  // 窄屏发布不是旁路：把不变量用 API 补齐（文字确认 + confirmed 热点 + modelReview），
  // 刷新后发布按钮在 <768px 也可用，并能真实发布成功（U-08/§6.1.4）。
  const prepare = await freshApi();
  const prepared = await makeEntitiesAndHotspotsReady(prepare, inputs.itemId, draftId);
  await patchDraftOk(prepare, inputs.itemId, draftId, prepared, {
    modelReview: { loaded: true, userConfirmed: true },
  });
  await prepare.context.dispose();
  await page.getByTestId("refresh-draft").click();
  const publishButton = page.getByRole("button", { name: "发布（生成不可变版本）" });
  await expect(publishButton).toBeEnabled({ timeout: 20_000 });

  // 并发 412（UI-056）：另一会话先改草稿 → 页面用过期 ETag 发布 → 冲突提示 + 刷新恢复。
  // 并发编辑必须**改变内容**（否则 PATCH 幂等、不递增 revision，构不成并发场景）。
  const interloper = await freshApi();
  const beforeConflict = await getDraft(interloper, inputs.itemId, draftId);
  const otherPart = parts[1] as PartView;
  await patchDraftOk(interloper, inputs.itemId, draftId, beforeConflict, {
    entities: {
      [otherPart.id]: { reviewStatus: "confirmed", userEdited: { description: "并发编辑（QA19-4）" } },
    },
  });
  await interloper.context.dispose();
  await publishButton.click();
  await expect(page.getByTestId("publish-conflict")).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId("publish-conflict")).toContainText("当前 r");
  // 观察（非阻断，见 QA 报告 P3）：冲突后发布按钮未按 UI-056「刷新前禁用发布按钮」禁用。
  const buttonDisabledDuringConflict = await publishButton.isDisabled();
  const revisionBeforeRefresh = (await page.getByTestId("draft-context").textContent()) ?? "";
  await page.getByTestId("publish-conflict").getByRole("button", { name: "刷新草稿" }).click();
  // 刷新必须真的重取服务端事实（draft-context 的 rN 反映并发修改后的新 revision）。
  await expect
    .poll(async () => (await page.getByTestId("draft-context").textContent()) ?? "", { timeout: 20_000 })
    .not.toBe(revisionBeforeRefresh);
  const conflictPanelStillVisible = await page.getByTestId("publish-conflict").isVisible();
  await expect(publishButton).toBeEnabled({ timeout: 20_000 });
  await publishButton.click();
  await expect(page.getByTestId("publish-success")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("publish-success")).toContainText("不可再修改");
  const narrowReleaseText = (await page.getByTestId("publish-success").textContent()) ?? "";
  const narrowReleaseId = /已发布版本 ([0-9a-f-]+)/.exec(narrowReleaseText)?.[1] ?? "";
  expect(narrowReleaseId, `窄屏发布必须产生 release：${narrowReleaseText}`).not.toBe("");
  await shot(page, "qa19-4-narrow-publish");

  // 拉宽后无需刷新即可用（禁用判定以视口宽度为准）。
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(page.getByTestId("pick-mode-toggle")).toBeEnabled({ timeout: 10_000 });

  writeEvidence("qa19-4-narrow.json", {
    itemId: inputs.itemId,
    draftId,
    narrowReleaseId,
    forbiddenHits: hits,
    modelReviewDeclaresLiteralAsterisks: modelReviewText.includes("**"),
    modelReviewText,
    publishConflict: { buttonDisabledDuringConflict, conflictPanelStillVisible },
    routing: { fulfilledOk: routing.fulfilledOk, external: routing.external },
  });
  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
  await api.context.dispose();
});

// ---------------------------------------------------------------------------
// QA19-5 · AC-054 + AC-052 的 API 侧：热点状态机、快照只读、modelReview 服务器赋值
// ---------------------------------------------------------------------------

test("QA19-5 热点状态机、供应商快照只读、userEdited 出处、modelReview 服务器赋值（AC-052/AC-054）", async () => {
  const api = await freshApi();
  const inputs = await seedReadyItem(api, "QA19 复核物品");
  const jobId = await createJobOnItem(api, inputs, "qa19-review-job");
  const { draftId, jobStatus } = await waitForDraft(api, jobId);
  expect(jobStatus).toBe("succeeded");
  let view = await getDraft(api, inputs.itemId, draftId);
  const model = modelIdentity(view.data.knowledge);
  const parts = partsOf(view.data);
  const steps = view.data.knowledge.knowledge?.steps ?? [];
  const specs = view.data.knowledge.knowledge?.specs ?? [];
  expect(parts.length).toBeGreaterThan(0);
  expect(steps.length).toBeGreaterThan(0);
  expect(specs.length).toBeGreaterThan(0);
  const partId = (parts[0] as PartView).id;
  const snapshotName = (parts[0] as PartView).name;

  // Evidence：1-based、页存在（本次输入 1..1）、bbox 为 null（不捏造框）。
  for (const part of parts) {
    for (const evidence of part.evidence) {
      expect(Number.isInteger(evidence.pageNumber) && evidence.pageNumber >= 1).toBe(true);
      expect(evidence.pageNumber).toBeLessThanOrEqual(1);
      expect(evidence.bbox ?? null, "bbox 必须为 null（不捏造）").toBeNull();
    }
  }

  // 未绑定 + [0,0,0] 占位 → 422。
  const placeholder = await patchDraft(api, inputs.itemId, draftId, view.etag, {
    hotspots: {
      upsert: [
        {
          partId,
          status: "unbound",
          anchor: { modelRevisionId: model.revisionId, modelSha256: model.sha256, positionLocal: [0, 0, 0] },
        },
      ],
    },
  });
  expect(placeholder.status, JSON.stringify(placeholder.json)).toBe(422);
  expect(JSON.stringify(placeholder.json)).toContain("占位");

  // confirmed 缺 anchor → 422；旧 sha → 422。
  const noAnchor = await patchDraft(api, inputs.itemId, draftId, view.etag, {
    hotspots: { upsert: [{ partId, status: "confirmed", anchor: null }] },
  });
  expect(noAnchor.status).toBe(422);
  expect(JSON.stringify(noAnchor.json)).toContain("非空 anchor");
  const staleSha = await patchDraft(api, inputs.itemId, draftId, view.etag, {
    hotspots: {
      upsert: [
        {
          partId,
          status: "confirmed",
          anchor: {
            modelRevisionId: "qa19-old-revision",
            modelSha256: "a".repeat(64),
            positionLocal: [0.1, 0.2, 0.3],
          },
        },
      ],
    },
  });
  expect(staleSha.status).toBe(422);
  expect(JSON.stringify(staleSha.json)).toContain("旧模型版本");

  // unbound（anchor=null）→ confirmed（人工直接拾取；一次请求）。
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    hotspots: { upsert: [{ partId, status: "unbound", anchor: null }] },
  });
  const unbound = hotspotsOf(view.data).find((hotspot) => hotspot.partId === partId) as HotspotView;
  expect(unbound.status).toBe("unbound");
  expect(unbound.anchor, "unbound 的 anchor 必须是 null（不是 [0,0,0]）").toBeNull();
  // unbound → candidate → confirmed（AC-052 的完整状态机；candidate 也要求匹配 anchor）。
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    hotspots: {
      upsert: [
        {
          id: unbound.id,
          partId,
          status: "candidate",
          anchor: {
            modelRevisionId: model.revisionId,
            modelSha256: model.sha256,
            positionLocal: [0.1, 0.2, 0.3],
          },
        },
      ],
    },
  });
  const candidate = hotspotsOf(view.data).find((hotspot) => hotspot.id === unbound.id) as HotspotView;
  expect(candidate.status).toBe("candidate");
  expect(candidate.anchor?.positionLocal).toEqual([0.1, 0.2, 0.3]);
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    hotspots: {
      upsert: [
        {
          id: unbound.id,
          partId,
          status: "confirmed",
          anchor: {
            modelRevisionId: model.revisionId,
            modelSha256: model.sha256,
            positionLocal: [0.125, -0.25, 0.5],
          },
        },
      ],
    },
  });
  const confirmed = hotspotsOf(view.data).find((hotspot) => hotspot.id === unbound.id) as HotspotView;
  expect(confirmed.status).toBe("confirmed");
  expect(confirmed.anchor?.positionLocal).toEqual([0.125, -0.25, 0.5]);

  // 供应商事实快照只读：直改 knowledgeJson / 未知字段 → 422。
  const snapshot = await patchDraft(api, inputs.itemId, draftId, view.etag, {
    knowledgeJson: { parts: [] },
  });
  expect(snapshot.status, JSON.stringify(snapshot.json)).toBe(422);

  // 实体级 confirmed ↔ needs_review 可切换；人工修订标 userEdited 且出处保留。
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    entities: { [partId]: { reviewStatus: "confirmed" } },
  });
  expect(view.data.review?.entities?.[partId]?.reviewStatus).toBe("confirmed");
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    entities: { [partId]: { reviewStatus: "needs_review" } },
  });
  expect(view.data.review?.entities?.[partId]?.reviewStatus).toBe("needs_review");
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    entities: { [partId]: { reviewStatus: "confirmed", userEdited: { name: "后盖（人工修订）" } } },
  });
  const edited = view.data.review?.entities?.[partId];
  expect(edited?.userEdited?.name).toBe("后盖（人工修订）");
  expect(typeof edited?.editedAt, "editedAt 必须由服务器赋值").toBe("number");
  expect((edited?.editedBy ?? "").length).toBeGreaterThan(0);
  expect(partsOf(view.data)[0]?.name, "供应商事实快照不得被修改").toBe(snapshotName);

  // modelReview：客户端不得自称 checkedAt；userConfirmed 不能脱离 loaded；
  // 服务器盖章 loadedAt/checkedAt 且记录模型身份。
  const clientCheckedAt = await patchDraft(api, inputs.itemId, draftId, view.etag, {
    modelReview: { loaded: true, userConfirmed: false, checkedAt: 1 },
  });
  expect(clientCheckedAt.status, "客户端不得写 checkedAt（服务器赋值）").toBe(422);
  const confirmWithoutLoaded = await patchDraft(api, inputs.itemId, draftId, view.etag, {
    modelReview: { loaded: false, userConfirmed: true },
  });
  expect(confirmWithoutLoaded.status, "userConfirmed 不能脱离 loaded").toBe(422);
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    modelReview: { loaded: true, userConfirmed: false },
  });
  let review = view.data.review?.modelReview ?? null;
  expect(review?.loaded).toBe(true);
  expect(review?.userConfirmed).toBe(false);
  expect(typeof review?.loadedAt, "loadedAt 由服务器赋值").toBe("number");
  expect(typeof review?.checkedAt, "checkedAt 由服务器赋值").toBe("number");
  expect(review?.modelRevisionId, "modelReview 记录模型身份（服务器赋值）").toBe(model.revisionId);
  expect(review?.modelSha256).toBe(model.sha256);
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    modelReview: { loaded: true, userConfirmed: true },
  });
  review = view.data.review?.modelReview ?? null;
  expect(review?.userConfirmed).toBe(true);
  expect(typeof review?.userConfirmedAt).toBe("number");

  // 全部实体已确认 + 热点齐备 + modelReview 双真：草稿可发布，但**没有任何自动发布**。
  view = await makeEntitiesAndHotspotsReady(api, inputs.itemId, draftId);
  for (const part of parts) {
    expect(
      hotspotsOf(view.data).some((hotspot) => hotspot.partId === part.id && hotspot.status === "confirmed"),
      `部件 ${part.id} 必须有 confirmed 热点`,
    ).toBe(true);
  }
  expect(await listReleases(api, inputs.itemId), "无自动发布路径：未显式发布时 releases 为空").toEqual([]);

  writeEvidence("qa19-5-review.json", {
    itemId: inputs.itemId,
    draftId,
    model: { revisionId: model.revisionId, sha256: model.sha256 },
    snapshotName,
    editedAt: edited?.editedAt ?? null,
    modelReview: review,
  });
  await api.context.dispose();
});

// ---------------------------------------------------------------------------
// QA19-6 · AC-057 多步分支 + AC-052 步骤视角（stepPoses 的保存/回到/清除真实复现）
// ---------------------------------------------------------------------------

test("QA19-6 多步导航（前进/后退/跳步不累积错误）与步骤视角保存/回到/清除（AC-057/AC-052）", async ({
  page,
}) => {
  const api = await freshApi();
  const inputs = await seedReadyItem(api, "QA19 步骤物品");
  const jobId = await createJobOnItem(api, inputs, "qa19-steps-job");
  const { draftId, jobStatus } = await waitForDraft(api, jobId);
  expect(jobStatus).toBe("succeeded");
  const draft = await getDraft(api, inputs.itemId, draftId);
  modelIdentity(draft.data.knowledge);
  const steps = draft.data.knowledge.knowledge?.steps ?? [];
  // QA fixture 提供两步：单步草稿覆盖不到"前进/后退/跳步"的多步分支（RD §T19-13 第 9 条）。
  expect(steps.length, "QA fixture 必须产出 ≥2 步").toBeGreaterThanOrEqual(2);
  const firstStep = steps[0] as StepView;
  const secondStep = steps[1] as StepView;

  const routing = await openReview(page, inputs.itemId, draftId);
  expect(routing.publishRequests).toEqual([]);
  const stepPosition = page.getByTestId("step-position");
  await expect(stepPosition).toContainText(`第 1 / ${steps.length} 步`);

  // 前进 → 末步禁用「下一步」；后退 → 回到首步（不累积错误状态）。
  await page.getByRole("button", { name: "下一步" }).click();
  await expect(stepPosition).toContainText(`第 2 / ${steps.length} 步`);
  await expect(page.getByRole("button", { name: "下一步" })).toBeDisabled();
  await page.getByRole("button", { name: "上一步" }).click();
  await expect(stepPosition).toContainText(`第 1 / ${steps.length} 步`);
  await expect(page.getByRole("button", { name: "上一步" })).toBeDisabled();
  // 跳步（直接点步骤行）→ 第 2 步 → 再跳回第 1 步；每步高亮一致。
  const secondRow = page.getByTestId(`step-row-${secondStep.id}`).getByRole("button").first();
  await secondRow.click();
  await expect(stepPosition).toContainText(`第 2 / ${steps.length} 步`);
  await expect(secondRow).toHaveAttribute("aria-current", "true");
  await expect(page.getByTestId(`step-row-${firstStep.id}`)).toContainText(firstStep.title);
  await expect(page.getByTestId("steps-list")).toContainText(secondStep.title);
  const firstRow = page.getByTestId(`step-row-${firstStep.id}`).getByRole("button").first();
  await firstRow.click();
  await expect(stepPosition).toContainText(`第 1 / ${steps.length} 步`);
  const bodyText = (await page.locator("body").textContent()) ?? "";
  for (const bad of ["草稿读取失败", "已被其他操作更新", "网络连接异常", "加载失败"]) {
    expect(bodyText, `跳步后不得出现错误状态：${bad}`).not.toContain(bad);
  }

  // 步骤视角（CameraPose）：保存 → 草稿持久化；改动相机后“回到该视角”可复现。
  await expect(page.getByTestId(`step-pose-${firstStep.id}`)).toContainText("未设置视角");
  await dragCanvas(page, 120, 60);
  await expect.poll(async () => viewerPose(page), { timeout: 15_000 }).not.toBeNull();
  const saved = await viewerPose(page);
  expect(saved, "保存前相机位姿必须可读").not.toBeNull();
  await page.getByRole("button", { name: "保存当前视角" }).first().click();
  await expect(page.getByTestId(`step-pose-${firstStep.id}`)).toContainText("视角已保存");
  await expect
    .poll(
      async () => {
        const current = await getDraft(api, inputs.itemId, draftId);
        const pose = (current.data.knowledge as { stepPoses?: Record<string, { positionLocal: number[] }> })
          .stepPoses?.[firstStep.id];
        return pose?.positionLocal ?? null;
      },
      { timeout: 15_000 },
    )
    .not.toBeNull();
  const storedPose = (
    (await getDraft(api, inputs.itemId, draftId)).data.knowledge as {
      stepPoses?: Record<string, { positionLocal: number[]; targetLocal: number[]; upLocal: number[]; fov: number }>;
    }
  ).stepPoses?.[firstStep.id];
  expect(storedPose, "视角必须随草稿持久化").toBeTruthy();
  for (let axis = 0; axis < 3; axis += 1) {
    expect(
      Math.abs(
        (storedPose?.positionLocal[axis] ?? Number.NaN) - ((saved?.positionLocal ?? [])[axis] ?? Number.NaN),
      ),
      `保存的 positionLocal 必须等于保存瞬间的相机位姿（轴 ${axis}）`,
    ).toBeLessThan(1e-6);
  }
  expect(Number.isFinite(storedPose?.fov ?? Number.NaN)).toBe(true);
  expect((storedPose?.fov ?? 0)).toBeGreaterThanOrEqual(1);
  expect((storedPose?.fov ?? 0)).toBeLessThanOrEqual(179);

  // 改动画布相机后「回到该视角」→ 位姿回到保存值。
  const component = (values: readonly number[] | undefined, axis: number): number =>
    values?.[axis] ?? Number.NaN;
  await dragCanvas(page, -160, 90);
  await expect
    .poll(async () => {
      const pose = await viewerPose(page);
      return pose === null || saved === null
        ? Number.NaN
        : Math.abs(component(pose.positionLocal, 0) - component(saved.positionLocal, 0));
    })
    .toBeGreaterThan(1e-3);
  await page.getByRole("button", { name: "回到该视角" }).first().click();
  await expect
    .poll(async () => {
      const pose = await viewerPose(page);
      if (pose === null || saved === null) {
        return Number.POSITIVE_INFINITY;
      }
      return Math.hypot(
        component(pose.positionLocal, 0) - component(saved.positionLocal, 0),
        component(pose.positionLocal, 1) - component(saved.positionLocal, 1),
        component(pose.positionLocal, 2) - component(saved.positionLocal, 2),
      );
    }, { timeout: 15_000 })
    .toBeLessThan(1e-3);

  // 清除视角 → 草稿中该步骤的视角被移除。
  await page.getByRole("button", { name: "清除视角" }).first().click();
  await expect(page.getByTestId(`step-pose-${firstStep.id}`)).toContainText("未设置视角");
  await expect
    .poll(async () => {
      const current = await getDraft(api, inputs.itemId, draftId);
      const poses = (current.data.knowledge as { stepPoses?: Record<string, unknown> }).stepPoses ?? {};
      return poses[firstStep.id] === undefined;
    }, { timeout: 15_000 })
    .toBe(true);

  await shot(page, "qa19-6-steps");
  writeEvidence("qa19-6-steps.json", {
    itemId: inputs.itemId,
    draftId,
    steps: steps.map((step) => step.id),
    savedPose: saved,
    storedPose,
    routing: { fulfilledOk: routing.fulfilledOk, external: routing.external },
  });
  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
  await api.context.dispose();
});

// ---------------------------------------------------------------------------
// QA19-7 · AC-057 阅读端：发布版四方联动、1-based 原文、文字替代与禁用措辞
// ---------------------------------------------------------------------------

test("QA19-7 发布版阅读器：部件↔热点↔步骤↔原文联动与 1-based 页码（AC-057）", async ({ page }) => {
  const api = await freshApi();
  const inputs = await seedReadyItem(api, "QA19 阅读物品");
  const jobId = await createJobOnItem(api, inputs, "qa19-reader-job");
  const { draftId, jobStatus } = await waitForDraft(api, jobId);
  expect(jobStatus).toBe("succeeded");
  const draft = await getDraft(api, inputs.itemId, draftId);
  modelIdentity(draft.data.knowledge);
  const part1 = (draft.data.knowledge.knowledge?.parts ?? [])[0] as PartView;
  const steps = draft.data.knowledge.knowledge?.steps ?? [];
  expect(steps.length).toBeGreaterThanOrEqual(2);

  let view = await makeEntitiesAndHotspotsReady(api, inputs.itemId, draftId);
  view = await patchDraftOk(api, inputs.itemId, draftId, view, {
    modelReview: { loaded: true, userConfirmed: true },
  });
  const published = await publishDraft(api, inputs.itemId, draftId, {
    ifMatch: view.etag,
    idempotencyKey: "qa19-reader-publish-1",
  });
  expect(published.status, JSON.stringify(published.json)).toBe(201);
  const releaseId = published.json.data?.id ?? "";

  const routing = await installRealRouting(page);
  await page.goto(`${WEB_BASE}/`);
  await loginViaUi(page, WEB_BASE, BACKEND_PASSWORD);
  await page.goto(`${WEB_BASE}/items/${inputs.itemId}/releases/${releaseId}`);
  await waitForViewer(page);
  expect(routing.publishRequests, "阅读器不得自动发布").toEqual([]);

  const context = (await page.getByTestId("release-context").first().textContent()) ?? "";
  expect(context).toContain(releaseId);
  expect(context).toContain("不可变");

  // 部件列表（热点的文字替代路径）↔ 3D：点部件 → 选中并定位热点。
  await expect(page.getByTestId("parts-list")).toContainText(part1.name);
  const partButton = page.getByTestId(`reader-part-${part1.id}`).getByRole("button").first();
  await partButton.click();
  await expect(partButton).toHaveAttribute("aria-current", "true");
  await expect(page.getByTestId("reader-notice")).toContainText("已在 3D 中定位部件");
  const anchors = await viewerAnchors(page);
  expect(anchors.length, "发布版的 confirmed 热点必须作为有效热点显示").toBeGreaterThan(0);
  expect(anchors.some((anchor) => anchor.partId === part1.id)).toBe(true);

  // 步骤导航（多步：前进/后退），并显示当前步骤。
  await expect(page.getByTestId("reader-step-position")).toContainText(`第 1 / ${steps.length} 步`);
  await page.getByRole("button", { name: "下一步" }).click();
  await expect(page.getByTestId("reader-step-position")).toContainText(`第 2 / ${steps.length} 步`);
  await expect(page.getByTestId("reader-current-step")).toContainText("当前步骤");
  await page.getByRole("button", { name: "上一步" }).click();
  await expect(page.getByTestId("reader-step-position")).toContainText(`第 1 / ${steps.length} 步`);

  // 原文跳页：1-based，且与文字层一致。
  const evidenceButton = page.getByTestId("steps-panel").getByRole("button", { name: /^第 \d+ 页$/ }).first();
  await evidenceButton.click();
  const evidencePage = Number(/第 (\d+) 页/.exec((await evidenceButton.textContent()) ?? "")?.[1] ?? "0");
  expect(evidencePage).toBeGreaterThanOrEqual(1);
  await expect(page.getByTestId("original-page-label")).toContainText(`第 ${evidencePage} /`);
  await expect
    .poll(async () => {
      const canvas = page.locator('[data-testid="original-canvas"]');
      return canvas.isVisible();
    }, { timeout: 30_000 })
    .toBe(true);

  // 发布版不提供任何编辑/发布入口；禁用措辞不得以正向断言出现。
  expect(await page.getByRole("button", { name: /发布（生成不可变版本）|编辑已发布内容|一键发布/ }).count()).toBe(0);
  expect(await page.getByRole("link", { name: /编辑已发布内容|一键发布/ }).count()).toBe(0);
  const readerText = (await page.locator("body").textContent()) ?? "";
  const forbidden = ["已自动校准", "总进度", "已证明页图来自原 PDF", "重试不会重复收费", "离线可用", "零费用"];
  const hits: { phrase: string; context: string }[] = [];
  for (const phrase of forbidden) {
    let index = readerText.indexOf(phrase);
    while (index >= 0) {
      const contextText = readerText.slice(Math.max(0, index - 14), index + phrase.length + 14);
      if (!/不存在|不提供|不会|不得|不是|无|未/.test(contextText)) {
        hits.push({ phrase, context: contextText });
      }
      index = readerText.indexOf(phrase, index + phrase.length);
    }
  }
  expect(hits, `阅读器禁用措辞不得以正向断言出现：${JSON.stringify(hits)}`).toEqual([]);

  await shot(page, "qa19-7-reader");
  writeEvidence("qa19-7-reader.json", {
    itemId: inputs.itemId,
    draftId,
    releaseId,
    anchors: anchors.map((anchor) => ({ id: anchor.id, partId: anchor.partId })),
    evidencePage,
    forbiddenHits: hits,
    routing: {
      fulfilledOk: routing.fulfilledOk,
      external: routing.external,
      publishRequests: routing.publishRequests,
      assetContent: routing.assetContent.length,
    },
  });
  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
  await api.context.dispose();
});
