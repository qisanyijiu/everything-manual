import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { expect, request, test, type APIRequestContext, type Page } from "@playwright/test";
import { apiLogin, loginViaUi } from "./helpers";
import { BACKEND_PASSWORD, LocalFixture, TestBackend } from "./job-recovery-harness";
import { E2E_WEB_PORT, REPO_ROOT, serverBinary } from "./runtime";
import { installRealBackendRouting } from "./viewer-harness";

test.describe.configure({ mode: "serial", timeout: 120_000 });
const web = `http://127.0.0.1:${E2E_WEB_PORT}`;
const evidenceDir = path.join(REPO_ROOT, "artifacts/prd-completion/pc02a-rd");
let backend: TestBackend;
let fixture: LocalFixture;
let api: APIRequestContext;
let itemId = "";
const documents: { id: string; title: string; sourceAssetId: string }[] = [];
const missing = "01930000-0000-7000-8000-000000000099";

// Original, deterministic two-page fixture; no third-party manual is copied.
function pdf(label: string): Buffer {
  const objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>"];
  for (let page = 1; page <= 2; page++) {
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 480 600] /Resources << /Font << /F1 7 0 R >> >> /Contents ${page * 2 + 2} 0 R >>`);
    const stream = `BT /F1 28 Tf 40 510 Td (${label} PAGE ${page}) Tj ET\n`;
    objects.push(`<< /Length ${stream.length} >>\nstream\n${stream}endstream`);
  }
  objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
  let content = "%PDF-1.4\n";
  const offsets = [0];
  objects.forEach((object, index) => { offsets.push(content.length); content += `${index + 1} 0 obj\n${object}\nendobj\n`; });
  const xref = content.length;
  content += `xref\n0 8\n0000000000 65535 f \n${offsets.slice(1).map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("")}trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(content);
}
function counts() {
  return JSON.parse(execFileSync("python3", ["-c", "import sqlite3,json,sys; c=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True); print(json.dumps({t:c.execute('SELECT count(*) FROM '+t).fetchone()[0] for t in ['preparations','jobs','provider_attempts','cost_ledger']}))", path.join(backend.dataDir, "manual.sqlite3")], { encoding: "utf8" })) as Record<string, number>;
}

test.beforeAll(async () => {
  fs.mkdirSync(evidenceDir, { recursive: true });
  fixture = new LocalFixture(); await fixture.start();
  backend = new TestBackend("pc02a-original", process.env.EM_PC02A_BINARY ?? serverBinary());
  await backend.start(fixture);
  await backend.stop();
  // Unconfigured providers are an explicit condition of AC-PC2-001.
  fs.writeFileSync(path.join(backend.workDir, "config.toml"), `public_origin = "${web}"\n[providers.tripo]\napi_key_env = "PC02A_UNUSED_TRIPO_KEY"\n[providers.manual_ai]\napi_key_env = "PC02A_UNUSED_LLM_KEY"\nmodel = "fixture-model"\n`);
  await backend.start(fixture);
  api = await request.newContext();
  const csrf = await apiLogin(api, backend.base, BACKEND_PASSWORD);
  const item = await api.post(`${backend.base}/api/v1/items`, { headers: { "x-csrf-token": csrf }, data: { name: "PC02A 原件阅读", model: "Original fixture", brand: "Fixture" } });
  expect(item.status()).toBe(201); itemId = (await item.json()).data.id;
  for (const label of ["ALPHA", "BRAVO"]) {
    const asset = await api.post(`${backend.base}/api/v1/items/${itemId}/assets`, { headers: { "x-csrf-token": csrf }, multipart: { purpose: "document", file: { name: `${label}.pdf`, mimeType: "application/pdf", buffer: pdf(label) } } });
    expect(asset.status()).toBe(201);
    const sourceAssetId = (await asset.json()).data.id as string;
    const response = await api.post(`${backend.base}/api/v1/items/${itemId}/documents`, { headers: { "x-csrf-token": csrf }, data: { sourceAssetId, title: `${label} 同产品原件`, sourceUrl: "https://example.invalid/original-fixture" } });
    expect(response.status()).toBe(201);
    documents.push({ ...(await response.json()).data, sourceAssetId });
  }
  // Move both interesting references beyond the server's default 20-row page.
  for (let index = 0; index < 20; index++) {
    const response = await api.post(`${backend.base}/api/v1/items/${itemId}/documents`, {
      headers: { "x-csrf-token": csrf }, data: { sourceAssetId: documents[0]?.sourceAssetId, title: `其他原件 ${index}` },
    });
    expect(response.status()).toBe(201);
  }
});
test.afterAll(async () => { await api?.dispose(); await backend?.cleanup(fixture); });
async function open(page: Page, route: string) {
  const routing = await installRealBackendRouting(page, backend.base);
  await loginViaUi(page, web, BACKEND_PASSWORD);
  await page.goto(web + route);
  return routing;
}
async function expectPage(page: Page, label: string, number: number) {
  await expect(page.getByTestId("original-page-label")).toHaveText(`第 ${number} / 2 页`);
  await expect(page.getByTestId("original-canvas")).toBeVisible();
  const text = page.getByTestId("original-text");
  if ((await text.getAttribute("open")) === null) await text.locator("summary").click();
  await expect(text.locator("pre")).toHaveText(`${label} PAGE ${number}`);
}

