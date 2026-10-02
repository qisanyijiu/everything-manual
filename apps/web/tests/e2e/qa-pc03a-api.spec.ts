/** Independent PC03A API acceptance. Run only after root's RD_READY freeze is supplied. */
import fs from "node:fs";
import path from "node:path";
import { expect, request, test, type APIRequestContext } from "@playwright/test";
import type { components } from "../../src/api/generated";
import { apiLogin, seedPhoto } from "./helpers";
import { fixturePath } from "./runtime";
import { Pc03aQaBackend, PASSWORD, digest, saveEvidence } from "./qa-pc03a-backend";

type Document = components["schemas"]["DocumentDto"];
type Preparation = components["schemas"]["PreparationDto"];
type Detail = components["schemas"]["PreparationDetailDto"];
type List = components["schemas"]["PreparationListResponse"];
type PagePut = components["schemas"]["PagePutRequest"];
type Asset = components["schemas"]["AssetDto"];
test.describe.configure({ mode: "serial", timeout: 120000 });
let backend: Pc03aQaBackend, api: APIRequestContext, anonymous: APIRequestContext, csrf: string;
let serial = 0;
const headers = () => ({ "x-csrf-token": csrf });
test.beforeAll(async () => { backend = new Pc03aQaBackend(); await backend.start(); api = await request.newContext(); anonymous = await request.newContext(); csrf = await apiLogin(api, backend.base, PASSWORD); });
test.afterEach(() => { expect(backend.fixture.counts).toEqual({ upload: 0, submit: 0, task: 0, manual: 0, cdn: 0 }); expect(backend.db("SELECT (SELECT count(*) FROM jobs) AS jobs,(SELECT count(*) FROM provider_attempts) AS attempts,(SELECT count(*) FROM cost_ledger) AS ledger")).toEqual([{ jobs: 0, attempts: 0, ledger: 0 }]); });
test.afterAll(async () => { await api?.dispose(); await anonymous?.dispose(); await backend?.cleanup(); });

async function upload(itemId: string, purpose: string, name: string, mimeType: string, buffer: Buffer): Promise<Asset> {
  const r = await api.post(`${backend.base}/api/v1/items/${itemId}/assets`, { headers: headers(), multipart: { purpose, file: { name, mimeType, buffer } } });
  expect(r.status()).toBe(201); const asset = (await r.json()).data as Asset; expect(asset.sha256).toBe(digest(buffer)); return asset;
}
async function document(itemId?: string, fixture = "sample-manual-text.pdf") {
  if (!itemId) { const r = await api.post(backend.base + "/api/v1/items", { headers: headers(), data: { name: `PC03A API QA ${++serial}`, model: "Isolated format fixture" } }); expect(r.status()).toBe(201); itemId = (await r.json()).data.id as string; }
  const bytes = fs.readFileSync(fixturePath(fixture)); const asset = await upload(itemId, "document", fixture, "application/pdf", bytes);
  const r = await api.post(`${backend.base}/api/v1/items/${itemId}/documents`, { headers: headers(), data: { sourceAssetId: asset.id, title: "PC03A API 原件" } }); expect(r.status()).toBe(201);
  return { itemId, doc: (await r.json()).data as Document };
}
async function create(doc: Document, fresh = true) {
  const r = await api.post(`${backend.base}/api/v1/documents/${doc.id}/preparations`, { headers: headers(), data: { sourceSha256: doc.sourceSha256, ...(fresh ? { createNew: true } : {}) } });
  expect([200, 201]).toContain(r.status()); return { status: r.status(), preparation: (await r.json()).data as Preparation };
}
async function detail(id: string) {
  const r = await api.get(`${backend.base}/api/v1/preparations/${id}`); expect(r.status()).toBe(200); const etag = r.headers()["etag"]; if (!etag) throw new Error("Missing preparation ETag"); return { data: (await r.json()).data as Detail, etag };
}
async function list(doc: Document, query = ""): Promise<List> { const r = await api.get(`${backend.base}/api/v1/documents/${doc.id}/preparations${query}`); expect(r.status()).toBe(200); return await r.json() as List; }
async function put(id: string, number: number, body: PagePut, etag?: string) { return api.put(`${backend.base}/api/v1/preparations/${id}/pages/${number}`, { headers: { ...headers(), ...(etag ? { "if-match": etag } : {}) }, data: body }); }
async function complete(id: string, pageCount: number, etag: string) { return api.post(`${backend.base}/api/v1/preparations/${id}/complete`, { headers: { ...headers(), "if-match": etag }, data: { pageCount } }); }

