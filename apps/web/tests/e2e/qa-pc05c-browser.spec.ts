/** Six-group acceptance with API file; never execute against mutable source. */
import { expect, request, test, type APIRequestContext, type BrowserContext } from "@playwright/test";
import { apiLogin } from "./helpers";
import { evidence, PASSWORD, Pc05cBackend, WEB } from "./qa-pc05c-backend";
import { boot, BUFFER_CANARY, expireSession, loginBack, SECRET_CANARY, tabTo } from "./qa-pc05c-controls";
import { cloneActivityJobs, detail } from "./qa-pc05c-jobs";
import { createItem, draftFixture, jobFixture, materialFixture, readDraft, readItem } from "./qa-pc05c-fixture";
let b: Pc05cBackend, api: APIRequestContext, csrf: string;
const contexts: BrowserContext[] = [], sessions: Awaited<ReturnType<typeof boot>>[] = [], releases: Array<() => void> = [];
test.beforeEach(async () => { b = new Pc05cBackend(); await b.start(); api = await request.newContext(); csrf = await apiLogin(api, b.base, PASSWORD); });
test.afterEach(async () => { try { for (const f of releases.splice(0)) f(); for (const s of sessions.splice(0)) await s.verify(); } finally { for (const c of contexts.splice(0)) await c.close(); await api?.dispose(); await b?.cleanup(); } });
async function session(...args: Parameters<typeof boot>) { const s = await boot(...args); sessions.push(s); return s; }

