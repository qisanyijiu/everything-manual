/** IA-01 独立 QA：真实后端造数 + 可控响应延迟/失败；无真实 Provider。
 * reader manifest 明确为布局夹具，PDF 字节来自真实后端，不据此验收发布语义。
 */
import fs from "node:fs";
import path from "node:path";

import { expect, test, type APIRequestContext, type Locator, type Page } from "@playwright/test";

import {
  apiLogin,
  fetchJobsForItem,
  loginViaUi,
  runtime,
  seedItemWithDocument,
  seedPhoto,
  seedReadyPreparation,
  setPreparationPointer,
  type SeededDocument,
} from "./helpers";
import { REPO_ROOT } from "./runtime";
import { draftPayload, fixtureModel, installViewerRoutes } from "./viewer-harness";

import { launchNativeZoom } from "./qa-native-browser-tools";

const QA_DIR = path.join(REPO_ROOT, "artifacts", "interaction-a", "qa");
const SCOPE_LABEL = /我已阅读并确认将上述资料发送给对应供应商/;

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}

async function screenshot(page: Page, name: string) {
  fs.mkdirSync(QA_DIR, { recursive: true });
  await page.screenshot({ path: path.join(QA_DIR, `${name}.png`), fullPage: true });
}

async function readyItem(request: APIRequestContext, name: string) {
  const { apiBase, password } = runtime();
  const seed = await seedItemWithDocument(request, apiBase, password, "sample-manual-text.pdf", name);
  const csrf = await apiLogin(request, apiBase, password);
  const preparationId = await seedReadyPreparation(request, apiBase, csrf, seed);
  await seedPhoto(request, apiBase, csrf, seed.itemId, "front", "sample-photo-front.jpg");
  await seedPhoto(request, apiBase, csrf, seed.itemId, "left", "sample-photo-left.png");
  return { ...seed, preparationId };
}

async function openConfirm(page: Page, seed: { itemId: string; preparationId: string }) {
  await loginViaUi(page, "", runtime().password);
  await setPreparationPointer(page, seed.itemId, seed.preparationId);
  await page.goto(`/items/${seed.itemId}/import/confirm`);
  await expect(page.getByTestId("quote-panel")).toBeVisible();
}

async function noOverflow(page: Page) {
  expect(await page.evaluate(() => ({
    width: document.documentElement.clientWidth,
    scroll: document.documentElement.scrollWidth,
  }))).toEqual(expect.objectContaining({ width: expect.any(Number), scroll: expect.any(Number) }));
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
}

async function touchTarget(locator: Locator) {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  expect(box?.width).toBeGreaterThanOrEqual(44);
  expect(box?.height).toBeGreaterThanOrEqual(44);
}

async function fontAtLeast(locator: Locator, minimum: number) {
  const sizes = await locator.evaluateAll((elements) => elements
    .filter((element) => element.getClientRects().length > 0)
    .map((element) => ({ text: element.textContent?.slice(0, 80), size: Number.parseFloat(getComputedStyle(element).fontSize) })));
  expect(sizes.length).toBeGreaterThan(0);
  for (const entry of sizes) expect(entry.size, JSON.stringify(entry)).toBeGreaterThanOrEqual(minimum);
}

async function installReaderFixture(page: Page, seed: SeededDocument & { preparationId: string }) {
  const model = fixtureModel("viewer-asymmetric", "interaction-a-qa-model");
  const draft = draftPayload({
    itemId: seed.itemId, documentId: seed.documentId, preparationId: seed.preparationId,
    model, hotspots: [], partNames: ["测试后盖", "测试电池"], stepTitles: ["断开电源再拆开后盖", "取出电池并保管连接线"], modelRevision: 1,
  });
  await installViewerRoutes(page, { drafts: {}, models: [model] });
  await page.route(`**/api/v1/items/${seed.itemId}/releases/interaction-a-qa-release`, async (route) => {
    await route.fulfill({ status: 200, contentType: "application/json", json: { data: {
      id: "interaction-a-qa-release", itemId: seed.itemId, draftRevision: 1,
      modelRevisionId: model.revisionId, manifestSha256: "a".repeat(64),
      manifest: { knowledge: draft.knowledge, review: null, model: { assetId: model.assetId }, documents: [{ documentId: seed.documentId, sourceAssetId: seed.sourceAssetId }] },
    } } });
  });
  return `/items/${seed.itemId}/releases/interaction-a-qa-release`;
}