test("PC2-001/003: real item entries without providers/preparation, keyboard paging and honest invalid URL", async ({ page }) => {
  const before = counts();
  const routing = await open(page, `/items/${itemId}`);
  await expect(page.getByRole("link", { name: /^查看原件 ·/ })).toHaveCount(22);
  for (const document of documents) {
    const link = page.getByRole("link", { name: `查看原件 · ${document.title}`, exact: true });
    await link.focus(); await link.press("Enter");
    await expect(page.getByRole("heading", { name: document.title, exact: true })).toBeVisible();
    await expectPage(page, document.title.split(" ")[0] ?? "", 1);
    await page.getByLabel("页码", { exact: true }).fill("2");
    await page.getByLabel("页码", { exact: true }).press("Enter");
    await expect(page).toHaveURL(/page=2$/);
    await expectPage(page, document.title.split(" ")[0] ?? "", 2);
    for (const invalid of ["0", "1.5", "3", "abc"]) {
      await page.getByLabel("页码", { exact: true }).fill(invalid);
      await page.getByRole("button", { name: "跳转", exact: true }).click();
      await expect(page.getByRole("alert")).toContainText("请输入 1 至 2 的整数页码");
      await expect(page.getByLabel("页码", { exact: true })).toBeFocused();
      await expect(page).toHaveURL(/page=2$/);
      await expect(page.getByTestId("original-page-label")).toHaveText("第 2 / 2 页");
      await expect(page.getByTestId("original-text").locator("pre")).toHaveText(`${document.title.split(" ")[0]} PAGE 2`);
    }
    await page.getByRole("button", { name: "上一页", exact: true }).click();
    await expectPage(page, document.title.split(" ")[0] ?? "", 1);
    await page.getByRole("link", { name: "返回物品资料", exact: true }).last().click();
    await expect(page.getByRole("link", { name: `查看原件 · ${document.title}`, exact: true })).toBeFocused();
  }
  const first = documents[0]; if (!first) throw new Error("fixture missing");
  await page.goto(`${web}/items/${itemId}/documents/${first.id}?page=999`);
  await expect(page.getByRole("alert")).toContainText("此页码不可用");
  await expect(page.getByTestId("original-canvas")).toBeHidden();
  await page.getByLabel("页码", { exact: true }).fill("1"); await page.getByLabel("页码", { exact: true }).press("Enter");
  await expectPage(page, "ALPHA", 1);
  expect(counts()).toEqual(before); expect(Object.values(before).every((value) => value === 0)).toBe(true);
  expect(fixture.counts).toEqual({ upload: 0, submit: 0, task: 0, manual: 0, cdn: 0 }); expect(routing.external).toEqual([]);
  fs.writeFileSync(path.join(evidenceDir, "original-counts.json"), JSON.stringify({ before, after: counts(), providerCalls: fixture.counts }));
});

