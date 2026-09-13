/**
 * T09 QA 独立验收 spec（回合 9）——由 QA 自写，**不共享** `pdf-preparation.spec.ts` 的断言与工具。
 *
 * 独立复现/加强的关键结论（每条都走真实 Chrome + 由 globalSetup 自管理的真实后端）：
 * 1. 续传「只补缺失页」用**真实关闭标签页 + 新标签页**复现（不是 reload），并用服务端
 *    `GET /preparations/{id}` 交叉验证已完成页的资产 id 未被替换；封存后直接查 SQLite
 *    断言 jobs / cost_ledger / provider_attempts 无新增（AC-023/AC-025）。
 * 2. 离线：阻断所有非 127.0.0.1/localhost 请求后仍能完成整轮准备；逐个确认 PDF.js
 *    worker 与 CMaps/standard fonts/WASM/ICC 四类静态资源可从本机取到（AC-023），并核对
 *    主包与 worker 的版本字符串与 `pdfjs-dist` 自身 package.json 一致。
 * 3. BUG-002（通知条遮挡顶栏）：`elementFromPoint` 命中判定 + 几何（通知条在顶栏之下）
 *    + 键盘聚焦与 Enter 激活 + 真实指针点击；宽屏 1280 与窄屏 390 都跑。
 * 4. 进度文案（第 n / N 页；无「总进度」「预计剩余」）、单 canvas（包一层
 *    `document.createElement` 计数）、`beforeunload`、取消后停止上传（UI-014/UI-015）。
 * 5. AC-024 负例（加密 / 101 页）：明确文案、零 `/preparations` 请求、SQLite 各表零新增。
 * 6. 页图规格：上报 viewport 与实际 JPEG 像素尺寸一致（旋转页含 /Rotate 90），
 *    且四角像素为白色（白底 JPEG，旋转后左上角为原点）。
 *
 * 运行：`npm --prefix apps/web run test:e2e -- qa-t09-independent.spec.ts`（QA 自管理后端）。
 * 本文件属验收测试，不修改生产代码；证据由 QA 复制到 `artifacts/web-mvp/t09-qa/`。
 */

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

const REPO_ROOT = path.resolve(import.meta.dirname, "../../../..");
const WORK_DIR = process.env.EM_E2E_WORK_DIR ?? path.join(os.tmpdir(), "em-web-mvp-e2e");
const FIXTURE_DIR = path.join(REPO_ROOT, "tests", "fixtures", "assets");
const PDFJS_DIR = path.join(REPO_ROOT, "apps", "web", "node_modules", "pdfjs-dist");

interface RuntimeInfo {
  readonly apiBase: string;
  readonly password: string;
  readonly dataDir: string;
  readonly serverPid: number;
}

/** 惰性读取运行时（globalSetup 在收集测试文件之后执行）。 */
function runtime(): RuntimeInfo {
  return JSON.parse(fs.readFileSync(path.join(WORK_DIR, "runtime.json"), "utf8")) as RuntimeInfo;
}

function api(): string {
  return runtime().apiBase;
}

interface SeedResult {
  readonly itemId: string;
  readonly documentId: string;
  readonly sourceAssetId: string;
  readonly sourceSha256: string;
}

interface PreparationDetail {
  readonly id: string;
  readonly state: string;
  readonly pageCount: number | null;
  readonly clientDerived: boolean;
  readonly revision: number;
  readonly missingPages: number[];
  readonly pages: {
    pageNumber: number;
    textAssetId: string | null;
    imageAssetId: string | null;
    viewport: { width: number; height: number; rotation: number } | null;
  }[];
}

// --- API 造数（QA 自己的最小实现；只走公开 HTTP 合同） ---

async function apiLogin(request: APIRequestContext): Promise<string> {
  const response = await request.post(`${api()}/api/v1/auth/login`, {
    data: { password: runtime().password },
  });
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: { csrfToken: string } };
  return body.data.csrfToken;
}

