/**
 * T17「任务中心与可行动错误」端到端验收（PRD 修订 2 / ui_revision 2）。
 *
 * 覆盖（卡内验证清单 ↔ 用例）：
 * 1. 列表与详情：状态/阶段/分列费用、无虚假总进度（UI-029/UI-030/UI-033/UI-038；AC-033 UI 侧）；
 * 2. **浏览器关闭后服务器继续**（关页面 → 服务端推进 → 重开看到新状态；AC-049）；
 * 3. **服务重启恢复**（重启后端 → 页面从数据库读到同一状态；AC-049）；
 * 4. `submission_unknown`：**不渲染重试按钮**、渲染对账入口、预留保留为未决（UI-034/UI-037；AC-037 UI 侧）；
 * 5. **重复操作不多收费**：重复点击/重放 → 付费提交计数不变（REQ-022/REQ-025）；
 * 6. `needs_input` 缺项与真实可执行动作一致（含 T15 P3①：被 `budgetNotHolding` 拒绝时
 *    不渲染重试、给"重新报价/新建任务"）+ 断网不误报业务失败（UI-031/UI-033；AC-049）；
 * 7. 失败阶段的错误摘要与不可重试原因（不误导）；
 * 8. 轮询频率：可见约 2 秒 / 后台约 15 秒 / 终态停止（AC-049 的观察点）。
 *
 * 设施：`job-recovery-harness.ts`（本机 fixture + 自管后端 + 造数）。
 * 浏览器侧通过路由改写把 `/api/v1/**` 指向本用例自己的后端（Vite 只提供 SPA），
 * 因此可以任意重启后端、隔离数据目录，且**零真实外网、零真实付费**。
 */

import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

import { apiLogin, captureTo } from "./helpers";
import { E2E_WEB_PORT } from "./runtime";
import {
  BACKEND_PASSWORD,
  LocalFixture,
  TestBackend,
  buildTestServerBinary,
  fetchJobDetail,
  seedJob,
  stageOf,
  waitForJob,
} from "./job-recovery-harness";

test.describe.configure({ timeout: 180_000 });

const WEB_BASE = `http://127.0.0.1:${E2E_WEB_PORT}`;

let fixture: LocalFixture;
let backend: TestBackend;

test.beforeAll(async () => {
  // 测试构建（仅它放行"明文 http + 回环"的模型下载）；冷构建可能较慢，给钩子足够预算。
  test.setTimeout(900_000);
  buildTestServerBinary();
  fixture = new LocalFixture();
  await fixture.start();
  backend = new TestBackend("job-recovery");
  await backend.start(fixture);
});

test.afterAll(async () => {
  await backend?.cleanup(fixture);
});

/**
 * 浏览器 ↔ 本用例后端的桥：
 * - 默认把 `/api/v1/**` 改写指到本用例自己的后端（Vite 只负责 SPA）；
 * - [`BackendBridge.setBlocked`] 模拟"断网"：后续 API 请求直接以
 *   `net::ERR_INTERNET_DISCONNECTED` 失败（与真实断网一致：fetch 抛 TypeError）。
 *   刻意不用 `context.setOffline`：那会同时掐断 Vite 的模块/HMR 连接，
 *   而这里要观察的是**业务请求网络失败**时的界面行为。
 */
interface BackendBridge {
  setBlocked(blocked: boolean): void;
}

async function attachBackend(page: Page, target: TestBackend): Promise<BackendBridge> {
  const state = { blocked: false };
  await page.route(`${WEB_BASE}/api/v1/**`, async (route) => {
    if (state.blocked) {
      await route.abort("internetdisconnected");
      return;
    }
    const url = route.request().url().replace(WEB_BASE, target.base);
    await route.continue({ url });
  });
  return {
    setBlocked: (blocked: boolean) => {
      state.blocked = blocked;
    },
  };
}

async function openApp(page: Page, path: string): Promise<BackendBridge> {
  const bridge = await attachBackend(page, backend);
  await page.goto(`${WEB_BASE}${path}`);
  return bridge;
}

