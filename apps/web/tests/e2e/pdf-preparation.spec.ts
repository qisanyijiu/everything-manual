/**
 * T09 e2e：浏览器 PDF 准备与续传（PRD 修订 2 / ui_revision 2；AC-022、AC-023、AC-024、AC-025）。
 *
 * 环境：真实 Chrome（Playwright chromium）+ 真实 Rust 后端（由 globalSetup 启动，
 * 临时 data-dir + 自动初始化管理员）+ Vite 前端；**不依赖任何已运行的外部服务**。
 *
 * 覆盖清单（命令 ↔ AC）：
 * - 文字 PDF：逐页处理、`PUT .../pages/{n}` 为 **1-based** 且按页序、页图是 JPEG、
 *   页文字资产内容正确、viewport 长边 ≤2000px、封存后 state=ready + clientDerived（AC-022/AC-025）；
 * - 中断续传：重新进入只补缺页（不重传已完成页）（AC-023）；
 * - 扫描页：无文字层时按页图上传（textAssetId=null，页图仍存在）（AC-023）；
 * - 旋转页：页图 viewport 的 rotation=90、宽高互换（页图坐标原点=旋转后 viewport 左上角）（AC-023）；
 * - 非拉丁字体页：CJK 文字提取正确（AC-023）；
 * - 加密 PDF / 101 页 PDF：明确拒绝、可行动文案、**不创建准备记录/页记录**（AC-024）；
 * - 断外网：PDF.js worker/CMaps/standard fonts 全部本地加载、零外网请求（AC-023）；
 * - 逐页进度与取消：显示「第 n / N 页」与已完成页数（不用线性总百分比）、可取消、
 *   DOM 中不出现 canvas（每次只渲染一页）、进行中注册 beforeunload（UI-014/UI-015）。
 */

import { expect, test, type Page, type Request } from "@playwright/test";

import {
  capture,
  fetchAsset,
  fetchPreparation,
  isJpeg,
  loginViaUi,
  runtime,
  seedItemWithDocument,
} from "./helpers";

// globalSetup 在收集测试文件之后运行，因此运行时信息必须**惰性读取**（首次用到时才读文件）。
const password = (): string => runtime().password;
const apiBase = (): string => runtime().apiBase;

/** 记录某次页面会话里 `PUT /preparations/{id}/pages/{n}` 的页号（按发生顺序）。 */
function recordPagePuts(page: Page): number[] {
  const puts: number[] = [];
  page.on("request", (request: Request) => {
    const match = /\/api\/v1\/preparations\/[^/]+\/pages\/(\d+)$/.exec(request.url());
    if (match !== null && request.method() === "PUT") {
      puts.push(Number(match[1]));
    }
  });
  return puts;
}