async function seedItemWithDocument(
  request: APIRequestContext,
  fixtureName: string,
  name: string,
): Promise<SeedResult> {
  const csrf = await apiLogin(request);

  const created = await request.post(`${api()}/api/v1/items`, {
    headers: { "x-csrf-token": csrf },
    data: { name, model: `QA-T09-${fixtureName}`, brand: "QA" },
  });
  expect(created.status(), await created.text()).toBe(201);
  const itemId = ((await created.json()) as { data: { id: string } }).data.id;

  const upload = await request.post(`${api()}/api/v1/items/${itemId}/assets`, {
    headers: { "x-csrf-token": csrf },
    multipart: {
      purpose: "document",
      file: {
        name: fixtureName,
        mimeType: "application/pdf",
        buffer: fs.readFileSync(path.join(FIXTURE_DIR, fixtureName)),
      },
    },
  });
  expect(upload.status(), await upload.text()).toBe(201);
  const asset = ((await upload.json()) as { data: { id: string; sha256: string } }).data;

  const documentResponse = await request.post(`${api()}/api/v1/items/${itemId}/documents`, {
    headers: { "x-csrf-token": csrf },
    data: { sourceAssetId: asset.id, title: `${name} 说明书` },
  });
  expect(documentResponse.status(), await documentResponse.text()).toBe(201);
  const document = (
    (await documentResponse.json()) as { data: { id: string; sourceSha256: string } }
  ).data;

  return {
    itemId,
    documentId: document.id,
    sourceAssetId: asset.id,
    sourceSha256: document.sourceSha256,
  };
}

async function loginViaUi(page: Page): Promise<void> {
  await page.goto("/login");
  await page.getByLabel("密码").fill(runtime().password);
  await page.getByRole("button", { name: "登录" }).click();
  await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();
}

async function fetchPreparation(
  request: APIRequestContext,
  preparationId: string,
): Promise<PreparationDetail> {
  const response = await request.get(`${api()}/api/v1/preparations/${preparationId}`);
  expect(response.status(), await response.text()).toBe(200);
  return ((await response.json()) as { data: PreparationDetail }).data;
}

/** 记录某页面会话里的 `PUT .../pages/{n}` 页号（发生顺序）。 */
function recordPagePuts(page: Page): number[] {
  const puts: number[] = [];
  page.on("request", (request) => {
    const match = /\/api\/v1\/preparations\/[^/]+\/pages\/(\d+)$/.exec(request.url());
    if (match !== null && request.method() === "PUT" && match[1] !== undefined) {
      puts.push(Number(match[1]));
    }
  });
  return puts;
}

/** 点击「开始准备/继续准备」并返回新建或复用的 preparation id（读服务端响应）。 */
async function startPreparation(page: Page): Promise<string> {
  const responsePromise = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      /\/api\/v1\/documents\/[^/]+\/preparations$/.test(new URL(response.url()).pathname),
  );
  await page.getByTestId("prepare-start").click();
  const response = await responsePromise;
  expect(response.status(), "创建/复用 preparation 必须 200/201").toBeLessThan(300);
  return ((await response.json()) as { data: { id: string } }).data.id;
}

// --- SQLite 直查（独立于应用层） ---

type TableName =
  | "preparations"
  | "pages"
  | "jobs"
  | "job_stages"
  | "cost_ledger"
  | "provider_attempts";

function dbCount(table: TableName): number {
  const stdout = execFileSync(
    "sqlite3",
    [path.join(runtime().dataDir, "manual.sqlite3"), `SELECT COUNT(*) FROM ${table};`],
    { encoding: "utf8" },
  );
  return Number(stdout.trim());
}

// ---------------------------------------------------------------------------
// 1. 续传（真实关闭标签页 → 新标签页）
// ---------------------------------------------------------------------------

