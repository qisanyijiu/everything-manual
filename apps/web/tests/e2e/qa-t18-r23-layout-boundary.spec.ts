/**
 * QA 回合 23 · 1280px 断点边界调查（T18 BUG-007 修复回合的附带发现，属 T08/T16 布局壳）。
 *
 * 问题：默认 e2e 视口 1280×720 **正好**等于 `wide ≥1280px` 断点；RD 报告曾观察到
 * `(min-width: 1280px)` 在 wide/mid 之间抖动，导致 `PageLayout` 换树、整页（含 3D
 * Canvas）反复重挂载。本文件**独立复核**该现象的触发条件，判定"产品问题"还是"测试脆弱"：
 *
 * 观察项（页面内记录，带 `performance.now()` 时间戳）：
 * - `matchMedia('(min-width: 1280px)')` 的 `change` 事件（含 innerWidth / clientWidth /
 *   visualViewport 尺寸，用于识别是"视口真变窄"还是"判定源异常"）；
 * - `document.documentElement` 的 ResizeObserver 报告；
 * - 阅读器状态节点（`[data-testid="viewer-status"]`）的身份变化 = 面板重挂载次数；
 * - 每个动作的时间窗（Node 侧打点，与页面同一时钟对齐）。
 *
 * 动作序列（模拟真实用户 + 测试常用操作）：等待就绪 → 空闲 2 秒 → 拖动旋转 →
 * fullPage 截图（`captureTo` 同款）→ 再拖动。观察哪些动作触发抖动。
 *
 * 结果落 `artifacts/web-mvp/t18-qa-r23/r23-layout-boundary.json`。
 */

import fs from "node:fs";
import path from "node:path";

import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

import { apiLogin, captureTo, loginViaUi, runtime, seedItemWithDocument, seedReadyPreparation } from "./helpers";
import { REPO_ROOT } from "./runtime";
import { draftPayload, fixtureModel, installViewerRoutes, viewerStats } from "./viewer-harness";

test.describe.configure({ timeout: 180_000 });

// 显式使用默认的 1280×720（断点边界现场）；与研究对象一致。
test.use({ viewport: { width: 1280, height: 720 } });

interface LayoutLog {
  init: { t: number; matches: boolean; iw: number; cw: number | null; href: string }[];
  mqEvents: { t: number; matches: boolean; iw: number; cw: number; vvW: number | null; vvH: number | null; href: string }[];
  resizeEvents: { t: number; iw: number; cw: number; sh: number; ch: number; vv: number | null }[];
  windowResize: { t: number; iw: number; ih: number; vvW: number | null; vvH: number | null }[];
  polls: { t: number; matches: boolean; iw: number; cw: number; vvW: number | null; vvH: number | null; href: string }[];
  probes: { t: number; query: string; matches: boolean; iw: number; vvW: number | null }[];
  raf: { t: number; iw: number; vvW: number | null }[];
  nodeEvents: { t: number; present: boolean }[];
  marks: { label: string; t: number }[];
}

