/** Independent PC03B API scenarios; execution requires root's frozen frontend/binary. */
import { randomUUID } from "node:crypto";
import { expect, request, test, type APIRequestContext } from "@playwright/test";
import { apiLogin, seedPhoto, seedReadyPreparation } from "./helpers";
import { waitForJob } from "./job-recovery-harness";
import { Pc03bQaBackend, PASSWORD, saveEvidence } from "./qa-pc03b-backend";
import { bindOriginal, businessCounts, confirmQuote, immutableFacts, makePublishable, patch, publish, readDraft, readQuote, seedDraft, seedQuote, submitQuote, summaries } from "./qa-pc03b-fixture";

let b: Pc03bQaBackend, api: APIRequestContext, anonymous: APIRequestContext;
test.describe.configure({ mode: "default", timeout: 150000 });
test.beforeAll(async () => { b = new Pc03bQaBackend(); await b.start(); api = await request.newContext(); anonymous = await request.newContext(); });
test.afterAll(async () => { await api?.dispose(); await anonymous?.dispose(); await b?.cleanup(); });
async function readonly<T>(read: () => Promise<T>): Promise<T> { const before = b.snapshot(); const result = await read(); expect(b.snapshot()).toEqual(before); return result; }
function historyClone(table: "generation_snapshots" | "manual_drafts" | "manual_releases", source: string, changes: Record<string, string | number>) {
  const columns = (b.db(`PRAGMA table_info(${table})`) as { name: string }[]).map(v => v.name), values: unknown[] = [];
  const expressions = columns.map(c => { if (Object.hasOwn(changes, c)) { values.push(changes[c]); return "?"; } return `"${c}"`; });
  b.db(`INSERT INTO ${table}(${columns.map(c => `"${c}"`).join(",")}) SELECT ${expressions.join(",")} FROM ${table} WHERE id=?`, [...values, source]);
}

test("QA PC3-006 batch bounds, deduplication, all-or-error ownership and GET logical zero-write", async () => {
  const csrf = await apiLogin(api, b.base, PASSWORD), ids: string[] = [];
  for (let n = 0; n < 101; n++) { const r = await api.post(b.base + "/api/v1/items", { headers: { "x-csrf-token": csrf }, data: { name: `PC03B batch ${n}`, model: "Local canary" } }); expect(r.status()).toBe(201); ids.push((await r.json()).data.id); }
  const counts = businessCounts(b);
  await readonly(async () => {
    const rows = await summaries(api, b, ids.slice(0, 100)); expect(rows).toHaveLength(100); expect(new Set(rows.map(v => v.itemId)).size).toBe(100); for (const row of rows) expect(row.steps).toEqual({ basic: "complete", document: "missing", views: "missing", prepare: "missing", confirm: "missing" }); expect(rows.every(v => v.action === "addDocument" && v.steps.basic === "complete" && v.steps.confirm === "missing")).toBe(true);
    expect(await summaries(api, b, [ids[0]!, ids[0]!])).toHaveLength(1);
    for (const query of ["", "ids=", "ids=not-a-uuid", `ids=${ids.join(",")}`, `ids=${ids[0]}&ids=${ids[1]}`, `ids=${ids[0]}&unexpected=1`, `ids=${ids[0]}&documentId=invalid`, `ids=${ids[0]},${ids[1]}&documentId=${randomUUID()}`]) expect((await api.get(b.base + "/api/v1/items/summaries?" + query)).status()).toBe(422);
    expect((await api.get(b.base + `/api/v1/items/summaries?ids=${ids[0]},${randomUUID()}`)).status()).toBe(404);
    expect((await anonymous.get(b.base + `/api/v1/items/summaries?ids=${ids[0]}`)).status()).toBe(401);
  });
  const q = await seedQuote(api, b, "PC03B summary ownership");
  await readonly(async () => { expect((await api.get(b.base + `/api/v1/items/summaries?ids=${ids[0]}&documentId=${q.documentId}`)).status()).toBe(404); expect((await api.get(b.base + `/api/v1/items/${ids[0]}/estimates/${q.quote.id}`)).status()).toBe(404); });
  expect(businessCounts(b)).toEqual(counts);
  saveEvidence("api-batch", { actualItems: ids.length + 1, maxBatch: 100, deduplicated: true, strict422: true, anonymous401: true, mixedAndForeign404: true, logicalReadWriteDelta: 0 });
});

