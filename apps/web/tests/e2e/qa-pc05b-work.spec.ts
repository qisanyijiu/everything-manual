/** Independent six-group PC05B acceptance. AUTHORING ONLY until root freezes B and supplies GO. */
import fs from "node:fs";
import { expect, request, test, type APIRequestContext, type BrowserContext, type Page } from "@playwright/test";
import type { components } from "../../src/api/generated";
import { apiLogin, fetchAsset, seedItemWithDocument } from "./helpers";
import { fixturePath } from "./runtime";
import { digest, evidence, PASSWORD, Pc05bBackend, WEB } from "./qa-pc05b-backend";
import { createItem, draftFixture, patchDraft, patchItem, readDraft, readItem, type DraftRef } from "./qa-pc05b-fixture";
import { boot, CANARY, enter, expireSession, gate, leaveDialog, library, loginInDocument, nativeClose, nativeReload, noPromptReload, prompt, promptRing, historyNavigation, SECRET_CANARY } from "./qa-pc05b-controls";

test.describe.configure({ mode: "default", timeout: 210000 });
let backend: Pc05bBackend, api: APIRequestContext, csrf: string;
const contexts: BrowserContext[] = [], sessions: Awaited<ReturnType<typeof boot>>[] = [], gates: Awaited<ReturnType<typeof gate>>[] = [], releases: (() => void)[] = [];
test.beforeEach(async () => { backend = new Pc05bBackend(); await backend.start(); api = await request.newContext(); csrf = await apiLogin(api, backend.base, PASSWORD); });
test.afterEach(async () => {
  try {
    for (const release of releases.splice(0)) release();
    for (const g of gates.splice(0)) await g.remove();
    for (const s of sessions.splice(0)) { expect(s.state.external).toBe(0); expect(s.state.pageErrors).toBe(0); if (!s.page.isClosed()) await s.verifyStorage(); }
  } finally {
    for (const c of contexts.splice(0)) await c.close(); await api?.dispose(); await backend?.cleanup();
  }
});

const itemPath = (id: string) => `/api/v1/items/${id}`;
const draftPath = (ref: DraftRef) => `${itemPath(ref.itemId)}/drafts/${ref.draftId}`;
const reviewUrl = (ref: DraftRef) => `${WEB}/items/${ref.itemId}/drafts/${ref.draftId}/review`;
const saveKnowledge = (page: Page) => page.getByRole("button", { name: "保存人工修订（并确认事实）", exact: true });
async function watch(...args: Parameters<typeof gate>) { const g = await gate(...args); gates.push(g); return g; }
async function session(...args: Parameters<typeof boot>) { const s = await boot(...args); sessions.push(s); return s; }
async function partEdit(page: Page, id: string, value: string) {
  const row = page.getByTestId(`knowledge-${id}`); await expect(row).toBeVisible(); await row.getByRole("button", { name: "复制为本地修订", exact: true }).click();
  await row.getByLabel("部件名", { exact: true }).fill(value); return row;
}
async function knowledgePage(page: Page, ref: DraftRef) { await page.goto(reviewUrl(ref)); await expect(page.getByTestId("knowledge-panel")).toBeVisible(); }
function patchCount(s: Awaited<ReturnType<typeof boot>>, path: string) { return s.state.writes.filter(v => v === "PATCH " + path).length; }
async function failReads(page: Page, path: string) {
  const pattern = "**" + path;
  await page.route(pattern, route => route.request().method() === "GET" ? route.continue({ url: backend.base + "/api/v1/items/01930000-0000-7000-8000-000000000099" }) : route.fallback());
  return () => page.unroute(pattern);
}
async function discardItem(page: Page, accept: boolean) {
  await page.getByRole("button", { name: "丢弃本页修改并加载最新版本", exact: true }).click(); await prompt(page, accept, "丢弃本页修改并加载最新版本");
}

