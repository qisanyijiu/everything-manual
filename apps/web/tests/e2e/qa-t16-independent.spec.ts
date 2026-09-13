/**
 * QA 回合 18（T16 独立验收）—— 资料库与新建向导。
 *
 * 本文件由 QA 编写，独立于 RD 的 `import-flow.spec.ts`：断言依据是 PRD 修订 2 的
 * REQ-016 / REQ-021 / REQ-030、AC-026 / AC-030（Playwright 侧）/ AC-048（UI 侧）、
 * §6.2 的 UI-005/006/009–026 与 §6.3.2 禁用措辞清单；证据来自 DOM、网络与服务端事实，
 * 不采信 RD 的 spec 结论。覆盖（命令 ↔ AC/UI）：
 *
 * 1. 资料库空/加载/失败三态与行内禁用入口、新建表单无物品级来源链接（AC-026 / UI-005/006）；
 * 2. 第 2/3/5 步刷新不丢资料、URL 即步骤、零重复上传（AC-026 / REQ-016 / UI-019）；
 * 3. 确认页逐项告知、默认不勾选、未确认提交被服务端拒绝且不产生 job（AC-030 / UI-024）；
 * 4. 失败重试复用同一 `Idempotency-Key`，服务端只产生一个 job（AC-026 / UI-020/025）；
 * 5. 受理后不判断远端成功、无自动发布入口、全程无禁用措辞、准备进度非百分比（AC-048 / §6.3.2）；
 * 6. 同视图占用 422 的真实服务端拒绝与可行动文案、detail 不进入发送集合（UI-012/013）；
 * 7. 缺侧面视图：生成禁用、缺项常驻并给修复链接、不提前报价（AC-026 / UI-013）；
 * 8. 向导第 2/5 步失败态可行动（重试恢复）（AC-026 / UI-019/026）；
 * 9. 窄屏/桌面断点：同一 URL 同一数据，抽屉与并排切换无需刷新（AC-060 前端侧 / UI-062）。
 *
 * 环境：与仓库既有 e2e 相同（真实 Rust 后端 + 临时 data-dir + 价格目录 +
 * `127.0.0.1:1` 假 Provider）；浏览器侧阻断一切非本机请求并断言为空。
 */

import fs from "node:fs";
import path from "node:path";

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

/** `POST/GET ...` 请求记录（网络层独立观察）。 */
function recordRequests(page: Page, predicate: (url: string, method: string) => boolean): Request[] {
  const seen: Request[] = [];
  page.on("request", (request) => {
    if (predicate(request.url(), request.method())) {
      seen.push(request);
    }
  });
  return seen;
}

/** 阻断一切非本机请求并记录（"零真实外网"的浏览器侧证据）。 */
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

interface ApiResult {
  readonly status: number;
  readonly body: unknown;
  readonly headers: Record<string, string>;
}

/** 直接调用服务端合同（独立于页面）。 */
async function apiCall(
  request: import("@playwright/test").APIRequestContext,
  options: {
    method: "GET" | "POST" | "PATCH" | "PUT";
    url: string;
    csrf?: string;
    data?: unknown;
    idempotencyKey?: string;
  },
): Promise<ApiResult> {
  const headers: Record<string, string> = {};
  if (options.csrf !== undefined) {
    headers["x-csrf-token"] = options.csrf;
  }
  if (options.idempotencyKey !== undefined) {
    headers["idempotency-key"] = options.idempotencyKey;
  }
  const response = await request.fetch(options.url, {
    method: options.method,
    headers,
    data: options.data as never,
  });
  const text = await response.text();
  return {
    status: response.status(),
    body: text === "" ? null : (JSON.parse(text) as unknown),
    headers: response.headers(),
  };
}

interface SeededQuote {
  readonly id: string;
  readonly upperMinor: { tripo: number; usd: number };
  readonly display: { tripo: string; usd: string };
  readonly priceVersion: string;
  readonly priceSnapshotDate: string;
  readonly pageCount: number;
  readonly pageRange: { from: number; to: number };
  readonly tripoModel: string;
  readonly manualModel: string;
  readonly manualPromptVersion: string;
  readonly itemName: string;
  readonly itemModel: string;
}

/** 通过服务端合同独立取得一份报价（用于与页面展示交叉核对）。 */
async function fetchQuoteViaApi(
  request: import("@playwright/test").APIRequestContext,
  csrf: string,
  itemId: string,
  preparationId: string,
  photoIds: string[],
): Promise<SeededQuote> {
  const response = await apiCall(request, {
    method: "POST",
    url: `${apiBase()}/api/v1/items/${itemId}/estimates`,
    csrf,
    data: { preparationId, photoIds, modelPreset: "tripo-h-v3.1-standard" },
  });
  expect(response.status, JSON.stringify(response.body)).toBe(201);
  const data = (response.body as { data: Record<string, never> }).data as unknown as {
    id: string;
    amounts: {
      tripo: { upperBoundMinor: number; upperBoundDisplay: string };
      manualAi: { upperBoundMinor: number; upperBoundDisplay: string };
    };
    priceVersion: string;
    priceSnapshotDate: string;
    pageCount: number;
    pageRange: { from: number; to: number };
    sendScope: {
      tripo: { model: string };
      manualAi: { model: string; promptVersion: string; itemName: string; itemModel: string };
    };
  };
  return {
    id: data.id,
    upperMinor: {
      tripo: data.amounts.tripo.upperBoundMinor,
      usd: data.amounts.manualAi.upperBoundMinor,
    },
    display: {
      tripo: data.amounts.tripo.upperBoundDisplay,
      usd: data.amounts.manualAi.upperBoundDisplay,
    },
    priceVersion: data.priceVersion,
    priceSnapshotDate: data.priceSnapshotDate,
    pageCount: data.pageCount,
    pageRange: data.pageRange,
    tripoModel: data.sendScope.tripo.model,
    manualModel: data.sendScope.manualAi.model,
    manualPromptVersion: data.sendScope.manualAi.promptVersion,
    itemName: data.sendScope.manualAi.itemName,
    itemModel: data.sendScope.manualAi.itemModel,
  };
}