test("续传：关闭标签页后用新标签页继续，只补缺失页且不替换已完成页资产", async ({
  page,
  context,
  request,
}) => {
  const seed = await seedItemWithDocument(request, "sample-manual-text.pdf", "QA 独立续传");
  await loginViaUi(page);

  // 第一轮：第 2 页的 PUT 被注入失败（模拟中断），第 1 页正常完成。
  await page.route("**/pages/2", async (route) => {
    if (route.request().method() === "PUT") {
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({
          error: { code: "INTERNAL", message: "QA 注入中断", details: null, requestId: "qa" },
        }),
      });
      return;
    }
    await route.continue();
  });
  const firstRunPuts = recordPagePuts(page);
  await page.goto(`/items/${seed.itemId}/import/prepare`);
  const preparationId = await startPreparation(page);
  await expect(page.getByTestId("prepare-failures")).toContainText("第 2 页失败", {
    timeout: 60_000,
  });
  expect(firstRunPuts, "第一轮按 1-based 页序处理 1、2").toEqual([1, 2]);

  const before = await fetchPreparation(request, preparationId);
  expect(before.state).toBe("preparing");
  expect(before.pages.map((entry) => entry.pageNumber)).toEqual([1]);
  const pageOne = before.pages[0];
  if (pageOne === undefined) {
    throw new Error("服务端第 1 页缺失");
  }
  expect(pageOne.imageAssetId).not.toBeNull();

  // 真实关闭整个标签页（不是 reload）。
  await page.close();

  // 新标签页：没有本地指针，点「开始准备」后必须从服务端得知"第 1 页已完成"。
  const resumed = await context.newPage();
  const resumedPuts = recordPagePuts(resumed);
  const uploadsInResume: string[] = [];
  resumed.on("request", (event) => {
    if (event.method() === "POST" && /\/items\/[^/]+\/assets$/.test(new URL(event.url()).pathname)) {
      uploadsInResume.push(event.url());
    }
  });
  await resumed.goto(`/items/${seed.itemId}/import/prepare`);
  await expect(resumed.getByTestId("prepare-start")).toHaveText("开始准备");
  await resumed.getByTestId("prepare-start").click();
  await expect(resumed.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });

  expect(resumedPuts, "续传只补第 2 页，绝不重传第 1 页").toEqual([2]);
  expect(uploadsInResume.length, "续传只为缺失页上传资产（页图+文字 ≤2）").toBeLessThanOrEqual(2);

  const after = await fetchPreparation(request, preparationId);
  expect(after.pages.map((entry) => entry.pageNumber)).toEqual([1, 2]);
  expect(after.pages[0]?.imageAssetId, "已完成页的页图资产 id 未被替换").toBe(
    pageOne.imageAssetId,
  );
  expect(after.pages[0]?.textAssetId, "已完成页的页文字资产 id 未被替换").toBe(
    pageOne.textAssetId,
  );

  // 封存：ready + clientDerived，且 SQLite 直查无 job / 费用（AC-025）。
  const jobsBefore = dbCount("jobs");
  const stagesBefore = dbCount("job_stages");
  const costBefore = dbCount("cost_ledger");
  const attemptsBefore = dbCount("provider_attempts");
  await resumed.getByTestId("prepare-seal").click();
  await expect(resumed.getByTestId("prepare-sealed")).toBeVisible({ timeout: 30_000 });
  await expect(resumed.getByTestId("prepare-sealed")).toContainText("clientDerived");

  const sealed = await fetchPreparation(request, preparationId);
  expect(sealed.state).toBe("ready");
  expect(sealed.pageCount).toBe(2);
  expect(sealed.clientDerived).toBe(true);
  expect(sealed.missingPages).toEqual([]);
  expect(dbCount("jobs") - jobsBefore, "complete 不创建 job").toBe(0);
  expect(dbCount("job_stages") - stagesBefore, "complete 不创建阶段").toBe(0);
  expect(dbCount("cost_ledger") - costBefore, "complete 不产生费用记录").toBe(0);
  expect(dbCount("provider_attempts") - attemptsBefore, "complete 不产生付费提交").toBe(0);
  await resumed.close();
});

// ---------------------------------------------------------------------------
// 2. 离线资源（阻断全部非本机请求）
// ---------------------------------------------------------------------------

