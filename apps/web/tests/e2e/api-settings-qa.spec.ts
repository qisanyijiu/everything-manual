/** AS-01 independent QA, PRD/UI revision 1.
 * Initial preparation: UI + real restart path; API assertions will use generated schema
 * after RD freezes it. Execute only after RD_READY with the dedicated QA config.
 */
import path from "node:path";
import fs from "node:fs";
import { execFileSync } from "node:child_process";

import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

import type { components } from "../../src/api/generated";
import { apiLogin, loginViaUi } from "./helpers";
import { waitForJob } from "./job-recovery-harness";
import {
  ApiSettingsQaBackend, CheckedProviderFixture, QA_DIR, QA_PASSWORD,
  attachQaBackend, readyQuote, writeQaEvidence,
} from "./api-settings-qa-harness";

let a: CheckedProviderFixture;
let b: CheckedProviderFixture;
let backend: ApiSettingsQaBackend;
let externalHosts: string[];

test.beforeEach(async ({ page }) => {
  a = new CheckedProviderFixture({ tripo: "qa-tripo-a", manualAi: "qa-manual-a" });
  b = new CheckedProviderFixture({ tripo: "qa-tripo-b", manualAi: "qa-manual-b" });
  await Promise.all([a.start(), b.start()]);
  backend = new ApiSettingsQaBackend(a, b);
  await backend.start();
  externalHosts = await attachQaBackend(page, backend);
});

test.afterEach(async () => {
  try {
    await backend?.stop();
    expect(backend?.logsContainSecret(), "backend log secret leakage").toBe(false);
    expect(externalHosts, "unexpected browser external hosts").toEqual([]);
  } finally {
    await backend?.cleanup();
    await Promise.all([a?.stop(), b?.stop()]);
  }
});

async function openSettings(page: Page) {
  await loginViaUi(page, "", QA_PASSWORD);
  await page.goto("/settings");
  await expect(page.getByLabel("Tripo Base URL", { exact: true })).toBeVisible();
}

async function screenshot(page: Page, name: string) {
  fs.mkdirSync(QA_DIR, { recursive: true });
  await page.screenshot({ path: path.join(QA_DIR, `${name}.png`), fullPage: true, mask: [page.locator('input[type="password"]')] });
}

async function noSecretInBrowser(page: Page) {
  const keys = [...Object.values(a.keys), ...Object.values(b.keys)];
  const leaks = await page.evaluate((needles) => {
    const strings = [location.href, JSON.stringify(localStorage), JSON.stringify(sessionStorage), document.documentElement.outerHTML];
    return needles.some((needle) => strings.some((value) => value.includes(needle)));
  }, keys);
  expect(leaks, "secret must not enter URL/storage/DOM attributes").toBe(false);
}

