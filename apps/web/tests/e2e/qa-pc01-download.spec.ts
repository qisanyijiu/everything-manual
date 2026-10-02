import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { expect, request, test, type APIRequestContext, type Locator, type Page } from "@playwright/test";
import { apiLogin, loginViaUi } from "./helpers";
import { BACKEND_PASSWORD, LocalFixture, seedJob, waitForJob } from "./job-recovery-harness";
import { Pc01QaBackend } from "./qa-pc01-backend";
import { E2E_WEB_PORT, REPO_ROOT, fixturePath } from "./runtime";
import { installRealBackendRouting } from "./viewer-harness";

test.describe.configure({ mode: "serial", timeout: 120000 });
const web = `http://127.0.0.1:${E2E_WEB_PORT}`;
const evidence = path.join(REPO_ROOT, "var/pc01-qa-round1/evidence");
const python = "/Users/qsyj/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3";
const description = "包含原件、模型、manifest 与哈希，用于该发布版本的数据便携与灾备。不是整库备份，也不承诺双击运行网站。";
let fixture: LocalFixture, backend: Pc01QaBackend, api: APIRequestContext, csrf: string;
let itemId: string, emptyId: string, draftId: string, partId: string, oldId: string, newId: string;
const summary: Record<string, unknown> = { scope: "PC01 frozen PRD/UI1", realExportResponses: true };

function write(name: string, value: unknown) { fs.writeFileSync(path.join(evidence, name), JSON.stringify(value, null, 2) + "\n"); }
async function getDraft() {
  const response = await api.get(`${backend.base}/api/v1/items/${itemId}/drafts/${draftId}`);
  expect(response.status()).toBe(200);
  const etag = response.headers().etag;
  if (!etag) throw new Error("QA draft response must include an ETag");
  return { data: (await response.json()).data, etag };
}
async function patch(body: unknown) {
  const current = await getDraft();
  const response = await api.patch(`${backend.base}/api/v1/items/${itemId}/drafts/${draftId}`, {
    headers: { "x-csrf-token": csrf, "if-match": current.etag }, data: body,
  });
  expect(response.status()).toBe(200);
}
async function publish() {
  const current = await getDraft();
  const response = await api.post(`${backend.base}/api/v1/items/${itemId}/drafts/${draftId}/publish`, {
    headers: { "x-csrf-token": csrf, "if-match": current.etag, "idempotency-key": randomUUID() },
  });
  expect(response.status()).toBe(201);
  return (await response.json()).data.id as string;
}
function counters() {
  return JSON.parse(execFileSync(python, ["-c", `
import json,sqlite3,sys
c=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True)
r={t:c.execute('select count(*) from '+t).fetchone()[0] for t in ['jobs','provider_attempts','cost_ledger']}
r['ledgerAmounts']=c.execute('select provider,currency,state,sum(reserved),sum(actual) from cost_ledger group by provider,currency,state order by provider,currency,state').fetchall()
print(json.dumps(r))
`, path.join(backend.dataDir, "manual.sqlite3")], { encoding: "utf8" }));
}
function inspectZip(file: string, id: string) {
  return JSON.parse(execFileSync(python, [path.join(REPO_ROOT, "var/pc01-qa-round1/inspect_zip.py"), file, id], { encoding: "utf8" }));
}
async function tabTo(page: Page, target: Locator) {
  for (let index = 0; index < 100; index += 1) {
    await page.keyboard.press("Tab");
    if (await target.evaluate(element => element === document.activeElement)) {
      const focus = await target.evaluate(element => ({ style: getComputedStyle(element).outlineStyle, width: getComputedStyle(element).outlineWidth }));
      expect(focus.style).not.toBe("none");
      expect(Number.parseFloat(focus.width)).toBeGreaterThanOrEqual(2);
      return;
    }
  }
  throw new Error("Download action was not reachable by Tab");
}
async function open(page: Page, route: string) {
  const routing = await installRealBackendRouting(page, backend.base);
  await loginViaUi(page, web, BACKEND_PASSWORD);
  await page.goto(web + route);
  return routing;
}
async function captureGeometry(page: Page, control: Locator, label: string) {
  const button = control.getByRole("button");
  const box = await button.boundingBox();
  expect(box?.height).toBeGreaterThanOrEqual(44);
  expect(box?.width).toBeGreaterThanOrEqual(44);
  const geometry = await page.evaluate(() => ({ width: innerWidth, scrollWidth: document.documentElement.scrollWidth }));
  expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.width);
  const described = await button.evaluate(element => (element.getAttribute("aria-describedby") ?? "").split(/\s+/).map(id => {
    const referenced = document.getElementById(id);
    return { id, exists: referenced !== null, text: referenced?.textContent ?? "", font: referenced ? getComputedStyle(referenced).fontSize : "0" };
  }));
  expect(described.every(entry => entry.exists)).toBe(true);
  expect(described.some(entry => entry.text === description)).toBe(true);
  expect(described.every(entry => Number.parseFloat(entry.font) >= 12)).toBe(true);
  expect(Number.parseFloat(await button.evaluate(element => getComputedStyle(element).fontSize))).toBeGreaterThanOrEqual(12);
  if (geometry.width === 375) await page.screenshot({ path: path.join(evidence, label + "-375.png"), fullPage: true });
  return { label, ...geometry, button: box, described };
}
async function realDownload(page: Page, id: string, filename: string, key = "Enter") {
  const control = page.getByTestId(`release-download-${id}`);
  const button = control.getByRole("button");
  await tabTo(page, button);
  const pending = page.waitForEvent("download");
  await page.keyboard.press(key);
  const download = await pending;
  const file = path.join(evidence, filename);
  await download.saveAs(file);
  await expect(control.getByRole("status")).toContainText(`已发起下载：${download.suggestedFilename()}`);
  await expect(button).toBeFocused();
  return { file, suggestedFilename: download.suggestedFilename() };
}

