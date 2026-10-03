/** Independent PC05C API assertions. Author only until explicit root-frozen GO. */
import { expect, request, test, type APIRequestContext } from "@playwright/test";
import { apiLogin } from "./helpers";
import { evidence, PASSWORD, Pc05cBackend } from "./qa-pc05c-backend";
import { cloneActivityJobs, detail } from "./qa-pc05c-jobs";
import { draftFixture, jobFixture } from "./qa-pc05c-fixture";
let b: Pc05cBackend, api: APIRequestContext;
test.beforeEach(async () => { b = new Pc05cBackend(); await b.start(); api = await request.newContext(); await apiLogin(api, b.base, PASSWORD); });
test.afterEach(async () => { await api?.dispose(); await b?.cleanup(); });

test("C1 real engine waiting and Retry-After use safety count, preserve poll count, exhaust exactly MAX5", async () => {
  b.fixture.taskReplies.push({ kind: "waiting" }, { kind: "waiting" }, ...[7, 3, 1, 1, 1, 1].map(after => ({ kind: "retry" as const, after })));
  const f = await jobFixture(api, b); const observed: Array<{ number: number; limit: number; pollCount: number; nextAt: number; retryAt: number; after: number }> = [];
  await expect.poll(async () => {
    const d = await detail(api, b, f.jobId); const s = d.stages.find(s => s.stageKind === "tripo_poll"); if (!s) return false;
    if (s.status === "retry_wait" && s.safeRetry && !observed.some(o => o.number === s.safeRetry!.number)) {
      const last = b.fixture.observed.filter(o => o.kind === "retry").at(-1); if (!last || last.after === undefined || !s.nextRunAt) throw new Error("Real Retry-After evidence missing");
      observed.push({ number: s.safeRetry.number, limit: s.safeRetry.limit, pollCount: s.pollCount, nextAt: Date.parse(s.nextRunAt), retryAt: last.at, after: last.after });
      expect(s.safeRetry.number).toBe(s.attemptCount); expect(s.safeRetry.limit).toBe(5); expect(s.pollCount).toBe(2);
      expect(Date.parse(s.nextRunAt) - last.at).toBeGreaterThanOrEqual(last.after * 1000 - 500); expect(Date.parse(s.nextRunAt) - last.at).toBeLessThan(last.after * 1000 + 2000);
    }
    return s.status === "failed";
  }, { timeout: 90000, intervals: [100] }).toBe(true);
  const final = (await detail(api, b, f.jobId)).stages.find(s => s.stageKind === "tripo_poll")!;
  expect(observed.map(v => v.number)).toEqual([1, 2, 3, 4, 5]); expect(final.attemptCount).toBe(5); expect(final.pollCount).toBe(2); expect(final.safeRetry).toBeNull(); expect(final.nextRunAt).toBeNull();
  expect(b.fixture.counts.submit).toBe(1); expect(b.fixture.observed.filter(o => o.kind === "retry")).toHaveLength(6);
  evidence("c1-real-retry", { jobId: f.jobId, observed, final: { status: final.status, attemptCount: final.attemptCount, pollCount: final.pollCount, safeRetry: final.safeRetry }, wire: b.fixture.counts });
});

test("C2 real activity COUNT includes all four states beyond page one and GET is read-only", async () => {
  const f = await draftFixture(api, b); const ids = cloneActivityJobs(b, f.ref.jobId);
  const anon = await request.newContext(); expect((await anon.get(b.base + "/api/v1/jobs/activity")).status()).toBe(401); await anon.dispose();
  const before = b.snapshot(); const r = await api.get(b.base + "/api/v1/jobs/activity"); expect(r.status()).toBe(200); expect((await r.json()).data).toEqual({ active: 28 }); expect(b.snapshot()).toEqual(before);
  const list = await api.get(b.base + "/api/v1/jobs?limit=20"); expect(list.status()).toBe(200); const payload = await list.json(); expect(payload.data).toHaveLength(20); expect(payload.nextCursor).toBeTruthy();
  b.db("UPDATE jobs SET status='succeeded',revision=revision+1 WHERE id=?", [ids.active[0]]); expect((await (await api.get(b.base + "/api/v1/jobs/activity")).json()).data).toEqual({ active: 27 });
  b.db("UPDATE jobs SET status='cancelled',revision=revision+1 WHERE status IN ('queued','running','retry_wait','waiting_provider')"); expect((await (await api.get(b.base + "/api/v1/jobs/activity")).json()).data).toEqual({ active: 0 });
  b.db("UPDATE jobs SET status='queued',revision=revision+1 WHERE id=?", [ids.active[0]]); expect((await (await api.get(b.base + "/api/v1/jobs/activity")).json()).data).toEqual({ active: 1 });
  evidence("c2-activity", { fixture: "Private owned DB clones of a real public-HTTP job; no stages, used only for aggregate read contract", ...ids, counts: [28, 27, 0, 1], pageSize: 20, zeroWriteDigest: before.sha256, wire: b.fixture.counts });
});
