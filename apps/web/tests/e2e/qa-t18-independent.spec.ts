/**
 * T18「GLB 阅读器和资源恢复」QA 独立验收（QA 回合 22；PRD 修订 2 / ui_revision 2）。
 *
 * 与 RD 的 `viewer.spec.ts` **故意不同的观察方式**（不复制 RD 断言、不采信其结论）：
 * 1. **真实端到端数据**：草稿由**真实流水线**产出（测试构建后端 + 本机 fixture 供应商，
 *    与 T17 QA 同一设施），模型字节由**真实 `GET /assets/{id}/content`** 从 data-dir 提供。
 *    链路用例**不使用任何正常响应伪造**（`route.fulfill` 计数必须为 0，只有故障注入
 *    用例会注入一次 500），只做源站改写（Vite 端口 → 测试后端端口）与非本机请求阻断
 *    ——逐字节回答"能否用真实后端数据驱动阅读器"。
 * 2. **独立渲染证据**：在真实 WebGL 上下文对象上就地包一层计数器（drawElements /
 *    createBuffer / deleteBuffer / createTexture / deleteTexture），不依赖 RD 的
 *    `__EM_VIEWER__` 账本；账本只用于对照。
 * 3. **坐标一致性用第一性原理重算**：世界坐标必须等于 `(局部 − 包围盒中心) · (1/半径)`
 *    （fit 的语义），而不是读 RD 的实现结果。
 * 4. 加载/错误状态用**故障注入**（延迟/500 真实端点）观察，不伪造响应体。
 */

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

import {
  expect,
  request as playwrightRequest,
  test,
  type APIRequestContext,
  type CDPSession,
  type Page,
  type Route,
} from "@playwright/test";

import { captureTo, loginViaUi } from "./helpers";
import {
  BACKEND_PASSWORD,
  LocalFixture,
  TestBackend,
  buildTestServerBinary,
  seedJob,
  type SeededJob,
} from "./job-recovery-harness";
import { E2E_WEB_PORT, REPO_ROOT, fixturePath } from "./runtime";

test.describe.configure({ timeout: 300_000 });

const WEB_BASE = `http://127.0.0.1:${E2E_WEB_PORT}`;
const EVIDENCE_DIR = path.join(REPO_ROOT, "artifacts", "web-mvp", "t18-qa");

interface RealDraft {
  readonly itemId: string;
  readonly jobId: string;
  readonly draftId: string;
  readonly assetId: string;
  readonly sha256: string;
  readonly revisionId: string;
  readonly raw: Record<string, unknown>;
}

let fixture: LocalFixture;
let backend: TestBackend;
let draftA: RealDraft;
let draftB: RealDraft;
let sampleModelSha: string;

/** 模型资产 id 集合（用于区分"模型内容"与其它资产）。 */
const modelAssetIds = new Set<string>();

async function fetchJobStatus(
  context: APIRequestContext,
  base: string,
  jobId: string,
): Promise<{ status: string; draftId: string | null }> {
  const response = await context.get(`${base}/api/v1/jobs/${jobId}`);
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: { status: string; draftId: string | null } };
  return body.data;
}

/** 等待任务成功（真实流水线；失败即报错，不掩盖）。 */
async function waitForSuccess(
  context: APIRequestContext,
  base: string,
  jobId: string,
  timeoutMs = 180_000,
): Promise<{ status: string; draftId: string | null }> {
  const deadline = Date.now() + timeoutMs;
  let last: { status: string; draftId: string | null } | null = null;
  while (Date.now() < deadline) {
    last = await fetchJobStatus(context, base, jobId);
    if (last.status === "succeeded") {
      return last;
    }
    if (last.status === "failed" || last.status === "cancelled") {
      throw new Error(`任务终态为 ${last.status}，期望 succeeded`);
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`等待任务成功超时：最后状态 ${last?.status ?? "未知"}`);
}

/** 读取**真实草稿** DTO 并抽出模型引用（全部来自真实后端）。 */
async function fetchRealDraft(
  context: APIRequestContext,
  seeded: SeededJob,
  tag: string,
): Promise<RealDraft> {
  const detail = await waitForSuccess(context, backend.base, seeded.jobId);
  expect(detail.draftId, `${tag} 成功任务必须产出草稿`).toBeTruthy();
  const response = await context.get(
    `${backend.base}/api/v1/items/${seeded.itemId}/drafts/${detail.draftId}`,
  );
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: Record<string, unknown> };
  const knowledge = body.data.knowledge as Record<string, unknown>;
  const model = knowledge.model as Record<string, unknown>;
  expect(model.validationState, `${tag} 草稿模型必须是 validated`).toBe("validated");
  const assetId = String(model.assetId);
  modelAssetIds.add(assetId);
  return {
    itemId: seeded.itemId,
    jobId: seeded.jobId,
    draftId: String(detail.draftId),
    assetId,
    sha256: String(model.sha256),
    revisionId: String(model.revisionId),
    raw: body.data,
  };
}

// ---------------------------------------------------------------------------
// 路由控制器：源站改写 + 非本机阻断 + 故障注入（不做正常响应伪造）
// ---------------------------------------------------------------------------

interface Routing {
  readonly external: string[];
  readonly threeRequests: string[];
  readonly pdfRequests: string[];
  readonly assetContent: { assetId: string; status: number | null }[];
  readonly fault: { mode: "passthrough" | "delay" | "fail"; delayMs: number };
  /** 被测试伪造的 2xx 响应数（必须恒为 0；只有故障注入允许非 0）。 */
  fulfilledOk: number;
  /** 被测试注入的 5xx 故障数（仅故障注入用例）。 */
  fulfilledError: number;
}

