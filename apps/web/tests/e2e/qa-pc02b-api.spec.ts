import { expect, request, test, type APIRequestContext } from "@playwright/test";
import { fetchAsset } from "./helpers";
import { Pc02bQaBackend, digest, saveEvidence } from "./qa-pc02b-backend";
import { businessCounts, entities, installStale, makePublishable, patch, publish, readDraft, releaseCount, seedDraft } from "./qa-pc02b-fixture";

test.describe.configure({ mode: "serial", timeout: 150000 });
let backend: Pc02bQaBackend, api: APIRequestContext;
test.beforeAll(async () => { backend = new Pc02bQaBackend(); await backend.start(); api = await request.newContext(); });
test.afterAll(async () => { await api?.dispose(); await backend?.cleanup(); });

test("QA PC2-005/007/008 actual review fields are independent; known 422, old anchors and stale CAS cannot mutate", async () => {
  const { ref, initial, csrf, baseline } = await seedDraft(api, backend, "PC02B independent fields"); const part = initial.knowledge.knowledge.parts[0]!;
  const rejected = await publish(api, backend, ref, csrf, initial.etag); expect(rejected.status()).toBe(422); const knownError = await rejected.json(); const issues = knownError.error.details.issues as { code: string; entityId: string | null; entityKind: string | null }[];
  expect(issues.some(i => i.code === "knowledgeUnreviewed" && i.entityId === part.id && i.entityKind === "part")).toBe(true); expect(issues.some(i => i.code === "hotspotMissing" && i.entityId === part.id)).toBe(true); expect(issues.some(i => i.code === "modelReviewMissing")).toBe(true); expect(releaseCount(backend, ref)).toBe(0); expect((await readDraft(api, backend, ref)).dto).toEqual(initial.dto);
  expect((await patch(api, backend, ref, csrf, { entities: { [part.id]: { reviewStatus: "confirmed" } } }, initial.etag)).status()).toBe(200);
  const fact = await readDraft(api, backend, ref); expect(fact.dto.revision).toBe(initial.dto.revision + 1); expect(fact.review.entities[part.id]?.reviewStatus).toBe("confirmed"); expect(Object.keys(fact.review.entities)).toEqual([part.id]); expect(fact.review.modelReview ?? null).toBe(initial.review.modelReview ?? null); expect(fact.knowledge).toEqual(initial.knowledge);
  const oldRevision = fact.etag; expect((await patch(api, backend, ref, csrf, { modelReview: { loaded: true, userConfirmed: false } }, fact.etag)).status()).toBe(200); const loaded = await readDraft(api, backend, ref); expect(loaded.review.modelReview?.loaded).toBe(true); expect(loaded.review.modelReview?.userConfirmed).not.toBe(true); expect(loaded.review.entities).toEqual(fact.review.entities); expect(loaded.knowledge).toEqual(fact.knowledge);
  const stalePatch = await patch(api, backend, ref, csrf, { entities: { [part.id]: { userEdited: { name: "uncommitted stale edit" } } } }, oldRevision); const stalePublish = await publish(api, backend, ref, csrf, oldRevision); expect([stalePatch.status(), stalePublish.status()]).toEqual([412, 412]); expect((await readDraft(api, backend, ref)).dto).toEqual(loaded.dto); expect(releaseCount(backend, ref)).toBe(0);
  const history = await installStale(api, backend, ref, csrf); const beforeAnchor = await readDraft(api, backend, ref); const hotspot = beforeAnchor.knowledge.hotspots.find(h => h.id === history.hotspotId)!;
  const badAnchor = await patch(api, backend, ref, csrf, { hotspots: { upsert: [{ id: hotspot.id, partId: part.id, status: "confirmed", anchor: hotspot.anchor }] } }); expect(badAnchor.status()).toBe(422); expect((await readDraft(api, backend, ref)).dto).toEqual(beforeAnchor.dto);
  expect((await patch(api, backend, ref, csrf, { hotspots: { upsert: [{ id: hotspot.id, partId: part.id, status: "confirmed", anchor: { modelRevisionId: initial.knowledge.model.revisionId, modelSha256: initial.knowledge.model.sha256, positionLocal: [0.1, 0.2, 1] } }] } })).status()).toBe(200);
  const rebound = await readDraft(api, backend, ref); expect(rebound.knowledge.hotspots.find(h => h.id === hotspot.id)?.status).toBe("confirmed"); expect(rebound.review).toEqual(beforeAnchor.review); expect(rebound.knowledge.stepPoses).toEqual(beforeAnchor.knowledge.stepPoses); expect(businessCounts(backend)).toEqual(baseline);
  saveEvidence("api-independent-review", { ...ref, knownIssueCodes: issues.map(i => [i.code, i.entityKind, i.entityId]), knownRequestId: knownError.error.requestId, revisions: [initial.dto.revision, fact.dto.revision, loaded.dto.revision, rebound.dto.revision], staleStatuses: [stalePatch.status(), stalePublish.status()], badAnchorStatus: badAnchor.status(), history, noAdditionalBusiness: true });
});

test("QA PC2-006/008 actual complete review requires explicit publish and freezes release bytes", async () => {
  const { ref, csrf, baseline } = await seedDraft(api, backend, "PC02B explicit release"); const ready = await makePublishable(api, backend, ref, csrf); expect(releaseCount(backend, ref)).toBe(0);
  const key = `qa-pc02b-release-${ref.draftId}`; const response = await publish(api, backend, ref, csrf, ready.etag, key); expect(response.status()).toBe(201); const release = (await response.json()).data as { id: string; manifestAssetId: string; draftRevision: number }; expect(release.draftRevision).toBe(ready.dto.revision); expect(releaseCount(backend, ref)).toBe(1);
  const replay = await publish(api, backend, ref, csrf, ready.etag, key); expect(replay.status()).toBe(201); expect(replay.headers()["x-idempotent-replay"]).toBe("true"); expect((await replay.json()).data.id).toBe(release.id); expect(releaseCount(backend, ref)).toBe(1);
  const manifest = await fetchAsset(api, backend.base, release.manifestAssetId); const first = entities(ready)[0]!; expect((await patch(api, backend, ref, csrf, { entities: { [first.id]: { userEdited: { name: "QA post-release name" } } } })).status()).toBe(200);
  expect((await fetchAsset(api, backend.base, release.manifestAssetId)).bytes.equals(manifest.bytes)).toBe(true); expect(businessCounts(backend)).toEqual(baseline);
  saveEvidence("api-explicit-release", { ...ref, releaseId: release.id, releaseCount: 1, explicitStatus: response.status(), replayStatus: replay.status(), manifestAssetId: release.manifestAssetId, manifestSha256: digest(manifest.bytes), immutableAfterDraftEdit: true, noAdditionalBusiness: true });
});
