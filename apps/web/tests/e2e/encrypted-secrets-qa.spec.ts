/** ES-01 independent real process/browser checks. Execute after RD_READY.
 * Keys/master are random and private; all evidence/failed assertions use booleans.
 */
import fs from "node:fs";
import path from "node:path";
import { expect, test, type APIRequestContext, type Page } from "@playwright/test";
import type { components } from "../../src/api/generated";
import { readDraftParts, readDraftSpecs, readDraftSteps } from "../../src/features/viewer/draft-view";
import { apiLogin, loginViaUi } from "./helpers";
import { REPO_ROOT, serverBinary } from "./runtime";
import { ApiSettingsQaBackend, CheckedProviderFixture, QA_PASSWORD, attachQaBackend, readyQuote } from "./api-settings-qa-harness";
import { runPrivateCommand, scanPrivateQaTree, secretPresence } from "./encrypted-secrets-qa-private";

const EVIDENCE = path.join(REPO_ROOT, "artifacts/encrypted-secrets/qa");
const SETTINGS = "/api/v1/settings/providers";
let a: CheckedProviderFixture;
let b: CheckedProviderFixture;
let backend: ApiSettingsQaBackend;
let externalHosts: string[];

function secrets(): string[] { return [backend.masterKey, ...Object.values(a.keys), ...Object.values(b.keys)]; }
function evidence(name: string, data: unknown) {
  fs.mkdirSync(EVIDENCE, { recursive: true });
  const encoded = JSON.stringify(data, null, 2);
  expect(secretPresence(encoded, secrets()).leaked, "evidence leak boolean").toBe(false);
  fs.writeFileSync(path.join(EVIDENCE, `${name}.json`), encoded);
}
async function screenshot(page: Page, name: string) {
  fs.mkdirSync(EVIDENCE, { recursive: true });
  await page.screenshot({ path: path.join(EVIDENCE, `${name}.png`), fullPage: true, mask: [page.locator('input[type="password"]')] });
}
async function safeJson<T>(response: Awaited<ReturnType<APIRequestContext["get"]>>): Promise<T> {
  const text = await response.text();
  expect(secretPresence(text, secrets()).leaked, "HTTP response leak boolean").toBe(false);
  return JSON.parse(text) as T;
}
async function view(request: APIRequestContext) {
  const response = await request.get(`${backend.base}${SETTINGS}`);
  expect(response.status()).toBe(200);
  return (await safeJson<components["schemas"]["ProviderSettingsResponse"]>(response)).data;
}
async function openSettings(page: Page) {
  await loginViaUi(page, "", QA_PASSWORD);
  await page.goto("/settings");
  await expect(page.getByLabel("Tripo Base URL", { exact: true })).toBeVisible();
}
async function browserSecretAbsent(page: Page) {
  const leak = await page.evaluate((values) => {
    const outputs = [location.href, JSON.stringify(localStorage), JSON.stringify(sessionStorage), document.documentElement.outerHTML];
    return values.some((key) => outputs.some((text) => text.includes(key)));
  }, secrets());
  expect(leak, "browser persisted secret boolean").toBe(false);
}

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
    if (backend) {
      const log = fs.readFileSync(backend.logPath);
      expect(secretPresence(log, secrets()).leaked, "server log leak boolean").toBe(false);
      const staged = runPrivateCommand("git", ["diff", "--cached", "--no-ext-diff", "--binary"], REPO_ROOT);
      expect(staged.status).toBe(0);
      expect(secretPresence(staged.stdout, secrets()).leaked, "Git index secret presence").toBe(false);
    }
    expect(externalHosts, "external browser requests").toEqual([]);
  } finally {
    await backend?.cleanup();
    await Promise.all([a?.stop(), b?.stop()]);
  }
});