async function installObservers(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const log: LayoutLog = {
      init: [],
      mqEvents: [],
      resizeEvents: [],
      windowResize: [],
      polls: [],
      probes: [],
      raf: [],
      nodeEvents: [],
      marks: [],
    };
    (window as unknown as { __qaLayout: LayoutLog }).__qaLayout = log;
    const list = window.matchMedia("(min-width: 1280px)");
    const snapshot = (): { matches: boolean; iw: number; cw: number; vvW: number | null; vvH: number | null; href: string } => ({
      matches: list.matches,
      iw: window.innerWidth,
      cw: document.documentElement?.clientWidth ?? -1,
      vvW: window.visualViewport?.width ?? null,
      vvH: window.visualViewport?.height ?? null,
      href: location.href,
    });
    log.init.push({ t: performance.now(), matches: list.matches, iw: window.innerWidth, cw: document.documentElement?.clientWidth ?? null, href: location.href });
    list.addEventListener("change", (event) => {
      log.mqEvents.push({ t: performance.now(), ...snapshot(), matches: event.matches });
    });
    window.addEventListener("resize", () => {
      log.windowResize.push({
        t: performance.now(),
        iw: window.innerWidth,
        ih: window.innerHeight,
        vvW: window.visualViewport?.width ?? null,
        vvH: window.visualViewport?.height ?? null,
      });
    });
    // 探针查询：探测瞬态的"深度"（如果视口瞬时变窄到 1279 / 1269 以下，这些会翻转）。
    for (const query of ["(min-width: 1279px)", "(min-width: 1269px)"]) {
      const probe = window.matchMedia(query);
      probe.addEventListener("change", (event) => {
        log.probes.push({
          t: performance.now(),
          query,
          matches: event.matches,
          iw: window.innerWidth,
          vvW: window.visualViewport?.width ?? null,
        });
      });
    }
    // 帧率采样（每帧记录一次视口宽度变化；能抓到跨帧的瞬态）。
    let lastRafKey = "";
    const sample = (): void => {
      const key = `${window.innerWidth}|${window.visualViewport?.width ?? -1}`;
      if (key !== lastRafKey) {
        lastRafKey = key;
        log.raf.push({ t: performance.now(), iw: window.innerWidth, vvW: window.visualViewport?.width ?? null });
      }
      window.requestAnimationFrame(sample);
    };
    window.requestAnimationFrame(sample);
    // 5ms 轮询：change 事件可能被合并，轮询能抓到瞬态（配对 init 条目判断是否真实页面）。
    let lastKey = `${list.matches}|${window.innerWidth}|${document.documentElement?.clientWidth ?? -1}|${window.visualViewport?.width ?? -1}|${window.visualViewport?.height ?? -1}`;
    const timer = window.setInterval(() => {
      const key = `${list.matches}|${window.innerWidth}|${document.documentElement?.clientWidth ?? -1}|${window.visualViewport?.width ?? -1}|${window.visualViewport?.height ?? -1}`;
      if (key !== lastKey) {
        lastKey = key;
        log.polls.push({ t: performance.now(), ...snapshot() });
      }
    }, 5);
    document.addEventListener("DOMContentLoaded", () => {
      const record = (): void => {
        log.resizeEvents.push({
          t: performance.now(),
          iw: window.innerWidth,
          cw: document.documentElement.clientWidth,
          sh: document.documentElement.scrollHeight,
          ch: document.documentElement.clientHeight,
          vv: window.visualViewport?.width ?? null,
        });
      };
      new ResizeObserver(record).observe(document.documentElement);
      record();
      // 页面导航走了以后停掉轮询（防止 about:blank 的定时器泄漏进统计）。
      window.addEventListener("pagehide", () => window.clearInterval(timer));
    });
  });
}

/** 面板状态节点身份跟踪：节点被替换 = 阅读器面板重挂载。 */
async function trackPanelNode(page: Page): Promise<void> {
  await page.evaluate(() => {
    const log = (window as unknown as { __qaLayout: LayoutLog }).__qaLayout;
    let current = document.querySelector('[data-testid="viewer-status"]');
    log.nodeEvents.push({ t: performance.now(), present: current !== null });
    const check = (): void => {
      const node = document.querySelector('[data-testid="viewer-status"]');
      if (node !== current) {
        log.nodeEvents.push({ t: performance.now(), present: node !== null });
        current = node;
      }
    };
    new MutationObserver(check).observe(document.body, { childList: true, subtree: true });
    window.setInterval(check, 50);
  });
}

async function mark(page: Page, label: string): Promise<void> {
  await page.evaluate((text) => {
    (window as unknown as { __qaLayout: LayoutLog }).__qaLayout.marks.push({
      label: text,
      t: performance.now(),
    });
  }, label);
}

async function readLog(page: Page): Promise<LayoutLog> {
  return page.evaluate(() => (window as unknown as { __qaLayout: LayoutLog }).__qaLayout);
}

