/**
 * QA 回合 23 · BUG-007 复验的独立语义用例（PRD 修订 2 / ui_revision 2）。
 *
 * 不修改回合 22 的 `qa-t18-independent.spec.ts`（复验纪律），本文件是 QA 新增的
 * **独立**现场：驱动一次「上下文丢失 → 立即重建」，逐项核对任务卡要求的恢复语义：
 *   1. 状态行回到可用态（「模型已加载」）；
 *   2. 「复位视角」「适配模型」恢复 enabled；
 *   3. **8.6 秒后**仍不得误报「浏览器 3D 上下文不可用」（BUG-007 的第二个症状）；
 *   4. 重建后拖动仍可旋转（独立 draw 计数继续增长，不依赖面板文案）；
 *   5. 「立即重建」入口消失（不再显示）；
 *   6. 相机位姿策略：重建后套用重建前位姿（1e-3）、「复位视角」回默认取景（1e-3）。
 *
 * 数据来源与观察手法（与回合 22 同一纪律）：
 * - 真实流水线（测试构建后端 + 本机 fixture 供应商）产出真实草稿与真实资产字节；
 * - 路由只做源站改写，**不伪造任何正常响应**（`fulfilledOk` 断言为 0），全程零真实外网；
 * - 渲染证据用**原型层** WebGL draw 计数器（不是面板文案、不是 RD 的账本）。
 *
 * 视口固定 1440×900：1280×720（默认）正好压在 `wide ≥1280px` 断点上，见
 * `qa-t18-r23-layout-boundary.spec.ts` 的独立调查；本用例只关心 BUG-007 语义。
 */

import fs from "node:fs";
import path from "node:path";

import { expect, request as playwrightRequest, test, type Page, type Route } from "@playwright/test";

import { captureTo, loginViaUi } from "./helpers";
import {
  BACKEND_PASSWORD,
  LocalFixture,
  TestBackend,
  buildTestServerBinary,
  seedJob,
  waitForJob,
} from "./job-recovery-harness";
import { E2E_WEB_PORT, REPO_ROOT } from "./runtime";

test.describe.configure({ timeout: 300_000 });

const WEB_BASE = `http://127.0.0.1:${E2E_WEB_PORT}`;
const EVIDENCE_DIR = `${REPO_ROOT}/artifacts/web-mvp/t18-qa-r23`;

test.use({ viewport: { width: 1440, height: 900 } });

interface RealDraft {
  readonly itemId: string;
  readonly draftId: string;
  readonly assetId: string;
  readonly sha256: string;
}

let fixture: LocalFixture;
let backend: TestBackend;
let draft: RealDraft;

/** 路由台账：只做源站改写 + 非本机阻断；`fulfilledOk` 必须恒为 0。 */
interface Routing {
  readonly external: string[];
  fulfilledOk: number;
}

async function installRouting(page: Page, base: string): Promise<Routing> {
  const routing: Routing = { external: [], fulfilledOk: 0 };
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
    const target = url.href.startsWith(`${WEB_BASE}/api/v1`)
      ? `${base}${url.pathname}${url.search}`
      : request.url();
    await route.continue({ url: target });
  });
  return routing;
}

/** 原型层 WebGL draw 计数（跨上下文累加；实例层包装会在重建后冻结）。 */
async function installDrawCounter(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const target = window as unknown as Record<string, unknown>;
    const counters = (target.__qaR23Gl as Record<string, number> | undefined) ?? { draws: 0 };
    target.__qaR23Gl = counters;
    if (target.__qaR23GlPatched === true) {
      return;
    }
    target.__qaR23GlPatched = true;
    const patch = (prototype: object | undefined): void => {
      if (prototype === undefined) {
        return;
      }
      const holder = prototype as unknown as Record<string, ((...args: unknown[]) => unknown) | undefined>;
      for (const name of ["drawElements", "drawArrays"]) {
        const original = holder[name];
        if (typeof original !== "function") {
          continue;
        }
        holder[name] = function (this: unknown, ...args: unknown[]) {
          counters.draws = (counters.draws ?? 0) + 1;
          return original.apply(this, args);
        };
      }
    };
    const scope = window as unknown as {
      WebGL2RenderingContext?: { prototype: object };
      WebGLRenderingContext?: { prototype: object };
    };
    patch(scope.WebGL2RenderingContext?.prototype);
    patch(scope.WebGLRenderingContext?.prototype);
  });
}

