/** Verify an already published Nikon release using installed Chrome and actual asset bytes.
 * Run from any directory:
 * EM_LIVE_PASSWORD_FILE=<private file> node apps/web/scripts/nikon-export-verification.mjs <releaseId>
 * Optional: EM_LIVE_ORIGIN, EM_LIVE_STATE_FILE, EM_EXPORT_EVIDENCE_DIR.
 * This script cannot approve knowledge, publish, retry a job, or purchase provider work.
 * Re-running only repeats read/export checks; each attempt has its own evidence directory.
 */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { inflateRawSync } from "node:zlib";
import { chromium, expect } from "@playwright/test";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const liveStateFile = path.resolve(process.env.EM_LIVE_STATE_FILE
  ?? path.join(root, "var/delivery-20261004/nikon-live/state.json"));
const live = JSON.parse(fs.readFileSync(liveStateFile, "utf8"));
const releaseId = process.argv[2] ?? process.env.EM_LIVE_RELEASE_ID;
assert.match(releaseId ?? "", /^[0-9a-f-]{36}$/i, "Explicit already published releaseId required");
assert.match(live.itemId ?? "", /^[0-9a-f-]{36}$/i, "Recorded Nikon item required");
const base = process.env.EM_LIVE_ORIGIN ?? live.origin;
const origin = new URL(base);
assert.equal(origin.protocol, "http:", "Dedicated loopback instance required");
assert.equal(origin.hostname, "127.0.0.1", "Dedicated loopback instance required");
assert.equal(origin.origin, live.origin, "Evidence belongs to a different instance");
const passwordFile = process.env.EM_LIVE_PASSWORD_FILE;
assert.ok(passwordFile, "EM_LIVE_PASSWORD_FILE must name a private password file");
assert.equal(fs.statSync(passwordFile).mode & 0o077, 0, "Password file must be owner-only");
const password = fs.readFileSync(passwordFile, "utf8").trim();
assert.ok(password, "Empty password file");
const out = path.resolve(process.env.EM_EXPORT_EVIDENCE_DIR
  ?? path.join(root, "var/delivery-20261004/nikon-live/export-verification", releaseId));
