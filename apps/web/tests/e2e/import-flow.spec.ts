/**
 * T16 e2e：资料库与新建向导（PRD 修订 2 / ui_revision 2；REQ-016/REQ-021/REQ-030；
 * AC-026、AC-030（Playwright 侧）、AC-048（UI 侧）；UI-005/006/009–014/019–026）。
 *
 * 环境：真实 Chrome（Playwright chromium）+ 真实 Rust 后端（globalSetup 启动：
 * 临时 data-dir + `init` + 价格目录 + 本机假凭据与保留端口 base_url，绝不外呼）
 * + Vite 前端；**不依赖任何已运行的外部服务**。
 *
 * 覆盖清单（命令 ↔ AC/UI）：
 * 1. 正常向导：建物品 → 上传 PDF 并绑定 → 上传 front/left 照片排视图 → 浏览器准备并封存
 *    → 报价（分列 credits/USD、价格版本、快照日期、有效期、保守上界）→ 告知确认（默认不勾选）
 *    → 生成（`Idempotency-Key`）→ 进入等待（202 受理），且只有 1 个 job（AC-026/AC-030/AC-048）；
 * 2. 缺 front：生成入口禁用并说明缺项（AC-026/UI-013）；
 * 3. 上传失败重试：超限文件（真实 413）+ 注入一次网络失败后重试成功（AC-019 的 UI 侧/UI-009/UI-010）；
 * 4. 预算变动：低于上界时禁用并说明；报价过期后「重新获取报价」（U-05/UI-022/UI-023/UI-026）；
 * 5. 返回上一步/刷新：只改 URL，不丢服务端已保存的资料（REQ-016/UI-019）；
 * 6. 窄屏（<768px）键盘操作：Tab/Enter 完成第 1 步、抽屉焦点陷阱与 Esc 归还焦点（AC-060/UI-062）。
 */

import { expect, test, type Page, type Request } from "@playwright/test";

import {
  apiLogin,
  captureTo,
  fetchDocuments,
  fetchJobsForItem,
  fetchPhotos,
  loginViaUi,
  runtime,
  seedItem,
  seedItemWithDocument,
  seedPhoto,
  seedReadyPreparation,
  setPreparationPointer,
} from "./helpers";
import { fixturePath } from "./runtime";

const password = (): string => runtime().password;
const apiBase = (): string => runtime().apiBase;

/** 记录发往某路径前缀的请求（方法 + 头），用于幂等键与"没有重复上传"的断言。 */
function recordRequests(page: Page, predicate: (url: string, method: string) => boolean): Request[] {
  const seen: Request[] = [];
  page.on("request", (request) => {
    if (predicate(request.url(), request.method())) {
      seen.push(request);
    }
  });
  return seen;
}

function isAssetsPost(url: string, method: string): boolean {
  return method === "POST" && /\/api\/v1\/items\/[^/]+\/assets$/.test(url);
}

function isPhotosPost(url: string, method: string): boolean {
  return method === "POST" && /\/api\/v1\/items\/[^/]+\/photos$/.test(url);
}

/** 阻断一切非本机请求并记录（离线证据：向导全程不访问外部网络）。 */
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

async function openStep(page: Page, itemId: string, segment: string, heading: string | RegExp): Promise<void> {
  await page.goto(`/items/${itemId}/import/${segment}`);
  await expect(page.getByRole("heading", { name: heading, level: 1 })).toBeVisible();
}

