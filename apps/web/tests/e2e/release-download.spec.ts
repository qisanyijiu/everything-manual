import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

import { expect, request, test, type APIRequestContext, type Page } from "@playwright/test";

import { apiLogin, loginViaUi } from "./helpers";
import { BACKEND_PASSWORD, buildTestServerBinary, LocalFixture, seedJob, TestBackend, waitForJob } from "./job-recovery-harness";
import { E2E_WEB_PORT, REPO_ROOT } from "./runtime";
import { installRealBackendRouting } from "./viewer-harness";

test.describe.configure({ timeout: 300_000, mode: "serial" });
const web = `http://127.0.0.1:${E2E_WEB_PORT}`;
const evidenceDir = path.join(REPO_ROOT, "artifacts/prd-completion/pc01-rd");
let fixture: LocalFixture;
let backend: TestBackend;
let api: APIRequestContext;
let csrf = "";
let itemId = "";
let draftId = "";
let oldRelease = "";
let newRelease = "";
let firstPartId = "";

async function draft() {
  const response = await api.get(`${backend.base}/api/v1/items/${itemId}/drafts/${draftId}`);
  expect(response.status()).toBe(200);
  return { data: (await response.json()).data, etag: response.headers()["etag"] ?? "" };
}

async function patch(body: unknown) {
  const current = await draft();
  const response = await api.patch(`${backend.base}/api/v1/items/${itemId}/drafts/${draftId}`, {
    headers: { "x-csrf-token": csrf, "if-match": current.etag }, data: body,
  });
  expect(response.status(), await response.text()).toBe(200);
}

async function publish() {
  const current = await draft();
  const response = await api.post(`${backend.base}/api/v1/items/${itemId}/drafts/${draftId}/publish`, {
    headers: { "x-csrf-token": csrf, "if-match": current.etag, "idempotency-key": randomUUID() },
  });
  expect(response.status(), await response.text()).toBe(201);
  return (await response.json()).data.id as string;
}

test.beforeAll(async () => {
  test.setTimeout(900_000);
  buildTestServerBinary();
  fs.mkdirSync(evidenceDir, { recursive: true });
  fixture = new LocalFixture();
  await fixture.start();
  backend = new TestBackend("pc01-download");
  await backend.start(fixture);
  api = await request.newContext();
  const seeded = await seedJob(api, backend, "PC01 independent release archive fixture");
  itemId = seeded.itemId;
  const job = await waitForJob(api, backend.base, seeded.jobId, (entry) => entry.status === "succeeded", "published download fixture", 180_000);
  draftId = job.draftId ?? "";
  csrf = await apiLogin(api, backend.base, BACKEND_PASSWORD);
  const current = await draft();
  const knowledge = current.data.knowledge.knowledge as Record<string, { id: string }[]>;
  firstPartId = knowledge.parts?.[0]?.id ?? "";
  const entities = Object.fromEntries([...(knowledge.parts ?? []), ...(knowledge.steps ?? []), ...(knowledge.specs ?? [])].map((entry) => [entry.id, {
    reviewStatus: "confirmed", ...(knowledge.parts?.some((part) => part.id === entry.id) ? { textOnly: true } : {}),
  }]));
  await patch({ entities, modelReview: { loaded: true, userConfirmed: true } });
  oldRelease = await publish();
  await patch({ entities: { [firstPartId]: { reviewStatus: "confirmed", userEdited: { description: "Second release editorial content" } } } });
  newRelease = await publish();
});

test.afterAll(async () => {
  await api?.dispose();
  await backend?.cleanup(fixture);
});

async function open(page: Page, route: string) {
  const routing = await installRealBackendRouting(page, backend.base);
  await loginViaUi(page, web, BACKEND_PASSWORD);
  await page.goto(web + route);
  return routing;
}

async function download(page: Page, releaseId: string, outputName: string, key = "Enter") {
  const control = page.getByTestId(`release-download-${releaseId}`);
  const button = control.getByRole("button");
  await button.focus();
  const pending = page.waitForEvent("download", { timeout: 15_000 }).catch(async () => {
    throw new Error(`No browser download: ${await control.textContent()}`);
  });
  await button.press(key);
  const file = await pending;
  expect(file.suggestedFilename()).toBe(`release-${releaseId}.zip`);
  const filename = path.join(evidenceDir, outputName);
  await file.saveAs(filename);
  await expect(control.getByRole("status")).toContainText(`已发起下载：${file.suggestedFilename()}`);
  await expect(button).toBeFocused();
  return filename;
}

