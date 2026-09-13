/**
 * T18 阅读器 e2e 的共享造数（真实后端 + 本机 fixture 模型）。
 *
 * 两种数据来源（**不要**再把"必须拦截"当成事实；QA 回合 22 已实测更正）：
 * 1. **真实链路**（`installRealBackendRouting` + 测试构建后端 + 本机 fixture 供应商，
 *    见 `viewer.spec.ts` 的"真实链路"describe 与 `qa-t18-independent.spec.ts`）：
 *    `GET /items/{id}/drafts/{draftId}`（T15）与 `GET /assets/{id}/content`（T06）
 *    均已可用且无测试构建门控，草稿 DTO 与模型字节全程来自真实后端。唯一需要
 *    "测试构建 + 显式 fixture 配置"的是**造出**带 validated 模型的草稿。
 * 2. **拦截注入**（`installViewerRoutes`，本文件上方）：只在需要合成 DTO 的场景使用
 *    ——stale 热点（T19 前真实草稿没有 `hotspots`）与 500 故障注入。拦截保留**真实
 *    的请求路径与响应形态**，不是"用 mock 冒充"。
 *
 * 两种方式下：**会话、物品、document、preparation、页资产、原 PDF 字节**全部走
 * 真实后端（`seedItemWithDocument` / `seedReadyPreparation`），因此"3D 失败仍可读
 * 文字与 PDF"是在真实数据上验证的。
 *
 * 断言用的可观察量来自**只读桥** `window.__EM_VIEWER__`（见
 * `src/features/viewer/bridge.ts`）：帧数、相机位姿、锚点投影、资源账本、上下文状态。
 */

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";

import { expect, type Page, type Route } from "@playwright/test";

import { E2E_WEB_PORT, readRuntime } from "./runtime";

const FIXTURE_DIR = path.join(import.meta.dirname, "fixtures");

export interface FixtureModel {
  readonly assetId: string;
  readonly revisionId: string;
  readonly bytes: Buffer;
  readonly sha256: string;
  /** asset-root 局部坐标下的表面采样点（由 fixture 生成器给出）。 */
  readonly localPoints: readonly (readonly [number, number, number])[];
}

function sha256(bytes: Buffer): string {
  return createHash("sha256").update(bytes).digest("hex");
}

/** 读取仓库内 fixture 模型（含哈希与采样点；逐字节确定）。 */
export function fixtureModel(name: "viewer-asymmetric" | "viewer-asymmetric-b", assetId: string): FixtureModel {
  const bytes = readFileSync(path.join(FIXTURE_DIR, `${name}.glb`));
  const points = JSON.parse(
    readFileSync(path.join(FIXTURE_DIR, `${name}.points.json`), "utf8"),
  ) as { assetRootLocalPoints: [number, number, number][] };
  return {
    assetId,
    revisionId: `revision-${name}`,
    bytes,
    sha256: sha256(bytes),
    localPoints: points.assetRootLocalPoints,
  };
}

export interface ViewerHarnessOptions {
  readonly itemId: string;
  readonly documentId: string;
  readonly preparationId: string;
  readonly model: FixtureModel;
  readonly hotspots: readonly {
    readonly id: string;
    readonly partId: string;
    readonly status: string;
    readonly positionLocal: readonly [number, number, number];
  }[];
  readonly partNames: readonly string[];
  readonly stepTitles: readonly string[];
  readonly modelRevision: number;
}

/**
 * 构造与 `DraftDto` 同形的草稿载荷。
 *
 * 字段与 T15 落库的外壳一致（`manual_draft_v1`：`model` / `knowledge` / `missing`），
 * hotspot 用 contracts §2 的字段名（`id`/`partId`/`status`/`anchor.{modelRevisionId,modelSha256,positionLocal}`）。
 */