/** Real uploaded JPEG/text bytes test server format/ownership facts only, NOT PDF.js provenance. */
async function shape(itemId: string) {
  const image = await upload(itemId, "pageImage", "qa-format.jpg", "image/jpeg", fs.readFileSync(fixturePath("sample-photo-front.jpg")));
  const text = await upload(itemId, "pageText", "qa-format.txt", "text/plain", Buffer.from("PC03A synthetic API page-format fixture; not PDF extraction."));
  return { image, text, body: { imageAssetId: image.id, textAssetId: text.id, viewport: { width: 32, height: 32, rotation: 0 } } satisfies PagePut };
}
async function donor(itemId: string, doc: Document, sealed = false) {
  const assets = await shape(itemId); const created = await create(doc); const id = created.preparation.id;
  for (const n of [1, 2]) expect((await put(id, n, assets.body)).status()).toBe(200);
  if (sealed) expect((await complete(id, 2, (await detail(id)).etag)).status()).toBe(200);
  return { id, ...assets };
}
/** Deliberate isolated historic-row injection. Page assets remain real uploaded backend assets. */
function clone(sourceId: string, id: string, options: { state?: "preparing" | "ready"; pageCount?: number | null; version?: number | null; timestamp?: number; pages?: number } = {}) {
  const state = options.state ?? "preparing", timestamp = options.timestamp ?? 1000;
  backend.db("INSERT INTO preparations(id,document_id,source_sha256,state,page_count,client_derived,revision,created_at,updated_at,format_version) SELECT ?,document_id,source_sha256,?,?,?,revision,?,?,? FROM preparations WHERE id=?", [id, state, options.pageCount ?? (state === "ready" ? 2 : null), state === "ready" ? 1 : 0, timestamp, timestamp, options.version === undefined ? 1 : options.version, sourceId]);
  backend.db("INSERT INTO pages(preparation_id,page_number,text_asset_id,image_asset_id,viewport_json,created_at,updated_at) SELECT ?,page_number,text_asset_id,image_asset_id,viewport_json,created_at,updated_at FROM pages WHERE preparation_id=? AND page_number<=?", [id, sourceId, options.pages ?? 2]);
  return id;
}
function facts(id: string) { return { preparation: backend.db("SELECT * FROM preparations WHERE id=?", [id]), pages: backend.db("SELECT * FROM pages WHERE preparation_id=? ORDER BY page_number", [id]) }; }
async function noWriteRead<T>(read: () => Promise<T>) { const before = backend.snapshot(); const value = await read(); expect(backend.snapshot()).toEqual(before); return value; }

test("QA PC3-001/005 global recommendation spans 100-row SQL batches and all HTTP pages without writes", async () => {
  const { itemId, doc } = await document(); const source = await donor(itemId, doc); backend.db("UPDATE preparations SET updated_at=1 WHERE id=?", [source.id]);
  const readyA = clone(source.id, "qa-global-ready-a", { state: "ready", timestamp: 10 });
  const readyZ = clone(source.id, "qa-global-ready-z", { state: "ready", timestamp: 10 });
  const usefulA = clone(source.id, "qa-global-useful-a", { timestamp: 80000 });
  const usefulZ = clone(source.id, "qa-global-useful-z", { timestamp: 80000 });
  clone(source.id, "qa-global-one-page-newer", { timestamp: 90000, pages: 1 });
  // These rows have no pages, marked v1, newer than the useful/ready rows.
  const padding = Array.from({ length: 105 }, (_, i) => [`qa-global-empty-${String(i).padStart(3, "0")}`, doc.id, doc.sourceSha256, 100000 + i, 100000 + i]);
  backend.db(`INSERT INTO preparations(id,document_id,source_sha256,state,page_count,client_derived,revision,created_at,updated_at,format_version) VALUES ${padding.map(() => "(?,?,?,'preparing',NULL,0,1,?,?,1)").join(",")}`, padding.flat());
  const seen: string[] = []; let pages = 0;
  await noWriteRead(async () => {
    const first = await list(doc); expect(first.data).toHaveLength(20); expect(first.data.some(row => row.preparation.id === readyZ)).toBe(false); expect(first.recommendedPreparationId).toBe(readyZ); expect(first.recommended?.readiness.completedPages).toEqual([1, 2]);
    expect((await list(doc, "?limit=100")).data).toHaveLength(100);
    let cursor: string | null = null;
    do { const result = await list(doc, "?limit=7" + (cursor ? "&cursor=" + encodeURIComponent(cursor) : "")); expect(result.data.length).toBeLessThanOrEqual(7); expect(result.recommendedPreparationId).toBe(readyZ); for (const row of result.data) { expect(row.preparation.documentId).toBe(doc.id); seen.push(row.preparation.id); } cursor = result.nextCursor ?? null; pages++; expect(pages).toBeLessThan(30); } while (cursor);
    expect(seen).toHaveLength(111); expect(new Set(seen).size).toBe(111);
    expect((await list(doc, "?cursor=" + encodeURIComponent(`v1:preparations:${doc.id}:0:after-all`))).recommendedPreparationId).toBe(readyZ);
  });
  backend.db("UPDATE preparations SET state='preparing',page_count=NULL,client_derived=0 WHERE id IN (?,?)", [readyA, readyZ]);
  expect((await noWriteRead(() => list(doc, "?limit=1"))).recommendedPreparationId).toBe(usefulZ);
  backend.db("UPDATE preparations SET updated_at=80001 WHERE id=?", [usefulA]);
  expect((await noWriteRead(() => list(doc, "?limit=1"))).recommendedPreparationId).toBe(usefulA);
  saveEvidence("api-global-ranking", { itemId, documentId: doc.id, historyFixture: "SQL clones over real uploaded JPEG/text; API format semantics only", total: seen.length, httpPages: pages, readyWinner: readyZ, pageCountThenIdWinner: usefulZ, timestampWinner: usefulA, getWriteDelta: 0 });
});