test("C3 actual Retry-After countdown updates, zero is inert, Firefox background covered separately, terminal and unknown stop retry UI", async ({ browser }) => {
  b.fixture.taskReplies.push({ kind: "retry", after: 20 }, { kind: "retry", after: 25 });
  const f = await jobFixture(api, b); const s = await session(browser, b, contexts); const { page } = s;
  const traffic: Array<{ kind: string; at: number; url: string; requestStart?: number; responseEnd?: number; status?: string; code?: number }> = [];
  page.on("request", r => { if (new URL(r.url()).pathname === `/api/v1/jobs/${f.jobId}`) traffic.push({ kind: "request", at: Date.now(), url: r.url(), requestStart: r.timing().startTime }); });
  page.on("response", r => { if (new URL(r.url()).pathname === `/api/v1/jobs/${f.jobId}`) void r.json().then(async data => { await r.finished(); const timing = r.request().timing(); traffic.push({ kind: "response", at: Date.now(), url: r.url(), requestStart: timing.startTime, responseEnd: timing.responseEnd, status: data.data?.status, code: r.status() }); }); });
  await page.addInitScript(() => { (window as unknown as { __qa5cEvents: unknown[] }).__qa5cEvents = []; for (const type of ["visibilitychange", "focus", "blur"]) (type === "visibilitychange" ? document : window).addEventListener(type, event => (window as unknown as { __qa5cEvents: unknown[] }).__qa5cEvents.push({ type, at: Date.now(), trusted: event.isTrusted, visibility: document.visibilityState })); });
  await page.goto(`${WEB}/jobs/${f.jobId}`); const row = page.getByTestId("job-stage").filter({ has: page.getByText("第 1/5 次安全重试", { exact: true }) });
  await expect(row).toBeVisible(); const first = (await detail(api, b, f.jobId)).stages.find(v => v.stageKind === "tripo_poll")!;
  expect(first.safeRetry).toEqual({ number: 1, limit: 5 }); expect(first.nextRunAt).toBeTruthy();
  await expect(row.getByTestId("safe-retry-countdown")).toHaveText(/预计 0:\d{2} 后重试/);
  expect(await row.getByTestId("safe-retry-countdown").evaluate(el => el.closest('[aria-live="polite"],[aria-live="assertive"],[role="alert"]') !== null)).toBe(false);
  // Root-authorized split: known Chrome tool limitation is retained in author-original.
  // Native hidden/resume is a separate real Firefox157 product case, never a synthetic event.
  await page.waitForTimeout(2300); const visibility = { browser: "Chrome154", status: "NOT_RUN_KNOWN_TOOL_LIMIT", productSupplement: "C7 Firefox157 native window" };
  const remaining = Math.max(0, Math.ceil((Date.parse(first.nextRunAt!) - Date.now()) / 1000));
  const shown = Number((await row.getByTestId("safe-retry-countdown").innerText()).match(/0:(\d{2})/)?.[1]); expect(Math.abs(shown - remaining)).toBeLessThanOrEqual(1);
  await expect(page.getByText("第 2/5 次安全重试", { exact: true })).toBeVisible({ timeout: 30000 });
  const second = (await detail(api, b, f.jobId)).stages.find(v => v.stageKind === "tripo_poll")!; expect(Date.parse(second.nextRunAt!)).toBeGreaterThan(Date.parse(first.nextRunAt!));
  // Real clock only. Hold an actual prerequisite in the owned DB, prove the dependency
  // and blocked status, then restore the exact original value after real deadline expiry.
  const prerequisite = b.db("SELECT dep.id,dep.stage_kind,dep.status FROM job_stage_deps d JOIN job_stages dep ON dep.id=d.depends_on_stage_id WHERE d.stage_id=? AND dep.stage_kind='tripo_submit'", [second.id])[0];
  expect(prerequisite?.status).toBe("succeeded"); expect(prerequisite?.stage_kind).toBe("tripo_submit");
  b.db("UPDATE job_stages SET status='needs_input' WHERE id=?", [prerequisite!.id]);
  expect(b.db("SELECT status FROM job_stages WHERE id=?", [prerequisite!.id])).toEqual([{ status: "needs_input" }]);
  const writes = [...s.wire.writes], heldFacts = b.facts(), holdStartedAt = Date.now();
  await expect(page.getByTestId("safe-retry-countdown")).toHaveText("等待调度更新", { timeout: 35000 }); await page.waitForTimeout(2100);
  expect(Date.now()).toBeGreaterThan(Date.parse(second.nextRunAt!)); expect(s.wire.writes).toEqual(writes); expect(b.facts()).toEqual(heldFacts);
  const blockedAfterDue = await detail(api, b, f.jobId); expect(blockedAfterDue.stages.find(stage => stage.id === second.id)?.status).toBe("retry_wait");
  b.db("UPDATE job_stages SET status=? WHERE id=?", [prerequisite!.status, prerequisite!.id]);
  evidence("c3-real-clock-hold", { jobId: f.jobId, stageId: second.id, prerequisite, nextRunAt: second.nextRunAt, holdStartedAt, resumedAt: Date.now(), blockedBeyondActualDeadline: true, noProviderOrBusinessDelta: heldFacts, noFakeClockInstalled: true });
  await expect.poll(async () => (await detail(api, b, f.jobId)).status, { timeout: 60000 }).toBe("succeeded");
  await expect(page.getByTestId("job-detail-status")).toContainText("已完成（可复核草稿已产出，尚未发布）"); await expect(page.getByTestId("safe-retry-countdown")).toHaveCount(0); const detailReads = s.wire.reads.filter(v => v === `/api/v1/jobs/${f.jobId}`).length, terminalObservedAt = Date.now(); await page.waitForTimeout(2600);
  try { expect(s.wire.reads.filter(v => v === `/api/v1/jobs/${f.jobId}`).length).toBe(detailReads); }
  catch (error) { await page.waitForTimeout(4500); evidence("c3-terminal-read-diagnostic", { jobId: f.jobId, terminalObservedAt, detailReads, finalReads: s.wire.reads.filter(v => v === `/api/v1/jobs/${f.jobId}`).length, traffic, events: await page.evaluate(() => (window as unknown as { __qa5cEvents: unknown[] }).__qa5cEvents) }); throw error; }
  // Explicit owned DB state fixture supplements real executor evidence: a blocked dependency
  // prevents this null-deadline/read-state matrix from launching any extra remote work.
  const beforeStates = b.facts();
  b.db("UPDATE job_stages SET status='needs_input' WHERE job_id=? AND stage_kind='tripo_submit'", [f.jobId]);
  b.db("UPDATE job_stages SET status='retry_wait',attempt_count=3,poll_count=89,next_run_at=NULL WHERE job_id=? AND stage_kind='tripo_poll'", [f.jobId]);
  b.db("UPDATE jobs SET status='retry_wait',revision=revision+1 WHERE id=?", [f.jobId]);
  await page.reload(); await expect(page.getByText("第 3/5 次安全重试", { exact: true })).toBeVisible(); await expect(page.getByTestId("safe-retry-countdown")).toHaveText("等待调度，重试时间暂不可用");
  b.db("UPDATE job_stages SET status='running',lease_owner='qa5c-read-only-fixture',lease_until=? WHERE job_id=? AND stage_kind='tripo_poll'", [Date.now() + 3600000, f.jobId]);
  b.db("UPDATE jobs SET status='running',revision=revision+1 WHERE id=?", [f.jobId]); await expect(page.getByTestId("safe-retry-countdown")).toHaveCount(0);
  b.db("UPDATE job_stages SET status='failed',lease_owner=NULL,lease_until=NULL WHERE job_id=? AND stage_kind='tripo_poll'", [f.jobId]);
  b.db("UPDATE jobs SET status='failed',revision=revision+1 WHERE id=?", [f.jobId]); await expect(page.getByTestId("job-detail-status")).toContainText("失败"); await expect(page.getByTestId("safe-retry-countdown")).toHaveCount(0);
  expect(b.facts()).toEqual(beforeStates);
  b.fixture.state.submitMode = "http500"; const unknown = await jobFixture(api, b); await expect.poll(async () => (await detail(api, b, unknown.jobId)).status, { timeout: 60000 }).toBe("submission_unknown");
  await page.goto(`${WEB}/jobs/${unknown.jobId}`); await expect(page.getByTestId("reconcile-panel")).toBeVisible(); await expect(page.getByTestId("safe-retry-countdown")).toHaveCount(0); await expect(page.getByTestId("stage-retry-button")).toHaveCount(0);
  const unknownFacts = b.facts(); await page.waitForTimeout(2300); expect(b.facts()).toEqual(unknownFacts);
  evidence("c3-countdown", { first: { safeRetry: first.safeRetry, nextRunAt: first.nextRunAt }, second: { safeRetry: second.safeRetry, nextRunAt: second.nextRunAt }, visibility, zeroTest: "real deadline with explicitly proven temporary owned dependency hold, restored before actual success", zeroNoPost: true, supplementalBlockedDbStates: ["missingNextRunAt", "running", "failed"], terminalDetailPollingStopped: true, unknownJobId: unknown.jobId, unknownNoExtraPurchase: true });
});