test("离线：阻断非本机请求仍完成准备；worker/CMaps/字体/WASM/ICC 均本机可用", async ({
  page,
  request,
}) => {
  const seed = await seedItemWithDocument(request, "sample-manual-nonlatin.pdf", "QA 独立离线");
  await loginViaUi(page);

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

  const allPaths: string[] = [];
  const vendorResponses: { path: string; status: number }[] = [];
  page.on("request", (event) => allPaths.push(new URL(event.url()).pathname));
  page.on("response", (event) => {
    const pathname = new URL(event.url()).pathname;
    if (pathname.startsWith("/vendor/pdfjs/")) {
      vendorResponses.push({ path: pathname, status: event.status() });
    }
  });

  await page.goto(`/items/${seed.itemId}/import/prepare`);
  const preparationId = await startPreparation(page);
  await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });

  expect(external, `不得出现任何外部请求：${external.join(", ")}`).toEqual([]);
  expect(vendorResponses.length, "本轮应请求本地 vendor 资源").toBeGreaterThan(0);
  expect(
    vendorResponses.filter((entry) => entry.status !== 200),
    `vendor 资源必须全部 200：${JSON.stringify(vendorResponses)}`,
  ).toEqual([]);
  expect(
    vendorResponses.some((entry) => entry.path.startsWith("/vendor/pdfjs/cmaps/")),
    `CMap 必须来自本机 vendor：${vendorResponses.map((entry) => entry.path).join(", ")}`,
  ).toBe(true);
  expect(
    allPaths.some((pathname) => /pdf\.worker\.min/.test(pathname)),
    "PDF.js worker 必须来自本机",
  ).toBe(true);

  // 非拉丁字体页文字提取正确（CMap 生效）。
  const detail = await fetchPreparation(request, preparationId);
  const textAssetId = detail.pages[0]?.textAssetId ?? null;
  expect(textAssetId).not.toBeNull();
  const textResponse = await request.get(`${api()}/api/v1/assets/${textAssetId ?? ""}/content`);
  expect(textResponse.status()).toBe(200);
  expect(await textResponse.text()).toContain("部件一：松开四颗螺丝");

  // 在"外部全部阻断"的页面上下文里，直接确认四类静态资源本机可取（非 200/空即失败）。
  const probe = await page.evaluate(async () => {
    const paths = [
      "/vendor/pdfjs/cmaps/UniGB-UCS2-H.bcmap",
      "/vendor/pdfjs/standard_fonts/LiberationSans-Regular.ttf",
      "/vendor/pdfjs/wasm/openjpeg.wasm",
      "/vendor/pdfjs/wasm/qcms_bg.wasm",
      "/vendor/pdfjs/iccs/CGATS001Compat-v2-micro.icc",
    ];
    const results: { path: string; status: number; bytes: number }[] = [];
    for (const target of paths) {
      const response = await fetch(target);
      const buffer = await response.arrayBuffer();
      results.push({ path: target, status: response.status, bytes: buffer.byteLength });
    }
    return results;
  });
  for (const entry of probe) {
    expect(entry.status, `${entry.path} 必须可从本机加载（离线语义）`).toBe(200);
    expect(entry.bytes, `${entry.path} 不得为空`).toBeGreaterThan(0);
  }

  // 主包与 worker 同版本（读 node_modules 产物，与网络无关）。
  const pdfjsVersion = JSON.parse(
    fs.readFileSync(path.join(PDFJS_DIR, "package.json"), "utf8"),
  ).version as string;
  const mainBundle = fs.readFileSync(path.join(PDFJS_DIR, "build", "pdf.mjs"), "utf8");
  const workerBundle = fs.readFileSync(path.join(PDFJS_DIR, "build", "pdf.worker.min.mjs"), "utf8");
  expect(mainBundle.includes(pdfjsVersion), "主包版本与 package.json 一致").toBe(true);
  expect(workerBundle.includes(pdfjsVersion), "worker 与主包同版本").toBe(true);
});

// ---------------------------------------------------------------------------
// 3. BUG-002 复验（通知条不遮挡顶栏）
// ---------------------------------------------------------------------------