const reviewPathOf = (itemId: string, draftId: string): string =>
  `/items/${itemId}/drafts/${draftId}/review`;

async function seed(page: Page, request: APIRequestContext): Promise<{ itemId: string; draftId: string }> {
  await apiLogin(request, runtime().apiBase, runtime().password);
  const seed = await seedItemWithDocument(
    request,
    runtime().apiBase,
    runtime().password,
    "sample-manual-text.pdf",
    "T18-R23 断点调查",
  );
  const csrf = await apiLogin(request, runtime().apiBase, runtime().password);
  const preparationId = await seedReadyPreparation(request, runtime().apiBase, csrf, seed);
  const draftId = "draft-lb";
  const model = fixtureModel("viewer-asymmetric", "e2e-model-lb");
  await installViewerRoutes(page, {
    drafts: {
      [draftId]: draftPayload({
        itemId: seed.itemId,
        documentId: seed.documentId,
        preparationId,
        model,
        modelRevision: 1,
        hotspots: [],
        partNames: ["断点调查-部件"],
        stepTitles: ["断点调查-步骤"],
      }),
    },
    models: [model],
  });
  return { itemId: seed.itemId, draftId };
}

const markAt = (log: LayoutLog, label: string): number =>
  log.marks.find((entry) => entry.label === label)?.t ?? Number.NaN;

interface SequenceOutcome {
  readonly log: LayoutLog;
  readonly outsideScreenshot: string[];
  readonly remountsOutside: number;
  readonly duringScreenshot: number;
  /** 截图窗口内是否出现"视口被改成极小尺寸"的瞬态（Playwright fullPage 截图的特征）。 */
  readonly tinyViewportTransient: boolean;
}

/** 打开阅读页 → 空闲 → 拖动 →（可选）fullPage 截图 → 再拖动，返回带时间戳的记录。 */
async function runSequence(
  page: Page,
  request: APIRequestContext,
  options: { screenshot: boolean; evidenceFile: string },
): Promise<SequenceOutcome> {
  await installObservers(page);
  const { itemId, draftId } = await seed(page, request);
  await loginViaUi(page, "", runtime().password);
  await page.goto(reviewPathOf(itemId, draftId));
  await expect(page.getByTestId("viewer-canvas")).toBeVisible();
  await expect.poll(async () => (await viewerStats(page)).modelsAlive, { timeout: 30_000 }).toBe(1);
  await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
  await trackPanelNode(page);

  await mark(page, "idle-start");
  await page.waitForTimeout(2_000);
  await mark(page, "idle-end");
  const box = await page.getByTestId("viewer-canvas").boundingBox();
  const cx = (box?.x ?? 0) + (box?.width ?? 0) / 2;
  const cy = (box?.y ?? 0) + (box?.height ?? 0) / 2;
  await mark(page, "drag-1-start");
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + 90, cy + 40, { steps: 8 });
  await page.mouse.up();
  await mark(page, "drag-1-end");

  if (options.screenshot) {
    await mark(page, "screenshot-start");
    await captureTo("t18-qa-r23", page, `r23-layout-${options.evidenceFile}-fullpage`);
    await mark(page, "screenshot-end");
    // 截图瞬态会让页面整树重挂载（模型重载）；等待恢复再继续。
    await expect(page.getByTestId("viewer-status")).toContainText("模型已加载", { timeout: 30_000 });
  } else {
    await mark(page, "screenshot-start");
    await mark(page, "screenshot-end");
  }

  await mark(page, "drag-2-start");
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx - 70, cy - 20, { steps: 8 });
  await page.mouse.up();
  await mark(page, "drag-2-end");
  await mark(page, "end");

  const log = await readLog(page);
  const evidenceDir = path.join(REPO_ROOT, "artifacts", "web-mvp", "t18-qa-r23");
  fs.mkdirSync(evidenceDir, { recursive: true });
  fs.writeFileSync(path.join(evidenceDir, options.evidenceFile), JSON.stringify(log, null, 2));

  // 分类：断点瞬态（mq change / 5ms 轮询 / 帧采样）是否落在截图窗口内（两侧放宽 300ms）。
  const screenshotFrom = markAt(log, "screenshot-start") - 300;
  const screenshotTo = markAt(log, "screenshot-end") + 300;
  const transients = [
    ...log.mqEvents.map((event) => ({ label: `change@${Math.round(event.t)}ms`, t: event.t })),
    ...log.polls.slice(1).map((event) => ({ label: `poll@${Math.round(event.t)}ms`, t: event.t })),
    ...log.probes.map((event) => ({ label: `probe@${Math.round(event.t)}ms`, t: event.t })),
  ];
  const during = transients.filter((event) => event.t >= screenshotFrom && event.t <= screenshotTo);
  const outside = transients.filter((event) => event.t < screenshotFrom || event.t > screenshotTo);
  const remountsOutside = log.nodeEvents.filter(
    (event, index) => index > 0 && (event.t < screenshotFrom || event.t > screenshotTo),
  ).length;
  const tinyViewportTransient = log.raf.some(
    (sample) => sample.t >= screenshotFrom && sample.t <= screenshotTo && sample.iw < 100,
  );
  return {
    log,
    outsideScreenshot: outside.map((event) => event.label),
    remountsOutside,
    duringScreenshot: during.length,
    tinyViewportTransient,
  };
}

