/** Four independent UI groups using true localhost data and the explicit frozen Vite cwd. */
import fs from "node:fs";
import path from "node:path";
import { expect, request, test, type APIRequestContext, type Locator, type Page, type Route } from "@playwright/test";
import { apiLogin, loginViaUi } from "./helpers";
import { OUT, PASSWORD, WEB, Pc05aBackend, attach, evidence } from "./qa-pc05a-backend";
import { catalog, createItem, exportZip, failedNewJob, inspectZip, items, makePending, summaries, twoVersions } from "./qa-pc05a-fixture";

let b: Pc05aBackend, api: APIRequestContext, data: Awaited<ReturnType<typeof catalog>>, book: Awaited<ReturnType<typeof twoVersions>>;
test.beforeAll(async () => {
  test.setTimeout(180000); b = new Pc05aBackend(); await b.start(); api = await request.newContext(); data = await catalog(api, b); book = await twoVersions(api, b);
  // Put the published row near the bottom of a real first page to make return-position observable.
  for (let i = 0; i < 18; i++) await createItem(api, b, book.csrf, `QA5A newest filler ${i}`);
});
test.afterAll(async () => { await api?.dispose(); await b?.cleanup(); });
const search = (page: Page) => page.getByLabel("按名称或型号搜索", { exact: true });
const rows = (page: Page) => page.locator("[data-library-item]");
const row = (page: Page, id: string) => page.locator(`[data-library-item="${id}"]`);
const param = (page: Page, key: string) => new URL(page.url()).searchParams.get(key);
function listReads(reads: string[]) { return reads.filter(v => new URL(v, WEB).pathname === "/api/v1/items"); }
async function settled(page: Page) { await expect(page.locator(".workflow-actions__status").filter({ hasText: "正在读取处理状态" })).toHaveCount(0); }
async function open(page: Page, route = "/") { const wire = await attach(page, b); await loginViaUi(page, WEB, PASSWORD); await page.goto(WEB + route); await expect(search(page)).toBeVisible(); await expect(rows(page).first()).toBeVisible(); await settled(page); return wire; }
async function submit(page: Page, q: string) { await search(page).fill(q); await search(page).press("Enter"); await expect.poll(() => param(page, "q")).toBe(q.trim() || null); }
async function ids(page: Page) { return rows(page).evaluateAll(elements => elements.map(e => (e as HTMLElement).dataset.libraryItem!)); }
function gate() { let release!: () => void; const promise = new Promise<void>(resolve => { release = resolve; }); return { promise, release }; }
async function forward(route: Route) { const u = new URL(route.request().url()); return route.fetch({ url: b.base + u.pathname + u.search }); }
async function tabTo(page: Page, control: Locator) { for (let i = 0; i < 100; i++) { await page.keyboard.press("Tab"); if (await control.evaluate(e => e === document.activeElement)) { const focus = await control.evaluate(e => ({ width: getComputedStyle(e).outlineWidth, style: getComputedStyle(e).outlineStyle })); expect(focus.style).not.toBe("none"); expect(Number.parseFloat(focus.width)).toBeGreaterThanOrEqual(2); return; } } throw new Error("Action not reachable by Tab"); }