/** 记录所有指向非本机地址的请求（断外网断言的证据）。 */
async function blockExternalRequests(page: Page): Promise<string[]> {
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

/** 记录以某前缀开头的本地请求路径（离线资源加载证据）。 */
function recordRequests(page: Page): string[] {
  const paths: string[] = [];
  page.on("request", (request: Request) => {
    const url = new URL(request.url());
    if (url.hostname === "127.0.0.1" || url.hostname === "localhost") {
      paths.push(url.pathname);
    }
  });
  return paths;
}

async function openPreparePage(page: Page, itemId: string): Promise<void> {
  await page.goto(`/items/${itemId}/import/prepare`);
  await expect(page.getByRole("heading", { name: "资料准备" })).toBeVisible();
  await expect(page.getByText("准备需要保持本标签页打开")).toBeVisible();
}

test.describe("PDF 准备与续传（T09）", () => {
  test("文字 PDF：逐页准备（1-based 页号）、JPEG 页图、封存 ready", async ({
    page,
    request,
  }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "T09 文字型说明书",
    );
    await loginViaUi(page, "", password());
    const puts = recordPagePuts(page);
    await openPreparePage(page, seed.itemId);

    // 准备前：DOM 中没有任何 canvas（每次只渲染一页，且渲染用离屏 canvas）。
    await expect(page.locator("canvas")).toHaveCount(0);
    // 人工复核截图（真实 Chrome + 真实后端；同时作为 implementation §T09 的冒烟证据）。
    await capture(page, "01-prepare-before");

    const [createResponse] = await Promise.all([
      page.waitForResponse(
        (response) =>
          response.request().method() === "POST" &&
          /\/api\/v1\/documents\/[^/]+\/preparations$/.test(response.url()),
      ),
      page.getByTestId("prepare-start").click(),
    ]);
    const preparationId = ((await createResponse.json()) as { data: { id: string } }).data.id;

    await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });
    expect(puts, "页号必须 1-based 且按页序处理").toEqual([1, 2]);
    await expect(page.getByTestId("prepare-status")).toContainText("已完成 2 / 2 页");
    // 不使用与真实页数无关的线性总百分比（PRD §6.3.2 禁用措辞）。
    await expect(page.locator("body")).not.toContainText("总进度");

    await capture(page, "02-pages-uploaded");
    const beforeSeal = await fetchPreparation(request, apiBase(), preparationId);
    expect(beforeSeal.state, "封存前必须是 preparing").toBe("preparing");
    expect(beforeSeal.pages.map((p) => p.pageNumber)).toEqual([1, 2]);
    for (const pageRecord of beforeSeal.pages) {
      expect(pageRecord.imageAssetId, "每页都要有页图").not.toBeNull();
      expect(pageRecord.textAssetId, "文字型 PDF 每页都要有页文字").not.toBeNull();
      const viewport = pageRecord.viewport;
      expect(viewport).not.toBeNull();
      expect(Math.max(viewport!.width, viewport!.height)).toBeLessThanOrEqual(2000);
      // 页图是白底 JPEG（内容魔数 + 服务端记录的 MIME）。
      const image = await fetchAsset(request, apiBase(), pageRecord.imageAssetId!);
      expect(image.contentType).toContain("image/jpeg");
      expect(isJpeg(image.bytes), "页图必须是 JPEG（FF D8 FF）").toBe(true);
    }
    // 页文字内容正确（不是空壳资产）。
    const firstText = await fetchAsset(request, apiBase(), beforeSeal.pages[0]!.textAssetId!);
    expect(firstText.bytes.toString("utf8")).toContain("Step 1: Loosen the four captive screws");

    // 封存：ready + clientDerived，且不产生任何任务（合同语义；服务端细节由 Rust 测试覆盖）。
    await page.getByTestId("prepare-seal").click();
    const sealedBlock = page.getByTestId("prepare-sealed");
    await expect(sealedBlock).toBeVisible();
    await expect(sealedBlock.getByText("准备完成（ready）")).toBeVisible();
    // 常驻 clientDerived 说明（哈希只证明字节一致，不证明页图来自原 PDF）。
    await expect(sealedBlock.getByText(/clientDerived/)).toBeVisible();
    await expect(page.getByTestId("prepare-seal")).toBeDisabled();

    await capture(page, "03-preparation-ready");
    const sealed = await fetchPreparation(request, apiBase(), preparationId);
    expect(sealed.state).toBe("ready");
    expect(sealed.pageCount).toBe(2);
    expect(sealed.clientDerived, "ready 的 preparation 必须标记 clientDerived").toBe(true);
    expect(sealed.missingPages).toEqual([]);
  });

  test("断线续传：重新进入只补缺页，不重传已完成页", async ({ page, request }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "T09 续传说明书",
    );
    await loginViaUi(page, "", password());

    // 让第 2 页的 PUT 失败一次（模拟中断），第 1 页照常完成。
    let injected = false;
    await page.route("**/pages/2", async (route) => {
      if (route.request().method() === "PUT" && !injected) {
        injected = true;
        await route.fulfill({
          status: 500,
          contentType: "application/json",
          body: JSON.stringify({
            error: { code: "INTERNAL", message: "注入的中断", details: null, requestId: "e2e" },
          }),
        });
        return;
      }
      await route.continue();
    });

    const puts = recordPagePuts(page);
    await openPreparePage(page, seed.itemId);
    await page.getByTestId("prepare-start").click();
    await expect(page.getByTestId("prepare-failures")).toBeVisible({ timeout: 60_000 });
    await expect(page.getByTestId("prepare-failures")).toContainText("第 2 页失败");
    expect(puts, "第一轮处理了 1、2 两页（第 2 页失败）").toEqual([1, 2]);
    // 失败页不阻塞其它页：第 1 页已经成功。
    await expect(page.getByTestId("prepare-status")).toContainText("已完成 1 / 2 页");

    // 模拟"关闭标签页后重新进入"：刷新页面，再继续准备。
    await page.unroute("**/pages/2");
    await page.reload();
    await expect(page.getByRole("heading", { name: "资料准备" })).toBeVisible();
    // 重新进入后先用服务端状态显示进度（尚未解析 PDF，因此总页数在开始后才确认）。
    await expect(page.getByTestId("prepare-resume-hint")).toContainText("已完成 1 页");

    const resumedPuts: number[] = [];
    page.on("request", (request) => {
      const match = /\/api\/v1\/preparations\/[^/]+\/pages\/(\d+)$/.exec(request.url());
      if (match !== null && request.method() === "PUT") {
        resumedPuts.push(Number(match[1]));
      }
    });
    await page.getByTestId("prepare-start").click();
    await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });
    expect(resumedPuts, "续传只补缺失页（不重传第 1 页）").toEqual([2]);
    await expect(page.getByTestId("prepare-status")).toContainText("已完成 2 / 2 页");
  });

  test("扫描 PDF：没有文字层时按页图上传", async ({ page, request }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-scan.pdf",
      "T09 扫描型说明书",
    );
    await loginViaUi(page, "", password());
    await openPreparePage(page, seed.itemId);

    const [createResponse] = await Promise.all([
      page.waitForResponse(
        (response) =>
          response.request().method() === "POST" &&
          /\/api\/v1\/documents\/[^/]+\/preparations$/.test(response.url()),
      ),
      page.getByTestId("prepare-start").click(),
    ]);
    const preparationId = ((await createResponse.json()) as { data: { id: string } }).data.id;
    await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });

    const detail = await fetchPreparation(request, apiBase(), preparationId);
    expect(detail.pages.map((p) => p.pageNumber)).toEqual([1, 2]);
    for (const pageRecord of detail.pages) {
      expect(pageRecord.textAssetId, "扫描页没有文字层 → 不上传页文字资产").toBeNull();
      expect(pageRecord.imageAssetId, "扫描页仍必须上传页图").not.toBeNull();
    }
  });

  test("旋转页：viewport 记录旋转角与旋转后尺寸", async ({ page, request }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-rotated.pdf",
      "T09 旋转页说明书",
    );
    await loginViaUi(page, "", password());
    await openPreparePage(page, seed.itemId);

    const [createResponse] = await Promise.all([
      page.waitForResponse(
        (response) =>
          response.request().method() === "POST" &&
          /\/api\/v1\/documents\/[^/]+\/preparations$/.test(response.url()),
      ),
      page.getByTestId("prepare-start").click(),
    ]);
    const preparationId = ((await createResponse.json()) as { data: { id: string } }).data.id;
    await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });

    const detail = await fetchPreparation(request, apiBase(), preparationId);
    const upright = detail.pages.find((p) => p.pageNumber === 1)!;
    const rotated = detail.pages.find((p) => p.pageNumber === 2)!;
    expect(upright.viewport!.rotation).toBe(0);
    expect(rotated.viewport!.rotation, "第 2 页 /Rotate 90 必须体现在 viewport").toBe(90);
    expect(
      rotated.viewport!.width > rotated.viewport!.height,
      "旋转后页图宽高互换（页图坐标以旋转后 viewport 左上角为原点）",
    ).toBe(true);
    expect(upright.viewport!.height).toBeGreaterThan(upright.viewport!.width);
    // 两页文字都能提取（含旋转页）。
    const rotatedText = await fetchAsset(request, apiBase(), rotated.textAssetId!);
    expect(rotatedText.bytes.toString("utf8")).toContain("ROTATE-PAGE-TWO");
  });

  test("非拉丁字体页：CJK 文字提取正确且 CMap 从本地加载", async ({ page, request }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-nonlatin.pdf",
      "T09 非拉丁字体说明书",
    );
    await loginViaUi(page, "", password());
    const requests = recordRequests(page);
    await openPreparePage(page, seed.itemId);

    const [createResponse] = await Promise.all([
      page.waitForResponse(
        (response) =>
          response.request().method() === "POST" &&
          /\/api\/v1\/documents\/[^/]+\/preparations$/.test(response.url()),
      ),
      page.getByTestId("prepare-start").click(),
    ]);
    const preparationId = ((await createResponse.json()) as { data: { id: string } }).data.id;
    await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });

    const detail = await fetchPreparation(request, apiBase(), preparationId);
    const pageRecord = detail.pages[0]!;
    expect(pageRecord.textAssetId, "非拉丁字体页必须有文字层").not.toBeNull();
    const text = await fetchAsset(request, apiBase(), pageRecord.textAssetId!);
    expect(text.bytes.toString("utf8")).toContain("部件一：松开四颗螺丝");

    // CMap 本地加载证据：请求落在 /vendor/pdfjs/cmaps/（同源静态资源），无外部域名。
    expect(
      requests.some((path) => path.startsWith("/vendor/pdfjs/cmaps/")),
      `应出现本地 CMap 请求，实际请求：${requests.join(", ")}`,
    ).toBe(true);
    expect(requests.some((path) => path.includes("cdn"))).toBe(false);
  });

  test("加密 PDF：明确拒绝且不创建任何准备/页记录", async ({ page, request }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-encrypted.pdf",
      "T09 加密说明书",
    );
    await loginViaUi(page, "", password());
    const puts = recordPagePuts(page);
    const preparationRequests: string[] = [];
    page.on("request", (request) => {
      if (/\/preparations(\/|$)/.test(new URL(request.url()).pathname)) {
        preparationRequests.push(`${request.method()} ${new URL(request.url()).pathname}`);
      }
    });

    await openPreparePage(page, seed.itemId);
    await page.getByTestId("prepare-start").click();

    await expect(
      page.getByText("该 PDF 已加密，首版不支持，请先解除加密后再上传"),
    ).toBeVisible();
    await expect(page.getByText("没有创建页记录，也没有任何收费请求")).toBeVisible();
    await expect(page.getByTestId("prepare-seal")).toBeDisabled();
    expect(puts, "加密 PDF 不得上传任何页").toEqual([]);
    expect(
      preparationRequests,
      "加密 PDF 不得创建 preparation 记录（拒绝发生在打开阶段）",
    ).toEqual([]);
  });

  test("101 页 PDF：明确拒绝并说明实际页数", async ({ page, request }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-many-pages.pdf",
      "T09 超页数说明书",
    );
    await loginViaUi(page, "", password());
    const puts = recordPagePuts(page);
    await openPreparePage(page, seed.itemId);
    await page.getByTestId("prepare-start").click();

    await expect(page.getByText("PDF 共 101 页，超过 100 页上限")).toBeVisible();
    await expect(page.getByTestId("prepare-seal")).toBeDisabled();
    expect(puts, "超页数 PDF 不得上传任何页").toEqual([]);
  });

  test("断外网仍可完成准备：worker/CMaps/字体全部本地加载，零外部请求", async ({
    page,
    request,
  }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-nonlatin.pdf",
      "T09 离线准备说明书",
    );
    await loginViaUi(page, "", password());
    const external = await blockExternalRequests(page);
    const requests = recordRequests(page);
    await openPreparePage(page, seed.itemId);

    const [createResponse] = await Promise.all([
      page.waitForResponse(
        (response) =>
          response.request().method() === "POST" &&
          /\/api\/v1\/documents\/[^/]+\/preparations$/.test(response.url()),
      ),
      page.getByTestId("prepare-start").click(),
    ]);
    const preparationId = ((await createResponse.json()) as { data: { id: string } }).data.id;
    await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });

    // 无任何外部请求（CDN 被阻断也不影响），且关键资源确实来自本地 vendor 目录。
    expect(external, `不得出现外部请求：${external.join(", ")}`).toEqual([]);
    // worker 由 Vite 的 `?url` 机制解析：dev 是 `/node_modules/pdfjs-dist/build/pdf.worker.min.mjs`，
    // 生产构建是 `/assets/pdf.worker.min-<hash>.mjs` —— 两者都必须来自本地（不是 CDN）。
    expect(
      requests.some((path) => /pdf\.worker\.min(\.mjs|-[^/]*\.mjs)$/.test(path)),
      `应加载本地 PDF.js worker，实际：${requests.join(", ")}`,
    ).toBe(true);
    expect(
      requests.some((path) => path.startsWith("/vendor/pdfjs/cmaps/")),
      "CMaps 必须来自本地 vendor 目录",
    ).toBe(true);

    const detail = await fetchPreparation(request, apiBase(), preparationId);
    expect(detail.pages[0]!.textAssetId).not.toBeNull();
  });

  test("逐页进度、可取消、离开确认与单 canvas 约束", async ({ page, request }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "T09 进度与取消说明书",
    );
    await loginViaUi(page, "", password());

    // 放慢页图上传，给取消留出稳定窗口（不改生产代码，只在网络层放大延迟）。
    await page.route("**/items/*/assets", async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 700));
      await route.continue();
    });

    const puts = recordPagePuts(page);
    await openPreparePage(page, seed.itemId);
    await page.getByTestId("prepare-start").click();

    // 进度是「第 n / N 页」与已完成页数；progressbar 的 valuenow 是已完成页数。
    const status = page.getByTestId("prepare-status");
    await expect(status).toContainText(/第 \d+ \/ 2 页/, { timeout: 60_000 });
    const progressbar = page.getByRole("progressbar", { name: "已完成的页数" });
    await expect(progressbar).toBeVisible();
    await expect(progressbar).toHaveAttribute("aria-valuemax", "2");

    // 进行中：注册离开确认（beforeunload）。
    const beforeUnloadPrevented = await page.evaluate(() => {
      const event = new Event("beforeunload", { cancelable: true });
      window.dispatchEvent(event);
      return event.defaultPrevented;
    });
    expect(beforeUnloadPrevented, "准备进行中必须阻止直接离开（beforeunload）").toBe(true);
    // 每次只渲染一页：DOM 中没有 canvas 元素。
    await expect(page.locator("canvas")).toHaveCount(0);

    // 取消：停止后续上传，已完成页保留。
    const putsBeforeCancel = puts.length;
    await page.getByTestId("prepare-cancel").click();
    await expect(page.getByTestId("prepare-cancel")).toBeDisabled();
    await page.waitForTimeout(1200);
    expect(puts.length, "取消后不得继续上传新页").toBeLessThanOrEqual(putsBeforeCancel + 1);

    // 取消后回到未开始状态，仍可继续补齐（服务端保留已完成页）。
    await expect(page.getByTestId("prepare-start")).toBeEnabled();
    await page.getByTestId("prepare-start").click();
    await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });
  });
});