test.beforeAll(async () => {
  test.setTimeout(180000);
  fs.mkdirSync(evidence, { recursive: true });
  fixture = new LocalFixture();
  await fixture.start();
  backend = new Pc01QaBackend("pc01-independent-qa");
  await backend.start(fixture);
  api = await request.newContext();
  const seeded = await seedJob(api, backend, "PC01 QA two-version isolated fixture");
  itemId = seeded.itemId;
  const job = await waitForJob(api, backend.base, seeded.jobId, entry => entry.status === "succeeded", "QA fixture ready", 120000);
  draftId = job.draftId ?? "";
  csrf = await apiLogin(api, backend.base, BACKEND_PASSWORD);
  const knowledge = (await getDraft()).data.knowledge.knowledge as Record<string, { id: string }[]>;
  const parts = knowledge.parts ?? [];
  const firstPart = parts[0];
  if (!firstPart) throw new Error("QA fixture must produce a reviewable part");
  partId = firstPart.id;
  const entities = Object.fromEntries([...parts, ...(knowledge.steps ?? []), ...(knowledge.specs ?? [])].map(entry => [entry.id, {
    reviewStatus: "confirmed", ...(parts.some(part => part.id === entry.id) ? { textOnly: true } : {}),
  }]));
  await patch({ entities, modelReview: { loaded: true, userConfirmed: true } });
  oldId = await publish();
  await patch({ entities: { [partId]: { reviewStatus: "confirmed", userEdited: { description: "QA second published description" } } } });
  newId = await publish();
  const empty = await api.post(`${backend.base}/api/v1/items`, { headers: { "x-csrf-token": csrf }, data: { name: "QA foreign unshared item", model: "QA-empty" } });
  expect(empty.status()).toBe(201);
  emptyId = (await empty.json()).data.id;
  const foreign = Buffer.concat([fs.readFileSync(fixturePath("sample-manual-text.pdf")), Buffer.from("\n% QA_FOREIGN_UNPUBLISHED_ASSET\n")]);
  const foreignAsset = await api.post(`${backend.base}/api/v1/items/${emptyId}/assets`, { headers: { "x-csrf-token": csrf }, multipart: {
    purpose: "document", file: { name: "foreign.pdf", mimeType: "application/pdf", buffer: foreign },
  } });
  expect(foreignAsset.status()).toBe(201);
  summary.ids = { itemId, draftId, oldId, newId, foreignItemId: emptyId };
  write("summary.json", summary);
});

test.afterAll(async () => { await api?.dispose(); await backend?.cleanup(fixture); });