test("ES-QA-01 加密说明/环境keep/两家网页密文与重启（AC001/009/010/012）", async ({ page, request }) => {
  await openSettings(page);
  await expect(page.getByText(/通过网页新增或替换的 API 密钥会加密保存在服务端/)).toBeVisible();
  await expect(page.getByText(/环境变量提供的密钥仅在运行内存中使用/)).toBeVisible();
  await expect(page.getByLabel(/主密钥/)).toHaveCount(0);
  for (const width of [375, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await screenshot(page, `es01-settings-${width}`);
  }
  await apiLogin(request, backend.base, QA_PASSWORD);
  await page.getByLabel("Tripo Base URL", { exact: true }).fill(`${a.base}/v3/changed`);
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByText("已保存，重启服务后生效", { exact: true }).first()).toBeVisible();
  let scanned = scanPrivateQaTree(backend.dataDir, secrets());
  expect(scanned.directMatches + scanned.jsonEscapedMatches).toBe(0);
  await page.getByLabel("Tripo Base URL", { exact: true }).fill(`${b.base}/v3`);
  await page.getByLabel("Tripo 模型", { exact: true }).fill(b.models.tripo);
  await page.getByLabel("说明书 AI Base URL", { exact: true }).fill(`${b.base}/v1`);
  await page.getByLabel("说明书 AI 模型", { exact: true }).fill(b.models.manualAi);
  await page.getByRole("radio", { name: "替换", exact: true }).nth(0).check();
  await page.getByRole("radio", { name: "替换", exact: true }).nth(1).check();
  await page.getByLabel("新的 Tripo 密钥", { exact: true }).fill(b.keys.tripo);
  await page.getByLabel(/^新的说明书 AI\s*密钥$/).fill(b.keys.manualAi);
  await expect(page.getByText("新密钥将在服务端加密保存，已保存密钥不会回显。", { exact: true })).toHaveCount(2);
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByLabel("新的 Tripo 密钥", { exact: true })).toHaveCount(0);
  const saved = await view(request);
  expect(saved.pending).toBe(true);
  const file = path.join(backend.dataDir, "provider-overrides.json");
  const bytes = fs.readFileSync(file);
  expect(secretPresence(bytes, secrets()).leaked).toBe(false);
  expect(bytes.includes(Buffer.from(backend.masterKey, "hex")), "raw master bytes absent").toBe(false);
  const envelopeCount = (bytes.toString("utf8").match(/everything-manual-secret/gu) ?? []).length;
  expect(envelopeCount).toBe(2);
  expect(fs.statSync(file).mode & 0o777).toBe(0o600);
  await backend.restart();
  const active = await view(request);
  expect(active.pending).toBe(false);
  expect(active.active).toEqual(saved.saved);
  await page.reload();
  await expect(page.getByText("当前运行配置已生效", { exact: true }).first()).toBeVisible();
  await browserSecretAbsent(page);
  scanned = scanPrivateQaTree(backend.dataDir, secrets());
  expect(scanned.directMatches + scanned.jsonEscapedMatches).toBe(0);
  evidence("es01-web-encryption", { envelopeCount, permissions: "0600", restartEffective: true, ...scanned });
});

test("ES-QA-02 真实写盘失败保持编辑，显式重试与375布局（AC008/010/012）", async ({ page, request }) => {
  await openSettings(page);
  await page.setViewportSize({ width: 375, height: 900 });
  await apiLogin(request, backend.base, QA_PASSWORD);
  const initial = await view(request);
  const file = path.join(backend.dataDir, "provider-overrides.json");
  fs.mkdirSync(file);
  await page.getByRole("radio", { name: "替换", exact: true }).first().check();
  const field = page.getByLabel("新的 Tripo 密钥", { exact: true });
  await field.fill(b.keys.tripo);
  let putCount = 0;
  page.on("request", (req) => { if (req.method() === "PUT" && new URL(req.url()).pathname === SETTINGS) putCount += 1; });
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  const alert = page.getByRole("alert");
  await expect(alert).toContainText("密钥配置文件不可读、不是受限普通文件或超出大小限制；未读取内容。");
  await expect(alert).toContainText(/请求 ID/);
  expect(await field.inputValue() === b.keys.tripo, "failure keeps secret only in form memory").toBe(true);
  await expect(field).not.toHaveAttribute("aria-invalid", "true");
  await expect(page.getByText("有未保存修改", { exact: true })).toBeVisible();
  expect((await view(request)).revision).toBe(initial.revision);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  await browserSecretAbsent(page);
  await screenshot(page, "es02-save-failure-375");
  expect(putCount).toBe(1);
  fs.rmdirSync(file);
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await expect(page.getByText("已保存，重启服务后生效", { exact: true }).first()).toBeVisible();
  expect(putCount).toBe(2);
  await expect(field).toHaveCount(0);
  evidence("es02-failure-recovery", { failedSavePreservedRevision: true, retainedEditing: true, explicitRetryCount: putCount, browserLeak: false });
});