type Preparation = components["schemas"]["PreparationDetailDto"];
async function preparation(id: string) { const r = await api.get(`${backend.base}/api/v1/preparations/${id}`); expect(r.status()).toBe(200); return (await r.json()).data as Preparation; }
async function savedPage(id: string, number = 1) {
  const row = (await preparation(id)).pages.find(p => p.pageNumber === number); if (!row?.imageAssetId || !row.textAssetId) throw new Error("Actual PDF.js saved assets missing");
  const image = await fetchAsset(api, backend.base, row.imageAssetId), text = await fetchAsset(api, backend.base, row.textAssetId); expect(text.bytes.toString()).toContain(`Page ${number} of 2`); expect(image.contentType).toContain("image/jpeg");
  const imageSha = digest(image.bytes), textSha = digest(text.bytes); expect(backend.db("SELECT id,blob_id FROM assets WHERE id IN (?,?) ORDER BY id", [row.imageAssetId, row.textAssetId])).toEqual([{ id: row.imageAssetId, blob_id: imageSha }, { id: row.textAssetId, blob_id: textSha }].sort((a, b) => a.id.localeCompare(b.id)));
  return { row, imageSha, textSha, persisted: backend.db("SELECT * FROM pages WHERE preparation_id=? AND page_number=?", [id, number]) };
}
async function partial(page: Page, itemId: string, documentId: string) {
  let release: () => void = () => {}; const held = new Promise<void>(resolve => { release = resolve; }); releases.push(release); const state = { id: "", held: false, settled: false, count: 0 };
  const pattern = "**/api/v1/preparations/*/pages/2";
  await page.route(pattern, async route => { state.count++; state.id = /\/preparations\/([^/]+)\/pages\/2$/.exec(new URL(route.request().url()).pathname)?.[1] ?? ""; state.held = true; await held; try { await route.abort("aborted"); } catch { /* canceled by real user departure */ } finally { state.settled = true; } });
  await page.goto(`${WEB}/items/${itemId}/import/prepare?documentId=${documentId}`); await expect(page.getByTestId("prepare-start")).toBeEnabled(); await enter(page, page.getByTestId("prepare-start")); await expect.poll(() => state.held).toBe(true);
  expect(state.id).not.toBe(""); const first = await savedPage(state.id); expect((await preparation(state.id)).pages.filter(p => p.imageAssetId && p.textAssetId && p.viewport).map(p => p.pageNumber)).toEqual([1]);
  return { state, first, async remove() { release(); if (!page.isClosed()) await page.unroute(pattern); } };
}

test("B1 real item and knowledge save states, no reentry, actual 422 and success-only navigation", async ({ browser }) => {
  const s = await session(browser, backend, contexts); const { page } = s;
  await page.goto(WEB + "/items/new"); await page.getByLabel(/^名称/).fill(CANARY + "create"); await page.getByLabel(/^准确型号/).fill("owned-model");
  await expect(page.locator("#item-save-status")).toHaveText("有未保存修改"); await expect(page.locator("#item-save-status")).toHaveAttribute("aria-live", "polite");
  const created = await watch(page, backend, "/api/v1/items", "POST", "committed"); await page.getByRole("button", { name: "创建并继续", exact: true }).click(); await expect.poll(() => created.state.held).toBe(true);
  expect(created.state.status).toBe(201); await expect(page.locator("#item-save-status")).toHaveText("正在保存…"); await expect(page.getByRole("button", { name: "保存中…", exact: true })).toBeDisabled(); await page.keyboard.press("Enter"); await page.keyboard.press("Enter"); expect(created.state.count).toBe(1);
  expect(backend.db("SELECT count(*) AS n FROM items WHERE name=?", [CANARY + "create"])).toEqual([{ n: 1 }]); await expect(page).toHaveURL(WEB + "/items/new");
  created.release("forward"); await expect(page).toHaveURL(/\/items\/[^/]+\/import\/document$/); await expect(leaveDialog(page)).toBeHidden(); await created.remove();
  const itemId = /\/items\/([^/]+)\//.exec(page.url())?.[1]; if (!itemId) throw new Error("Missing created item ID");
  await page.goto(`${WEB}/items/${itemId}/edit`); await page.getByLabel(/^名称/).fill(CANARY + "edit"); const edited = await watch(page, backend, itemPath(itemId), "PATCH", "committed");
  await page.getByRole("button", { name: "保存", exact: true }).click(); await expect.poll(() => edited.state.held).toBe(true); await expect(page.locator("#item-save-status")).toHaveText("正在保存…"); await page.keyboard.press("Enter"); expect(edited.state.count).toBe(1); edited.release("forward"); await expect(page).toHaveURL(`${WEB}/items/${itemId}`); await expect(leaveDialog(page)).toBeHidden(); await edited.remove();
  expect((await readItem(api, backend, itemId)).item.name).toBe(CANARY + "edit");
  await page.goto(WEB + "/items/new"); await page.getByLabel(/^名称/).fill("x".repeat(201)); await page.getByLabel(/^准确型号/).fill("retained-model");
  const invalid = page.waitForResponse(r => r.request().method() === "POST" && r.url().endsWith("/api/v1/items")); await page.getByRole("button", { name: "创建并继续", exact: true }).click(); expect((await invalid).status()).toBe(422); await expect(page.getByLabel(/^名称/)).toBeFocused(); await expect(page.getByLabel(/^准确型号/)).toHaveValue("retained-model"); await expect(page.getByLabel(/^名称/)).toHaveAttribute("aria-invalid", "true");
  await library(page).click(); await prompt(page, true);
  const book = await draftFixture(api, backend); csrf = book.csrf; await knowledgePage(page, book.ref); const part = book.initial.knowledge.knowledge.parts[0]!; const before = await readDraft(api, backend, book.ref); const baseline = backend.facts();
  const row = await partEdit(page, part.id, CANARY + "knowledge-save"); const read = await watch(page, backend, draftPath(book.ref), "GET", "committed");
  const patchResponse = page.waitForResponse(r => r.request().method() === "PATCH" && r.url().endsWith(draftPath(book.ref))); await saveKnowledge(page).click(); expect((await patchResponse).status()).toBe(200); await expect.poll(() => read.state.held).toBe(true);
  await expect(page.locator("#knowledge-save-status")).toHaveText("正在保存…"); await expect(row.getByLabel("部件名", { exact: true })).toHaveValue(CANARY + "knowledge-save"); await expect(saveKnowledge(page)).toBeDisabled(); await page.keyboard.press("Enter"); expect(patchCount(s, draftPath(book.ref))).toBe(1);
  read.release("forward"); await expect(row.getByLabel("部件名", { exact: true })).toHaveCount(0); await expect(page.locator("#knowledge-save-status")).toHaveText("已保存"); await read.remove();
  const after = await readDraft(api, backend, book.ref); expect(after.dto.revision).toBe(before.dto.revision + 1); expect(after.knowledge).toEqual(before.knowledge); expect(after.review.entities[part.id]?.userEdited?.name).toBe(CANARY + "knowledge-save"); expect(backend.facts()).toEqual(baseline);
  evidence("b1-save", { editedItemId: itemId, ...book.ref, actualStatuses: [201, 200, 422], createdRequests: created.state.count, editedRequests: edited.state.count, knowledgePatchCount: 1, readHeldBeforeSaved: true, originalKnowledgeUnchanged: true, storage: await s.verifyStorage() });
});

