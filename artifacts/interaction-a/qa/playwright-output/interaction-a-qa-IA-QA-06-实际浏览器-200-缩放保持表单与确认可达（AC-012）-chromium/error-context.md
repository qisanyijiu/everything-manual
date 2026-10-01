# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: interaction-a-qa.spec.ts >> IA-QA-06 实际浏览器 200% 缩放保持表单与确认可达（AC-012）
- Location: tests/e2e/interaction-a-qa.spec.ts:354:1

# Error details

```
Error: browserType.launchPersistentContext: "deviceScaleFactor" option is not supported with null "viewport"
Call log:
  - <launching> /Users/qsyj/Library/Caches/ms-playwright/chromium-1223/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing --disable-field-trial-config --disable-background-networking --disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-back-forward-cache --disable-breakpad --disable-client-side-phishing-detection --disable-component-extensions-with-background-pages --disable-component-update --no-default-browser-check --disable-default-apps --disable-dev-shm-usage --disable-edgeupdater --disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,BoundaryEventDispatchTracksNodeRemoval,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding,ThirdPartyStoragePartitioning,Translate,AutoDeElevate,RenderDocument,OptimizationHints,msForceBrowserSignIn,msEdgeUpdateLaunchServicesPreferredVersion --enable-features=CDPScreenshotNewSurface --allow-pre-commit-input --disable-hang-monitor --disable-ipc-flooding-protection --disable-popup-blocking --disable-prompt-on-repost --disable-renderer-backgrounding --force-color-profile=srgb --metrics-recording-only --no-first-run --password-store=basic --use-mock-keychain --no-service-autorun --export-tagged-pdf --disable-search-engine-choice-screen --unsafely-disable-devtools-self-xss-warnings --edge-skip-compat-layer-relaunch --disable-infobars --disable-search-engine-choice-screen --disable-sync --enable-unsafe-swiftshader --headless --hide-scrollbars --mute-audio --blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4 --no-sandbox --disable-extensions-except=/Users/qsyj/Code/rust/everything-manual/apps/web/tests/e2e/fixtures/interaction-a-qa-zoom --load-extension=/Users/qsyj/Code/rust/everything-manual/apps/web/tests/e2e/fixtures/interaction-a-qa-zoom --window-size=1440,1000 --user-data-dir=/var/folders/5y/y4jt01gd7mzglrwt5pf7p3m00000gn/T/interaction-a-qa-zoom-pRTIl4 --remote-debugging-pipe about:blank
  - <launched> pid=49152
  - [pid=49152] <gracefully close start>
  - [pid=49152][err] [49192:24380431:0919/235657.344144:ERROR:ui/display/mac/cv_display_link_mac.mm:184] CVDisplayLinkCreateWithCGDisplay failed. CVReturn: -6670
  - [pid=49152][err] [49192:24380503:0919/235657.344508:ERROR:ui/display/mac/cv_display_link_mac.mm:184] CVDisplayLinkCreateWithCGDisplay failed. CVReturn: -6670
  - [pid=49152][err] [49192:24380431:0919/235657.344870:ERROR:ui/display/mac/cv_display_link_mac.mm:184] CVDisplayLinkCreateWithCGDisplay failed. CVReturn: -6670
  - [pid=49152][err] [49192:24380431:0919/235657.345157:ERROR:ui/display/mac/cv_display_link_mac.mm:184] CVDisplayLinkCreateWithCGDisplay failed. CVReturn: -6670
  - [pid=49152][err] [49192:24380503:0919/235657.345232:ERROR:ui/display/mac/cv_display_link_mac.mm:184] CVDisplayLinkCreateWithCGDisplay failed. CVReturn: -6670
  - [pid=49152][err] [49192:24380431:0919/235657.345247:ERROR:ui/display/mac/cv_display_link_mac.mm:184] CVDisplayLinkCreateWithCGDisplay failed. CVReturn: -6670
  - [pid=49152][err] [49192:24380431:0919/235657.345489:ERROR:ui/display/mac/cv_display_link_mac.mm:184] CVDisplayLinkCreateWithCGDisplay failed. CVReturn: -6670
  - [pid=49152][err] [49192:24380503:0919/235657.345538:ERROR:ui/display/mac/cv_display_link_mac.mm:184] CVDisplayLinkCreateWithCGDisplay failed. CVReturn: -6670
  - [pid=49152][err] [49192:24380431:0919/235657.345555:ERROR:ui/display/mac/cv_display_link_mac.mm:184] CVDisplayLinkCreateWithCGDisplay failed. CVReturn: -6670
  - [pid=49152] <process did exit: exitCode=0, signal=null>
  - [pid=49152] starting temporary directories cleanup
  - [pid=49152] finished temporary directories cleanup
  - [pid=49152] <gracefully close end>

```