/**
 * 真实登录页登录（不走注入 cookie 的捷径）。
 *
 * 定向打开受保护路由时会先被 `RequireSession` 挡到 `/login?next=…`，
 * 登录成功后回到原路径，因此这里等的是应用外壳（主导航）而不是某个具体页面。
 */
async function login(page: Page): Promise<void> {
  await page.getByLabel("密码").fill(BACKEND_PASSWORD);
  await page.getByRole("button", { name: "登录" }).click();
  await expect(page.getByRole("navigation", { name: "主导航" })).toBeVisible();
}

/** 计数前往任务接口的浏览器请求（轮询频率的观察点）。 */
function countJobRequests(page: Page): () => number {
  let count = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/v1/jobs")) {
      count += 1;
    }
  });
  return () => count;
}

async function setVisibility(page: Page, state: "visible" | "hidden"): Promise<void> {
  await page.evaluate((value) => {
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => value,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  }, state);
}

/** API 侧取消（轮询终态测试用；界面取消由用例 5 覆盖）。 */
async function cancelViaApi(request: APIRequestContext, jobId: string): Promise<void> {
  const csrf = await apiLogin(request, backend.base, BACKEND_PASSWORD);
  const detail = await fetchJobDetail(request, backend.base, jobId);
  expect(detail.etag).toBeTruthy();
  const response = await request.fetch(`${backend.base}/api/v1/jobs/${jobId}/cancel`, {
    method: "POST",
    headers: { "x-csrf-token": csrf, "if-match": detail.etag ?? "" },
  });
  expect(response.status(), await response.text()).toBe(200);
}

test("列表与详情：状态、阶段、分列费用，不出现虚假线性总进度", async ({ page, request }) => {
  fixture.state.manualMode = "success";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const { jobId } = await seedJob(request, backend, "T17-A 成功链路");
  await waitForJob(request, backend.base, jobId, (detail) => detail.status === "succeeded", "任务成功");
  const detail = await fetchJobDetail(request, backend.base, jobId);
  expect(detail.draftId, "成功后必须产出可复核草稿").toBeTruthy();
  expect(fixture.paidSubmissions()).toBeGreaterThanOrEqual(1);

  await openApp(page, "/jobs");
  await login(page);

  // 最新一行就是本用例的任务（服务端按创建时间倒序）。
  const row = page.locator('[data-testid="job-list-row"]').first();
  await expect(row).toContainText("T17-A 成功链路");
  await expect(row.getByTestId("job-list-status")).toContainText("可复核草稿已产出");
  await expect(row.getByTestId("job-list-status")).toContainText("尚未发布");
  // 费用分列：credits 与 USD 各自带单位，不相加。
  await expect(row.getByTestId("job-list-costs")).toContainText("credits");
  await expect(row.getByTestId("job-list-costs")).toContainText("USD");
  await expect(row.getByTestId("job-list-stage-summary")).toContainText("阶段：共");

  await captureTo("t17-rd", page, "01-jobs-list-succeeded");

  await row.getByRole("link", { name: "查看阶段与恢复入口" }).click();
  await expect(page.getByTestId("job-detail-status")).toContainText("可复核草稿已产出");

  // 阶段逐个展示（不是百分比）。
  await expect(page.locator('[data-testid="job-stage"]')).not.toHaveCount(0);
  for (const kind of ["freeze_inputs", "manual_extract", "tripo_submit", "model_validate", "assemble_draft"]) {
    await expect(page.locator(`[data-stage-kind="${kind}"]`)).toHaveCount(1);
  }
  await expect(page.locator('[data-stage-kind="manual_extract"] [data-testid="job-stage-status"]')).toContainText(
    "已完成",
  );

  // 费用区块：分列 + 预算语义说明（不是供应商账户级封顶）。
  await expect(page.getByTestId("budget-notice")).toContainText("不是供应商账户级硬封顶");
  await expect(page.getByTestId("cost-provider-tripo")).toContainText("credits");
  await expect(page.getByTestId("cost-provider-manual_ai")).toContainText("USD");

  // 终态：不提供取消；草稿入口存在（生成完成 ≠ 已发布）。
  await expect(page.getByTestId("cancel-not-needed")).toBeVisible();
  await expect(page.getByRole("link", { name: /打开草稿（待复核）/ })).toBeVisible();

  // 不出现虚假总进度：整页没有百分比数字。
  const text = (await page.locator("body").innerText()).replace(/\s+/g, " ");
  expect(text).not.toContain("总进度");
  expect(text).not.toMatch(/\d+\s*%/);
  expect(text).not.toContain("自动发布");
  await captureTo("t17-rd", page, "02-job-detail-succeeded");
});