test("B2 lost responses and failed readback retain inputs, uncertainty blocks duplicate creation", async ({ browser }) => {
  const s = await session(browser, backend, contexts); const { page } = s;
  await page.goto(WEB + "/items/new"); await page.getByLabel(/^名称/).fill(CANARY + "unknown-create"); await page.getByLabel(/^准确型号/).fill("owned-model");
  const create = await watch(page, backend, "/api/v1/items", "POST", "committed"); await page.getByRole("button", { name: "创建并继续", exact: true }).click(); await expect.poll(() => create.state.held).toBe(true); expect(create.state.status).toBe(201); create.release("abort");
  await expect(page.getByText("创建结果未知，请先读取核对。未收到响应不代表未创建，不会自动重复提交。", { exact: true })).toBeVisible(); await expect(page.getByRole("button", { name: "创建并继续", exact: true })).toBeDisabled(); await page.getByRole("button", { name: "核对创建结果", exact: true }).click(); await expect(page.getByText(/名称相同或未找到都不能证明/)).toBeVisible(); await page.keyboard.press("Enter"); expect(create.state.count).toBe(1); await expireSession(page, backend); await page.getByRole("button", { name: "核对创建结果", exact: true }).click(); await loginInDocument(page); await expect(page.getByRole("button", { name: "创建并继续", exact: true })).toBeDisabled(); expect(create.state.count).toBe(1); expect(backend.db("SELECT count(*) AS n FROM items WHERE name=?", [CANARY + "unknown-create"])).toEqual([{ n: 1 }]); await create.remove();
  await library(page).click(); await prompt(page, true);
  const item = await createItem(api, backend, csrf); const before = await readItem(api, backend, item.id); await page.goto(`${WEB}/items/${item.id}/edit`); await page.getByLabel(/^名称/).fill(CANARY + "unsent"); const unsent = await watch(page, backend, itemPath(item.id), "PATCH"); await page.getByRole("button", { name: "保存", exact: true }).click(); await expect.poll(() => unsent.state.held).toBe(true); unsent.release("abort");
  await expect(page.getByText("保存结果未知，请先读取核对。", { exact: true })).toBeVisible(); expect(await readItem(api, backend, item.id)).toEqual(before); await expect(page.getByLabel(/^名称/)).toHaveValue(CANARY + "unsent"); await expect(page.getByRole("button", { name: "保存", exact: true })).toBeDisabled(); await unsent.remove(); await discardItem(page, true); await expect(page.getByLabel(/^名称/)).toHaveValue(item.name);
  await page.getByLabel(/^名称/).fill(CANARY + "committed-lost"); const committed = await watch(page, backend, itemPath(item.id), "PATCH", "committed"); await page.getByRole("button", { name: "保存", exact: true }).click(); await expect.poll(() => committed.state.held).toBe(true); expect(committed.state.status).toBe(200); committed.release("abort");
  await expect(page.getByText("保存结果未知，请先读取核对。", { exact: true })).toBeVisible(); expect((await readItem(api, backend, item.id)).item.revision).toBe(before.item.revision + 1); await page.getByRole("button", { name: "核对最新版本", exact: true }).click(); await expect(page.getByRole("region", { name: "服务器最新物品", exact: true })).toBeVisible(); expect(committed.state.count).toBe(1); await expect(page.getByRole("button", { name: "保存", exact: true })).toBeDisabled(); await committed.remove(); await discardItem(page, true);
  const book = await draftFixture(api, backend); csrf = book.csrf; await knowledgePage(page, book.ref); const part = book.initial.knowledge.knowledge.parts[0]!; await partEdit(page, part.id, CANARY + "readback-lost"); const removeReadFailure = await failReads(page, draftPath(book.ref));
  const saved = page.waitForResponse(r => r.request().method() === "PATCH" && r.url().endsWith(draftPath(book.ref))); await saveKnowledge(page).click(); expect((await saved).status()).toBe(200); await expect(page.getByTestId("workspace-notice")).toContainText("修改已提交，但读取结果失败"); await expect(page.getByLabel("部件名", { exact: true })).toHaveValue(CANARY + "readback-lost"); await expect(saveKnowledge(page)).toBeDisabled(); expect(patchCount(s, draftPath(book.ref))).toBe(1); expect((await readDraft(api, backend, book.ref)).review.entities[part.id]?.userEdited?.name).toBe(CANARY + "readback-lost"); await removeReadFailure();
  evidence("b2-uncertainty", { editedItemId: item.id, ...book.ref, preSendNotCommitted: true, committedResponseLost: true, createRows: 1, unknownCreateNotRetried: true, knowledgePatchSucceededReadback404: true, storage: await s.verifyStorage() });
});