test("C4 aggregate polls beyond page one, observes zero-to-new, error is unavailable and 375 keyboard link stays usable", async ({ browser }) => {
  const f = await draftFixture(api, b); const ids = cloneActivityJobs(b, f.ref.jobId); const s = await session(browser, b, contexts, 375); const { page } = s;
  const link = page.getByTestId("job-activity-link"); await expect(link).toHaveAccessibleName("进行中任务 28 个");
  const box = await link.boundingBox(); expect(box?.height).toBeGreaterThanOrEqual(44); expect(box?.width).toBeGreaterThanOrEqual(44);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
  const listReads = () => s.wire.reads.filter(v => /^\/api\/v1\/jobs(?:\?|$)/.test(v)); expect(listReads()).toEqual([]);
  b.db("UPDATE jobs SET status='succeeded',revision=revision+1 WHERE id=?", [ids.active[0]]); await expect(link).toHaveAccessibleName("进行中任务 27 个");
  b.db("UPDATE jobs SET status='cancelled',revision=revision+1 WHERE status IN ('queued','running','retry_wait','waiting_provider')"); await expect(link).toHaveAccessibleName("进行中任务 0 个");
  b.db("UPDATE jobs SET status='queued',revision=revision+1 WHERE id=?", [ids.active[0]]); await expect(link).toHaveAccessibleName("进行中任务 1 个");
  const countBefore = s.wire.reads.filter(v => v === "/api/v1/jobs/activity").length; await page.waitForTimeout(4500); const countAfter = s.wire.reads.filter(v => v === "/api/v1/jobs/activity").length; expect(countAfter - countBefore).toBeLessThanOrEqual(3); expect(listReads()).toEqual([]);
  const failure = async (route: import("@playwright/test").Route) => route.abort("failed"); await page.route("**/api/v1/jobs/activity", failure); await expect(link).toHaveAccessibleName("任务数暂不可用"); await expect(link).toHaveAttribute("href", "/jobs");
  await tabTo(page, link); await page.keyboard.press("Enter"); await expect(page).toHaveURL(WEB + "/jobs"); await page.unroute("**/api/v1/jobs/activity", failure); await expect(link).toHaveAccessibleName("进行中任务 1 个");
  evidence("c4-activity-ui", { counts: [28, 27, 0, 1], privateCountFixture: true, noPagedListReadsInShell: true, visiblePollReadsIn4500ms: countAfter - countBefore, unavailableNotZero: true, keyboardEntry: true, width: 375, geometry: box });
});

