# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: interaction-a-qa.spec.ts >> IA-QA-05 阅读器键盘标签/抽屉与手机触控（AC-009/010/011/012）
- Location: tests/e2e/interaction-a-qa.spec.ts:305:1

# Error details

```
Error: expect(locator).toBeFocused() failed

Locator:  getByRole('dialog', { name: '步骤与原文' }).getByTestId('original-text').locator('summary')
Expected: focused
Received: inactive
Timeout:  15000ms

Call log:
  - Expect "toBeFocused" with timeout 15000ms
  - waiting for getByRole('dialog', { name: '步骤与原文' }).getByTestId('original-text').locator('summary')
    34 × locator resolved to <summary>本页文字（PDF 文字层）</summary>
       - unexpected value "inactive"

```

```yaml
- text: 本页文字（PDF 文字层）
```

# Test source

```ts
  247 |     keys.push(await route.request().headerValue("idempotency-key"));
  248 |     if (keys.length === 1) { await submitGate.promise; await route.abort("connectionfailed"); }
  249 |     else await route.continue();
  250 |   });
  251 |   await generate.click();
  252 |   await expect(generate).toBeDisabled();
  253 |   expect(keys).toHaveLength(1);
  254 |   submitGate.resolve();
  255 |   await expect(page.getByTestId("submit-error")).toBeVisible();
  256 |   await expect(generate).toBeEnabled();
  257 |   await page.waitForTimeout(300);
  258 |   expect(keys).toHaveLength(1);
  259 |   await generate.click();
  260 |   await expect(page.getByTestId("job-accepted")).toBeVisible();
  261 |   expect(keys).toHaveLength(2);
  262 |   expect(keys[0]).toBeTruthy();
  263 |   expect(keys[1]).toBe(keys[0]);
  264 |   expect(await fetchJobsForItem(request, runtime().apiBase, seed.itemId)).toHaveLength(1);
  265 |   await expect(page.getByTestId("job-accepted")).toContainText("服务");
  266 |   await expect(page.getByRole("link", { name: /查看任务详情/ })).toBeVisible();
  267 |   await screenshot(page, "03-confirm-accepted");
  268 | });
  269 | 
  270 | test("IA-QA-04 过期手动重报与旧确认晚到隔离（AC-005/007）", async ({ page, request }) => {
  271 |   const seed = await readyItem(request, "IA QA 过期旧响应");
  272 |   let quotes = 0;
  273 |   await page.route(`**/api/v1/items/${seed.itemId}/estimates`, async (route) => {
  274 |     quotes += 1;
  275 |     const response = await route.fetch();
  276 |     const json = await response.json();
  277 |     if (quotes === 1) json.data.expiresAt = new Date(Date.now() + 2500).toISOString();
  278 |     await route.fulfill({ response, json });
  279 |   });
  280 |   await openConfirm(page, seed);
  281 |   const pending = deferred();
  282 |   let oldReplyReady = false;
  283 |   await page.route(/\/estimates\/[^/]+\/confirm$/, async (route) => {
  284 |     const response = await route.fetch();
  285 |     oldReplyReady = true;
  286 |     await pending.promise;
  287 |     await route.fulfill({ response });
  288 |   });
  289 |   await page.getByRole("checkbox", { name: SCOPE_LABEL }).check();
  290 |   await expect.poll(() => oldReplyReady).toBe(true);
  291 |   await expect(page.getByTestId("quote-expiry")).toContainText("已过期", { timeout: 10000 });
  292 |   expect(quotes).toBe(1);
  293 |   await expect(page.getByTestId("generate-button")).toBeDisabled();
  294 |   await page.getByTestId("requote-button").click();
  295 |   await expect.poll(() => quotes).toBe(2);
  296 |   await expect(page.getByRole("checkbox", { name: SCOPE_LABEL })).not.toBeChecked();
  297 |   pending.resolve();
  298 |   await page.waitForTimeout(300);
  299 |   await expect(page.getByRole("checkbox", { name: SCOPE_LABEL })).not.toBeChecked();
  300 |   await expect(page.getByTestId("generate-button")).toBeDisabled();
  301 |   await expect(page.getByTestId("confirmed-at")).toHaveCount(0);
  302 |   await screenshot(page, "04-stale-confirm-response");
  303 | });
  304 | 
  305 | test("IA-QA-05 阅读器键盘标签/抽屉与手机触控（AC-009/010/011/012）", async ({ page, request }) => {
  306 |   const seed = await readyItem(request, "IA QA 阅读器可达性");
  307 |   await loginViaUi(page, "", runtime().password);
  308 |   const url = await installReaderFixture(page, seed);
  309 |   await page.setViewportSize({ width: 1024, height: 900 });
  310 |   await page.goto(url);
  311 |   await page.getByRole("button", { name: "显示步骤与原文" }).click();
  312 |   const parts = page.getByRole("tab", { name: "部件", exact: true });
  313 |   const steps = page.getByRole("tab", { name: "步骤与原文", exact: true });
  314 |   await parts.focus();
  315 |   for (const [key, target] of [["ArrowLeft", steps], ["ArrowRight", parts], ["End", steps], ["Home", parts], ["ArrowRight", steps]] as const) {
  316 |     await page.keyboard.press(key);
  317 |     await expect(target).toBeFocused();
  318 |     await expect(target).toHaveAttribute("aria-selected", "true");
  319 |     await expect(target).toHaveAttribute("tabindex", "0");
  320 |     await expect(page.locator('[role="tab"][tabindex="0"]')).toHaveCount(1);
  321 |   }
  322 |   await page.keyboard.press("Tab");
  323 |   expect(await page.getByRole("tabpanel").evaluate((el) => el.contains(document.activeElement))).toBe(true);
  324 |   await expect(page.getByTestId("original-page-label")).toContainText("/ 2", { timeout: 20000 });
  325 |   await parts.click();
  326 |   await expect(page.getByTestId("parts-panel")).toBeVisible();
  327 |   await steps.click();
  328 |   await fontAtLeast(page.locator(".step-detail li"), 14);
  329 |   await fontAtLeast(page.locator(".step-evidence button, .original-panel__nav, .step-nav"), 12);
  330 |   await screenshot(page, "05-reader-keyboard-mid");
  331 | 
  332 |   await page.setViewportSize({ width: 375, height: 812 });
  333 |   const trigger = page.getByRole("button", { name: "步骤与原文", exact: true });
  334 |   await touchTarget(trigger);
  335 |   await trigger.click();
  336 |   const dialog = page.getByRole("dialog", { name: "步骤与原文" });
  337 |   await expect(dialog).toHaveCount(1);
  338 |   await expect(dialog.getByTestId("original-page-label")).toContainText("/ 2", { timeout: 20000 });
  339 |   const textLayer = dialog.getByTestId("original-text");
  340 |   await expect(textLayer).toBeVisible();
  341 |   await textLayer.locator("summary").click();
  342 |   await expect(textLayer.locator("pre")).not.toBeEmpty();
  343 |   await fontAtLeast(textLayer.locator("summary, pre"), 12);
  344 |   for (const target of [dialog.getByRole("button", { name: "关闭", exact: true }), dialog.getByRole("button", { name: "上一步", exact: true }), dialog.getByRole("button", { name: "下一步", exact: true }), dialog.getByRole("button", { name: "上一页", exact: true }), dialog.getByRole("button", { name: "下一页", exact: true }), dialog.locator(".step-evidence button").first()]) await touchTarget(target);
  345 |   await dialog.getByRole("button", { name: "下一页", exact: true }).focus();
  346 |   await page.keyboard.press("Tab");
> 347 |   await expect(textLayer.locator("summary")).toBeFocused();
      |                                              ^ Error: expect(locator).toBeFocused() failed
  348 |   const close = dialog.getByRole("button", { name: "关闭", exact: true });
  349 |   await close.focus();
  350 |   await page.keyboard.press("Shift+Tab");
  351 |   expect(await dialog.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  352 |   await page.keyboard.press("Tab");
  353 |   await expect(close).toBeFocused();
  354 |   await page.keyboard.press("Tab");
  355 |   expect(await dialog.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  356 |   await noOverflow(page);
  357 |   await screenshot(page, "05-reader-touch-375");
  358 |   await textLayer.scrollIntoViewIfNeeded();
  359 |   await screenshot(page, "05-reader-pdf-text-375");
  360 |   await page.keyboard.press("Escape");
  361 |   await expect(dialog).toHaveCount(0);
  362 |   await expect(trigger).toBeFocused();
  363 | });
  364 | 
  365 | test("IA-QA-06 实际浏览器 200% 缩放保持表单与确认可达（AC-012）", async ({ request }) => {
  366 |   const seed = await readyItem(request, "IA QA 真实浏览器缩放");
  367 |   const extension = path.join(import.meta.dirname, "fixtures", "interaction-a-qa-zoom");
  368 |   const profile = fs.mkdtempSync(path.join(os.tmpdir(), "interaction-a-qa-zoom-"));
  369 |   const context = await chromium.launchPersistentContext(profile, {
  370 |     channel: "chromium", headless: true, viewport: null, deviceScaleFactor: undefined, isMobile: undefined, locale: "zh-CN",
  371 |     baseURL: `http://127.0.0.1:${E2E_WEB_PORT}`,
  372 |     args: [`--disable-extensions-except=${extension}`, `--load-extension=${extension}`, "--window-size=1440,1000"],
  373 |     ignoreDefaultArgs: ["--disable-extensions"],
  374 |   });
  375 |   try {
  376 |     const worker = context.serviceWorkers()[0] ?? await context.waitForEvent("serviceworker");
  377 |     const page = context.pages()[0] ?? await context.newPage();
  378 |     const external: string[] = [];
  379 |     await page.route("**/*", async (route) => {
  380 |       const url = new URL(route.request().url());
  381 |       if ((url.protocol === "http:" || url.protocol === "https:") && !["127.0.0.1", "localhost"].includes(url.hostname)) {
  382 |         external.push(url.hostname); await route.abort();
  383 |       } else await route.continue();
  384 |     });
  385 |     await openConfirm(page, seed);
  386 |     const before = await page.evaluate(() => ({ innerWidth, outerWidth, dpr: devicePixelRatio }));
  387 |     const zoom = await worker.evaluate(`(async () => {
  388 |       const tabs = await chrome.tabs.query({});
  389 |       const tab = tabs.find((tab) => tab.url?.startsWith('http://127.0.0.1:${E2E_WEB_PORT}/'));
  390 |       if (!tab) throw new Error('QA tab missing');
  391 |       await chrome.tabs.setZoom(tab.id, 2);
  392 |       return await chrome.tabs.getZoom(tab.id);
  393 |     })()`);
  394 |     expect(zoom).toBe(2);
  395 |     await expect.poll(() => page.evaluate(() => devicePixelRatio)).toBe(before.dpr * 2);
  396 |     const after = await page.evaluate(() => ({ innerWidth, outerWidth, dpr: devicePixelRatio }));
  397 |     expect(after.outerWidth).toBe(before.outerWidth);
  398 |     expect(Math.abs(after.innerWidth * 2 - before.innerWidth)).toBeLessThanOrEqual(2);
  399 |     await noOverflow(page);
  400 |     await page.locator("#budget-tripo").focus();
  401 |     await page.keyboard.press("Tab");
  402 |     await expect(page.locator("#budget-manual")).toBeFocused();
  403 |     const check = page.getByRole("checkbox", { name: SCOPE_LABEL });
  404 |     for (let n = 0; n < 10 && !(await check.evaluate((el) => el === document.activeElement)); n += 1) await page.keyboard.press("Tab");
  405 |     await expect(check).toBeFocused();
  406 |     await page.keyboard.press("Space");
  407 |     await expect(page.getByTestId("generate-button")).toBeEnabled();
  408 |     await page.keyboard.press("Tab");
  409 |     await expect(page.getByTestId("generate-button")).toBeFocused();
  410 |     expect(await page.getByTestId("generate-button").evaluate((el) => {
  411 |       const rect = el.getBoundingClientRect();
  412 |       return el.contains(document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2));
  413 |     })).toBe(true);
  414 |     await screenshot(page, "06-confirm-browser-zoom-200");
  415 |     await page.goto("/items/new");
  416 |     await expect(page.getByRole("heading", { name: "新建物品" })).toBeVisible();
  417 |     await page.getByLabel(/^名称/).fill("缩放键盘输入");
  418 |     await page.keyboard.press("Tab");
  419 |     await expect(page.getByLabel(/^准确型号/)).toBeFocused();
  420 |     await page.keyboard.type("IA-ZOOM");
  421 |     await page.keyboard.press("Tab"); await page.keyboard.press("Tab"); await page.keyboard.press("Tab");
  422 |     await expect(page.getByRole("button", { name: "创建并继续" })).toBeFocused();
  423 |     await noOverflow(page);
  424 |     await screenshot(page, "06-create-browser-zoom-200");
  425 |     fs.writeFileSync(path.join(QA_DIR, "06-browser-zoom-evidence.json"), JSON.stringify({ method: "chrome.tabs.setZoom", zoom, before, after, browser: context.browser()?.version(), external }, null, 2));
  426 |     expect(external).toEqual([]);
  427 |   } finally {
  428 |     await context.close();
  429 |     fs.rmSync(profile, { recursive: true, force: true });
  430 |   }
  431 | });
  432 | 
  433 | test("IA-QA-07 缺项、配置未就绪、报价加载/失败均留在主流程（AC-004）", async ({ page, request }) => {
  434 |   const seed = await readyItem(request, "IA QA 报价状态");
  435 |   await loginViaUi(page, "", runtime().password);
  436 |   await page.setViewportSize({ width: 375, height: 812 });
  437 |   await page.goto(`/items/${seed.itemId}/import/confirm`);
  438 |   await expect(page.getByRole("link", { name: "去准备", exact: true })).toBeVisible();
  439 |   await expect(page.getByTestId("quote-panel")).toHaveCount(0);
  440 |   await expect(page.getByTestId("generate-button")).toBeDisabled();
  441 |   await setPreparationPointer(page, seed.itemId, seed.preparationId);
  442 |   await page.route("**/api/v1/settings/status", async (route) => {
  443 |     const response = await route.fetch();
  444 |     const json = await response.json();
  445 |     json.data.capabilities.generation = false;
  446 |     await route.fulfill({ response, json });
  447 |   });
```