test("B3 actual 401 preserves same-document memory; actual item and knowledge 412 require explicit discard", async ({ browser }) => {
  const item = await createItem(api, backend, csrf); const s = await session(browser, backend, contexts); const { page } = s; await page.goto(`${WEB}/items/${item.id}/edit`);
  await page.getByLabel(/^名称/).fill(CANARY + "401-item"); await expireSession(page, backend); const denied = page.waitForResponse(r => r.request().method() === "PATCH" && r.url().endsWith(itemPath(item.id))); await page.getByRole("button", { name: "保存", exact: true }).click(); expect((await denied).status()).toBe(401); await loginInDocument(page); await expect(page.getByLabel(/^名称/)).toHaveValue(CANARY + "401-item"); expect(patchCount(s, itemPath(item.id))).toBe(1); expect((await readItem(api, backend, item.id)).item.name).toBe(item.name); await s.verifyStorage();
  const latest = await patchItem(api, backend, csrf, item.id, "PC05B remote item"); const conflict = page.waitForResponse(r => r.request().method() === "PATCH" && r.url().endsWith(itemPath(item.id))); await page.getByRole("button", { name: "保存", exact: true }).click(); expect((await conflict).status()).toBe(412); await expect(page.getByRole("button", { name: "保存", exact: true })).toBeDisabled(); await page.getByRole("button", { name: "核对最新版本", exact: true }).click(); await expect(page.getByRole("region", { name: "服务器最新物品", exact: true })).toContainText(`r${latest.item.revision}`); await expect(page.getByLabel(/^名称/)).toHaveValue(CANARY + "401-item");
  await discardItem(page, false); await expect(page.getByLabel(/^名称/)).toHaveValue(CANARY + "401-item"); const remove = await failReads(page, itemPath(item.id)); await discardItem(page, true); await expect(page.getByText(/读取失败，修改仍在本页/)).toBeVisible(); await expect(page.getByLabel(/^名称/)).toHaveValue(CANARY + "401-item"); await expect(page.getByRole("button", { name: "保存", exact: true })).toBeDisabled(); await remove(); await discardItem(page, true); await expect(page.getByLabel(/^名称/)).toHaveValue("PC05B remote item");
  await page.getByLabel(/^名称/).fill("PC05B explicit correction"); await page.getByRole("button", { name: "保存", exact: true }).click(); await expect(page).toHaveURL(`${WEB}/items/${item.id}`); expect((await readItem(api, backend, item.id)).item.revision).toBe(latest.item.revision + 1);
  const book = await draftFixture(api, backend); csrf = book.csrf; await knowledgePage(page, book.ref); const part = book.initial.knowledge.knowledge.parts[0]!; await partEdit(page, part.id, CANARY + "401-knowledge"); await expireSession(page, backend);
  const deniedDraft = page.waitForResponse(r => r.request().method() === "PATCH" && r.url().endsWith(draftPath(book.ref))); await saveKnowledge(page).click(); expect((await deniedDraft).status()).toBe(401); await loginInDocument(page); await expect(page.getByLabel("部件名", { exact: true })).toHaveValue(CANARY + "401-knowledge"); expect(patchCount(s, draftPath(book.ref))).toBe(1); await s.verifyStorage();
  const other = await patchDraft(api, backend, book.ref, csrf, { entities: { [part.id]: { userEdited: { name: "PC05B remote knowledge" } } } });
  const conflictDraft = page.waitForResponse(r => r.request().method() === "PATCH" && r.url().endsWith(draftPath(book.ref))); await saveKnowledge(page).click(); expect((await conflictDraft).status()).toBe(412); await expect(saveKnowledge(page)).toBeDisabled();
  await page.getByRole("button", { name: "核对最新版本", exact: true }).click(); await prompt(page, false); await expect(page.getByLabel("部件名", { exact: true })).toHaveValue(CANARY + "401-knowledge");
  await page.getByRole("button", { name: "核对最新版本", exact: true }).click(); await expect(leaveDialog(page)).toBeVisible(); const removeDraftFailure = await failReads(page, draftPath(book.ref)); await prompt(page, true, "丢弃本页修改并加载最新版本"); await expect(page.getByTestId("workspace-notice")).toContainText("读取失败，本地编辑仍保留"); await expect(page.getByLabel("部件名", { exact: true })).toHaveValue(CANARY + "401-knowledge"); await removeDraftFailure();
  await page.getByRole("button", { name: "核对最新版本", exact: true }).click(); await prompt(page, true, "丢弃本页修改并加载最新版本"); await expect(page.getByLabel("部件名", { exact: true })).toHaveCount(0); await partEdit(page, part.id, "PC05B new explicit knowledge"); await saveKnowledge(page).click(); await expect(page.getByLabel("部件名", { exact: true })).toHaveCount(0); const final = await readDraft(api, backend, book.ref); expect(final.dto.revision).toBe(other.dto.revision + 1); expect(final.review.entities[part.id]?.userEdited?.name).toBe("PC05B new explicit knowledge"); expect(final.knowledge).toEqual(book.initial.knowledge);
  evidence("b3-auth-cas", { editedItemId: item.id, ...book.ref, actual401: 2, actual412: 2, sameDocumentLogin: true, noAutomaticReplay: true, canceledAndFailedDiscardRetainedBuffers: true, originalKnowledgeUnchanged: true, storage: await s.verifyStorage() });
});