test("浏览器关闭后服务器继续：关页面 → 服务端推进 → 重开看到新状态", async ({ browser, request }) => {
  fixture.state.manualMode = "refuse";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 8000; // 给"页面打开时任务仍在进行"留出确定窗口
  const { jobId } = await seedJob(request, backend, "T17-B 关浏览器");

  const context = await browser.newContext();
  const page = await context.newPage();
  await openApp(page, `/jobs/${jobId}`);
  await login(page);
  // 页面打开时批次仍在等待（fixture 延迟 8 秒），状态是"进行中"而不是最终状态。
  await expect(page.getByTestId("job-detail-status")).toContainText("进行中");
  const beforeClose = (await page.getByTestId("job-detail-status").innerText()).trim();

  // 关掉浏览器（页面与上下文都关闭）：服务端任务不受影响。
  await context.close();

  const afterClose = await waitForJob(
    request,
    backend.base,
    jobId,
    (detail) => stageOf(detail, "manual_extract").status === "needs_input",
    "浏览器关闭期间批次进入 needs_input",
  );
  expect(afterClose.status, "关页面后服务端继续推进").toBe("needs_input");

  const context2 = await browser.newContext();
  const page2 = await context2.newPage();
  await openApp(page2, `/jobs/${jobId}`);
  await login(page2);
  await expect(page2.getByTestId("job-detail-status")).toContainText("等待人工补齐");
  await expect(
    page2.locator('[data-stage-kind="manual_extract"] [data-testid="job-stage-missing"]'),
  ).toContainText("拒答");
  const afterReopen = (await page2.getByTestId("job-detail-status").innerText()).trim();
  expect(afterReopen, `重开后的状态必须来自服务端（关闭前：${beforeClose}）`).not.toBe(beforeClose);
  await context2.close();
});

test("服务重启恢复：页面从数据库读到同一状态", async ({ page, request }) => {
  fixture.state.manualMode = "refuse";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const { jobId } = await seedJob(request, backend, "T17-C 重启恢复");
  const before = await waitForJob(
    request,
    backend.base,
    jobId,
    (detail) => detail.status === "needs_input",
    "任务进入 needs_input",
  );

  await openApp(page, `/jobs/${jobId}`);
  await login(page);
  await expect(page.getByTestId("job-detail-status")).toContainText("等待人工补齐");

  // 重启后端（同一 data-dir）：界面必须从数据库恢复显示，而不是靠内存状态。
  await backend.restart(fixture);

  await page.reload();
  await expect(page.getByTestId("job-detail-status")).toContainText("等待人工补齐");
  await expect(
    page.locator('[data-stage-kind="manual_extract"] [data-testid="job-stage-status"]'),
  ).toContainText("缺项");
  // `knowledgeProduced=false`（拒答）是持久化事实，重启后仍如实显示。
  await expect(page.locator('[data-stage-kind="manual_extract"]')).toContainText("未产出正式知识");
  const after = await fetchJobDetail(request, backend.base, jobId);
  expect(after.id).toBe(before.id);
  expect(after.status).toBe(before.status);
  expect(after.revision).toBeGreaterThanOrEqual(before.revision);
});