const runId = new Date().toISOString().replaceAll(/[:.]/g, "-");
const run = path.join(out, "runs", runId);
fs.mkdirSync(run, { recursive: true, mode: 0o700 });
const state = {
  schemaVersion: "nikon_export_verification_v1", origin: origin.origin,
  itemId: live.itemId, jobId: live.jobId ?? null, releaseId,
  runId, runDirectory: run, status: "running", startedAt: new Date().toISOString(),
  policy: "Chrome only; existing published release; auth/login POST and local GET only; no provider calls, approval, publication or retry",
  browser: null, stages: {}, externalRequests: [], blockedWrites: [], onlineErrors: [],
};
const writeJson = (name, data) => fs.writeFileSync(name, `${JSON.stringify(data, null, 2)}\n`, { mode: 0o600 });
const save = () => {
  writeJson(path.join(run, "verification.json"), state);
  const next = path.join(out, "state.json.next");
  writeJson(next, state);
  fs.renameSync(next, path.join(out, "state.json"));
};
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const scrub = (text) => String(text).replaceAll(password, "[redacted]")
  .replaceAll(/(?:Bearer\s+|(?:api[-_]?key|csrfToken|session)[=:"\s]+)[^\s"<>,]+/gi, "[redacted]")
  .slice(0, 2000);
const shortUrl = (url) => { const parsed = new URL(url); return `${parsed.origin}${parsed.pathname}`; };
save();

/** Parse bounded ZIP data without extracting paths to disk. Check CRC and central headers. */
function readZip(bytes) {
  assert.ok(bytes.length <= 512 * 1024 * 1024, "ZIP exceeds verification memory limit");
  let eocd = -1;
  for (let i = bytes.length - 22; i >= Math.max(0, bytes.length - 65557); i--) {
    if (bytes.readUInt32LE(i) === 0x06054b50 && i + 22 + bytes.readUInt16LE(i + 20) === bytes.length) { eocd = i; break; }
  }
  assert.ok(eocd >= 0, "Missing ZIP end record");
  assert.equal(bytes.readUInt16LE(eocd + 4), 0, "Multi-disk ZIP unsupported");
  assert.equal(bytes.readUInt16LE(eocd + 6), 0, "Multi-disk ZIP unsupported");
  const count = bytes.readUInt16LE(eocd + 10);
  assert.equal(bytes.readUInt16LE(eocd + 8), count);
  assert.ok(count > 0 && count <= 1000, "Unexpected ZIP entry count");
  const centralEnd = bytes.readUInt32LE(eocd + 16) + bytes.readUInt32LE(eocd + 12);
  assert.equal(centralEnd, eocd, "Unexpected ZIP central directory extent");
  let offset = bytes.readUInt32LE(eocd + 16);
  let expanded = 0;
  const entries = new Map();
  for (let i = 0; i < count; i++) {
    assert.equal(bytes.readUInt32LE(offset), 0x02014b50, "Invalid ZIP central header");
    const flags = bytes.readUInt16LE(offset + 8);
    assert.equal(flags & 1, 0, "Encrypted ZIP unsupported");
    const method = bytes.readUInt16LE(offset + 10);
    const crc = bytes.readUInt32LE(offset + 16);
    const compressedSize = bytes.readUInt32LE(offset + 20);
    const size = bytes.readUInt32LE(offset + 24);
    const nameSize = bytes.readUInt16LE(offset + 28);
    const extraSize = bytes.readUInt16LE(offset + 30);
    const commentSize = bytes.readUInt16LE(offset + 32);
    const local = bytes.readUInt32LE(offset + 42);
    const name = bytes.subarray(offset + 46, offset + 46 + nameSize).toString("utf8");
    assert.ok(!name.includes("\\") && !name.startsWith("/") && !name.includes(":"), "Unsafe ZIP path");
    assert.ok(name.split("/").every((part) => part && part !== "." && part !== ".."), "Unsafe ZIP path");
    assert.ok(!entries.has(name), "Duplicate ZIP path");
    expanded += size;
    assert.ok(expanded <= 512 * 1024 * 1024, "Expanded ZIP exceeds limit");
    assert.equal(bytes.readUInt32LE(local), 0x04034b50, "Invalid ZIP local header");
    assert.equal(bytes.readUInt16LE(local + 8), method);
    const localNameSize = bytes.readUInt16LE(local + 26);
    assert.equal(bytes.subarray(local + 30, local + 30 + localNameSize).toString("utf8"), name);
    const start = local + 30 + localNameSize + bytes.readUInt16LE(local + 28);
    assert.ok(start + compressedSize <= bytes.readUInt32LE(eocd + 16), "ZIP entry overlaps directory");
    const compressed = bytes.subarray(start, start + compressedSize);
    assert.ok(method === 0 || method === 8, "Unsupported ZIP compression");
    const body = method === 0 ? compressed : inflateRawSync(compressed, { maxOutputLength: Math.max(1, size) });
    assert.equal(body.length, size, "ZIP size mismatch");
    let actualCrc = 0xffffffff;
    for (const value of body) {
      actualCrc ^= value;
      for (let bit = 0; bit < 8; bit++) actualCrc = (actualCrc >>> 1) ^ ((actualCrc & 1) ? 0xedb88320 : 0);
    }
    assert.equal((actualCrc ^ 0xffffffff) >>> 0, crc, "ZIP CRC mismatch");
    entries.set(name, body);
    offset += 46 + nameSize + extraSize + commentSize;
  }
  assert.equal(offset, centralEnd);
  return entries;
}

/** Independent expected text from the immutable original facts and frozen user edit layer. */
function reviewedFacts(manifest) {
  const facts = manifest.knowledge.knowledge;
  const entities = manifest.review?.entities ?? {};
  const evidence = (entries) => (entries ?? []).map((entry) => ({ pageNumber: entry.pageNumber, quote: entry.quote ?? null }));
  return {
    parts: facts.parts.map((part) => {
      const edit = entities[part.id]?.userEdited ?? {};
      return { id: part.id, name: edit.name ?? part.name, description: edit.description ?? part.description ?? "", evidence: evidence(part.evidence) };
    }),
    steps: facts.steps.map((step) => {
      const edit = entities[step.id]?.userEdited ?? {};
      return { id: step.id, title: edit.title ?? step.title, orderedActions: edit.orderedActions ?? step.orderedActions ?? [],
        safetyNotes: edit.safetyNotes ?? step.safetyNotes ?? [], partIds: step.partIds ?? [], evidence: evidence(step.evidence) };
    }),
    specs: facts.specs.map((spec) => {
      const edit = entities[spec.id]?.userEdited ?? {};
      return { id: spec.id, label: edit.label ?? spec.label, value: edit.value ?? spec.value ?? "", evidence: evidence(spec.evidence) };
    }),
  };
}

async function stage(name, operation) {
  state.currentStage = name;
  state.stages[name] = { status: "running", startedAt: new Date().toISOString() }; save();
  const result = await operation();
  state.stages[name] = { ...state.stages[name], status: "passed", finishedAt: new Date().toISOString(), ...result }; save();
  console.log(`${new Date().toISOString()} ${name}: passed`);
}

async function canvasInteraction(page, canvas, reset, prefix) {
  await expect(canvas).toBeVisible();
  await canvas.scrollIntoViewIfNeeded();
  await reset.click();
  const before = await canvas.screenshot({ path: path.join(run, `${prefix}-canvas-before.png`) });
  const box = await canvas.boundingBox();
  assert.ok(box && box.width > 20 && box.height > 20, "Visible canvas dimensions required");
  await page.mouse.move(box.x + box.width * 0.45, box.y + box.height * 0.55);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.68, box.y + box.height * 0.65, { steps: 15 });
  await page.mouse.up();
  await expect.poll(async () => !(await canvas.screenshot()).equals(before), { timeout: 15_000 }).toBe(true);
  const changed = await canvas.screenshot({ path: path.join(run, `${prefix}-canvas-rotated.png`) });
  await reset.click();
  // Reduced motion disables damping; the default view must reproduce the original pixels.
  await expect.poll(async () => (await canvas.screenshot()).equals(before), { timeout: 15_000 }).toBe(true);
  const restored = await canvas.screenshot({ path: path.join(run, `${prefix}-canvas-reset.png`) });
  return { interaction: "pointer rotation", canvasChanged: true, resetRestoredOriginalPixels: true,
    beforeSha256: sha256(before), changedSha256: sha256(changed), restoredSha256: sha256(restored) };
}