test("QA PC3-006 priority and deterministic history ties preserve an older published entry", async () => {
  const { ref, initial, csrf } = await seedDraft(api, b, "PC03B ranked history");
  const ready = await makePublishable(api, b, ref, csrf); const key = `qa-pc03b-release-${randomUUID()}`; const published = await publish(api, b, ref, csrf, ready.etag, key); expect(published.status()).toBe(201); let releaseId = (await published.json()).data.id as string;
  const afterPublish = await readDraft(api, b, ref); const afterSummary = (await readonly(() => summaries(api, b, [ref.itemId])))[0];
  saveEvidence("bug-PC3-002-published-summary", { ...ref, releaseId, beforePublishRevision: ready.dto.revision, afterPublishRevision: afterPublish.dto.revision, persistedRelease: b.db("SELECT id,draft_id,draft_revision FROM manual_releases WHERE id=?", [releaseId]), summary: afterSummary, source: "Actual HTTP generation, review and publish; no SQL fixture at this point" });
  expect(afterSummary).toMatchObject({ action: "readRelease", targetId: releaseId, latestReleaseId: releaseId });
  const firstRelease = { id: releaseId, facts: b.db("SELECT * FROM manual_releases WHERE id=?", [releaseId]) }; const replayBefore = b.snapshot();
  const replay = await publish(api, b, ref, csrf, ready.etag, key); expect(replay.status()).toBe(201); expect(replay.headers()["x-idempotent-replay"]).toBe("true"); expect((await replay.json()).data.id).toBe(releaseId); expect(b.snapshot()).toEqual(replayBefore);
  expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]).toMatchObject({ action: "readRelease", targetId: releaseId });
  expect((await patch(api, b, ref, csrf, { entities: { [initial.knowledge.knowledge.parts[0]!.id]: { reviewStatus: "needs_review" } } })).status()).toBe(200);
  expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]).toMatchObject({ action: "reviewDraft", targetId: ref.draftId, latestReleaseId: releaseId });
  const editedRevision = (await readDraft(api, b, ref)).dto.revision; const readyAgain = await makePublishable(api, b, ref, csrf); const republished = await publish(api, b, ref, csrf, readyAgain.etag); expect(republished.status()).toBe(201); releaseId = (await republished.json()).data.id as string; expect(releaseId).not.toBe(firstRelease.id);
  expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]).toMatchObject({ action: "readRelease", targetId: releaseId, latestReleaseId: releaseId }); expect(b.db("SELECT * FROM manual_releases WHERE id=?", [firstRelease.id])).toEqual(firstRelease.facts);
  // Isolated audit corruption is negative evidence only. Actual publication above remains the main gate.
  const audit = b.db("SELECT * FROM audit_events WHERE entity_type='manual_release' AND entity_id=? AND action='release_published'", [releaseId])[0] as Record<string, string | number | null>; expect(audit).toBeTruthy(); const metadata = JSON.parse(audit.metadata_json as string); const auditBaseline = b.snapshot();
  const variants: { name: string; column: string; value: string | number | null }[] = [
    { name: "missing-metadata", column: "metadata_json", value: null },
    { name: "wrong-item", column: "metadata_json", value: JSON.stringify({ ...metadata, itemId: randomUUID() }) },
    { name: "wrong-draft", column: "metadata_json", value: JSON.stringify({ ...metadata, draftId: randomUUID() }) },
    { name: "after-revision-string", column: "metadata_json", value: JSON.stringify({ ...metadata, draftRevisionAfterPublish: String(metadata.draftRevisionAfterPublish) }) },
    { name: "wrong-release-identity", column: "entity_id", value: randomUUID() },
    { name: "unrelated-time", column: "created_at", value: Number(audit.created_at) + 1 },
  ];
  for (const variant of variants) { try { b.db(`UPDATE audit_events SET ${variant.column}=? WHERE id=?`, [variant.value, audit.id]); expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]).toMatchObject({ action: "reviewDraft", targetId: ref.draftId, latestReleaseId: releaseId }); } finally { b.db(`UPDATE audit_events SET ${variant.column}=? WHERE id=?`, [audit[variant.column], audit.id]); } expect(b.snapshot()).toEqual(auditBaseline); }
  try { b.db("DELETE FROM audit_events WHERE id=?", [audit.id]); expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]).toMatchObject({ action: "reviewDraft", targetId: ref.draftId, latestReleaseId: releaseId }); } finally { const keys = Object.keys(audit); b.db(`INSERT INTO audit_events(${keys.map(k => `"${k}"`).join(",")}) VALUES (${keys.map(() => "?").join(",")})`, keys.map(k => audit[k])); } expect(b.snapshot()).toEqual(auditBaseline);
  expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]?.action).toBe("readRelease");
  saveEvidence("published-lifecycle-and-audit", { ...ref, firstReleaseId: firstRelease.id, releaseId, revisions: { before: ready.dto.revision, after: afterPublish.dto.revision, afterEdit: editedRevision, beforeRepublish: readyAgain.dto.revision, afterRepublish: (await readDraft(api, b, ref)).dto.revision }, replayNoWrites: true, firstReleaseUnchanged: true, invalidAuditCases: [...variants.map(v => v.name), "missing-audit-row"], getNoRepairWrites: true, missingAuditDoesNotGuessPlusOne: true });
  // Leave a real post-publication edit so the remaining independent priority/tie branches exercise review.
  expect((await patch(api, b, ref, csrf, { entities: { [initial.knowledge.knowledge.parts[0]!.id]: { reviewStatus: "needs_review" } } })).status()).toBe(200);
  expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]?.action).toBe("reviewDraft");
  // History-only SQL rows: copy a completed job identity with no stages/attempts, never submit to a provider.
  const columns = (b.db("PRAGMA table_info(jobs)") as { name: string }[]).map(v => v.name); const quoted = columns.map(c => `"${c}"`).join(",");
  const a = "00000000-0000-4000-8000-000000000001", z = "00000000-0000-4000-8000-000000000002";
  for (const id of [a, z]) { const expressions = columns.map(c => c === "id" ? "?" : c === "status" ? "'failed'" : c === "updated_at" ? "9000000000000" : `"${c}"`).join(","); b.db(`INSERT INTO jobs(${quoted}) SELECT ${expressions} FROM jobs WHERE id=?`, [id, ref.jobId]); }
  const before = businessCounts(b), stable = immutableFacts(b, ref.itemId);
  expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]).toMatchObject({ action: "handleJob", targetId: z, latestReleaseId: releaseId });
  b.db("UPDATE jobs SET updated_at=9000000000001 WHERE id=?", [a]); expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]?.targetId).toBe(a);
  b.db("UPDATE jobs SET status='succeeded' WHERE id IN (?,?)", [a, z]); b.db("UPDATE jobs SET status='waiting_provider' WHERE id=?", [z]);
  expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]).toMatchObject({ action: "viewJob", targetId: z, latestReleaseId: releaseId });
  b.db("UPDATE jobs SET status='succeeded' WHERE id=?", [z]);
  expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]?.action).toBe("reviewDraft");
  expect(businessCounts(b)).toEqual(before); expect(immutableFacts(b, ref.itemId)).toEqual(stable); expect((await readDraft(api, b, ref)).dto.id).toBe(ref.draftId);
  // Additional deterministic metadata ties only; never claim these SQL records were generated/published.
  const sourceSnapshot = b.db("SELECT snapshot_id FROM manual_drafts WHERE id=?", [ref.draftId])[0].snapshot_id as string;
  const drafts = [randomUUID(), randomUUID()].sort(), releases = [randomUUID(), randomUUID()].sort();
  for (const id of drafts) { const snapshot = randomUUID(); historyClone("generation_snapshots", sourceSnapshot, { id: snapshot }); historyClone("manual_drafts", ref.draftId, { id, snapshot_id: snapshot, updated_at: 9000000000000 }); }
  for (const id of releases) historyClone("manual_releases", releaseId, { id, created_at: 9000000000000 });
  expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]).toMatchObject({ action: "reviewDraft", targetId: drafts[1], latestReleaseId: releases[1] });
  b.db("UPDATE manual_drafts SET updated_at=9000000000001 WHERE id=?", [drafts[0]]); expect((await readonly(() => summaries(api, b, [ref.itemId])))[0]?.targetId).toBe(drafts[0]); expect(businessCounts(b)).toEqual(before);
  saveEvidence("api-priority", { ...ref, releaseId, sqlFixture: "Historical job/draft/release metadata cloned from real completed workflow; classification only, no generated or published history claim", tieWinner: z, timestampWinner: a, draftTieWinner: drafts[1], draftTimestampWinner: drafts[0], releaseTieWinner: releases[1], priority: ["handleJob", "viewJob", "reviewDraft", "readRelease"], oldReleasePreserved: true });
});