# Test source

```ts
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
  338 |   for (const target of [dialog.getByRole("button", { name: "关闭", exact: true }), dialog.getByRole("button", { name: "上一步", exact: true }), dialog.getByRole("button", { name: "下一步", exact: true }), dialog.getByRole("button", { name: "上一页", exact: true }), dialog.getByRole("button", { name: "下一页", exact: true }), dialog.locator(".step-evidence button").first()]) await touchTarget(target);
  339 |   const close = dialog.getByRole("button", { name: "关闭", exact: true });
  340 |   await close.focus();
  341 |   await page.keyboard.press("Shift+Tab");
  342 |   expect(await dialog.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  343 |   await page.keyboard.press("Tab");
  344 |   await expect(close).toBeFocused();
  345 |   await page.keyboard.press("Tab");
  346 |   expect(await dialog.evaluate((el) => el.contains(document.activeElement))).toBe(true);
  347 |   await noOverflow(page);
  348 |   await screenshot(page, "05-reader-touch-375");
  349 |   await page.keyboard.press("Escape");
  350 |   await expect(dialog).toHaveCount(0);
  351 |   await expect(trigger).toBeFocused();
  352 | });
  353 | 
  354 | test("IA-QA-06 实际浏览器 200% 缩放保持表单与确认可达（AC-012）", async ({ request }) => {
  355 |   const seed = await readyItem(request, "IA QA 真实浏览器缩放");
  356 |   const extension = path.join(import.meta.dirname, "fixtures", "interaction-a-qa-zoom");
  357 |   const profile = fs.mkdtempSync(path.join(os.tmpdir(), "interaction-a-qa-zoom-"));
> 358 |   const context = await chromium.launchPersistentContext(profile, {
      |                   ^ Error: browserType.launchPersistentContext: "deviceScaleFactor" option is not supported with null "viewport"
  359 |     channel: "chromium", headless: true, viewport: null, locale: "zh-CN",
  360 |     baseURL: `http://127.0.0.1:${E2E_WEB_PORT}`,
  361 |     args: [`--disable-extensions-except=${extension}`, `--load-extension=${extension}`, "--window-size=1440,1000"],
  362 |     ignoreDefaultArgs: ["--disable-extensions"],
  363 |   });
  364 |   try {
  365 |     const worker = context.serviceWorkers()[0] ?? await context.waitForEvent("serviceworker");
  366 |     const page = context.pages()[0] ?? await context.newPage();
  367 |     const external: string[] = [];
  368 |     await page.route("**/*", async (route) => {
  369 |       const url = new URL(route.request().url());
  370 |       if ((url.protocol === "http:" || url.protocol === "https:") && !["127.0.0.1", "localhost"].includes(url.hostname)) {
  371 |         external.push(url.hostname); await route.abort();
  372 |       } else await route.continue();
  373 |     });
  374 |     await openConfirm(page, seed);
  375 |     const before = await page.evaluate(() => ({ innerWidth, outerWidth, dpr: devicePixelRatio }));
  376 |     const zoom = await worker.evaluate(`(async () => {
  377 |       const tabs = await chrome.tabs.query({});
  378 |       const tab = tabs.find((tab) => tab.url?.startsWith('http://127.0.0.1:${E2E_WEB_PORT}/'));
  379 |       if (!tab) throw new Error('QA tab missing');
  380 |       await chrome.tabs.setZoom(tab.id, 2);
  381 |       return await chrome.tabs.getZoom(tab.id);
  382 |     })()`);
  383 |     expect(zoom).toBe(2);
  384 |     await expect.poll(() => page.evaluate(() => devicePixelRatio)).toBe(before.dpr * 2);
  385 |     const after = await page.evaluate(() => ({ innerWidth, outerWidth, dpr: devicePixelRatio }));
  386 |     expect(after.outerWidth).toBe(before.outerWidth);
  387 |     expect(Math.abs(after.innerWidth * 2 - before.innerWidth)).toBeLessThanOrEqual(2);
  388 |     await noOverflow(page);
  389 |     await page.locator("#budget-tripo").focus();
  390 |     await page.keyboard.press("Tab");
  391 |     await expect(page.locator("#budget-manual")).toBeFocused();
  392 |     const check = page.getByRole("checkbox", { name: SCOPE_LABEL });
  393 |     for (let n = 0; n < 10 && !(await check.evaluate((el) => el === document.activeElement)); n += 1) await page.keyboard.press("Tab");
  394 |     await expect(check).toBeFocused();
  395 |     await page.keyboard.press("Space");
  396 |     await expect(page.getByTestId("generate-button")).toBeEnabled();
  397 |     await page.keyboard.press("Tab");
  398 |     await expect(page.getByTestId("generate-button")).toBeFocused();
  399 |     expect(await page.getByTestId("generate-button").evaluate((el) => {
  400 |       const rect = el.getBoundingClientRect();
  401 |       return el.contains(document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2));
  402 |     })).toBe(true);
  403 |     await screenshot(page, "06-confirm-browser-zoom-200");
  404 |     await page.goto("/items/new");
  405 |     await expect(page.getByRole("heading", { name: "新建物品" })).toBeVisible();
  406 |     await page.getByLabel(/^名称/).fill("缩放键盘输入");
  407 |     await page.keyboard.press("Tab");
  408 |     await expect(page.getByLabel(/^准确型号/)).toBeFocused();
  409 |     await page.keyboard.type("IA-ZOOM");
  410 |     await page.keyboard.press("Tab"); await page.keyboard.press("Tab"); await page.keyboard.press("Tab");
  411 |     await expect(page.getByRole("button", { name: "创建并继续" })).toBeFocused();
  412 |     await noOverflow(page);
  413 |     await screenshot(page, "06-create-browser-zoom-200");
  414 |     fs.writeFileSync(path.join(QA_DIR, "06-browser-zoom-evidence.json"), JSON.stringify({ method: "chrome.tabs.setZoom", zoom, before, after, browser: context.browser()?.version(), external }, null, 2));
  415 |     expect(external).toEqual([]);
  416 |   } finally {
  417 |     await context.close();
  418 |     fs.rmSync(profile, { recursive: true, force: true });
  419 |   }
  420 | });
  421 | 
  422 | test("IA-QA-07 缺项、配置未就绪、报价加载/失败均留在主流程（AC-004）", async ({ page, request }) => {
  423 |   const seed = await readyItem(request, "IA QA 报价状态");
  424 |   await loginViaUi(page, "", runtime().password);
  425 |   await page.setViewportSize({ width: 375, height: 812 });
  426 |   await page.goto(`/items/${seed.itemId}/import/confirm`);
  427 |   await expect(page.getByRole("link", { name: "去准备", exact: true })).toBeVisible();
  428 |   await expect(page.getByTestId("quote-panel")).toHaveCount(0);
  429 |   await expect(page.getByTestId("generate-button")).toBeDisabled();
  430 |   await setPreparationPointer(page, seed.itemId, seed.preparationId);
  431 |   await page.route("**/api/v1/settings/status", async (route) => {
  432 |     const response = await route.fetch();
  433 |     const json = await response.json();
  434 |     json.data.capabilities.generation = false;
  435 |     await route.fulfill({ response, json });
  436 |   });
  437 |   await page.reload();
  438 |   await expect(page.getByTestId("generation-gaps")).toContainText("生成能力未就绪");
  439 |   await expect(page.getByRole("link", { name: "查看服务状态", exact: true })).toBeVisible();
  440 |   await expect(page.getByTestId("quote-panel")).toHaveCount(0);
  441 |   await screenshot(page, "07-config-not-ready");
  442 |   await page.unroute("**/api/v1/settings/status");
  443 |   const gate = deferred();
  444 |   let failed = true;
  445 |   await page.route(`**/api/v1/items/${seed.itemId}/estimates`, async (route) => {
  446 |     if (failed) {
  447 |       await gate.promise;
  448 |       await route.fulfill({ status: 500, json: { error: { code: "INTERNAL", message: "QA 报价暂不可用，请重试", details: null, requestId: "ia-qa-quote" } } });
  449 |     } else await route.continue();
  450 |   });
  451 |   await page.reload();
  452 |   await expect(page.getByText("正在获取报价…", { exact: true })).toBeVisible();
  453 |   await expect(page.getByTestId("quote-tripo-upper")).toHaveCount(0);
  454 |   gate.resolve();
  455 |   await expect(page.getByTestId("quote-error")).toContainText("QA 报价暂不可用，请重试");
  456 |   await expect(page.getByRole("button", { name: "重试获取报价", exact: true })).toBeVisible();
  457 |   await noOverflow(page);
  458 |   await screenshot(page, "07-quote-error");
```