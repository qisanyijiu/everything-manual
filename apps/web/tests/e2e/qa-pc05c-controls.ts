import { expect, type Browser, type BrowserContext, type Locator, type Page } from "@playwright/test";
import { attach, evidence, PASSWORD, Pc05cBackend, WEB } from "./qa-pc05c-backend";
import { loginViaUi } from "./helpers";
export const BUFFER_CANARY = "QA5C-MEMORY-";
export const SECRET_CANARY = "sk-pc05c-owned-model-canary-000000000000";
export async function boot(browser: Browser, backend: Pc05cBackend, contexts: BrowserContext[], width = 1440, login = true) {
  expect(browser.version(), "actual Chrome154 required").toMatch(/^154\./);
  const context = await browser.newContext({ viewport: { width, height: 1000 } }); contexts.push(context);
  const storage = { attempted: 0, stored: 0 };
  await context.exposeBinding("qa5cStorageViolation", () => { storage.attempted++; });
  await context.addInitScript(({ buffer, secret }) => {
    const setItem = Storage.prototype.setItem;
    Storage.prototype.setItem = function(key: string, value: string) {
      if ([key, value].some(v => String(v).includes(buffer) || String(v).includes(secret))) void (window as unknown as { qa5cStorageViolation: () => Promise<void> }).qa5cStorageViolation();
      return setItem.call(this, key, value);
    };
  }, { buffer: BUFFER_CANARY, secret: SECRET_CANARY });
  const page = await context.newPage(); const wire = await attach(page, backend);
  if (login) await loginViaUi(page, WEB, PASSWORD);
  const ua = await page.evaluate(() => navigator.userAgent); expect(ua).toMatch(/Chrome\/154\./); expect(ua).not.toContain("Headless");
  evidence("browser-identity", { version: browser.version(), nativeUA: ua, headed: true, channel: "chrome" });
  return { context, page, wire, async verify() {
    expect(wire.external).toBe(0); expect(wire.pageErrors).toBe(0);
    if (!page.isClosed() && page.url().startsWith(WEB)) storage.stored = await page.evaluate(({ buffer, secret }) => [localStorage, sessionStorage].reduce((n, s) => n + Object.keys(s).filter(k => [k, s.getItem(k) ?? ""].some(v => v.includes(buffer) || v.includes(secret))).length, 0), { buffer: BUFFER_CANARY, secret: SECRET_CANARY });
    expect(storage).toEqual({ attempted: 0, stored: 0 }); return { ...storage };
  } };
}
export async function tabTo(page: Page, target: Locator) {
  for (let i = 0; i < 160; i++) { if (await target.evaluate(e => e === document.activeElement)) return; await page.keyboard.press("Tab"); }
  throw new Error("PC05C keyboard target unreachable");
}
export async function expireSession(page: Page, b: Pc05cBackend) {
  const session = await page.context().request.get(b.base + "/api/v1/auth/session"); expect(session.status()).toBe(200);
  const csrf = (await session.json()).data.csrfToken as string;
  expect((await page.context().request.post(b.base + "/api/v1/auth/logout", { headers: { "x-csrf-token": csrf } })).status()).toBe(204);
}
export async function loginBack(page: Page) {
  await expect(page).toHaveURL(/\/login\?next=/); await expect(page.getByText("登录已过期，请重新登录。", { exact: false })).toBeVisible();
  await page.getByLabel("密码", { exact: true }).fill(PASSWORD); await page.getByRole("button", { name: "登录", exact: true }).click(); await expect(page).not.toHaveURL(/\/login/);
}
/** Real headed tab activation only. A platform that never hides is an explicit environment failure. */
export async function background(page: Page) {
  await page.evaluate(() => {
    const target = window as unknown as { __qaVisibility: Array<{ state: string; trusted: boolean; at: number }> };
    target.__qaVisibility = []; document.addEventListener("visibilitychange", event => target.__qaVisibility.push({ state: document.visibilityState, trusted: event.isTrusted, at: Date.now() }));
  });
  const cover = await page.context().newPage(); await cover.goto("about:blank"); await cover.bringToFront();
  await expect.poll(() => page.evaluate(() => document.visibilityState), { message: "must observe real Chrome hidden state; never dispatch a synthetic event" }).toBe("hidden");
  return async () => {
    await page.bringToFront(); await expect.poll(() => page.evaluate(() => document.visibilityState)).toBe("visible");
    const events = await page.evaluate(() => (window as unknown as { __qaVisibility: Array<{ state: string; trusted: boolean; at: number }> }).__qaVisibility);
    expect(events.some(e => e.state === "hidden" && e.trusted)).toBe(true); expect(events.some(e => e.state === "visible" && e.trusted)).toBe(true);
    await cover.close(); return events;
  };
}