test("QA PC05A 3/6: explicit server search, pagination commit, scope reset and URL history/fresh refresh", async ({ page, browser }) => {
  const historyFacts: unknown[] = [];
  const recordHistory = async (label: string) => { historyFacts.push({ label, ...await page.evaluate(() => ({ href: location.href, historyLength: history.length, state: history.state, input: (document.querySelector("#library-search") as HTMLInputElement | null)?.value, checked: (document.querySelector("#library-include-archived") as HTMLInputElement | null)?.checked })) }); evidence("03-history-diagnostic", historyFacts); };
  const wire = await open(page), baseline = b.facts(); expect(await ids(page)).not.toContain(data.target.id); const initial = await ids(page);
  evidence("browser-identity", { version: browser.version(), userAgent: await page.evaluate(() => navigator.userAgent), width: page.viewportSize()?.width });
  const readCount = listReads(wire.reads).length; await search(page).fill("原文折叠椅"); expect(param(page, "q")).toBeNull(); expect(listReads(wire.reads)).toHaveLength(readCount); expect(await ids(page)).toEqual(initial);
  await search(page).press("Enter"); await expect(rows(page)).toHaveCount(1); await expect(row(page, data.target.id)).toBeVisible(); await expect(search(page)).toBeFocused();
  await submit(page, "QA5A"); await expect(rows(page)).toHaveCount(20); await settled(page);
  const beforeMore = page.url(), hold = gate(); let nextCursor = "";
  const more = async (route: Route) => { const u = new URL(route.request().url()); if (u.pathname !== "/api/v1/items" || !u.searchParams.has("cursor")) return route.fallback(); nextCursor = u.searchParams.get("cursor")!; await hold.promise; await route.fulfill({ response: await forward(route) }); };
  await page.route("**/api/v1/items?*", more);
  try { await page.getByRole("button", { name: "加载更多", exact: true }).click(); await expect.poll(() => nextCursor !== "").toBe(true); expect(page.url()).toBe(beforeMore); await expect(page.getByText("正在加载更多…", { exact: true })).toBeVisible(); }
  finally { hold.release(); }
  await expect.poll(() => param(page, "cursor")).toBe(nextCursor); await expect(rows(page)).toHaveCount(40); expect(new Set(await ids(page)).size).toBe(40); const continuedUrl = page.url(); await page.unroute("**/api/v1/items?*", more);
  await search(page).fill("aZ19"); await page.getByRole("button", { name: "搜索", exact: true }).click(); await expect(rows(page)).toHaveCount(1); expect(param(page, "q")).toBe("aZ19"); expect(param(page, "cursor")).toBeNull();
  await page.getByLabel("显示已归档", { exact: true }).click(); await expect(page.getByLabel("显示已归档", { exact: true })).toBeChecked(); await expect(rows(page)).toHaveCount(12); expect(param(page, "cursor")).toBeNull(); expect(param(page, "archived")).toBe("true");
  await recordHistory("before-clear"); await page.getByRole("button", { name: "清除搜索", exact: true }).click(); await expect.poll(() => param(page, "q")).toBeNull(); expect(param(page, "archived")).toBe("true"); await expect(rows(page)).toHaveCount(12); await recordHistory("after-clear");
  await page.goBack(); await recordHistory("after-back"); try { await expect(search(page)).toHaveValue("aZ19"); } finally { await recordHistory("after-value-check"); } await expect(page.getByLabel("显示已归档", { exact: true })).toBeChecked();
  await page.goBack(); await expect(page.getByLabel("显示已归档", { exact: true })).not.toBeChecked(); await expect(rows(page)).toHaveCount(1); await page.goForward(); await expect(rows(page)).toHaveCount(12);
  // A new context has no library cache; it must resume the true URL page, not rely on old client memory.
  const context = await browser.newContext(); const fresh = await context.newPage(); const freshWire = await open(fresh, new URL(continuedUrl).pathname + new URL(continuedUrl).search);
  try { const expected = await items(api, b, { q: "QA5A", cursor: nextCursor }); await expect(rows(fresh)).toHaveCount(expected.data.length); expect(await ids(fresh)).toEqual(expected.data.map(v => v.id)); expect(param(fresh, "cursor")).toBe(nextCursor); expect(freshWire.external).toBe(0); }
  finally { await context.close(); }
  expect(b.facts()).toEqual(baseline); expect(wire.external).toBe(0); expect(wire.pageErrors).toBe(0);
  evidence("03-search-url", { initialCount: initial.length, continuedCursor: nextCursor, retainedPrefix: 40, freshContextStartsAtUrlCursor: true, facts: baseline, listRequests: listReads(wire.reads) });
});