test("C5 anonymous login is ordinary; actual expiry preserves safe next and B item/knowledge memory without storage or replay", async ({ browser }) => {
  const s = await session(browser, b, contexts, 375, false); const { page } = s;
  const next = "/items/new?qa=pc05c"; await page.goto(WEB + next); await expect(page).toHaveURL(WEB + "/login?next=" + encodeURIComponent(next));
  await expect(page.getByText(/登录已过期/)).toHaveCount(0); await page.getByLabel("密码", { exact: true }).fill("PC05C-wrong-local-password"); await page.getByRole("button", { name: "登录", exact: true }).click(); await expect(page.getByRole("heading", { name: "密码不正确", exact: true })).toBeVisible(); await expect(page.getByText(/登录已过期/)).toHaveCount(0);
  await page.getByLabel("密码", { exact: true }).fill(PASSWORD); await page.getByRole("button", { name: "登录", exact: true }).click(); await expect(page).toHaveURL(WEB + next);
  const item = await createItem(api, b, csrf); await page.goto(`${WEB}/items/${item.id}/edit`); await page.getByLabel(/^名称/).fill(BUFFER_CANARY + "item"); await expireSession(page, b);
  const denied = page.waitForResponse(r => r.request().method() === "PATCH" && r.url().endsWith(`/items/${item.id}`)); await page.getByRole("button", { name: "保存", exact: true }).click(); expect((await denied).status()).toBe(401); await loginBack(page); await expect(page).toHaveURL(`${WEB}/items/${item.id}/edit`); await expect(page.getByLabel(/^名称/)).toHaveValue(BUFFER_CANARY + "item");
  expect(s.wire.writes.filter(v => v === `PATCH /api/v1/items/${item.id}`)).toHaveLength(1); expect((await readItem(api, b, item.id)).item.name).toBe(item.name); await s.verify();
  // Full navigation is an explicit discard of this test buffer, not a claim of cross-refresh persistence.
  page.once("dialog", dialog => dialog.accept()); const f = await draftFixture(api, b); await page.goto(`${WEB}/items/${f.ref.itemId}/drafts/${f.ref.draftId}/review`); await page.getByRole("button", { name: "步骤与原文", exact: true }).click();
  const part = f.initial.knowledge.knowledge.parts[0]!; const row = page.getByTestId(`knowledge-${part.id}`); await row.getByRole("button", { name: "复制为本地修订", exact: true }).click(); await page.getByLabel("部件名", { exact: true }).fill(BUFFER_CANARY + "knowledge"); await expireSession(page, b);
  const deniedDraft = page.waitForResponse(r => r.request().method() === "PATCH" && r.url().endsWith(`/drafts/${f.ref.draftId}`)); await page.getByRole("button", { name: "保存人工修订（并确认事实）", exact: true }).click(); expect((await deniedDraft).status()).toBe(401); await loginBack(page); await page.getByRole("button", { name: "步骤与原文", exact: true }).click(); await expect(page.getByLabel("部件名", { exact: true })).toHaveValue(BUFFER_CANARY + "knowledge");
  expect(s.wire.writes.filter(v => v === `PATCH /api/v1/items/${f.ref.itemId}/drafts/${f.ref.draftId}`)).toHaveLength(1); expect((await readDraft(api, b, f.ref)).review).toEqual(f.initial.review); await s.verify();
  const unsafe = await session(browser, b, contexts, 1440, false); await unsafe.page.goto(WEB + "/login?next=" + encodeURIComponent("//outside.invalid/path")); await unsafe.page.getByLabel("密码", { exact: true }).fill(PASSWORD); await unsafe.page.getByRole("button", { name: "登录", exact: true }).click(); await expect(unsafe.page).toHaveURL(WEB + "/");
  evidence("c5-auth-memory", { firstVisitOrdinary: true, wrongPasswordNotExpired: true, real401: 2, preservedSafeNext: next, editedItemId: item.id, ...f.ref, sameDocumentMemory: true, automaticReplay: 0, persistentCanaryWrites: 0, unsafeNextRejected: true });
});