test("QA PC3-003/005 legacy NULL, malformed page facts, SHA and asset ownership are never silently reused", async () => {
  const { itemId, doc } = await document(); const second = await document(itemId, "sample-manual-nonlatin.pdf"); const other = await document(); const source = await donor(itemId, doc);
  const foreign = await shape(other.itemId); const wrongPurpose = await upload(itemId, "photo", "qa-photo.jpg", "image/jpeg", fs.readFileSync(fixturePath("sample-photo-front.jpg")));
  const cases: { id: string; compatible: boolean; reason: string | null; count: number; version: string | null }[] = [];
  const add = (name: string, options: Parameters<typeof clone>[2], compatible: boolean, reason: string | null, count = 2, version: string | null = "v1") => { const id = clone(source.id, `qa-compat-${name}`, options); cases.push({ id, compatible, reason, count, version }); return id; };
  const legacy = add("legacy-v1", { version: null }, true, null);
  add("legacy-empty", { version: null, pages: 0 }, false, "unsupportedFormat", 0, null);
  add("future", { version: 2 }, false, "unsupportedFormat", 2, null);
  const mismatch = add("sha", {}, false, "sourceMismatch"); backend.db("UPDATE preparations SET source_sha256=? WHERE id=?", [second.doc.sourceSha256, mismatch]);
  const viewport = add("viewport", {}, false, "invalidPages", 1); backend.db("UPDATE pages SET viewport_json=NULL WHERE preparation_id=? AND page_number=1", [viewport]);
  const badRotation = add("rotation", {}, false, "invalidPages", 1); backend.db("UPDATE pages SET viewport_json=? WHERE preparation_id=? AND page_number=1", [JSON.stringify({ width: 32, height: 32, rotation: 45 }), badRotation]);
  const missing = add("image-null", {}, false, "missingAssets", 1); backend.db("UPDATE pages SET image_asset_id=NULL WHERE preparation_id=? AND page_number=1", [missing]);
  const purpose = add("image-purpose", {}, false, "missingAssets", 1); backend.db("UPDATE pages SET image_asset_id=? WHERE preparation_id=? AND page_number=1", [wrongPurpose.id, purpose]);
  const foreignImage = add("image-foreign", {}, false, "missingAssets", 1); backend.db("UPDATE pages SET image_asset_id=? WHERE preparation_id=? AND page_number=1", [foreign.image.id, foreignImage]);
  const foreignText = add("text-foreign", {}, false, "missingAssets", 1); backend.db("UPDATE pages SET text_asset_id=? WHERE preparation_id=? AND page_number=1", [foreign.text.id, foreignText]);
  add("ready-incomplete", { state: "ready", pages: 1 }, false, "incompleteReady", 1);
  const extra = add("ready-outside-total", { state: "ready", pageCount: 1 }, false, "invalidPages", 1);
  const beforeLegacy = facts(legacy); const before = backend.snapshot();
  const response = await list(doc, "?limit=100");
  for (const c of cases) { const row = response.data.find(r => r.preparation.id === c.id); expect(row, c.id).toBeDefined(); expect(row!.readiness).toMatchObject({ compatible: c.compatible, reason: c.reason, completedPageCount: c.count, formatVersion: c.version }); if (!c.compatible) { expect(row!.readiness.explanation).toBeTruthy(); expect(response.recommendedPreparationId).not.toBe(c.id); } expect((await detail(c.id)).data.readiness).toEqual(row!.readiness); }
  expect((await list(second.doc)).data).toEqual([]); expect((await list(other.doc)).data).toEqual([]); expect(backend.snapshot()).toEqual(before); expect(facts(legacy)).toEqual(beforeLegacy);
  expect(backend.db("SELECT format_version FROM preparations WHERE id=?", [legacy])).toEqual([{ format_version: null }]);
  expect((await detail(extra)).data.readiness.missingPages).toEqual([]);
  const photos = await Promise.all([seedPhoto(api, backend.base, csrf, itemId, "front", "sample-photo-front.jpg"), seedPhoto(api, backend.base, csrf, itemId, "left", "sample-photo-left.png")]);
  const invalidReady = cases.find(c => c.reason === "incompleteReady")!; const beforeEstimate = backend.snapshot();
  const estimate = await api.post(`${backend.base}/api/v1/items/${itemId}/estimates`, { headers: headers(), data: { preparationId: invalidReady.id, photoIds: photos.map(p => p.photoId), modelPreset: "tripo-h-v3.1-standard" } }); expect(estimate.status()).toBe(422); expect((await estimate.json()).error.details.reason).toBe("preparationIncompatible"); expect(backend.snapshot()).toEqual(beforeEstimate);
  const foreignEstimate = await api.post(`${backend.base}/api/v1/items/${other.itemId}/estimates`, { headers: headers(), data: { preparationId: source.id, photoIds: photos.map(p => p.photoId), modelPreset: "tripo-h-v3.1-standard" } }); expect(foreignEstimate.status()).toBe(404);
  saveEvidence("api-compatibility", { itemId, documentId: doc.id, otherDocumentId: second.doc.id, cases, getWriteDelta: 0, legacyFormatRemainsNull: true, incompatibleEstimateStatus: 422, crossItemEstimateStatus: 404, fixtureBoundary: "actual uploaded bytes with synthetic historic DB facts, not browser extraction" });
});