test.beforeEach(async ({ page }) => {
  const external: string[] = [];
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if ((url.protocol === "http:" || url.protocol === "https:") && !["localhost", "127.0.0.1"].includes(url.hostname)) {
      external.push(url.hostname);
      await route.abort();
    } else await route.continue();
  });
  page.on("close", () => { expect(external).toEqual([]); });
});

test("IA-QA-01 创建等待/失败/成功及编辑路由（AC-001/002）", async ({ page }) => {
  await loginViaUi(page, "", runtime().password);
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto("/items/new");
  await page.getByLabel(/^名称/).fill("IA QA 创建路线");
  await page.getByLabel(/^准确型号/).fill("IA-QA-001");
  await fontAtLeast(page.locator(".item-form .field__label, .item-form .field__input"), 14);
  await fontAtLeast(page.locator(".item-form .field__hint, .item-form__note"), 12);
  await touchTarget(page.getByRole("button", { name: "创建并继续", exact: true }));
  await noOverflow(page);
  await screenshot(page, "01-create-375");
  const gate = deferred();
  let creates = 0;
  let fail = true;
  await page.route("**/api/v1/items", async (route) => {
    if (route.request().method() !== "POST") { await route.continue(); return; }
    creates += 1;
    if (fail) {
      await gate.promise;
      await route.fulfill({ status: 422, json: { error: { code: "VALIDATION_FAILED", message: "QA 创建字段错误", details: { fields: [{ field: "name", message: "QA 名称错误" }] }, requestId: "ia-qa-create" } } });
    } else await route.continue();
  });
  const submit = page.getByRole("button", { name: "创建并继续", exact: true });
  await submit.click();
  await expect(page.locator(".item-form button[type=submit]")).toBeDisabled();
  await page.locator(".item-form").evaluate((form) => form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(creates).toBe(1);
  gate.resolve();
  await expect(page.getByText("QA 名称错误").first()).toBeVisible();
  await expect(page.getByLabel(/^名称/)).toHaveValue("IA QA 创建路线");
  await expect(page).toHaveURL(/\/items\/new$/);
  fail = false;
  await submit.click();
  await expect(page).toHaveURL(/\/items\/[^/]+\/import\/document$/);
  await expect(page.getByRole("heading", { name: /说明书原件/ })).toBeVisible();
  expect(creates).toBe(2);
  const itemId = new URL(page.url()).pathname.split("/")[2];
  await page.goto(`/items/${itemId}/edit`);
  await page.getByLabel(/^名称/).fill("IA QA 已修改");
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/items/${itemId}$`));
  await screenshot(page, "01-create-edit-routes");
});

test("IA-QA-02 四尺寸确认主流程与字体/触控（AC-003/004/011/012）", async ({ page, request }) => {
  const seed = await readyItem(request, "IA QA 四尺寸与长型号文字校验物品");
  await openConfirm(page, seed);
  const measurements: unknown[] = [];
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(page.getByTestId("quote-panel")).toBeVisible();
    await expect(page.getByTestId("send-scope")).toBeVisible();
    await expect(page.getByRole("checkbox", { name: SCOPE_LABEL })).toHaveCount(1);
    await expect(page.locator(".page-layout__panel-bar")).toHaveCount(0);
    const order = await page.evaluate(() => {
      const quote = document.querySelector('[data-testid="quote-panel"]')!;
      const scope = document.querySelector('[data-testid="send-scope"]')!;
      const confirmation = document.querySelector('[data-testid="confirmation-box"]')!;
      const generate = document.querySelector('[data-testid="generate-button"]')!;
      return [quote, scope, confirmation].map((el, index) => Boolean(el.compareDocumentPosition([scope, confirmation, generate][index]!) & Node.DOCUMENT_POSITION_FOLLOWING));
    });
    expect(order).toEqual([true, true, true]);
    await expect(page.getByTestId("quote-tripo-upper")).toContainText("credits");
    await expect(page.getByTestId("quote-manual-upper")).toContainText("USD");
    await expect(page.getByTestId("send-scope")).toContainText("Tripo");
    await expect(page.getByTestId("send-scope")).toContainText("页文字页");
    await expect(page.getByTestId("send-scope")).toContainText("页图页");
    await noOverflow(page);
    await fontAtLeast(page.locator(".confirm-step input[type=text], .confirm-step .scope-list li"), 14);
    await fontAtLeast(page.locator(".confirm-step .field__hint, .confirm-actions__reason, .amount-list__label, .confirm-step .meta-list"), 12);
    if (width === 375) {
      await touchTarget(page.getByTestId("generate-button"));
      await page.locator("#budget-tripo").focus();
      await page.keyboard.press("Tab");
      await expect(page.locator("#budget-manual")).toBeFocused();
      await page.keyboard.press("Tab");
      // Any optional technical details may precede the checkbox; keyboard must reach it.
      for (let n = 0; n < 8 && !(await page.getByRole("checkbox", { name: SCOPE_LABEL }).evaluate((el) => el === document.activeElement)); n += 1) await page.keyboard.press("Tab");
      await expect(page.getByRole("checkbox", { name: SCOPE_LABEL })).toBeFocused();
      expect(await page.getByRole("checkbox", { name: SCOPE_LABEL }).evaluate((el) => {
        const box = el.getBoundingClientRect();
        const target = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
        return target === el;
      })).toBe(true);
    }
    measurements.push(await page.evaluate(() => ({ width: innerWidth, scrollWidth: document.documentElement.scrollWidth, checkbox: document.querySelector<HTMLInputElement>("#send-scope-confirm")?.checked })));
    await screenshot(page, `02-confirm-${width}`);
  }
  fs.writeFileSync(path.join(QA_DIR, "02-layout-measurements.json"), JSON.stringify(measurements, null, 2));
});

test("IA-QA-03 确认延迟/取消/失败、低预算及同键显式重试（AC-005/006/007/008）", async ({ page, request }) => {
  const seed = await readyItem(request, "IA QA 确认状态");
  await openConfirm(page, seed);
  const checkbox = page.getByRole("checkbox", { name: SCOPE_LABEL });
  const generate = page.getByTestId("generate-button");
  await expect(checkbox).not.toBeChecked();
  await expect(generate).toBeDisabled();
  let gate = deferred();
  let failConfirmation = false;
  let confirms = 0;
  await page.route(/\/estimates\/[^/]+\/confirm$/, async (route) => {
    confirms += 1;
    await gate.promise;
    if (failConfirmation) await route.fulfill({ status: 500, json: { error: { code: "INTERNAL", message: "QA 确认保存失败", details: null, requestId: "ia-qa-confirm" } } });
    else await route.continue();
  });
  await checkbox.check();
  await expect(page.getByText("正在保存确认…", { exact: true })).toBeVisible();
  await expect(generate).toBeDisabled();
  gate.resolve();
  await expect(generate).toBeEnabled();
  await checkbox.uncheck();
  await expect(generate).toBeDisabled();
  gate = deferred();
  await checkbox.check();
  await expect(generate).toBeDisabled();
  await expect(page.getByText("正在保存确认…", { exact: true })).toBeVisible();
  failConfirmation = true;
  gate.resolve();
  await expect(checkbox).not.toBeChecked();
  await expect(page.getByTestId("confirm-error")).toContainText("QA 确认保存失败");
  await expect(generate).toBeDisabled();
  failConfirmation = false;
  await checkbox.check();
  await expect(generate).toBeEnabled();
  expect(confirms).toBe(3);
  const tripo = page.locator("#budget-tripo");
  const original = await tripo.inputValue();
  await tripo.fill("abc");
  await expect(generate).toBeDisabled();
  await expect(page.getByTestId("generate-reason")).toContainText("格式");
  await tripo.fill("0");
  await expect(generate).toBeDisabled();
  await expect(page.getByTestId("generate-reason")).toContainText("低于");
  await tripo.fill(original);
  const keys: (string | null)[] = [];
  const bodies: (string | null)[] = [];
  const submitGate = deferred();
  const retryGate = deferred();
  const quoteId = new URL(page.url()).searchParams.get("quoteId");
  expect(quoteId).toBeTruthy();
  await page.route(`**/api/v1/items/${seed.itemId}/jobs`, async (route) => {
    if (route.request().method() !== "POST") { await route.continue(); return; }
    keys.push(await route.request().headerValue("idempotency-key"));
    bodies.push(route.request().postData());
    if (keys.length === 1) { await submitGate.promise; await route.abort("connectionfailed"); }
    else { await retryGate.promise; await route.continue(); }
  });
  await generate.click();
  await expect(generate).toBeDisabled();
  expect(keys).toHaveLength(1);
  const consumptionRead = page.waitForResponse(response => response.request().method() === "GET"
    && new URL(response.url()).pathname === `/api/v1/items/${seed.itemId}/estimates/${quoteId}`);
  submitGate.resolve();
  const consumption = await consumptionRead;
  expect(consumption.status()).toBe(200);
  expect((await consumption.json()).data.consumedJobId ?? null).toBeNull();
  const retry = page.getByRole("button", { name: "重试同一提交（使用原授权）", exact: true });
  await expect(retry).toBeEnabled();
  await expect(generate).toBeDisabled();
  await expect(tripo).toBeDisabled();
  await expect(page.getByTestId("job-accepted")).toHaveCount(0);
  expect(await fetchJobsForItem(request, runtime().apiBase, seed.itemId)).toHaveLength(0);
  await page.waitForTimeout(300);
  expect(keys).toHaveLength(1);
  await retry.focus(); await retry.press("Enter");
  await expect(retry).toBeDisabled();
  await expect(generate).toHaveAttribute("aria-busy", "true");
  await expect.poll(() => keys.length).toBe(2);
  expect(keys[0]).toBeTruthy(); expect(keys[1]).toBe(keys[0]);
  expect(bodies[1]).toBe(bodies[0]);
  expect(JSON.parse(bodies[0] ?? "{}").limits).toBeTruthy();
  retryGate.resolve();
  await expect(page.getByTestId("job-accepted")).toBeVisible();
  expect(await fetchJobsForItem(request, runtime().apiBase, seed.itemId)).toHaveLength(1);
  await expect(page.getByTestId("job-accepted")).toContainText("服务");
  await expect(page.getByRole("link", { name: /查看任务详情/ })).toBeVisible();
  await screenshot(page, "03-confirm-accepted");
});

test("IA-QA-04 过期手动重报与旧确认晚到隔离（AC-005/007）", async ({ page, request }) => {
  const seed = await readyItem(request, "IA QA 过期旧响应");
  let quotes = 0;
  await page.route(`**/api/v1/items/${seed.itemId}/estimates`, async (route) => {
    quotes += 1;
    const response = await route.fetch();
    const json = await response.json();
    if (quotes === 1) json.data.expiresAt = new Date(Date.now() + 2500).toISOString();
    await route.fulfill({ response, json });
  });
  await openConfirm(page, seed);
  const pending = deferred();
  let oldReplyReady = false;
  await page.route(/\/estimates\/[^/]+\/confirm$/, async (route) => {
    const response = await route.fetch();
    oldReplyReady = true;
    await pending.promise;
    await route.fulfill({ response });
  });
  await page.getByRole("checkbox", { name: SCOPE_LABEL }).check();
  await expect.poll(() => oldReplyReady).toBe(true);
  await expect(page.getByTestId("quote-expiry")).toContainText("已过期", { timeout: 10000 });
  expect(quotes).toBe(1);
  await expect(page.getByTestId("generate-button")).toBeDisabled();
  await page.getByTestId("requote-button").click();
  await expect.poll(() => quotes).toBe(2);
  await expect(page.getByRole("checkbox", { name: SCOPE_LABEL })).not.toBeChecked();
  pending.resolve();
  await page.waitForTimeout(300);
  await expect(page.getByRole("checkbox", { name: SCOPE_LABEL })).not.toBeChecked();
  await expect(page.getByTestId("generate-button")).toBeDisabled();
  await expect(page.getByTestId("confirmed-at")).toHaveCount(0);
  await screenshot(page, "04-stale-confirm-response");
});

test("IA-QA-05 阅读器键盘标签/抽屉与手机触控（AC-009/010/011/012）", async ({ page, request }) => {
  const seed = await readyItem(request, "IA QA 阅读器可达性");
  await loginViaUi(page, "", runtime().password);
  const url = await installReaderFixture(page, seed);
  await page.setViewportSize({ width: 1024, height: 900 });
  await page.goto(url);
  await page.getByRole("button", { name: "显示步骤与原文" }).click();
  const parts = page.getByRole("tab", { name: "部件", exact: true });
  const steps = page.getByRole("tab", { name: "步骤与原文", exact: true });
  const originalTab = page.getByRole("tab", { name: "原文", exact: true });
  const tabs = [parts, steps, originalTab];
  await expect(page.getByRole("tab")).toHaveText(["部件", "步骤与原文", "原文"]);
  await parts.focus();
  for (const [key, target] of [
    ["ArrowLeft", originalTab], ["ArrowRight", parts], ["ArrowRight", steps],
    ["End", originalTab], ["Home", parts], ["ArrowRight", steps],
  ] as const) {
    await page.keyboard.press(key);
    await expect(target).toBeFocused();
    await expect(target).toHaveAttribute("aria-selected", "true");
    await expect(target).toHaveAttribute("tabindex", "0");
    await expect(page.locator('[role="tab"][tabindex="0"]')).toHaveCount(1);
    for (const tab of tabs) if (tab !== target) {
      await expect(tab).toHaveAttribute("aria-selected", "false");
      await expect(tab).toHaveAttribute("tabindex", "-1");
    }
    const panel = page.getByRole("tabpanel");
    await expect(panel).toHaveCount(1);
    await expect(target).toHaveAttribute("aria-controls", (await panel.getAttribute("id")) ?? "");
    await expect(panel).toHaveAttribute("aria-labelledby", (await target.getAttribute("id")) ?? "");
  }
  await page.keyboard.press("Tab");
  expect(await page.getByRole("tabpanel").evaluate(el => el.contains(document.activeElement))).toBe(true);
  await parts.click(); await expect(page.getByTestId("parts-panel")).toBeVisible();
  await steps.click(); await expect(page.getByTestId("steps-panel")).toBeVisible();
  await fontAtLeast(page.locator(".step-detail li"), 14);
  await fontAtLeast(page.locator(".step-evidence button, .step-nav"), 12);
  await originalTab.click();
  await expect(page.getByTestId("original-page-label")).toHaveText("第 1 / 2 页", { timeout: 20000 });
  await fontAtLeast(page.locator(".original-panel__nav, .original-panel__nav button"), 12);
  await steps.click();
  await screenshot(page, "05-reader-keyboard-mid");

  const assertDrawerRing = async (activeDialog: Locator) => {
    const close = activeDialog.getByRole("button", { name: "关闭", exact: true });
    await close.focus(); await page.keyboard.press("Shift+Tab");
    expect(await activeDialog.evaluate(el => el.contains(document.activeElement))).toBe(true);
    await expect(close).not.toBeFocused();
    await page.keyboard.press("Tab"); await expect(close).toBeFocused();
    await page.keyboard.press("Tab");
    expect(await activeDialog.evaluate(el => el.contains(document.activeElement))).toBe(true);
    await expect(close).not.toBeFocused();
  };
  await page.setViewportSize({ width: 375, height: 812 });
  const trigger = page.getByRole("button", { name: "步骤与原文", exact: true });
  const stepsDialog = page.getByRole("dialog", { name: "步骤与原文", exact: true });
  // Active selection survives the breakpoint; close that inherited drawer before re-opening.
  await expect(stepsDialog).toBeVisible(); await expect(page.getByRole("dialog")).toHaveCount(1);
  await page.keyboard.press("Escape"); await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(trigger).toBeFocused();
  for (const label of ["部件", "步骤与原文", "原文"]) await touchTarget(page.getByRole("button", { name: label, exact: true }));
  await page.keyboard.press("Enter");
  await expect(stepsDialog).toBeVisible(); await expect(page.getByRole("dialog")).toHaveCount(1);
  // Exact current step source, not a specification/part source or an arbitrary first button.
  const source = stepsDialog.locator(".step-detail .step-evidence button");
  await expect(source).toHaveCount(1);
  for (const target of [stepsDialog.getByRole("button", { name: "关闭", exact: true }),
    stepsDialog.getByRole("button", { name: "上一步", exact: true }),
    stepsDialog.getByRole("button", { name: "下一步", exact: true }), source]) await touchTarget(target);
  await fontAtLeast(stepsDialog.locator(".step-detail li"), 14);
  await fontAtLeast(stepsDialog.locator(".step-evidence button, .step-nav"), 12);
  await assertDrawerRing(stepsDialog); await noOverflow(page);
  await screenshot(page, "05-reader-touch-375");
  await source.focus(); await page.keyboard.press("Enter");
  const originalDialog = page.getByRole("dialog", { name: "原文", exact: true });
  await expect(originalDialog).toBeVisible(); await expect(page.getByRole("dialog")).toHaveCount(1);
  await expect(stepsDialog).toHaveCount(0);
  await expect(originalDialog.locator("#original-heading")).toBeFocused();
  const textLayer = originalDialog.getByTestId("original-text");
  const assertOriginalPage = async (number: number) => {
    await expect(originalDialog.getByTestId("original-page-label")).toHaveText(`第 ${number} / 2 页`, { timeout: 20000 });
    const canvas = originalDialog.getByTestId("original-canvas");
    await expect(canvas).toBeVisible({ timeout: 20000 });
    await expect(canvas).toHaveAttribute("aria-label", `原 PDF 第 ${number} 页`);
    await expect(textLayer).toBeVisible({ timeout: 20000 });
    if ((await textLayer.getAttribute("open")) === null) await textLayer.locator("summary").click();
    await expect(textLayer.locator("pre")).toContainText(`Page ${number} of 2`);
  };
  await assertOriginalPage(1); await fontAtLeast(textLayer.locator("summary, pre"), 12);
  const previousPage = originalDialog.getByRole("button", { name: "上一页", exact: true });
  const nextPage = originalDialog.getByRole("button", { name: "下一页", exact: true });
  const pageInput = originalDialog.getByLabel("页码", { exact: true });
  const jump = originalDialog.getByRole("button", { name: "跳转", exact: true });
  const back = originalDialog.getByRole("button", { name: "返回出处", exact: true });
  for (const target of [originalDialog.getByRole("button", { name: "关闭", exact: true }), previousPage, nextPage, pageInput, jump, back]) await touchTarget(target);
  await assertDrawerRing(originalDialog);
  await nextPage.focus(); await page.keyboard.press("Tab"); await expect(pageInput).toBeFocused();
  await page.keyboard.press("Tab"); await expect(jump).toBeFocused();
  await page.keyboard.press("Tab"); await expect(textLayer.locator("summary")).toBeFocused();
  await page.keyboard.press("Space"); await expect(textLayer).not.toHaveAttribute("open", "");
  await page.keyboard.press("Enter"); await expect(textLayer).toHaveAttribute("open", "");
  await nextPage.focus(); await page.keyboard.press("Enter"); await assertOriginalPage(2);
  await previousPage.focus(); await page.keyboard.press("Enter"); await assertOriginalPage(1);
  await noOverflow(page); await textLayer.scrollIntoViewIfNeeded();
  await screenshot(page, "05-reader-pdf-text-375");
  await back.focus(); await page.keyboard.press("Enter");
  await expect(stepsDialog).toBeVisible(); await expect(page.getByRole("dialog")).toHaveCount(1);
  await expect(originalDialog).toHaveCount(0); await expect(source).toBeFocused();
  await expect(stepsDialog.getByTestId("reader-step-position")).toHaveText("第 1 / 2 步");
  await expect(stepsDialog.getByRole("button", { name: "断开电源再拆开后盖", exact: true })).toHaveAttribute("aria-current", "true");
  // Esc from source-opened original returns to source; a second Esc returns to the toolbar.
  await page.keyboard.press("Enter");
  await expect(originalDialog).toBeVisible(); await expect(page.getByRole("dialog")).toHaveCount(1);
  await expect(originalDialog.locator("#original-heading")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(stepsDialog).toBeVisible(); await expect(page.getByRole("dialog")).toHaveCount(1);
  await expect(source).toBeFocused();
  await page.keyboard.press("Escape"); await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test("IA-QA-06 实际浏览器 200% 缩放保持表单与确认可达（AC-012）", async ({ request }, testInfo) => {
  const seed = await readyItem(request, "IA QA 真实浏览器缩放");
  const native = await launchNativeZoom(testInfo);
  const { context, page } = native;
  try {
    const external: string[] = [];
    await page.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      if ((url.protocol === "http:" || url.protocol === "https:") && !["127.0.0.1", "localhost"].includes(url.hostname)) {
        external.push(url.hostname); await route.abort();
      } else await route.continue();
    });
    await openConfirm(page, seed);
    const before = await page.evaluate(() => ({ innerWidth, outerWidth, dpr: devicePixelRatio }));
    const nativeOuterBefore = await native.nativeOuterWidth();
    const zoom = await native.setZoom(2);
    expect(zoom).toBe(2);
    await expect.poll(() => page.evaluate(() => devicePixelRatio)).toBe(before.dpr * 2);
    const after = await page.evaluate(() => ({ innerWidth, outerWidth, dpr: devicePixelRatio }));
    const nativeOuterAfter = await native.nativeOuterWidth();
    expect(nativeOuterAfter).toBe(nativeOuterBefore);
    // Firefox content outerWidth is zoom-relative; actual browser window pixels remain unchanged.
    if (native.engine !== "firefox") expect(after.outerWidth).toBe(before.outerWidth);
    expect(Math.abs(after.innerWidth * 2 - before.innerWidth)).toBeLessThanOrEqual(2);
    await noOverflow(page);
    await page.locator("#budget-tripo").focus();
    expect(await page.locator("#budget-tripo").evaluate((el) => {
      const box = el.getBoundingClientRect();
      return document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2) === el;
    })).toBe(true);
    await page.keyboard.press("Tab");
    await expect(page.locator("#budget-manual")).toBeFocused();
    expect(await page.locator("#budget-manual").evaluate((el) => {
      const box = el.getBoundingClientRect();
      return document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2) === el;
    })).toBe(true);
    const check = page.getByRole("checkbox", { name: SCOPE_LABEL });
    for (let n = 0; n < 10 && !(await check.evaluate((el) => el === document.activeElement)); n += 1) await page.keyboard.press("Tab");
    await expect(check).toBeFocused();
    await page.keyboard.press("Space");
    await expect(page.getByTestId("generate-button")).toBeEnabled();
    await page.keyboard.press("Tab");
    await expect(page.getByTestId("generate-button")).toBeFocused();
    expect(await page.getByTestId("generate-button").evaluate((el) => {
      const rect = el.getBoundingClientRect();
      return el.contains(document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2));
    })).toBe(true);
    const confirmGeometry = await page.evaluate(() => {
      const selectors = [".confirm-step", ".confirm-step .page__lead", "#budget-tripo", "#budget-manual", ".confirm-box", '[data-testid="generate-button"]'];
      return { innerWidth, innerHeight, scrollY, visualViewport: { width: visualViewport?.width, height: visualViewport?.height, scale: visualViewport?.scale }, elements: selectors.map((selector) => {
        const element = document.querySelector<HTMLElement>(selector)!;
        const rect = element.getBoundingClientRect();
        return { selector, left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom, fontSize: getComputedStyle(element).fontSize };
      }) };
    });
    for (const element of confirmGeometry.elements) {
      expect(element.left).toBeGreaterThanOrEqual(0);
      expect(element.right).toBeLessThanOrEqual(confirmGeometry.innerWidth + 1);
    }
    await page.screenshot({ path: path.join(QA_DIR, "06-confirm-browser-zoom-200-viewport.png"), fullPage: false });
    await page.goto("/items/new");
    await expect.poll(() => page.evaluate(() => devicePixelRatio)).toBe(before.dpr * 2);
    expect(await native.nativeOuterWidth()).toBe(nativeOuterBefore);
    expect(Math.abs((await page.evaluate(() => innerWidth)) * 2 - before.innerWidth)).toBeLessThanOrEqual(2);
    await expect(page.getByRole("heading", { name: "新建物品" })).toBeVisible();
    await page.getByLabel(/^名称/).fill("缩放键盘输入");
    expect(await page.getByLabel(/^名称/).evaluate((el) => {
      const box = el.getBoundingClientRect();
      return document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2) === el;
    })).toBe(true);
    await page.keyboard.press("Tab");
    await expect(page.getByLabel(/^准确型号/)).toBeFocused();
    expect(await page.getByLabel(/^准确型号/).evaluate((el) => {
      const box = el.getBoundingClientRect();
      return document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2) === el;
    })).toBe(true);
    await page.keyboard.type("IA-ZOOM");
    await page.keyboard.press("Tab"); await page.keyboard.press("Tab"); await page.keyboard.press("Tab");
    await expect(page.getByRole("button", { name: "创建并继续" })).toBeFocused();
    await noOverflow(page);
    const formFocusBeforeSettle = await page.getByRole("button", { name: "创建并继续" }).evaluate((button) => {const rect=button.getBoundingClientRect();return {innerWidth,innerHeight,dpr:devicePixelRatio,scrollY,focused:button===document.activeElement,centerHit:button.contains(document.elementFromPoint(rect.left+rect.width/2,rect.top+rect.height/2)),rect:{x:rect.x,y:rect.y,width:rect.width,height:rect.height}};});
    // Observe Firefox's native focus scroll without initiating scroll or changing focus.
    await expect.poll(() => page.getByRole("button", { name: "创建并继续" }).evaluate((button) => {const rect=button.getBoundingClientRect();return button.contains(document.elementFromPoint(rect.left+rect.width/2,rect.top+rect.height/2));})).toBe(true);
    const formGeometry = await page.evaluate(() => {
      const button = document.querySelector<HTMLButtonElement>('.item-form button[type="submit"]')!;
      const rect = button.getBoundingClientRect();
      const selectors = [".item-form-page", ".item-form-page .page__lead", ".item-form", "#field-name", "#field-model", '.item-form button[type="submit"]'];
      return { innerWidth, innerHeight, dpr: devicePixelRatio, scrollY, buttonHit: button.contains(document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2)), elements: selectors.map((selector) => {
        const element = document.querySelector<HTMLElement>(selector)!;
        const box = element.getBoundingClientRect();
        return { selector, left: box.left, right: box.right, top: box.top, bottom: box.bottom, fontSize: getComputedStyle(element).fontSize };
      }) };
    });
    expect(formGeometry.buttonHit).toBe(true);
    for (const element of formGeometry.elements) {
      expect(element.left).toBeGreaterThanOrEqual(0);
      expect(element.right).toBeLessThanOrEqual(formGeometry.innerWidth + 1);
    }
    await page.screenshot({ path: path.join(QA_DIR, "06-create-browser-zoom-200-viewport.png"), fullPage: false });
    fs.writeFileSync(path.join(QA_DIR, "06-browser-zoom-evidence.json"), JSON.stringify({ method: native.method, engine: native.engine, nativeOuterBefore, nativeOuterAfter, zoom, before, after, confirmGeometry, formGeometry, formFocusBeforeSettle, browser: context.browser()?.version(), external }, null, 2));
    expect(external).toEqual([]);
  } finally {
    await native.close();
  }
});

test("IA-QA-07 缺项、配置未就绪、报价加载/失败均留在主流程（AC-004）", async ({ page, request }) => {
  // Genuine missing preparation, not an existing ready record without a local hint.
  const missing = await seedItemWithDocument(request, runtime().apiBase, runtime().password, "sample-manual-text.pdf", "IA QA 真正缺准备");
  const csrf = await apiLogin(request, runtime().apiBase, runtime().password);
  await seedPhoto(request, runtime().apiBase, csrf, missing.itemId, "front", "sample-photo-front.jpg");
  await seedPhoto(request, runtime().apiBase, csrf, missing.itemId, "left", "sample-photo-left.png");
  const seed = await readyItem(request, "IA QA 报价状态");
  await loginViaUi(page, "", runtime().password);
  await page.setViewportSize({ width: 375, height: 812 });
  const writes: string[] = [];
  page.on("request", request => {
    if (request.method() === "POST" && /\/(?:estimates|jobs)$/.test(new URL(request.url()).pathname)) writes.push(request.url());
  });
  await page.goto(`/items/${missing.itemId}/import/confirm`);
  await expect(page.getByRole("link", { name: "去准备", exact: true })).toBeVisible();
  await expect(page.getByTestId("quote-panel")).toHaveCount(0);
  await expect(page.getByTestId("generate-button")).toBeDisabled();
  expect(writes).toEqual([]);
  await setPreparationPointer(page, seed.itemId, seed.preparationId);
  // Retain the original explicit settings-status fault injection.
  await page.route("**/api/v1/settings/status", async (route) => {
    const response = await route.fetch();
    const json = await response.json();
    json.data.capabilities.generation = false;
    await route.fulfill({ response, json });
  });
  // The ready item has never been opened and has no quote to recover.
  await page.goto(`/items/${seed.itemId}/import/confirm`);
  await expect(page.getByTestId("generation-gaps")).toContainText("生成能力未就绪");
  await expect(page.getByRole("link", { name: "查看服务状态", exact: true })).toBeVisible();
  await expect(page.getByTestId("quote-panel")).toHaveCount(0);
  expect(writes).toEqual([]);
  await screenshot(page, "07-config-not-ready");
  await page.unroute("**/api/v1/settings/status");
  const gate = deferred();
  let failed = true;
  await page.route(`**/api/v1/items/${seed.itemId}/estimates`, async (route) => {
    if (failed) {
      await gate.promise;
      await route.fulfill({ status: 500, json: { error: { code: "INTERNAL", message: "QA 报价暂不可用，请重试", details: null, requestId: "ia-qa-quote" } } });
    } else await route.continue();
  });
  await page.reload();
  await expect(page.getByText("正在获取报价…", { exact: true })).toBeVisible();
  await expect(page.getByTestId("quote-tripo-upper")).toHaveCount(0);
  gate.resolve();
  await expect(page.getByTestId("quote-error")).toContainText("QA 报价暂不可用，请重试");
  await expect(page.getByRole("button", { name: "重试获取报价", exact: true })).toBeVisible();
  await noOverflow(page);
  await screenshot(page, "07-quote-error");
  failed = false;
  await page.getByRole("button", { name: "重试获取报价", exact: true }).click();
  await expect(page.getByTestId("quote-panel")).toBeVisible();
  await expect(page.getByTestId("send-scope")).toBeVisible();
  await expect(page.getByRole("checkbox", { name: SCOPE_LABEL })).not.toBeChecked();
});