async function waitJob(request: APIRequestContext, id: string, status: string) {
  let last: components["schemas"]["JobDetailDto"] | undefined;
  await expect.poll(async () => {
    const response = await request.get(`${backend.base}/api/v1/jobs/${id}`);
    expect(response.status()).toBe(200);
    last = (await safeJson<components["schemas"]["JobDetailResponse"]>(response)).data;
    return last.status;
  }, { timeout: 60_000, message: "job reaches expected state without secret-bearing failure context" }).toBe(status);
  return last!;
}

test("ES-QA-03 外部文件CLI→重启→真实worker→反射诊断/备份/导出无泄漏（AC007/010/011）", async ({ request }) => {
  await backend.stop();
  const configPath = path.join(backend.workDir, "config.toml");
  let config = fs.readFileSync(configPath, "utf8");
  for (const [provider, selector, key] of [
    ["tripo", "EM_API_SETTINGS_QA_TRIPO", a.keys.tripo],
    ["manual-ai", "EM_API_SETTINGS_QA_MANUAL", a.keys.manualAi],
  ] as const) {
    const input = path.join(backend.workDir, `${provider}.input`);
    const output = path.join(backend.workDir, `${provider}.encrypted`);
    fs.writeFileSync(input, `${key}\n`, { mode: 0o600 });
    const original = fs.readFileSync(input);
    const args = ["encrypt-api-key", "--provider", provider, "--input", input, "--output", output];
    const converted = runPrivateCommand(serverBinary(), args, backend.workDir, { EM_SECRETS_MASTER_KEY: backend.masterKey });
    expect(secretPresence(Buffer.concat([converted.stdout, converted.stderr]), secrets()).leaked).toBe(false);
    expect(converted.status).toBe(0);
    expect(fs.readFileSync(input).equals(original), "external plaintext source unchanged").toBe(true);
    expect(fs.statSync(output).mode & 0o777).toBe(0o600);
    expect(secretPresence(fs.readFileSync(output), secrets()).leaked).toBe(false);
    const duplicate = runPrivateCommand(serverBinary(), args, backend.workDir, { EM_SECRETS_MASTER_KEY: backend.masterKey });
    expect(duplicate.status).not.toBe(0);
    expect(secretPresence(Buffer.concat([duplicate.stdout, duplicate.stderr]), secrets()).leaked).toBe(false);
    config = config.replace(`api_key_env = "${selector}"`, `api_key_file = ${JSON.stringify(output)}`);
  }
  fs.writeFileSync(configPath, config);
  let reflectedReplies = 0;
  const reflectDiagnostic = (pathname: string, reply: { status: number; body: Buffer }) => {
    if (pathname !== "/v1/responses") return reply;
    const body = JSON.parse(reply.body.toString("utf8")) as Record<string, unknown>;
    body.qaDiagnostic = { direct: a.keys.tripo, escaped: a.keys.manualAi };
    reflectedReplies += 1;
    let encoded = JSON.stringify(body);
    const escaped = [...a.keys.manualAi].map((char) => `\\u${char.charCodeAt(0).toString(16).padStart(4, "0")}`).join("");
    encoded = encoded.replace(a.keys.manualAi, escaped);
    return { status: reply.status, body: Buffer.from(encoded) };
  };
  await backend.start();
  const ready = await readyQuote(request, backend, 0);
  const created = await request.post(`${backend.base}/api/v1/items/${ready.itemId}/jobs`, {
    headers: { "x-csrf-token": ready.csrf, "idempotency-key": "es-qa-encrypted-files-worker" }, data: ready.jobBody,
  });
  expect(created.status()).toBe(202);
  const id = (await safeJson<components["schemas"]["JobResponse"]>(created)).data.id;
  const done = await waitJob(request, id, "succeeded");
  expect(a.checks.authMatches).toBe(true);
  expect(a.checks.modelMatches).toBe(true);
  expect(a.checks.tripoRequests).toBeGreaterThan(0);
  expect(a.checks.manualRequests).toBeGreaterThan(0);
  expect(done.draftId).toBeTruthy();
  const draftPath = `${backend.base}/api/v1/items/${ready.itemId}/drafts/${done.draftId}`;
  const draftRead = await request.get(draftPath);
  expect(draftRead.status()).toBe(200);
  const draft = (await safeJson<components["schemas"]["DraftResponse"]>(draftRead)).data;
  const entities: NonNullable<components["schemas"]["DraftPatchRequest"]["entities"]> = {};
  for (const part of readDraftParts(draft.knowledge)) entities[part.id] = { reviewStatus: "confirmed", textOnly: true };
  for (const entity of [...readDraftSteps(draft.knowledge), ...readDraftSpecs(draft.knowledge)]) entities[entity.id] = { reviewStatus: "confirmed" };
  const body: components["schemas"]["DraftPatchRequest"] = { entities, modelReview: { loaded: true, userConfirmed: true } };
  expect(draftRead.headers().etag).toBeTruthy();
  const patched = await request.patch(draftPath, { headers: { "x-csrf-token": ready.csrf, "if-match": draftRead.headers().etag ?? "" }, data: body });
  expect(patched.status()).toBe(200);
  await safeJson(patched);
  expect(patched.headers().etag).toBeTruthy();
  const published = await request.post(`${draftPath}/publish`, { headers: { "x-csrf-token": ready.csrf, "if-match": patched.headers().etag ?? "", "idempotency-key": "es-qa-publish" } });
  expect(published.status()).toBe(201);
  const release = await safeJson<components["schemas"]["ReleaseResponse"]>(published);
  const exported = await request.get(`${backend.base}/api/v1/releases/${release.data.id}/export`);
  expect(exported.status()).toBe(200);
  const zip = await exported.body();
  expect(zip.length).toBeGreaterThan(1000);
  expect(secretPresence(zip, secrets()).leaked, "release export secret presence").toBe(false);
  expect(zip.includes(Buffer.from("provider-overrides")), "private configuration excluded from export").toBe(false);
  // The frozen implementation safely refuses secret-bearing 2xx replies. Verify
  // that refusal, rather than requiring it to accept unsafe diagnostics as success.
  a.responseTransform = reflectDiagnostic;
  const reflectedReady = await readyQuote(request, backend, 0);
  const reflectedCreate = await request.post(`${backend.base}/api/v1/items/${reflectedReady.itemId}/jobs`, {
    headers: { "x-csrf-token": reflectedReady.csrf, "idempotency-key": "es-qa-sensitive-2xx" }, data: reflectedReady.jobBody,
  });
  expect(reflectedCreate.status()).toBe(202);
  const reflectedId = (await safeJson<components["schemas"]["JobResponse"]>(reflectedCreate)).data.id;
  await expect.poll(async () => {
    const response = await request.get(`${backend.base}/api/v1/jobs/${reflectedId}`);
    const current = (await safeJson<components["schemas"]["JobDetailResponse"]>(response)).data;
    return current.stages.find((stage) => stage.stageKind === "manual_extract")?.status;
  }, { timeout: 60_000 }).toBe("submission_unknown");
  expect(reflectedReplies).toBe(1);
  await backend.stop();
  const backup = path.join(backend.workDir, "backup-out");
  const backed = runPrivateCommand(serverBinary(), ["backup", "--data-dir", backend.dataDir, "--out", backup], backend.workDir, { EM_SECRETS_MASTER_KEY: backend.masterKey });
  expect(backed.status).toBe(0);
  expect(secretPresence(Buffer.concat([backed.stdout, backed.stderr]), secrets()).leaked).toBe(false);
  const scans = { data: scanPrivateQaTree(backend.dataDir, secrets()), backup: scanPrivateQaTree(backup, secrets()) };
  for (const result of Object.values(scans)) expect(result.directMatches + result.jsonEscapedMatches).toBe(0);
  expect(fs.existsSync(path.join(backup, "provider-overrides.json"))).toBe(false);
  evidence("es03-cli-worker-reflection-backup-export", { convertedProviders: 2, reflectedReplies, providerChecks: a.checks, exportBytes: zip.length, exportLeak: false, sourcePreserved: true, scans });
});