test("QA PC3-003 physical blob missing or truncated is incompatible and GET cannot repair it", async () => {
  const { itemId, doc } = await document(); const source = await donor(itemId, doc, true); const sha = source.image.sha256; const blob = path.join(backend.dataDir, "blobs", sha.slice(0, 2), sha); const bytes = fs.readFileSync(blob); const original = facts(source.id); const results = [];
  try {
    for (const fault of ["missing", "truncated"] as const) {
      if (fault === "missing") fs.unlinkSync(blob); else fs.writeFileSync(blob, bytes.subarray(0, Math.floor(bytes.length / 2)));
      const before = backend.snapshot(); const response = await list(doc); expect(response.recommendedPreparationId).toBeNull(); expect(response.data[0]?.readiness).toMatchObject({ compatible: false, reason: "missingAssets", completedPageCount: 0 }); expect((await detail(source.id)).data.readiness.reason).toBe("missingAssets"); expect(backend.snapshot()).toEqual(before); expect(facts(source.id)).toEqual(original); expect(fs.existsSync(blob)).toBe(fault !== "missing"); results.push({ fault, getWriteDelta: 0, readyUnchanged: true });
      fs.writeFileSync(blob, bytes);
    }
  } finally { fs.writeFileSync(blob, bytes); }
  expect((await noWriteRead(() => list(doc))).recommendedPreparationId).toBe(source.id); expect(digest(fs.readFileSync(blob))).toBe(sha); saveEvidence("api-physical-assets", { documentId: doc.id, preparationId: source.id, results, restoredSha256: sha });
});