test("QA PC3-007/009 current document, preparation, photos, expiry and consumed quote stay server facts", async () => {
  const q = await seedQuote(api, b, "PC03B current facts"); const baseline = businessCounts(b);
  expect((await readonly(() => summaries(api, b, [q.itemId])))[0]?.steps).toEqual({ basic: "complete", document: "complete", views: "complete", prepare: "complete", confirm: "missing" });
  await confirmQuote(api, b, q.itemId, q.csrf, q.quote.id); expect((await summaries(api, b, [q.itemId]))[0]?.steps.confirm).toBe("complete");
  const second = await bindOriginal(api, b, q); const fresh = await summaries(api, b, [q.itemId], second.documentId); expect(fresh[0]?.steps.prepare).toBe("needsReview"); expect(fresh[0]?.steps.confirm).toBe("needsReview");
  expect((await summaries(api, b, [q.itemId], q.documentId))[0]?.steps.prepare).toBe("complete");
  const secondPrep = await seedReadyPreparation(api, b.base, q.csrf, second); expect((await summaries(api, b, [q.itemId], second.documentId))[0]?.preparationId).toBe(secondPrep);
  // Private clock fixture changes both the authoritative expiry and public frozen expiry consistently.
  const row = b.quoteClock(q.quote.id, 946684800000);
  expect((await readonly(() => summaries(api, b, [q.itemId], q.documentId)))[0]?.steps.confirm).toBe("needsReview");
  b.quoteClock(q.quote.id, row.expires_at, row);
  expect(businessCounts(b)).toEqual(baseline);
  const jobId = await submitQuote(api, b, q); await waitForJob(api, b.base, jobId, j => j.status === "succeeded", "PC03B consumed job before mutable changes", 90000);
  const frozen = immutableFacts(b, q.itemId); await seedPhoto(api, b.base, q.csrf, q.itemId, "right", "sample-photo-front.jpg");
  const item = await api.get(`${b.base}/api/v1/items/${q.itemId}`); expect((await api.patch(`${b.base}/api/v1/items/${q.itemId}`, { headers: { "x-csrf-token": q.csrf, "if-match": item.headers()["etag"]! }, data: { model: "PC03B changed current model" } })).status()).toBe(200);
  const restored = await readonly(() => readQuote(api, b, q.itemId, q.quote.id)); expect(restored.consumedJobId).toBe(jobId); expect(restored.inputIssue ?? null).toBeNull(); expect(immutableFacts(b, q.itemId)).toEqual(frozen);
  saveEvidence("api-current-facts", { itemId: q.itemId, oldDocument: q.documentId, newDocument: second.documentId, secondPrep, quoteId: q.quote.id, jobId, consumedWinsAfterCurrentModelAndPhotoChange: true, immutableSnapshotsUnchanged: true, quoteExpiryFixture: "private persisted expiry only" });
});