for (const [label, endpoint, stage, state] of [
  ["manual-ai", "/v1/responses", "manual_extract", "needs_input"],
  ["tripo", "/v3/generation/multiview-to-model", "tripo_submit", "failed"],
] as const) {
  test(`ES-QA-04-${label} 错误直接与JSON转义反射，不改变业务分类（AC011）`, async ({ request }) => {
    let reflected = 0;
    for (const mode of ["direct", "json-escaped"] as const) {
      const key = label === "manual-ai" ? a.keys.manualAi : a.keys.tripo;
      a.responseTransform = (pathname, reply) => {
        if (pathname !== endpoint) return reply;
        reflected += 1;
        const payload = label === "manual-ai"
          ? { error: { message: `fixture rejected ${key}` } }
          : { code: 1001, message: `fixture rejected ${key}` };
        const escaped = [...key].map((char) => `\\u${char.charCodeAt(0).toString(16).padStart(4, "0")}`).join("");
        const encoded = JSON.stringify(payload);
        return { status: 400, body: Buffer.from(mode === "direct" ? encoded : encoded.replace(key, escaped)) };
      };
      const ready = await readyQuote(request, backend, 0);
      const created = await request.post(`${backend.base}/api/v1/items/${ready.itemId}/jobs`, {
        headers: { "x-csrf-token": ready.csrf, "idempotency-key": `es-qa-reflection-${label}-${mode}` }, data: ready.jobBody,
      });
      expect(created.status()).toBe(202);
      const id = (await safeJson<components["schemas"]["JobResponse"]>(created)).data.id;
      await expect.poll(async () => {
        const response = await request.get(`${backend.base}/api/v1/jobs/${id}`);
        expect(response.status()).toBe(200);
        const current = (await safeJson<components["schemas"]["JobDetailResponse"]>(response)).data;
        return current.stages.find((value) => value.stageKind === stage)?.status;
      }, { timeout: 60_000, message: "expected refusal classification" }).toBe(state);
    }
    expect(reflected).toBe(2);
    await backend.stop();
    const backup = path.join(backend.workDir, "backup-reflection");
    const backed = runPrivateCommand(serverBinary(), ["backup", "--data-dir", backend.dataDir, "--out", backup], backend.workDir, { EM_SECRETS_MASTER_KEY: backend.masterKey });
    expect(backed.status).toBe(0);
    expect(secretPresence(Buffer.concat([backed.stdout, backed.stderr]), secrets()).leaked).toBe(false);
    const scans = { data: scanPrivateQaTree(backend.dataDir, secrets()), backup: scanPrivateQaTree(backup, secrets()) };
    for (const result of Object.values(scans)) expect(result.directMatches + result.jsonEscapedMatches).toBe(0);
    evidence(`es04-error-reflection-${label}`, { reflected, stage, state, scans, authMatches: a.checks.authMatches });
  });
}