export function draftPayload(options: ViewerHarnessOptions): Record<string, unknown> {
  const { model, hotspots } = options;
  return {
    id: `draft-${model.revisionId}`,
    itemId: options.itemId,
    snapshotId: "snapshot-e2e",
    modelRevisionId: model.revisionId,
    revision: options.modelRevision,
    status: "needs_review",
    completeness: "complete",
    missing: [],
    notices: ["生成完成 ≠ 已发布：不存在自动发布路径。"],
    knowledge: {
      schemaVersion: "manual_draft_v1",
      sourceJobId: "job-e2e",
      completeness: "complete",
      model: {
        revisionId: model.revisionId,
        sha256: model.sha256,
        validationState: "validated",
        assetId: model.assetId,
        bounds: null,
      },
      knowledge: {
        schemaVersion: "manual_extract_v1",
        promptVersion: "prompt-e2e",
        pageFrom: 1,
        pageTo: 1,
        coverage: { plannedPages: [1], coveredPages: [1], complete: true },
        parts: options.partNames.map((name, index) => ({
          id: `part-${model.revisionId}-${index}`,
          name,
          description: `${name} 的说明（e2e 合成数据）`,
          evidence: [
            {
              documentId: options.documentId,
              preparationId: options.preparationId,
              pageNumber: 1,
              quote: `${name} 出现在第 1 页`,
              bbox: null,
              derived: false,
            },
          ],
          reviewStatus: "needs_review",
          sourceBatches: [0],
        })),
        steps: options.stepTitles.map((title, index) => ({
          id: `step-${model.revisionId}-${index}`,
          title,
          orderedActions: [`${title} 的第一步`, `${title} 的第二步`],
          partIds: [`part-${model.revisionId}-0`],
          evidence: [
            {
              documentId: options.documentId,
              preparationId: options.preparationId,
              pageNumber: 1,
              quote: `${title} 出自第 1 页`,
              bbox: null,
              derived: false,
            },
          ],
          safetyNotes: [`${title} 的注意事项`],
          reviewStatus: "needs_review",
          sourceBatches: [0],
        })),
        specs: [],
        uncertainties: [],
        conflicts: [],
      },
      hotspots: hotspots.map((hotspot) => ({
        id: hotspot.id,
        partId: hotspot.partId,
        status: hotspot.status,
        anchor: {
          modelRevisionId: model.revisionId,
          modelSha256: model.sha256,
          positionLocal: [...hotspot.positionLocal],
        },
      })),
      missing: [],
    },
    review: null,
    createdAt: "2026-09-12T00:00:00Z",
    updatedAt: "2026-09-12T00:00:00Z",
  };
}

export interface ViewerRoutesOptions {
  /** 按 draftId 返回的草稿载荷。 */
  readonly drafts: Record<string, Record<string, unknown>>;
  /** 可提供的模型（按 assetId 匹配 URL；顺序无关）。 */
  readonly models: readonly FixtureModel[];
  /** 令模型内容返回 500（验证"3D 失败仍可读文字与 PDF"）。 */
  readonly failModelContent?: boolean;
}

// ---------------------------------------------------------------------------
// 真实链路路由（不拦截草稿/模型字节；QA 回合 22 的更正）
// ---------------------------------------------------------------------------

export interface RealBackendRouting {
  /** 被阻断的非本机 URL（离线证据；必须为空）。 */
  readonly external: string[];
  /** 被本套件伪造的响应数：真实链路用例必须恒为 0（本函数不做任何 `route.fulfill`）。 */
  fulfilled: number;
  /** 命中 `/assets/{id}/content` 的路径（证据：模型字节确实取自真实端点）。 */
  readonly assetRequests: string[];
}

/**
 * 真实链路路由：**不伪造任何响应**，只把页面源站（Vite 端口）的 `/api/v1/**` 改写
 * 到一个真实后端的源站，并阻断一切非本机地址（离线证据）。
 *
 * 为什么可以这样做（`implementation.md` §T18-8 的订正，QA 回合 22 实测）：
 * `GET /items/{id}/drafts/{draftId}`（T15）与 `GET /assets/{id}/content`（T06）**均已
 * 可用且无测试构建门控**；唯一需要"测试构建 + 显式 fixture 配置"的环节是**造出**一份
 * 带 validated 模型的草稿（本机 fixture 供应商走真实流水线）。因此草稿 DTO 与模型
 * 字节可以全程来自真实后端。
 *
 * `installViewerRoutes` 的拦截**仅**用于需要合成 DTO 的注入场景（例如 T19 前无法在
 * 真实链路构造的 stale 热点、以及 500 故障注入），不作为"阅读器只能被假数据驱动"的依据。
 */
export async function installRealBackendRouting(
  page: Page,
  backendBase: string,
): Promise<RealBackendRouting> {
  const webApiPrefix = `http://127.0.0.1:${E2E_WEB_PORT}/api/v1`;
  const routing: RealBackendRouting = { external: [], fulfilled: 0, assetRequests: [] };
  await page.route("**/*", async (route: Route) => {
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
    if (/\/api\/v1\/assets\/[^/]+\/content$/.test(url.pathname)) {
      routing.assetRequests.push(url.pathname);
    }
    // 只改写源站：路径、方法、头、Cookie 全部保留（页面里的相对 `/api/v1` 请求）。
    const target = request.url().startsWith(webApiPrefix)
      ? `${backendBase}${url.pathname}${url.search}`
      : request.url();
    await route.continue({ url: target });
  });
  return routing;
}