/**
 * BUG-001-r8 回归（T08 回合 8 的 P3 缺陷，与 T09 同轮修复）：
 * 成功通知条曾用固定定位的整宽容器盖住顶栏，在 6 秒内吞掉对顶栏「资料库」的指针点击。
 *
 * 修复：`.notices` 容器 `pointer-events: none` + `.notice` 恢复 `pointer-events: auto`。
 * 这里用 `elementFromPoint`（比"点了没反应"强）在**宽屏与窄屏**各验证一次：
 * 通知条可见期间点击顶栏入口必须命中链接本身并完成导航；关闭按钮仍然可点。
 *
 * 放在本文件是为了让 T09 的验收命令（`test:e2e -- pdf-preparation.spec.ts`）
 * 能直接复现该修复的证据（T09 是与修复同轮的切片刻）。
 */
test.describe("BUG-001-r8 回归：通知条不遮挡顶栏指针操作", () => {
  test("通知条出现期间顶栏入口可点击（宽屏 + 窄屏）", async ({ page }) => {
    await loginViaUi(page, "", password());

    for (const width of [1280, 390]) {
      await page.setViewportSize({ width, height: 800 });
      await page.goto("/items/new");
      await page.getByLabel("名称").fill(`通知条回归 ${width}`);
      await page.getByLabel("准确型号").fill(`NOTICE-${width}`);
      await page.getByRole("button", { name: "创建并继续" }).click();

      // 成功通知条出现（6 秒自动消失窗口内做指针命中判定）。
      // 用通知条内的「关闭」按钮定位：`role=status` 在页面上可能还有其它状态区。
      const dismissButton = page.getByRole("button", { name: /^关闭通知：/ }).first();
      await expect(dismissButton).toBeVisible();

      const link = page
        .getByRole("navigation", { name: "主导航" })
        .getByRole("link", { name: "资料库", exact: true });
      const box = await link.boundingBox();
      expect(box, "顶栏「资料库」入口应可见").not.toBeNull();

      const hitAtLinkCenter = async (): Promise<{
        link: string | null;
        className: string;
        topBarHeight: number | null;
        noticesTop: string;
        noticeTop: number | null;
        clickY: number;
      }> =>
        page.evaluate(
          ([x, y]) => {
            const element = document.elementFromPoint(x as number, y as number);
            const topBar = document.querySelector(".top-bar");
            const notice = document.querySelector(".notice");
            return {
              link: element?.closest("a")?.textContent?.trim() ?? null,
              className: typeof element?.className === "string" ? element.className : "",
              topBarHeight:
                topBar === null ? null : Math.round(topBar.getBoundingClientRect().height),
              noticesTop: getComputedStyle(document.documentElement).getPropertyValue(
                "--notices-top",
              ),
              noticeTop: notice === null ? null : Math.round(notice.getBoundingClientRect().top),
              clickY: Math.round(y as number),
            };
          },
          [box!.x + box!.width / 2, box!.y + box!.height / 2],
        );

      // 通知条定位由实测顶栏高度驱动（顶栏会因物品上下文/换行变化），给它一个稳定窗口；
      // 之后必须始终命中顶栏链接本身（而不是通知条）。
      await expect
        .poll(async () => (await hitAtLinkCenter()).link, {
          message: `宽度 ${width}px：通知条出现时顶栏点击被吞`,
          timeout: 3000,
        })
        .toBe("资料库");
      const hit = await hitAtLinkCenter();
      expect(
        hit.link,
        `宽度 ${width}px：通知条出现时顶栏点击被吞（命中 ${JSON.stringify(hit)}）`,
      ).toBe("资料库");

      await link.click();
      await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();

      // 通知条自身仍可交互（关闭按钮可点）。
      if (await dismissButton.isVisible().catch(() => false)) {
        await dismissButton.click();
      }
    }
  });
});