// Metadata-only synthetic references exercise a future/malformed multi-document manifest.
// Asset bytes, authentication, item/doc rows and failures continue to the real isolated backend.
async function references(page: Page, release: boolean) {
  const first = documents[0], second = documents[1]; if (!first || !second) throw new Error("fixture missing");
  const evidence = (id: string, pageNumber = 2) => ({ documentId: id, pageNumber, quote: "fixture reference" });
  const knowledge = { model: { assetId: missing, revisionId: missing, sha256: "a".repeat(64), validationState: "validated" }, knowledge: {
    parts: [{ id: "part-alpha", name: "甲部件", description: "独立文档引用", evidence: [evidence(first.id), evidence(missing), evidence(first.id, 99)] }],
    steps: [{ id: "step-bravo", title: "乙步骤", orderedActions: ["核对第二份说明书"], partIds: ["part-alpha"], safetyNotes: [], evidence: [evidence(second.id)] }], specs: [],
  }, hotspots: [], stepPoses: {} };
  const route = `/items/${itemId}/${release ? "releases/pc02a-release" : "drafts/pc02a-draft/review"}`;
  const endpoint = `**/api/v1/items/${itemId}/${release ? "releases/pc02a-release" : "drafts/pc02a-draft"}`;
  await page.route(endpoint, async (entry) => entry.fulfill({ status: 200, contentType: "application/json", headers: { etag: '"1"' }, body: JSON.stringify({ data: release ? {
    id: "pc02a-release", itemId, draftRevision: 1, modelRevisionId: missing, manifestSha256: "a".repeat(64),
    manifest: { knowledge, review: {}, documents: documents.map((doc) => ({ ...doc, documentId: doc.id })) },
  } : { id: "pc02a-draft", itemId, revision: 1, status: "needs_review", completeness: "complete", knowledge, review: {} } }) }));
  return route;
}
async function showPanel(page: Page, width: number, panel: "parts" | "steps") {
  if (width < 768) await page.getByRole("button", { name: panel === "parts" ? /^(部件|部件与热点)$/ : "步骤与原文", exact: true }).click();
  else if (width < 1280) {
    const toggle = page.getByRole("button", { name: /^(显示|隐藏)步骤与原文$/ });
    await expect(toggle).toBeVisible();
    if ((await toggle.textContent())?.startsWith("显示")) await toggle.click();
    await page.getByRole("tab", { name: panel === "parts" ? /^(部件|部件与热点)$/ : "步骤与原文", exact: true }).click();
  }
}