test("AS-QA-01 运行配置、375/1440布局与键盘标签（AC001/014/015）", async ({ page }) => {
  await openSettings(page);
  await expect(page.getByLabel("Tripo Base URL", { exact: true })).toHaveValue(`${a.base}/v3`);
  await expect(page.getByLabel("Tripo 模型", { exact: true })).toHaveValue(a.models.tripo);
  await expect(page.getByLabel("说明书 AI 模型", { exact: true })).toHaveValue(a.models.manualAi);
  await expect(page.getByText("连接未验证", { exact: false }).first()).toBeVisible();
  await expect(page.getByRole("button", { name: "测试连接", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "重启服务", exact: true })).toHaveCount(0);
  const measurements = [];
  for (const width of [375, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    const values = await page.evaluate(() => ({ viewport: innerWidth, scrollWidth: document.documentElement.scrollWidth }));
    expect(values.scrollWidth - values.viewport).toBeLessThanOrEqual(1);
    for (const label of ["Tripo Base URL", "Tripo 模型", "说明书 AI Base URL", "说明书 AI 模型"]) {
      const field = page.getByLabel(label, { exact: true });
      await field.focus();
      await expect(field).toBeFocused();
      expect(await field.evaluate((el) => Number.parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(14);
      expect(await field.evaluate((el) => (el as HTMLInputElement).labels?.length ?? 0)).toBeGreaterThan(0);
    }
    for (const button of await page.getByRole("button", { name: /保存配置|恢复部署配置/ }).all()) {
      const box = await button.boundingBox();
      expect(box?.width).toBeGreaterThanOrEqual(44);
      expect(box?.height).toBeGreaterThanOrEqual(44);
    }
    await page.getByLabel("Tripo Base URL", { exact: true }).focus();
    await page.keyboard.press("Tab");
    await expect(page.getByLabel("Tripo 模型", { exact: true })).toBeFocused();
    measurements.push(values);
    await screenshot(page, `01-settings-${width}`);
  }
  expect(a.checks.tripoRequests + a.checks.manualRequests + b.checks.tripoRequests + b.checks.manualRequests).toBe(0);
  await noSecretInBrowser(page);
  writeQaEvidence("01-layout", measurements);
});

test("AS-QA-02 未保存导航与刷新取消/丢弃，密钥不缓存（AC013/UI006）", async ({ page }) => {
  await openSettings(page);
  await page.getByLabel("Tripo 模型", { exact: true }).fill("qa-unsaved-model");
  await page.getByRole("radio", { name: "替换", exact: true }).first().check();
  await page.getByLabel("新的 Tripo 密钥", { exact: true }).fill(b.keys.tripo);
  await noSecretInBrowser(page);
  await page.getByRole("link", { name: "资料库", exact: true }).first().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("有未保存的 API 配置");
  await expect(dialog.getByRole("button", { name: "继续编辑", exact: true })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  expect(await page.getByLabel("新的 Tripo 密钥", { exact: true }).inputValue() === b.keys.tripo, "cancel retains secret in form memory").toBe(true);
  const dismissed = page.waitForEvent("dialog");
  // A cancelled native beforeunload has no successful navigation completion.
  // Bound only Playwright's navigation wait; assert the still-live form afterwards.
  const reload = page.reload({ timeout: 3_000 }).catch(() => null);
  const native = await dismissed;
  expect(native.type()).toBe("beforeunload");
  await native.dismiss();
  await reload;
  await expect(page.getByLabel("Tripo 模型", { exact: true })).toHaveValue("qa-unsaved-model");
  await page.getByRole("link", { name: "资料库", exact: true }).first().click();
  await dialog.getByRole("button", { name: "丢弃并离开", exact: true }).click();
  await expect(page).not.toHaveURL(/\/settings$/);
  await page.goto("/settings");
  await expect(page.getByLabel("Tripo 模型", { exact: true })).toHaveValue(a.models.tripo);
  await page.getByRole("radio", { name: "替换", exact: true }).first().check();
  await expect(page.getByLabel("新的 Tripo 密钥", { exact: true })).toHaveValue("");
  await noSecretInBrowser(page);
  await page.getByLabel("Tripo 模型", { exact: true }).fill("qa-refresh-discard");
  await page.getByLabel("新的 Tripo 密钥", { exact: true }).fill(b.keys.tripo);
  const accepting = page.waitForEvent("dialog");
  const acceptedReload = page.reload({ timeout: 10_000 });
  const acceptDialog = await accepting;
  expect(acceptDialog.type()).toBe("beforeunload");
  await acceptDialog.accept();
  await acceptedReload;
  await expect(page.getByLabel("Tripo 模型", { exact: true })).toHaveValue(a.models.tripo);
  await page.getByRole("radio", { name: "替换", exact: true }).first().check();
  await expect(page.getByLabel("新的 Tripo 密钥", { exact: true })).toHaveValue("");
  await noSecretInBrowser(page);
  writeQaEvidence("02-unsaved-native-dialog", { nativeDialogType: "beforeunload", dismissPreservedForm: true, acceptedReloadDiscardedEdits: true, discardedSecretNotRestored: true, browserStorageLeak: false });
});

type ConfigView = components["schemas"]["ProviderSettingsData"];
type ConfigWrite = components["schemas"]["ProviderSettingsWrite"];
const SETTINGS_PATH = "/api/v1/settings/providers";

async function configView(request: APIRequestContext): Promise<ConfigView> {
  const response = await request.get(`${backend.base}${SETTINGS_PATH}`);
  expect(response.status()).toBe(200);
  expect(response.headers()["cache-control"]).toBe("no-store");
  const result = await response.json() as components["schemas"]["ProviderSettingsResponse"];
  expect([...Object.values(a.keys), ...Object.values(b.keys)].some((key) => JSON.stringify(result).includes(key)), "GET key leak boolean").toBe(false);
  return result.data;
}

function configWrite(view: ConfigView): ConfigWrite {
  const edit = (provider: "tripo" | "manualAi"): components["schemas"]["ProviderEdit"] => ({
    action: "update", baseUrl: view.saved[provider].baseUrl, model: view.saved[provider].model, keyAction: "keep",
  });
  return { revision: view.revision, tripo: edit("tripo"), manualAi: edit("manualAi") };
}

function alternateWrite(view: ConfigView): ConfigWrite {
  return {
    revision: view.revision,
    tripo: { action: "update", baseUrl: `${b.base}/v3`, model: b.models.tripo, keyAction: "replace", apiKey: b.keys.tripo },
    manualAi: { action: "update", baseUrl: `${b.base}/v1`, model: b.models.manualAi, keyAction: "replace", apiKey: b.keys.manualAi },
  };
}

async function putConfig(request: APIRequestContext, csrf: string, data: ConfigWrite) {
  return request.put(`${backend.base}${SETTINGS_PATH}`, { headers: { "x-csrf-token": csrf }, data });
}

function businessCounts() {
  const rows = execFileSync("/usr/bin/sqlite3", ["-readonly", "-json", path.join(backend.dataDir, "manual.sqlite3"),
    "SELECT (SELECT count(*) FROM jobs) jobs, (SELECT count(*) FROM quotes) quotes, (SELECT count(*) FROM provider_attempts) attempts, (SELECT count(*) FROM cost_ledger) ledger;"], { encoding: "utf8" });
  return JSON.parse(rows) as unknown;
}

async function errorReason(response: Awaited<ReturnType<APIRequestContext["post"]>>, reason: string) {
  expect(response.status()).toBe(422);
  const result = await response.json() as components["schemas"]["ApiErrorResponse"];
  expect((result.error.details as { reason?: string } | undefined)?.reason).toBe(reason);
}

test("AS-QA-03 浏览器保存B→刷新→真实重启→worker消费B（AC003/006/008/009/012）", async ({ page, request }) => {
  await openSettings(page);
  const csrf = await apiLogin(request, backend.base, QA_PASSWORD);
  const initial = await configView(request);
  const counts = businessCounts();
  await page.getByLabel("Tripo Base URL", { exact: true }).fill(`${b.base}/v3/`);
  await page.getByLabel("Tripo 模型", { exact: true }).fill(b.models.tripo);
  await page.getByLabel("说明书 AI Base URL", { exact: true }).fill(`${b.base}/v1`);
  await page.getByLabel("说明书 AI 模型", { exact: true }).fill(b.models.manualAi);
  await page.getByRole("radio", { name: "替换", exact: true }).nth(0).check();
  await page.getByLabel("新的 Tripo 密钥", { exact: true }).fill(b.keys.tripo);
  await page.getByRole("radio", { name: "替换", exact: true }).nth(1).check();
  await page.getByLabel(/^新的说明书 AI\s*密钥$/).fill(b.keys.manualAi);
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByText("已保存，重启服务后生效", { exact: true }).first()).toBeVisible();
  await expect(page.getByLabel("新的 Tripo 密钥", { exact: true })).toHaveCount(0);
  const saved = await configView(request);
  expect(saved.pending).toBe(true);
  expect(saved.active).toEqual(initial.active);
  expect(saved.saved.tripo.baseUrl).toBe(`${b.base}/v3`);
  expect(saved.saved.manualAi.model).toBe(b.models.manualAi);
  expect(saved.revision).not.toBe(initial.revision);
  expect(businessCounts()).toEqual(counts);
  expect(a.checks.tripoRequests + a.checks.manualRequests + b.checks.tripoRequests + b.checks.manualRequests).toBe(0);
  await page.reload();
  await expect(page.getByText("已保存，重启服务后生效", { exact: true }).first()).toBeVisible();
  await screenshot(page, "03-saved-pending");
  await noSecretInBrowser(page);
  expect(fs.statSync(path.join(backend.dataDir, "provider-overrides.json")).mode & 0o777).toBe(0o600);
  await backend.restart();
  const active = await configView(request);
  expect(active.pending).toBe(false);
  expect(active.active).toEqual(active.saved);
  await page.reload();
  await expect(page.getByText("当前运行配置已生效", { exact: true }).first()).toBeVisible();
  await screenshot(page, "03-restart-effective");
  const ready = await readyQuote(request, backend, 1);
  const jobUrl = `${backend.base}/api/v1/items/${ready.itemId}/jobs`;
  const key = "api-settings-qa-b-create";
  const create = await request.post(jobUrl, { headers: { "x-csrf-token": ready.csrf, "idempotency-key": key }, data: ready.jobBody });
  expect(create.status()).toBe(202);
  const created = await create.json() as components["schemas"]["JobResponse"];
  await waitForJob(request, backend.base, created.data.id, (job) => job.status === "succeeded", "B fixture completes");
  expect(b.checks.tripoRequests).toBeGreaterThan(0);
  expect(b.checks.manualRequests).toBeGreaterThan(0);
  expect(b.checks.authMatches).toBe(true);
  expect(b.checks.modelMatches).toBe(true);
  expect(a.checks.tripoRequests + a.checks.manualRequests).toBe(0);
  expect(b.upstream.paidSubmissions()).toBe(1);
  const beforeReplay = businessCounts();
  const current = await configView(request);
  // Existing accepted request may replay while pending, with no new work.
  const restore = await putConfig(request, ready.csrf, { revision: current.revision, tripo: { action: "restore" }, manualAi: { action: "restore" } });
  expect(restore.status()).toBe(200);
  const replay = await request.post(jobUrl, { headers: { "x-csrf-token": ready.csrf, "idempotency-key": key }, data: ready.jobBody });
  expect(replay.status()).toBe(202);
  expect((await replay.json() as components["schemas"]["JobResponse"]).data.id).toBe(created.data.id);
  expect(businessCounts()).toEqual(beforeReplay);
  expect(b.upstream.paidSubmissions()).toBe(1);
  // csrf is session scoped; the older token must not accidentally be used after readyQuote logs in.
  expect(typeof csrf).toBe("string");
  writeQaEvidence("03-restart-worker", { pendingBeforeRestart: saved.pending, pendingAfterRestart: active.pending, permissions: "0600", a: a.checks, b: b.checks, bPaidSubmissions: b.upstream.paidSubmissions(), idempotentReplaySameJob: true });
});

test("AS-QA-04 真实HTTP认证/CSRF、写入失败与旧修订冲突（AC002/004/005）", async ({ request }) => {
  expect((await request.get(`${backend.base}${SETTINGS_PATH}`)).status()).toBe(401);
  expect((await request.put(`${backend.base}${SETTINGS_PATH}`, { data: {} })).status()).toBe(401);
  const csrf = await apiLogin(request, backend.base, QA_PASSWORD);
  const initial = await configView(request);
  const body = alternateWrite(initial);
  const counts = businessCounts();
  const invalidHeaders: Record<string, string>[] = [{}, { "x-csrf-token": "wrong" }, { "x-csrf-token": csrf, Origin: "https://other.invalid" }];
  for (const headers of invalidHeaders) {
    expect((await request.put(`${backend.base}${SETTINGS_PATH}`, { headers, data: body })).status()).toBe(403);
  }
  expect(await configView(request)).toEqual(initial);
  const bad = { ...body, tripo: { ...body.tripo, baseUrl: `https://user:${b.keys.tripo}@example.invalid/v3` } };
  const invalid = await putConfig(request, csrf, bad);
  expect(invalid.status()).toBe(422);
  expect((await invalid.text()).includes(b.keys.tripo), "validation leak boolean").toBe(false);
  const file = path.join(backend.dataDir, "provider-overrides.json");
  fs.mkdirSync(file);
  expect((await putConfig(request, csrf, body)).status()).toBe(500);
  expect(await configView(request)).toEqual(initial);
  fs.rmdirSync(file);
  expect((await putConfig(request, csrf, body)).status()).toBe(200);
  const saved = await configView(request);
  const bytes = fs.readFileSync(file);
  const stale = await putConfig(request, csrf, { ...body, manualAi: { ...body.manualAi, model: "qa-stale-model" } });
  expect(stale.status()).toBe(409);
  expect(await configView(request)).toEqual(saved);
  expect(fs.readFileSync(file).equals(bytes), "stale save file unchanged").toBe(true);
  expect(businessCounts()).toEqual(counts);
  expect(a.checks.tripoRequests + b.checks.tripoRequests + a.checks.manualRequests + b.checks.manualRequests).toBe(0);
});

test("AS-QA-05 密钥清除屏蔽部署，单家恢复，缺配置不冒充可生成（AC006/007/015）", async ({ request }) => {
  const csrf = await apiLogin(request, backend.base, QA_PASSWORD);
  const initial = await configView(request);
  expect(initial.active.tripo.model).toBe(a.models.tripo); // explicit ENV beats private test TOML
  expect((await putConfig(request, csrf, alternateWrite(initial))).status()).toBe(200);
  const saved = await configView(request);
  const clear = configWrite(saved);
  clear.tripo.keyAction = "clear";
  clear.manualAi.model = null;
  expect((await putConfig(request, csrf, clear)).status()).toBe(200);
  await backend.restart();
  const cleared = await configView(request);
  expect(cleared.active.tripo.keyConfigured).toBe(false);
  expect(cleared.active.tripo.keySource).toBe("web");
  expect(cleared.active.manualAi.model).toBeNull();
  expect(cleared.active.manualAi.keyConfigured).toBe(true);
  const status = await request.get(`${backend.base}/api/v1/settings/status`);
  expect((await status.json() as components["schemas"]["SettingsStatusResponse"]).data.capabilities.generation).toBe(false);
  expect((await request.get(`${backend.base}/api/v1/health/ready`)).status()).toBe(200);
  const restore = configWrite(cleared); restore.tripo = { action: "restore" };
  expect((await putConfig(request, csrf, restore)).status()).toBe(200);
  await backend.restart();
  const restored = await configView(request);
  expect(restored.active.tripo).toEqual(initial.active.tripo);
  expect(restored.active.manualAi.baseUrl).toBe(`${b.base}/v1`);
  expect(restored.active.manualAi.model).toBeNull();
  expect(restored.active.manualAi.keySource).toBe("web");
  expect(restored.pending).toBe(false);
  expect(a.checks.tripoRequests + b.checks.tripoRequests + a.checks.manualRequests + b.checks.manualRequests).toBe(0);
  await backend.stop();
  fs.writeFileSync(path.join(backend.dataDir, "provider-overrides.json"), "{corrupt");
  await expect(backend.start()).rejects.toThrow(/QA backend exited/);
});

test("AS-QA-06 待重启门禁/旧报价失效/目录外模型与断网（AC011/012/015）", async ({ page, request }) => {
  const ready = await readyQuote(request, backend, 0);
  const before = await configView(request);
  const counts = businessCounts();
  expect((await putConfig(request, ready.csrf, alternateWrite(before))).status()).toBe(200);
  const estimateUrl = `${backend.base}/api/v1/items/${ready.itemId}/estimates`;
  const jobUrl = `${backend.base}/api/v1/items/${ready.itemId}/jobs`;
  const estimateBody: components["schemas"]["EstimateRequest"] = { preparationId: ready.jobBody.preparationId, photoIds: ready.jobBody.photoIds, modelPreset: "api-settings-qa-0" };
  await errorReason(await request.post(estimateUrl, { headers: { "x-csrf-token": ready.csrf }, data: estimateBody }), "providerConfigPending");
  await errorReason(await request.post(`${estimateUrl}/${ready.estimateId}/confirm`, { headers: { "x-csrf-token": ready.csrf } }), "providerConfigPending");
  await errorReason(await request.post(jobUrl, { headers: { "x-csrf-token": ready.csrf, "idempotency-key": "api-settings-qa-old-quote" }, data: ready.jobBody }), "providerConfigPending");
  expect((await request.get(`${backend.base}/api/v1/items/${ready.itemId}`)).status()).toBe(200);
  expect((await request.get(`${backend.base}/api/v1/health/ready`)).status()).toBe(200);
  expect(businessCounts()).toEqual(counts);
  await backend.restart();
  await errorReason(await request.post(jobUrl, { headers: { "x-csrf-token": ready.csrf, "idempotency-key": "api-settings-qa-old-after-restart" }, data: ready.jobBody }), "providerConfigChanged");
  expect(businessCounts()).toEqual(counts);
  expect(a.checks.tripoRequests + b.checks.tripoRequests + a.checks.manualRequests + b.checks.manualRequests).toBe(0);
  const unknown = configWrite(await configView(request));
  unknown.manualAi.model = "qa-model-outside-price-catalog";
  expect((await putConfig(request, ready.csrf, unknown)).status()).toBe(200);
  await b.stop(); // configuration saving and server readiness must not probe this address
  await backend.restart();
  const unavailable = await request.post(estimateUrl, { headers: { "x-csrf-token": ready.csrf }, data: { ...estimateBody, modelPreset: "api-settings-qa-1" } });
  expect(unavailable.status()).toBe(409);
  expect((await unavailable.json() as components["schemas"]["ApiErrorResponse"]).error.code).toBe("PRICE_CATALOG_MISSING");
  expect((await request.get(`${backend.base}/api/v1/health/ready`)).status()).toBe(200);
  expect((await request.get(`${backend.base}/api/v1/items/${ready.itemId}`)).status()).toBe(200);
  expect(businessCounts()).toEqual(counts);
  await openSettings(page);
  await expect(page.getByText("连接未验证", { exact: true }).first()).toBeVisible();
  await expect(page.getByLabel("说明书 AI 模型", { exact: true })).toHaveValue("qa-model-outside-price-catalog");
  await expect(page.locator(".settings-page")).not.toContainText(/生成能力\s+可用/);
  await screenshot(page, "06-unknown-model-offline-readiness");
});

test("AS-QA-07 保存与真实建单竞争只有一方受理（AC010/011）", async ({ request }) => {
  a.upstream.state.manualDelayMs = 5_000;
  const ready = await readyQuote(request, backend, 0);
  const before = await configView(request);
  const [saved, job] = await Promise.all([
    putConfig(request, ready.csrf, alternateWrite(before)),
    request.post(`${backend.base}/api/v1/items/${ready.itemId}/jobs`, { headers: { "x-csrf-token": ready.csrf, "idempotency-key": "api-settings-qa-create-race" }, data: ready.jobBody }),
  ]);
  expect(Number(saved.ok()) + Number(job.ok())).toBe(1);
  if (saved.ok()) {
    await errorReason(job, "providerConfigPending");
    expect((await configView(request)).pending).toBe(true);
  } else {
    expect(job.status()).toBe(202);
    await errorReason(saved, "providerConfigBusy");
    expect(await configView(request)).toEqual(before);
  }
  writeQaEvidence("07-create-save-race", { saveStatus: saved.status(), jobStatus: job.status(), exactlyOneAccepted: true });
});

test("AS-QA-08 读取失败/重复提交/字段错误/写失败/冲突重读（AC001/005）", async ({ page, request }) => {
  let failRead = true;
  let mode: "field" | "write" | "real" = "field";
  let puts = 0;
  let release!: () => void;
  const held = new Promise<void>((resolve) => { release = resolve; });
  await page.route(`**${SETTINGS_PATH}`, async (route) => {
    if (route.request().method() === "GET" && failRead) {
      await route.fulfill({ status: 500, json: { error: { code: "INTERNAL", message: "QA configuration read failure", requestId: "as-qa-read", details: null } } });
    } else if (route.request().method() === "PUT") {
      puts += 1;
      if (mode === "field") {
        await held;
        await route.fulfill({ status: 422, json: { error: { code: "VALIDATION_FAILED", message: "字段校验失败", requestId: "as-qa-field", details: { fields: [{ field: "manualAi.model", message: "QA 模型字段错误" }] } } } });
      } else if (mode === "write") {
        await route.fulfill({ status: 500, json: { error: { code: "INTERNAL", message: "无法写入私有 API 配置", requestId: "as-qa-write", details: null } } });
      } else await route.fallback();
    } else await route.fallback();
  });
  await loginViaUi(page, "", QA_PASSWORD);
  await page.goto("/settings");
  await expect(page.getByRole("alert")).toContainText("无法读取 API 配置");
  await expect(page.getByLabel("Tripo Base URL", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "保存配置", exact: true })).toHaveCount(0);
  failRead = false;
  await page.getByRole("button", { name: "重新读取", exact: true }).click();
  await expect(page.getByLabel("Tripo Base URL", { exact: true })).toHaveValue(`${a.base}/v3`);
  await page.getByLabel("说明书 AI 模型", { exact: true }).fill("qa-unsaved-manual");
  await page.getByRole("radio", { name: "替换", exact: true }).first().check();
  await page.getByLabel("新的 Tripo 密钥", { exact: true }).fill(b.keys.tripo);
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  const submitting = page.getByRole("button", { name: "正在保存…", exact: true });
  await expect(submitting).toBeDisabled();
  await expect(submitting).toHaveAttribute("aria-busy", "true");
  await expect(page.getByLabel("Tripo Base URL", { exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "恢复部署配置", exact: true }).first()).toBeDisabled();
  await page.locator(".provider-settings form").evaluate((form) => form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(puts).toBe(1);
  release();
  await expect(page.getByLabel("说明书 AI 模型", { exact: true })).toHaveAttribute("aria-invalid", "true");
  await expect(page.getByLabel("说明书 AI 模型", { exact: true })).toBeFocused();
  expect(await page.getByLabel("新的 Tripo 密钥", { exact: true }).inputValue() === b.keys.tripo, "field failure retains in-memory new key").toBe(true);
  mode = "write";
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("未能保存配置");
  await expect(page.getByLabel("说明书 AI 模型", { exact: true })).toHaveValue("qa-unsaved-manual");
  expect(await page.getByLabel("新的 Tripo 密钥", { exact: true }).inputValue() === b.keys.tripo, "write failure retains in-memory new key").toBe(true);
  mode = "real";
  const csrf = await apiLogin(request, backend.base, QA_PASSWORD);
  expect((await putConfig(request, csrf, alternateWrite(await configView(request)))).status()).toBe(200);
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("配置已在其他页面更新");
  await expect(page.getByRole("button", { name: "保存配置", exact: true })).toBeDisabled();
  expect(await page.getByLabel("新的 Tripo 密钥", { exact: true }).inputValue() === b.keys.tripo, "conflict retains in-memory new key").toBe(true);
  await noSecretInBrowser(page);
  await page.getByRole("button", { name: "重新加载已保存配置", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.getByRole("button", { name: "继续编辑", exact: true }).click();
  await expect(page.getByLabel("说明书 AI 模型", { exact: true })).toHaveValue("qa-unsaved-manual");
  await page.getByRole("button", { name: "重新加载已保存配置", exact: true }).click();
  await page.getByRole("button", { name: "丢弃并重新加载", exact: true }).click();
  await expect(page.getByLabel("说明书 AI 模型", { exact: true })).toHaveValue(b.models.manualAi);
  await expect(page.getByLabel("新的 Tripo 密钥", { exact: true })).toHaveCount(0);
  await screenshot(page, "08-conflict-explicit-reload");
});

test("AS-QA-09 手机密钥空替换/切换清空、取消恢复、清除与单家恢复（AC006/007/014）", async ({ page, request }) => {
  await page.setViewportSize({ width: 375, height: 900 });
  await openSettings(page);
  const csrf = await apiLogin(request, backend.base, QA_PASSWORD);
  const initial = await configView(request);
  let writes = 0;
  page.on("request", (req) => { if (req.url().endsWith(SETTINGS_PATH) && req.method() === "PUT") writes += 1; });
  await page.getByRole("radio", { name: "替换", exact: true }).first().check();
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  const key = page.getByLabel("新的 Tripo 密钥", { exact: true });
  await expect(key).toBeFocused();
  await expect(key).toHaveAttribute("aria-invalid", "true");
  expect(writes).toBe(0);
  await key.fill(b.keys.tripo);
  await page.getByRole("radio", { name: "保留现有", exact: true }).first().check();
  await page.getByRole("radio", { name: "替换", exact: true }).first().check();
  await expect(key).toHaveValue("");
  await key.fill(b.keys.tripo);
  await page.getByLabel("说明书 AI 模型", { exact: true }).fill("qa-other-unsaved");
  await page.getByRole("button", { name: "恢复部署配置", exact: true }).first().click();
  await expect(key).toHaveCount(0);
  await expect(page.getByText("保存后恢复部署配置", { exact: true })).toBeVisible();
  expect(writes).toBe(0);
  await page.getByRole("button", { name: "取消恢复", exact: true }).click();
  await expect(page.getByLabel("Tripo 模型", { exact: true })).toHaveValue(a.models.tripo);
  await expect(page.getByLabel("说明书 AI 模型", { exact: true })).toHaveValue("qa-other-unsaved");
  await expect(page.getByRole("radio", { name: "保留现有", exact: true }).first()).toBeChecked();
  await page.getByRole("radio", { name: "清除", exact: true }).first().check();
  await expect(page.getByText(/也不会自动使用部署配置中的密钥/)).toBeVisible();
  for (const radio of await page.getByRole("radio").all()) {
    const box = await radio.locator("..").boundingBox();
    expect(box?.height).toBeGreaterThanOrEqual(44);
  }
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  await screenshot(page, "09-mobile-clear-impact");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByText("所有修改已保存", { exact: true })).toBeVisible();
  expect((await configView(request)).saved.tripo.keyConfigured).toBe(false);
  await page.getByRole("button", { name: "恢复部署配置", exact: true }).first().click();
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByText("所有修改已保存", { exact: true })).toBeVisible();
  const restored = await configView(request);
  expect(restored.saved.tripo).toEqual(initial.saved.tripo);
  expect(restored.saved.manualAi.model).toBe("qa-other-unsaved");
  expect(restored.pending).toBe(true);
  expect(writes).toBe(2);
  await noSecretInBrowser(page);
  // Only pending, with no local edits: navigation must not show a discard dialog.
  await page.getByRole("link", { name: "资料库", exact: true }).first().click();
  await expect(page).not.toHaveURL(/\/settings$/);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(typeof csrf).toBe("string");
});
