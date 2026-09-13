/**
 * T19 端到端验收：热点校准、步骤联动、知识确认与发布（PRD 修订 2 / ui_revision 2）。
 *
 * **真实链路门禁（QA 回合 23 登记）**：release 相关用例（发布 → 阅读 → 修改草稿后
 * 复读版本）**不拦截草稿与模型字节**——草稿由**真实流水线**（测试构建后端 + 本机
 * fixture 供应商，与 T17/T18 QA 同一设施）产出，模型/PDF 字节由真实
 * `GET /assets/{id}/content` 提供；路由层只做**源站改写**（Vite 端口 → 测试后端）
 * 与非本机请求阻断。每个用例都断言 `route.fulfill` 正常响应计数为 0 与零真实外网。
 *
 * 覆盖（命令 ↔ AC 见 implementation.md §T19）：
 * - AC-052：旋转/缩放后点击位置仍正确（桥投影点 ↔ 落库 anchor 逐轴比对）、
 *   拖动旋转不建点、人工直接拾取得到 confirmed 且数值有限；
 * - AC-057：部件列表 ↔ 3D 热点双向联动；步骤前进/后退不累积错误状态；
 *   引用跳到正确 1-based 页并与页图/页文字一致；键盘路径与减少动效；
 * - AC-055/AC-056：不完整发布不可用（原因常驻）；服务端 422 明细可见；
 *   发布成功 → 阅读器可读（真实 manifest）；发布后修改草稿不改变已发布版本；
 * - AC-050 回归：校准页上的上下文丢失 → 重建状态机（QA 回合 23 登记项）。
 */

import fs from "node:fs";

import {
  expect,
  request as playwrightRequest,
  test,
  type APIRequestContext,
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
import { E2E_WEB_PORT, REPO_ROOT } from "./runtime";

test.describe.configure({ timeout: 300_000 });

const WEB_BASE = `http://127.0.0.1:${E2E_WEB_PORT}`;

let fixture: LocalFixture;
let backend: TestBackend;

interface HotspotView {
  readonly id: string;
  readonly partId: string;
  readonly status: string;
  readonly anchor: { readonly positionLocal: number[] } | null;
}

interface RealDraft {
  readonly itemId: string;
  readonly jobId: string;
  readonly draftId: string;
  readonly model: { readonly revisionId: string; readonly sha256: string; readonly assetId: string };
  readonly parts: { id: string; name: string }[];
  readonly steps: { id: string; title: string }[];
  readonly specs: { id: string; label: string }[];
  readonly hotspots: HotspotView[];
  readonly etag: string;
}

// ---------------------------------------------------------------------------
// 真实链路路由：只改写源站 + 阻断外网（不做任何正常响应伪造）
// ---------------------------------------------------------------------------

interface Routing {
  external: string[];
  assetContent: string[];
  /** 被测试伪造的 2xx 响应数（必须恒为 0）。 */
  fulfilledOk: number;
}

/**
 * 真实链路路由：只做源站改写 + 外网阻断。**任何 `route.fulfill` 都会被计数**
 * （用一个包装对象拦截 `fulfill`：正常响应伪造数为 0 是断言而不是口号）。
 */
async function installRealRouting(page: Page): Promise<Routing> {
  const routing: Routing = { external: [], assetContent: [], fulfilledOk: 0 };
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
    let target = request.url();
    if (url.href.startsWith(`${WEB_BASE}/api/v1`)) {
      target = `${backend.base}${url.pathname}${url.search}`;
    }
    await route.continue({ url: target });
  });
  return routing;
}

async function loginAndOpen(page: Page, target: string): Promise<Routing> {
  const routing = await installRealRouting(page);
  await page.goto(`${WEB_BASE}/`);
  await loginViaUi(page, WEB_BASE, BACKEND_PASSWORD);
  await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();
  await page.goto(WEB_BASE + target);
  return routing;
}

// ---------------------------------------------------------------------------
// 真实后端 API 辅助（独立会话；用于造数、并发干扰与服务器事实核对）
// ---------------------------------------------------------------------------

async function freshApi(): Promise<{ context: APIRequestContext; csrf: string }> {
  const context = await playwrightRequest.newContext();
  const response = await context.post(`${backend.base}/api/v1/auth/login`, {
    data: { password: BACKEND_PASSWORD },
  });
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: { csrfToken: string } };
  return { context, csrf: body.data.csrfToken };
}

/**
 * 造一份真实草稿：用**独立会话**做造数（`seedJob` 内部会再次登录，会话 cookie 随之
 * 更换），完成后开一个**新会话**返回——CSRF token 与会话 cookie 绑定，必须取"当前
 * cookie jar 那一次会话"的 token（否则 403 CSRF_REJECTED；T15/T17 已记录该陷阱）。
 */