test("QA AC001/008/010: true ZIP versions, parsed PDF/GLB, immutable contents and no new costs offline", async ({ page }) => {
  const routing = await open(page, `/items/${itemId}/releases/${oldId}`);
  const before = { db: counters(), provider: { ...fixture.counts } };
  await expect(page.getByText(description, { exact: true })).toBeVisible();
  const first = await realDownload(page, oldId, "older-reader.zip");
  expect(first.suggestedFilename).toBe(`release-${oldId}.zip`);
  const a = inspectZip(first.file, oldId);
  await patch({ entities: { [partId]: { reviewStatus: "confirmed", userEdited: { description: "QA unpublished third change" } } } });
  await fixture.stop();
  const repeated = await realDownload(page, oldId, "older-offline-after-edit.zip", "Space");
  expect(inspectZip(repeated.file, oldId)).toEqual(a);
  await page.getByRole("link", { name: "返回版本列表" }).click();
  await expect(page.getByRole("heading", { name: "发布版本", exact: true })).toBeVisible();
  const latest = await realDownload(page, newId, "newer-list.zip");
  const b = inspectZip(latest.file, newId);
  expect(b.manifestSha256).not.toBe(a.manifestSha256);
  expect(counters()).toEqual(before.db);
  expect(fixture.counts).toEqual(before.provider);
  expect(routing.external).toEqual([]);
  expect(routing.fulfilled).toBe(0);
  summary.archives = { old: a, newer: b, before, after: { db: counters(), provider: fixture.counts }, providerOffline: true, syntheticSuccessResponses: 0 };
  write("summary.json", summary);
});

test("QA AC002: empty, loading and missing entities cannot export", async ({ page }) => {
  await open(page, `/items/${emptyId}/releases`);
  await expect(page.getByText("该物品还没有发布版本", { exact: false })).toBeVisible();
  await expect(page.getByRole("button", { name: /下载/ })).toHaveCount(0);
  let finish!: () => void;
  const gate = new Promise<void>(resolve => { finish = resolve; });
  const entityRoute = `**/api/v1/items/${itemId}/releases/${oldId}`;
  await page.route(entityRoute, async route => { await gate; await route.continue({ url: `${backend.base}/api/v1/items/${itemId}/releases/${oldId}` }); });
  await page.goto(`${web}/items/${itemId}/releases/${oldId}`);
  await expect(page.getByRole("button", { name: /下载/ })).toHaveCount(0);
  finish();
  await expect(page.getByTestId(`release-download-${oldId}`)).toBeVisible();
  await page.unroute(entityRoute);
  await page.goto(`${web}/items/${itemId}/releases/01930000-0000-7000-8000-000000000000`);
  await expect(page.getByRole("alert")).toContainText("发布版本读取失败");
  await expect(page.getByRole("button", { name: /下载/ })).toHaveCount(0);
  summary.emptyLoadingMissing = "passed"; write("summary.json", summary);
});