test("ES-QA-05 真实serve迁移保留旧报价代次，check只读（AC005/006/010）", async ({ request }) => {
  const csrf = await apiLogin(request, backend.base, QA_PASSWORD);
  const initial = await view(request);
  const edit = (provider: "tripo" | "manualAi"): components["schemas"]["ProviderEdit"] => ({
    action: "update", baseUrl: initial.saved[provider].baseUrl, model: initial.saved[provider].model, keyAction: "replace", apiKey: a.keys[provider],
  });
  const body: components["schemas"]["ProviderSettingsWrite"] = { revision: initial.revision, tripo: edit("tripo"), manualAi: edit("manualAi") };
  const save = await request.put(`${backend.base}${SETTINGS}`, { headers: { "x-csrf-token": csrf }, data: body });
  expect(save.status()).toBe(200);
  const before = await view(request);
  expect(before.pending).toBe(false);
  expect(before.revision).not.toBe("deployment");
  const ready = await readyQuote(request, backend, 0);
  await backend.stop();
  const legacyProvider = (provider: "tripo" | "manualAi") => ({
    baseUrl: before.saved[provider].baseUrl, model: before.saved[provider].model,
    key: { mode: "replace", value: a.keys[provider] },
  });
  // Exact historical Overlay fixture, intentionally plaintext and never evidence.
  const legacy = Buffer.from(JSON.stringify({ revision: before.revision, tripo: legacyProvider("tripo"), manualAi: legacyProvider("manualAi") }));
  const file = path.join(backend.dataDir, "provider-overrides.json");
  fs.writeFileSync(file, legacy, { mode: 0o600 });
  const check = runPrivateCommand(serverBinary(), ["check", "--data-dir", backend.dataDir], backend.workDir, {
    EM_SECRETS_MASTER_KEY: backend.masterKey,
    EM_API_SETTINGS_QA_TRIPO: a.keys.tripo, EM_API_SETTINGS_QA_MANUAL: a.keys.manualAi,
    EM_PROVIDERS__TRIPO__MODEL: a.models.tripo, EM_PROVIDERS__MANUAL_AI__MODEL: a.models.manualAi,
  });
  expect(check.status).not.toBe(0);
  expect(check.stderr.toString("utf8").includes("serve"), "check supplies migration guidance").toBe(true);
  expect(secretPresence(Buffer.concat([check.stdout, check.stderr]), secrets()).leaked).toBe(false);
  expect(fs.readFileSync(file).equals(legacy), "check did not rewrite legacy input").toBe(true);
  await backend.start();
  const after = await view(request);
  expect(after.revision).toBe(before.revision);
  expect(after.pending).toBe(false);
  expect(after.active.tripo.keySource).toBe("web");
  expect(after.active.manualAi.keySource).toBe("web");
  expect(secretPresence(fs.readFileSync(file), secrets()).leaked).toBe(false);
  const created = await request.post(`${backend.base}/api/v1/items/${ready.itemId}/jobs`, {
    headers: { "x-csrf-token": ready.csrf, "idempotency-key": "es-qa-quote-before-migration" }, data: ready.jobBody,
  });
  expect(created.status()).toBe(202);
  const job = (await safeJson<components["schemas"]["JobResponse"]>(created)).data;
  await waitJob(request, job.id, "succeeded");
  expect(a.checks.authMatches).toBe(true);
  expect(a.checks.modelMatches).toBe(true);
  const scanned = scanPrivateQaTree(backend.dataDir, secrets());
  expect(scanned.directMatches + scanned.jsonEscapedMatches).toBe(0);
  evidence("es05-legacy-process-migration", { checkReadOnly: true, sameRevision: true, pending: after.pending, preMigrationQuoteConsumed: true, providerChecks: a.checks, scanned });
});

