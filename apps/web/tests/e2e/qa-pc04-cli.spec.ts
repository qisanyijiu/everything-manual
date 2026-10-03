/** Authoring against the confirmed PC04 contract. Run only after root freezes both binaries. */
import fs from "node:fs";
import path from "node:path";
import { test, expect } from "@playwright/test";
import { randomUUID } from "node:crypto";
import { Pc04Harness, FAKE_CANARY, REMOTE_CANARY, SUBMIT, MANUAL, evidence, object, outputJson, sha, type CliResult, type Dict } from "./qa-pc04-harness";

async function refused(h: Pc04Harness, args?: string[], expectedCode?: string) {
  const before = h.snapshot(), requests = h.proxy.requests.length; const r = await h.cli(args);
  expect(r.code, "negative CLI exit").not.toBe(0); expect(r.signal).toBeNull();
  expect(r.stderr.includes("未执行")).toBe(true); expect(r.stderr.includes("未发起供应商请求")).toBe(true);
  if (expectedCode) expect(r.stderr.includes(`[${expectedCode}]`), "fixed refusal code").toBe(true);
  h.scan(r.stdout + r.stderr); expect(h.proxy.requests.length).toBe(requests); expect(h.snapshot()).toEqual(before);
  return { code: r.code, refusalCode: /\[([A-Za-z]+)\]/.exec(r.stderr)?.[1] ?? "argument", zeroCalls: true, zeroLogicalWrites: true };
}
async function authorize(h: Pc04Harness) { const p = await h.plan(); const b = h.budget(p); h.writeBudget(b); return { p, b }; }
function reportFacts(h: Pc04Harness, result: CliResult) {
  const report = outputJson(result); h.scan(result.stdout + result.stderr); const saved = h.savedReport();
  expect(saved.report).toEqual(report); h.scan(JSON.stringify(report));
  expect(report.realSupplierAcceptance).toBe("NOT_RUN"); expect(report["AC-042"]).toBe("NOT_RUN"); expect(report.T23).toBe("NOT_RUN");
  expect(report.automaticReviews).toBe(0); expect(report.automaticReleases).toBe(0);
  expect(fs.statSync(path.join(saved.dir, "report.json")).mode & 0o077).toBe(0);
  expect(h.facts()?.releases).toBe(0); return { report, dir: saved.dir };
}
function reportCosts(report: Dict) { return (report.costs as unknown[]).map(object); }
function submitRequests(h: Pc04Harness) { return h.proxy.requests.filter(r => r.path === SUBMIT).map(r => object(JSON.parse(r.body.toString()))); }

test("QA PC4-001 CLI parsing, restricted authorization and secret-like identifier rejection", async ({ request: api }) => {
  const h = new Pc04Harness(); try {
    await h.setup(api); const help = await h.cli(["--help"]); expect(help.code).toBe(0);
    for (const term of ["--plan", "--budget-file", "creditMinor", "usdMicros", "独立用户授权"]) expect(help.stdout.includes(term)).toBe(true);
    const { b } = await authorize(h); const results: unknown[] = [];
    results.push(await refused(h, ["--case", h.casePath], "budgetRequired"));
    results.push(await refused(h, ["--case", h.casePath, "--credential", FAKE_CANARY]));
    fs.writeFileSync(h.budgetPath, '{"invalid":"' + FAKE_CANARY + '"', { mode: 0o600 }); results.push(await refused(h, undefined, "budgetJson"));
    h.writeBudget({ ...b, limits: { ...b.limits, unknown: FAKE_CANARY } }); results.push(await refused(h, undefined, "budgetJson"));
    h.writeBudget(b, 0o644); results.push(await refused(h, undefined, "budgetFilePermissions"));
    h.writeBudget({ ...b, allowed: false }); results.push(await refused(h, undefined, "notAuthorized"));
    h.writeBudget({ ...b, expiresAt: "2000-01-01T00:00:00Z" }); results.push(await refused(h, undefined, "authorizationExpired"));
    h.writeBudget({ ...b, authorizationId: FAKE_CANARY }); results.push(await refused(h, undefined, "budgetFields"));
    h.writeBudget(b); h.writeCase({ ...h.case, caseId: FAKE_CANARY }); results.push(await refused(h, undefined, "caseFields"));
    h.writeCase({ ...h.case, instance: { ...h.case.instance, instanceId: FAKE_CANARY } }); results.push(await refused(h, undefined, "caseFields"));
    h.writeCase({ ...h.case, material: { ...h.case.material, unknown: FAKE_CANARY } }); results.push(await refused(h, undefined, "caseJson"));
    h.writeCase(); h.scan(help.stdout + help.stderr); expect(h.proxy.requests).toHaveLength(0);
    evidence("01-parsing", { checks: results, providerRequests: 0, canaryHits: 0 });
  } finally { await h.cleanup(); }
});