test("QA PC05A 4/6: pending/failure preserves old rows, known cursor 422, empty/length errors and late response", async ({ page, browser }) => {
  const wire = await open(page), baseline = b.facts(), previous = await ids(page), hold = gate(); let held = 0;
  const failure = async (route: Route) => { const u = new URL(route.request().url()); if (u.pathname !== "/api/v1/items" || u.searchParams.get("q") !== "catalog") return route.fallback(); held++; await hold.promise; await route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: { code: "INTERNAL", message: "PC05A local transport fixture unavailable", requestId: "pc05a-local-search-failure", details: null } }) }); };
  await page.route("**/api/v1/items?*", failure);
  try { await submit(page, "catalog"); await expect.poll(() => held).toBe(1); await expect(page.getByText("正在查找…，下方为上次结果", { exact: true })).toBeVisible(); expect(await ids(page)).toEqual(previous); await expect(search(page)).toBeFocused(); }
  finally { hold.release(); }
  await expect(page.getByRole("alert").filter({ hasText: "此次搜索未完成" })).toContainText("仍显示上次结果"); await expect(page.getByRole("alert")).toContainText("pc05a-local-search-failure"); expect(await ids(page)).toEqual(previous);
  await page.unroute("**/api/v1/items?*", failure); await page.getByRole("button", { name: "重试", exact: true }).click(); await expect(page.getByRole("alert").filter({ hasText: "此次搜索未完成" })).toHaveCount(0); await expect(rows(page)).toHaveCount(20); expect(await ids(page)).not.toEqual(previous);
  const mixed = (await items(api, b, { q: "QA5A", limit: 7 })).nextCursor!; const response = page.waitForResponse(r => new URL(r.url()).pathname === "/api/v1/items" && r.status() === 422);
  await page.goto(WEB + "/?" + new URLSearchParams({ q: "catalog", cursor: mixed })); expect((await response).status()).toBe(422); await expect(page.getByRole("heading", { name: "列表条件已变化，请回到开头" })).toBeVisible();
  await page.getByRole("button", { name: "回到列表开头", exact: true }).click(); await expect.poll(() => param(page, "cursor")).toBeNull(); expect(param(page, "q")).toBe("catalog"); await expect(rows(page)).toHaveCount(20);
  const beforeLong = listReads(wire.reads).length; await search(page).fill("中".repeat(201)); await search(page).press("Enter"); await expect(search(page)).toHaveAttribute("aria-invalid", "true"); await expect(page.locator("#library-search-error")).toContainText("最多200个字符"); expect(listReads(wire.reads)).toHaveLength(beforeLong); expect(param(page, "q")).toBe("catalog");
  await submit(page, "QA5A no-match-unique"); await expect(page.getByRole("heading", { name: "当前范围没有匹配物品", exact: true })).toBeVisible(); await expect(rows(page)).toHaveCount(0); await page.getByRole("button", { name: "清除搜索", exact: true }).first().click(); await expect(rows(page)).toHaveCount(20);
  const late = gate(), finishedLate = gate(); let delayed = 0; const delay = async (route: Route) => { const u = new URL(route.request().url()); if (u.pathname !== "/api/v1/items" || u.searchParams.get("q") !== "percent%") return route.fallback(); delayed++; const actual = await forward(route); await late.promise; try { await route.fulfill({ response: actual }).catch(() => {}); } finally { finishedLate.release(); } };
  await page.route("**/api/v1/items?*", delay);
  try { await submit(page, "percent%"); await expect.poll(() => delayed).toBe(1); await submit(page, "under_"); await expect(rows(page)).toHaveCount(1); await expect(row(page, data.active[2]!.id)).toBeVisible(); }
  finally { late.release(); }
  await finishedLate.promise; await page.unroute("**/api/v1/items?*", delay); expect(param(page, "q")).toBe("under_"); expect(await ids(page)).toEqual([data.active[2]!.id]); await expect(search(page)).toBeFocused();
  // An independently owned empty-archive instance avoids changing the shared catalog.
  const empty = new Pc05aBackend(), emptyApi = await request.newContext(), context = await browser.newContext();
  try {
    await empty.start(); const csrf = await apiLogin(emptyApi, empty.base, PASSWORD); const active = await createItem(emptyApi, empty, csrf, "QA5A active-only empty archive fixture"); const emptyPage = await context.newPage(); const emptyWire = await attach(emptyPage, empty); await loginViaUi(emptyPage, WEB, PASSWORD); await expect(row(emptyPage, active.id)).toBeVisible(); await emptyPage.getByLabel("显示已归档", { exact: true }).click(); await expect(emptyPage.getByLabel("显示已归档", { exact: true })).toBeChecked();
    await expect(emptyPage.getByRole("heading", { name: "没有已归档的物品", exact: true })).toBeVisible(); await expect(rows(emptyPage)).toHaveCount(0); await emptyPage.getByRole("button", { name: "查看使用中的物品", exact: true }).click(); await expect(row(emptyPage, active.id)).toBeVisible(); expect(param(emptyPage, "archived")).toBeNull(); expect(emptyWire.external).toBe(0);
  } finally { await context.close(); await emptyApi.dispose(); await empty.cleanup(); }
  expect(b.facts()).toEqual(baseline); expect(wire.external).toBe(0); expect(wire.pageErrors).toBe(0);
  evidence("04-error-and-late-response", { injectedBoundary: "One explicit local HTTP 500 transport fixture; all successful rows and mixed-cursor 422 are actual backend responses", originalRows: previous, overlongRequests: 0, actualCursor422: true, emptyArchiveUsesSeparateOwnedInstance: true, finalIds: await ids(page), facts: baseline });
});