test.describe("资料库与新建向导（T16）", () => {
  test("正常向导：建物品 → PDF → 照片 → 准备 → 报价 → 确认 → 生成进入等待", async ({
    page,
    request,
  }) => {
    const external = await blockExternalRequests(page);
    // APIRequestContext 自身的会话（用于断言服务端事实；与浏览器上下文各自独立）。
    await apiLogin(request, apiBase(), password());
    await loginViaUi(page, "", password());

    // --- 第 1 步：基本信息（真实 POST /items，表单键盘输入） ---------------------
    await page.goto("/items/new");
    await expect(page.getByRole("heading", { name: "新建物品" })).toBeVisible();
    await page.getByLabel("名称").fill("T16 联调相机");
    await page.getByLabel("准确型号").fill("X100V-T16");
    await page.getByRole("button", { name: "创建并继续" }).click();
    await expect(page).toHaveURL(/\/items\/[0-9a-f-]{36}$/);
    const itemId = new URL(page.url()).pathname.split("/").pop() ?? "";
    expect(itemId).not.toBe("");
    await captureTo("t16-rd", page, "01-item-created");

    // --- 第 2 步：说明书（上传 PDF + 绑定 document） ---------------------------
    await page.getByRole("link", { name: "绑定说明书原件（向导第 2 步）" }).click();
    await expect(page.getByRole("heading", { name: "说明书原件" })).toBeVisible();
    await page.getByLabel("选择 PDF 文件").setInputFiles(fixturePath("sample-manual-text.pdf"));
    await expect(page.getByText("待绑定文件：sample-manual-text.pdf", { exact: false })).toBeVisible();
    await page.getByLabel("标题（可选）").fill("T16 联调说明书");
    await page.getByRole("button", { name: "绑定为说明书" }).click();
    await expect(page.getByRole("heading", { name: "已绑定的说明书" })).toBeVisible();
    await expect(page.locator(".entity-list__title").filter({ hasText: "T16 联调说明书" })).toBeVisible();
    const documents = await fetchDocuments(request, apiBase(), itemId);
    expect(documents.map((document) => document.title)).toContain("T16 联调说明书");
    await captureTo("t16-rd", page, "02-document-bound");

    // --- 第 3 步：视图排列（front + left） -------------------------------------
    await page.getByRole("link", { name: /下一步：视图排列/ }).click();
    await expect(page.getByRole("heading", { name: "视图排列" })).toBeVisible();
    await page.getByLabel("上传正面视图照片").setInputFiles(fixturePath("sample-photo-front.jpg"));
    await expect(page.getByTestId("view-slot-front").getByRole("img", { name: "正面视图照片" })).toBeVisible();
    await page.getByLabel("上传左侧视图照片").setInputFiles(fixturePath("sample-photo-left.png"));
    await expect(page.getByTestId("view-slot-left").getByRole("img", { name: "左侧视图照片" })).toBeVisible();
    const photos = await fetchPhotos(request, apiBase(), itemId);
    expect(photos.map((photo) => photo.view).sort()).toEqual(["front", "left"]);
    await captureTo("t16-rd", page, "03-views-arranged");

    // --- 第 4 步：准备（浏览器 PDF.js 逐页准备 + 封存） --------------------------
    await page.getByRole("link", { name: /下一步：准备/ }).click();
    await expect(page.getByRole("heading", { name: "资料准备" })).toBeVisible();
    await page.getByTestId("prepare-start").click();
    await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });
    await page.getByTestId("prepare-seal").click();
    await expect(page.getByTestId("prepare-sealed")).toBeVisible();
    await captureTo("t16-rd", page, "04-preparation-ready");

    // --- 第 5 步：报价与确认 ---------------------------------------------------
    await page.getByRole("link", { name: "下一步：预算/隐私确认" }).click();
    await expect(page.getByRole("heading", { name: "预算与隐私确认" })).toBeVisible();
    const quotePanel = page.getByTestId("quote-panel");
    await expect(quotePanel).toBeVisible({ timeout: 20_000 });

    // 分列金额：Tripo credits 与 Manual AI USD 各自带单位，不相加；含价格版本/快照/有效期。
    await expect(page.getByTestId("quote-tripo-upper")).toContainText("credits");
    await expect(page.getByTestId("quote-manual-upper")).toContainText("USD");
    await expect(quotePanel.getByText("价格版本", { exact: false })).toBeVisible();
    await expect(quotePanel.getByText(/快照日期/)).toBeVisible();
    await expect(page.getByTestId("quote-expiry")).toContainText("剩余");
    await expect(quotePanel.getByText("不是供应商账户级硬封顶")).toBeVisible();

    // 告知：列出将发送给 Tripo 的视图与说明书 AI 的页范围/型号文本/模型名。
    const scope = page.getByTestId("send-scope");
    await expect(scope).toBeVisible();
    await expect(scope.getByText("发送给 Tripo（模型生成）")).toBeVisible();
    await expect(scope.getByText(/front/).first()).toBeVisible();
    await expect(scope.getByText(/left/).first()).toBeVisible();
    await expect(scope.getByText("发送给说明书 AI")).toBeVisible();
    await expect(scope.getByText(/物品身份文本/)).toBeVisible();
    await expect(scope.getByText(/页范围/)).toBeVisible();
    await captureTo("t16-rd", page, "05-quote-and-disclosure");

    // 默认不勾选：生成禁用并说明缺什么（前端不判断远端成功）。
    const generate = page.getByTestId("generate-button");
    await expect(page.getByRole("checkbox", { name: /我已阅读并确认将上述资料发送给对应供应商/ })).not.toBeChecked();
    await expect(generate).toBeDisabled();
    await expect(page.getByTestId("generate-reason")).toContainText("需要先勾选确认");

    // 显式确认（写 audit_events）后可生成。
    await page.getByRole("checkbox", { name: /我已阅读并确认将上述资料发送给对应供应商/ }).check();
    await expect(page.getByTestId("confirmed-at")).toContainText("已确认发送范围");
    await expect(generate).toBeEnabled();
    await captureTo("t16-rd", page, "06-confirmed");

    // --- 生成：建单（Idempotency-Key）→ 进入等待 -------------------------------
    const jobRequests = recordRequests(page, (url, method) =>
      method === "POST" && /\/api\/v1\/items\/[^/]+\/jobs$/.test(url),
    );
    const [jobResponse] = await Promise.all([
      page.waitForResponse(
        (response) =>
          response.request().method() === "POST" && /\/api\/v1\/items\/[^/]+\/jobs$/.test(response.url()),
      ),
      generate.click(),
    ]);
    expect(jobResponse.status(), await jobResponse.text()).toBe(202);
    expect(jobRequests.length).toBe(1);
    const idempotencyKey = jobRequests[0]?.headers()["idempotency-key"];
    expect(idempotencyKey, "建单必须带 Idempotency-Key").toBeTruthy();

    const accepted = page.getByTestId("job-accepted");
    await expect(accepted).toBeVisible();
    await expect(accepted.getByText("任务已受理（202）")).toBeVisible();
    await expect(accepted.getByText("关闭浏览器不影响已提交任务")).toBeVisible();
    await expect(generate).toHaveCount(0);
    await captureTo("t16-rd", page, "07-job-accepted");

    // 服务端事实：只有 1 个 job（重复点击/重放由服务端幂等兜底）。
    const jobs = await fetchJobsForItem(request, apiBase(), itemId);
    expect(jobs.length).toBe(1);
    expect(accepted.getByText(jobs[0]!.id, { exact: false })).toBeVisible();

    // UI-021：物品页出现"进行中任务使用冻结快照"的常驻提示（只读，不做任务中心的功能）。
    await page.goto(`/items/${itemId}`);
    await expect(page.getByTestId("snapshot-notice")).toBeVisible();
    await expect(page.getByTestId("snapshot-notice")).toContainText("冻结的资料快照");
    await captureTo("t16-rd", page, "16-snapshot-notice");

    // 全程没有访问本机之外的地址（外部资源一律阻断）。
    expect(external, `不应访问外部地址：${external.join(", ")}`).toEqual([]);
  });

  test("缺 front：生成入口禁用并说明缺项", async ({ page, request }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "T16 缺 front",
    );
    const csrf = await apiLogin(request, apiBase(), password());
    // 只有 left，没有 front（其余前置保持真实：这里只验证缺视图的呈现）。
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "left", "sample-photo-left.png");

    await loginViaUi(page, "", password());
    await openStep(page, seed.itemId, "confirm", "预算与隐私确认");

    const gaps = page.getByTestId("generation-gaps");
    await expect(gaps).toBeVisible();
    await expect(gaps.getByText("缺少 front（正面）视图照片")).toBeVisible();
    await expect(page.getByTestId("generate-button")).toBeDisabled();
    await expect(page.getByTestId("generate-reason")).toBeVisible();
    // 缺项必须给可行动的修复入口（不出现"静默失败"）。
    await expect(gaps.getByRole("link", { name: "去补充视图" })).toBeVisible();
    await captureTo("t16-rd", page, "08-missing-front");
  });

  test("上传失败重试：超限文件 + 注入一次失败后重试成功", async ({ page, request }) => {
    const itemId = await seedItem(request, apiBase(), password(), "T16 上传失败重试");
    await loginViaUi(page, "", password());
    await openStep(page, itemId, "views", "视图排列");
    // 空态证据：五个空槽位，front 标「必需」、detail 注明不发送给 Tripo。
    await expect(page.getByTestId("view-slot-front")).toContainText("必需");
    await expect(page.getByTestId("view-slot-detail")).toContainText("不发送给 Tripo");
    await captureTo("t16-rd", page, "00-views-empty");

    // (a) 真实超限：21 MiB > 照片单文件上限（20 MiB）→ 413 + 可行动错误卡片。
    const oversized = Buffer.alloc(21 * 1024 * 1024, 0x20);
    oversized[0] = 0xff;
    oversized[1] = 0xd8;
    oversized[2] = 0xff;
    await page.getByLabel("上传正面视图照片").setInputFiles({
      name: "oversized.jpg",
      mimeType: "image/jpeg",
      buffer: oversized,
    });
    const errorCard = page.getByTestId("photo-upload-front").getByRole("alert");
    await expect(errorCard).toBeVisible({ timeout: 30_000 });
    await expect(errorCard.getByText(/超过大小上限|磁盘空间不足|未通过校验/)).toBeVisible();
    await expect(errorCard.getByRole("button", { name: "重试" })).toBeVisible();
    // 重试沿用的还是同一个超限文件：仍然失败（证明真的重新上传过，而不是假装成功）。
    const attempts = recordRequests(page, isAssetsPost);
    await errorCard.getByRole("button", { name: "重试" }).click();
    await expect(errorCard).toBeVisible();
    expect(attempts.length).toBeGreaterThanOrEqual(1);
    await captureTo("t16-rd", page, "09-upload-too-large");
    await errorCard.getByRole("button", { name: "移除" }).click();
    await expect(errorCard).toHaveCount(0);

    // (b) 注入网络失败：第一次 POST /assets 直接断开，错误卡片出现后「重试」成功。
    let injected = false;
    await page.route("**/api/v1/items/*/assets", async (route) => {
      if (route.request().method() === "POST" && !injected) {
        injected = true;
        await route.abort("connectionfailed");
        return;
      }
      await route.continue();
    });
    await page.getByLabel("上传左侧视图照片").setInputFiles(fixturePath("sample-photo-left.png"));
    const leftError = page.getByTestId("photo-upload-left").getByRole("alert");
    await expect(leftError).toBeVisible();
    await expect(leftError.getByText("上传失败")).toBeVisible();
    await captureTo("t16-rd", page, "10-upload-network-error");
    await leftError.getByRole("button", { name: "重试" }).click();
    await expect(page.getByTestId("view-slot-left").getByRole("img", { name: "左侧视图照片" })).toBeVisible({
      timeout: 20_000,
    });
    await page.unroute("**/api/v1/items/*/assets");
    const photos = await fetchPhotos(request, apiBase(), itemId);
    expect(photos.map((photo) => photo.view)).toEqual(["left"]);
  });

  test("预算变动：低于上界被禁用；报价过期后重新获取报价（不自动重报）", async ({
    page,
    request,
  }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "T16 预算变动",
    );
    const csrf = await apiLogin(request, apiBase(), password());
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "front", "sample-photo-front.jpg");
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "left", "sample-photo-left.png");
    const preparationId = await seedReadyPreparation(request, apiBase(), csrf, seed);

    // 报价过期注入：把下一次 estimate 响应的 expiresAt 改写为过去（故障注入，不修改服务端）。
    let injectExpired = true;
    await page.route("**/api/v1/items/*/estimates", async (route) => {
      if (route.request().method() !== "POST" || !injectExpired) {
        await route.continue();
        return;
      }
      injectExpired = false;
      const response = await route.fetch();
      const body = (await response.json()) as { data: { expiresAt: string } };
      body.data.expiresAt = new Date(Date.now() - 60_000).toISOString();
      await route.fulfill({ response, json: body });
    });

    await loginViaUi(page, "", password());
    await setPreparationPointer(page, seed.itemId, preparationId);
    await openStep(page, seed.itemId, "confirm", "预算与隐私确认");

    // 过期报价：生成禁用 + 明确文案 + 「重新获取报价」入口（U-05：不自动重新报价）。
    await expect(page.getByTestId("quote-panel")).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId("quote-expiry")).toContainText("已过期");
    await expect(page.getByTestId("generate-button")).toBeDisabled();
    await expect(page.getByTestId("generate-reason")).toContainText("报价已过期");
    const requote = page.getByTestId("requote-button");
    await expect(requote).toBeVisible();
    await captureTo("t16-rd", page, "11-quote-expired");

    // 重新获取报价：真实（未被注入改写）的报价回来，有效期恢复。
    await page.unroute("**/api/v1/items/*/estimates");
    await requote.click();
    await expect(page.getByTestId("quote-expiry")).toContainText("剩余", { timeout: 20_000 });

    // 预算变动：把 Tripo 上限改到低于报价上界 → 生成禁用并说明；改回上界以上 → 恢复可用。
    await page.getByRole("checkbox", { name: /我已阅读并确认将上述资料发送给对应供应商/ }).check();
    await expect(page.getByTestId("confirmed-at")).toBeVisible();
    const generate = page.getByTestId("generate-button");
    await expect(generate).toBeEnabled();
    const tripoUpper = await page.getByTestId("quote-tripo-upper").innerText();
    expect(tripoUpper).toContain("credits");
    await page.getByLabel("Tripo credits").fill("1.00");
    await expect(generate).toBeDisabled();
    await expect(page.getByTestId("generate-reason")).toContainText("低于本次报价的保守上界");
    await captureTo("t16-rd", page, "12-budget-below-bound");
    await page.getByLabel("Tripo credits").fill("50.00");
    await expect(generate).toBeEnabled();
  });

  test("返回上一步与刷新：只改 URL，不丢服务端已保存的资料", async ({ page, request }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "T16 返回上一步",
    );
    const csrf = await apiLogin(request, apiBase(), password());
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "front", "sample-photo-front.jpg");
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "left", "sample-photo-left.png");

    await loginViaUi(page, "", password());
    await openStep(page, seed.itemId, "views", "视图排列");
    await expect(page.getByTestId("view-slot-front").getByRole("img", { name: "正面视图照片" })).toBeVisible();
    await expect(page.getByTestId("view-slot-left").getByRole("img", { name: "左侧视图照片" })).toBeVisible();

    const assetPosts = recordRequests(page, isAssetsPost);
    const photoPosts = recordRequests(page, isPhotosPost);

    // 上一步：只改 URL（说明书来自服务端）。
    await page.getByRole("link", { name: /上一步：说明书/ }).click();
    await expect(page).toHaveURL(new RegExp(`/items/${seed.itemId}/import/document$`));
    await expect(page.getByText("T16 返回上一步 说明书")).toBeVisible();

    // 下一步回到视图排列：照片仍在（服务端事实），且没有重复上传/登记。
    await page.getByRole("link", { name: /下一步：视图排列/ }).click();
    await expect(page).toHaveURL(new RegExp(`/items/${seed.itemId}/import/views$`));
    await expect(page.getByTestId("view-slot-front").getByRole("img", { name: "正面视图照片" })).toBeVisible();
    await expect(page.getByTestId("view-slot-left").getByRole("img", { name: "左侧视图照片" })).toBeVisible();
    expect(assetPosts.length, "返回/前进不应重新上传资产").toBe(0);
    expect(photoPosts.length, "返回/前进不应重新登记照片").toBe(0);

    // 刷新：仍从服务端恢复（URL 即步骤，不用内存状态代替）。
    await page.reload();
    await expect(page.getByTestId("view-slot-front").getByRole("img", { name: "正面视图照片" })).toBeVisible();
    const photos = await fetchPhotos(request, apiBase(), seed.itemId);
    expect(photos.map((photo) => photo.view).sort()).toEqual(["front", "left"]);
    await captureTo("t16-rd", page, "13-back-forward-refresh");
  });

  test("窄屏（<768px）键盘：Tab/Enter 完成第 1 步，抽屉焦点陷阱与 Esc", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await loginViaUi(page, "", password());

    // 键盘完成第 1 步：Tab 到名称/型号字段，Enter 提交。
    await page.goto("/items/new");
    await expect(page.getByRole("heading", { name: "新建物品" })).toBeVisible();
    await expect
      .poll(async () => {
        for (let index = 0; index < 12; index += 1) {
          const id = await page.evaluate(() => document.activeElement?.id ?? "");
          if (id === "field-name") {
            return true;
          }
          await page.keyboard.press("Tab");
        }
        return false;
      })
      .toBe(true);
    await page.keyboard.type("T16 ");
    await page.keyboard.insertText("窄屏相机");
    await page.keyboard.press("Tab"); // → 型号
    await page.keyboard.type("NARROW-T16");
    // Tab 到「创建并继续」按钮后 Enter 提交（隐式表单提交）。
    await expect
      .poll(async () => {
        for (let index = 0; index < 6; index += 1) {
          const label = await page.evaluate(() => document.activeElement?.textContent ?? "");
          if (label.includes("创建并继续")) {
            return true;
          }
          await page.keyboard.press("Tab");
        }
        return false;
      })
      .toBe(true);
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/items\/[0-9a-f-]{36}$/);
    const itemId = new URL(page.url()).pathname.split("/").pop() ?? "";
    await captureTo("t16-rd", page, "14-narrow-keyboard-create");

    // 窄屏抽屉：面板触发按钮键盘可达，打开后焦点进入抽屉，Esc 关闭并归还焦点。
    await openStep(page, itemId, "confirm", "预算与隐私确认");
    const trigger = page.getByRole("button", { name: "将发送的资料与确认" });
    await expect(trigger).toBeVisible();
    await trigger.focus();
    await page.keyboard.press("Enter");
    const drawer = page.getByRole("dialog", { name: "将发送的资料与确认" });
    await expect(drawer).toBeVisible();
    const focusInside = await drawer.evaluate((node) => node.contains(document.activeElement));
    expect(focusInside, "抽屉打开后焦点应在抽屉内").toBe(true);
    await captureTo("t16-rd", page, "15-narrow-drawer");
    await page.keyboard.press("Escape");
    await expect(drawer).toHaveCount(0);
    const focusReturned = await trigger.evaluate((node) => document.activeElement === node);
    expect(focusReturned, "Esc 关闭后焦点应回到触发按钮").toBe(true);
  });
});