for (const [label, endpoint, stage, state] of [
  ["manual-ai", "/v1/responses", "manual_extract", "submission_unknown"],
  ["tripo", "/v3/generation/multiview-to-model", "tripo_submit", "failed"],
] as const) {
  test(`ES-QA-06-${label} BUG-ES-001 损坏JSON全/混合转义丢弃且不重复购买（AC011）`, async ({ request }) => {
    let reflected = 0;
    const observations: { mode: string; state: string; requestsStable: boolean }[] = [];
    for (const mode of ["unicode", "mixed"] as const) {
      const key = label === "manual-ai" ? a.keys.manualAi : a.keys.tripo;
      const encoded = [...key].map((char, index) => mode === "mixed" && index % 2 === 0 ? char : `\\u${char.charCodeAt(0).toString(16).padStart(4, "0")}`).join("");
      a.responseTransform = (pathname, reply) => {
        if (pathname !== endpoint) return reply;
        reflected += 1;
        // Complete HTTP body, deliberately unfinished JSON string/envelope.
        return { status: label === "manual-ai" ? 200 : 400, body: Buffer.from(`{"message":"fixture ${encoded}`) };
      };
      const ready = await readyQuote(request, backend, 0);
      const created = await request.post(`${backend.base}/api/v1/items/${ready.itemId}/jobs`, {
        headers: { "x-csrf-token": ready.csrf, "idempotency-key": `es-qa-malformed-${label}-${mode}` }, data: ready.jobBody,
      });
      expect(created.status()).toBe(202);
      const id = (await safeJson<components["schemas"]["JobResponse"]>(created)).data.id;
      await expect.poll(async () => {
        const response = await request.get(`${backend.base}/api/v1/jobs/${id}`);
        const current = (await safeJson<components["schemas"]["JobDetailResponse"]>(response)).data;
        return current.stages.find((value) => value.stageKind === stage)?.status;
      }, { timeout: 60_000 }).toBe(state);
      const atRest = reflected;
      // The production executor wakes every 250ms. Observe > 3 wakeups after unknown.
      for (let observation = 0; observation < 4; observation += 1) {
        await new Promise((resolve) => setTimeout(resolve, 350));
        const response = await request.get(`${backend.base}/api/v1/jobs/${id}`);
        const current = (await safeJson<components["schemas"]["JobDetailResponse"]>(response)).data;
        expect(current.stages.find((value) => value.stageKind === stage)?.status).toBe(state);
        expect(reflected, "unknown/failed response does not trigger an automatic new purchase").toBe(atRest);
      }
      observations.push({ mode, state, requestsStable: reflected === atRest });
    }
    expect(reflected).toBe(2);
    await backend.stop();
    const backup = path.join(backend.workDir, "backup-malformed");
    const backed = runPrivateCommand(serverBinary(), ["backup", "--data-dir", backend.dataDir, "--out", backup], backend.workDir, { EM_SECRETS_MASTER_KEY: backend.masterKey });
    expect(backed.status).toBe(0);
    expect(secretPresence(Buffer.concat([backed.stdout, backed.stderr]), secrets()).leaked).toBe(false);
    const scans = { data: scanPrivateQaTree(backend.dataDir, secrets()), backup: scanPrivateQaTree(backup, secrets()) };
    evidence(`es06-malformed-${label}`, { reflected, observations, scans, authMatches: a.checks.authMatches });
    for (const result of Object.values(scans)) expect(result.directMatches + result.jsonEscapedMatches).toBe(0);
  });
}