test("QA PC05A 5/6: true two publications, pending draft/new failed task retain latest and history, return context", async ({ page }) => {
  const initial = (await summaries(api, b, [book.ref.itemId]))[0]!; expect(initial.latestReleaseId).toBe(book.latest.id); expect(initial.action).toBe("readRelease");
  await makePending(api, b, book); const pending = (await summaries(api, b, [book.ref.itemId]))[0]!; expect(pending.action).toBe("reviewDraft"); expect(pending.latestReleaseId).toBe(book.latest.id);
  const failedId = await failedNewJob(api, b, book); const attention = (await summaries(api, b, [book.ref.itemId]))[0]!; expect(attention.action).toBe("handleJob"); expect(attention.targetId).toBe(failedId); expect(attention.latestReleaseId).toBe(book.latest.id);
  const wire = await open(page, "/?q=QA5A"), baseline = b.facts(); const target = row(page, book.ref.itemId); await expect(target.getByRole("link", { name: "处理任务", exact: true })).toBeVisible();
  await expect(target.getByRole("link", { name: "阅读已发布版", exact: true })).toHaveAttribute("href", `/items/${book.ref.itemId}/releases/${book.latest.id}`); await expect(target.locator(".workflow-actions__version")).toContainText(`草稿 r${book.latest.draftRevision}`);
  await target.scrollIntoViewIfNeeded(); const position = await target.evaluate(e => e.getBoundingClientRect().top), libraryUrl = page.url(); await target.getByRole("link", { name: "阅读已发布版", exact: true }).click();
  await expect(page.getByRole("heading", { name: "已发布说明书", exact: true })).toBeVisible(); await expect(page.getByTestId(`release-download-${book.latest.id}`)).toBeVisible();
  await page.getByRole("link", { name: "返回资料库", exact: true }).click(); await expect(page).toHaveURL(libraryUrl); await expect(target).toBeInViewport(); const returnedPosition = await target.evaluate(e => e.getBoundingClientRect().top);
  await target.getByRole("link", { name: "历史版本", exact: true }).click(); await expect(page.getByTestId("release-list").locator(":scope > li")).toHaveCount(2); await page.locator(`a[href="/items/${book.ref.itemId}/releases/${book.old.id}"]`).click(); await expect(page.getByTestId(`release-download-${book.old.id}`)).toBeVisible();
  await page.getByRole("link", { name: "返回资料库", exact: true }).click(); await expect(page).toHaveURL(libraryUrl); await target.locator(".item-row__name").click(); await expect(page.getByRole("link", { name: "返回资料库", exact: true })).toBeVisible(); await page.getByRole("link", { name: "返回资料库", exact: true }).click(); await expect(page).toHaveURL(libraryUrl);
  expect(b.db("SELECT * FROM manual_releases WHERE id=?", [book.old.id])).toEqual(book.frozenOld); expect(b.facts()).toEqual(baseline); expect(wire.external).toBe(0); expect(wire.pageErrors).toBe(0);
  evidence("05-real-latest-and-history", { ref: book.ref, old: book.old.id, latest: book.latest.id, initial, pending, attention, failedJob: failedId, realPublishes: 2, priorManifestRowUnchanged: true, returnPosition: { before: position, after: returnedPosition, sameRowVisible: true }, facts: baseline });
});

