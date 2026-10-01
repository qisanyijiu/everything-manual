# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: interaction-a-qa.spec.ts >> IA-QA-03 确认延迟/取消/失败、低预算及同键显式重试（AC-005/006/007/008）
- Location: tests/e2e/interaction-a-qa.spec.ts:198:1

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: getByText(/正在保存确认/)
Expected: visible
Error: strict mode violation: getByText(/正在保存确认/) resolved to 2 elements:
    1) <p role="status" class="empty-note">正在保存确认…</p> aka getByText('正在保存确认…')
    2) <p id="generate-reason" data-testid="generate-reason" class="confirm-actions__reason">正在保存确认，请稍候再开始生成。</p> aka getByTestId('generate-reason')

Call log:
  - Expect "toBeVisible" with timeout 15000ms
  - waiting for getByText(/正在保存确认/)

```

# Page snapshot

```yaml
- generic [ref=e2]:
  - generic [ref=e3]:
    - link "跳到主要内容" [ref=e4] [cursor=pointer]:
      - /url: "#main"
    - complementary "应用导航" [ref=e5]:
      - link "万物说明书" [ref=e6] [cursor=pointer]:
        - /url: /
        - img [ref=e8]
        - generic [ref=e10]:
          - text: 万物说明书
          - generic [ref=e11]: EVERYTHING MANUAL
      - paragraph [ref=e12]: 我的工作空间
      - navigation "主导航" [ref=e13]:
        - link "资料库" [ref=e14] [cursor=pointer]:
          - /url: /
          - img [ref=e15]
          - generic [ref=e17]: 资料库
        - link "任务中心" [ref=e18] [cursor=pointer]:
          - /url: /jobs
          - img [ref=e19]
          - generic [ref=e21]: 任务中心
        - link "设置" [ref=e22] [cursor=pointer]:
          - /url: /settings
          - img [ref=e23]
          - generic [ref=e25]: 设置
      - generic [ref=e26]:
        - generic [ref=e27]:
          - img [ref=e28]
          - paragraph [ref=e30]:
            - text: 你的物品，你的资料。
            - generic [ref=e31]: 自托管 · 本地保存
        - button "登出" [ref=e32] [cursor=pointer]:
          - generic [ref=e33]: 我
          - generic [ref=e34]:
            - text: 管理员
            - generic [ref=e35]: 登出
          - img [ref=e36]
    - banner [ref=e38]:
      - generic [ref=e39]:
        - link "工作空间" [ref=e40] [cursor=pointer]:
          - /url: /
        - img [ref=e41]
        - generic [ref=e43]: 资料库
        - paragraph [ref=e44]: IA QA 确认状态 · T09-sample-manual-text.pdf
      - generic [ref=e46]: 个人资料库
    - main [ref=e48]:
      - region "预算与隐私确认" [ref=e50]:
        - list "新建向导步骤" [ref=e51]:
          - listitem [ref=e52]:
            - link "基本信息" [ref=e53] [cursor=pointer]:
              - /url: /items/01a0ba62-3f08-75a4-9de3-70ba7ce23358/edit
          - listitem [ref=e54]:
            - link "说明书" [ref=e55] [cursor=pointer]:
              - /url: /items/01a0ba62-3f08-75a4-9de3-70ba7ce23358/import/document
          - listitem [ref=e56]:
            - link "视图排列" [ref=e57] [cursor=pointer]:
              - /url: /items/01a0ba62-3f08-75a4-9de3-70ba7ce23358/import/views
          - listitem [ref=e58]:
            - link "准备" [ref=e59] [cursor=pointer]:
              - /url: /items/01a0ba62-3f08-75a4-9de3-70ba7ce23358/import/prepare
          - listitem [ref=e60]:
            - generic [ref=e61]: 预算/隐私确认
        - heading "预算与隐私确认" [level=1] [ref=e62]
        - paragraph [ref=e63]: IA QA 确认状态：报价只计算计划、不调用生成服务；确认后才允许提交（生成在后台继续执行）。
        - status [ref=e64]: 资料已齐：准备已封存，视图满足 front + 侧面至少一张。
        - region "报价与预算" [ref=e65]:
          - heading "报价与预算" [level=2] [ref=e66]
          - generic [ref=e67]:
            - generic [ref=e68]:
              - generic [ref=e69]:
                - term [ref=e70]: Tripo（credits）
                - definition [ref=e71]:
                  - text: 30.00 credits
                  - generic [ref=e72]: 保守上界
                - definition [ref=e73]: 预计 30.00 credits
              - generic [ref=e74]:
                - term [ref=e75]: 说明书 AI（USD）
                - definition [ref=e76]:
                  - text: 0.008504 USD
                  - generic [ref=e77]: 保守上界
                - definition [ref=e78]: 预计 0.001679 USD
            - paragraph [ref=e79]: 两个供应商的金额分列显示，不相加、不换算成同一币种。
            - generic [ref=e80]:
              - generic [ref=e81]:
                - term [ref=e82]: 价格版本
                - definition [ref=e83]: 2026-09-11
              - generic [ref=e84]:
                - term [ref=e85]: 快照日期
                - definition [ref=e86]: 2026-09-11
              - generic [ref=e87]:
                - term [ref=e88]: 页数 / 页范围
                - definition [ref=e89]: 1 页（1–1）
              - generic [ref=e90]:
                - term [ref=e91]: 有效期
                - definition [ref=e92]: 剩余 10 分 0 秒（至 2026-09-20 00:06）
            - heading "本次授权上限" [level=3] [ref=e93]
            - paragraph [ref=e94]: 默认等于服务端计算的保守上界；低于上界会被服务端拒绝（不自动降质量、不换模型）。
            - generic [ref=e95]:
              - generic [ref=e96]:
                - generic [ref=e97]: Tripo credits
                - textbox "Tripo credits" [ref=e98]: "30.00"
              - generic [ref=e99]:
                - generic [ref=e100]: 说明书 AI USD
                - textbox "说明书 AI USD" [ref=e101]: "0.008504"
            - paragraph [ref=e102]: 预算上限只表示本应用不会主动发起超出本次授权估算的请求，不是供应商账户级硬封顶；供应商实际计费以账单为准
        - generic [ref=e103]:
          - generic [ref=e104]:
            - heading "将发送的资料与确认" [level=2] [ref=e105]
            - heading "发送给 Tripo（模型生成）" [level=3] [ref=e106]
            - list [ref=e107]:
              - listitem [ref=e108]:
                - text: 正面（front）照片：
                - code [ref=e109]: 01a0ba62…
                - text: sha256
                - code [ref=e110]: 122e64539103…
              - listitem [ref=e111]:
                - text: 左侧（left）照片：
                - code [ref=e112]: 01a0ba62…
                - text: sha256
                - code [ref=e113]: 0a74e5a5b542…
            - paragraph [ref=e114]: 模型：v3.1-20260211（预设 tripo-h-v3.1-standard）；参数：face_limit 100000、texture true、pbr true、standard/standard。 detail（特写）照片不发送。
            - heading "发送给说明书 AI" [level=3] [ref=e115]
            - list [ref=e116]:
              - listitem [ref=e117]: 物品身份文本：IA QA 确认状态 · T09-sample-manual-text.pdf
              - listitem [ref=e118]: 页范围：第 1–1 页（共 1 页）
              - listitem [ref=e119]: 页文字页：第 1 页
              - listitem [ref=e120]: 页图页（扫描/无文字层）：无
              - listitem [ref=e121]: 模型：gpt-5-mini（prompt 版本 manual_extract_v1）；最大输出 token：4096
            - paragraph [ref=e122]: 价格版本 2026-09-11（快照 2026-09-11）；本次保守上界：Tripo 30.00 credits、说明书 AI 0.008504 USD（分列，不相加）。
            - paragraph [ref=e123]: 预算上限只表示本应用不会主动发起超出本次授权估算的请求，不是供应商账户级硬封顶；供应商实际计费以账单为准
          - generic [ref=e124]:
            - heading "云端发送确认" [level=3] [ref=e125]
            - generic [ref=e126]:
              - checkbox "我已阅读并确认将上述资料发送给对应供应商" [checked] [disabled] [ref=e127]
              - generic [ref=e128] [cursor=pointer]: 我已阅读并确认将上述资料发送给对应供应商
            - paragraph [ref=e129]: 请核对资料与接收方。确认保存成功后，才能按本次授权上限开始生成。
            - status [ref=e130]: 正在保存确认…
        - generic [ref=e131]:
          - button "生成 3D 与说明书草稿" [disabled] [ref=e132]
          - paragraph [ref=e133]: 正在保存确认，请稍候再开始生成。
  - generic "全局通知"