function inspectZip(file: string, releaseId: string) {
  const summary = execFileSync("python3", ["-c", `
import hashlib,json,sys,zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    assert z.testzip() is None
    names=z.namelist()
    assert all(not n.startswith('/') and '..' not in n and '\\\\' not in n for n in names)
    m=json.loads(z.read('manifest.json'))
    assert m['release']['releaseId']==sys.argv[2]
    assert m['schemaVersion']=='manual_release_export_v1'
    frozen=z.read('release/manifest.json')
    assert hashlib.sha256(frozen).hexdigest()==m['releaseManifest']['sha256']
    declared={'manifest.json','release/manifest.json'}
    types=set()
    for f in m['files']:
        content=z.read(f['path']); declared.add(f['path'])
        assert hashlib.sha256(content).hexdigest()==f['sha256']
        assert f['source']
        if f['path'].endswith('.pdf'): assert content.startswith(b'%PDF'); types.add('pdf')
        if f['path'].endswith('.glb'): assert content.startswith(b'glTF'); types.add('glb')
    assert set(names)==declared and types=={'pdf','glb'}
    entire=b'\\n'.join(z.read(n) for n in names)
    for forbidden in [b't17-fake-tripo-key',b't17-fake-manual-ai-key',b'Bearer ',b'http://',b'https://',b'/private/',b'/Users/']:
        assert forbidden not in entire
    print(json.dumps({'releaseId':sys.argv[2],'entries':len(names),'assetTypes':sorted(types),'manifestSha256':hashlib.sha256(frozen).hexdigest(),'assetHashes':{f['path']:f['sha256'] for f in m['files']}}))
`, file, releaseId], { encoding: "utf8" });
  return JSON.parse(summary) as { manifestSha256: string };
}

test("PC01 real ZIP from old reader and newer version row; frozen after draft edits and no provider calls", async ({ page }) => {
  const counts = { ...fixture.counts };
  const routing = await open(page, `/items/${itemId}/releases/${oldRelease}`);
  const first = await download(page, oldRelease, "old-reader.zip");
  const firstManifest = inspectZip(first, oldRelease);
  await patch({ entities: { [firstPartId]: { reviewStatus: "confirmed", userEdited: { description: "Unpublished third edit" } } } });
  // Existing versions remain usable even when the fixture provider is offline.
  await fixture.stop();
  const second = await download(page, oldRelease, "old-after-draft-edit.zip", "Space");
  // The export envelope records the export time; the frozen release and asset
  // hashes must remain identical, rather than the whole timestamped ZIP.
  expect(inspectZip(second, oldRelease)).toEqual(firstManifest);
  await page.getByRole("link", { name: "返回版本列表" }).click();
  const latest = await download(page, newRelease, "new-list.zip");
  const newManifest = inspectZip(latest, newRelease);
  expect(newManifest.manifestSha256).not.toBe(firstManifest.manifestSha256);
  expect(fixture.counts).toEqual(counts);
  expect(routing.external).toEqual([]); expect(routing.fulfilled).toBe(0);
  fs.writeFileSync(path.join(evidenceDir, "zip-summary.json"), JSON.stringify({ firstManifest, newManifest, noProviderCallsDuringExport: true, noSyntheticSuccess: true }, null, 2));
});

test("PC01 error/retry remains scoped, safe, keyboard accessible at three widths", async ({ page }) => {
  const routing = await open(page, `/items/${itemId}/releases`);
  let attempts = 0;
  const route = `**/api/v1/releases/${oldRelease}/export`;
  await page.route(route, async (entry) => {
    attempts += 1;
    // A real authenticated backend error, no synthetic error body.
    await entry.continue({ url: `${backend.base}/api/v1/releases/01930000-0000-7000-8000-000000000000/export` });
  });
  const geometries = [];
  for (const width of [375, 1024, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    const control = page.getByTestId(`release-download-${oldRelease}`);
    const button = control.getByRole("button");
    await button.focus(); await button.press("Enter");
    await expect(control.getByRole("alert")).toContainText("该发布版本已无法找到");
    await expect(control.getByRole("alert")).toContainText("诊断 ID");
    await expect(button).toBeFocused();
    const box = await button.boundingBox();
    expect(box?.height).toBeGreaterThanOrEqual(44); expect(box?.width).toBeGreaterThanOrEqual(44);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    geometries.push({ width, button: box });
    await page.screenshot({ path: path.join(evidenceDir, `list-error-${width}.png`), fullPage: true });
  }
  expect(attempts).toBe(3);
  await page.unroute(route);
  await download(page, oldRelease, "old-retry.zip", "Space");
  expect(routing.external).toEqual([]);
  fs.writeFileSync(path.join(evidenceDir, "layout-summary.json"), JSON.stringify(geometries, null, 2));
});

test("PC01 pending is single-flight and leaving discards it; reader size and selection remain usable", async ({ page }) => {
  await open(page, `/items/${itemId}/releases/${oldRelease}`);
  const route = `**/api/v1/releases/${oldRelease}/export`;
  let attempts = 0;
  let releaseRequest!: () => void;
  const gate = new Promise<void>((resolve) => { releaseRequest = resolve; });
  await page.route(route, async (entry) => {
    attempts += 1;
    await gate;
    await entry.continue({ url: `${backend.base}/api/v1/releases/${oldRelease}/export` }).catch(() => undefined);
  });
  const downloadEvents: string[] = [];
  page.on("download", (entry) => downloadEvents.push(entry.suggestedFilename()));
  const control = page.getByTestId(`release-download-${oldRelease}`);
  const button = control.getByRole("button");
  await button.focus(); await button.press("Enter");
  await expect(button).toHaveText("正在打包…");
  await button.press("Space"); await button.press("Enter");
  expect(attempts).toBe(1);
  for (const width of [375, 1024, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(control.getByRole("status")).toContainText("可离开页面");
    const box = await button.boundingBox();
    expect(box?.height).toBeGreaterThanOrEqual(44);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: path.join(evidenceDir, `reader-pending-${width}.png`), fullPage: true });
  }
  await page.getByRole("link", { name: "返回版本列表" }).click();
  await expect(page.getByRole("heading", { name: "发布版本", exact: true })).toBeVisible();
  releaseRequest();
  await page.unroute(route);
  await page.getByRole("link", { name: "打开阅读器" }).last().click();
  // List and reader deliberately share the per-release control; wait for the
  // route to finish before focusing, rather than pressing the old list button.
  await expect(page.getByRole("heading", { name: "已发布说明书", exact: true })).toBeVisible();
  await expect(control.getByRole("button")).toHaveText("下载说明书资料包");
  expect(downloadEvents).toEqual([]);
  await download(page, oldRelease, "after-leave-return.zip");
  expect(downloadEvents).toHaveLength(1);
});

