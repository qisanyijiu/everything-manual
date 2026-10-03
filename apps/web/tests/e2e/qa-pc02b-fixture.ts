/** Real HTTP draft/review/publish fixture. Preparation input is a declared format fixture, not PDF.js extraction. */
import { randomUUID } from "node:crypto";
import { expect, type APIRequestContext, type APIResponse } from "@playwright/test";
import type { components } from "../../src/api/generated";
import { apiLogin, fetchPhotos, seedItemWithDocument, seedPhoto, seedReadyPreparation } from "./helpers";
import { JOB_LIMITS, MODEL_PRESET, waitForJob } from "./job-recovery-harness";
import { Pc02bQaBackend, PASSWORD } from "./qa-pc02b-backend";

export type Patch = components["schemas"]["DraftPatchRequest"];
export type DraftDto = components["schemas"]["DraftDto"];
export interface Entity { id: string; name?: string; title?: string; label?: string; description?: string; orderedActions?: string[]; value?: string; evidence: { documentId: string; pageNumber: number }[] }
export interface Hotspot { id: string; partId: string; status: string; anchor: { modelRevisionId: string; modelSha256: string; positionLocal: number[] } | null }
export interface Knowledge { model: { revisionId: string; assetId: string; sha256: string; validationState: string }; knowledge: { parts: Entity[]; steps: Entity[]; specs: Entity[] }; hotspots: Hotspot[]; stepPoses?: Record<string, unknown> }
export interface Review { entities: Record<string, { reviewStatus?: string; userEdited?: Record<string, unknown> | null; textOnly?: boolean }>; modelReview?: { loaded: boolean; userConfirmed: boolean; modelRevisionId: string; modelSha256: string } | null }
export interface DraftRef { itemId: string; draftId: string; documentId: string; jobId: string }
export interface DraftView { dto: DraftDto; etag: string; knowledge: Knowledge; review: Review }