test("unknown：不渲染重试按钮，渲染对账入口，预留保留为未决", async ({ page, request }) => {
  fixture.state.manualMode = "success";
  fixture.state.submitMode = "http500";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const paidBefore = fixture.paidSubmissions();
  const { jobId } = await seedJob(request, backend, "T17-D 结果未知");
  const detail = await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => stageOf(current, "tripo_submit").status === "submission_unknown",
    "付费提交结果未知",
  );
  expect(detail.status).toBe("submission_unknown");
  expect(fixture.paidSubmissions() - paidBefore, "本用例只有一次付费提交").toBe(1);

  await openApp(page, `/jobs/${jobId}`);
  await login(page);

  await expect(page.getByTestId("job-detail-status")).toContainText("付费提交结果未知");
  await expect(
    page.locator('[data-stage-kind="tripo_submit"] [data-testid="job-stage-status"]'),
  ).toContainText("结果未知（等待对账）");
  // **不是禁用而是不渲染重试**（UI-034）。
  await expect(page.locator('[data-testid="stage-retry-button"]')).toHaveCount(0);
  await expect(page.locator('[data-stage-kind="tripo_submit"] [data-testid="stage-retry-denied"]')).toHaveCount(0);
  // 对账入口存在，且远端任务链路提供 attachRemoteTask。
  await expect(page.locator('[data-stage-kind="tripo_submit"] [data-testid="reconcile-panel"]')).toBeVisible();
  await expect(page.getByLabel(/附加账户中查到的远端任务/)).toBeVisible();
  await expect(page.getByLabel(/记录「账户中未找到该任务」/)).toBeVisible();
  await expect(page.getByLabel(/授权替代提交/)).toBeVisible();
  // 预留保留为未决（不是 0）。
  const reservation = page.getByTestId("cost-amount-tripo-unknown");
  await expect(reservation).toContainText("30.00");
  await expect(reservation).toContainText("credits");
  await expect(page.getByText(/未决预留（等待对账）/)).toBeVisible();
  await captureTo("t17-rd", page, "03-unknown-reconcile");
});

test("重复操作不多收费：重复点击与重放不产生新的付费提交", async ({ page, request }) => {
  fixture.state.manualMode = "success";
  fixture.state.submitMode = "http500";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const paidBefore = fixture.paidSubmissions();
  const { jobId } = await seedJob(request, backend, "T17-E 重复操作");
  await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => stageOf(current, "tripo_submit").status === "submission_unknown",
    "付费提交结果未知",
  );
  expect(fixture.paidSubmissions() - paidBefore).toBe(1);

  await openApp(page, `/jobs/${jobId}`);
  await login(page);

  // 对账动作的 POST 延迟 600ms：用于观察"提交中禁用"（防重复点击）。
  await page.route(`${WEB_BASE}/api/v1/jobs/*/reconcile`, async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 600));
    await route.continue({ url: route.request().url().replace(WEB_BASE, backend.base) });
  });

  await page.getByLabel(/核查证据（必填）/).fill("e2e：已核对账户任务列表，无对应任务（2026-09-12）");
  const submit = page.getByTestId("reconcile-submit");
  await submit.click();
  // 提交中：按钮禁用（第二次点击不产生动作）。
  await expect(submit).toBeDisabled();
  await submit.click({ force: true }).catch(() => undefined);
  await expect(
    page.locator('[data-stage-kind="tripo_submit"] [data-testid="job-stage-status"]'),
  ).toContainText("缺项", { timeout: 20_000 });
  expect(fixture.paidSubmissions() - paidBefore, "对账不得产生新的付费提交").toBe(1);

  // 重放同一请求体（服务端按状态拒绝：非 submission_unknown 无副作用）。
  const csrf = await apiLogin(request, backend.base, BACKEND_PASSWORD);
  const current = await fetchJobDetail(request, backend.base, jobId);
  const submitStage = stageOf(current, "tripo_submit");
  const replay = await request.fetch(`${backend.base}/api/v1/jobs/${jobId}/reconcile`, {
    method: "POST",
    headers: { "x-csrf-token": csrf, "if-match": current.etag ?? "" },
    data: { action: "recordNoTask", stageId: submitStage.id, evidence: "重放同一请求体" },
  });
  expect(replay.status(), await replay.text()).toBe(422);
  expect(fixture.paidSubmissions() - paidBefore, "重放不得产生新的付费提交").toBe(1);

  // 界面取消：写明后果，确认后不产生新的付费步骤。
  await page.getByTestId("cancel-button").click();
  await expect(page.getByTestId("cancel-consequences")).toContainText("不会被撤销");
  await page.getByTestId("cancel-confirm").click();
  await expect(page.getByTestId("job-detail-status")).toContainText("已取消", { timeout: 20_000 });
  expect(fixture.paidSubmissions() - paidBefore, "取消不得产生新的付费提交").toBe(1);
});