async function installRouting(page: Page): Promise<Routing> {
  const routing: Routing = {
    external: [],
    threeRequests: [],
    pdfRequests: [],
    assetContent: [],
    fault: { mode: "passthrough", delayMs: 1_500 },
    fulfilledOk: 0,
    fulfilledError: 0,
  };
  await page.route("**/*", async (route: Route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      await route.continue();
      return;
    }
    if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") {
      // 离线证据：任何外部地址一律阻断并记录（零真实外网）。
      routing.external.push(url.href);
      await route.abort();
      return;
    }
    if (/three|@react-three|ViewerStage/.test(url.href)) {
      routing.threeRequests.push(url.href);
    }
    if (/pdfjs|pdf\.worker|OriginalDocumentPanel|prepare-/.test(url.href)) {
      routing.pdfRequests.push(url.href);
    }
    const assetMatch = /\/api\/v1\/assets\/([^/]+)\/content/.exec(url.pathname);
    let target = request.url();
    if (url.href.startsWith(`${WEB_BASE}/api/v1`)) {
      // 只改写源站（Vite 端口 → 测试后端端口）：路径、方法、头、Cookie 全部保留。
      target = `${backend.base}${url.pathname}${url.search}`;
    }
    if (assetMatch !== null) {
      const assetId = assetMatch[1] ?? "";
      const entry = { assetId, status: null as number | null };
      routing.assetContent.push(entry);
      if (modelAssetIds.has(assetId) && routing.fault.mode !== "passthrough") {
        if (routing.fault.mode === "delay") {
          await new Promise((resolve) => setTimeout(resolve, routing.fault.delayMs));
          await route.continue({ url: target });
          return;
        }
        // 故障注入（不是伪造正常响应）：真实端点的失败链路。
        routing.fulfilledError += 1;
        entry.status = 500;
        await route.fulfill({
          status: 500,
          contentType: "application/json",
          body: JSON.stringify({
            error: {
              code: "INTERNAL",
              message: "QA 故障注入：模型内容 500（真实端点的失败路径）",
              details: null,
              requestId: "qa-t18-injected-failure",
            },
          }),
        });
        return;
      }
    }
    await route.continue({ url: target });
  });
  return routing;
}

async function loginAndOpenLibrary(page: Page, routing: Routing): Promise<void> {
  await page.goto(`${WEB_BASE}/`);
  await loginViaUi(page, WEB_BASE, BACKEND_PASSWORD);
  // 首屏（资料库）：3D/PDF 模块不得被强制加载（AC-062 / REQ-040）。
  await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();
  expect(routing.threeRequests, "资料库首屏不得下载 three/R3F 模块").toEqual([]);
  expect(routing.pdfRequests, "资料库首屏不得下载 pdfjs 模块").toEqual([]);
  expect(
    routing.assetContent.filter((entry) => modelAssetIds.has(entry.assetId)).length,
    "资料库首屏不得请求模型字节",
  ).toBe(0);
}

// ---------------------------------------------------------------------------
// 真实 WebGL 上下文上的独立计数器（就地包一层；不改页面源码）
// ---------------------------------------------------------------------------

type GlCounts = {
  draws: number;
  buffersCreated: number;
  buffersDeleted: number;
  texturesCreated: number;
  texturesDeleted: number;
  programsDeleted: number;
  contexts: number;
  contextsLost: number;
};

/**
 * 在**原型**上包计数（同一文档内所有 WebGL 上下文，含换路由/重建后的新上下文，
 * 都记入同一累加器）。为什么不在上下文实例上包：SPA 内切换草稿/「立即重建」会
 * 换一个 canvas 与新上下文，实例上的包装会失效（QA 首轮实测计数冻结）。
 */
async function installGlCounters(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const target = window as unknown as Record<string, unknown>;
    const counters = (target.__qaGl as Record<string, number> | undefined) ?? {
      draws: 0,
      buffersCreated: 0,
      buffersDeleted: 0,
      texturesCreated: 0,
      texturesDeleted: 0,
      programsDeleted: 0,
      contexts: 0,
      contextsLost: 0,
    };
    target.__qaGl = counters;
    if (target.__qaGlPatched === true) {
      return;
    }
    target.__qaGlPatched = true;
    type Counted = (...args: unknown[]) => unknown;
    const patch = (prototype: object | undefined): void => {
      if (prototype === undefined) {
        return;
      }
      const holder = prototype as unknown as Record<string, Counted | undefined>;
      const wrap = (name: string, key: string): void => {
        const original = holder[name];
        if (typeof original !== "function") {
          return;
        }
        holder[name] = function (this: unknown, ...args: unknown[]) {
          counters[key] = (counters[key] ?? 0) + 1;
          return original.apply(this, args);
        } as Counted;
      };
      wrap("drawElements", "draws");
      wrap("drawArrays", "draws");
      wrap("createBuffer", "buffersCreated");
      wrap("deleteBuffer", "buffersDeleted");
      wrap("createTexture", "texturesCreated");
      wrap("deleteTexture", "texturesDeleted");
      wrap("deleteProgram", "programsDeleted");
    };
    const scope = window as unknown as {
      WebGL2RenderingContext?: { prototype: object };
      WebGLRenderingContext?: { prototype: object };
      HTMLCanvasElement?: { prototype: object };
    };
    patch(scope.WebGL2RenderingContext?.prototype);
    patch(scope.WebGLRenderingContext?.prototype);
    // 统计"真的拿到了 WebGL 上下文的 canvas 数"（同一 canvas 只计一次）。
    const canvasPrototype = scope.HTMLCanvasElement?.prototype as
      | (Record<string, unknown> & { __qaCounted?: boolean })
      | undefined;
    const originalGetContext = canvasPrototype?.getContext as
      | ((this: unknown, type: string, ...rest: unknown[]) => unknown)
      | undefined;
    if (canvasPrototype !== undefined && typeof originalGetContext === "function") {
      canvasPrototype.getContext = function (
        this: { __qaCounted?: boolean },
        type: string,
        ...rest: unknown[]
      ) {
        const context = originalGetContext.call(this, type, ...rest);
        if (
          context !== null &&
          context !== undefined &&
          (type === "webgl2" || type === "webgl") &&
          this.__qaCounted !== true
        ) {
          this.__qaCounted = true;
          counters.contexts = (counters.contexts ?? 0) + 1;
          // 该 canvas 的上下文被显式释放（context lost）时计数：证明旧上下文不是
          // "留在那里等 GC"，而是被应用（R3F 卸载时的 forceContextLoss）主动丢掉。
          (this as unknown as HTMLCanvasElement).addEventListener("webglcontextlost", () => {
            counters.contextsLost = (counters.contextsLost ?? 0) + 1;
          });
        }
        return context;
      } as unknown;
    }
  });
}