test("QA PC4-002 actual plan facts, stable hashes, asset bytes and same-version catalog binding", async ({ request: api }) => {
  const h = new Pc04Harness(); try {
    await h.setup(api); const before = h.snapshot(); const { p, b } = await authorize(h); expect(await h.plan()).toEqual(p); expect(h.snapshot()).toEqual(before);
    const quoteRow = h.db("SELECT input_hash FROM quotes WHERE id=?", [h.quote.id])[0]; expect(p.inputHash).toBe(quoteRow?.input_hash);
    expect(p.providers).toEqual(h.quote.providerConfig); expect(p.priceVersion).toBe(h.quote.priceVersion); expect(p.pageRange).toEqual(h.quote.pageRange); expect(p.modelPreset).toBe(h.case.generation.modelPreset); expect(p.parameters).toEqual(object(object(h.quote.sendScope).tripo).parameters);
    const amounts = object(h.quote.amounts); expect(p.requiredLimits).toEqual({ creditMinor: object(amounts.tripo).upperBoundMinor, usdMicros: object(amounts.manualAi).upperBoundMinor });
    expect(p.views.map(v => v.view)).toEqual(["front", "left"]); expect(p.views).toEqual(object(object(h.quote.sendScope).tripo).views); expect(p.pages).toHaveLength(1); expect(p.stages.filter(s => s.stageKind === "manual_extract")).toHaveLength(1);
    const results: unknown[] = []; h.writeBudget({ ...b, planHash: "0".repeat(64) }); results.push(await refused(h, undefined, "planChanged")); h.writeBudget(b);
    h.writeCase({ ...h.case, material: { ...h.case.material, sourceSha256: "f".repeat(64) } }); results.push(await refused(h)); h.writeCase();
    h.writeCase({ ...h.case, generation: { ...h.case.generation, tripo: { identity: "tripo", model: "different-model" } } }); results.push(await refused(h)); h.writeCase();
    h.writeCase({ ...h.case, material: { ...h.case.material, photos: h.case.material.photos.map((p, i) => i ? { ...p, view: "back" } : p) } }); results.push(await refused(h)); h.writeCase();
    const catalog = fs.readFileSync(h.catalogPath); fs.writeFileSync(h.catalogPath, catalog.toString().replace('credits = "30"', 'credits = "31"'));
    const altered = await h.plan(); expect(altered.priceVersion).toBe(p.priceVersion); expect(altered.planHash).not.toBe(p.planHash); results.push(await refused(h, undefined, "planChanged")); fs.writeFileSync(h.catalogPath, catalog);
    // Private corruption fixture changes bytes without changing catalogued SHA; no metadata success mock.
    const source = findBlob(h.dataDir, h.case.material.sourceSha256); const original = fs.readFileSync(source); const corrupted = Buffer.from(original); corrupted[corrupted.length - 1] = corrupted[corrupted.length - 1]! ^ 1;
    fs.writeFileSync(source, corrupted); results.push(await refused(h, undefined, "materialAsset")); fs.writeFileSync(source, original);
    expect(await h.plan()).toEqual(p); expect(h.proxy.requests).toHaveLength(0); h.scan(JSON.stringify(p));
    evidence("02-plan", { inputHash: p.inputHash, planHash: p.planHash, requiredLimits: p.requiredLimits, views: p.views, stageCount: p.stages.length, stable: true, checks: results, providerRequests: 0 });
  } finally { await h.cleanup(); }
});
function findBlob(root: string, name: string): string {
  const pending = [path.join(root, "blobs")]; while (pending.length) { const dir = pending.pop()!; for (const entry of fs.readdirSync(dir, { withFileTypes: true })) { const file = path.join(dir, entry.name); if (entry.isDirectory()) pending.push(file); else if (entry.isFile() && entry.name === name) return file; } } throw new Error("Owned fixture blob missing");
}