test("QA PC3-001/005 cursor scope, strict queries, anonymous and CSRF denial retain database facts", async () => {
  const { itemId, doc } = await document(); const other = await document(itemId, "sample-manual-nonlatin.pdf"); await create(doc); await create(doc);
  const first = await list(doc, "?limit=1"); const cursor = first.nextCursor; expect(cursor).toBeTruthy();
  const before = backend.snapshot(); const statuses: Record<string, number> = {};
  for (const query of ["?limit=0", "?limit=101", "?limit=-1", "?limit=1.5", "?limit=oops", "?limit=1&limit=2", "?unknown=1", "?cursor=oops", "?cursor=x&cursor=y"]) { const r = await api.get(`${backend.base}/api/v1/documents/${doc.id}/preparations${query}`); expect(r.status(), query).toBe(422); statuses[query] = r.status(); }
  const cross = await api.get(`${backend.base}/api/v1/documents/${other.doc.id}/preparations?cursor=${encodeURIComponent(cursor!)}`); expect(cross.status()).toBe(422);
  expect((await api.get(`${backend.base}/api/v1/documents/01930000-0000-7000-8000-000000000093/preparations`)).status()).toBe(404);
  expect((await anonymous.get(`${backend.base}/api/v1/documents/${doc.id}/preparations`)).status()).toBe(401);
  expect((await api.post(`${backend.base}/api/v1/documents/${doc.id}/preparations`, { data: { sourceSha256: doc.sourceSha256, createNew: true } })).status()).toBe(403);
  expect((await api.post(`${backend.base}/api/v1/documents/${doc.id}/preparations`, { headers: headers(), data: { sourceSha256: other.doc.sourceSha256, createNew: true } })).status()).toBe(422);
  expect(backend.snapshot()).toEqual(before); saveEvidence("api-read-security", { documentId: doc.id, strictQueries: statuses, crossDocumentCursor: 422, missingDocument: 404, anonymous: 401, csrf: 403, sourceMismatch: 422, fullLogicalDigestUnchanged: true });
});

test("QA PC3-003/004 explicit createNew, default reuse, page idempotency and CAS preserve prior ready", async () => {
  const { itemId, doc } = await document(); const old = await donor(itemId, doc, true); const readyBefore = facts(old.id);
  const preparing = await create(doc); expect(preparing.status).toBe(201); const reuseBefore = backend.snapshot(); const reused = await create(doc, false); expect(reused.status).toBe(200); expect(reused.preparation.id).toBe(preparing.preparation.id); expect(backend.snapshot()).toEqual(reuseBefore);
  // Legacy empty unknown must survive an explicitly new record, with no migration-on-read.
  backend.db("UPDATE preparations SET format_version=NULL WHERE id=?", [preparing.preparation.id]); const unknownBefore = facts(preparing.preparation.id);
  const fresh = await create(doc); expect(fresh.status).toBe(201); expect(fresh.preparation.id).not.toBe(preparing.preparation.id); expect(backend.db("SELECT format_version FROM preparations WHERE id=?", [fresh.preparation.id])).toEqual([{ format_version: 1 }]); expect(facts(preparing.preparation.id)).toEqual(unknownBefore);
  expect((await put(fresh.preparation.id, 1, old.body)).status()).toBe(200); const baseline = await detail(fresh.preparation.id); const sameBefore = backend.snapshot();
  const same = await Promise.all([put(fresh.preparation.id, 1, old.body), put(fresh.preparation.id, 1, old.body)]); expect(same.map(r => r.status())).toEqual([200, 200]); expect(backend.snapshot()).toEqual(sameBefore);
  const changed: PagePut = { ...old.body, viewport: { width: 32, height: 32, rotation: 90 } };
  expect((await put(fresh.preparation.id, 1, changed, baseline.etag)).status()).toBe(200); const staleBefore = backend.snapshot(); expect((await put(fresh.preparation.id, 1, old.body, baseline.etag)).status()).toBe(412); expect((await complete(fresh.preparation.id, 1, baseline.etag)).status()).toBe(412); expect(backend.snapshot()).toEqual(staleBefore);
  const current = await detail(fresh.preparation.id); expect((await complete(fresh.preparation.id, 1, current.etag)).status()).toBe(200); const final = await detail(fresh.preparation.id); expect(final.data.state).toBe("ready"); const closedBefore = backend.snapshot(); expect((await put(fresh.preparation.id, 1, old.body, final.etag)).status()).toBe(422); expect((await complete(fresh.preparation.id, 1, final.etag)).status()).toBe(422); expect(backend.snapshot()).toEqual(closedBefore); expect(facts(old.id)).toEqual(readyBefore);
  saveEvidence("api-create-cas", { documentId: doc.id, previousReadyId: old.id, unknownId: preparing.preparation.id, newId: fresh.preparation.id, defaultReuse: true, explicitV1: true, samePageConcurrentStatuses: [200, 200], stalePutAndComplete: [412, 412], sealedWriteStatuses: [422, 422], previousReadyUnchanged: true, noJobOrCost: true });
});