test("QA-R23-LB 1280px 视口 + fullPage 截图：瞬态只在截图窗口内（测试环境行为）", async ({
  page,
  request,
}) => {
  const outcome = await runSequence(page, request, {
    screenshot: true,
    evidenceFile: "r23-layout-boundary.json",
  });

  // 产品级不变量：稳定视口下的用户级交互（空闲、拖动）不得跨断点、不得重挂载面板。
  expect(
    outcome.outsideScreenshot,
    `常规交互期间不得发生断点切换；完整记录：${JSON.stringify(outcome.log)}`,
  ).toEqual([]);
  expect(outcome.remountsOutside, "常规交互期间阅读器面板不得被重挂载").toBe(0);
  await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
  // 证据：截图窗口内的瞬态数量与"极小视口瞬态"（0 = 未复现，>0 = 复现）。
  expect(outcome.duringScreenshot, "截图窗口内瞬态数量（证据字段）").toBeGreaterThanOrEqual(0);
});

test("QA-R23-LB2 对照：同视口、同交互但**不截图** → 零瞬态、零重挂载（产品级核查）", async ({
  page,
  request,
}) => {
  const outcome = await runSequence(page, request, {
    screenshot: false,
    evidenceFile: "r23-layout-control.json",
  });
  const log = outcome.log;
  expect(log.mqEvents, `不得出现跨断点事件：${JSON.stringify(log)}`).toEqual([]);
  expect(log.probes, "探针查询不得翻转（瞬态证据）").toEqual([]);
  // raf[0] 是安装时的基线采样（不是变化）。
  expect(log.raf.slice(1), "帧采样不得观察到视口宽度变化").toEqual([]);
  expect(log.nodeEvents.length, "阅读器面板不得被重挂载").toBe(1);
  await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
});

// 核验 RD 的规避手段：1440×900 下 fullPage 截图是否仍有瞬态。
test.describe("1440×900（RD 修复回合采用的视口）", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("QA-R23-LB3 1440×900 + fullPage 截图：瞬态是否也出现（机制核验）", async ({
    page,
    request,
  }) => {
    const outcome = await runSequence(page, request, {
      screenshot: true,
      evidenceFile: "r23-layout-1440.json",
    });
    // 产品级不变量同 LB：截图窗口外不得有瞬态/重挂载。
    expect(
      outcome.outsideScreenshot,
      `常规交互期间不得发生断点切换；完整记录：${JSON.stringify(outcome.log)}`,
    ).toEqual([]);
    expect(outcome.remountsOutside, "常规交互期间阅读器面板不得被重挂载").toBe(0);
    await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
  });
});