test("QA PC4-003 independent integer limits, missing catalog/config and PC06 model rejection", async ({ request: api }) => {
  const h = new Pc04Harness(); try {
    await h.setup(api); const { b } = await authorize(h); const checks: unknown[] = [];
    for (const currency of ["creditMinor", "usdMicros"] as const) {
      expect(b.limits[currency]).toBeGreaterThan(0); h.writeBudget({ ...b, limits: { ...b.limits, [currency]: b.limits[currency] - 1 } }); checks.push(await refused(h, undefined, currency === "creditMinor" ? "creditBudget" : "usdBudget"));
      for (const bad of [-1, 0.25, "1", 1e30]) { h.writeBudget({ ...b, limits: { ...b.limits, [currency]: bad } }); checks.push(await refused(h)); }
    }
    h.writeBudget(b); const config = fs.readFileSync(h.configPath); const catalog = fs.readFileSync(h.catalogPath);
    fs.unlinkSync(h.catalogPath); checks.push(await refused(h)); fs.writeFileSync(h.catalogPath, catalog);
    h.missingManualKey = true; checks.push(await refused(h, undefined, "providerUnavailable")); h.missingManualKey = false;
    fs.writeFileSync(h.configPath, config.toString().replace('model = "gpt-5-mini"', `model = "${FAKE_CANARY}"`)); checks.push(await refused(h, undefined, "providerModelInvalid")); fs.writeFileSync(h.configPath, config);
    h.writeBudget({ ...b, retryScopes: [{ stageKind: "manual_extract", batchIndex: 99, stageInputHash: "0".repeat(64), operation: "safeRetry", maxAdditionalAttempts: 1 }] }); checks.push(await refused(h, undefined, "retryScope"));
    expect(h.proxy.requests).toHaveLength(0); evidence("03-budget", { checks, providerRequests: 0, distinctCurrencyGate: true });
  } finally { await h.cleanup(); }
});