test("B4 Link, Back and Forward use one keyboard-safe dialog and preserve multiple knowledge buffers", async ({ browser }) => {
  const item = await createItem(api, backend, csrf); const s = await session(browser, backend, contexts); const { page } = s;
  await page.locator(`a[href="/items/${item.id}"]`).first().click(); await page.locator(`a[href="/items/${item.id}/edit"]`).click(); await expect(page.getByLabel(/^名称/)).toHaveValue(item.name);
  const editUrl = page.url(); const historyBefore = await page.evaluate(() => history.length); await page.getByLabel(/^名称/).fill(CANARY + "navigation");
  const trigger = library(page); await enter(page, trigger); const dialog = leaveDialog(page); await expect(dialog).toBeVisible(); await expect(dialog.getByRole("button", { name: "继续处理", exact: true })).toBeFocused();
  await promptRing(page);
  await page.keyboard.press("Escape"); await expect(dialog).toBeHidden(); await expect(trigger).toBeFocused(); await expect(page).toHaveURL(editUrl); await expect(page.getByLabel(/^名称/)).toHaveValue(CANARY + "navigation");
  await historyNavigation(page, "back"); await prompt(page, false); await expect(page).toHaveURL(editUrl); expect(await page.evaluate(() => history.length)).toBe(historyBefore);
  await historyNavigation(page, "back"); await prompt(page, true); await expect(page).toHaveURL(`${WEB}/items/${item.id}`); await historyNavigation(page, "forward"); await expect(page).toHaveURL(editUrl); await expect(page.getByLabel(/^名称/)).toHaveValue(item.name); await expect(dialog).toBeHidden();
  // A successful save is a reachable useNavigate action; it must navigate once without a second prompt.
  await page.getByLabel(/^名称/).fill("PC05B saved navigation"); await page.getByRole("button", { name: "保存", exact: true }).click(); await expect(page).toHaveURL(`${WEB}/items/${item.id}`); await expect(dialog).toBeHidden(); await historyNavigation(page, "back"); await expect(page).toHaveURL(editUrl);
  await page.getByLabel(/^名称/).fill(CANARY + "forward"); const forwardHistory = await page.evaluate(() => history.length); await historyNavigation(page, "forward"); await prompt(page, false); await expect(page).toHaveURL(editUrl); await expect(page.getByLabel(/^名称/)).toHaveValue(CANARY + "forward"); expect(await page.evaluate(() => history.length)).toBe(forwardHistory);
  await historyNavigation(page, "forward"); await prompt(page, true); await expect(page).toHaveURL(`${WEB}/items/${item.id}`); expect(await page.evaluate(() => history.length)).toBe(forwardHistory);
  const book = await draftFixture(api, backend); csrf = book.csrf; await knowledgePage(page, book.ref); const parts = book.initial.knowledge.knowledge.parts, spec = book.initial.knowledge.knowledge.specs[0]!; expect(parts.length).toBeGreaterThanOrEqual(1); const original = await readDraft(api, backend, book.ref);
  await partEdit(page, parts[0]!.id, CANARY + "buffer-one"); const secondRow = page.getByTestId(`knowledge-${spec.id}`); await secondRow.getByRole("button", { name: "复制为本地修订", exact: true }).click(); await secondRow.getByLabel("规格值", { exact: true }).fill(CANARY + "buffer-two");
  await library(page).click(); await prompt(page, false); await expect(page.getByLabel("规格值", { exact: true })).toHaveValue(CANARY + "buffer-two");
  await page.getByTestId(`knowledge-${parts[0]!.id}`).getByRole("button", { name: "复制为本地修订", exact: true }).click(); await expect(page.getByLabel("部件名", { exact: true })).toHaveValue(CANARY + "buffer-one"); await expect(leaveDialog(page)).toBeHidden();
  // Hash-only skip navigation keeps every buffer and does not prompt.
  await page.getByRole("link", { name: "跳到主要内容", exact: true }).focus(); await page.keyboard.press("Enter"); await expect(leaveDialog(page)).toBeHidden();
  await library(page).click(); await prompt(page, true); await expect(page).toHaveURL(WEB + "/"); await knowledgePage(page, book.ref); await page.getByTestId(`knowledge-${parts[0]!.id}`).getByRole("button", { name: "复制为本地修订", exact: true }).click(); await expect(page.getByLabel("部件名", { exact: true })).toHaveValue(parts[0]!.name!); await page.getByTestId(`knowledge-${spec.id}`).getByRole("button", { name: "复制为本地修订", exact: true }).click(); await expect(page.getByLabel("规格值", { exact: true })).toHaveValue(spec.value!);
  expect(await readDraft(api, backend, book.ref)).toEqual(original); expect(patchCount(s, draftPath(book.ref))).toBe(0);
  const narrow = await session(browser, backend, contexts, 375); await narrow.page.goto(`${WEB}/items/${book.ref.itemId}`); await narrow.page.locator(`a[href="/items/${book.ref.itemId}/drafts/${book.ref.draftId}/review"]`).first().click(); await expect(narrow.page.getByTestId("review-tasks")).toBeVisible(); await narrow.page.getByRole("button", { name: "步骤与原文", exact: true }).click(); await expect(narrow.page.getByRole("dialog", { name: "步骤与原文", exact: true })).toBeVisible();
  const narrowRow = await partEdit(narrow.page, parts[0]!.id, CANARY + "drawer-buffer"); const field = narrowRow.getByLabel("部件名", { exact: true }); await field.focus(); const narrowUrl = narrow.page.url(); await historyNavigation(narrow.page, "back"); await expect(leaveDialog(narrow.page)).toBeVisible(); await promptRing(narrow.page); await narrow.page.keyboard.press("Escape"); await expect(leaveDialog(narrow.page)).toBeHidden(); await expect(narrow.page).toHaveURL(narrowUrl); await expect(field).toBeFocused(); await expect(field).toHaveValue(CANARY + "drawer-buffer"); await expect(narrow.page.getByRole("dialog", { name: "步骤与原文", exact: true })).toBeVisible();
  evidence("b4-navigation", { editedItemId: item.id, ...book.ref, actualLinkBackForward: true, cancelRestoredFocus: true, keyboardTrap: true, drawer375TrapAndEscapeRestore: true, noExtraHistory: true, twoLocalBuffersPreservedUntilExplicitLeave: true, successfulSaveProgramNavigation: true, dirtyProgramNavigationRequiresFrozenComponentEvidence: true, storage: await s.verifyStorage() });
});