async function draws(page: Page): Promise<number> {
  return page.evaluate(() => (window as unknown as { __qaR23Gl?: { draws: number } }).__qaR23Gl?.draws ?? 0);
}

async function viewerFrames(page: Page): Promise<number> {
  return page.evaluate(() => window.__EM_VIEWER__?.frames() ?? 0);
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

async function statusText(page: Page): Promise<string> {
  return (await page.getByTestId("viewer-status").textContent()) ?? "";
}

async function loseContext(page: Page): Promise<void> {
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
    (window as unknown as Record<string, unknown>).__qaR23Lose = extension;
    extension.loseContext();
    return "lost";
  });
  expect(result, "必须能通过 WEBGL_lose_context 真实丢失上下文").toBe("lost");
}

function axisDistance(a: readonly number[], b: readonly number[]): number {
  const deltas = [0, 1, 2].map((axis) => (a[axis] ?? Number.NaN) - (b[axis] ?? Number.NaN));
  return Math.hypot(deltas[0] ?? Number.NaN, deltas[1] ?? Number.NaN, deltas[2] ?? Number.NaN);
}

async function drag(page: Page, dx: number, dy: number): Promise<void> {
  const box = await page.getByTestId("viewer-canvas").boundingBox();
  const cx = (box?.x ?? 0) + (box?.width ?? 0) / 2;
  const cy = (box?.y ?? 0) + (box?.height ?? 0) / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + dx, cy + dy, { steps: 10 });
  await page.mouse.up();
}

test.beforeAll(async () => {
  test.setTimeout(900_000);
  buildTestServerBinary();
  fixture = new LocalFixture();
  fixture.state.manualMode = "success";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  await fixture.start();
  backend = new TestBackend("qa-t18-r23");
  await backend.start(fixture);

  const context = await playwrightRequest.newContext();
  const seeded = await seedJob(context, backend, "QA23 手动重建复验");
  const detail = await waitForJob(
    context,
    backend.base,
    seeded.jobId,
    (candidate) => candidate.status === "succeeded" || candidate.status === "failed",
    "真实流水线产出草稿",
    180_000,
  );
  expect(detail.status, "真实流水线必须产出草稿").toBe("succeeded");
  expect(detail.draftId).toBeTruthy();
  const response = await context.get(
    `${backend.base}/api/v1/items/${seeded.itemId}/drafts/${detail.draftId}`,
  );
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: { knowledge: { model: Record<string, unknown> } } };
  const model = body.data.knowledge.model;
  expect(model.validationState, "草稿模型必须是 validated").toBe("validated");
  draft = {
    itemId: seeded.itemId,
    draftId: String(detail.draftId),
    assetId: String(model.assetId),
    sha256: String(model.sha256),
  };
  await context.dispose();
});

test.afterAll(async () => {
  await backend?.cleanup(fixture);
});