test("QA PC4-004/007 actual adapters, GLB/evidence/needs_review, replay and private canary-safe reports", async ({ request: api }) => {
  const h = new Pc04Harness(); try {
    await h.setup(api); const { p } = await authorize(h); h.proxy.remoteCanary = true;
    const r = await h.cli(); expect(r.code, "real local CLI generation exit").toBe(0); expect(r.stdout.includes("本机链路验证通过；真实供应商验收未运行")).toBe(true);
    const { report, dir } = reportFacts(h, r); expect(report.status).toBe("needs_review"); expect(report.planHash).toBe(p.planHash); expect(report.inputHash).toBe(p.inputHash); expect(report.maxInitialGenerations).toBe(1); expect(report.retryScopes).toEqual([]);
    expect((report.attempts as unknown[]).map(v => object(v).remoteTaskIdHash)).toContain(sha(JSON.stringify(REMOTE_CANARY)));
    const submits = submitRequests(h); expect(submits).toHaveLength(1); const body = submits[0]!;
    expect((body.inputs as unknown[]).map(v => Object.keys(object(v)))).toEqual([["front"], ["left"]]);
    expect(body).toMatchObject({ model: "v3.1-20260211", texture: true, pbr: true, texture_quality: "standard", geometry_quality: "standard", face_limit: 100000, quad: false, generate_parts: false });
    const manual = h.proxy.requests.filter(r => r.path === MANUAL); expect(manual).toHaveLength(p.batches.length);
    for (const request of manual) { const body = object(JSON.parse(request.body.toString())); expect(body.model).toBe("gpt-5-mini"); expect(body.store).toBe(false); expect(object(object(body.text).format)).toMatchObject({ type: "json_schema", strict: true }); expect([...request.body.toString().matchAll(/\[第 (\d+) 页\]/g)].map(x => Number(x[1]))).toEqual([1]); }
    const model = fs.readFileSync(path.join(dir, "model.glb")); expect(model.subarray(0, 4).toString()).toBe("glTF"); expect(model.readUInt32LE(4)).toBe(2); expect(model.readUInt32LE(8)).toBe(model.length);
    const knowledge = object(JSON.parse(fs.readFileSync(path.join(dir, "knowledge.json"), "utf8"))); expect(knowledge.status).toBe("needs_review");
    const entities = object(knowledge.knowledge); for (const name of ["parts", "steps", "specs"]) for (const value of entities[name] as unknown[]) { const proofs = object(value).evidence as unknown[]; expect(proofs.length).toBeGreaterThan(0); expect(proofs.map(v => object(v).pageNumber)).toContain(1); }
    for (const a of report.savedArtifacts as unknown[]) { const artifact = object(a); const file = path.join(dir, String(artifact.file)); expect(sha(fs.readFileSync(file))).toBe(artifact.sha256); expect(fs.statSync(file).mode & 0o077).toBe(0); }
    expect(h.db("SELECT status,review_json FROM manual_drafts WHERE id=?", [report.draftId])[0]).toMatchObject({ status: "needs_review" });
    expect(h.facts()?.jobs).toBe(1); const paidBefore = { submit: h.proxy.count(SUBMIT), manual: h.proxy.count(MANUAL) }, before = h.facts();
    const replay = await h.cli(); expect(replay.code).toBe(0); expect(replay.stdout.includes("恢复已有任务")).toBe(true); expect(outputJson(replay).jobId).toBe(report.jobId); expect(h.facts()).toEqual(before); expect({ submit: h.proxy.count(SUBMIT), manual: h.proxy.count(MANUAL) }).toEqual(paidBefore);
    h.scan(fs.readFileSync(h.casePath, "utf8"), false); h.scan(fs.readFileSync(h.budgetPath, "utf8"), false); h.scan(h.logs.join(""), false);
    evidence("04-success", { jobId: report.jobId, draftId: report.draftId, facts: before, paidRequests: paidBefore, artifactHash: sha(model), planned: report.plannedUpperBound, costs: report.costs, privateFiles: true, replaySameJob: true, realSupplierAcceptance: "NOT_RUN", canaryHits: 0 });
  } finally { await h.cleanup(); }
});

test("QA PC4-005 concurrent authorization and interrupted known remote task do not repurchase", async ({ request: api }) => {
  const h = new Pc04Harness(); try {
    await h.setup(api); await authorize(h); h.proxy.holdNext("/v3/tasks/"); const first = h.start();
    await expect.poll(() => h.proxy.held, { message: "actual remote poll held", timeout: 30000 }).toBe(true);
    expect(h.db("SELECT a.remote_task_id FROM provider_attempts a JOIN job_stages s ON s.id=a.stage_id WHERE s.stage_kind='tripo_submit'").some(a => typeof a.remote_task_id === "string")).toBe(true);
    const competing = await h.cli(); expect(competing.code).not.toBe(0); expect(competing.stderr.includes("[instanceBusy]")).toBe(true); h.scan(competing.stdout + competing.stderr);
    const submitBefore = h.proxy.count(SUBMIT); expect(submitBefore).toBe(1); first.child.kill("SIGKILL"); expect((await first.done).signal).toBe("SIGKILL"); h.proxy.release();
    const resumed = await h.cli(); expect(resumed.code).toBe(0); const { report } = reportFacts(h, resumed); expect(report.status).toBe("needs_review"); expect(h.facts()?.jobs).toBe(1); expect(h.proxy.count(SUBMIT)).toBe(submitBefore);
    evidence("05-concurrent-interrupted", { jobId: report.jobId, jobs: 1, initialPaidSubmit: submitBefore, finalPaidSubmit: h.proxy.count(SUBMIT), safeQueries: h.proxy.count("/v3/tasks/"), killedAfterRemoteIdStored: true, competingRefusal: "instanceBusy" });
  } finally { await h.cleanup(); }
});