test("ES-QA-07 合法JSON信封仍保留安全诊断（AC011）", async ({ request }) => {
  const marker = "encrypted-secrets-qa-safe-diagnostic-control";
  let replies = 0;
  a.responseTransform = (pathname, reply) => {
    if (pathname !== "/v1/responses") return reply;
    replies += 1;
    const body = JSON.parse(reply.body.toString("utf8")) as { output: { content: { text?: string }[] }[]; qaMarker?: string };
    body.output[0]!.content[0]!.text = "{invalid structured knowledge";
    body.qaMarker = marker;
    return { status: 200, body: Buffer.from(JSON.stringify(body)) };
  };
  const ready = await readyQuote(request, backend, 0);
  const created = await request.post(`${backend.base}/api/v1/items/${ready.itemId}/jobs`, {
    headers: { "x-csrf-token": ready.csrf, "idempotency-key": "es-qa-safe-json-diagnostic" }, data: ready.jobBody,
  });
  expect(created.status()).toBe(202);
  const id = (await safeJson<components["schemas"]["JobResponse"]>(created)).data.id;
  await expect.poll(async () => {
    const response = await request.get(`${backend.base}/api/v1/jobs/${id}`);
    const current = (await safeJson<components["schemas"]["JobDetailResponse"]>(response)).data;
    return current.stages.find((value) => value.stageKind === "manual_extract")?.status;
  }, { timeout: 60_000 }).toBe("needs_input");
  await backend.stop();
  let preservedDiagnosticFiles = 0;
  function visit(dir: string) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) visit(file);
      else if (entry.isFile() && fs.readFileSync(file).includes(Buffer.from(marker))) preservedDiagnosticFiles += 1;
    }
  }
  visit(path.join(backend.dataDir, "blobs"));
  expect(replies).toBe(1);
  expect(preservedDiagnosticFiles, "valid JSON diagnostic asset remains available").toBeGreaterThan(0);
  const scans = scanPrivateQaTree(backend.dataDir, secrets());
  expect(scans.directMatches + scans.jsonEscapedMatches).toBe(0);
  evidence("es07-safe-diagnostic-control", { replies, preservedDiagnosticFiles, scans });
});