test("QA PC05A 6/6: 375/1024/1440 keyboard, bounded summary reads, both version ZIPs retain frozen hashes", async ({ page }) => {
  fs.mkdirSync(OUT, { recursive: true }); const wire = await open(page), baseline = b.facts(); const zipBefore = { old: await exportZip(api, b, book.old.id, "old-http"), latest: await exportZip(api, b, book.latest.id, "latest-http") }; expect(zipBefore.old.frozenManifestSha256).not.toBe(zipBefore.latest.frozenManifestSha256);
  const geometry = [];
  for (const width of [375, 1024, 1440]) {
    await page.setViewportSize({ width, height: 900 }); wire.reads.length = 0; await page.goto(WEB + "/?q=QA5A"); await expect(rows(page)).toHaveCount(20); await settled(page);
    const batch = wire.reads.filter(v => new URL(v, WEB).pathname === "/api/v1/items/summaries"); expect(batch).toHaveLength(1); expect(new URL(batch[0]!, WEB).searchParams.get("ids")!.split(",")).toHaveLength(20); expect(listReads(wire.reads)).toHaveLength(1); expect(wire.reads.filter(v => /^\/api\/v1\/items\/[^/?]+(?:\/releases|$)/.test(v) && !v.startsWith("/api/v1/items/summaries"))).toEqual([]);
    await search(page).focus(); await page.keyboard.press("Tab"); await expect(page.getByRole("button", { name: "搜索", exact: true })).toBeFocused(); await page.keyboard.press("Tab"); await expect(page.getByRole("button", { name: "清除搜索", exact: true })).toBeFocused(); await page.keyboard.press("Tab"); await expect(page.getByLabel("显示已归档", { exact: true })).toBeFocused();
    await submit(page, "published manual"); await expect(rows(page)).toHaveCount(1); await expect(search(page)).toBeFocused();
    const measurements = [];
    for (const control of [search(page), page.getByRole("button", { name: "搜索", exact: true }), page.getByRole("button", { name: "清除搜索", exact: true }), row(page, book.ref.itemId).getByRole("link", { name: /阅读.*版|阅读说明书/ })]) { const box = await control.boundingBox(); expect(box?.height).toBeGreaterThanOrEqual(44); expect(box?.width).toBeGreaterThanOrEqual(44); measurements.push(box); }
    const overflow = await page.evaluate(() => ({ width: innerWidth, scrollWidth: document.documentElement.scrollWidth })); expect(overflow.scrollWidth).toBeLessThanOrEqual(overflow.width);
    await tabTo(page, row(page, book.ref.itemId).getByRole("link", { name: "历史版本", exact: true })); await page.keyboard.press("Enter"); await expect(page.getByRole("heading", { name: "发布版本", exact: true })).toBeVisible();
    if (width === 375) for (const version of ["old", "latest"] as const) { const release = book[version], control = page.getByTestId(`release-download-${release.id}`), button = control.getByRole("button"); await tabTo(page, button); const downloading = page.waitForEvent("download"); await page.keyboard.press("Enter"); const file = path.join(OUT, `${version}-keyboard-375.zip`); await (await downloading).saveAs(file); expect(inspectZip(file, release.id)).toEqual(zipBefore[version]); await expect(button).toBeFocused(); }
    await page.getByRole("link", { name: "返回资料库", exact: true }).click(); await expect(search(page)).toHaveValue("published manual"); await expect(rows(page)).toHaveCount(1);
    geometry.push({ width, measurements, overflow, listRequests: 1, boundedSummaryRequests: batch.length, summaryIds: 20 });
  }
  expect(b.facts()).toEqual(baseline); expect(wire.external).toBe(0); expect(wire.pageErrors).toBe(0); evidence("06-keyboard-zip", { geometry, zipBefore, facts: baseline, after: b.facts(), traceVideoHar: "off", provider: "localhost only" });
});