async function glCounts(page: Page): Promise<GlCounts | null> {
  return page.evaluate(
    () => ((window as unknown as { __qaGl?: GlCounts }).__qaGl ?? null) as GlCounts | null,
  );
}

/** 真实丢失上下文（`WEBGL_lose_context`，与 three 的 forceContextLoss 同一 API）。 */
async function loseWebglContext(page: Page): Promise<void> {
  const result = await page.evaluate(() => {
    const element = document.querySelector('[data-testid="viewer-canvas"]');
    if (!(element instanceof HTMLCanvasElement)) {
      return "no-canvas";
    }
    const gl = (element.getContext("webgl2") ??
      element.getContext("webgl")) as WebGL2RenderingContext | WebGLRenderingContext | null;
    const extension = gl?.getExtension("WEBGL_lose_context");
    if (extension == null) {
      return "no-extension";
    }
    (window as unknown as Record<string, unknown>).__qaLose = extension;
    extension.loseContext();
    return "lost";
  });
  expect(result, "必须能通过 WEBGL_lose_context 真实丢失上下文").toBe("lost");
}

/** 恢复上下文（丢失后无法再取扩展，用丢失前暂存的对象）。 */
async function restoreWebglContext(page: Page): Promise<void> {
  const result = await page.evaluate(() => {
    const extension = (window as unknown as Record<string, unknown>).__qaLose as
      | WEBGL_lose_context
      | undefined;
    if (extension === undefined) {
      return "no-extension";
    }
    extension.restoreContext();
    return "restored";
  });
  expect(result, "必须能恢复上下文").toBe("restored");
}

/** CDP 堆用量（比 `performance.memory` 粒度细；仅作趋势观察）。 */
// 旧实现（回合 22–25 的 cdpHeapBytes）已删除：每次采样新建/关闭 CDP 会话会引入与页面
// 无关的累积（见下方 cdpHeapSession/cdpHeapSample 的说明）。历史数据见回合 26 报告。

/**
 * 复用一个 CDP 会话做堆采样（回合 26 修正）。
 *
 * 旧实现**每次采样都新建/关闭一个 CDP 会话**：会话本身（含 Performance 域缓冲）在浏览器
 * 进程里分配内存，10 次采样会引入与被测页面无关的累积 → 首末比天然漂移（历史 22 次运行
 * 的首末比 0.94–1.59，RD 回合 26 一次 1.59 越界、隔离重跑 1.23 通过）。改为复用同一会话，
 * 并在采样前**强制 GC**（`HeapProfiler.collectGarbage`）：测的是"保留堆"，与页面/账本
 * 侧的资源断言（modelsAlive=1、每轮真实 delete、卸载后 disposed==created）语义一致。
 */
async function cdpHeapSession(page: Page): Promise<CDPSession> {
  const session = await page.context().newCDPSession(page);
  await session.send("Performance.enable");
  return session;
}

/** 采一次堆（`gc: true` 时先强制 GC，排除"还没回收的垃圾"这种噪声）。 */
async function cdpHeapSample(
  session: CDPSession,
  options?: { gc?: boolean },
): Promise<{ bytes: number | null; gcApplied: boolean }> {
  let gcApplied = false;
  if (options?.gc === true) {
    try {
      await session.send("HeapProfiler.collectGarbage");
      gcApplied = true;
    } catch {
      // 该 Chromium 不支持强制 GC：如实记录，不因此让用例失败（阈值照常判定）。
      gcApplied = false;
    }
  }
  const result = (await session.send("Performance.getMetrics")) as {
    metrics: { name: string; value: number }[];
  };
  return {
    bytes: result.metrics.find((metric) => metric.name === "JSHeapUsedSize")?.value ?? null,
    gcApplied,
  };
}

/** 中位数（少量样本下的稳健统计；避免用单点判定趋势）。 */
function median(values: number[]): number {
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted[middle] ?? 0;
}

interface ViewerStats {
  modelsAlive: number;
  modelsLoaded: number;
  modelsDisposed: number;
  geometries: { created: number; disposed: number; alive: number };
  materials: { created: number; disposed: number; alive: number };
  textures: { created: number; disposed: number; alive: number };
}

async function viewerStats(page: Page): Promise<ViewerStats> {
  return page.evaluate(() => {
    const bridge = window.__EM_VIEWER__;
    if (bridge === undefined) {
      throw new Error("__EM_VIEWER__ 不存在");
    }
    return bridge.stats() as ViewerStats;
  });
}

async function viewerFrames(page: Page): Promise<number> {
  return page.evaluate(() => window.__EM_VIEWER__?.frames() ?? 0);
}

async function viewerModel(page: Page): Promise<{
  assetId: string;
  revisionId: string;
  sha256: string;
  triangles: number;
  textures: number;
  bounds: { min: readonly number[]; max: readonly number[] };
} | null> {
  return page.evaluate(() => window.__EM_VIEWER__?.model() ?? null);
}

async function viewerPose(
  page: Page,
): Promise<{ positionLocal: number[]; targetLocal: number[] } | null> {
  return page.evaluate(() => {
    const pose = window.__EM_VIEWER__?.cameraPose() ?? null;
    return pose === null ? null : { positionLocal: [...pose.positionLocal], targetLocal: [...pose.targetLocal] };
  });
}