/** 安装两条拦截：草稿读取与（仅）模型资产内容；其它请求一律放行到真实后端。 */
export async function installViewerRoutes(page: Page, options: ViewerRoutesOptions): Promise<void> {
  await page.route(/\/api\/v1\/items\/[^/]+\/drafts\/[^/?]+(\?.*)?$/, async (route: Route) => {
    const url = new URL(route.request().url());
    const draftId = url.pathname.split("/").pop() ?? "";
    const payload = options.drafts[draftId];
    if (payload === undefined) {
      await route.fulfill({
        status: 404,
        contentType: "application/json",
        body: JSON.stringify({
          error: {
            code: "NOT_FOUND",
            message: `草稿 ${draftId} 不存在`,
            details: null,
            requestId: "e2e-request-id",
          },
        }),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: { etag: '"r1"' },
      body: JSON.stringify({ data: payload }),
    });
  });

  await page.route(/\/api\/v1\/assets\/[^/]+\/content(\?.*)?$/, async (route: Route) => {
    const url = new URL(route.request().url());
    const model = options.models.find((candidate) => url.pathname.includes(candidate.assetId));
    if (model === undefined) {
      // 其它资产（原 PDF、页图）走真实后端：PDF 路径必须真实可读。
      await route.continue();
      return;
    }
    if (options.failModelContent === true) {
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({
          error: {
            code: "INTERNAL",
            message: "模型内容暂时不可用（e2e 注入）",
            details: null,
            requestId: "e2e-model-failure",
          },
        }),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "model/gltf-binary",
      body: model.bytes,
    });
  });
}

// ---------------------------------------------------------------------------
// 只读桥的读取封装（`window.__EM_VIEWER__`）
// ---------------------------------------------------------------------------

export interface ViewerStats {
  frames: number;
  modelsAlive: number;
  modelsLoaded: number;
  modelsDisposed: number;
  geometries: { created: number; disposed: number; alive: number };
  materials: { created: number; disposed: number; alive: number };
  textures: { created: number; disposed: number; alive: number };
}

export async function viewerStats(page: Page): Promise<ViewerStats> {
  return page.evaluate(() => {
    const bridge = window.__EM_VIEWER__;
    if (bridge === undefined) {
      throw new Error("__EM_VIEWER__ 不存在：3D 模块尚未加载");
    }
    return bridge.stats() as ViewerStats;
  });
}

export async function viewerFrames(page: Page): Promise<number> {
  return page.evaluate(() => window.__EM_VIEWER__?.frames() ?? 0);
}

export interface ViewerPose {
  positionLocal: number[];
  targetLocal: number[];
  upLocal: number[];
  fov: number;
}

export async function viewerPose(page: Page): Promise<ViewerPose | null> {
  return page.evaluate(() => (window.__EM_VIEWER__?.cameraPose() ?? null) as ViewerPose | null);
}

export async function viewerAnchors(
  page: Page,
): Promise<{ id: string; partId: string; local: number[]; world: number[] }[]> {
  return page.evaluate(
    () => (window.__EM_VIEWER__?.anchors() ?? []) as never,
  );
}

export interface RoundTripView {
  readonly local: readonly number[];
  readonly world: readonly number[];
  readonly back: readonly number[];
  readonly error: number;
}

export async function roundTrip(page: Page, local: readonly number[]): Promise<RoundTripView | null> {
  return page.evaluate(
    (point) => (window.__EM_VIEWER__?.roundTrip(point as [number, number, number]) ?? null) as RoundTripView | null,
    [local[0] ?? 0, local[1] ?? 0, local[2] ?? 0],
  );
}

export async function modelInfo(page: Page): Promise<{
  assetId: string;
  revisionId: string;
  sha256: string;
  triangles: number;
  textures: number;
  bounds: { min: number[]; max: number[] };
} | null> {
  return page.evaluate(() => (window.__EM_VIEWER__?.model() ?? null) as never);
}

/** 元素是否有非空白的像素内容（用于"PDF 页真的画出来了"的断言）。 */
export async function canvasHasPixels(page: Page, selector: string): Promise<boolean> {
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
      // 背景是白底：只要出现明显非白像素就说明页内容画出来了。
      if ((pixels[index] ?? 255) < 240 || (pixels[index + 1] ?? 255) < 240 || (pixels[index + 2] ?? 255) < 240) {
        return true;
      }
    }
    return false;
  }, selector);
}