test("BUG-002 复验：通知可见时顶栏指针/键盘可用（1280/390；并记录定位轮询窗口）", async ({
  page,
}) => {
  await loginViaUi(page);

  interface HitSample {
    readonly tag: string | null;
    readonly linkText: string | null;
    readonly topBarBottom: number | null;
    readonly noticeTop: number | null;
    readonly noticesTopVar: string;
  }

  /** `elementFromPoint` 判定顶栏「资料库」链接中心命中什么（BUG-001 的原始证明手法）。 */
  const hitAtLibraryCenter = async (): Promise<HitSample> => {
    const geometry = await page.evaluate(() => {
      const topBar = document.querySelector(".top-bar");
      const notice = document.querySelector(".notice");
      return {
        topBarBottom: topBar === null ? null : Math.round(topBar.getBoundingClientRect().bottom),
        noticeTop: notice === null ? null : Math.round(notice.getBoundingClientRect().top),
        noticesTopVar: getComputedStyle(document.documentElement).getPropertyValue("--notices-top"),
      };
    });
    const libraryLink = page
      .getByRole("navigation", { name: "主导航" })
      .getByRole("link", { name: "资料库", exact: true });
    const box = await libraryLink.boundingBox();
    if (box === null) {
      throw new Error("顶栏「资料库」入口不可见");
    }
    const hit = await page.evaluate(
      ([x, y]) => {
        const element = document.elementFromPoint(x as number, y as number);
        return {
          tag: element?.tagName ?? null,
          linkText: element?.closest("a")?.textContent?.trim() ?? null,
        };
      },
      [box.x + box.width / 2, box.y + box.height / 2],
    );
    return { ...hit, ...geometry };
  };

  const transientRecords: unknown[] = [];

  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 800 });

    const createItem = async (suffix: string): Promise<void> => {
      await page.goto("/items/new");
      await page.getByLabel("名称").fill(`QA 通知条-${suffix}`);
      await page.getByLabel("准确型号").fill(`QA-NOTICE-${suffix}`);
      await page.getByRole("button", { name: "创建并继续" }).click();
      await expect(page.getByRole("button", { name: /^关闭通知：/ }).first()).toBeVisible();
    };
    const libraryLink = page
      .getByRole("navigation", { name: "主导航" })
      .getByRole("link", { name: "资料库", exact: true });

    // (A) 键盘路径：通知可见时聚焦顶栏链接并用 Enter 激活。
    await createItem(`kbd-${width}`);
    await libraryLink.focus();
    const focusedText = await page.evaluate(
      () => document.activeElement?.closest("a")?.textContent?.trim() ?? null,
    );
    expect(focusedText, "键盘必须能聚焦顶栏「资料库」").toBe("资料库");
    await page.keyboard.press("Enter");
    await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();

    // (B) 指针路径：重新触发一条通知，记录"通知刚出现"的第一帧命中（定位轮询窗口的残留），
    //     再等待稳定态并做**显式断言**（不是只靠 poll 通过）。
    await createItem(`ptr-${width}`);
    const firstFrame = await hitAtLibraryCenter();
    await expect
      .poll(async () => (await hitAtLibraryCenter()).linkText, {
        message: `宽度 ${width}px：通知条可见时顶栏链接中心必须命中链接本身`,
        timeout: 2000,
      })
      .toBe("资料库");
    const settled = await hitAtLibraryCenter();
    expect(
      settled.linkText,
      `宽度 ${width}px：稳定态必须命中顶栏链接（实际 ${JSON.stringify(settled)}）`,
    ).toBe("资料库");
    expect(
      settled.noticeTop ?? -1,
      `宽度 ${width}px：稳定态通知条位于顶栏之下（顶栏底 ${settled.topBarBottom}）`,
    ).toBeGreaterThanOrEqual((settled.topBarBottom ?? 0) - 1);
    transientRecords.push({ width, firstFrame, settled });

    await libraryLink.click();
    await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();

    // 通知条自身仍可交互（关闭按钮命中按钮本身并可点击）。
    const dismiss = page.getByRole("button", { name: /^关闭通知：/ }).first();
    if ((await dismiss.count()) > 0) {
      const dismissBox = await dismiss.boundingBox();
      if (dismissBox !== null) {
        const dismissHit = await page.evaluate(
          ([x, y]) => document.elementFromPoint(x as number, y as number)?.tagName ?? null,
          [dismissBox.x + dismissBox.width / 2, dismissBox.y + dismissBox.height / 2],
        );
        expect(dismissHit, "通知「关闭」按钮中心必须命中按钮").toBe("BUTTON");
      }
      await dismiss.click().catch(() => undefined);
    }
  }

  // 把"第一帧 vs 稳定态"的命中记录留在 QA 证据目录（定位轮询窗口的残留）。
  fs.writeFileSync(
    path.join(REPO_ROOT, "artifacts", "web-mvp", "t09-qa", "bug002-hit-samples.json"),
    JSON.stringify(transientRecords, null, 2),
  );
  console.log(`BUG-002 命中样本：${JSON.stringify(transientRecords)}`);
});

// ---------------------------------------------------------------------------
// 4. 进度文案、单 canvas、beforeunload、取消
// ---------------------------------------------------------------------------