test("C6 real model/provider/price business errors keep login and original/release reading without extra generation", async ({ browser }) => {
  const f = await draftFixture(api, b, true); expect(f.releaseId).toBeTruthy();
  const outcomes: Array<{ mode: string; status: number; code: unknown; reason: unknown }> = [];
  for (const mode of ["model", "price", "provider"] as const) {
    await b.stop(); await b.start({}); const material = await materialFixture(api, b); const s = await session(browser, b, contexts); const { page } = s;
    let proceed: () => void = () => {}; let held = false; const gate = new Promise<void>(resolve => { proceed = resolve; }); releases.push(proceed);
    const pattern = `**/api/v1/items/${material.seed.itemId}/estimates`;
    await page.route(pattern, async route => { if (route.request().method() !== "POST") { await route.fallback(); return; } held = true; await gate; await route.fallback(); });
    await page.goto(`${WEB}/items/${material.seed.itemId}/import/confirm?documentId=${material.seed.documentId}&preparationId=${material.preparationId}`); await expect.poll(() => held).toBe(true);
    await b.stop(); await b.start(mode === "model" ? { model: SECRET_CANARY } : mode === "price" ? { prices: false } : { providers: false }); const before = b.facts();
    const response = page.waitForResponse(r => r.request().method() === "POST" && r.url().endsWith(`/items/${material.seed.itemId}/estimates`)); proceed(); const rejected = await response; const body = await rejected.json(); expect(rejected.status()).not.toBe(401); expect(rejected.ok()).toBe(false); expect(JSON.stringify(body).includes(SECRET_CANARY)).toBe(false);
    if (mode === "model") { expect(rejected.status()).toBe(422); expect(body.error.details.reason).toBe("providerModelInvalid"); }
    await expect(page.getByRole("heading", { name: "无法获取报价", exact: true })).toBeVisible(); await expect(page).not.toHaveURL(/\/login/); expect((await page.context().request.get(b.base + "/api/v1/auth/session")).status()).toBe(200);
    await page.goto(`${WEB}/items/${f.ref.itemId}/documents/${f.ref.documentId}?page=1`); await expect(page.getByTestId("original-canvas")).toBeVisible(); await expect.poll(() => page.getByTestId("original-canvas").evaluate(e => (e as HTMLCanvasElement).width)).toBeGreaterThan(0);
    await page.goto(`${WEB}/items/${f.ref.itemId}/releases/${f.releaseId}`); await expect(page.getByTestId(`reader-part-${f.initial.knowledge.knowledge.parts[0]!.id}`)).toBeVisible(); await expect(page).not.toHaveURL(/\/login/); expect(b.facts()).toEqual(before);
    expect((await page.locator("body").innerText()).includes(SECRET_CANARY)).toBe(false); outcomes.push({ mode, status: rejected.status(), code: body.error.code, reason: body.error.details?.reason });
  }
  evidence("c6-config-read", { ...f.ref, releaseId: f.releaseId, outcomes, noLogout: true, originalPixels: true, releaseReadable: true, additionalJobsAttemptsLedgerProviders: 0, secretEcho: false });
});