test("PC2-002/003: part and step references own their document, focus and a single drawer at 375/1024/1440", async ({ page }) => {
  const routing = await installRealBackendRouting(page, backend.base);
  await loginViaUi(page, web, BACKEND_PASSWORD);
  const results = [];
  const images = new Map<string, string>();
  for (const release of [false, true]) {
    const route = await references(page, release);
    for (const width of [375, 1024, 1440]) {
      await page.setViewportSize({ width, height: 900 }); await page.goto(web + route);
      for (const panel of ["parts", "steps"] as const) {
        await showPanel(page, width, panel);
        const source = page.locator(panel === "parts" ? "#evidence-part-alpha-0" : "#evidence-step-bravo-0");
        expect((await source.boundingBox())?.height).toBeGreaterThanOrEqual(44);
        await source.focus(); await source.press("Enter");
        await expect(page.locator("#original-heading")).toBeFocused();
        await expectPage(page, panel === "parts" ? "ALPHA" : "BRAVO", 2);
        const pixels = await page.getByTestId("original-canvas").evaluate((canvas: HTMLCanvasElement) => canvas.toDataURL());
        const digest = createHash("sha256").update(pixels).digest("hex");
        if (panel === "parts") images.set(`${release}-${width}`, digest);
        else expect(digest).not.toBe(images.get(`${release}-${width}`));
        if (width === 375 && panel === "steps") await page.screenshot({ path: path.join(evidenceDir, `${release ? "release" : "draft"}-375.png`) });
        if (width < 768) await expect(page.getByRole("dialog")).toHaveCount(1);
        if (width >= 768 && width < 1280) await expect(page.getByRole("tab", { name: "原文", exact: true })).toHaveAttribute("aria-selected", "true");
        const button = page.getByRole("button", { name: "返回出处", exact: true });
        expect((await button.boundingBox())?.height).toBeGreaterThanOrEqual(44);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        await button.click(); await expect(source).toBeFocused();
        if (width < 768) { await page.keyboard.press("Escape"); await expect(page.getByRole("dialog")).toHaveCount(0); }
      }
      results.push({ release, width, part: "ALPHA PAGE 2", step: "BRAVO PAGE 2", focus: true });
    }
    await showPanel(page, 1440, "parts");
    await page.locator("#evidence-part-alpha-1").click();
    await expect(page.locator(".original-section").getByRole("alert")).toContainText("此出处的原件不可用");
    await expect(page.getByTestId("original-canvas")).toHaveCount(0);
    await page.getByRole("button", { name: "返回出处", exact: true }).click();
    await page.locator("#evidence-part-alpha-2").click();
    await expect(page.locator(".original-section").getByRole("alert")).toContainText("此出处页码超出原件范围");
    await expect(page.getByTestId("original-canvas")).toBeHidden();
    await page.getByLabel("原件", { exact: true }).selectOption(documents[1]?.id ?? ""); await expectPage(page, "BRAVO", 1);
    await page.getByLabel("页码", { exact: true }).fill("2"); await page.getByLabel("页码", { exact: true }).press("Enter");
    for (const width of [1024, 375, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      await expect(page.locator(`.page-layout--${width < 768 ? "narrow" : width < 1280 ? "mid" : "wide"}`)).toBeVisible();
      await expectPage(page, "BRAVO", 2);
      await expect(page.locator("#original-heading")).toContainText("BRAVO");
    }
    await page.screenshot({ path: path.join(evidenceDir, `${release ? "release" : "draft"}-1440.png`), fullPage: true });
  }
  expect(routing.external).toEqual([]); expect(fixture.paidSubmissions()).toBe(0);
  fs.writeFileSync(path.join(evidenceDir, "navigation-summary.json"), JSON.stringify(results, null, 2));
});

test("PC2-004: local 404 retry, WebGL failure and keyboard PDF reading remain independent", async ({ page }) => {
  await page.addInitScript(() => {
    const getContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, type: string, ...args: unknown[]) {
      if (type.includes("webgl")) return null;
      return Reflect.apply(getContext, this, [type, ...args]);
    } as typeof getContext;
  });
  const routing = await installRealBackendRouting(page, backend.base);
  await loginViaUi(page, web, BACKEND_PASSWORD);
  const route = await references(page, true);
  const first = documents[0]; if (!first) throw new Error("fixture missing");
  const assetRoute = `**/api/v1/assets/${first.sourceAssetId}/content`;
  await page.route(assetRoute, (entry) => entry.continue({ url: `${backend.base}/api/v1/assets/${missing}/content` }));
  await page.setViewportSize({ width: 375, height: 900 }); await page.goto(web + route);
  await expect(page.getByText(/浏览器 3D 上下文不可用/)).toBeVisible();
  await showPanel(page, 375, "parts"); await page.locator("#evidence-part-alpha-0").click();
  await expect(page.getByTestId("original-error")).toContainText("诊断请求 ID");
  await page.unroute(assetRoute);
  const retry = page.getByRole("button", { name: "重新加载原文", exact: true }); await retry.focus(); await retry.press("Enter");
  await expectPage(page, "ALPHA", 2);
  await page.getByRole("button", { name: "上一页", exact: true }).focus(); await page.keyboard.press("Enter"); await expectPage(page, "ALPHA", 1);
  await page.keyboard.press("Escape"); await expect(page.locator("#evidence-part-alpha-0")).toBeFocused();
  await expect(page.getByRole("dialog")).toHaveCount(1);
  await page.keyboard.press("Escape"); await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(routing.external).toEqual([]); expect(fixture.counts).toEqual({ upload: 0, submit: 0, task: 0, manual: 0, cdn: 0 });
});
