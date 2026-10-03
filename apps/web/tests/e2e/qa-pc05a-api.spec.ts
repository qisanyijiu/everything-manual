/** Independent PC05A HTTP checks; run only against the explicit root-frozen candidate. */
import { expect, request, test, type APIRequestContext } from "@playwright/test";
import { Pc05aBackend, evidence } from "./qa-pc05a-backend";
import { allItems, catalog, expectedMatches, items } from "./qa-pc05a-fixture";

let b: Pc05aBackend, api: APIRequestContext, data: Awaited<ReturnType<typeof catalog>>;
test.beforeAll(async () => { b = new Pc05aBackend(); await b.start(); api = await request.newContext(); data = await catalog(api, b); });
test.afterAll(async () => { await api?.dispose(); await b?.cleanup(); });

test("QA PC05A 1/6: server finds off-page literal name/model matches, stable scoped pagination", async () => {
  const before = b.snapshot(), facts = b.facts();
  const first = await items(api, b); expect(first.data).toHaveLength(20); expect(first.data.map(v => v.id)).not.toContain(data.target.id);
  const results = [];
  for (const archived of [false, true]) for (const q of ["原文折叠椅", "aZ19", "%", "_", "é", "É", "中文", "  QA5A  ", "", "   "]) {
    const rows = await allItems(api, b, q, archived);
    expect(rows.items.map(v => v.id).sort(), `literal query ${JSON.stringify(q)} archived=${archived}`).toEqual(expectedMatches(archived ? data.archived : data.active, q));
    expect(new Set(rows.items.map(v => v.id)).size).toBe(rows.items.length);
    expect(rows.items.every(v => Boolean(v.archivedAt) === archived)).toBe(true);
    results.push({ q, archived, count: rows.items.length, pages: rows.pages });
  }
  expect(b.snapshot()).toEqual(before);
  // Private fixture changes only creation timestamps to make the tie-break observable.
  // This does not stand in for search/create HTTP; those records were all created above by HTTP.
  b.db("UPDATE items SET created_at=1800000000000 WHERE id IN (?,?,?)", data.active.slice(0, 3).map(v => v.id));
  const tiedBefore = b.snapshot();
  const ordered = await allItems(api, b, "QA5A", false, 7);
  expect(ordered.items.map(v => v.id)).toEqual(b.db("SELECT id FROM items WHERE archived_at IS NULL ORDER BY created_at DESC,id DESC").map(v => v.id));
  expect(b.snapshot()).toEqual(tiedBefore); expect(b.facts()).toEqual(facts);
  evidence("01-literal-search", { results, targetOutsideFirstPage: true, firstPageCount: 20, tiedOrder: ordered.items.map(v => v.id), before, after: tiedBefore, tieFixture: "Three HTTP-created rows get equal created_at in owned DB; all subsequent GETs have identical logical digest", facts });
});

test("QA PC05A 2/6: cursor scope, 200 Unicode characters, legacy/invalid queries, auth and GET zero writes", async () => {
  const before = b.snapshot(), facts = b.facts();
  const first = await items(api, b, { q: "QA5A", limit: 7 }); expect(first.nextCursor).toBeTruthy(); const cursor = first.nextCursor!;
  const next = await items(api, b, { q: "  qa5A  ", limit: 7, cursor }); expect(next.data).toHaveLength(7); expect(next.data.some(v => first.data.some(f => f.id === v.id))).toBe(false);
  const failures: { query: string; field: string; status: number }[] = [];
  async function invalid(params: Record<string, string | number | boolean>, field: string) {
    const r = await api.get(b.base + "/api/v1/items", { params }); expect(r.status()).toBe(422); const body = await r.json(); expect(body.error.details.fields.some((v: { field: string }) => v.field === field)).toBe(true); expect(body.error.requestId).toBeTruthy(); failures.push({ query: new URL(r.url()).search, field, status: r.status() });
  }
  await invalid({ q: "catalog", cursor }, "cursor"); await invalid({ q: "QA5A", archived: true, cursor }, "cursor");
  await invalid({ q: "QA5A", cursor: cursor.replace("created-desc-id-desc", "created-asc-id-asc") }, "cursor");
  const last = first.data.at(-1)!; const legacy = `v1:items:active:${Date.parse(last.createdAt)}:${last.id}`;
  expect((await items(api, b, { cursor: legacy, limit: 7 })).data).toHaveLength(7); await invalid({ q: "QA5A", cursor: legacy }, "cursor");
  for (const q of ["a".repeat(200), "中".repeat(200), "😀".repeat(200)]) expect((await items(api, b, { q })).data).toEqual([]);
  for (const q of ["a".repeat(201), "中".repeat(201), "😀".repeat(201)]) await invalid({ q }, "q");
  await invalid({ cursor: "v1:jobs:anything" }, "cursor"); await invalid({ archived: "all" }, "archived"); await invalid({ limit: 0 }, "limit"); await invalid({ sort: "name" }, "sort");
  const duplicate = await api.get(b.base + "/api/v1/items?q=one&q=two"); expect(duplicate.status()).toBe(422); expect((await duplicate.json()).error.details.fields.some((v: { field: string }) => v.field === "q")).toBe(true);
  const anonymous = await request.newContext(); try { expect((await anonymous.get(b.base + "/api/v1/items?q=QA5A")).status()).toBe(401); } finally { await anonymous.dispose(); }
  expect(b.snapshot()).toEqual(before); expect(b.facts()).toEqual(facts);
  evidence("02-cursor-and-read-only", { failures, normalizedCursorAccepted: true, legacyOnlyWithoutQuery: true, acceptedLengths: [200, 200, 200], rejectedLengths: [201, 201, 201], before, after: b.snapshot(), facts });
});