test("逐页进度（第 n / N 页）、单 canvas、beforeunload 与取消停止上传", async ({
  page,
  request,
}) => {
  const seed = await seedItemWithDocument(request, "sample-manual-text.pdf", "QA 独立进度");
  await loginViaUi(page);

  await page.addInitScript(() => {
    const target = window as unknown as { __qaCanvasCreated: number };
    target.__qaCanvasCreated = 0;
    const original = document.createElement.bind(document);
    document.createElement = ((tagName: string, options?: ElementCreationOptions) => {
      if (String(tagName).toLowerCase() === "canvas") {
        target.__qaCanvasCreated += 1;
      }
      return original(tagName, options);
    }) as typeof document.createElement;
  });

  // 放慢资产上传，留出稳定的观测窗口（只在网络层延迟，不改生产代码）。
  await page.route("**/items/*/assets", async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 600));
    await route.continue();
  });

  const puts = recordPagePuts(page);
  await page.goto(`/items/${seed.itemId}/import/prepare`);
  await startPreparation(page);

  await expect(page.getByTestId("prepare-status")).toContainText(/第 \d+ \/ 2 页/, {
    timeout: 60_000,
  });
  const pageText = await page.locator("body").innerText();
  expect(pageText, "不得出现线性总进度文案").not.toContain("总进度");
  expect(pageText, "不得出现预计剩余时间").not.toContain("预计剩余");
  expect(await page.locator("canvas").count(), "DOM 中不得出现 canvas").toBe(0);

  const beforeUnloadPrevented = await page.evaluate(() => {
    const event = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(beforeUnloadPrevented, "准备进行中必须注册 beforeunload 离开确认").toBe(true);

  const created = await page.evaluate(
    () => (window as unknown as { __qaCanvasCreated: number }).__qaCanvasCreated,
  );
  expect(created, "整轮准备至少创建 1 张 canvas").toBeGreaterThanOrEqual(1);
  expect(created, "每次只渲染一页：同一轮最多 1 张 canvas").toBeLessThanOrEqual(1);

  const putsBeforeCancel = puts.length;
  await page.getByTestId("prepare-cancel").click();
  await expect(page.getByTestId("prepare-cancel")).toBeDisabled();
  await page.waitForTimeout(1500);
  expect(
    puts.length,
    "取消后不再上传新页（在途页最多 +1）",
  ).toBeLessThanOrEqual(putsBeforeCancel + 1);
});

// ---------------------------------------------------------------------------
// 5. AC-024 负例：加密 / >100 页（零记录、零 job、零收费）
// ---------------------------------------------------------------------------

test("加密与 101 页 PDF：明确拒绝、零 preparation 请求、SQLite 各表零新增", async ({
  page,
  request,
}) => {
  await loginViaUi(page);

  const cases = [
    {
      fixture: "sample-manual-encrypted.pdf",
      name: "QA 加密",
      expected: "该 PDF 已加密，首版不支持，请先解除加密后再上传",
    },
    {
      fixture: "sample-manual-many-pages.pdf",
      name: "QA 超页数",
      expected: "PDF 共 101 页，超过 100 页上限",
    },
  ] as const;

  const before = {
    preparations: dbCount("preparations"),
    pages: dbCount("pages"),
    jobs: dbCount("jobs"),
    cost: dbCount("cost_ledger"),
    attempts: dbCount("provider_attempts"),
  };

  const writes: string[] = [];
  page.on("request", (event) => {
    const pathname = new URL(event.url()).pathname;
    if (/\/preparations(\/|$)/.test(pathname)) {
      writes.push(`${event.method()} ${pathname}`);
    }
  });

  for (const testCase of cases) {
    writes.length = 0;
    const seed = await seedItemWithDocument(request, testCase.fixture, testCase.name);
    await page.goto(`/items/${seed.itemId}/import/prepare`);
    await page.getByTestId("prepare-start").click();
    await expect(page.getByText(testCase.expected)).toBeVisible({ timeout: 30_000 });
    expect(writes, `${testCase.fixture}：拒绝路径不得创建 preparation / 页记录`).toEqual([]);
    await expect(page.getByTestId("prepare-seal")).toBeDisabled();
  }

  expect(dbCount("preparations") - before.preparations, "不得创建 preparation 行").toBe(0);
  expect(dbCount("pages") - before.pages, "不得创建页行").toBe(0);
  expect(dbCount("jobs") - before.jobs, "不得进入 jobs").toBe(0);
  expect(dbCount("cost_ledger") - before.cost, "不得产生费用记录").toBe(0);
  expect(dbCount("provider_attempts") - before.attempts, "不得产生付费提交").toBe(0);
});

