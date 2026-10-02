import { expect, type Browser, type BrowserContext, type Dialog, type Locator, type Page, type Route } from "@playwright/test";
import { loginViaUi } from "./helpers";
import { attach, evidence, PASSWORD, Pc05bBackend, WEB } from "./qa-pc05b-backend";
export const CANARY = "QA5B-BUFFER-";
export const SECRET_CANARY = "sk-pc05b-owned-secret-only-000000000000";
export const leaveDialog = (page: Page) => page.getByRole("dialog", { name: "离开当前页面？", exact: true });
export const library = (page: Page) => page.getByRole("navigation", { name: "主导航", exact: true }).getByRole("link", { name: "资料库", exact: true });
export async function boot(browser: Browser, backend: Pc05bBackend, contexts: BrowserContext[], width = 1440) {
  expect(browser.version(), "Use the actual current Chrome154 channel, never bundled Chromium148").toMatch(/^154\./);
  const context = await browser.newContext({ viewport: { width, height: 1000 } }); contexts.push(context);
  const storage = { attemptedCanaryWrites: 0, storedCanaries: 0 };
  await context.exposeBinding("qa5bStorageViolation", () => { storage.attemptedCanaryWrites++; });
  await context.addInitScript(({ prefix, secret }) => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function(key: string, value: string) {
      if (String(key).includes(prefix) || String(value).includes(prefix) || String(value).includes(secret)) {
        const callback = (window as unknown as { qa5bStorageViolation: () => Promise<void> }).qa5bStorageViolation;
        void callback();
      }
      return original.call(this, key, value);
    };
  }, { prefix: CANARY, secret: SECRET_CANARY });
  const page = await context.newPage(); const state = await attach(page, backend); await loginViaUi(page, WEB, PASSWORD);
  const ua = await page.evaluate(() => navigator.userAgent); expect(ua).toMatch(/Chrome\/154\./); expect(ua).not.toContain("Headless");
  evidence("browser-identity", { actualVersion: browser.version(), nativeUserAgent: ua, headed: true, channel: "chrome", trace: false, har: false, video: false });
  return { context, page, state, storage, async verifyStorage() {
    storage.storedCanaries = await page.evaluate(({ prefix, secret }) => [localStorage, sessionStorage].reduce((n, s) => n + Object.keys(s).filter(k => k.includes(prefix) || (s.getItem(k) ?? "").includes(prefix) || (s.getItem(k) ?? "").includes(secret)).length, 0), { prefix: CANARY, secret: SECRET_CANARY });
    expect(storage).toEqual({ attemptedCanaryWrites: 0, storedCanaries: 0 }); return { ...storage };
  } };
}
export async function tabTo(page: Page, target: Locator) {
  for (let i = 0; i < 160; i++) { if (await target.evaluate(e => e === document.activeElement)) return; await page.keyboard.press("Tab"); }
  throw new Error("PC05B target unreachable with keyboard Tab");
}
export async function enter(page: Page, target: Locator) { await tabTo(page, target); await page.keyboard.press("Enter"); }
export async function prompt(page: Page, accept = false, acceptLabel = "离开页面") {
  const dialog = leaveDialog(page); await expect(dialog).toBeVisible(); await expect(dialog).toHaveCount(1); await expect(page.locator("dialog[open]")).toHaveCount(1);
  const cancel = dialog.getByRole("button", { name: "继续处理", exact: true }); await expect(cancel).toBeFocused();
  if (accept) await dialog.getByRole("button", { name: acceptLabel, exact: true }).click(); else await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
}
export async function promptRing(page: Page) {
  const dialog = leaveDialog(page), first = dialog.getByRole("button", { name: "继续处理", exact: true }), last = dialog.getByRole("button", { name: "离开页面", exact: true });
  await expect(first).toBeFocused(); await page.keyboard.press("Shift+Tab"); await expect(last).toBeFocused();
  await page.keyboard.press("Tab"); await expect(first).toBeFocused();
  await page.keyboard.press("Tab"); await expect(last).toBeFocused();
  await page.keyboard.press("Tab"); await expect(first).toBeFocused();
  await page.keyboard.press("Shift+Tab"); await expect(last).toBeFocused();
  {
    const cdp = await page.context().newCDPSession(page);
    try {
      const tree = await cdp.send("Accessibility.getFullAXTree");
      const activeDialogs = tree.nodes.filter(node => !node.ignored && node.role?.value === "dialog").map(node => ({ name: node.name?.value, modal: node.properties?.find(p => p.name === "modal")?.value.value }));
      const domDialogs = await page.getByRole("dialog").evaluateAll(elements => elements.map(el => ({ tag: el.tagName, ariaLabel: el.getAttribute("aria-label"), labelledby: el.getAttribute("aria-labelledby"), ariaModal: el.getAttribute("aria-modal"), explicitInert: el.hasAttribute("inert"), nativeModal: el.matches(":modal"), ownsFocus: el.contains(document.activeElement) })));
      evidence(`dialog-diagnostic-${page.viewportSize()?.width}-${domDialogs.length}`, { activeDialogs, domDialogs, forwardAndReverseRingPassed: true });
      expect(activeDialogs).toEqual([{ name: "离开当前页面？", modal: true }]);
    } finally { await cdp.detach(); }
  }
  await expect(page.locator("dialog:modal")).toHaveCount(1);
  await expect(dialog).toHaveCount(1);
}
/** A real request boundary. before holds without reaching server; committed fetches the real response first. */
export async function gate(page: Page, backend: Pc05bBackend, pathname: string, method: string, phase: "before" | "committed" = "before") {
  let release: (outcome: "forward" | "abort") => void = () => {};
  const released = new Promise<"forward" | "abort">(resolve => { release = resolve; });
  const state = { count: 0, held: false, settled: false, status: null as number | null, ifMatch: null as string | null };
  const pattern = "**" + pathname;
  const handler = async (route: Route) => {
    if (route.request().method() !== method) { await route.fallback(); return; }
    state.count++; state.ifMatch = route.request().headers()["if-match"] ?? null;
    const response = phase === "committed" ? await route.fetch({ url: backend.base + pathname }) : null;
    state.status = response?.status() ?? null; state.held = true;
    const outcome = await released;
    try { if (outcome === "abort") await route.abort("failed"); else if (response) await route.fulfill({ response }); else await route.fallback(); }
    catch { /* A user may cancel the held client request; persisted facts are asserted separately. */ }
    finally { state.settled = true; }
  };
  await page.route(pattern, handler);
  return { state, release, async remove() { release("abort"); if (!page.isClosed()) await page.unroute(pattern, handler); } };
}
export async function nativeReload(page: Page, accept: boolean) {
  const seen: string[] = []; const handler = async (dialog: Dialog) => { seen.push(dialog.type()); if (accept) await dialog.accept(); else await dialog.dismiss(); };
  page.on("dialog", handler);
  try {
    // Real user activation; no synthetic BeforeUnloadEvent, route mock, or listener substitution.
    await page.locator("h1").first().click();
    const marker = `qa-document-${Date.now()}-${Math.random()}`;
    await page.evaluate(value => { (window as unknown as { __qa5bDocument: string }).__qa5bDocument = value; }, marker);
    let canceledNavigationWait = false;
    const reload = page.reload({ waitUntil: "domcontentloaded", timeout: accept ? 15000 : 3000 }).catch(error => {
      if (accept || !(String(error).includes("ERR_ABORTED") || (error instanceof Error && error.name === "TimeoutError"))) throw error;
      canceledNavigationWait = true;
    });
    await expect.poll(() => seen).toEqual(["beforeunload"]); await reload;
    const current = await page.evaluate(() => (window as unknown as { __qa5bDocument?: string }).__qa5bDocument);
    if (accept) expect(current).not.toBe(marker); else expect(current).toBe(marker);
    await expect(leaveDialog(page)).toBeHidden(); return { accepted: accept, types: seen, canceledNavigationWait, sameDocument: current === marker };
  } finally { page.off("dialog", handler); }
}
export async function nativeClose(page: Page) {
  const seen: string[] = []; page.once("dialog", async dialog => { seen.push(dialog.type()); await dialog.accept(); });
  await page.locator("h1").first().click(); const closed = page.waitForEvent("close"); await page.close({ runBeforeUnload: true }); await closed;
  expect(seen).toEqual(["beforeunload"]); return { accepted: true, types: seen };
}
export async function historyNavigation(page: Page, direction: "back" | "forward") {
  try { if (direction === "back") await page.goBack({ waitUntil: "commit" }); else await page.goForward({ waitUntil: "commit" }); }
  catch (error) { if (!String(error).includes("ERR_ABORTED")) throw error; }
  // A canceled browser navigation can reject in Playwright. Callers still assert dialog, route,
  // input, focus and history length; suppressing ERR_ABORTED is not a passing product verdict.
}
export async function noPromptReload(page: Page) {
  const seen: string[] = []; const handler = async (dialog: Dialog) => { seen.push(dialog.type()); await dialog.dismiss(); }; page.on("dialog", handler);
  try { await page.locator("h1").first().click(); await page.reload({ waitUntil: "domcontentloaded" }); expect(seen).toEqual([]); await expect(leaveDialog(page)).toBeHidden(); }
  finally { page.off("dialog", handler); }
}
/** Revoke only the owned browser's actual backend session. Next UI request really receives 401. */
export async function expireSession(page: Page, backend: Pc05bBackend) {
  const session = await page.context().request.get(backend.base + "/api/v1/auth/session"); expect(session.status()).toBe(200);
  const csrf = (await session.json()).data.csrfToken as string;
  expect((await page.context().request.post(backend.base + "/api/v1/auth/logout", { headers: { "x-csrf-token": csrf } })).status()).toBe(204);
}
export async function loginInDocument(page: Page) {
  await expect(page).toHaveURL(/\/login\?next=/); await expect(leaveDialog(page)).toBeHidden();
  await page.getByLabel("密码", { exact: true }).fill(PASSWORD); await page.getByRole("button", { name: "登录", exact: true }).click();
  await expect(page).not.toHaveURL(/\/login/); // No goto/reload: preserve the same SPA memory.
}