declare global {
  interface Window {
    /** e2e 暂存的 WEBGL_lose_context 扩展（丢失后 getExtension 不再返回它）。 */
    __emLoseContextExtension?: WEBGL_lose_context;
  }
}

/**
 * 模拟 WebGL 上下文丢失/恢复（真实浏览器路径：`WEBGL_lose_context` 扩展，
 * 与 three 的 `forceContextLoss()` 内部同一条 API）。
 *
 * 注意：上下文丢失后 `getExtension("WEBGL_lose_context")` 返回 null，因此丢失时把
 * 扩展对象暂存在页面上，恢复时复用它（这与真实驱动复位后的恢复路径等价）。
 */
export async function loseWebglContext(page: Page): Promise<void> {
  const lost = await page.evaluate(() => {
    const canvas = document.querySelector('[data-testid="viewer-canvas"]');
    if (!(canvas instanceof HTMLCanvasElement)) {
      return "no-canvas";
    }
    const gl =
      (canvas.getContext("webgl2") as WebGL2RenderingContext | null) ??
      (canvas.getContext("webgl") as WebGLRenderingContext | null);
    const extension = gl?.getExtension("WEBGL_lose_context");
    if (extension === null || extension === undefined) {
      return "no-extension";
    }
    window.__emLoseContextExtension = extension;
    extension.loseContext();
    return "lost";
  });
  expect(lost, "强制丢失上下文必须成功（forceContextLoss 路径）").toBe("lost");
}

export async function restoreWebglContext(page: Page): Promise<void> {
  const restored = await page.evaluate(() => {
    const stashed = window.__emLoseContextExtension;
    if (stashed !== undefined) {
      stashed.restoreContext();
      return "restored";
    }
    const canvas = document.querySelector('[data-testid="viewer-canvas"]');
    if (!(canvas instanceof HTMLCanvasElement)) {
      return "no-canvas";
    }
    const gl =
      (canvas.getContext("webgl2") as WebGL2RenderingContext | null) ??
      (canvas.getContext("webgl") as WebGLRenderingContext | null);
    const extension = gl?.getExtension("WEBGL_lose_context");
    if (extension === null || extension === undefined) {
      return "no-extension";
    }
    extension.restoreContext();
    return "restored";
  });
  expect(restored, "恢复上下文必须成功").toBe("restored");
}

/**
 * 在**同一页面内**做 SPA 路由跳转（不触发文档重载）。
 *
 * 为什么需要它：React Router 的 BrowserRouter 监听 popstate（并包装 pushState），
 * 因此这样跳转走的是应用自身的客户端路由——3D 组件的卸载/换模型发生在同一个 JS
 * 上下文里，资源账本（`__EM_VIEWER__.stats()`）才能证明"旧模型被释放"。
 * 用 `page.goto` 会重建 JS 上下文，账本归零，无法证伪资源泄漏。
 */
export async function spaNavigate(page: Page, path: string): Promise<void> {
  await page.evaluate((target) => {
    window.history.pushState({}, "", target);
    window.dispatchEvent(new PopStateEvent("popstate", { state: {} }));
  }, path);
}

/** 读取当前运行时的 API 基址（harness 里偶尔需要直接打后端）。 */
export function apiBase(): string {
  return readRuntime().apiBase;
}

/** 阻断一切非本机请求并记录（离线证据：阅读器全程不访问外部网络）。 */
export async function blockExternalRequests(page: Page): Promise<string[]> {
  const external: string[] = [];
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      await route.continue();
      return;
    }
    if (url.hostname === "127.0.0.1" || url.hostname === "localhost") {
      await route.continue();
      return;
    }
    external.push(url.href);
    await route.abort();
  });
  return external;
}

/** 记录模块请求（用于"3D 懒加载"证据：three/R3F 相关模块何时被下载）。 */
export function recordModuleRequests(page: Page): string[] {
  const seen: string[] = [];
  page.on("request", (request) => {
    const url = request.url();
    if (/three|@react-three|ViewerStage|viewer\//.test(url)) {
      seen.push(url);
    }
  });
  return seen;
}

/** 当前 JS 堆用量（Chrome 专有；仅作趋势补充证据，不是断言依据）。 */
export async function heapBytes(page: Page): Promise<number | null> {
  return page.evaluate(() => {
    const memory = (performance as unknown as { memory?: { usedJSHeapSize: number } }).memory;
    return memory?.usedJSHeapSize ?? null;
  });
}