/** 打开第 5 步并等待报价（页面自动请求一次报价）。 */
async function openConfirmWithQuote(page: Page, itemId: string): Promise<void> {
  await page.goto(`/items/${itemId}/import/confirm`);
  await expect(page.getByRole("heading", { name: "预算与隐私确认" })).toBeVisible();
  await expect(page.getByTestId("quote-panel")).toBeVisible({ timeout: 20_000 });
}

const CONFIRM_LABEL = /我已阅读并确认将上述资料发送给对应供应商/;

async function confirmSendScope(page: Page): Promise<void> {
  await page.getByRole("checkbox", { name: CONFIRM_LABEL }).check();
  await expect(page.getByTestId("confirmed-at")).toContainText("已确认发送范围");
}

test.describe("T16 独立验收（QA 回合 18）", () => {
  test("QA-1 资料库三态与行内禁用入口；新建表单无物品级来源链接（AC-026）", async ({
    page,
    request,
  }) => {
    await apiLogin(request, apiBase(), password());
    await loginViaUi(page, "", password());
    const itemsPattern = /\/api\/v1\/items(\?.*)?$/;

    // (a) 空态：用服务端真实 envelope 把集合置空（不修改服务端数据，只验证空态渲染）。
    await page.route(itemsPattern, async (route) => {
      if (route.request().method() !== "GET") {
        await route.continue();
        return;
      }
      const response = await route.fetch();
      const body = (await response.json()) as Record<string, unknown>;
      await route.fulfill({ response, json: { ...body, data: [], nextCursor: null } });
    });
    await page.goto("/");
    const empty = page.locator(".empty-state");
    await expect(empty).toBeVisible();
    await expect(empty.getByRole("heading", { name: "还没有物品" })).toBeVisible();
    await expect(empty.getByRole("link", { name: "新建物品" })).toBeVisible();
    await captureTo("t16-qa", page, "01-library-empty");

    // (b) 加载态：延迟响应，骨架必须可见（不是空白页）。
    await page.unroute(itemsPattern);
    await page.route(itemsPattern, async (route) => {
      if (route.request().method() !== "GET") {
        await route.continue();
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, 1500));
      await route.continue();
    });
    await page.reload();
    await expect(page.getByText("正在加载物品…")).toBeVisible();
    await captureTo("t16-qa", page, "02-library-loading");
    await expect(page.getByText("正在加载物品…")).toHaveCount(0, { timeout: 15_000 });

    // (c) 失败态：500 + 合同错误体 → 可行动恢复（重试后恢复真实列表）。
    await page.unroute(itemsPattern);
    await page.route(itemsPattern, async (route) => {
      if (route.request().method() !== "GET") {
        await route.continue();
        return;
      }
      await route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({
          error: {
            code: "UNAVAILABLE",
            message: "服务暂不可用（QA 注入）",
            requestId: "qa-t16-503",
            details: null,
          },
        }),
      });
    });
    await page.reload();
    const failure = page.locator(".error-panel");
    await expect(failure.getByRole("heading", { name: "加载失败" })).toBeVisible();
    await expect(failure).toContainText("服务暂不可用（QA 注入）");
    const retry = failure.getByRole("button", { name: "重试" });
    await expect(retry).toBeVisible();
    await captureTo("t16-qa", page, "03-library-error");
    await page.unroute(itemsPattern);
    await retry.click();
    await expect(page.locator(".error-panel")).toHaveCount(0);

    // (d) 新建：请求体只含合同字段（无物品级 sourceUrl），表单无来源链接字段。
    const createPosts = recordRequests(
      page,
      (url, method) => method === "POST" && /\/api\/v1\/items$/.test(url),
    );
    await page.goto("/items/new");
    const form = page.locator(".item-form");
    await expect(form.getByLabel("名称")).toBeVisible();
    await expect(form.getByLabel("准确型号")).toBeVisible();
    await expect(form.getByLabel(/来源|链接|出处|URL/i)).toHaveCount(0);
    await form.getByLabel("名称").fill("QA 资料库行内入口");
    await form.getByLabel("准确型号").fill("QA-T16-ROW");
    await form.getByRole("button", { name: "创建并继续" }).click();
    await expect(page).toHaveURL(/\/items\/[0-9a-f-]{36}$/);
    const createdBody = JSON.parse(createPosts[0]?.postData() ?? "{}") as Record<string, unknown>;
    expect(Object.keys(createdBody).sort()).toEqual(["brand", "model", "name", "variant"]);
    expect(JSON.stringify(createdBody)).not.toContain("sourceUrl");

    // (d2) 物品概览：未交付能力如实说明，不留假装可用的入口。
    // 事实更新（QA 回合 21，T17 已交付）：原断言"任务中心（T17）与阅读器（T18/T19）尚未交付"
    // 已不成立；现在只有阅读器未交付，任务入口指向真实任务中心。守卫不变：未交付能力没有可用入口。
    await expect(page.getByText("阅读器（T18/T19）尚未交付")).toBeVisible();
    await expect(page.getByRole("link", { name: "本物品的任务" })).toHaveAttribute(
      "href",
      /\/jobs\?itemId=/,
    );
    await expect(page.getByRole("link", { name: /查看任务|打开说明书/ })).toHaveCount(0);

    // (e) 第 2 步：未绑定说明书时「下一步」禁用并说明缺什么（UI-019）。
    const newItemId = new URL(page.url()).pathname.split("/").pop() ?? "";
    await page.goto(`/items/${newItemId}/import/document`);
    const nextToViews = page.getByRole("button", { name: /下一步：视图排列/ });
    await expect(nextToViews).toBeDisabled();
    await expect(page.locator("#wizard-next-reason")).toContainText("先绑定一份说明书原件");
    await expect(nextToViews).toHaveAttribute("aria-describedby", "wizard-next-reason");

    // (f) 行内入口：不可用能力禁用并写明原因（不假装可用）。
    await page.goto("/");
    const row = page.locator(".item-row").filter({ hasText: "QA 资料库行内入口" });
    await expect(row).toBeVisible();
    await expect(row.getByRole("link", { name: "继续准备" })).toHaveAttribute(
      "href",
      /\/import\/prepare$/,
    );
    // 事实更新（QA 回合 21，T17 已交付）：「查看任务」现在是可用的真实入口（按物品过滤）；
    // 「打开说明书」（T18/T19）仍禁用并写明原因——守卫不变：未交付能力不假装可用。
    const viewJobs = row.getByRole("link", { name: "查看任务" });
    const openManual = row.getByRole("button", { name: "打开说明书" });
    await expect(viewJobs).toHaveAttribute("href", /\/jobs\?itemId=/);
    await expect(openManual).toBeDisabled();
    await expect(openManual).toHaveAttribute("title", /T18/);
    await expect(row.locator(".item-row__note")).toContainText("尚未交付");
    await captureTo("t16-qa", page, "04-library-row-disabled");
  });

  test("QA-2 第 2/3/5 步刷新不丢资料、URL 即步骤、零重复上传（AC-026 / REQ-016）", async ({
    page,
    request,
  }) => {
    const external = await blockExternalRequests(page);
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "QA 刷新保留",
    );
    const csrf = await apiLogin(request, apiBase(), password());
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "front", "sample-photo-front.jpg");
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "left", "sample-photo-left.png");
    const preparationId = await seedReadyPreparation(request, apiBase(), csrf, seed);

    await loginViaUi(page, "", password());
    await setPreparationPointer(page, seed.itemId, preparationId);

    const assetPosts = recordRequests(
      page,
      (url, method) => method === "POST" && /\/api\/v1\/items\/[^/]+\/assets$/.test(url),
    );
    const photoPosts = recordRequests(
      page,
      (url, method) => method === "POST" && /\/api\/v1\/items\/[^/]+\/photos$/.test(url),
    );

    // 第 2 步：刷新前后都从服务端恢复已绑定说明书。
    await page.goto(`/items/${seed.itemId}/import/document`);
    await expect(page.getByText("QA 刷新保留 说明书")).toBeVisible();
    await expect(page.locator('.wizard-steps [aria-current="step"]')).toHaveText("说明书");
    await page.reload();
    await expect(page.getByText("QA 刷新保留 说明书")).toBeVisible();
    await captureTo("t16-qa", page, "05-document-after-reload");

    // 第 3 步：刷新前后照片仍在（服务端事实）。
    await page.goto(`/items/${seed.itemId}/import/views`);
    await expect(page.getByTestId("view-slot-front").getByRole("img")).toBeVisible();
    await page.reload();
    await expect(page.getByTestId("view-slot-front").getByRole("img")).toBeVisible();
    await expect(page.getByTestId("view-slot-left").getByRole("img")).toBeVisible();
    await expect(page.locator('.wizard-steps [aria-current="step"]')).toHaveText("视图排列");
    await captureTo("t16-qa", page, "06-views-after-reload");

    // 第 5 步：刷新后重报价、勾选复位、生成仍禁用；准备状态来自服务端。
    await page.goto(`/items/${seed.itemId}/import/confirm`);
    await expect(page.getByTestId("quote-panel")).toBeVisible({ timeout: 20_000 });
    await page.reload();
    await expect(page.getByRole("heading", { name: "预算与隐私确认" })).toBeVisible();
    await expect(page.getByTestId("quote-panel")).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId("generation-gaps")).toContainText("资料已齐");
    await expect(page.getByRole("checkbox", { name: CONFIRM_LABEL })).not.toBeChecked();
    await expect(page.getByTestId("generate-button")).toBeDisabled();
    await captureTo("t16-qa", page, "07-confirm-after-reload");

    // 服务端事实：照片与说明书未重复创建；返回/刷新不触发任何重复上传。
    const photos = await fetchPhotos(request, apiBase(), seed.itemId);
    expect(photos.map((photo) => photo.view).sort()).toEqual(["front", "left"]);
    const documents = await fetchDocuments(request, apiBase(), seed.itemId);
    expect(documents.length).toBe(1);
    expect(assetPosts.length, "刷新不应重新上传资产").toBe(0);
    expect(photoPosts.length, "刷新不应重新登记照片").toBe(0);
    expect(external, `不应访问外部地址：${external.join(", ")}`).toEqual([]);
  });

  test("QA-3 确认页逐项告知、默认不勾选、未确认提交被拒（AC-030 / UI-024）", async ({
    page,
    request,
  }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "QA 告知确认",
    );
    const csrf = await apiLogin(request, apiBase(), password());
    const front = await seedPhoto(request, apiBase(), csrf, seed.itemId, "front", "sample-photo-front.jpg");
    const left = await seedPhoto(request, apiBase(), csrf, seed.itemId, "left", "sample-photo-left.png");
    const detail = await seedPhoto(request, apiBase(), csrf, seed.itemId, "detail", "sample-photo-front.jpg");
    const preparationId = await seedReadyPreparation(request, apiBase(), csrf, seed);

    // 服务端事实：报价/发送范围由独立 API 调用取得，用于交叉核对页面展示。
    const quote = await fetchQuoteViaApi(request, csrf, seed.itemId, preparationId, [
      front.photoId,
      left.photoId,
    ]);

    await loginViaUi(page, "", password());
    await setPreparationPointer(page, seed.itemId, preparationId);

    const confirmPosts = recordRequests(
      page,
      (url, method) => method === "POST" && /\/estimates\/[^/]+\/confirm$/.test(url),
    );
    const estimatePosts = recordRequests(
      page,
      (url, method) => method === "POST" && /\/estimates$/.test(url),
    );
    await openConfirmWithQuote(page, seed.itemId);

    // 前端只把 front+left（不含 detail）放进报价请求体。
    const estimateBody = JSON.parse(estimatePosts[0]?.postData() ?? "{}") as { photoIds?: string[] };
    expect(estimateBody.photoIds).toEqual([front.photoId, left.photoId]);
    expect(estimateBody.photoIds).not.toContain(detail.photoId);

    // 默认不勾选：未确认前没有任何 /confirm 调用，生成禁用并说明原因。
    const checkbox = page.getByRole("checkbox", { name: CONFIRM_LABEL });
    await expect(checkbox).not.toBeChecked();
    await expect(page.getByTestId("generate-button")).toBeDisabled();
    await expect(page.getByTestId("generate-reason")).toContainText("需要先勾选确认");
    expect(confirmPosts.length, "未确认时不应调用 confirm").toBe(0);

    // 逐项告知：Tripo 视图、说明书 AI 页范围/页文字页/型号文本、模型名、价格版本与上界。
    const scope = page.getByTestId("send-scope");
    await expect(scope).toContainText("发送给 Tripo（模型生成）");
    await expect(scope).toContainText(`发送给说明书 AI`);
    await expect(scope).toContainText(quote.tripoModel);
    await expect(scope).toContainText(quote.manualModel);
    await expect(scope).toContainText(quote.manualPromptVersion);
    await expect(scope).toContainText(quote.itemName);
    await expect(scope).toContainText(quote.itemModel);
    await expect(scope).toContainText(`第 ${quote.pageRange.from}–${quote.pageRange.to} 页`);
    await expect(scope).toContainText("页文字页");
    await expect(scope).toContainText("页图页");
    await expect(scope).toContainText(quote.priceVersion);
    await expect(scope).toContainText(quote.priceSnapshotDate);
    await expect(page.getByTestId("quote-tripo-upper")).toHaveText(quote.display.tripo);
    await expect(page.getByTestId("quote-manual-upper")).toHaveText(quote.display.usd);
    await captureTo("t16-qa", page, "08-send-scope");

    // 服务端拒绝：未确认的提交 422（不创建 job、不产生第二次尝试）。
    const unconfirmed = await apiCall(request, {
      method: "POST",
      url: `${apiBase()}/api/v1/items/${seed.itemId}/jobs`,
      csrf,
      idempotencyKey: `qa-t16-unconfirmed-${Date.now()}`,
      data: {
        quoteId: quote.id,
        preparationId,
        photoIds: [front.photoId, left.photoId],
        limits: { tripoCreditMinor: quote.upperMinor.tripo, manualAiUsdMicros: quote.upperMinor.usd },
      },
    });
    expect(unconfirmed.status, JSON.stringify(unconfirmed.body)).toBe(422);
    const reason = (unconfirmed.body as { error?: { details?: { reason?: string } } }).error?.details
      ?.reason;
    expect(reason).toBe("confirmationRequired");
    const jobsAfterReject = await fetchJobsForItem(request, apiBase(), seed.itemId);
    expect(jobsAfterReject.length, "未确认提交不应产生 job").toBe(0);

    // 显式确认后才允许提交（确认动作写 audit_events 由 T11 用例覆盖）。
    await checkbox.check();
    await expect(page.getByTestId("confirmed-at")).toContainText("已确认发送范围");
    await expect(page.getByTestId("generate-button")).toBeEnabled();
    expect(confirmPosts.length, "勾选动作恰好一次 confirm").toBe(1);
    await captureTo("t16-qa", page, "09-confirmed");
  });

  test("QA-4 失败重试复用幂等键，服务端只产生一个 job（AC-026 / UI-020）", async ({
    page,
    request,
  }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "QA 幂等",
    );
    const csrf = await apiLogin(request, apiBase(), password());
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "front", "sample-photo-front.jpg");
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "left", "sample-photo-left.png");
    const preparationId = await seedReadyPreparation(request, apiBase(), csrf, seed);

    await loginViaUi(page, "", password());
    await setPreparationPointer(page, seed.itemId, preparationId);
    await openConfirmWithQuote(page, seed.itemId);
    await confirmSendScope(page);

    const jobPosts: { key: string | undefined; body: string | null }[] = [];
    let attempt = 0;
    await page.route(/\/api\/v1\/items\/[^/]+\/jobs$/, async (route) => {
      if (route.request().method() !== "POST") {
        await route.continue();
        return;
      }
      attempt += 1;
      jobPosts.push({
        key: route.request().headers()["idempotency-key"],
        body: route.request().postData(),
      });
      if (attempt === 1) {
        // 跨卡知识采样：Playwright 在请求阶段能看到的头名（T17/T19 断言 CSRF/幂等头时参考）。
        const dir = path.join(
          path.resolve(import.meta.dirname, "../../../../artifacts/web-mvp/t16-qa"),
        );
        fs.mkdirSync(dir, { recursive: true });
        fs.writeFileSync(
          path.join(dir, "playwright-request-headers.json"),
          JSON.stringify(
            {
              url: route.request().url(),
              method: route.request().method(),
              headerNames: Object.keys(route.request().headers()).sort(),
              hasCsrfHeader: "x-csrf-token" in route.request().headers(),
              hasIfMatch: "if-match" in route.request().headers(),
            },
            null,
            2,
          ),
        );
      }
      if (attempt === 1) {
        // 模拟"服务端已受理但响应丢失"：真实发出请求，丢弃响应。
        await route.fetch();
        await route.abort("failed");
        return;
      }
      // 第二次（用户重试）：延迟 1.2s，用于观察提交中的禁用与 aria-busy。
      await new Promise((resolve) => setTimeout(resolve, 1200));
      await route.continue();
    });

    const generate = page.getByTestId("generate-button");
    await generate.click();
    const submitError = page.getByTestId("submit-error");
    await expect(submitError).toBeVisible();
    await expect(submitError).toContainText("网络");
    // 未受理：界面不写"成功"，生成按钮保留供重试。
    await expect(page.getByTestId("job-accepted")).toHaveCount(0);
    await captureTo("t16-qa", page, "10-submit-network-error");

    const replayPromise = page.waitForResponse(
      (response) =>
        response.request().method() === "POST" && /\/api\/v1\/items\/[^/]+\/jobs$/.test(response.url()),
    );
    await generate.click();
    // 提交中：按钮禁用且 aria-busy（防重复点击的前端侧；重试请求被延迟 1.2s）。
    await expect(generate).toBeDisabled();
    await expect(generate).toHaveAttribute("aria-busy", "true");
    await expect(generate).toContainText("正在创建任务");
    const replay = await replayPromise;

    await expect(page.getByTestId("job-accepted")).toBeVisible();
    await expect(page.getByTestId("job-accepted")).toContainText("任务已受理（202）");
    expect(replay.status()).toBe(202);
    expect(replay.headers()["x-idempotent-replay"], "重放应命中服务端幂等").toBe("true");

    // 前端复用同一幂等键：两次 POST 的键必须一致。
    expect(jobPosts.length).toBe(2);
    expect(jobPosts[0]?.key).toBeTruthy();
    expect(jobPosts[1]?.key).toBe(jobPosts[0]?.key);

    // 服务端事实：只有一个 job，页面显示的就是它。
    const jobs = await fetchJobsForItem(request, apiBase(), seed.itemId);
    expect(jobs.length, "重复提交只允许 1 个 job").toBe(1);
    await expect(page.getByTestId("job-accepted")).toContainText(jobs[0]!.id);

    // 服务端幂等（独立于页面，直接打合同）：同键同 body 重放 → 202 + x-idempotent-replay，仍 1 个 job。
    const replayBody = JSON.parse(jobPosts[0]?.body ?? "{}") as Record<string, unknown>;
    const sameKeyHeaders = {
      "x-csrf-token": csrf,
      "idempotency-key": jobPosts[0]!.key ?? "",
      "content-type": "application/json",
    };
    const replaySame = await request.fetch(`${apiBase()}/api/v1/items/${seed.itemId}/jobs`, {
      method: "POST",
      headers: sameKeyHeaders,
      data: replayBody as never,
    });
    expect(replaySame.status(), await replaySame.text()).toBe(202);
    expect(replaySame.headers()["x-idempotent-replay"]).toBe("true");
    // 同键不同 body → 409（不新建第二份）。
    const conflictBody = { ...replayBody, limits: { ...(replayBody.limits as object), tripoCreditMinor: 999 } };
    const conflict = await request.fetch(`${apiBase()}/api/v1/items/${seed.itemId}/jobs`, {
      method: "POST",
      headers: sameKeyHeaders,
      data: conflictBody as never,
    });
    expect(conflict.status(), await conflict.text()).toBe(409);
    const jobsAfterReplay = await fetchJobsForItem(request, apiBase(), seed.itemId);
    expect(jobsAfterReplay.length, "重放与冲突都不应新建 job").toBe(1);
    await captureTo("t16-qa", page, "11-accepted-single-job");
  });

  test("QA-5 受理后不判断远端成功、无自动发布入口、无禁用措辞、准备进度非百分比（AC-048 / §6.3.2）", async ({
    page,
    request,
  }) => {
    const external = await blockExternalRequests(page);
    await apiLogin(request, apiBase(), password());
    await loginViaUi(page, "", password());

    const forbidden = [
      "总进度",
      "预计剩余",
      "已自动校准",
      "自动发布",
      "一键发布",
      "零费用",
      "离线可用",
      "已证明页图来自原 PDF",
      "重试不会重复收费",
    ];
    const bodyText = async (): Promise<string> => page.locator("body").innerText();

    // 全程走真实 UI：建物品 → PDF → 照片 → 准备 → 报价 → 确认 → 生成。
    await page.goto("/items/new");
    await page.getByLabel("名称").fill("QA 全流程相机");
    await page.getByLabel("准确型号").fill("QA-T16-FLOW");
    await page.getByRole("button", { name: "创建并继续" }).click();
    await expect(page).toHaveURL(/\/items\/[0-9a-f-]{36}$/);
    const itemId = new URL(page.url()).pathname.split("/").pop() ?? "";

    await page.goto(`/items/${itemId}/import/document`);
    await page.getByLabel("选择 PDF 文件").setInputFiles(fixturePath("sample-manual-text.pdf"));
    await expect(page.getByText("待绑定文件：sample-manual-text.pdf", { exact: false })).toBeVisible();
    // UI-011：出处链接字段旁固定提示「服务器不会访问该地址」（绑定表单打开时可见）。
    await expect(page.getByText("来源链接仅作出处记录，服务器不会访问该地址。")).toBeVisible();
    await page.getByRole("button", { name: "绑定为说明书" }).click();
    await expect(page.getByRole("heading", { name: "已绑定的说明书" })).toBeVisible();

    await page.getByRole("link", { name: /下一步：视图排列/ }).click();
    await page.getByLabel("上传正面视图照片").setInputFiles(fixturePath("sample-photo-front.jpg"));
    await page.getByLabel("上传左侧视图照片").setInputFiles(fixturePath("sample-photo-left.png"));
    await expect(page.getByTestId("view-slot-detail")).toContainText("不发送给 Tripo");

    // 第 4 步：进度必须是"第 n / N 页"（无线性百分比、无预计剩余）。
    await page.getByRole("link", { name: /下一步：准备/ }).click();
    await expect(page.getByRole("heading", { name: "资料准备" })).toBeVisible();
    await expect(page.getByRole("note")).toContainText("关闭标签页会中断准备");
    await page.getByTestId("prepare-start").click();
    // 逐页进度是「第 n / N 页」/「已完成 n / N 页」（页数取自 PDF.js 实际解析，不写死）。
    await expect(page.getByTestId("prepare-status")).toContainText(/第 \d+ \/ \d+ 页|已完成 \d+ \/ \d+ 页/);
    const preparingText = await bodyText();
    expect(preparingText).not.toMatch(/\d+\s*%/);
    for (const phrase of ["总进度", "预计剩余"]) {
      expect(preparingText, `准备页不应出现「${phrase}」`).not.toContain(phrase);
    }
    await captureTo("t16-qa", page, "12-preparing-progress");
    await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 60_000 });
    await page.getByTestId("prepare-seal").click();
    await expect(page.getByTestId("prepare-sealed")).toContainText("准备完成（ready）");
    await expect(page.getByTestId("prepare-sealed")).toContainText("不证明其确实来自原 PDF");
    await captureTo("t16-qa", page, "13-preparation-sealed");

    // 第 5 步 → 受理。
    await page.getByRole("link", { name: /下一步：预算\/隐私确认/ }).click();
    await expect(page.getByTestId("quote-panel")).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId("quote-panel")).toContainText("不是供应商账户级硬封顶");
    await confirmSendScope(page);
    await page.getByTestId("generate-button").click();
    const accepted = page.getByTestId("job-accepted");
    await expect(accepted).toBeVisible();
    await expect(accepted).toContainText("不代表生成结果");

    // AC-048（UI 侧）：界面不判断远端成功、不存在发布入口与自动发布路径。
    const acceptedText = await bodyText();
    expect(acceptedText).not.toMatch(/生成成功|已发布|发布成功|已经发布/);
    expect(acceptedText).toContain("已受理（202）");
    await expect(page.getByRole("button", { name: /发布/ })).toHaveCount(0);
    await expect(page.getByRole("link", { name: /发布/ })).toHaveCount(0);
    for (const phrase of forbidden) {
      expect(acceptedText, `页面不应出现「${phrase}」`).not.toContain(phrase);
    }
    await captureTo("t16-qa", page, "14-accepted-no-publish");

    // 事实更新（QA 回合 21，T17 已交付）：受理面板的「查看任务详情」现在进入真实任务详情页
    // （阶段明细/费用/恢复入口），不再是占位页；AC-048 的「无自动发布路径」在任务页同样成立。
    await page.getByRole("link", { name: /查看任务/ }).click();
    await expect(page).toHaveURL(/\/jobs\/[0-9a-f-]{36}$/);
    await expect(page.getByRole("heading", { name: "阶段明细" })).toBeVisible();
    await expect(page.getByRole("link", { name: /发布/ })).toHaveCount(0);
    await captureTo("t16-qa", page, "15-jobs-detail-real");
    expect(external, `不应访问外部地址：${external.join(", ")}`).toEqual([]);
  });

  test("QA-6 同视图占用 422 的可行动文案 + 缺项提示（UI-012/UI-013）", async ({ page, request }) => {
    const itemId = await seedItem(request, apiBase(), password(), "QA 视图占用");
    const csrf = await apiLogin(request, apiBase(), password());
    await loginViaUi(page, "", password());
    await page.goto(`/items/${itemId}/import/views`);
    await expect(page.getByRole("heading", { name: "视图排列" })).toBeVisible();

    // 空槽位与说明（UI-012）。
    await expect(page.getByTestId("view-slot-front")).toContainText("必需");
    await expect(page.getByTestId("view-slot-detail")).toContainText("不发送给 Tripo");
    await expect(page.getByTestId("generation-gaps")).toContainText("缺少 front（正面）视图照片");
    await expect(page.getByTestId("generation-gaps")).toContainText("缺少侧面视图");

    // 页面上 front 仍是空槽，但服务端已被另一个会话登记 front（并发窗口）。
    await seedPhoto(request, apiBase(), csrf, itemId, "front", "sample-photo-front.jpg");
    await page.getByLabel("上传正面视图照片").setInputFiles(fixturePath("sample-photo-front.jpg"));
    const errorPanel = page.getByTestId("photo-error");
    await expect(errorPanel).toBeVisible();
    await expect(errorPanel).toContainText("已有照片");
    await expect(errorPanel).toContainText("替换照片");
    await captureTo("t16-qa", page, "16-view-occupied-422");

    // 服务端事实：front 只有一张（没有被第二次登记覆盖成两条）。
    const photos = await fetchPhotos(request, apiBase(), itemId);
    expect(photos.filter((photo) => photo.view === "front").length).toBe(1);

    // 刷新后按服务端事实渲染：front 槽位显示照片与缺项更新。
    await page.reload();
    await expect(page.getByTestId("view-slot-front").getByRole("img")).toBeVisible();
    await expect(page.getByTestId("generation-gaps")).toContainText("缺少侧面视图");
    await expect(page.getByTestId("generation-gaps")).not.toContainText("缺少 front");
  });

  test("QA-7 缺侧面视图：生成禁用、缺项常驻、不提前请求报价（AC-026 / UI-013）", async ({
    page,
    request,
  }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "QA 缺侧面",
    );
    const csrf = await apiLogin(request, apiBase(), password());
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "front", "sample-photo-front.jpg");
    const preparationId = await seedReadyPreparation(request, apiBase(), csrf, seed);

    await loginViaUi(page, "", password());
    await setPreparationPointer(page, seed.itemId, preparationId);
    const estimatePosts = recordRequests(
      page,
      (url, method) => method === "POST" && /\/estimates$/.test(url),
    );
    await page.goto(`/items/${seed.itemId}/import/confirm`);
    await expect(page.getByRole("heading", { name: "预算与隐私确认" })).toBeVisible();

    const gaps = page.getByTestId("generation-gaps");
    await expect(gaps).toContainText("缺少侧面视图");
    await expect(gaps.getByRole("link", { name: "去补充视图" })).toBeVisible();
    await expect(page.getByTestId("generate-button")).toBeDisabled();
    await expect(page.getByTestId("generate-reason")).toBeVisible();
    // 禁用原因与按钮关联（UI-013 可访问性要求）。
    await expect(page.getByTestId("generate-button")).toHaveAttribute(
      "aria-describedby",
      "generate-reason",
    );
    await expect(page.getByTestId("quote-missing")).toContainText("资料未齐");
    expect(estimatePosts.length, "前置不满足时不应请求报价").toBe(0);
    await captureTo("t16-qa", page, "17-missing-side");
  });

  test("QA-11 无准备指针：如实显示缺项、不伪造状态、不请求报价（REQ-016 诚实性）", async ({
    page,
    request,
  }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "QA 无准备指针",
    );
    const csrf = await apiLogin(request, apiBase(), password());
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "front", "sample-photo-front.jpg");
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "left", "sample-photo-left.png");
    // 服务端确实有一份 ready preparation，但本浏览器会话没有指针（换浏览器/清存储场景）。
    await seedReadyPreparation(request, apiBase(), csrf, seed);

    await loginViaUi(page, "", password());
    const estimatePosts = recordRequests(
      page,
      (url, method) => method === "POST" && /\/estimates$/.test(url),
    );
    await page.goto(`/items/${seed.itemId}/import/confirm`);
    await expect(page.getByRole("heading", { name: "预算与隐私确认" })).toBeVisible();

    const gaps = page.getByTestId("generation-gaps");
    await expect(gaps).toContainText("还没有可用的资料准备记录");
    await expect(gaps.getByRole("link", { name: "去准备" })).toBeVisible();
    await expect(page.getByTestId("generate-button")).toBeDisabled();
    await expect(page.getByTestId("generate-reason")).toContainText("准备");
    await expect(page.getByTestId("quote-missing")).toContainText("资料未齐");
    await expect(page.getByTestId("quote-panel")).toHaveCount(0);
    expect(estimatePosts.length, "无准备指针时不应请求报价").toBe(0);
    await captureTo("t16-qa", page, "23-confirm-without-pointer");
  });

  test("QA-8 第 2/5 步失败态可行动（重试后恢复）（AC-026 / UI-019/026）", async ({
    page,
    request,
  }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "QA 失败恢复",
    );
    const csrf = await apiLogin(request, apiBase(), password());
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "front", "sample-photo-front.jpg");
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "left", "sample-photo-left.png");
    const preparationId = await seedReadyPreparation(request, apiBase(), csrf, seed);
    await loginViaUi(page, "", password());
    await setPreparationPointer(page, seed.itemId, preparationId);

    // 第 2 步：资料清单加载失败 → 重试恢复。
    let failDocuments = true;
    await page.route(/\/api\/v1\/items\/[^/]+\/documents(\?.*)?$/, async (route) => {
      if (route.request().method() === "GET" && failDocuments) {
        failDocuments = false;
        await route.fulfill({
          status: 503,
          contentType: "application/json",
          body: JSON.stringify({
            error: { code: "UNAVAILABLE", message: "清单暂不可用（QA 注入）", requestId: "qa-t16-doc" },
          }),
        });
        return;
      }
      await route.continue();
    });
    await page.goto(`/items/${seed.itemId}/import/document`);
    const docError = page.locator(".error-panel", { hasText: "说明书清单加载失败" });
    await expect(docError).toBeVisible();
    await captureTo("t16-qa", page, "18-document-step-error");
    await docError.getByRole("button", { name: "重试" }).click();
    await expect(page.locator(".error-panel", { hasText: "说明书清单加载失败" })).toHaveCount(0);
    await expect(page.getByText("QA 失败恢复 说明书")).toBeVisible();

    // 第 5 步：报价请求 503 → 可行动错误（重试获取报价 + 服务状态入口）→ 重试恢复。
    let failEstimate = true;
    await page.route(/\/api\/v1\/items\/[^/]+\/estimates$/, async (route) => {
      if (route.request().method() === "POST" && failEstimate) {
        failEstimate = false;
        await route.fulfill({
          status: 503,
          contentType: "application/json",
          body: JSON.stringify({
            error: { code: "UNAVAILABLE", message: "报价暂不可用（QA 注入）", requestId: "qa-t16-est" },
          }),
        });
        return;
      }
      await route.continue();
    });
    await page.goto(`/items/${seed.itemId}/import/confirm`);
    const quoteError = page.getByTestId("quote-error");
    await expect(quoteError).toBeVisible({ timeout: 20_000 });
    await expect(quoteError).toContainText("无法获取报价");
    await expect(quoteError.getByRole("link", { name: "查看服务状态" })).toBeVisible();
    await captureTo("t16-qa", page, "19-quote-error");
    await quoteError.getByRole("button", { name: /重试获取报价/ }).click();
    await expect(page.getByTestId("quote-panel")).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId("quote-error")).toHaveCount(0);
  });

  test("QA-9 窄屏抽屉与桌面并排：同一 URL 切换无需刷新（AC-060 前端侧 / UI-062）", async ({
    page,
    request,
  }) => {
    const seed = await seedItemWithDocument(
      request,
      apiBase(),
      password(),
      "sample-manual-text.pdf",
      "QA 断点",
    );
    const csrf = await apiLogin(request, apiBase(), password());
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "front", "sample-photo-front.jpg");
    await seedPhoto(request, apiBase(), csrf, seed.itemId, "left", "sample-photo-left.png");
    const preparationId = await seedReadyPreparation(request, apiBase(), csrf, seed);
    await loginViaUi(page, "", password());
    await setPreparationPointer(page, seed.itemId, preparationId);

    await page.setViewportSize({ width: 1280, height: 900 });
    await openConfirmWithQuote(page, seed.itemId);
    // 桌面：两栏面板同时在 DOM 中可见（并排）。
    const rail = page.locator(".page-layout__rail");
    const aside = page.locator(".page-layout__aside");
    await expect(rail).toBeVisible();
    await expect(aside).toBeVisible();
    const railBox = await rail.boundingBox();
    const asideBox = await aside.boundingBox();
    expect(railBox!.x + railBox!.width).toBeLessThanOrEqual(asideBox!.x + 1);
    await captureTo("t16-qa", page, "20-desktop-two-columns");

    // 窄屏：同一 URL、不刷新 → 面板进入抽屉，触发按钮在顶栏下方。
    await page.setViewportSize({ width: 375, height: 812 });
    await expect(rail).toHaveCount(0);
    await expect(aside).toHaveCount(0);
    const railTrigger = page.getByRole("button", { name: "报价与预算" });
    const asideTrigger = page.getByRole("button", { name: "将发送的资料与确认" });
    await expect(railTrigger).toBeVisible();
    await expect(asideTrigger).toBeVisible();
    await railTrigger.click();
    const drawer = page.getByRole("dialog", { name: "报价与预算" });
    await expect(drawer).toBeVisible();
    await expect(drawer.getByTestId("quote-panel")).toBeVisible();
    await captureTo("t16-qa", page, "21-narrow-drawer");
    await page.keyboard.press("Escape");
    await expect(drawer).toHaveCount(0);

    // 拉宽回桌面：无需刷新即恢复并排。
    await page.setViewportSize({ width: 1280, height: 900 });
    await expect(page.locator(".page-layout__aside")).toBeVisible();
    await expect(page.getByRole("button", { name: "报价与预算" })).toHaveCount(0);
    await expect(page.getByTestId("quote-panel")).toBeVisible();
  });

  test("QA-12 磁盘满 413（insufficientStorage）文案：所需/可用字节 + 清理提示（UI-009）", async ({
    page,
    request,
  }) => {
    const itemId = await seedItem(request, apiBase(), password(), "QA 磁盘满");
    await loginViaUi(page, "", password());
    await page.goto(`/items/${itemId}/import/views`);
    await expect(page.getByRole("heading", { name: "视图排列" })).toBeVisible();

    // 注入服务端形状的 413（字段与文案逐字取自 crates/server/src/assets/error.rs 与
    // blob_store.rs 的 insufficient_space_message；本机无法真的写满磁盘，故注入响应）。
    const REQUIRED = 10_485_760;
    const AVAILABLE = 1_048_576;
    await page.route(/\/api\/v1\/items\/[^/]+\/assets$/, async (route) => {
      if (route.request().method() !== "POST") {
        await route.continue();
        return;
      }
      await route.fulfill({
        status: 413,
        contentType: "application/json",
        body: JSON.stringify({
          error: {
            code: "PAYLOAD_TOO_LARGE",
            message: `磁盘可用空间不足：本次上传至少需要 ${REQUIRED} 字节，当前可用 ${AVAILABLE} 字节；请清理磁盘后重试（未保存任何资产）`,
            requestId: "qa-t16-disk",
            details: {
              reason: "insufficientStorage",
              requiredBytes: REQUIRED,
              availableBytes: AVAILABLE,
            },
          },
        }),
      });
    });
    await page.getByLabel("上传正面视图照片").setInputFiles(fixturePath("sample-photo-front.jpg"));
    const card = page.getByTestId("photo-upload-front").getByRole("alert");
    await expect(card).toBeVisible();
    await expect(card).toContainText("磁盘空间不足");
    await expect(card).toContainText("需要 10 MiB");
    await expect(card).toContainText("当前可用 1 MiB");
    await expect(card).toContainText("清理磁盘");
    await expect(card.getByRole("button", { name: "重试" })).toBeVisible();
    await expect(card.getByRole("button", { name: "移除" })).toBeVisible();
    await captureTo("t16-qa", page, "24-insufficient-storage");
    // 未产生资产/照片（服务端保证不半提交；这里同时核对前端没有伪造成功）。
    const photos = await fetchPhotos(request, apiBase(), itemId);
    expect(photos.length).toBe(0);
    await expect(page.getByTestId("view-slot-front").getByRole("img")).toHaveCount(0);
  });

  test("QA-10 资料库行布局：名称/型号列在 1280px 下可读（UI-005 回归守卫）", async ({
    page,
    request,
  }) => {
    // 历史（回合 18）：BUG-005（P2）时本用例在 1024–1920px 全部不满足名称列可读（1280/1366px 下
    // 身份列计算宽度 0px、名称逐字换行、单行高 358px），按仓库惯例（同 BUG-003/004）标 fixme 保留
    // 复现。回合 19：RD 修复 BUG-005 后**已移除 fixme**，下方断言为复验依据（阈值不变）。
    await seedItem(request, apiBase(), password(), "QA 行布局测量");
    await loginViaUi(page, "", password());
    await page.goto("/");

    const widths = [1024, 1280, 1366, 1440, 1600, 1920];
    const measurements: Record<string, unknown> = {};
    for (const width of widths) {
      await page.setViewportSize({ width, height: 900 });
      const row = page.locator(".item-row").filter({ hasText: "QA 行布局测量" });
      await expect(row).toBeVisible();
      const measure = async (selector: string) => {
        const box = await row.locator(selector).boundingBox();
        return box === null ? null : { x: box.x, width: box.width, height: box.height };
      };
      const styles = await row.evaluate((node) => {
        const pick = (selector: string) => {
          const element = node.querySelector(selector);
          if (element === null) {
            return null;
          }
          const computed = getComputedStyle(element);
          return { display: computed.display, flexBasis: computed.flexBasis };
        };
        return {
          gridTemplateColumns: getComputedStyle(node).gridTemplateColumns,
          identity: pick(".item-row__identity"),
          actions: pick(".item-row__actions"),
          note: pick(".item-row__note"),
        };
      });
      measurements[String(width)] = {
        row: await measure(":scope"),
        identity: await measure(".item-row__identity"),
        name: await measure(".item-row__name"),
        status: await measure(".item-row__status"),
        time: await measure(".item-row__time"),
        actions: await measure(".item-row__actions"),
        note: await measure(".item-row__note"),
        styles,
      };
      if (width === 1280) {
        await captureTo("t16-qa", page, "22-library-row-geometry");
      }
    }
    const outDir = path.join(
      path.resolve(import.meta.dirname, "../../../../artifacts/web-mvp/t16-qa"),
    );
    fs.mkdirSync(outDir, { recursive: true });
    fs.writeFileSync(
      path.join(outDir, "library-row-geometry.json"),
      JSON.stringify(measurements, null, 2),
    );

    // UI-005：行要显示名称/型号/状态/最近使用——名称列被挤成逐字换行即不可读。
    for (const width of widths) {
      const entry = measurements[String(width)] as { name?: { width: number } };
      expect(
        entry.name?.width ?? 0,
        `${width}px：名称列宽 ${entry.name?.width}；全部测量 ${JSON.stringify(measurements)}`,
      ).toBeGreaterThanOrEqual(120);
    }
  });
});
