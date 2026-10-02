/** PC05B owns all state. Format pages are for draft setup only; B5 uses actual PDF.js. */
import { randomUUID } from "node:crypto";
import { expect, type APIRequestContext } from "@playwright/test";
import type { components } from "../../src/api/generated";
import type { DraftRef, DraftView } from "./qa-pc03b-fixture";
import { apiLogin, fetchPhotos, seedItemWithDocument, seedPhoto, seedReadyPreparation } from "./helpers";
import { JOB_LIMITS, MODEL_PRESET, waitForJob } from "./job-recovery-harness";
import { PASSWORD, Pc05bBackend } from "./qa-pc05b-backend";
export type { DraftRef, DraftView };
export type Item = components["schemas"]["ItemDto"];
type Patch = components["schemas"]["DraftPatchRequest"];
export async function createItem(api: APIRequestContext, b: Pc05bBackend, csrf: string, name = "PC05B owned item"): Promise<Item> {
  const r = await api.post(b.base + "/api/v1/items", { headers: { "x-csrf-token": csrf }, data: { name, model: "PC05B-local-only" } }); expect(r.status()).toBe(201); return (await r.json()).data;
}
export async function readItem(api: APIRequestContext, b: Pc05bBackend, itemId: string) {
  const r = await api.get(`${b.base}/api/v1/items/${itemId}`); expect(r.status()).toBe(200); return { item: (await r.json()).data as Item, etag: r.headers()["etag"]! };
}
export async function patchItem(api: APIRequestContext, b: Pc05bBackend, csrf: string, itemId: string, name: string) {
  const before = await readItem(api, b, itemId); const r = await api.patch(`${b.base}/api/v1/items/${itemId}`, { headers: { "x-csrf-token": csrf, "if-match": before.etag }, data: { name } }); expect(r.status()).toBe(200); return readItem(api, b, itemId);
}
export async function readDraft(api: APIRequestContext, b: Pc05bBackend, ref: DraftRef): Promise<DraftView> {
  const r = await api.get(`${b.base}/api/v1/items/${ref.itemId}/drafts/${ref.draftId}`); expect(r.status()).toBe(200); const dto = (await r.json()).data; return { dto, etag: r.headers()["etag"]!, knowledge: dto.knowledge, review: dto.review ?? { entities: {} } };
}
export async function patchDraft(api: APIRequestContext, b: Pc05bBackend, ref: DraftRef, csrf: string, body: Patch) {
  const d = await readDraft(api, b, ref); const r = await api.patch(`${b.base}/api/v1/items/${ref.itemId}/drafts/${ref.draftId}`, { headers: { "x-csrf-token": csrf, "if-match": d.etag }, data: body }); expect(r.status()).toBe(200); return readDraft(api, b, ref);
}
export async function draftFixture(api: APIRequestContext, b: Pc05bBackend, published = false) {
  const seed = await seedItemWithDocument(api, b.base, PASSWORD, "sample-manual-text.pdf", "PC05B draft input"); const csrf = await apiLogin(api, b.base, PASSWORD);
  const preparationId = await seedReadyPreparation(api, b.base, csrf, seed); // Explicit setup fixture: not B5 preparation acceptance.
  await seedPhoto(api, b.base, csrf, seed.itemId, "front", "sample-photo-front.jpg"); await seedPhoto(api, b.base, csrf, seed.itemId, "left", "sample-photo-left.png"); const photoIds = (await fetchPhotos(api, b.base, seed.itemId)).map(v => v.id);
  const estimate = await api.post(`${b.base}/api/v1/items/${seed.itemId}/estimates`, { headers: { "x-csrf-token": csrf }, data: { preparationId, photoIds, modelPreset: MODEL_PRESET } }); expect(estimate.status()).toBe(201); const quoteId = (await estimate.json()).data.id as string;
  expect((await api.post(`${b.base}/api/v1/items/${seed.itemId}/estimates/${quoteId}/confirm`, { headers: { "x-csrf-token": csrf } })).status()).toBe(200);
  const create = await api.post(`${b.base}/api/v1/items/${seed.itemId}/jobs`, { headers: { "x-csrf-token": csrf, "idempotency-key": randomUUID() }, data: { quoteId, preparationId, photoIds, limits: JOB_LIMITS } }); expect(create.status()).toBe(202); const jobId = (await create.json()).data.id as string;
  const job = await waitForJob(api, b.base, jobId, v => v.status === "succeeded" && v.draftId !== null, "PC05B actual localhost generation", 90000); if (!job.draftId) throw new Error("Missing actual draft");
  const ref: DraftRef = { itemId: seed.itemId, documentId: seed.documentId, draftId: job.draftId, jobId }; const initial = await readDraft(api, b, ref); let releaseId: string | null = null;
  if (published) {
    const entities: NonNullable<Patch["entities"]> = {}; for (const e of [...initial.knowledge.knowledge.parts, ...initial.knowledge.knowledge.steps, ...initial.knowledge.knowledge.specs]) entities[e.id] = { reviewStatus: "confirmed" };
    await patchDraft(api, b, ref, csrf, { entities }); const model = initial.knowledge.model;
    await patchDraft(api, b, ref, csrf, { hotspots: { upsert: initial.knowledge.knowledge.parts.map((part, i) => ({ partId: part.id, status: "confirmed", anchor: { modelRevisionId: model.revisionId, modelSha256: model.sha256, positionLocal: [0.3 - i * 0.2, 0.3, 1] } })) } });
    const ready = await patchDraft(api, b, ref, csrf, { modelReview: { loaded: true, userConfirmed: true } });
    const r = await api.post(`${b.base}/api/v1/items/${ref.itemId}/drafts/${ref.draftId}/publish`, { headers: { "x-csrf-token": csrf, "if-match": ready.etag, "idempotency-key": randomUUID() } }); expect(r.status()).toBe(201); releaseId = (await r.json()).data.id;
  }
  return { ref, csrf, initial, releaseId };
}