async function roundTrip(
  page: Page,
  local: readonly [number, number, number],
): Promise<{ world: readonly number[]; back: readonly number[]; error: number } | null> {
  return page.evaluate(
    (point) => window.__EM_VIEWER__?.roundTrip(point as [number, number, number]) ?? null,
    [local[0], local[1], local[2]],
  );
}

async function spaNavigate(page: Page, target: string): Promise<void> {
  await page.evaluate((next) => {
    window.history.pushState({}, "", next);
    window.dispatchEvent(new PopStateEvent("popstate", { state: {} }));
  }, target);
}

async function canvasHasPixels(page: Page, selector: string): Promise<boolean> {
  return page.evaluate((target) => {
    const canvas = document.querySelector(target);
    if (!(canvas instanceof HTMLCanvasElement) || canvas.width === 0 || canvas.height === 0) {
      return false;
    }
    const context = canvas.getContext("2d");
    if (context === null) {
      return false;
    }
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    for (let index = 0; index < pixels.length; index += 4) {
      if (
        (pixels[index] ?? 255) < 240 ||
        (pixels[index + 1] ?? 255) < 240 ||
        (pixels[index + 2] ?? 255) < 240
      ) {
        return true;
      }
    }
    return false;
  }, selector);
}

async function waitForRealViewer(page: Page): Promise<void> {
  await expect(page.getByTestId("viewer-canvas")).toBeVisible();
  await expect.poll(async () => (await viewerStats(page)).modelsAlive, { timeout: 60_000 }).toBe(1);
  await expect.poll(async () => await viewerFrames(page), { timeout: 60_000 }).toBeGreaterThan(0);
  await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
}

const reviewPath = (draft: RealDraft): string =>
  `/items/${draft.itemId}/drafts/${draft.draftId}/review`;

// ---------------------------------------------------------------------------

test.beforeAll(async () => {
  test.setTimeout(900_000);
  buildTestServerBinary();
  fixture = new LocalFixture();
  fixture.state.manualMode = "success";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  await fixture.start();
  backend = new TestBackend("qa-t18");
  await backend.start(fixture);

  const context = await playwrightRequest.newContext();
  const seededA = await seedJob(context, backend, "QA22 真实链路 A");
  const seededB = await seedJob(context, backend, "QA22 真实链路 B");
  draftA = await fetchRealDraft(context, seededA, "草稿 A");
  draftB = await fetchRealDraft(context, seededB, "草稿 B");
  await context.dispose();

  const bytes = fs.readFileSync(fixturePath("sample-model.glb"));
  sampleModelSha = createHash("sha256").update(bytes).digest("hex");

  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });
  fs.writeFileSync(
    path.join(EVIDENCE_DIR, "r22-real-drafts.json"),
    JSON.stringify(
      {
        说明:
          "真实流水线（测试构建后端 + 本机 fixture 供应商）产出的草稿 DTO；QA 回合 22。模型字节来自 tests/fixtures/assets/sample-model.glb，经真实 job 下载/校验/落库。",
        fixtureCounts: fixture.counts,
        sampleModelSha,
        draftA: {
          itemId: draftA.itemId,
          draftId: draftA.draftId,
          assetId: draftA.assetId,
          sha256: draftA.sha256,
          revisionId: draftA.revisionId,
        },
        draftB: {
          itemId: draftB.itemId,
          draftId: draftB.draftId,
          assetId: draftB.assetId,
          sha256: draftB.sha256,
          revisionId: draftB.revisionId,
        },
        draftA_knowledge: draftA.raw.knowledge,
      },
      null,
      2,
    ),
  );
});

test.afterAll(async () => {
  await backend?.cleanup(fixture);
});

test("QA-T18-1 真实链路：真实草稿 + 真实资产字节驱动阅读器（零正常响应伪造）", async ({
  page,
}) => {
  const routing = await installRouting(page);
  await loginAndOpenLibrary(page, routing);

  await page.goto(WEB_BASE + reviewPath(draftA));
  await expect(page.getByRole("heading", { name: "阅读与复核" })).toBeVisible();
  await waitForRealViewer(page);

  // 3D 模块进入阅读页后才下载（懒加载）。
  expect(routing.threeRequests.length, "进入阅读页后应下载 three 模块").toBeGreaterThan(0);

  // 模型确实来自真实后端：资产 id、哈希与草稿事实一致。
  const info = await viewerModel(page);
  expect(info?.assetId).toBe(draftA.assetId);
  expect(info?.sha256).toBe(draftA.sha256);
  expect(info?.sha256, "草稿哈希必须等于仓库 fixture GLB 的真实哈希").toBe(sampleModelSha);
  expect(info?.triangles, "fixture 模型 12 三角面").toBe(12);
  expect(info?.textures).toBe(1);

  // 真实知识（fixture 说明书 AI 输出经真实合并后落库）在左/右栏呈现。
  await expect(page.getByTestId("parts-list")).toContainText("后盖");
  await expect(page.getByTestId("steps-list")).toContainText("取下后盖");
  await expect(page.getByTestId("reader-context")).toContainText(`草稿 ${draftA.draftId}`);

  // 原文（真实 PDF 字节 + 本地 PDF.js）真的画出了像素。
  await expect(page.getByTestId("original-page-label")).toContainText("第 1 /");
  await expect
    .poll(async () => await canvasHasPixels(page, '[data-testid="original-canvas"]'), {
      timeout: 30_000,
    })
    .toBe(true);

  // 真实模型字节请求：URL 命中草稿里的 assetId，且没有任何伪造响应。
  const modelRequests = routing.assetContent.filter((entry) => entry.assetId === draftA.assetId);
  expect(modelRequests.length, "必须请求真实资产内容端点").toBeGreaterThan(0);
  expect(routing.fulfilledOk + routing.fulfilledError, "本用例不得使用 route.fulfill").toBe(0);
  expect(routing.external, "全程零真实外网").toEqual([]);

  await captureTo("t18-qa", page, "r22-01-real-draft-viewer");
});