test("QA PC4-005 unknown accepted response retains reservation and never spends optional retry", async ({ request: api }) => {
  const h = new Pc04Harness(); try {
    await h.setup(api); const { p, b } = await authorize(h); const stage = p.stages.find(s => s.stageKind === "tripo_submit")!;
    b.retryScopes = [{ ...stage, operation: "safeRetry", maxAdditionalAttempts: 1 }]; h.writeBudget(b); h.proxy.dropAcceptedOnce = SUBMIT;
    const failed = await h.cli(); expect(failed.code).not.toBe(0); const { report } = reportFacts(h, failed); expect(report.status).toBe("submission_unknown");
    const tripo = reportCosts(report).find(e => e.provider === "tripo")!; expect(tripo.actual).toBeNull(); expect(tripo.reserved).toBeGreaterThan(0); expect(["reserved", "unknown"]).toContain(tripo.state); expect(report.retryScopes).toEqual(b.retryScopes);
    const before = h.facts(), count = h.proxy.count(SUBMIT); expect(count).toBe(1); const rerun = await h.cli(); expect(rerun.code).not.toBe(0); expect(outputJson(rerun).jobId).toBe(report.jobId); expect(h.facts()).toEqual(before); expect(h.proxy.count(SUBMIT)).toBe(count);
    evidence("06-unknown", { jobId: report.jobId, facts: before, paidSubmit: count, reservation: tripo.reserved, actual: null, optionalRetryNotSpent: true });
  } finally { await h.cleanup(); }
});

test("QA PC4-006 failure, over-budget fact and expiry stop preserve honest partial results", async ({ request: api }) => {
  const checks: unknown[] = [];
  for (const variant of ["businessFailure", "invalidGlb", "costRisk", "expiredWhileHeld", "credentialEcho"] as const) {
    const h = new Pc04Harness(); try {
      await h.setup(api); const { b } = await authorize(h);
      if (variant === "businessFailure") h.proxy.fixture.state.submitMode = "business400";
      if (variant === "invalidGlb") h.proxy.fixture.state.modelMode = "truncated";
      if (variant === "costRisk") h.proxy.credits = 31;
      if (variant === "credentialEcho") h.proxy.echoCredential = true;
      let r: CliResult; let requestsAtExpiry: number | null = null;
      if (variant === "expiredWhileHeld") {
        b.expiresAt = new Date(Date.now() + 5000).toISOString(); h.writeBudget(b); h.proxy.holdNext("/v3/tasks/"); const running = h.start();
        await expect.poll(() => h.proxy.held, { timeout: 15000 }).toBe(true); await new Promise(resolve => setTimeout(resolve, Math.max(0, Date.parse(b.expiresAt) - Date.now() + 100))); requestsAtExpiry = h.proxy.requests.length; h.proxy.release(); r = await running.done;
      } else r = await h.cli();
      expect(r.code, variant + " must not claim success").not.toBe(0); const { report } = reportFacts(h, r); expect(report.status).not.toBe("needs_review");
      expect(h.proxy.count(SUBMIT)).toBe(1); expect(report.nextAction).toBeTruthy(); expect(report.notCompleted).toBeTruthy();
      if (variant === "costRisk") expect(report.budgetRisk).toBe(true);
      if (variant === "expiredWhileHeld") { expect(report.stoppedBy).toBe("authorizationExpired"); expect(h.proxy.requests.length).toBe(requestsAtExpiry); }
      if (variant === "invalidGlb") expect((report.savedAssets as unknown[]).length).toBeGreaterThan(0);
      if (variant === "credentialEcho") { expect(report.status).toBe("submission_unknown"); expect(reportCosts(report).find(c => c.provider === "tripo")?.actual).toBeNull(); expect(h.db("SELECT remote_task_id FROM provider_attempts").every(a => a.remote_task_id === null)).toBe(true); }
      checks.push({ variant, status: report.status, stoppedBy: report.stoppedBy, budgetRisk: report.budgetRisk, costs: report.costs, savedAssets: report.savedAssets, paidSubmit: h.proxy.count(SUBMIT), facts: h.facts() });
    } finally { await h.cleanup(); }
  }
  evidence("07-failures", { checks, realSupplierAcceptance: "NOT_RUN" });
});

