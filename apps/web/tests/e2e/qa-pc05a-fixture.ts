import { randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { expect, type APIRequestContext } from "@playwright/test";
import type { components } from "../../src/api/generated";
import type { DraftView, DraftRef } from "./qa-pc03b-fixture";
import { apiLogin, fetchPhotos, seedItemWithDocument, seedPhoto, seedReadyPreparation } from "./helpers";
import { JOB_LIMITS, MODEL_PRESET, waitForJob } from "./job-recovery-harness";
import { OUT, PASSWORD, PYTHON, Pc05aBackend } from "./qa-pc05a-backend";
export type Item = components["schemas"]["ItemDto"];
type Patch = components["schemas"]["DraftPatchRequest"];
export interface ItemPage { data: Item[]; nextCursor: string | null }
export async function items(api: APIRequestContext, b: Pc05aBackend, params: Record<string, string | number | boolean> = {}): Promise<ItemPage> { const r = await api.get(b.base + "/api/v1/items", { params }); expect(r.status()).toBe(200); return r.json(); }
export async function createItem(api: APIRequestContext, b: Pc05aBackend, csrf: string, name: string, model = "fixture-model", archived = false): Promise<Item> {
  const r = await api.post(b.base + "/api/v1/items", { headers: { "x-csrf-token": csrf }, data: { name, model } }); expect(r.status()).toBe(201); let item = (await r.json()).data as Item;
  if (archived) { const v = await api.patch(`${b.base}/api/v1/items/${item.id}`, { headers: { "x-csrf-token": csrf, "if-match": r.headers()["etag"]! }, data: { archived: true } }); expect(v.status()).toBe(200); item = (await v.json()).data as Item; } return item;
}
export async function catalog(api: APIRequestContext, b: Pc05aBackend) {
  const csrf = await apiLogin(api, b.base, PASSWORD); const active: Item[] = [], archived: Item[] = [];
  const cases = [["QA5A 原文折叠椅", "hidden-model-Az19"], ["QA5A percent%target", "fixture"], ["QA5A under_score", "fixture"], ["QA5A percentZtarget", "fixture"], ["QA5A underXscore", "fixture"], ["QA5A café", "fixture"], ["QA5A CAFÉ", "fixture"], ["QA5A 中文说明桌", "fixture"]];
  for (const [name, model] of cases) active.push(await createItem(api, b, csrf, name!, model!));
  for (let i = active.length; i < 45; i++) active.push(await createItem(api, b, csrf, `QA5A catalog item ${String(i).padStart(2, "0")}`, `catalog-model-${i}`));
  for (let i = 0; i < 12; i++) archived.push(await createItem(api, b, csrf, `QA5A 原文折叠椅 archived ${i}`, "ARCHIVE-Az19", true));
  return { csrf, active, archived, target: active[0]! };
}
export const asciiLower = (v: string) => v.replace(/[A-Z]/g, c => c.toLowerCase());
export function expectedMatches(rows: Item[], q: string) { const query = asciiLower(q.trim()); return rows.filter(v => asciiLower(v.name).includes(query) || asciiLower(v.model).includes(query)).map(v => v.id).sort(); }
export async function allItems(api: APIRequestContext, b: Pc05aBackend, q: string, archived = false, limit = 7) { const result: Item[] = []; let cursor: string | null = null; let pages = 0; do { const page = await items(api, b, { q, archived, limit, ...(cursor ? { cursor } : {}) }); result.push(...page.data); cursor = page.nextCursor; expect(++pages).toBeLessThan(20); } while (cursor); return { items: result, pages }; }

async function draft(api: APIRequestContext, b: Pc05aBackend, ref: DraftRef): Promise<DraftView> { const r = await api.get(`${b.base}/api/v1/items/${ref.itemId}/drafts/${ref.draftId}`); expect(r.status()).toBe(200); const dto = (await r.json()).data; return { dto, etag: r.headers()["etag"]!, knowledge: dto.knowledge, review: dto.review ?? { entities: {} } }; }
async function patch(api: APIRequestContext, b: Pc05aBackend, ref: DraftRef, csrf: string, body: Patch) { const d = await draft(api, b, ref); const r = await api.patch(`${b.base}/api/v1/items/${ref.itemId}/drafts/${ref.draftId}`, { headers: { "x-csrf-token": csrf, "if-match": d.etag }, data: body }); expect(r.status()).toBe(200); }
async function review(api: APIRequestContext, b: Pc05aBackend, ref: DraftRef, csrf: string) {
  const d = await draft(api, b, ref), changes: NonNullable<Patch["entities"]> = {};
  for (const entity of [...d.knowledge.knowledge.parts, ...d.knowledge.knowledge.steps, ...d.knowledge.knowledge.specs]) changes[entity.id] = { reviewStatus: "confirmed" };
  await patch(api, b, ref, csrf, { entities: changes }); const model = d.knowledge.model;
  await patch(api, b, ref, csrf, { hotspots: { upsert: d.knowledge.knowledge.parts.map((part, i) => ({ partId: part.id, status: "confirmed", anchor: { modelRevisionId: model.revisionId, modelSha256: model.sha256, positionLocal: [0.3 - i * 0.2, 0.3, 1] } })) } });
  await patch(api, b, ref, csrf, { modelReview: { loaded: true, userConfirmed: true } }); return draft(api, b, ref);
}
async function publish(api: APIRequestContext, b: Pc05aBackend, ref: DraftRef, csrf: string) { const d = await draft(api, b, ref); const r = await api.post(`${b.base}/api/v1/items/${ref.itemId}/drafts/${ref.draftId}/publish`, { headers: { "x-csrf-token": csrf, "if-match": d.etag, "idempotency-key": randomUUID() } }); expect(r.status()).toBe(201); return (await r.json()).data as components["schemas"]["ReleaseDto"]; }
export async function twoVersions(api: APIRequestContext, b: Pc05aBackend) {
  const seed = await seedItemWithDocument(api, b.base, PASSWORD, "sample-manual-text.pdf", "QA5A published manual"); const csrf = await apiLogin(api, b.base, PASSWORD);
  const preparationId = await seedReadyPreparation(api, b.base, csrf, seed); // Declared format fixture, not PDF.js extraction.
  await seedPhoto(api, b.base, csrf, seed.itemId, "front", "sample-photo-front.jpg"); await seedPhoto(api, b.base, csrf, seed.itemId, "left", "sample-photo-left.png"); const photoIds = (await fetchPhotos(api, b.base, seed.itemId)).map(v => v.id);
  const estimate = await api.post(`${b.base}/api/v1/items/${seed.itemId}/estimates`, { headers: { "x-csrf-token": csrf }, data: { preparationId, photoIds, modelPreset: MODEL_PRESET } }); expect(estimate.status()).toBe(201); const quoteId = (await estimate.json()).data.id as string;
  expect((await api.post(`${b.base}/api/v1/items/${seed.itemId}/estimates/${quoteId}/confirm`, { headers: { "x-csrf-token": csrf } })).status()).toBe(200);
  const create = await api.post(`${b.base}/api/v1/items/${seed.itemId}/jobs`, { headers: { "x-csrf-token": csrf, "idempotency-key": randomUUID() }, data: { quoteId, preparationId, photoIds, limits: JOB_LIMITS } }); expect(create.status()).toBe(202); const jobId = (await create.json()).data.id as string;
  const job = await waitForJob(api, b.base, jobId, v => v.status === "succeeded" && v.draftId !== null, "PC05A actual local generation", 90000); if (!job.draftId) throw new Error("Missing real draft");
  const ref: DraftRef = { itemId: seed.itemId, documentId: seed.documentId, draftId: job.draftId, jobId };
  const first = await review(api, b, ref, csrf); const old = await publish(api, b, ref, csrf); const frozenOld = b.db("SELECT * FROM manual_releases WHERE id=?", [old.id]); const partId = first.knowledge.knowledge.parts[0]!.id;
  await patch(api, b, ref, csrf, { entities: { [partId]: { userEdited: { name: "QA5A second published part" }, reviewStatus: "confirmed" } } }); await review(api, b, ref, csrf); const latest = await publish(api, b, ref, csrf);
  expect(latest.id).not.toBe(old.id); expect(b.db("SELECT * FROM manual_releases WHERE id=?", [old.id])).toEqual(frozenOld);
  return { ref, csrf, old, latest, frozenOld, partId, preparationId, photoIds };
}
export async function makePending(api: APIRequestContext, b: Pc05aBackend, book: Awaited<ReturnType<typeof twoVersions>>) { await patch(api, b, book.ref, book.csrf, { entities: { [book.partId]: { userEdited: { name: "QA5A unpublished local change" } } } }); }
/** A second real localhost job, explicitly rejected by the fake provider; no DB status fabrication. */
export async function failedNewJob(api: APIRequestContext, b: Pc05aBackend, book: Awaited<ReturnType<typeof twoVersions>>) {
  const estimate = await api.post(`${b.base}/api/v1/items/${book.ref.itemId}/estimates`, { headers: { "x-csrf-token": book.csrf }, data: { preparationId: book.preparationId, photoIds: book.photoIds, modelPreset: MODEL_PRESET } }); expect(estimate.status()).toBe(201); const quoteId = (await estimate.json()).data.id as string;
  expect((await api.post(`${b.base}/api/v1/items/${book.ref.itemId}/estimates/${quoteId}/confirm`, { headers: { "x-csrf-token": book.csrf } })).status()).toBe(200);
  b.fixture.state.submitMode = "business400";
  try {
    const created = await api.post(`${b.base}/api/v1/items/${book.ref.itemId}/jobs`, { headers: { "x-csrf-token": book.csrf, "idempotency-key": randomUUID() }, data: { quoteId, preparationId: book.preparationId, photoIds: book.photoIds, limits: JOB_LIMITS } }); expect(created.status()).toBe(202); const id = (await created.json()).data.id as string;
    await waitForJob(api, b.base, id, v => v.status === "failed", "PC05A second local job fails explicitly", 90000); return id;
  } finally { b.fixture.state.submitMode = "success"; }
}
export async function summaries(api: APIRequestContext, b: Pc05aBackend, ids: string[]) { const r = await api.get(b.base + "/api/v1/items/summaries", { params: { ids: ids.join(",") } }); expect(r.status()).toBe(200); return (await r.json()).data as components["schemas"]["ItemSummaryDto"][]; }
export interface ZipFacts { releaseId: string; frozenManifestSha256: string; portableManifestSha256: string; assetHashes: Record<string, string>; entries: string[] }
export function inspectZip(file: string, releaseId: string): ZipFacts {
  // ZIP/frozen-byte regression only; no PDF visual parsing or original-text extraction.
  const script = "import sys,zipfile,json,hashlib,pathlib\nf,r=sys.argv[1:]\nwith zipfile.ZipFile(f) as z:\n assert z.testzip() is None\n names=z.namelist();assert len(names)==len(set(names))\n assert all(not pathlib.PurePosixPath(n).is_absolute() and '..' not in pathlib.PurePosixPath(n).parts and '\\\\' not in n for n in names)\n m=json.loads(z.read('manifest.json'));frozen=z.read('release/manifest.json');h=hashlib.sha256(frozen).hexdigest()\n assert m['schemaVersion']=='manual_release_export_v1' and m['release']['releaseId']==r and m['releaseManifest']['sha256']==h\n assets={}\n for e in m['files']:\n  b=z.read(e['path']);v=hashlib.sha256(b).hexdigest();assert v==e['sha256'];assets[e['path']]=v\n assert set(names)=={'manifest.json','release/manifest.json',*assets}\n m.pop('exportedAtMillis',None)\n print(json.dumps({'releaseId':r,'frozenManifestSha256':h,'portableManifestSha256':hashlib.sha256(json.dumps(m,sort_keys=True,separators=(',',':')).encode()).hexdigest(),'assetHashes':assets,'entries':names}))";
  const r = spawnSync(PYTHON, ["-c", script, file, releaseId], { encoding: "utf8", env: { PATH: process.env.PATH }, timeout: 10000 }); expect(r.status, "actual ZIP manifest/assets hash validation; raw content withheld").toBe(0); return JSON.parse(r.stdout) as ZipFacts;
}
export async function exportZip(api: APIRequestContext, b: Pc05aBackend, releaseId: string, name: string) { const r = await api.get(`${b.base}/api/v1/releases/${releaseId}/export`); expect(r.status()).toBe(200); expect(r.headers()["content-type"]).toContain("application/zip"); fs.mkdirSync(OUT, { recursive: true }); const file = path.join(OUT, name + ".zip"); fs.writeFileSync(file, await r.body()); return inspectZip(file, releaseId); }