test("QA-T18-2 坐标一致性（真实模型 + 第一性原理重算 + 相机旋转/缩放不变）", async ({ page }) => {
  const routing = await installRouting(page);
  await loginAndOpenLibrary(page, routing);
  await page.goto(WEB_BASE + reviewPath(draftA));
  await waitForRealViewer(page);

  const info = await viewerModel(page);
  expect(info).not.toBeNull();
  const min = info?.bounds.min ?? [];
  const max = info?.bounds.max ?? [];
  const center = [0, 1, 2].map((axis) => ((min[axis] ?? 0) + (max[axis] ?? 0)) / 2);
  const radius = Math.max(
    Math.hypot(
      (max[0] ?? 0) - (min[0] ?? 0),
      (max[1] ?? 0) - (min[1] ?? 0),
      (max[2] ?? 0) - (min[2] ?? 0),
    ) / 2,
    1e-6,
  );
  const displayScale = 1 / radius;

  // 采样点：包围盒中心、一个顶点、一个角内点。
  const localPoints: [number, number, number][] = [
    [center[0] ?? 0, center[1] ?? 0, center[2] ?? 0],
    [min[0] ?? 0, max[1] ?? 0, max[2] ?? 0],
    [
      (min[0] ?? 0) * 0.5 + (max[0] ?? 0) * 0.5,
      (min[1] ?? 0) * 0.25 + (max[1] ?? 0) * 0.75,
      (min[2] ?? 0) * 0.75 + (max[2] ?? 0) * 0.25,
    ],
  ];

  const expectedWorld = (local: readonly [number, number, number]): number[] =>
    [0, 1, 2].map((axis) => ((local[axis] ?? 0) - (center[axis] ?? 0)) * displayScale);

  const measure = async (): Promise<Map<string, readonly number[]>> => {
    const out = new Map<string, readonly number[]>();
    for (const local of localPoints) {
      const trip = await roundTrip(page, local);
      expect(trip, "桥的 roundTrip 必须可用").not.toBeNull();
      expect(trip?.error ?? 1, `往返误差（局部 ${local.join(",")}）`).toBeLessThan(1e-6);
      const expected = expectedWorld(local);
      for (let axis = 0; axis < 3; axis += 1) {
        expect(
          Math.abs((trip?.world[axis] ?? Number.NaN) - (expected[axis] ?? Number.NaN)),
          `世界坐标必须等于 (局部−中心)·s（轴 ${axis}）`,
        ).toBeLessThan(1e-6);
      }
      out.set(local.join(","), trip?.world ?? []);
    }
    return out;
  };

  const before = await measure();

  // 相机旋转 + 缩放（显示变换不变，"各种旋转/缩放"场景）。
  const canvas = page.getByTestId("viewer-canvas");
  const box = await canvas.boundingBox();
  const cx = (box?.x ?? 0) + (box?.width ?? 0) / 2;
  const cy = (box?.y ?? 0) + (box?.height ?? 0) / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + 160, cy + 70, { steps: 12 });
  await page.mouse.up();
  await page.mouse.wheel(0, -420);
  await expect.poll(async () => await viewerFrames(page), { timeout: 15_000 }).toBeGreaterThan(0);

  const afterRotate = await measure();
  for (const [key, world] of before) {
    const current = afterRotate.get(key) ?? [];
    for (let axis = 0; axis < 3; axis += 1) {
      expect(
        Math.abs((world[axis] ?? Number.NaN) - (current[axis] ?? Number.NaN)),
        `旋转/缩放后同一局部点的世界位置不得改变（${key} 轴 ${axis}）`,
      ).toBeLessThan(1e-6);
    }
  }

  // 复位与适配（键盘等价控件）后仍然一致。
  await page.getByRole("button", { name: "复位视角" }).click();
  await page.getByRole("button", { name: "适配模型" }).click();
  const afterFit = await measure();
  for (const [key, world] of before) {
    const current = afterFit.get(key) ?? [];
    for (let axis = 0; axis < 3; axis += 1) {
      expect(
        Math.abs((world[axis] ?? Number.NaN) - (current[axis] ?? Number.NaN)),
      ).toBeLessThan(1e-6);
    }
  }

  // 真实草稿没有热点（热点属 T19）：锚点投影必须为空，而不是伪造标记。
  const anchors = await page.evaluate(() => window.__EM_VIEWER__?.anchors() ?? []);
  expect(anchors, "T18 真实草稿没有热点记录，锚点投影应为空").toEqual([]);

  await captureTo("t18-qa", page, "r22-02-coordinates-real-model");
});