test("B5 real upload and PDF.js departure stop unfinished work and preserve completed assets and pages", async ({ browser }) => {
  const item = await createItem(api, backend, csrf); const s = await session(browser, backend, contexts); const { page } = s; const baseline = backend.facts(); await page.goto(`${WEB}/items/${item.id}/import/document`);
  await page.getByLabel("选择 PDF 文件", { exact: true }).setInputFiles(fixturePath("sample-manual-text.pdf")); await expect(page.getByRole("button", { name: "绑定为说明书", exact: true })).toBeEnabled(); await page.getByLabel("标题（可选）", { exact: true }).fill("PC05B completed original"); await page.getByRole("button", { name: "绑定为说明书", exact: true }).click(); await expect(page.getByRole("heading", { name: "已绑定的说明书", exact: true })).toBeVisible();
  const list = await api.get(`${backend.base}${itemPath(item.id)}/documents`); expect(list.status()).toBe(200); const docs = (await list.json()).data as components["schemas"]["DocumentDto"][]; expect(docs).toHaveLength(1); const doc = docs[0]!; expect(doc.sourceSha256).toBe(digest(fs.readFileSync(fixturePath("sample-manual-text.pdf"))));
  const upload = await watch(page, backend, `${itemPath(item.id)}/assets`, "POST"); let canceledUploads = 0; page.on("requestfailed", r => { if (new URL(r.url()).pathname === `${itemPath(item.id)}/assets`) canceledUploads++; });
  await page.getByLabel("选择 PDF 文件", { exact: true }).setInputFiles(fixturePath("sample-manual-scan.pdf")); await expect.poll(() => upload.state.held).toBe(true); const nativeUploadCancel = await nativeReload(page, false); await expect(page.getByLabel("选择 PDF 文件", { exact: true })).toBeDisabled();
  await library(page).click(); await prompt(page, false); await expect(page.getByLabel("选择 PDF 文件", { exact: true })).toBeDisabled(); expect(upload.state.count).toBe(1); await library(page).click(); await prompt(page, true); await expect(page).toHaveURL(WEB + "/"); await expect.poll(() => canceledUploads).toBeGreaterThan(0); await upload.remove();
  const preserved = await api.get(`${backend.base}${itemPath(item.id)}/documents`); expect((await preserved.json()).data).toEqual(docs);
  const active = await partial(page, item.id, doc.id); const nativePreparationCancel = await nativeReload(page, false); await expect(page.getByTestId("prepare-cancel")).toBeVisible(); await library(page).click(); await prompt(page, false); await expect(page.getByTestId("prepare-cancel")).toBeVisible(); expect(active.state.count).toBe(1);
  await library(page).click(); await prompt(page, true); await expect(page).toHaveURL(WEB + "/"); await active.remove(); const stopped = await preparation(active.state.id); expect(stopped.state).toBe("preparing"); expect(await savedPage(active.state.id)).toEqual(active.first); expect(s.state.writes.filter(v => v === `POST /api/v1/preparations/${active.state.id}/seal`)).toEqual([]);
  const resumed = await session(browser, backend, contexts); await resumed.page.goto(`${WEB}/items/${item.id}/import/prepare?documentId=${doc.id}`); await expect(resumed.page.getByRole("region", { name: "当前准备进度", exact: true }).locator("code")).toHaveText(active.state.id); await expect(resumed.page.getByTestId("prepare-start")).toBeEnabled(); resumed.state.writes.length = 0;
  await resumed.page.getByTestId("prepare-start").click(); await expect(resumed.page.getByTestId("prepare-seal")).toBeEnabled(); expect(resumed.state.writes.filter(v => v.includes("/pages/"))).toEqual([`PUT /api/v1/preparations/${active.state.id}/pages/2`]); expect(await savedPage(active.state.id)).toEqual(active.first); const second = await savedPage(active.state.id, 2); expect((await preparation(active.state.id)).state).toBe("preparing"); expect(backend.facts()).toEqual(baseline);
  evidence("b5-partial-retention", { itemId: item.id, documentId: doc.id, preparationId: active.state.id, documentSha: doc.sourceSha256, firstPage: active.first, secondPage: second, uploadCanceled: canceledUploads > 0, nativeUploadCancel, nativePreparationCancel, completedOriginalUnchanged: true, freshContextOnlyMissingPut: true, noAutoSeal: true, noJobsOrProviderCalls: true, storage: await s.verifyStorage() });
});