async function seedDraftWithSession(tag: string): Promise<{
  draft: RealDraft;
  api: { context: APIRequestContext; csrf: string };
}> {
  const seedContext = await playwrightRequest.newContext();
  const seeded = await seedJob(seedContext, backend, tag);
  const draft = await fetchRealDraft(seedContext, seeded, tag);
  await seedContext.dispose();
  const api = await freshApi();
  return { draft, api };
}

async function waitForSuccess(context: APIRequestContext, jobId: string): Promise<string> {
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    const response = await context.get(`${backend.base}/api/v1/jobs/${jobId}`);
    expect(response.status(), await response.text()).toBe(200);
    const body = (await response.json()) as { data: { status: string; draftId: string | null } };
    if (body.data.status === "succeeded" && body.data.draftId !== null) {
      return body.data.draftId;
    }
    if (body.data.status === "failed" || body.data.status === "cancelled") {
      throw new Error(`任务终态为 ${body.data.status}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error("等待任务成功超时");
}

async function fetchRealDraft(
  context: APIRequestContext,
  seeded: SeededJob,
  tag: string,
): Promise<RealDraft> {
  const draftId = await waitForSuccess(context, seeded.jobId);
  const response = await context.get(
    `${backend.base}/api/v1/items/${seeded.itemId}/drafts/${draftId}`,
  );
  expect(response.status(), await response.text()).toBe(200);
  const etag = response.headers()["etag"] ?? null;
  expect(etag, `${tag} 草稿必须带 ETag`).not.toBeNull();
  const body = (await response.json()) as { data: Record<string, unknown> };
  const knowledge = body.data.knowledge as Record<string, unknown>;
  const model = knowledge.model as Record<string, unknown>;
  expect(model.validationState, `${tag} 模型必须 validated`).toBe("validated");
  const merged = knowledge.knowledge as Record<string, unknown>;
  const parts = (merged.parts as { id: string; name: string }[]).map((part) => ({
    id: part.id,
    name: part.name,
  }));
  const steps = (merged.steps as { id: string; title: string }[]).map((step) => ({
    id: step.id,
    title: step.title,
  }));
  const specs = (merged.specs as { id: string; label: string }[]).map((spec) => ({
    id: spec.id,
    label: spec.label,
  }));
  const hotspots = ((knowledge.hotspots as HotspotView[]) ?? []).map((hotspot) => ({
    id: hotspot.id,
    partId: hotspot.partId,
    status: hotspot.status,
    anchor: hotspot.anchor ?? null,
  }));
  return {
    itemId: seeded.itemId,
    jobId: seeded.jobId,
    draftId,
    model: {
      revisionId: String(model.revisionId),
      sha256: String(model.sha256),
      assetId: String(model.assetId),
    },
    parts,
    steps,
    specs,
    hotspots,
    etag: etag ?? "",
  };
}

async function getDraftApi(
  context: APIRequestContext,
  draft: RealDraft,
): Promise<{ etag: string; json: Record<string, unknown>; hotspots: HotspotView[] }> {
  const response = await context.get(
    `${backend.base}/api/v1/items/${draft.itemId}/drafts/${draft.draftId}`,
  );
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: Record<string, unknown> };
  const knowledge = body.data.knowledge as Record<string, unknown>;
  return {
    etag: response.headers()["etag"] ?? "",
    json: body.data,
    hotspots: (knowledge.hotspots as HotspotView[]) ?? [],
  };
}

/** 通过独立会话读取草稿热点（服务器事实；不依赖页面缓存）。 */
async function hotspotsViaApi(draft: RealDraft): Promise<HotspotView[]> {
  const api = await freshApi();
  const view = await getDraftApi(api.context, draft);
  await api.context.dispose();
  return view.hotspots;
}

async function patchDraftApi(
  context: APIRequestContext,
  csrf: string,
  draft: RealDraft,
  body: unknown,
  ifMatch?: string,
): Promise<void> {
  const response = await context.patch(
    `${backend.base}/api/v1/items/${draft.itemId}/drafts/${draft.draftId}`,
    {
      headers: {
        "x-csrf-token": csrf,
        ...(ifMatch !== undefined ? { "if-match": ifMatch } : {}),
      },
      data: body as Record<string, unknown>,
    },
  );
  expect(response.status(), await response.text()).toBe(200);
}

/** 把草稿推到"可发布"（全部实体确认 + 每个部件一个 confirmed 热点 + modelReview 双声明）。 */
async function makePublishable(
  context: APIRequestContext,
  csrf: string,
  draft: RealDraft,
): Promise<void> {
  const entities: Record<string, unknown> = {};
  for (const part of draft.parts) {
    entities[part.id] = { reviewStatus: "confirmed" };
  }
  for (const step of draft.steps) {
    entities[step.id] = { reviewStatus: "confirmed" };
  }
  for (const spec of draft.specs) {
    entities[spec.id] = { reviewStatus: "confirmed" };
  }
  const current = await getDraftApi(context, draft);
  await patchDraftApi(context, csrf, draft, { entities }, current.etag);
  const afterEntities = await getDraftApi(context, draft);
  // 锚点放在**立方体正面**（本地 +Z 面）上：默认相机从 +Z 看向原点，
  // 标记因此贴在可见表面上（不是模型内部的点——内部的锚点虽然数值合法，
  // 但在阅读器里会被模型挡住，截图/人工走查会误判为"没有热点"）。
  const upsert = draft.parts.map((part, index) => ({
    partId: part.id,
    status: "confirmed",
    anchor: {
      modelRevisionId: draft.model.revisionId,
      modelSha256: draft.model.sha256,
      positionLocal: [0.4 - index * 0.2, 0.3, 1.0],
    },
  }));
  await patchDraftApi(context, csrf, draft, { hotspots: { upsert } }, afterEntities.etag);
  const afterHotspots = await getDraftApi(context, draft);
  await patchDraftApi(
    context,
    csrf,
    draft,
    { modelReview: { loaded: true, userConfirmed: true } },
    afterHotspots.etag,
  );
}

const reviewPath = (draft: RealDraft): string =>
  `/items/${draft.itemId}/drafts/${draft.draftId}/review`;
const readerPath = (draft: RealDraft, releaseId: string): string =>
  `/items/${draft.itemId}/releases/${releaseId}`;

// ---------------------------------------------------------------------------
// 页面观察辅助（只读桥 + 真实鼠标事件）
// ---------------------------------------------------------------------------

async function waitForRealViewer(page: Page): Promise<void> {
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
  expect(projection, "桥必须提供 project()（局部点 → 屏幕像素）").not.toBeNull();
  return { screen: projection?.screen ?? [0, 0], visible: projection?.visible ?? false };
}

/** 在 canvas 内的相对像素位置点击（真实鼠标事件；拖动判定同样生效）。 */
async function clickCanvasAt(page: Page, screen: [number, number]): Promise<void> {
  const box = await page.getByTestId("viewer-canvas").boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move((box?.x ?? 0) + screen[0], (box?.y ?? 0) + screen[1]);
  await page.mouse.down();
  await page.mouse.up();
}

async function dragCanvas(page: Page, dx: number, dy: number): Promise<void> {
  const box = await page.getByTestId("viewer-canvas").boundingBox();
  const cx = (box?.x ?? 0) + (box?.width ?? 0) / 2;
  const cy = (box?.y ?? 0) + (box?.height ?? 0) / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + dx, cy + dy, { steps: 12 });
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

async function canvasHasPixels(page: Page): Promise<boolean> {
  return page.evaluate(() => {
    const canvas = document.querySelector('[data-testid="original-canvas"]');
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
  });
}

async function spaNavigate(page: Page, target: string): Promise<void> {
  await page.evaluate((next) => {
    window.history.pushState({}, "", next);
    window.dispatchEvent(new PopStateEvent("popstate", { state: {} }));
  }, target);
}

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
  backend = new TestBackend("t19");
  await backend.start(fixture);
  fs.mkdirSync(`${REPO_ROOT}/artifacts/web-mvp/t19-rd`, { recursive: true });
});

test.afterAll(async () => {
  await backend?.cleanup(fixture);
});

// ---------------------------------------------------------------------------
// 1) 拾取绑定：旋转/缩放后点击位置仍正确；拖动旋转不建点（AC-052）
// ---------------------------------------------------------------------------

test("T19-1 拾取绑定：旋转/缩放后点击位置正确；拖动旋转不建点（真实链路）", async ({ page }) => {
  const { draft } = await seedDraftWithSession("T19-1 拾取");

  const routing = await loginAndOpen(page, reviewPath(draft));
  await waitForRealViewer(page);
  expect(routing.fulfilledOk, "真实链路用例不得伪造正常响应").toBe(0);

  const part = draft.parts[0];
  expect(part, "真实知识必须至少一个部件").toBeTruthy();
  const partId = part?.id ?? "";
  const partRow = page.getByTestId(`part-row-${partId}`);
  await expect(partRow).toContainText(part?.name ?? "");
  await expect(page.getByTestId(`part-hotspot-${partId}`)).toContainText("未绑定");

  // 先旋转 + 缩放：之后点击必须仍落在"模型表面上的同一点"。
  await dragCanvas(page, 140, 60);
  await page.mouse.wheel(0, -360);
  await expect.poll(async () => viewerPose(page), { timeout: 15_000 }).not.toBeNull();

  // 进入拾取模式（模式互斥）。
  await partRow.getByRole("button", { name: "绑定热点" }).click();
  await expect(page.getByTestId("pick-mode-active")).toBeVisible();

  // 拖动旋转不得建点（UI-048）：热点数量不变、相机改变、仍在拾取模式。
  const poseBeforeDrag = await viewerPose(page);
  await dragCanvas(page, 130, 90);
  await page.waitForTimeout(400);
  expect((await hotspotsViaApi(draft)).length, "拖动旋转后热点数量必须不变").toBe(0);
  const poseAfterDrag = await viewerPose(page);
  const moved =
    Math.abs((poseAfterDrag?.positionLocal[0] ?? 0) - (poseBeforeDrag?.positionLocal[0] ?? 0)) +
    Math.abs((poseAfterDrag?.positionLocal[1] ?? 0) - (poseBeforeDrag?.positionLocal[1] ?? 0));
  expect(moved, "拖动必须真的旋转了相机").toBeGreaterThan(0);
  await expect(page.getByTestId("pick-mode-active")).toBeVisible();

  // 靶点 = **朝向相机的表面点**（本测试模型是单位立方体：取朝向相机的面中心）。
  // 由只读桥的 bounds + cameraPose 独立推导（不读应用的计算结果）：
  // 面朝相机 ⇔ 面法线的局部方向与"相机 - 目标"方向同向（asset-root 为均匀缩放，符号不变）。
  const bounds = await page.evaluate(() => window.__EM_VIEWER__?.localBounds() ?? null);
  expect(bounds, "桥必须提供 localBounds()").not.toBeNull();
  const pose = await viewerPose(page);
  expect(pose, "相机位姿必须可读").not.toBeNull();
  const target: [number, number, number] = (() => {
    const center = [0, 1, 2].map(
      (axis) => ((bounds?.min[axis] ?? 0) + (bounds?.max[axis] ?? 0)) / 2,
    ) as [number, number, number];
    const direction = [0, 1, 2].map(
      (axis) => (pose?.positionLocal[axis] ?? 0) - (pose?.targetLocal[axis] ?? 0),
    ) as [number, number, number];
    let best: { point: [number, number, number]; score: number } | null = null;
    for (let axis = 0; axis < 3; axis += 1) {
      for (const sign of [1, -1] as const) {
        const point: [number, number, number] = [...center] as [number, number, number];
        point[axis] = sign > 0 ? (bounds?.max[axis] ?? 0) : (bounds?.min[axis] ?? 0);
        const normal = [0, 0, 0];
        normal[axis] = sign;
        const score =
          (normal[0] ?? 0) * (direction[0] ?? 0) +
          (normal[1] ?? 0) * (direction[1] ?? 0) +
          (normal[2] ?? 0) * (direction[2] ?? 0);
        // 朝向相机的面（正分）中取最正的一个：保证射线先命中它。
        const current = best as { point: [number, number, number]; score: number } | null;
        if (score > 0 && (current === null || score > current.score)) {
          best = { point, score };
        }
      }
    }
    const chosen = best as { point: [number, number, number]; score: number } | null;
    expect(chosen, "必须能推出一个朝向相机的表面点").not.toBeNull();
    return chosen?.point ?? [0, 0, 0];
  })();
  const projection = await projectLocal(page, target);
  expect(projection.visible, "靶点必须在视口内（旋转/缩放后）").toBe(true);
  await clickCanvasAt(page, projection.screen);
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
  for (let axis = 0; axis < 3; axis += 1) {
    expect(
      Math.abs(((picked ?? [])[axis] ?? Number.NaN) - (target[axis] ?? Number.NaN)),
      `命中点应等于朝向相机的面中心（轴 ${axis}；旋转/缩放后仍正确）`,
    ).toBeLessThan(0.05);
  }
  // 独立一致性：命中点投影回屏幕必须落在点击的像素上（≤2px）。
  const reprojected = await projectLocal(page, [
    (picked ?? [0])[0] ?? 0,
    (picked ?? [0, 0])[1] ?? 0,
    (picked ?? [0, 0, 0])[2] ?? 0,
  ]);
  expect(Math.abs(reprojected.screen[0] - projection.screen[0])).toBeLessThan(2);
  expect(Math.abs(reprojected.screen[1] - projection.screen[1])).toBeLessThan(2);

  // 落库：confirmed + 有限数值 + anchor 与拾取点逐轴一致。
  await expect.poll(async () => (await hotspotsViaApi(draft)).length, { timeout: 15_000 }).toBe(1);
  const stored = (await hotspotsViaApi(draft))[0];
  expect(stored?.status, `落库热点必须是 confirmed：${JSON.stringify(stored)}`).toBe("confirmed");
  const anchorLocal = stored?.anchor?.positionLocal ?? [];
  expect(anchorLocal).toHaveLength(3);
  for (const value of anchorLocal) {
    expect(Number.isFinite(value)).toBe(true);
  }
  for (let axis = 0; axis < 3; axis += 1) {
    expect(
      Math.abs((anchorLocal[axis] ?? Number.NaN) - ((picked ?? [])[axis] ?? Number.NaN)),
      `落库 anchor 必须等于拾取点（轴 ${axis}）`,
    ).toBeLessThan(1e-9);
  }
  await expect(page.getByTestId(`part-hotspot-${partId}`)).toContainText("热点已确认");
  await captureTo("t19-rd", page, "01-pick-after-rotate");

  expect(routing.fulfilledOk, "全程不得伪造正常响应").toBe(0);
  expect(routing.external, "全程零真实外网").toEqual([]);
});

// ---------------------------------------------------------------------------
// 2) 部件 ↔ 热点双向联动；步骤导航不累积错误状态（AC-057）
// ---------------------------------------------------------------------------

test("T19-2 部件列表 ↔ 3D 热点双向联动；步骤前进/后退不累积错误状态", async ({ page }) => {
  const { draft, api } = await seedDraftWithSession("T19-2 联动");
  await makePublishable(api.context, api.csrf, draft);
  await api.context.dispose();

  const routing = await loginAndOpen(page, reviewPath(draft));
  await waitForRealViewer(page);

  const partId = draft.parts[0]?.id ?? "";
  const partRow = page.getByTestId(`part-row-${partId}`);
  const partButton = partRow.getByRole("button").first();

  // 侧栏 → 3D：选中部件后该行是当前项（3D 同步居中/高亮由同一状态驱动）。
  await partButton.click();
  await expect(partButton).toHaveAttribute("aria-current", "true");

  // 3D → 侧栏：先选中别的部件，再点击热点标记的投影位置，必须选回原部件。
  await page.getByTestId("parts-panel").click();
  const stored = (await hotspotsViaApi(draft)).find((hotspot) => hotspot.status === "confirmed");
  expect(stored, "造数必须留下 confirmed 热点").toBeTruthy();
  const anchorLocal = stored?.anchor?.positionLocal ?? [0, 0, 0];
  const projection = await projectLocal(page, [
    anchorLocal[0] ?? 0,
    anchorLocal[1] ?? 0,
    anchorLocal[2] ?? 0,
  ]);
  await clickCanvasAt(page, projection.screen);
  await expect
    .poll(async () =>
      page.evaluate((id) => {
        const row = document.querySelector(`[data-testid="part-row-${id}"]`);
        return row?.querySelector('[aria-current="true"]') !== null;
      }, partId),
    )
    .toBe(true);
  await captureTo("t19-rd", page, "02-two-way-linkage");

  // 步骤导航：前进/后退后状态一致，不累积错误状态。
  const stepCount = draft.steps.length;
  expect(stepCount).toBeGreaterThan(0);
  const stepPosition = page.getByTestId("step-position");
  await expect(stepPosition).toContainText(`第 1 / ${stepCount} 步`);
  if (stepCount > 1) {
    await page.getByRole("button", { name: "下一步" }).click();
    await expect(stepPosition).toContainText(`第 2 / ${stepCount} 步`);
    await page.getByRole("button", { name: "上一步" }).click();
    await expect(stepPosition).toContainText(`第 1 / ${stepCount} 步`);
  }
  await expect(page.getByRole("button", { name: "上一步" })).toBeDisabled();
  await expect(page.getByTestId("step-position")).toContainText(`第 1 / ${stepCount} 步`);
  const bodyText = (await page.locator("body").textContent()) ?? "";
  expect(bodyText).not.toContain("草稿读取失败");
  expect(bodyText).not.toContain("已被其他操作更新");
  expect(bodyText).not.toContain("网络连接异常");
  // 原文页签必须是 1-based；总页数在 PDF 加载完成后才出现（"第 1 / ? 页"是中间态）。
  await expect
    .poll(async () => (await page.getByTestId("original-page-label").textContent()) ?? "")
    .toMatch(/第 \d+ \/ \d+ 页/);

  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
});

// ---------------------------------------------------------------------------
// 3) 引用跳到正确 1-based 页，并与页图/页文字一致（AC-057）
// ---------------------------------------------------------------------------

test("T19-3 引用跳到正确 1-based 页（页图与页文字一致）", async ({ page }) => {
  const { draft } = await seedDraftWithSession("T19-3 原文");

  const routing = await loginAndOpen(page, reviewPath(draft));
  await waitForRealViewer(page);

  // 步骤的出处按钮跳到真实页码（1-based；0 会显示成"第 0 页"或跳到末页）。
  const pageButton = page
    .getByTestId("steps-panel")
    .getByRole("button", { name: /^第 \d+ 页$/ })
    .first();
  await expect(pageButton).toBeVisible();
  const evidencePage = Number(/第 (\d+) 页/.exec((await pageButton.textContent()) ?? "")?.[1] ?? "0");
  expect(evidencePage, "出处页码必须 ≥1（1-based）").toBeGreaterThanOrEqual(1);
  await pageButton.click();
  await expect(page.getByTestId("original-page-label")).toContainText(`第 ${evidencePage} /`);
  await expect.poll(async () => await canvasHasPixels(page), { timeout: 30_000 }).toBe(true);

  // 页文字与页号一致：真实 PDF 文字层第 N 页含 "Page N of 2"。
  const details = page.getByTestId("original-text");
  await expect(details).toBeVisible();
  await details.locator("summary").click();
  await expect(details.locator("pre")).toContainText(`Page ${evidencePage} of 2`);

  // 翻页同样映射到正确页（不跳错页、不做 0-based 偏移）。
  await page.getByRole("button", { name: "下一页" }).click();
  const nextPage = evidencePage + 1;
  await expect(page.getByTestId("original-page-label")).toContainText(`第 ${nextPage} /`);
  await expect(details.locator("pre")).toContainText(`Page ${nextPage} of 2`);
  await captureTo("t19-rd", page, "03-evidence-page-jump");

  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
});

// ---------------------------------------------------------------------------
// 4) 412 冲突可恢复（草稿 PATCH；UI-008/UI-056）
// ---------------------------------------------------------------------------

test("T19-4 并发冲突 412：提示当前 revision 且刷新后恢复（真实链路）", async ({ page }) => {
  const { draft } = await seedDraftWithSession("T19-4 冲突草稿");

  const routing = await loginAndOpen(page, reviewPath(draft));
  await waitForRealViewer(page);

  // 另一个会话先改草稿（浏览器持有的 ETag 因此过期）。
  const other = await freshApi();
  const current = await getDraftApi(other.context, draft);
  await patchDraftApi(
    other.context,
    other.csrf,
    draft,
    { entities: { [draft.parts[0]?.id ?? ""]: { reviewStatus: "confirmed" } } },
    current.etag,
  );
  await other.context.dispose();

  // 浏览器用过期 ETag 提交 → 412 → 显示当前 revision 与刷新入口（不自动覆盖）。
  await page
    .getByTestId(`knowledge-${draft.parts[0]?.id ?? ""}`)
    .getByRole("button", { name: "确认事实" })
    .click();
  await expect(page.getByTestId("conflict-panel")).toBeVisible();
  await expect(page.getByTestId("conflict-panel")).toContainText("r2");
  await captureTo("t19-rd", page, "04-conflict-412");

  // 刷新后按最新 revision 重新载入（外部修改可见），可继续操作。
  await page.getByTestId("conflict-panel").getByRole("button", { name: "刷新草稿" }).click();
  await expect(page.getByTestId("conflict-panel")).toBeHidden();
  await expect(page.getByTestId(`knowledge-status-${draft.parts[0]?.id ?? ""}`)).toContainText(
    "已确认",
  );
  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
});

// ---------------------------------------------------------------------------
// 5) 不完整发布：原因常驻、按钮禁用；服务端 422 明细可见（UI-054/AC-056）
// ---------------------------------------------------------------------------

test("T19-5 不完整发布不可用（原因常驻）；服务端 422 明细可见", async ({ page }) => {
  const { draft } = await seedDraftWithSession("T19-5 不变量");

  const routing = await loginAndOpen(page, reviewPath(draft));
  await waitForRealViewer(page);

  // 未确认知识/缺热点/模型复核未声明：按钮禁用 + 原因列表常驻（UI-054）。
  const publishButton = page.getByRole("button", { name: "发布（生成不可变版本）" });
  await expect(publishButton).toBeDisabled();
  await expect(page.getByTestId("publish-pending")).toBeVisible();
  await expect(page.getByTestId("publish-pending")).toContainText("尚未确认或修订");
  await expect(page.getByTestId("publish-checklist")).toContainText("缺 confirmed 热点部件");
  await captureTo("t19-rd", page, "05-publish-pending");

  // 服务端 422 明细：先让本地清单满足，再篡改一条出处页码（服务端独有校验），
  // 界面点发布 → 服务端逐条列出不满足项（不伪造 HTTP 响应，只改数据库事实）。
  const prepare = await freshApi();
  await makePublishable(prepare.context, prepare.csrf, draft);
  await prepare.context.dispose();
  await page.getByTestId("refresh-draft").click();
  await expect(publishButton).toBeEnabled({ timeout: 20_000 });

  const { DatabaseSync } = await import("node:sqlite");
  const db = new DatabaseSync(`${backend.dataDir}/manual.sqlite3`);
  try {
    const row = db
      .prepare("SELECT knowledge_json FROM manual_drafts WHERE id = ?")
      .get(draft.draftId) as { knowledge_json: string };
    const knowledge = JSON.parse(row.knowledge_json) as {
      knowledge: { parts: { evidence: { pageNumber: number }[] }[] };
    };
    knowledge.knowledge.parts[0]!.evidence[0]!.pageNumber = 99;
    db.prepare("UPDATE manual_drafts SET knowledge_json = ? WHERE id = ?").run(
      JSON.stringify(knowledge),
      draft.draftId,
    );
  } finally {
    db.close();
  }
  await page.getByTestId("refresh-draft").click();
  await publishButton.click();
  await expect(page.getByTestId("publish-issues")).toBeVisible();
  await expect(page.getByTestId("publish-issues")).toContainText("evidencePageMissing");
  await expect(page.getByTestId("publish-issues")).toContainText("不在本次提取范围");
  await captureTo("t19-rd", page, "06-publish-422-issues");

  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
});

// ---------------------------------------------------------------------------
// 6) 发布 → 真实阅读器可读；发布后修改草稿不改变已发布版本（AC-055/AC-057）
// ---------------------------------------------------------------------------

test("T19-6 发布（真实链路）→ 阅读器可读；修改草稿不改变已发布版本", async ({ page }) => {
  const { draft, api } = await seedDraftWithSession("T19-6 发布");
  await makePublishable(api.context, api.csrf, draft);

  const routing = await loginAndOpen(page, reviewPath(draft));
  await waitForRealViewer(page);

  const publishButton = page.getByRole("button", { name: "发布（生成不可变版本）" });
  await expect(publishButton).toBeEnabled({ timeout: 20_000 });
  await publishButton.click();
  await expect(page.getByTestId("publish-success")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("publish-success")).toContainText("不可再修改");
  const successText = (await page.getByTestId("publish-success").textContent()) ?? "";
  const releaseId = /已发布版本 ([0-9a-f-]+)/.exec(successText)?.[1] ?? "";
  expect(releaseId, `发布成功必须给出 release id：${successText}`).not.toBe("");
  await expect(page.getByRole("link", { name: "打开阅读器" })).toBeVisible();
  await captureTo("t19-rd", page, "07-publish-success");

  // 阅读器（真实 release + 真实模型/PDF 字节）。fullPage 截图后整页可能重挂载
  // （QA 回合 23 的 N6 瞬态：1280 断点边界 + 滚动条），点击目标可能在重挂载窗口里
  // 短暂消失，因此截图后用客户端路由前进（等价于点击链接，但不依赖元素存活）。
  await spaNavigate(page, readerPath(draft, releaseId));
  await expect(page.getByRole("heading", { name: "已发布说明书" })).toBeVisible();
  await waitForRealViewer(page);
  const releaseContextBefore = (await page.getByTestId("release-context").textContent()) ?? "";
  expect(releaseContextBefore).toContain(releaseId);
  expect(releaseContextBefore).toContain("不可变");
  // 真实字节：阅读器里的模型就是发布版 manifest 指向的本地资产（同一 assetId/sha256），
  // 且资产内容确实经源站改写打到真实后端（`/api/v1/assets/**` 计数 > 0）。
  const readerModel = await page.evaluate(() => window.__EM_VIEWER__?.model() ?? null);
  expect(readerModel?.assetId, "阅读器必须加载发布版模型资产").toBe(draft.model.assetId);
  expect(readerModel?.sha256).toBe(draft.model.sha256);
  expect(
    routing.assetContent.length,
    "必须真的经 GET /assets/{id}/content 拿模型/PDF 字节",
  ).toBeGreaterThan(0);
  await expect(page.getByTestId("parts-list")).toContainText(draft.parts[0]?.name ?? "");
  await expect(page.getByTestId("steps-list")).toContainText(draft.steps[0]?.title ?? "");
  await expect.poll(async () => await canvasHasPixels(page), { timeout: 30_000 }).toBe(true);
  await captureTo("t19-rd", page, "08-release-reader");

  // 发布后修改草稿（改回 needs_review）→ release 内容与哈希不变。
  const current = await getDraftApi(api.context, draft);
  await patchDraftApi(
    api.context,
    api.csrf,
    draft,
    { entities: { [draft.parts[0]?.id ?? ""]: { reviewStatus: "needs_review" } } },
    current.etag,
  );
  await api.context.dispose();

  await spaNavigate(page, readerPath(draft, releaseId));
  await expect(page.getByTestId("release-context")).toBeVisible();
  const releaseContextAfter = (await page.getByTestId("release-context").textContent()) ?? "";
  expect(releaseContextAfter, "发布后修改草稿不得改变已发布版本").toBe(releaseContextBefore);
  // 阅读器里没有"编辑已发布内容"入口。
  const readerText = (await page.locator("body").textContent()) ?? "";
  expect(readerText).not.toContain("发布（生成不可变版本）");

  // 版本列表也指向同一 release。
  await spaNavigate(page, `/items/${draft.itemId}/releases`);
  await expect(page.getByTestId("release-list")).toContainText(releaseId);

  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
});

// ---------------------------------------------------------------------------
// 7) 窄屏：几何校准禁用并提示；文字确认与发布可用（§6.1.4 / AC-060）
// ---------------------------------------------------------------------------

test("T19-7 窄屏：几何校准禁用并解释；文字确认与发布仍可用", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 900 });
  const { draft } = await seedDraftWithSession("T19-7 窄屏");

  const routing = await loginAndOpen(page, reviewPath(draft));
  await waitForRealViewer(page);

  // 几何校准禁用 + 解释性提示（不能坏掉无解释）。
  await expect(page.getByTestId("pick-mode-toggle")).toBeDisabled();
  await expect(page.getByTestId("pick-hint")).toContainText("≥768px");
  await expect(page.getByTestId("pick-hint")).toContainText("只读热点");
  // 部件列表是热点的文字替代路径：窄屏仍可读、可选。
  const partId = draft.parts[0]?.id ?? "";
  await page.getByRole("button", { name: "部件与热点" }).click();
  const partButton = page.getByTestId(`part-row-${partId}`).getByRole("button").first();
  await expect(partButton).toContainText(draft.parts[0]?.name ?? "");
  await partButton.click();
  await expect(partButton).toHaveAttribute("aria-current", "true");
  // 抽屉同一时刻只开一个（覆盖层）：先关闭部件抽屉（Esc 归还焦点）。
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("parts-panel")).toBeHidden();

  // 文字确认可用（抽屉内）。
  await page.getByRole("button", { name: "步骤与原文" }).click();
  const confirmButton = page
    .getByTestId(`knowledge-${partId}`)
    .getByRole("button", { name: "确认事实" });
  await expect(confirmButton).toBeEnabled();
  await confirmButton.click();
  await expect(page.getByTestId(`knowledge-status-${partId}`)).toContainText("已确认");
  // 发布入口保留：原因常驻；缺 confirmed 热点时禁用并说明（窄屏不是旁路）。
  await expect(page.getByTestId("publish-panel")).toBeVisible();
  await expect(page.getByTestId("narrow-publish-note")).toContainText("≥768px");
  await expect(page.getByTestId("publish-checklist")).toContainText("缺 confirmed 热点部件");
  await expect(page.getByRole("button", { name: "发布（生成不可变版本）" })).toBeDisabled();
  await captureTo("t19-rd", page, "09-narrow-calibration-disabled");

  // 拉宽后无需刷新即可恢复可用（禁用判定以视口宽度为准）。
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(page.getByTestId("pick-mode-toggle")).toBeEnabled({ timeout: 10_000 });

  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
});

// ---------------------------------------------------------------------------
// 8) 键盘路径、减少动效与上下文重建状态机（QA 回合 23 登记项）
// ---------------------------------------------------------------------------

test("T19-8 键盘路径与减少动效；上下文丢失 → 重建状态机（回归断言集）", async ({ page }) => {
  const { draft } = await seedDraftWithSession("T19-8 键盘");

  await page.emulateMedia({ reducedMotion: "reduce" });
  const routing = await loginAndOpen(page, reviewPath(draft));
  await waitForRealViewer(page);

  // 键盘：部件按钮可聚焦、有可见 focus 环、回车选中。
  const partButton = page
    .getByTestId(`part-row-${draft.parts[0]?.id ?? ""}`)
    .getByRole("button")
    .first();
  await partButton.focus();
  const outline = await partButton.evaluate((element) => getComputedStyle(element).outlineStyle);
  expect(outline, "键盘焦点必须有可见 focus 样式").not.toBe("none");
  await page.keyboard.press("Enter");
  await expect(partButton).toHaveAttribute("aria-current", "true");
  // 减少动效下没有自动相机动画：位姿稳定。
  const poseA = await viewerPose(page);
  await page.waitForTimeout(400);
  const poseB = await viewerPose(page);
  expect(poseA?.positionLocal[0] ?? Number.NaN).toBeCloseTo(poseB?.positionLocal[0] ?? Number.NaN, 5);

  // 上下文丢失 → 状态可观察、交互禁用、给出「立即重建」。
  const lost = await page.evaluate(() => {
    const canvas = document.querySelector('[data-testid="viewer-canvas"]');
    if (!(canvas instanceof HTMLCanvasElement)) {
      return "no-canvas";
    }
    const gl = canvas.getContext("webgl2") ?? canvas.getContext("webgl");
    const extension = gl?.getExtension("WEBGL_lose_context") ?? null;
    if (extension === null) {
      return "no-extension";
    }
    extension.loseContext();
    return "lost";
  });
  expect(lost).toBe("lost");
  await expect(page.getByTestId("viewer-status")).toContainText("3D 显示已中断");
  await expect(page.getByRole("button", { name: "复位视角" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "立即重建" })).toBeVisible();

  // 「立即重建」→ 状态回到可用 + 键盘等价控件恢复（BUG-007 断言集纳入回归）。
  await page.getByRole("button", { name: "立即重建" }).click();
  await expect
    .poll(async () => page.evaluate(() => window.__EM_VIEWER__?.stats().modelsAlive ?? 0), {
      timeout: 60_000,
    })
    .toBe(1);
  await expect(page.getByTestId("viewer-status")).toContainText("模型已加载", { timeout: 15_000 });
  await expect(page.getByRole("button", { name: "复位视角" })).toBeEnabled();
  // 8.5 秒后仍正常（覆盖 timer 未清除的旧症状）。
  await page.waitForTimeout(8_500);
  await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
  await expect(page.getByRole("button", { name: "复位视角" })).toBeEnabled();
  await captureTo("t19-rd", page, "10-rebuild-state-machine");

  expect(routing.fulfilledOk).toBe(0);
  expect(routing.external).toEqual([]);
});