test("QA-T18-3 上下文丢失 → 真实绘制中断 → restored 后真实恢复（独立 draw 计数）", async ({
  page,
}) => {
  const routing = await installRouting(page);
  // 计数器必须在任何文档加载前注入（原型层，覆盖后续所有 WebGL 上下文）。
  await installGlCounters(page);
  await loginAndOpenLibrary(page, routing);
  await page.goto(WEB_BASE + reviewPath(draftA));
  await waitForRealViewer(page);

  // 先转一下相机，验证恢复后位姿保留。
  const canvas = page.getByTestId("viewer-canvas");
  const box = await canvas.boundingBox();
  const cx = (box?.x ?? 0) + (box?.width ?? 0) / 2;
  const cy = (box?.y ?? 0) + (box?.height ?? 0) / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + 90, cy + 40, { steps: 8 });
  await page.mouse.up();
  const poseBefore = await viewerPose(page);
  const framesBefore = await viewerFrames(page);
  const drawsBefore = (await glCounts(page))?.draws ?? 0;
  expect(drawsBefore, "丢失前必须真的在绘制").toBeGreaterThan(0);

  // 丢失：状态可观察、交互禁用、给出「立即重建」（不是只让刷新页面）。
  await loseWebglContext(page);
  await expect(page.getByTestId("viewer-status")).toContainText("3D 显示已中断");
  await expect(page.getByRole("button", { name: "复位视角" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "立即重建" })).toBeVisible();
  await page.waitForTimeout(600);
  const drawsDuringLoss = (await glCounts(page))?.draws ?? 0;
  expect(drawsDuringLoss - drawsBefore, "丢失期间不得继续真实绘制").toBeLessThanOrEqual(2);

  // restored：真实绘制恢复 + 位姿保留（无需刷新页面）。
  await restoreWebglContext(page);
  await expect
    .poll(async () => (await glCounts(page))?.draws ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(drawsBefore + 2);
  await expect
    .poll(async () => await viewerFrames(page), { timeout: 30_000 })
    .toBeGreaterThan(framesBefore);
  await expect(page.getByTestId("viewer-status")).toContainText("3D 显示已恢复");
  await expect(page.getByRole("button", { name: "复位视角" })).toBeEnabled();
  const poseAfter = await viewerPose(page);
  for (let axis = 0; axis < 3; axis += 1) {
    expect(
      Math.abs(
        (poseAfter?.positionLocal[axis] ?? Number.NaN) -
          (poseBefore?.positionLocal[axis] ?? Number.NaN),
      ),
    ).toBeLessThan(1e-4);
  }
  await captureTo("t18-qa", page, "r22-03-restored-real-draws");

  // 「立即重建」（换新上下文，不是刷新页面）：丢失后点它，绘制继续。
  await loseWebglContext(page);
  await expect(page.getByTestId("viewer-status")).toContainText("3D 显示已中断");
  const drawsBeforeRebuild = (await glCounts(page))?.draws ?? 0;
  await page.getByRole("button", { name: "立即重建" }).click();
  await expect(page.getByTestId("viewer-canvas")).toBeVisible();
  // 模型重新加载并**真的在绘制**（独立 draw 计数，不依赖面板状态文案）。
  await expect
    .poll(async () => (await viewerStats(page)).modelsAlive, { timeout: 60_000 })
    .toBe(1);
  await expect
    .poll(async () => (await glCounts(page))?.draws ?? 0, { timeout: 30_000 })
    .toBeGreaterThan(drawsBeforeRebuild + 2);

  // 给面板 5 秒把状态恢复到可用态（公平窗口），然后记录现场。
  try {
    await expect(page.getByTestId("viewer-status")).toContainText("模型已加载", {
      timeout: 5_000,
    });
  } catch {
    // 未恢复：下面按实际文案断言失败（BUG-007 的现场已先落证据）。
  }
  const statusAfterRebuild = (await page.getByTestId("viewer-status").textContent()) ?? "";
  const resetDisabled = await page.getByRole("button", { name: "复位视角" }).isDisabled();
  const rebuildVisible = await page
    .getByRole("button", { name: "立即重建" })
    .isVisible()
    .catch(() => false);
  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });
  fs.writeFileSync(
    path.join(EVIDENCE_DIR, "r22-rebuild-state.json"),
    JSON.stringify(
      {
        说明: "手动「立即重建」后 5 秒的现场（QA 独立 GL draw 计数证明渲染已恢复）",
        modelsAlive: (await viewerStats(page)).modelsAlive,
        drawsBeforeRebuild,
        drawsAfterRebuild: (await glCounts(page))?.draws ?? null,
        statusText: statusAfterRebuild,
        resetButtonDisabled: resetDisabled,
        rebuildButtonVisible: rebuildVisible,
      },
      null,
      2,
    ),
  );
  await captureTo("t18-qa", page, "r22-03b-after-rebuild");

  // 要求（UI-044/UI-043/AC-050）：手动重建成功后，面板必须回到可用态。
  expect(statusAfterRebuild, "重建成功后状态必须回到可用（见 BUG-007）").toContain("模型已加载");
  expect(resetDisabled, "重建成功后键盘等价控件必须可用（见 BUG-007）").toBe(false);
});