test("needs_input：缺项与真实恢复动作一致（P3① 不再承诺可重试）；断网不误报业务失败", async ({
  page,
  request,
}) => {
  // 模型分支：远端成功但 GLB 截断 → model_validate needs_input（tripo 已结算 → 重试被
  // budgetNotHolding 拒绝 = T15 P3①）；知识分支：拒答 → needs_input（预留仍占用 → 可重试）。
  fixture.state.manualMode = "refuse";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "truncated";
  fixture.state.manualDelayMs = 0;
  const { jobId } = await seedJob(request, backend, "T17-F 缺项与断网");
  const detail = await waitForJob(
    request,
    backend.base,
    jobId,
    (current) =>
      stageOf(current, "model_validate").status === "needs_input" &&
      stageOf(current, "manual_extract").status === "needs_input",
    "两条分支各自阻塞",
  );
  expect(stageOf(detail, "model_validate").retry.allowed).toBe(false);
  expect(stageOf(detail, "model_validate").retry.reason).toBe("budgetNotHolding");
  expect(stageOf(detail, "manual_extract").retry.allowed).toBe(true);

  const bridge = await openApp(page, `/jobs/${jobId}`);
  await login(page);

  const validate = page.locator('[data-stage-kind="model_validate"]');
  await expect(validate.locator('[data-testid="job-stage-missing"]')).toBeVisible();
  await expect(validate.locator('[data-testid="stage-retry-denied"]')).toBeVisible();
  await expect(validate.locator('[data-testid="stage-retry-reason"]')).toContainText("重新获取报价");
  await expect(validate.locator('[data-testid="stage-retry-button"]')).toHaveCount(0);

  const batch = page.locator('[data-stage-kind="manual_extract"]');
  await expect(batch.locator('[data-testid="job-stage-missing"]')).toContainText("拒答");
  await expect(batch.locator('[data-testid="stage-retry-button"]')).toBeVisible();

  // T15 P3①：不得出现"可对该阶段重试"这类与实际判定矛盾的文案。
  const pageText = (await page.locator("body").innerText()).replace(/\s+/g, " ");
  expect(pageText).not.toContain("可对该阶段重试");
  expect(pageText).not.toContain("可对失败阶段重试");

  // 断网：API 请求以网络错误失败 → 显示网络问题（保留旧数据），不把任务写成失败。
  bridge.setBlocked(true);
  await expect(page.getByTestId("network-notice")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("job-detail-status")).toContainText("等待人工补齐");
  expect((await page.getByTestId("job-detail-status").innerText()).trim()).not.toContain("失败");
  bridge.setBlocked(false);
  await expect(page.getByTestId("network-notice")).toBeHidden({ timeout: 15_000 });
  await expect(page.getByTestId("job-detail-status")).toContainText("等待人工补齐");
  await captureTo("t17-rd", page, "04-needs-input-recovery");
});