test("QA AC003/004/005/007/009: Tab keyboard, long filenames and four states on both entries at three widths", async ({ page }) => {
  const routing = await open(page, `/items/${itemId}/releases`);
  const geometries = [];
  const longName = `manual_${"a".repeat(140)}.zip`;
  const routePattern = `**/api/v1/releases/${oldId}/export`;
  const downloadCounts = [];
  for (const width of [375, 1024, 1440]) for (const entry of ["list", "reader"]) {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto(`${web}/items/${itemId}/releases${entry === "reader" ? "/" + oldId : ""}`);
    const control = page.getByTestId(`release-download-${oldId}`), button = control.getByRole("button");
    await expect(control).toBeVisible();
    await expect(page.getByText(description, { exact: true })).toBeVisible();
    await tabTo(page, button);
    geometries.push(await captureGeometry(page, control, `${entry}-idle`));
    let release!: () => void, attempts = 0;
    const gate = new Promise<void>(resolve => { release = resolve; });
    await page.route(routePattern, async route => {
      attempts++;
      await gate;
      const response = await route.fetch({ url: `${backend.base}/api/v1/releases/${oldId}/export` });
      await route.fulfill({ response, headers: { ...response.headers(), "content-disposition": `attachment; filename="${longName}"` } });
    });
    const pendingDownload = page.waitForEvent("download");
    await page.keyboard.press("Enter");
    await expect(button).toHaveText("正在打包…");
    await page.keyboard.press("Space"); await page.keyboard.press("Enter");
    await expect.poll(() => attempts).toBe(1);
    geometries.push(await captureGeometry(page, control, `${entry}-pending`));
    release();
    const downloaded = await pendingDownload;
    expect(downloaded.suggestedFilename()).toBe(longName);
    await expect(control.getByRole("status")).toContainText(longName);
    await expect(button).toBeFocused();
    geometries.push(await captureGeometry(page, control, `${entry}-long-success`));
    await page.unroute(routePattern);
    await page.route(routePattern, async route => { attempts++; await route.continue({ url: `${backend.base}/api/v1/releases/01930000-0000-7000-8000-000000000000/export` }); });
    await page.keyboard.press("Space");
    await expect(control.getByRole("alert")).toContainText("该发布版本已无法找到");
    await expect(control.getByRole("alert")).toContainText("诊断 ID");
    await expect(button).toBeFocused();
    geometries.push(await captureGeometry(page, control, `${entry}-real404`));
    await page.unroute(routePattern);
    const invalidName = width === 375 ? undefined : width === 1024 ? 'attachment; filename="../bad.zip"' : "attachment; filename*=UTF-8''bad%00name.zip";
    await page.route(routePattern, async route => {
      attempts++;
      const response = await route.fetch({ url: `${backend.base}/api/v1/releases/${oldId}/export` });
      const headers = { ...response.headers() };
      delete headers["content-disposition"];
      if (invalidName !== undefined) headers["content-disposition"] = invalidName;
      await route.fulfill({ response, headers });
    });
    const retryDownload = page.waitForEvent("download");
    await page.keyboard.press("Enter");
    expect((await retryDownload).suggestedFilename()).toBe(`release-${oldId}.zip`);
    await expect(control.getByRole("alert")).toHaveCount(0);
    await expect(control.getByRole("status")).toContainText(`release-${oldId}.zip`);
    await expect(button).toBeFocused();
    expect(attempts).toBe(3);
    downloadCounts.push({ width, entry, requests: attempts, pendingRequests: 1, explicitSuccesses: 2 });
    await page.unroute(routePattern);
  }
  expect(routing.external).toEqual([]);
  write("geometry.json", geometries); summary.keyboardAndFilenameMatrix = downloadCounts; write("summary.json", summary);
});