test("QA-T18-4 资源释放与 10 次切换趋势（真实 GL 计数 + 账本对照 + CDP 堆采样）", async ({
  page,
}) => {
  test.setTimeout(300_000);
  const routing = await installRouting(page);
  await installGlCounters(page);
  await loginAndOpenLibrary(page, routing);
  await page.goto(WEB_BASE + reviewPath(draftA));
  await waitForRealViewer(page);

  const heapSamples: (number | null)[] = [];
  const heapRetainedSamples: (number | null)[] = [];
  const heapSession = await cdpHeapSession(page);
  let heapGcApplied = false;
  const aliveSamples: string[] = [];
  const glRawSamples: Record<string, number>[] = [];
  const deleteDeltas: { buffers: number; textures: number }[] = [];
  const draftIds: string[] = [];

  for (let round = 0; round < 10; round += 1) {
    const before = await glCounts(page);
    const target = round % 2 === 0 ? draftB : draftA;
    await spaNavigate(page, reviewPath(target));
    await waitForRealViewer(page);
    draftIds.push(target.draftId);

    const stats = await viewerStats(page);
    const counts = await glCounts(page);
    expect(stats.modelsAlive, `第 ${round + 1} 次切换后 modelsAlive`).toBe(1);
    // 换模型后不得串入旧模型/旧草稿：桥与页面文本都必须是当前草稿的事实。
    const switched = await viewerModel(page);
    expect(switched?.assetId, `第 ${round + 1} 次切换后必须是当前草稿的模型`).toBe(target.assetId);
    expect(switched?.sha256).toBe(target.sha256);
    const contextText = (await page.getByTestId("reader-context").textContent()) ?? "";
    const other = target === draftB ? draftA : draftB;
    expect(contextText, `第 ${round + 1} 次切换后的页面事实`).toContain(`草稿 ${target.draftId}`);
    expect(contextText, "不得残留上一份草稿").not.toContain(`草稿 ${other.draftId}`);
    expect(stats.geometries.alive).toBe(1);
    expect(stats.materials.alive).toBe(1);
    expect(stats.textures.alive).toBe(1);
    expect(counts, "GL 计数器必须可用").not.toBeNull();
    // 每次切换都必须真的发生 GL 删除（旧模型自己的 GPU 资源被释放）。
    const bufferDeletes = (counts?.buffersDeleted ?? 0) - (before?.buffersDeleted ?? 0);
    const textureDeletes = (counts?.texturesDeleted ?? 0) - (before?.texturesDeleted ?? 0);
    deleteDeltas.push({ buffers: bufferDeletes, textures: textureDeletes });
    expect(
      bufferDeletes,
      `第 ${round + 1} 次切换必须释放旧模型的 GPU buffer（实测 ${bufferDeletes}）`,
    ).toBeGreaterThan(0);
    expect(
      textureDeletes,
      `第 ${round + 1} 次切换必须释放旧模型的 GPU 贴图（实测 ${textureDeletes}）`,
    ).toBeGreaterThan(0);
    aliveSamples.push(
      `${stats.geometries.alive}/${stats.materials.alive}/${stats.textures.alive}`,
    );
    glRawSamples.push({ ...counts });
    heapSamples.push((await cdpHeapSample(heapSession)).bytes);
    const retained = await cdpHeapSample(heapSession, { gc: true });
    heapGcApplied = heapGcApplied || retained.gcApplied;
    heapRetainedSamples.push(retained.bytes);
  }

  // 每次切换都只有一份模型存活（RD 账本）；真实 GL 删除逐轮发生（QA 独立计数）。
  expect(new Set(aliveSamples).size).toBe(1);
  const firstHeap = heapSamples[0] ?? 0;
  const lastHeap = heapSamples[heapSamples.length - 1] ?? 0;
  const heapRatio = firstHeap > 0 ? lastHeap / firstHeap : Number.NaN;
  // 回合 26 修正：单点首末比会被"未回收垃圾 + 每次新建 CDP 会话"的噪声支配
  // （历史 22 次运行 0.94–1.59；RD 一次 1.59 越界、隔离重跑 1.23 通过）。
  // 判据改为：**强制 GC 后保留堆**的前 3 次中位数 vs 后 3 次中位数，阈值仍为 1.5。
  const retained = heapRetainedSamples.filter((value): value is number => value !== null);
  const retainedFirst = median(retained.slice(0, 3));
  const retainedLast = median(retained.slice(-3));
  const retainedRatio = retainedFirst > 0 ? retainedLast / retainedFirst : Number.NaN;
  expect(
    retainedRatio,
    `强制 GC 后保留堆中位数首末比（${retainedFirst} → ${retainedLast}；GC=${String(heapGcApplied)}）不得持续增长`,
  ).toBeLessThan(1.5);
  // 每个草稿页各挂载一次 Canvas（新 WebGL 上下文）：上下文数必须与切换次数同阶，
  // 而不是"每次渲染/每帧"增长。
  const finalCounts = glRawSamples[glRawSamples.length - 1] ?? {};
  expect(
    finalCounts.contexts ?? 0,
    `WebGL 上下文数必须与渲染器挂载次数同阶（实测 ${String(finalCounts.contexts)}）`,
  ).toBeLessThanOrEqual(13);
  // 每次卸载（换草稿）都必须显式释放其 WebGL 上下文（否则 GPU 资源会随挂载累积）。
  // 未被释放的两个上下文 = 探测用临时 canvas（probeWebgl）与当前存活的这一个。
  expect(
    finalCounts.contextsLost ?? 0,
    `每次换草稿都必须释放旧上下文：10 次切换应至少 10 次 context lost（实测上下文 ${String(finalCounts.contexts)}，已释放 ${String(finalCounts.contextsLost)}）`,
  ).toBeGreaterThanOrEqual(10);

  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });
  fs.writeFileSync(
    path.join(EVIDENCE_DIR, "r22-resource-trend.json"),
    JSON.stringify(
      {
        说明:
          "10 次草稿切换（真实后端）。deleteDeltas 是每次切换期间真实发生的 gl.deleteBuffer/deleteTexture 调用数（QA 独立原型计数）；glRawSamples 是累计原始计数（含 three 每个渲染器实例自建的 4 张 empty texture：TEXTURE_2D/CUBE/2D_ARRAY/3D，因此累计 createTexture 会随上下文数增长，这不是应用资源泄漏指标）；contexts = 真正拿到 WebGL 上下文的 canvas 数。",
        draftIds,
        aliveSamples,
        deleteDeltas,
        glRawSamples,
        contexts: finalCounts.contexts ?? null,
        heapSamples,
        heapRatio,
        heapRetainedSamples,
        heapRetainedRatio: retainedRatio,
        heapGcApplied,
      },
      null,
      2,
    ),
  );
  console.log(`QA 资源趋势：存活=${aliveSamples.join(" ")}`);
  console.log(
    `QA 每轮真实 GL 删除=${deleteDeltas.map((entry) => `${entry.buffers}b/${entry.textures}t`).join(" ")}`,
  );
  console.log(
    `QA 累计原始计数（含 three 每渲染器 4 张 empty texture）：${glRawSamples.map((entry) => `${String(entry.buffersCreated)}c/${String(entry.buffersDeleted)}d b, ${String(entry.texturesCreated)}c/${String(entry.texturesDeleted)}d t`).join(" | ")}`,
  );
  console.log(
    `QA WebGL 上下文数=${String(finalCounts.contexts)}（其中显式释放 ${String(finalCounts.contextsLost)}）`,
  );
  console.log(`QA CDP 堆采样=${heapSamples.join(", ")}（首末比 ${heapRatio.toFixed(2)}）`);
  console.log(
    `QA CDP 保留堆采样（强制 GC=${String(heapGcApplied)}）=${heapRetainedSamples.map(String).join(", ")}（中位数首末比 ${retainedRatio.toFixed(2)}）`,
  );

  // 卸载（SPA 内离开阅读页）：全部释放，且真实 GL 删除发生。
  const beforeUnmount = await glCounts(page);
  await spaNavigate(page, "/");
  await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();
  await expect
    .poll(async () => (await viewerStats(page)).geometries.alive, { timeout: 20_000 })
    .toBe(0);
  const statsAfter = await viewerStats(page);
  expect(statsAfter.geometries.disposed).toBe(statsAfter.geometries.created);
  expect(statsAfter.materials.disposed).toBe(statsAfter.materials.created);
  expect(statsAfter.textures.disposed).toBe(statsAfter.textures.created);
  expect(statsAfter.modelsAlive).toBe(0);
  await expect
    .poll(
      async () => {
        const counts = await glCounts(page);
        return (
          (counts?.buffersDeleted ?? 0) +
          (counts?.texturesDeleted ?? 0) -
          ((beforeUnmount?.buffersDeleted ?? 0) + (beforeUnmount?.texturesDeleted ?? 0))
        );
      },
      { timeout: 20_000 },
    )
    .toBeGreaterThan(0);
});