test("QA PC4-006 exact stage scope and unrelated queued-job isolation", async ({ request: api }) => {
  const checks: unknown[] = [];
  for (const variant of ["unrelatedJob", "stageHashMutation", "qualityMutation"] as const) {
    const h = new Pc04Harness(); try {
      await h.setup(api); await authorize(h); h.proxy.holdNext("/v3/files"); const running = h.start();
      await expect.poll(() => h.proxy.held, { message: "first real upload held", timeout: 30000 }).toBe(true);
      const job = h.db("SELECT * FROM jobs")[0]!; const stageId = randomUUID(), otherJob = randomUUID(); let trap: Dict[] = [];
      if (variant === "unrelatedJob") {
        // Deliberate private SQL trap: another queued job sharing immutable inputs.
        // It has no business request/authorization; its exact stored rows must not move.
        h.db("INSERT INTO jobs(id,item_id,snapshot_id,status,revision,created_at,updated_at) VALUES(?,?,?,'queued',1,?,?)", [otherJob, job.item_id, job.snapshot_id, job.created_at, job.updated_at]);
        h.db("INSERT INTO job_stages(id,job_id,stage_kind,input_hash,status,created_at,updated_at) SELECT ?,?,'freeze_inputs',input_hash,'queued',created_at,updated_at FROM job_stages WHERE job_id=? AND stage_kind='freeze_inputs'", [stageId, otherJob, job.id]);
        trap = h.db("SELECT j.status AS job_status,j.revision,j.updated_at AS job_updated,s.* FROM jobs j JOIN job_stages s ON s.job_id=j.id WHERE j.id=?", [otherJob]); expect(trap).toHaveLength(1);
      } else if (variant === "stageHashMutation") {
        // Corrupt only this owned fixture's frozen stage identity while first upload is in flight.
        h.db("UPDATE job_stages SET input_hash=? WHERE job_id=? AND stage_kind='manual_extract'", ["0".repeat(64), job.id]);
      } else {
        const catalog = fs.readFileSync(h.catalogPath, "utf8"); fs.writeFileSync(h.catalogPath, catalog.replace('texture_quality = "standard"', 'texture_quality = "high"'));
      }
      h.proxy.release(); const r = await running.done; const { report } = reportFacts(h, r);
      if (variant === "unrelatedJob") {
        expect(r.code).toBe(0); expect(h.db("SELECT j.status AS job_status,j.revision,j.updated_at AS job_updated,s.* FROM jobs j JOIN job_stages s ON s.job_id=j.id WHERE j.id=?", [otherJob])).toEqual(trap);
        expect(h.proxy.count(SUBMIT)).toBe(1); expect(h.db("SELECT count(*) AS count FROM provider_attempts WHERE job_id=?", [otherJob])[0]?.count).toBe(0);
      } else { expect(r.code).not.toBe(0); expect(report.status).not.toBe("needs_review"); expect(h.proxy.count(SUBMIT)).toBe(0); }
      checks.push({ variant, status: report.status, stoppedBy: report.stoppedBy, paidSubmit: h.proxy.count(SUBMIT), unrelatedUnchanged: variant === "unrelatedJob" ? true : null, declaredPrivateSqlFixture: variant !== "qualityMutation" });
    } finally { await h.cleanup(); }
  }
  evidence("08-scope", { checks });
});