test("B6 real native refresh and close, no false prompts, settings key exclusion and 375 keyboard layout", async ({ browser }) => {
  const item = await createItem(api, backend, csrf); const s = await session(browser, backend, contexts, 375); const { page } = s; await page.goto(`${WEB}/items/${item.id}/edit`); await expect(page.getByLabel(/^名称/)).toHaveValue(item.name); await noPromptReload(page);
  await page.getByLabel(/^名称/).fill(CANARY + "native-item"); const canceled = await nativeReload(page, false); await expect(page.getByLabel(/^名称/)).toHaveValue(CANARY + "native-item");
  await enter(page, library(page)); const dialog = leaveDialog(page); await expect(dialog).toBeVisible(); for (const label of ["继续处理", "离开页面"]) { const button = dialog.getByRole("button", { name: label, exact: true }); expect((await button.boundingBox())?.height).toBeGreaterThanOrEqual(44); expect(await button.evaluate(el => parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(14); }
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true); await promptRing(page); await page.keyboard.press("Escape"); await expect(library(page)).toBeFocused(); const accepted = await nativeReload(page, true); await expect(page.getByLabel(/^名称/)).toHaveValue(item.name); await s.verifyStorage();
  await page.goto(WEB + "/settings"); await page.getByRole("group", { name: "Tripo 密钥操作", exact: true }).getByLabel("替换", { exact: true }).check(); await page.getByLabel("新的 Tripo 密钥", { exact: true }).fill(SECRET_CANARY); await library(page).click(); await prompt(page, false); await expect(page.getByLabel("新的 Tripo 密钥", { exact: true })).toHaveValue(SECRET_CANARY); await s.verifyStorage(); await library(page).click(); await prompt(page, true); await expect(page).toHaveURL(WEB + "/"); await page.goto(WEB + "/settings"); await expect(page.getByLabel("新的 Tripo 密钥", { exact: true })).toHaveCount(0); await noPromptReload(page); await s.verifyStorage();
  // Only owned fake settings are read. No key is saved or shared with ordinary edit memory.
  const book = await draftFixture(api, backend, true); csrf = book.csrf; const desktop = await session(browser, backend, contexts); const p = desktop.page; const facts = backend.facts();
  await knowledgePage(p, book.ref); const part = book.initial.knowledge.knowledge.parts[0]!; await partEdit(p, part.id, CANARY + "native-knowledge"); const knowledgeCanceled = await nativeReload(p, false); await expect(p.getByLabel("部件名", { exact: true })).toHaveValue(CANARY + "native-knowledge"); await desktop.verifyStorage(); await library(p).click(); await prompt(p, true);
  for (const route of [`/items/${book.ref.itemId}/documents/${book.ref.documentId}`, `/items/${book.ref.itemId}/releases/${book.releaseId}`, `/jobs/${book.ref.jobId}`]) { await p.goto(WEB + route); await expect(p.locator("h1").first()).toBeVisible(); await noPromptReload(p); }
  expect(backend.facts()).toEqual(facts);
  const source = await seedItemWithDocument(api, backend.base, PASSWORD, "sample-manual-text.pdf", "PC05B native-close source"); csrf = await apiLogin(api, backend.base, PASSWORD); const closing = await session(browser, backend, contexts); const active = await partial(closing.page, source.itemId, source.documentId); await closing.verifyStorage(); const closeAccepted = await nativeClose(closing.page); await active.remove();
  const fresh = await session(browser, backend, contexts); await fresh.page.goto(`${WEB}/items/${source.itemId}/import/prepare?documentId=${source.documentId}`); await expect(fresh.page.getByRole("region", { name: "当前准备进度", exact: true }).locator("code")).toHaveText(active.state.id); expect(await savedPage(active.state.id)).toEqual(active.first); expect((await preparation(active.state.id)).state).toBe("preparing"); expect(backend.facts()).toEqual(facts);
  evidence("b6-native-layout", { canceled, accepted, knowledgeCanceled, closeAccepted, nativeClosePreparationId: active.state.id, preservedPageSha: { image: active.first.imageSha, text: active.first.textSha }, cleanFormOriginalReleaseAcceptedJobNoPrompt: true, settingsKeyDiscarded: true, width: 375, dialogMinHeight: 44, textMinPx: 14, noHorizontalOverflow: true, storage: await s.verifyStorage() });
});