// ---------------------------------------------------------------------------
// 6. 页图规格：viewport ↔ JPEG 像素一致（含旋转页）+ 白底
// ---------------------------------------------------------------------------

test("页图上报 viewport 与实际 JPEG 像素一致（旋转页 /Rotate 90）且四角为白色", async ({
  page,
  request,
}) => {
  const seed = await seedItemWithDocument(request, "sample-manual-rotated.pdf", "QA 独立页图规格");
  await loginViaUi(page);
  await page.goto(`/items/${seed.itemId}/import/prepare`);
  const preparationId = await startPreparation(page);
  await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });

  const detail = await fetchPreparation(request, preparationId);
  expect(detail.pages.map((entry) => entry.pageNumber)).toEqual([1, 2]);

  for (const entry of detail.pages) {
    const viewport = entry.viewport;
    if (viewport === null) {
      throw new Error(`第 ${entry.pageNumber} 页缺少 viewport`);
    }
    const imageAssetId = entry.imageAssetId;
    if (imageAssetId === null) {
      throw new Error(`第 ${entry.pageNumber} 页缺少页图`);
    }
    const probe = await page.evaluate(async (assetId) => {
      const response = await fetch(`/api/v1/assets/${assetId}/content`);
      if (!response.ok) {
        return { ok: false, width: 0, height: 0, corners: [] as number[][] };
      }
      const blob = await response.blob();
      const bitmap = await createImageBitmap(blob);
      const canvas = document.createElement("canvas");
      canvas.width = bitmap.width;
      canvas.height = bitmap.height;
      const context = canvas.getContext("2d");
      if (context === null) {
        return { ok: false, width: 0, height: 0, corners: [] as number[][] };
      }
      context.drawImage(bitmap, 0, 0);
      const points: [number, number][] = [
        [1, 1],
        [bitmap.width - 2, 1],
        [1, bitmap.height - 2],
        [bitmap.width - 2, bitmap.height - 2],
      ];
      const corners = points.map(([x, y]) => {
        const data = context.getImageData(x, y, 1, 1).data;
        return [data[0] ?? -1, data[1] ?? -1, data[2] ?? -1, data[3] ?? -1];
      });
      return { ok: true, width: bitmap.width, height: bitmap.height, corners };
    }, imageAssetId);

    expect(probe.ok, `第 ${entry.pageNumber} 页页图必须可解码为图像`).toBe(true);
    expect(
      { width: probe.width, height: probe.height },
      `第 ${entry.pageNumber} 页 JPEG 像素尺寸必须等于上报的旋转后 viewport（原点=旋转后左上角）`,
    ).toEqual({ width: viewport.width, height: viewport.height });
    expect(Math.max(probe.width, probe.height)).toBeLessThanOrEqual(2000);
    for (const pixel of probe.corners) {
      expect(pixel.length).toBe(4);
      const [r, g, b, a] = pixel;
      expect(r, `第 ${entry.pageNumber} 页四角必须为白色（实测 RGB(${r},${g},${b})）`).toBeGreaterThanOrEqual(250);
      expect(g).toBeGreaterThanOrEqual(250);
      expect(b).toBeGreaterThanOrEqual(250);
      expect(a).toBe(255);
    }
  }

  const rotated = detail.pages.find((entry) => entry.pageNumber === 2);
  expect(rotated?.viewport?.rotation, "第 2 页 /Rotate 90 必须反映在 viewport").toBe(90);
  expect(
    (rotated?.viewport?.width ?? 0) > (rotated?.viewport?.height ?? 0),
    "旋转后宽高互换",
  ).toBe(true);
  const upright = detail.pages.find((entry) => entry.pageNumber === 1);
  expect(upright?.viewport?.rotation).toBe(0);
  expect((upright?.viewport?.height ?? 0) > (upright?.viewport?.width ?? 0)).toBe(true);
});