test("QA-T18-5 加载与错误状态可观察（延迟/500 故障注入真实端点）+ 重试真的生效", async ({
  page,
}) => {
  const routing = await installRouting(page);
  routing.fault.mode = "delay";
  routing.fault.delayMs = 1_500;
  await loginAndOpenLibrary(page, routing);

  await page.goto(WEB_BASE + reviewPath(draftA));
  // 延迟窗口内：loading 状态可见且可读（不是空白）。
  await expect(page.getByTestId("viewer-loading")).toBeVisible();
  await expect(page.getByTestId("viewer-loading")).toContainText("正在加载模型");
  await waitForRealViewer(page);

  // 失败注入（真实端点返回 500）：错误态可读 + 重试入口；文字/PDF 不受影响。
  routing.fault.mode = "fail";
  await spaNavigate(page, "/");
  await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();
  await spaNavigate(page, reviewPath(draftA));
  await expect(page.getByTestId("viewer-error")).toBeVisible();
  await expect(page.getByTestId("viewer-error")).toContainText("模型加载失败");
  await expect(page.getByRole("button", { name: "重试加载" })).toBeVisible();
  await expect(page.getByTestId("parts-list")).toContainText("后盖");
  await expect
    .poll(async () => await canvasHasPixels(page, '[data-testid="original-canvas"]'), {
      timeout: 30_000,
    })
    .toBe(true);
  await captureTo("t18-qa", page, "r22-04-error-state-with-text-pdf");

  // 恢复端点（撤掉故障注入）后点「重试加载」：必须真的加载成功。
  routing.fault.mode = "passthrough";
  await page.getByRole("button", { name: "重试加载" }).click();
  await waitForRealViewer(page);
  expect((await viewerModel(page))?.assetId).toBe(draftA.assetId);
  // 故障注入只作用于模型内容；草稿与其它资产全程走真实后端（零 2xx 伪造）。
  expect(routing.fulfilledError, "本用例必须真的注入过 500（不是沉默失败）").toBeGreaterThanOrEqual(1);
  expect(routing.fulfilledOk, "不得伪造正常响应").toBe(0);
});

test("QA-T18-6 WebGL 不可用：真实降级（不下载 three）+ 文字/PDF 可用 + 键盘与减少动效", async ({
  page,
}) => {
  const routing = await installRouting(page);
  // 真实降级路径：浏览器拒绝一切 WebGL 上下文（同 RD 的注入方式，但驱动真实数据）。
  await page.addInitScript(() => {
    const original = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, type: string) {
      if (type === "webgl" || type === "webgl2" || type === "experimental-webgl") {
        return null;
      }
      return original.call(this, type as never) as never;
    } as typeof HTMLCanvasElement.prototype.getContext;
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await loginAndOpenLibrary(page, routing);
  await page.goto(WEB_BASE + reviewPath(draftA));

  await expect(page.getByTestId("viewer-unavailable")).toBeVisible();
  await expect(page.getByTestId("viewer-unavailable")).toContainText("浏览器 3D 上下文不可用");
  await expect(page.getByTestId("viewer-canvas")).toHaveCount(0);
  // 效率与降级：3D 不可用时连 three chunk 都不必下载（独立观察）。
  expect(routing.threeRequests, "WebGL 不可用时不应下载 three 模块").toEqual([]);

  // 文字路径真实可用（部件/步骤 + 原文页真实渲染）。
  await expect(page.getByTestId("parts-list")).toContainText("后盖");
  await expect(page.getByTestId("steps-list")).toContainText("取下后盖");
  await expect
    .poll(async () => await canvasHasPixels(page, '[data-testid="original-canvas"]'), {
      timeout: 30_000,
    })
    .toBe(true);

  // 键盘路径：聚焦「改用文字阅读」，回车后焦点交给部件面板（UI-059）。
  const textPath = page.getByRole("button", { name: "改用文字阅读" });
  await textPath.focus();
  const outline = await textPath.evaluate((element) => getComputedStyle(element).outlineStyle);
  expect(outline, "键盘焦点必须有可见 focus 样式").not.toBe("none");
  await page.keyboard.press("Enter");
  await expect
    .poll(() => page.evaluate(() => document.activeElement?.getAttribute("data-testid") ?? ""))
    .toBe("parts-panel");

  // 减少动效：阅读器无相机自动动画；文字路径照常。
  await expect(page.getByTestId("reader-context")).toContainText("1-based");
  expect(routing.external, "全程零真实外网").toEqual([]);
  await captureTo("t18-qa", page, "r22-05-webgl-unavailable-real-data");
});