test("QA-R23-1 手动重建完整语义：状态/控件/8.6 秒稳定性/位姿策略/旋转（BUG-007 复验）", async ({
  page,
}) => {
  const routing = await installRouting(page, backend.base);
  await installDrawCounter(page);
  await loginViaUi(page, WEB_BASE, BACKEND_PASSWORD);
  await page.goto(`${WEB_BASE}/items/${draft.itemId}/drafts/${draft.draftId}/review`);

  await expect(page.getByTestId("viewer-canvas")).toBeVisible();
  await expect.poll(async () => await viewerFrames(page), { timeout: 60_000 }).toBeGreaterThan(0);
  await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");

  // 默认取景（load 后首次 fit）——用于核对「复位视角」回默认取景的语义。
  const poseDefault = await viewerPose(page);
  expect(poseDefault, "加载后必须能读到相机位姿").not.toBeNull();

  // 先旋转，制造一个与默认取景不同的位姿（重建后应保留的就是它）。
  await drag(page, 100, 45);
  const poseRotated = await viewerPose(page);
  expect(axisDistance(poseRotated?.positionLocal ?? [], poseDefault?.positionLocal ?? [])).toBeGreaterThan(
    0.01,
  );
  const drawsBeforeLoss = await draws(page);
  expect(drawsBeforeLoss, "丢失前必须真的在绘制").toBeGreaterThan(0);

  // --- 上下文丢失 → 立即重建 ---------------------------------------------------
  await loseContext(page);
  await expect(page.getByTestId("viewer-status")).toContainText("3D 显示已中断");
  await expect(page.getByRole("button", { name: "立即重建" })).toBeVisible();
  await page.getByRole("button", { name: "立即重建" }).click();

  // 1. 状态行回到可用态（公平窗口内轮询；失败会让下面的断言给出具体文案）。
  await expect(page.getByTestId("viewer-status")).toContainText("模型已加载", { timeout: 15_000 });
  // 4. 真实绘制继续（独立 draw 计数）。
  await expect
    .poll(async () => await draws(page), { timeout: 30_000 })
    .toBeGreaterThan(drawsBeforeLoss + 2);
  // 2. 键盘等价控件恢复 enabled。
  await expect(page.getByRole("button", { name: "复位视角" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "适配模型" })).toBeEnabled();
  // 5. 重建入口消失。
  await expect(page.getByRole("button", { name: "立即重建" })).toHaveCount(0);

  // 6a. 位姿策略：重建后套用重建前位姿（asset-root 局部坐标，1e-3）。
  const poseAfterRebuild = await viewerPose(page);
  expect(
    axisDistance(poseAfterRebuild?.positionLocal ?? [], poseRotated?.positionLocal ?? []),
    `重建后应保留重建前位姿（实际 ${JSON.stringify(poseAfterRebuild?.positionLocal)}，期望 ${JSON.stringify(poseRotated?.positionLocal)}）`,
  ).toBeLessThan(1e-3);

  // 3. 8.6 秒后不得误报不可用（丢失时的 8s 兜底计时必须已被清除）。
  const statusAtRebuild = await statusText(page);
  await page.waitForTimeout(8_600);
  const statusAfter8s = await statusText(page);
  const rebuildVisibleAfter8s = await page
    .getByRole("button", { name: "立即重建" })
    .isVisible()
    .catch(() => false);
  const resetDisabledAfter8s = await page.getByRole("button", { name: "复位视角" }).isDisabled();

  // 4'. 重建后仍可旋转（拖动改变位姿 + draw 继续增长）。
  const drawsBeforeSecondDrag = await draws(page);
  const poseBeforeSecondDrag = await viewerPose(page);
  await drag(page, -90, 30);
  const poseAfterSecondDrag = await viewerPose(page);
  expect(
    axisDistance(poseAfterSecondDrag?.positionLocal ?? [], poseBeforeSecondDrag?.positionLocal ?? []),
    "重建后拖动必须仍能旋转",
  ).toBeGreaterThan(0.01);
  await expect
    .poll(async () => await draws(page), { timeout: 15_000 })
    .toBeGreaterThan(drawsBeforeSecondDrag);

  // 6b. 「复位视角」回到默认初始取景（不是回到重建前的位姿）。
  await page.getByRole("button", { name: "复位视角" }).click();
  await expect
    .poll(async () => {
      const pose = await viewerPose(page);
      return pose === null ? Number.NaN : axisDistance(pose.positionLocal, poseDefault?.positionLocal ?? []);
    })
    .toBeLessThan(1e-3);

  // 证据落档（供报告引用）。
  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });
  fs.writeFileSync(
    path.join(EVIDENCE_DIR, "r23-manual-rebuild-state.json"),
    JSON.stringify(
      {
        说明:
          "QA 回合 23 独立复验：手动「立即重建」→ 8.6 秒后的现场（真实链路 + 独立 draw 计数）",
        statusAtRebuild,
        statusAfter8s,
        rebuildButtonVisible: rebuildVisibleAfter8s,
        resetButtonDisabled: resetDisabledAfter8s,
        drawsBeforeLoss,
        drawsAfter8s: await draws(page),
        poseRotated: poseRotated?.positionLocal ?? null,
        poseAfterRebuild: poseAfterRebuild?.positionLocal ?? null,
        poseDefault: poseDefault?.positionLocal ?? null,
        routingFulfilledOk: routing.fulfilledOk,
        externalRequests: routing.external,
      },
      null,
      2,
    ),
  );
  await captureTo("t18-qa-r23", page, "r23-manual-rebuild-8s-after");

  // 离线与不伪造证据。
  expect(routing.fulfilledOk, "本用例不得伪造正常响应").toBe(0);
  expect(routing.external, "全程零真实外网").toEqual([]);
  expect(statusAfter8s, "8.6 秒后必须仍为可用态").toContain("模型已加载");
  expect(statusAfter8s, "不得误报上下文不可用").not.toContain("不可用");
  expect(rebuildVisibleAfter8s, "重建成功后不得再显示「立即重建」").toBe(false);
  expect(resetDisabledAfter8s, "8.6 秒后控件必须仍可用").toBe(false);
});
