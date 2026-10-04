/** Real Chrome/manual/provider delivery check. Re-running resumes the recorded job.
 * Requires a dedicated initialized instance. No key/cookie logging, replacement purchase,
 * automatic retry, bulk knowledge approval, or public deployment.
 * prepare: upload original material and freeze a quote; submit: one explicitly allowed job;
 * inspect: read the existing job/draft. Review and publication are separate audited steps.
 */
/* global document */
import fs from "node:fs";
import path from "node:path";
import { chromium, expect } from "@playwright/test";

const mode = process.argv[2] ?? "inspect";
if (!["prepare", "submit", "inspect"].includes(mode)) throw new Error("Unknown mode");
const out = path.resolve(process.env.EM_LIVE_EVIDENCE_DIR ?? "../../var/delivery-20261004/nikon-live");
const base = process.env.EM_LIVE_ORIGIN ?? "http://127.0.0.1:18082";
const url = new URL(base);
if (url.protocol !== "http:" || url.hostname !== "127.0.0.1") throw new Error("Dedicated loopback instance required");
const passwordFile = process.env.EM_LIVE_PASSWORD_FILE;
if (!passwordFile || (fs.statSync(passwordFile).mode & 0o077) !== 0) throw new Error("Private password file required");
const password = fs.readFileSync(passwordFile, "utf8").trim();
fs.mkdirSync(out, { recursive: true });
const stateFile = path.join(out, "state.json");
const state = fs.existsSync(stateFile) ? JSON.parse(fs.readFileSync(stateFile, "utf8")) : { origin: base, pageErrors: [] };
if (state.origin !== base) throw new Error("Evidence belongs to a different instance");
const save = () => fs.writeFileSync(stateFile, JSON.stringify(state, null, 2), { mode: 0o600 });
const browser = await chromium.launch({ channel: "chrome", headless: true,
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
page.on("pageerror", error => { state.pageErrors.push(error.message); save(); });
page.on("dialog", dialog => dialog.dismiss());
page.setDefaultTimeout(60_000);
const log = value => console.log(`${new Date().toISOString()} ${value}`);
async function api(method, endpoint, body, headers = {}) {
  return page.evaluate(async ({ method, endpoint, body, headers }) => {
    const session = await (await fetch("/api/v1/auth/session")).json();
    const response = await fetch(`/api/v1${endpoint}`, { method,
      headers: { "content-type": "application/json", "x-csrf-token": session.data.csrfToken, ...headers },
      body: body === undefined ? undefined : JSON.stringify(body) });
    const text = await response.text();
    return { status: response.status, etag: response.headers.get("etag"), body: text ? JSON.parse(text) : null };
  }, { method, endpoint, body, headers });
}
async function login() {
  await page.goto(`${base}/login`);
  await page.locator('input[type="password"]').fill(password);
  await page.keyboard.press("Enter");
  await page.waitForURL(u => !u.pathname.startsWith("/login"));
}
async function quote() {
  await page.goto(`${base}/items/${state.itemId}/import/confirm${state.quote?.id ? `?quoteId=${state.quote.id}` : ""}`);
  await expect(page.getByTestId("quote-panel")).toBeVisible();
  if (await page.getByTestId("quote-expiry").textContent().then(text => text?.includes("已过期"))) {
    await page.getByTestId("requote-button").click();
    await expect(page.getByTestId("quote-expiry")).not.toContainText("已过期");
  }
  await page.waitForURL(u => u.searchParams.has("quoteId"));
  const quoteId = new URL(page.url()).searchParams.get("quoteId");
  const r = await api("GET", `/items/${state.itemId}/estimates/${quoteId}`);
  if (r.status !== 200) throw new Error(`Quote read rejected ${r.status}`);
  state.quote = r.body.data; save();
  await expect(page.locator("#send-scope-confirm")).not.toBeChecked();
  await page.screenshot({ path: path.join(out, "quote.png"), fullPage: true });
}
async function prepare() {
  const pdf = process.env.EM_LIVE_PDF;
  const front = process.env.EM_LIVE_FRONT;
  const second = process.env.EM_LIVE_SECOND;
  const secondView = process.env.EM_LIVE_SECOND_VIEW ?? "back";
  if (!pdf || !front || !second || !["left", "back", "right"].includes(secondView)) throw new Error("Verified original PDF/front/side-or-back required");
  if (!state.itemId) {
    // Persist before creation; an unknown response is checked, never blindly repeated.
    if (state.creationStarted) throw new Error("Creation outcome unknown; inspect the item list before continuing");
    await page.goto(`${base}/items/new`);
    await page.getByRole("textbox", { name: /^名称/ }).fill("Nikon F3HP 原版说明书验收");
    await page.getByRole("textbox", { name: /^准确型号/ }).fill("Nikon F3 High-Eyepoint (F3HP / DE-3)");
    await page.getByLabel("品牌", { exact: true }).fill("Nikon");
    await page.getByLabel("变体/配置", { exact: true }).fill("DE-3 高眼点取景器；50mm 标准镜头参考图");
    const created = page.waitForResponse(r => r.request().method() === "POST" && new URL(r.url()).pathname === "/api/v1/items");
    state.creationStarted = true; save();
    await page.locator('form button[type="submit"]').click();
    const r = await created;
    if (!r.ok()) throw new Error(`Creation rejected ${r.status()}`);
    state.itemId = (await r.json()).data.id; save();
    await page.waitForURL(u => u.pathname === `/items/${state.itemId}/import/document`);
    log(`Created item ${state.itemId}`);
  }
  const documents = await api("GET", `/items/${state.itemId}/documents`);
  if (documents.body.data.length === 0) {
    await page.goto(`${base}/items/${state.itemId}/import/document`);
    await page.getByTestId("document-upload").locator('input[type="file"]').setInputFiles(pdf);
    await page.getByLabel("标题（可选）").fill("Nikon F3 High-Eyepoint 原版说明书（47页）");
    await page.getByLabel("出处链接（可选）").fill("https://www.pacificrimcamera.com/rl/01328/01328.pdf");
    const bound = page.waitForResponse(r => r.request().method() === "POST" && new URL(r.url()).pathname.endsWith("/documents"));
    await page.getByRole("button", { name: "绑定为说明书", exact: true }).click();
    const r = await bound;
    if (!r.ok()) throw new Error(`Document rejected ${r.status()}`);
    state.documentId = (await r.json()).data.id; save();
    await expect(page.getByRole("button", { name: "绑定为说明书", exact: true })).toHaveCount(0);
  } else { state.documentId = documents.body.data[0].id; save(); }
  await page.goto(`${base}/items/${state.itemId}/import/views`);
  await page.locator("summary").filter({ hasText: "逐个视图上传 / 替换照片" }).click();
  const photos = (await api("GET", `/items/${state.itemId}/photos`)).body.data;
  for (const [view, file] of [["front", front], [secondView, second]]) {
    if (photos.some(p => p.view === view)) continue;
    const registered = page.waitForResponse(r => r.request().method() === "POST" && new URL(r.url()).pathname.endsWith("/photos"));
    await page.getByTestId(`photo-upload-${view}`).locator('input[type="file"]').setInputFiles(file);
    const r = await registered;
    if (!r.ok()) throw new Error(`Photo rejected ${r.status()}`);
    await expect(page.getByTestId(`photo-replace-${view}`).locator('input[type="file"]')).toBeEnabled();
  }
  state.views = (await api("GET", `/items/${state.itemId}/photos`)).body.data.map(p => ({ id: p.id, view: p.view, assetId: p.assetId })); save();
  await page.screenshot({ path: path.join(out, "views.png"), fullPage: true });
  await page.goto(`${base}/items/${state.itemId}/import/prepare`);
  await page.waitForFunction(() => document.querySelector('[data-testid="prepare-start"]') !== null || /已就绪|准备完成 ·/.test(document.body.innerText));
  if (await page.getByTestId("prepare-start").isVisible()) await page.getByTestId("prepare-start").click();
  await page.waitForFunction(() => /封存资料|已就绪|准备完成 ·/.test(document.body.innerText), null, { timeout: 900_000 });
  if (await page.getByTestId("prepare-seal").isVisible()) await page.getByTestId("prepare-seal").click();
  await page.waitForFunction(() => /已就绪|准备完成 ·/.test(document.body.innerText));
  await page.screenshot({ path: path.join(out, "prepared.png"), fullPage: true });
  await quote();
  log(`Prepared ${state.quote.pageCount} pages; Tripo ${state.quote.amounts.tripo.upperBoundDisplay}; LLM ${state.quote.amounts.manualAi.upperBoundDisplay}`);
}
async function submit() {
  if (state.jobId) { log(`Resuming job ${state.jobId}`); return; }
  if (state.submissionStarted) throw new Error("Submission outcome unknown; inspect original quote/job before any new purchase");
  if (process.env.EM_LIVE_GENERATION_ALLOWED !== "1") throw new Error("Explicit live generation authorization required");
  await quote();
  if (state.quote.amounts.tripo.upperBoundMinor > 7000 || state.quote.amounts.manualAi.upperBoundMinor !== 0) throw new Error("Outside this run's70credits / $0 plan");
  await page.locator("#send-scope-confirm").check();
  await expect(page.getByTestId("confirmed-at")).toBeVisible();
  const accepted = page.waitForResponse(r => r.request().method() === "POST" && new URL(r.url()).pathname === `/api/v1/items/${state.itemId}/jobs`);
  state.submissionStarted = true; save();
  await page.getByTestId("generate-button").click();
  const r = await accepted;
  const data = await r.json();
  if (!r.ok()) throw new Error(`Job rejected ${data.error?.code ?? r.status()}; inspect before resubmission`);
  state.jobId = data.data.id; save();
  log(`Accepted job ${state.jobId}`);
}
async function inspect() {
  if (!state.jobId) throw new Error("No recorded job");
  const r = await api("GET", `/jobs/${state.jobId}`);
  if (r.status !== 200) throw new Error(`Job read ${r.status}`);
  state.job = r.body.data; save();
  log(`Job ${state.job.status}: ${state.job.stages.map(s => `${s.stageKind}#${s.batchIndex}:${s.status}`).join(", ")}`);
  await page.goto(`${base}/jobs/${state.jobId}${state.job.status === "succeeded" ? "/result" : ""}`);
  if (state.job.status === "succeeded") {
    await expect(page.getByTestId("viewer-status")).toContainText("模型已加载", { timeout: 180_000 });
    const d = await api("GET", `/items/${state.itemId}/drafts/${state.job.draftId}`);
    state.draftId = state.job.draftId; save();
    fs.writeFileSync(path.join(out, "draft.json"), JSON.stringify(d.body.data, null, 2), { mode: 0o600 });
  }
  await page.screenshot({ path: path.join(out, "latest-result.png"), fullPage: true });
}
try {
  await login();
  log("Chrome login succeeded");
  delete state.lastError;
  if (mode === "prepare") await prepare();
  if (mode === "submit") await submit();
  if (mode === "inspect" || (mode === "submit" && state.jobId)) await inspect();
} catch (error) {
  state.lastError = String(error.message).slice(0, 800); save();
  await page.screenshot({ path: path.join(out, "failure.png"), fullPage: true }).catch(() => {});
  console.error(state.lastError); process.exitCode = 1;
} finally { save(); await browser.close(); }