test("失败阶段：错误摘要可读，不可重试的原因如实（不误导）", async ({ page, request }) => {
  fixture.state.manualMode = "success";
  fixture.state.submitMode = "business400";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const { jobId } = await seedJob(request, backend, "T17-G 失败摘要");
  const detail = await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => stageOf(current, "tripo_submit").status === "failed",
    "付费提交业务失败",
  );
  // 业务错误明确未计费 → 预留已释放 → 重试没有预算背书（如实拒绝，不误导）。
  expect(stageOf(detail, "tripo_submit").retry.allowed).toBe(false);
  expect(stageOf(detail, "tripo_submit").retry.reason).toBe("budgetNotHolding");

  await openApp(page, `/jobs/${jobId}`);
  await login(page);
  await expect(page.getByTestId("failed-summary")).toBeVisible();
  const submit = page.locator('[data-stage-kind="tripo_submit"]');
  await expect(submit.locator('[data-testid="job-stage-status"]')).toContainText("失败");
  await expect(submit.locator('[data-testid="stage-retry-button"]')).toHaveCount(0);
  await expect(submit.locator('[data-testid="stage-retry-reason"]')).toContainText("重新获取报价");
});

test("轮询频率：可见约 2 秒、后台降频、终态停止", async ({ page, request }) => {
  fixture.state.manualMode = "refuse";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const { jobId } = await seedJob(request, backend, "T17-H 轮询频率");
  await waitForJob(
    request,
    backend.base,
    jobId,
    (detail) => detail.status === "needs_input",
    "任务进入 needs_input（非终态，轮询继续）",
  );
  const paidAfterSetup = fixture.paidSubmissions();

  await openApp(page, "/jobs");
  await login(page);
  const requestCount = countJobRequests(page);
  await expect(page.getByTestId("job-list-row").first()).toContainText("T17-H 轮询频率");

  // 可见页：约 2 秒一次。
  const visibleStart = requestCount();
  await page.waitForTimeout(5500);
  const visible = requestCount() - visibleStart;
  expect(visible, `可见页 5.5 秒内至少 2 次轮询（实际 ${visible} 次）`).toBeGreaterThanOrEqual(2);

  // 页面不可见：降为约 15 秒（17 秒窗口内 1 次左右；若仍是 2 秒会出现约 8 次）。
  await setVisibility(page, "hidden");
  const hiddenStart = requestCount();
  await page.waitForTimeout(17_000);
  const hidden = requestCount() - hiddenStart;
  expect(hidden, `页面不可见 17 秒内应约 1 次（实际 ${hidden} 次；2 秒轮询会给约 8 次）`).toBeGreaterThanOrEqual(1);
  expect(hidden, `后台轮询必须显著降频（实际 ${hidden} 次）`).toBeLessThanOrEqual(3);

  // 恢复可见后继续轮询。
  await setVisibility(page, "visible");
  const resumeStart = requestCount();
  await page.waitForTimeout(4500);
  expect(requestCount() - resumeStart, "恢复可见后应继续轮询").toBeGreaterThanOrEqual(1);

  // 终态停止：详情页只轮询该任务；任务被取消（终态）后不再轮询。
  // 注：列表页在**已加载任务里仍存在非终态**时按设计继续轮询（本次运行的其它用例
  // 留下了等待人工的任务），因此终态停止的观察点是"只含该任务的详情页"。
  await cancelViaApi(request, jobId);
  await page.goto(`${WEB_BASE}/jobs/${jobId}`);
  await expect(page.getByTestId("job-detail-status")).toContainText("已取消", { timeout: 20_000 });
  await page.waitForTimeout(2000); // 让终态数据落地
  const stopStart = requestCount();
  await page.waitForTimeout(6000);
  const afterTerminal = requestCount() - stopStart;
  expect(afterTerminal, `终态后必须停止轮询（实际 ${afterTerminal} 次）`).toBe(0);
  // 取消（以及全程轮询）不得产生任何新的付费提交。
  expect(fixture.paidSubmissions(), "取消不得产生新的付费提交").toBe(paidAfterSetup);
});