```

# Test source

```ts
  115 |   await touchTarget(page.getByRole("button", { name: "创建并继续", exact: true }));
  116 |   await noOverflow(page);
  117 |   await screenshot(page, "01-create-375");
  118 |   const gate = deferred();
  119 |   let creates = 0;
  120 |   let fail = true;
  121 |   await page.route("**/api/v1/items", async (route) => {
  122 |     if (route.request().method() !== "POST") { await route.continue(); return; }
  123 |     creates += 1;
  124 |     if (fail) {
  125 |       await gate.promise;
  126 |       await route.fulfill({ status: 422, json: { error: { code: "VALIDATION_FAILED", message: "QA 创建字段错误", details: { fields: [{ field: "name", message: "QA 名称错误" }] }, requestId: "ia-qa-create" } } });
  127 |     } else await route.continue();
  128 |   });
  129 |   const submit = page.getByRole("button", { name: "创建并继续", exact: true });
  130 |   await submit.click();
  131 |   await expect(page.locator(".item-form button[type=submit]")).toBeDisabled();
  132 |   await page.locator(".item-form").evaluate((form) => form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  133 |   expect(creates).toBe(1);
  134 |   gate.resolve();
  135 |   await expect(page.getByText("QA 名称错误").first()).toBeVisible();
  136 |   await expect(page.getByLabel(/^名称/)).toHaveValue("IA QA 创建路线");
  137 |   await expect(page).toHaveURL(/\/items\/new$/);
  138 |   fail = false;
  139 |   await submit.click();
  140 |   await expect(page).toHaveURL(/\/items\/[^/]+\/import\/document$/);
  141 |   await expect(page.getByRole("heading", { name: /说明书原件/ })).toBeVisible();
  142 |   expect(creates).toBe(2);
  143 |   const itemId = new URL(page.url()).pathname.split("/")[2];
  144 |   await page.goto(`/items/${itemId}/edit`);
  145 |   await page.getByLabel(/^名称/).fill("IA QA 已修改");
  146 |   await page.getByRole("button", { name: "保存", exact: true }).click();
  147 |   await expect(page).toHaveURL(new RegExp(`/items/${itemId}$`));
  148 |   await screenshot(page, "01-create-edit-routes");
  149 | });
  150 | 
  151 | test("IA-QA-02 四尺寸确认主流程与字体/触控（AC-003/004/011/012）", async ({ page, request }) => {
  152 |   const seed = await readyItem(request, "IA QA 四尺寸与长型号文字校验物品");
  153 |   await openConfirm(page, seed);
  154 |   const measurements: unknown[] = [];
  155 |   for (const width of [375, 768, 1024, 1440]) {
  156 |     await page.setViewportSize({ width, height: 900 });
  157 |     await expect(page.getByTestId("quote-panel")).toBeVisible();
  158 |     await expect(page.getByTestId("send-scope")).toBeVisible();
  159 |     await expect(page.getByRole("checkbox", { name: SCOPE_LABEL })).toHaveCount(1);
  160 |     await expect(page.locator(".page-layout__panel-bar")).toHaveCount(0);
  161 |     const order = await page.evaluate(() => {
  162 |       const quote = document.querySelector('[data-testid="quote-panel"]')!;
  163 |       const scope = document.querySelector('[data-testid="send-scope"]')!;
  164 |       const confirmation = document.querySelector('[data-testid="confirmation-box"]')!;
  165 |       const generate = document.querySelector('[data-testid="generate-button"]')!;
  166 |       return [quote, scope, confirmation].map((el, index) => Boolean(el.compareDocumentPosition([scope, confirmation, generate][index]!) & Node.DOCUMENT_POSITION_FOLLOWING));
  167 |     });
  168 |     expect(order).toEqual([true, true, true]);
  169 |     await expect(page.getByTestId("quote-tripo-upper")).toContainText("credits");
  170 |     await expect(page.getByTestId("quote-manual-upper")).toContainText("USD");
  171 |     await expect(page.getByTestId("send-scope")).toContainText("Tripo");
  172 |     await expect(page.getByTestId("send-scope")).toContainText("页文字页");
  173 |     await expect(page.getByTestId("send-scope")).toContainText("页图页");
  174 |     await noOverflow(page);
  175 |     await fontAtLeast(page.locator(".confirm-step input[type=text], .confirm-step .scope-list li"), 14);
  176 |     await fontAtLeast(page.locator(".confirm-step .field__hint, .confirm-actions__reason, .amount-list__label, .confirm-step .meta-list"), 12);
  177 |     if (width === 375) {
  178 |       await touchTarget(page.getByTestId("generate-button"));
  179 |       await page.locator("#budget-tripo").focus();
  180 |       await page.keyboard.press("Tab");
  181 |       await expect(page.locator("#budget-manual")).toBeFocused();
  182 |       await page.keyboard.press("Tab");
  183 |       // Any optional technical details may precede the checkbox; keyboard must reach it.
  184 |       for (let n = 0; n < 8 && !(await page.getByRole("checkbox", { name: SCOPE_LABEL }).evaluate((el) => el === document.activeElement)); n += 1) await page.keyboard.press("Tab");
  185 |       await expect(page.getByRole("checkbox", { name: SCOPE_LABEL })).toBeFocused();
  186 |       expect(await page.getByRole("checkbox", { name: SCOPE_LABEL }).evaluate((el) => {
  187 |         const box = el.getBoundingClientRect();
  188 |         const target = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
  189 |         return target === el;
  190 |       })).toBe(true);
  191 |     }
  192 |     measurements.push(await page.evaluate(() => ({ width: innerWidth, scrollWidth: document.documentElement.scrollWidth, checkbox: document.querySelector<HTMLInputElement>("#send-scope-confirm")?.checked })));
  193 |     await screenshot(page, `02-confirm-${width}`);
  194 |   }
  195 |   fs.writeFileSync(path.join(QA_DIR, "02-layout-measurements.json"), JSON.stringify(measurements, null, 2));
  196 | });
  197 | 
  198 | test("IA-QA-03 确认延迟/取消/失败、低预算及同键显式重试（AC-005/006/007/008）", async ({ page, request }) => {
  199 |   const seed = await readyItem(request, "IA QA 确认状态");
  200 |   await openConfirm(page, seed);
  201 |   const checkbox = page.getByRole("checkbox", { name: SCOPE_LABEL });
  202 |   const generate = page.getByTestId("generate-button");
  203 |   await expect(checkbox).not.toBeChecked();
  204 |   await expect(generate).toBeDisabled();
  205 |   let gate = deferred();
  206 |   let failConfirmation = false;
  207 |   let confirms = 0;
  208 |   await page.route(/\/estimates\/[^/]+\/confirm$/, async (route) => {
  209 |     confirms += 1;
  210 |     await gate.promise;
  211 |     if (failConfirmation) await route.fulfill({ status: 500, json: { error: { code: "INTERNAL", message: "QA 确认保存失败", details: null, requestId: "ia-qa-confirm" } } });
  212 |     else await route.continue();
  213 |   });
  214 |   await checkbox.check();
> 215 |   await expect(page.getByText(/正在保存确认/)).toBeVisible();
      |                                          ^ Error: expect(locator).toBeVisible() failed
  216 |   await expect(generate).toBeDisabled();
  217 |   gate.resolve();
  218 |   await expect(generate).toBeEnabled();
  219 |   await checkbox.uncheck();
  220 |   await expect(generate).toBeDisabled();
  221 |   gate = deferred();
  222 |   await checkbox.check();
  223 |   await expect(generate).toBeDisabled();
  224 |   await expect(page.getByText(/正在保存确认/)).toBeVisible();
  225 |   failConfirmation = true;
  226 |   gate.resolve();
  227 |   await expect(checkbox).not.toBeChecked();
  228 |   await expect(page.getByTestId("confirm-error")).toContainText("QA 确认保存失败");
  229 |   await expect(generate).toBeDisabled();
  230 |   failConfirmation = false;
  231 |   await checkbox.check();
  232 |   await expect(generate).toBeEnabled();
  233 |   expect(confirms).toBe(3);
  234 |   const tripo = page.locator("#budget-tripo");
  235 |   const original = await tripo.inputValue();
  236 |   await tripo.fill("abc");
  237 |   await expect(generate).toBeDisabled();
  238 |   await expect(page.getByTestId("generate-reason")).toContainText("格式");
  239 |   await tripo.fill("0");
  240 |   await expect(generate).toBeDisabled();
  241 |   await expect(page.getByTestId("generate-reason")).toContainText("低于");
  242 |   await tripo.fill(original);
  243 |   const keys: (string | null)[] = [];
  244 |   const submitGate = deferred();
  245 |   await page.route(`**/api/v1/items/${seed.itemId}/jobs`, async (route) => {
  246 |     if (route.request().method() !== "POST") { await route.continue(); return; }
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
```