test("QA AC004/006: unsafe responses, failure/pending leave and 401 require explicit recovery", async ({ page }) => {
  const routing = await open(page, `/items/${itemId}/releases/${oldId}`);
  const pattern = `**/api/v1/releases/${oldId}/export`;
  const control = page.getByTestId(`release-download-${oldId}`);
  const errors: string[] = [], downloads: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("download", download => downloads.push(download.suggestedFilename()));
  for (const mode of ["network", "403", "500", "json", "html", "fake-zip", "truncated-zip"]) {
    let calls = 0;
    await page.route(pattern, async route => {
      calls++;
      if (mode === "network") return route.abort("failed");
      await route.fulfill({ status: mode === "403" || mode === "500" ? Number(mode) : 200,
        headers: { "content-type": mode.includes("zip") ? "application/zip" : mode === "html" ? "text/html" : "application/json", "x-request-id": "qa-pc01-diagnostic" },
        body: mode === "truncated-zip" ? "PK\u0003\u0004truncated" : mode === "html" ? "<html>/private/qa-internal</html>" : '{"error":"/private/qa-internal"}' });
    });
    await control.getByRole("button").click();
    await expect(control.getByRole("alert")).toBeVisible();
    await expect(control.getByRole("alert")).not.toContainText("qa-internal");
    if (mode !== "network") await expect(control.getByRole("alert")).toContainText("qa-pc01-diagnostic");
    expect(calls).toBe(1); expect(downloads).toEqual([]);
    await page.unroute(pattern);
  }
  await page.getByRole("link", { name: "返回版本列表" }).click();
  await page.getByRole("link", { name: "打开阅读器" }).last().click();
  await expect(page.getByRole("heading", { name: "已发布说明书", exact: true })).toBeVisible();
  await expect(control.getByRole("alert")).toHaveCount(0);
  await expect(control.getByRole("button")).toHaveText("下载说明书资料包");
  let finish!: () => void;
  const gate = new Promise<void>(resolve => { finish = resolve; });
  await page.route(pattern, async route => { await gate; await route.continue({ url: `${backend.base}/api/v1/releases/${oldId}/export` }).catch(() => {}); });
  await control.getByRole("button").click();
  await expect(control.getByRole("button")).toHaveText("正在打包…");
  await page.getByRole("link", { name: "返回版本列表" }).click();
  await expect(page.getByRole("heading", { name: "发布版本", exact: true })).toBeVisible();
  finish(); await page.unroute(pattern);
  await page.getByRole("link", { name: "打开阅读器" }).last().click();
  await expect(page.getByRole("heading", { name: "已发布说明书", exact: true })).toBeVisible();
  await expect(control.getByRole("button")).toHaveText("下载说明书资料包");
  expect(downloads).toEqual([]);
  let unauthorized = 0;
  await page.route(pattern, async route => { unauthorized++; await route.fulfill({ status: 401, contentType: "application/json", body: JSON.stringify({ error: { code: "UNAUTHORIZED", message: "Session expired", requestId: "qa-session", details: null } }) }); });
  await control.getByRole("button").click();
  await expect(page).toHaveURL(/\/login\?next=/);
  expect(new URL(page.url()).searchParams.get("next")).toBe(`/items/${itemId}/releases/${oldId}`);
  await page.unroute(pattern);
  await page.getByLabel("密码").fill(BACKEND_PASSWORD);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(control.getByRole("button")).toHaveText("下载说明书资料包");
  expect(unauthorized).toBe(1); expect(downloads).toEqual([]);
  await realDownload(page, oldId, "explicit-after-session.zip");
  expect(downloads).toHaveLength(1); expect(errors).toEqual([]); expect(routing.external).toEqual([]);
  summary.recovery = { rejectedModes: 7, automaticDownloads: 0, explicitFinalDownloads: 1, pageErrors: errors, externalRequests: routing.external };
  write("summary.json", summary);
});

test("QA AC009/resource cleanup: selected part and original page survive download and viewport changes", async ({ page }) => {
  await page.addInitScript(() => {
    const state = { created: [] as string[], revoked: [] as string[] };
    (window as unknown as { __pc01Urls: typeof state }).__pc01Urls = state;
    const create = URL.createObjectURL.bind(URL), revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = blob => {
      const url = create(blob);
      if (blob instanceof Blob && blob.type === "application/zip") state.created.push(url);
      return url;
    };
    URL.revokeObjectURL = url => { if (state.created.includes(url)) state.revoked.push(url); revoke(url); };
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await open(page, `/items/${itemId}/releases/${oldId}`);
  const selectedPart = page.getByTestId(`reader-part-${partId}`).getByRole("button").first();
  await selectedPart.click();
  await expect(selectedPart).toHaveAttribute("aria-current", "true");
  await expect(page.getByTestId("original-page-label")).toHaveText("第 1 / 2 页");
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(page.getByTestId("original-page-label")).toHaveText("第 2 / 2 页");
  const stepBefore = await page.getByTestId("reader-current-step").textContent();
  await page.setViewportSize({ width: 375, height: 1000 });
  await realDownload(page, oldId, "selection-retained.zip");
  await expect.poll(() => page.evaluate(() => {
    const state = (window as unknown as { __pc01Urls: { created: string[]; revoked: string[] } }).__pc01Urls;
    return state.created.length === 1 && state.created.every(url => state.revoked.includes(url));
  })).toBe(true);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await expect(selectedPart).toHaveAttribute("aria-current", "true");
  await expect(page.getByTestId("original-page-label")).toHaveText("第 2 / 2 页");
  await expect(page.getByTestId("reader-current-step")).toHaveText(stepBefore ?? "");
  const urlCounts = await page.evaluate(() => {
    const state = (window as unknown as { __pc01Urls: { created: string[]; revoked: string[] } }).__pc01Urls;
    return { created: state.created.length, revoked: state.revoked.length };
  });
  summary.readingContext = { selectedPartRetained: true, pageRetained: 2, stepRetained: true, viewportSequence: [1440, 375, 1440], zipObjectUrls: urlCounts };
  write("summary.json", summary);
});