test("PC01 failed transports never download; retry and 401 return require explicit action", async ({ page }) => {
  await open(page, `/items/${itemId}/releases/${oldRelease}`);
  const route = `**/api/v1/releases/${oldRelease}/export`;
  const control = page.getByTestId(`release-download-${oldRelease}`);
  const downloads: string[] = [];
  page.on("download", (entry) => downloads.push(entry.suggestedFilename()));
  for (const mode of ["network", "403", "500", "json", "html", "fake-zip"]) {
    await page.route(route, async (entry) => {
      if (mode === "network") { await entry.abort("failed"); return; }
      const status = mode === "403" || mode === "500" ? Number(mode) : 200;
      await entry.fulfill({ status, headers: {
        "content-type": mode === "html" ? "text/html" : mode === "fake-zip" ? "application/zip" : "application/json",
        "content-disposition": 'attachment; filename="error.zip"', "x-request-id": "pc01-test-request",
      }, body: mode === "html" ? "<html>/private/server-details</html>" : '{"error":"/private/server-details"}' });
    });
    await control.getByRole("button").click();
    await expect(control.getByRole("alert")).toBeVisible();
    await expect(control.getByRole("alert")).not.toContainText("/private");
    await expect(control.getByRole("button")).toHaveText("重试下载");
    await page.unroute(route);
  }
  expect(downloads).toEqual([]);
  let unauthorizedCount = 0;
  await page.route(route, async (entry) => {
    unauthorizedCount += 1;
    await entry.fulfill({ status: 401, contentType: "application/json", body: JSON.stringify({ error: { code: "UNAUTHORIZED", message: "需要登录", requestId: "pc01-expired", details: null } }) });
  });
  await control.getByRole("button").click();
  await expect(page).toHaveURL(new RegExp(`/login\\?next=`));
  expect(new URL(page.url()).searchParams.get("next")).toBe(`/items/${itemId}/releases/${oldRelease}`);
  await page.unroute(route);
  await page.getByLabel("密码").fill(BACKEND_PASSWORD);
  await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(control.getByRole("button")).toHaveText("下载说明书资料包");
  expect(unauthorizedCount).toBe(1); expect(downloads).toEqual([]);
  await download(page, oldRelease, "after-session-return.zip");
});

test("PC01 empty list and unavailable reader expose no download action", async ({ page }) => {
  const item = await api.post(`${backend.base}/api/v1/items`, {
    headers: { "x-csrf-token": csrf }, data: { name: "PC01 no releases", model: "empty" },
  });
  expect(item.status()).toBe(201);
  const emptyId = (await item.json()).data.id;
  await open(page, `/items/${emptyId}/releases`);
  await expect(page.getByText("该物品还没有发布版本", { exact: false })).toBeVisible();
  await expect(page.getByRole("button", { name: /下载/ })).toHaveCount(0);
  await page.goto(`${web}/items/${itemId}/releases/01930000-0000-7000-8000-000000000000`);
  await expect(page.getByRole("alert")).toContainText("发布版本读取失败");
  await expect(page.getByRole("button", { name: /下载/ })).toHaveCount(0);
});