let browser;
let context;
let offlineContext;
let page;
let release;
let expected;
let zipEntries;
let htmlPath;
const reader = `${origin.origin}/items/${live.itemId}/releases/${releaseId}`;
try {
  browser = await chromium.launch({ channel: "chrome", headless: true,
    args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });
  state.browser = { channel: "chrome", version: browser.version(), reducedMotion: "reduce" }; save();
  context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce", acceptDownloads: true });
  await context.route("**/*", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (!/^https?:$/.test(url.protocol)) { await route.continue(); return; }
    if (url.origin !== origin.origin) {
      state.externalRequests.push(shortUrl(url.href)); save(); await route.abort(); return;
    }
    if (!["GET", "HEAD"].includes(request.method())
      && !(request.method() === "POST" && url.pathname === "/api/v1/auth/login")) {
      state.blockedWrites.push({ method: request.method(), path: url.pathname }); save(); await route.abort(); return;
    }
    await route.continue();
  });
  page = await context.newPage();
  page.setDefaultTimeout(60_000);
  page.on("pageerror", (error) => { state.onlineErrors.push(scrub(error.message)); save(); });
  page.on("console", (message) => { if (message.type() === "error") { state.onlineErrors.push(scrub(message.text())); save(); } });
  await stage("login", async () => {
    await page.goto(`${origin.origin}/login`);
    await page.locator('input[type="password"]').fill(password);
    await page.keyboard.press("Enter");
    await page.waitForURL((url) => !url.pathname.startsWith("/login"));
    return { authenticated: true };
  });
  const get = async (endpoint) => {
    const response = await context.request.get(`${origin.origin}/api/v1${endpoint}`);
    assert.equal(response.status(), 200, `Read rejected ${response.status()} for ${endpoint}`);
    return response;
  };
  await stage("immutableRelease", async () => {
    release = (await (await get(`/items/${live.itemId}/releases/${releaseId}`)).json()).data;
    assert.equal(release.id, releaseId); assert.equal(release.itemId, live.itemId);
    const raw = await (await get(`/assets/${release.manifestAssetId}/content`)).body();
    assert.equal(sha256(raw), release.manifestSha256, "Frozen manifest fingerprint");
    assert.deepEqual(JSON.parse(raw), release.manifest, "Reader manifest differs from frozen bytes");
    fs.writeFileSync(path.join(run, "release-manifest.json"), raw, { mode: 0o600 });
    expected = reviewedFacts(release.manifest);
    assert.ok(expected.parts.length && expected.steps.length, "Nikon release must contain parts and steps");
    return { manifestSha256: release.manifestSha256, modelRevisionId: release.modelRevisionId,
      parts: expected.parts.length, steps: expected.steps.length, specs: expected.specs.length };
  });
  await stage("onlineReader", async () => {
    await page.goto(reader);
    await expect(page.getByTestId("viewer-status")).toContainText("模型已加载", { timeout: 180_000 });
    const interaction = await canvasInteraction(page, page.getByTestId("viewer-canvas"), page.getByRole("button", { name: "复位视角", exact: true }), "online");
    for (const part of expected.parts) {
      await expect(page.getByTestId(`reader-part-${part.id}`).getByRole("button", { name: part.name, exact: true })).toBeVisible();
    }
    for (const step of expected.steps) {
      const entry = page.getByTestId(`reader-step-${step.id}`);
      await entry.getByRole("button", { name: step.title, exact: true }).click();
      for (const text of [...step.orderedActions, ...step.safetyNotes]) await expect(entry.locator(".step-detail")).toContainText(text);
    }
    for (const spec of expected.specs) await expect(page.getByTestId(`reader-spec-${spec.id}`)).toContainText(`${spec.label}：${spec.value}`);
    await page.screenshot({ path: path.join(run, "online-reader.png"), fullPage: true });
    return { ...interaction, allReviewedTextVisible: true };
  });
  await stage("evidencePdf", async () => {
    const facts = release.manifest.knowledge.knowledge;
    const entities = [...facts.parts.map((entry) => ({ kind: "part", entry })),
      ...facts.steps.map((entry) => ({ kind: "step", entry })), ...facts.specs.map((entry) => ({ kind: "spec", entry }))];
    const source = entities.find(({ entry }) => entry.evidence?.some((evidence) =>
      release.manifest.documents.some((doc) => doc.documentId === evidence.documentId)));
    assert.ok(source, "Published Nikon fact with bound PDF evidence required");
    const index = source.entry.evidence.findIndex((evidence) => release.manifest.documents.some((doc) => doc.documentId === evidence.documentId));
    const evidence = source.entry.evidence[index];
    if (source.kind === "step") await page.getByTestId(`reader-step-${source.entry.id}`).getByRole("button").first().click();
    await page.locator(`[id="evidence-${source.entry.id}-${index}"]`).click();
    await expect(page.getByTestId("original-page-label")).toContainText(`第 ${evidence.pageNumber} /`);
    const canvas = page.getByTestId("original-canvas");
    await expect(canvas).toBeVisible({ timeout: 90_000 });
    assert.equal(await canvas.getAttribute("aria-label"), `原 PDF 第 ${evidence.pageNumber} 页`);
    const pixels = await canvas.evaluate((element) => {
      const ctx = element.getContext("2d");
      const bytes = ctx.getImageData(0, 0, element.width, element.height).data;
      let differs = 0;
      for (let i = 0; i < bytes.length; i += 4) if (bytes[i] !== bytes[0] || bytes[i + 1] !== bytes[1] || bytes[i + 2] !== bytes[2]) differs++;
      return { width: element.width, height: element.height, differs };
    });
    assert.ok(pixels.differs > 100, "PDF canvas must contain real page content");
    await canvas.screenshot({ path: path.join(run, "evidence-pdf-page.png") });
    return { entityId: source.entry.id, documentId: evidence.documentId, pageNumber: evidence.pageNumber, ...pixels };
  });
  await stage("zipExport", async () => {
    const downloadEvent = page.waitForEvent("download", { timeout: 180_000 });
    await page.getByTestId(`release-download-${releaseId}`).getByRole("button", { name: "下载说明书资料包", exact: true }).click();
    const download = await downloadEvent;
    assert.equal(await download.failure(), null, "ZIP download failed");
    const zipPath = path.join(run, "nikon-release.zip");
    await download.saveAs(zipPath); fs.chmodSync(zipPath, 0o600);
    const bytes = fs.readFileSync(zipPath);
    zipEntries = readZip(bytes);
    assert.ok(zipEntries.has("manifest.json") && zipEntries.has("release/manifest.json"));
    const manifest = JSON.parse(zipEntries.get("manifest.json"));
    assert.equal(manifest.schemaVersion, "manual_release_export_v1");
    assert.equal(manifest.item.id, live.itemId); assert.equal(manifest.release.releaseId, releaseId);
    assert.equal(manifest.release.manifestSha256, release.manifestSha256);
    assert.deepEqual(manifest.knowledge, release.manifest.knowledge); assert.deepEqual(manifest.review, release.manifest.review);
    const raw = zipEntries.get("release/manifest.json");
    assert.equal(sha256(raw), release.manifestSha256);
    assert.equal(manifest.releaseManifest.path, "release/manifest.json");
    assert.equal(manifest.releaseManifest.sha256, release.manifestSha256); assert.equal(manifest.releaseManifest.size, raw.length);
    assert.ok(raw.equals(fs.readFileSync(path.join(run, "release-manifest.json"))), "Export frozen bytes differ");
    const records = [];
    const allowed = new Set(["manifest.json", "release/manifest.json"]);
    for (const file of manifest.files) {
      allowed.add(file.path);
      const frozen = release.manifest.assets.find((entry) => entry.assetId === file.assetId && entry.role === file.role);
      assert.ok(frozen, "Unexpected export asset"); assert.equal(file.sha256, frozen.sha256);
      const body = zipEntries.get(file.path); assert.ok(body, "Missing exported asset");
      assert.equal(body.length, file.size); assert.equal(sha256(body), file.sha256, `Asset hash ${file.path}`);
      if (file.role === "document") assert.equal(body.subarray(0, 5).toString(), "%PDF-");
      if (file.role === "model" || file.role === "model_parts") assert.equal(body.subarray(0, 4).toString(), "glTF");
      records.push({ path: file.path, role: file.role, size: file.size, sha256: file.sha256 });
    }
    assert.deepEqual([...zipEntries.keys()].sort(), [...allowed].sort(), "Undeclared ZIP entries");
    assert.equal(manifest.files.length, release.manifest.assets.length, "Export omits frozen assets");
    assert.ok(records.some((file) => file.role === "model") && records.some((file) => file.role === "document"));
    writeJson(path.join(run, "zip-integrity.json"), { zipSha256: sha256(bytes), size: bytes.length, files: records,
      manifestSha256: release.manifestSha256, crcVerified: true, allFrozenAssetsVerified: true });
    return { zipSha256: sha256(bytes), size: bytes.length, files: records.length, crcVerified: true, allFrozenAssetsVerified: true };
  });
  await stage("standaloneExport", async () => {
    const downloadEvent = page.waitForEvent("download", { timeout: 180_000 });
    await page.getByTestId("standalone-export").click();
    const download = await downloadEvent;
    assert.equal(await download.failure(), null, "HTML download failed");
    htmlPath = path.join(run, "nikon-release-3d.html");
    await download.saveAs(htmlPath); fs.chmodSync(htmlPath, 0o600);
    const html = fs.readFileSync(htmlPath, "utf8");
    const payloadText = html.match(/<script id="em-payload" type="application\/json">([\s\S]*?)<\/script>/)?.[1];
    const modelText = html.match(/<script id="em-model" type="application\/octet-stream">([\s\S]*?)<\/script>/)?.[1];
    assert.ok(payloadText && modelText, "Missing standalone embedded payload/model");
    const payload = JSON.parse(payloadText);
    assert.equal(payload.schemaVersion, "em_standalone_viewer_v1"); assert.equal(payload.releaseId, releaseId);
    assert.deepEqual(payload.parts, expected.parts); assert.deepEqual(payload.steps, expected.steps); assert.deepEqual(payload.specs, expected.specs);
    const assetId = payload.interactive === null ? release.manifest.knowledge.model.assetId
      : release.manifest.knowledge.interactive.partsModel.assetId;
    const asset = release.manifest.assets.find((entry) => entry.assetId === assetId);
    assert.ok(asset, "Embedded GLB must be a frozen exported asset");
    const modelBytes = Buffer.from(modelText, "base64");
    assert.equal(sha256(modelBytes), asset.sha256);
    assert.ok([...zipEntries.values()].some((body) => body.equals(modelBytes)), "HTML/ZIP model bytes must match");
    return { htmlSha256: sha256(Buffer.from(html)), size: Buffer.byteLength(html), modelSha256: asset.sha256,
      allFactsAndSafetyNotesMatchFrozenReview: true, interactiveActions: payload.interactive?.actions.length ?? 0 };
  });
  await stage("offlineFileReader", async () => {
    offlineContext = await browser.newContext({ viewport: { width: 1440, height: 1000 }, offline: true,
      reducedMotion: "reduce", serviceWorkers: "block" });
    const requests = [];
    const errors = [];
    await offlineContext.route(/^https?:/, (route) => route.abort());
    const offline = await offlineContext.newPage();
    offline.setDefaultTimeout(60_000);
    offline.on("request", (request) => { if (/^https?:/.test(request.url())) requests.push(shortUrl(request.url())); });
    offline.on("pageerror", (error) => errors.push(scrub(error.message)));
    offline.on("console", (message) => { if (message.type() === "error") errors.push(scrub(message.text())); });
    await offline.goto(pathToFileURL(htmlPath).href);
    assert.equal(new URL(offline.url()).protocol, "file:");
    await expect(offline.locator("#em-status")).toContainText("拖动旋转", { timeout: 180_000 });
    const interaction = await canvasInteraction(offline, offline.locator("#em-canvas canvas"), offline.locator("#em-reset"), "offline");
    for (const [index, part] of expected.parts.entries()) {
      // Distinct parts may legitimately have identical displayed names.
      const button = offline.locator("#em-parts > li").nth(index).getByRole("button", { name: part.name, exact: true });
      await button.click(); await expect(button).toHaveAttribute("aria-pressed", "true");
    }
    for (let index = 0; index < expected.steps.length; index++) {
      const step = expected.steps[index];
      await expect(offline.locator("#em-step")).toContainText(step.title);
      for (const text of [...step.orderedActions, ...step.safetyNotes]) await expect(offline.locator("#em-step")).toContainText(text);
      if (index < expected.steps.length - 1) await offline.locator("#em-step-next").click();
    }
    for (const spec of expected.specs) {
      await expect(offline.locator("#em-specs")).toContainText(spec.label);
      await expect(offline.locator("#em-specs")).toContainText(spec.value);
    }
    const toggles = offline.locator('#em-interactions [role="group"][aria-label="动作"] button[aria-pressed]');
    // Each actual published toggle remains usable; no synthetic replacement action is added.
    for (let index = 0; index < await toggles.count(); index++) {
      const button = toggles.nth(index);
      if (await button.getAttribute("aria-pressed") === "false") {
        await button.click(); await expect(button).toHaveAttribute("aria-pressed", "true");
      }
    }
    const resetActions = offline.locator("#em-interactions").getByRole("button", { name: "复原", exact: true });
    if (await resetActions.count()) {
      await resetActions.click();
      for (let index = 0; index < await toggles.count(); index++) await expect(toggles.nth(index)).toHaveAttribute("aria-pressed", "false");
    }
    await offline.locator("#em-reset").click();
    await offline.screenshot({ path: path.join(run, "offline-reader.png"), fullPage: true });
    assert.deepEqual(requests, [], "Offline file attempted HTTP(S) requests");
    assert.deepEqual(errors, [], "Offline console/page errors");
    const actionTogglesVerified = await toggles.count();
    writeJson(path.join(run, "offline-verification.json"), { browser: state.browser, protocol: "file:", offline: true,
      requests, errors, ...interaction, actionTogglesVerified, allFactsAndSafetyNotesVisible: true });
    await offlineContext.close(); offlineContext = null;
    return { ...interaction, protocol: "file:", offline: true, httpRequests: 0, consoleAndPageErrors: 0,
      actionTogglesVerified, allFactsAndSafetyNotesVisible: true };
  });
  await stage("historyAndRequestPolicy", async () => {
    await page.goto(`${origin.origin}/items/${live.itemId}/generations`);
    const entry = page.getByTestId("history-entry").filter({ has: page.locator(`a[href="/items/${live.itemId}/releases/${releaseId}"]`) });
    await expect(entry).toHaveCount(1);
    if (live.jobId) await expect(entry.locator(`a[href="/jobs/${live.jobId}/result"]`)).toHaveCount(1);
    await page.screenshot({ path: path.join(run, "published-generation-history.png"), fullPage: true });
    assert.deepEqual(state.externalRequests, []); assert.deepEqual(state.blockedWrites, []); assert.deepEqual(state.onlineErrors, []);
    return { publishedHistoryPreserved: true, externalRequests: 0, blockedWrites: 0, onlineErrors: 0 };
  });
  state.status = "passed";
  delete state.currentStage;
} catch (error) {
  state.status = "failed";
  state.lastError = scrub(error.message);
  if (state.currentStage && state.stages[state.currentStage]) {
    state.stages[state.currentStage].status = "failed";
    state.stages[state.currentStage].error = state.lastError;
  }
  await page?.screenshot({ path: path.join(run, "failure.png"), fullPage: true }).catch(() => {});
  console.error(`Verification failed at ${state.currentStage ?? "browser setup"}; see ${path.join(run, "verification.json")}`);
  process.exitCode = 1;
} finally {
  await offlineContext?.close().catch(() => {});
  await context?.close().catch(() => {});
  await browser?.close().catch(() => {});
  state.finishedAt = new Date().toISOString(); save();
}