export async function readDraft(api: APIRequestContext, backend: Pc02bQaBackend, ref: Pick<DraftRef, "itemId" | "draftId">): Promise<DraftView> {
  const response = await api.get(`${backend.base}/api/v1/items/${ref.itemId}/drafts/${ref.draftId}`); expect(response.status()).toBe(200);
  const dto = (await response.json()).data as DraftDto; const etag = response.headers()["etag"]; if (!etag) throw new Error("Missing actual draft ETag");
  const knowledge = dto.knowledge as Knowledge; expect(knowledge.model.validationState).toBe("validated");
  const review = (dto.review ?? { entities: {} }) as Review; review.entities ??= {};
  return { dto, etag, knowledge, review };
}
export async function seedDraft(api: APIRequestContext, backend: Pc02bQaBackend, title: string) {
  const seed = await seedItemWithDocument(api, backend.base, PASSWORD, "sample-manual-text.pdf", title);
  const csrf = await apiLogin(api, backend.base, PASSWORD);
  // Deliberately reuse stored format fixture only for generation setup; no extraction claim.
  const preparationId = await seedReadyPreparation(api, backend.base, csrf, seed);
  await seedPhoto(api, backend.base, csrf, seed.itemId, "front", "sample-photo-front.jpg");
  await seedPhoto(api, backend.base, csrf, seed.itemId, "left", "sample-photo-left.png");
  const photoIds = (await fetchPhotos(api, backend.base, seed.itemId)).map(p => p.id);
  const estimate = await api.post(`${backend.base}/api/v1/items/${seed.itemId}/estimates`, { headers: { "x-csrf-token": csrf }, data: { preparationId, photoIds, modelPreset: MODEL_PRESET } }); expect(estimate.status()).toBe(201);
  const quoteId = (await estimate.json()).data.id as string;
  expect((await api.post(`${backend.base}/api/v1/items/${seed.itemId}/estimates/${quoteId}/confirm`, { headers: { "x-csrf-token": csrf } })).status()).toBe(200);
  const response = await api.post(`${backend.base}/api/v1/items/${seed.itemId}/jobs`, { headers: { "x-csrf-token": csrf, "idempotency-key": `qa-pc02b-${randomUUID()}` }, data: { quoteId, preparationId, photoIds, limits: JOB_LIMITS } }); expect(response.status()).toBe(202);
  const jobId = (await response.json()).data.id as string;
  const job = await waitForJob(api, backend.base, jobId, v => v.status === "succeeded" && v.draftId !== null, "PC02B local fixture draft", 90000);
  if (!job.draftId) throw new Error("Missing generated draft");
  const ref: DraftRef = { itemId: seed.itemId, documentId: seed.documentId, draftId: job.draftId, jobId };
  const initial = await readDraft(api, backend, ref); expect(initial.knowledge.knowledge.parts.length).toBeGreaterThan(0); expect(initial.knowledge.knowledge.steps.length).toBeGreaterThan(0); expect(initial.knowledge.knowledge.specs.length).toBeGreaterThan(0);
  return { ref, initial, csrf, baseline: businessCounts(backend) };
}
export async function patch(api: APIRequestContext, backend: Pc02bQaBackend, ref: DraftRef, csrf: string, body: Patch, etag?: string): Promise<APIResponse> {
  return api.patch(`${backend.base}/api/v1/items/${ref.itemId}/drafts/${ref.draftId}`, { headers: { "x-csrf-token": csrf, "if-match": etag ?? (await readDraft(api, backend, ref)).etag }, data: body });
}
export async function publish(api: APIRequestContext, backend: Pc02bQaBackend, ref: DraftRef, csrf: string, etag: string, key = `qa-pc02b-publish-${randomUUID()}`) {
  return api.post(`${backend.base}/api/v1/items/${ref.itemId}/drafts/${ref.draftId}/publish`, { headers: { "x-csrf-token": csrf, "if-match": etag, "idempotency-key": key } });
}
export function entities(view: DraftView) { return [...view.knowledge.knowledge.parts, ...view.knowledge.knowledge.steps, ...view.knowledge.knowledge.specs]; }
export function businessCounts(backend: Pc02bQaBackend) { return { providers: { ...backend.fixture.counts }, db: backend.db("SELECT (SELECT count(*) FROM jobs) AS jobs,(SELECT count(*) FROM provider_attempts) AS attempts,(SELECT count(*) FROM cost_ledger) AS ledger") }; }
export function releaseCount(backend: Pc02bQaBackend, ref: DraftRef): number { return backend.db("SELECT count(*) AS count FROM manual_releases WHERE item_id=?", [ref.itemId])[0].count as number; }
export async function makePublishable(api: APIRequestContext, backend: Pc02bQaBackend, ref: DraftRef, csrf: string, except?: string) {
  const current = await readDraft(api, backend, ref); const changes: NonNullable<Patch["entities"]> = {};
  for (const e of entities(current)) if (e.id !== except) changes[e.id] = { reviewStatus: "confirmed" };
  if (Object.keys(changes).length) expect((await patch(api, backend, ref, csrf, { entities: changes })).status()).toBe(200);
  const model = current.knowledge.model;
  expect((await patch(api, backend, ref, csrf, { hotspots: { upsert: current.knowledge.knowledge.parts.map((p, i) => ({ partId: p.id, status: "confirmed", anchor: { modelRevisionId: model.revisionId, modelSha256: model.sha256, positionLocal: [0.3 - i * 0.2, 0.3, 1] } })) } })).status()).toBe(200);
  expect((await patch(api, backend, ref, csrf, { modelReview: { loaded: true, userConfirmed: true } })).status()).toBe(200);
  return readDraft(api, backend, ref);
}
/** Historical stale anchor in owned temporary DB; all subsequent reads/rebinds use real API. */
export async function installStale(api: APIRequestContext, backend: Pc02bQaBackend, ref: DraftRef, csrf: string) {
  const current = await readDraft(api, backend, ref); const part = current.knowledge.knowledge.parts[0]; if (!part) throw new Error("Missing fixture part");
  const existing = current.knowledge.hotspots.find(h => h.partId === part.id);
  expect((await patch(api, backend, ref, csrf, { hotspots: { upsert: [{ ...(existing ? { id: existing.id } : {}), partId: part.id, status: "confirmed", anchor: { modelRevisionId: current.knowledge.model.revisionId, modelSha256: current.knowledge.model.sha256, positionLocal: [0.2, 0.2, 1] } }] } })).status()).toBe(200);
  const bound = await readDraft(api, backend, ref); const h = bound.knowledge.hotspots[0]; if (!h?.anchor) throw new Error("Actual anchor missing"); h.status = "stale"; h.anchor.modelRevisionId = "01930000-0000-7000-8000-000000000087"; h.anchor.modelSha256 = "0".repeat(64);
  backend.db("UPDATE manual_drafts SET knowledge_json=?,revision=revision+1 WHERE id=?", [JSON.stringify(bound.knowledge), ref.draftId]);
  return { hotspotId: h.id, partId: part.id, historicalSqlFixture: true };
}
