/**
 * T17「任务中心与可行动错误」QA 独立验收（QA 回合 21；PRD 修订 2 / ui_revision 2）。
 *
 * 与 RD 的 `job-recovery.spec.ts` **故意不同的观察方式**（不复制 RD 断言）：
 * 1. 界面金额与状态文本**逐字段与 API 事实比对**（`reservedDisplay` 原样显示、不相加）；
 * 2. 轮询频率按请求时间戳测**间隔**（可见 ≈2s、后台 ≈15s）；终态停止在
 *    **只含终态任务的列表**上测 0 次请求。注：headless Chromium 里 `bringToFront()`
 *    不会真的产生 hidden（本回合探针实测两个标签页都是 visible），
 *    后台场景只能用 `visibilitychange` + `visibilityState` 改写（同应用读取的 API 面）；
 * 3. 断网用 `route.abort('internetdisconnected')`（只掐 API），断网期间在服务端改状态，
 *    恢复后必须显示**服务端新状态**（证明不是本地猜测）；
 * 4. 412 用「只掐 GET、放行 POST」制造陈旧 ETag，验证 UI-008 的统一冲突组件；
 * 5. T15 P3①：详情 `retry.allowed=false` 必须与端点 422 的 reason/message 逐字一致，
 *    并且真的点一次 allowed=true 的重试验证"可重试 = 真的生效"；
 * 6. 收尾直接读 SQLite `cost_ledger`（只读）留证：unknown 行的 `actual` 必须为 NULL。
 *
 * 全部零真实外网/付费：fixture 只监听 127.0.0.1，后端 provider 指向它，
 * 凭据是假环境变量；本文件另阻断所有非本地请求并断言未发生。
 */

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

import { apiLogin, captureTo } from "./helpers";
import { E2E_WEB_PORT, REPO_ROOT } from "./runtime";
import {
  BACKEND_PASSWORD,
  LocalFixture,
  TestBackend,
  buildTestServerBinary,
  fetchJobDetail,
  seedJob,
  stageOf,
  waitForJob,
  type JobDetailView,
} from "./job-recovery-harness";

test.describe.configure({ timeout: 240_000 });

const WEB_BASE = `http://127.0.0.1:${E2E_WEB_PORT}`;
const QA_EVIDENCE_DIR = path.join(REPO_ROOT, "artifacts", "web-mvp", "t17-qa");

let fixture: LocalFixture;
let backend: TestBackend;

test.beforeAll(async () => {
  test.setTimeout(900_000);
  buildTestServerBinary();
  fixture = new LocalFixture();
  await fixture.start();
  backend = new TestBackend("qa-t17");
  await backend.start(fixture);
});

test.afterAll(async () => {
  // 独立证据：直接读 SQLite 账本（只读），确认 unknown 行 actual IS NULL。
  try {
    fs.mkdirSync(QA_EVIDENCE_DIR, { recursive: true });
    const dump = execFileSync(
      "sqlite3",
      [
        "-readonly",
        path.join(backend.dataDir, "manual.sqlite3"),
        ".mode list",
        "SELECT provider, currency, reserved, COALESCE(actual,'NULL'), state FROM cost_ledger ORDER BY provider, state;",
      ],
      { encoding: "utf8" },
    );
    fs.writeFileSync(
      path.join(QA_EVIDENCE_DIR, "r21-cost-ledger-dump.txt"),
      `# QA 回合 21 · T17：临时 data-dir 的 cost_ledger（provider|currency|reserved|actual|state）\n${dump}`,
    );
  } catch (error) {
    fs.writeFileSync(
      path.join(QA_EVIDENCE_DIR, "r21-cost-ledger-dump.txt"),
      `导出失败（不伪造）：${String(error)}\n`,
    );
  }
  await backend?.cleanup(fixture);
});

type BlockMode = "off" | "all" | "gets-only";

interface Bridge {
  setBlocked(mode: BlockMode): void;
}

async function attachBridge(page: Page): Promise<Bridge> {
  const state: { mode: BlockMode } = { mode: "off" };
  await page.route(`${WEB_BASE}/api/v1/**`, async (route) => {
    const blocked =
      state.mode === "all" || (state.mode === "gets-only" && route.request().method() === "GET");
    if (blocked) {
      await route.abort("internetdisconnected");
      return;
    }
    await route.continue({ url: route.request().url().replace(WEB_BASE, backend.base) });
  });
  return { setBlocked: (mode) => (state.mode = mode) };
}

async function openApp(page: Page, url: string): Promise<Bridge> {
  const bridge = await attachBridge(page);
  await page.goto(`${WEB_BASE}${url}`);
  return bridge;
}

async function login(page: Page): Promise<void> {
  await page.getByLabel("密码").fill(BACKEND_PASSWORD);
  await page.getByRole("button", { name: "登录" }).click();
  await expect(page.getByRole("navigation", { name: "主导航" })).toBeVisible();
}

function countJobApiRequests(page: Page): () => number {
  let count = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/v1/jobs")) {
      count += 1;
    }
  });
  return () => count;
}

async function bodyText(page: Page): Promise<string> {
  return (await page.locator("body").innerText()).replace(/\s+/g, " ");
}

async function apiRetry(
  request: APIRequestContext,
  detail: JobDetailView,
  stageId: string,
  key: string,
): Promise<{ status: number; body: { error?: { message?: string; details?: { reason?: string } } } }> {
  const csrf = await apiLogin(request, backend.base, BACKEND_PASSWORD);
  const response = await request.fetch(`${backend.base}/api/v1/jobs/${detail.id}/retry`, {
    method: "POST",
    headers: {
      "x-csrf-token": csrf,
      "if-match": detail.etag ?? "",
      "idempotency-key": key,
    },
    data: { stageId },
  });
  const text = await response.text();
  return { status: response.status(), body: JSON.parse(text) as never };
}

/** 原始详情 JSON（合同字段断言用：harness 类型只声明了常用字段）。 */
async function rawJobDetail(
  request: APIRequestContext,
  jobId: string,
): Promise<{
  data: {
    reservations: { provider: string; state: string; reservedMinor: number; reservedDisplay: string }[];
    stages: { stageKind: string; knowledgeProduced?: boolean | null }[];
  };
}> {
  const response = await request.get(`${backend.base}/api/v1/jobs/${jobId}`);
  expect(response.status(), await response.text()).toBe(200);
  return (await response.json()) as never;
}

async function apiCancel(request: APIRequestContext, jobId: string): Promise<number> {
  const csrf = await apiLogin(request, backend.base, BACKEND_PASSWORD);
  const detail = await fetchJobDetail(request, backend.base, jobId);
  const response = await request.fetch(`${backend.base}/api/v1/jobs/${jobId}/cancel`, {
    method: "POST",
    headers: { "x-csrf-token": csrf, "if-match": detail.etag ?? "" },
  });
  return response.status();
}

/** AC-034：界面不得提供降质量/换模型/加阶段/自动修复/一键重试之类的捷径控件。 */
async function expectNoShortcutControls(page: Page): Promise<void> {
  for (const role of ["button", "link", "radio"] as const) {
    await expect(
      page.getByRole(role, { name: /降低质量|降质量|换模型|增加阶段|自动修复|一键重试|立即重试|自动校准/ }),
    ).toHaveCount(0);
  }
}

/** 阻断一切非本地请求（零真实外网的独立守卫）。 */
async function blockExternal(page: Page): Promise<() => string[]> {
  const external: string[] = [];
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") {
      external.push(url.href);
      await route.abort();
      return;
    }
    await route.continue();
  });
  return () => external;
}

test("QA-T17-1 列表/详情：分列金额与服务端逐字段一致、终态停止轮询、无虚假总进度", async ({
  page,
  request,
}) => {
  // 本文件第一条用例：此刻后端只有本用例的任务（列表终态停止才有确定观察点）。
  fixture.state.manualMode = "success";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const external = await blockExternal(page);
  const { itemId, jobId } = await seedJob(request, backend, "QA21 成功链路");
  const detail = await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => current.status === "succeeded",
    "任务成功（服务端事实）",
  );
  const jobsInBackend = await request.get(`${backend.base}/api/v1/jobs`);
  const allJobs = (await jobsInBackend.json()) as { data: { id: string }[] };
  expect(allJobs.data.length, "该后端此刻应只有本用例一个任务").toBe(1);
  expect(detail.draftId, "成功任务必须产出可复核草稿").toBeTruthy();

  await openApp(page, "/jobs");
  await login(page);
  const row = page.locator('[data-testid="job-list-row"]').first();
  await expect(row).toContainText("QA21 成功链路");
  await expect(row.getByTestId("job-list-status")).toContainText("可复核草稿已产出");
  await expect(row.getByTestId("job-list-status")).toContainText("尚未发布");
  await expect(row.getByTestId("job-list-stage-summary")).toContainText("阶段：共");

  // 列表金额 = 服务端 reservedDisplay 原样（含单位），credits 与 USD 分列。
  const listCostText = await row.getByTestId("job-list-costs").innerText();
  for (const reservation of detail.reservations) {
    const cell = row.getByTestId(`job-list-cost-${reservation.provider}-${reservation.state}`);
    await expect(cell).toHaveText(reservation.reservedDisplay);
  }
  expect(listCostText).toContain("credits");
  expect(listCostText).toContain("USD");
  expect(listCostText).not.toMatch(/合计|总计|total/i);

  // 终态停止（列表：已加载行全为终态）：6 秒内 0 次任务接口请求。
  const listCount = countJobApiRequests(page);
  const listStart = listCount();
  await page.waitForTimeout(6000);
  expect(listCount() - listStart, "列表全部终态后必须停止轮询").toBe(0);
  await captureTo("t17-qa", page, "01-list-terminal-stop");

  // 物品过滤入口（资料库「查看任务」指到真实列表）。
  await page.goto(`${WEB_BASE}/jobs?itemId=${itemId}`);
  await expect(page.getByTestId("jobs-item-scope")).toBeVisible();
  await expect(page.locator('[data-testid="job-list-row"]')).toHaveCount(1);

  // 详情：金额与 API 逐字段一致；预算语义常驻；终态停止轮询。
  await page.goto(`${WEB_BASE}/jobs/${jobId}`);
  await expect(page.getByTestId("job-detail-status")).toContainText("可复核草稿已产出");
  for (const reservation of detail.reservations) {
    await expect(page.getByTestId(`cost-amount-${reservation.provider}-${reservation.state}`)).toHaveText(
      reservation.reservedDisplay,
    );
  }
  await expect(page.getByTestId("budget-notice")).toContainText("不是供应商账户级硬封顶");
  const detailText = await bodyText(page);
  expect(detailText, "不得出现虚假总进度").not.toContain("总进度");
  expect(detailText, "不得出现百分比").not.toMatch(/\d+\s*%/);
  expect(detailText, "不得出现自动发布").not.toContain("自动发布");
  expect(detailText, "不得出现禁用措辞「重试不会重复收费」").not.toContain("重试不会重复收费");
  expect(detailText, "不得出现「零费用」").not.toContain("零费用");
  await expectNoShortcutControls(page);
  const detailCount = countJobApiRequests(page);
  const detailStart = detailCount();
  await page.waitForTimeout(6000);
  expect(detailCount() - detailStart, "详情终态后必须停止轮询").toBe(0);
  await captureTo("t17-qa", page, "02-detail-terminal-stop");

  expect(external(), "不应访问外部地址").toEqual([]);
});

test("QA-T17-2 进行中状态区分 + 浏览器关闭后服务器继续推进", async ({ browser, request }) => {
  fixture.state.manualMode = "refuse";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 10_000; // 给"页面打开时仍在本地执行"留确定窗口
  const { jobId } = await seedJob(request, backend, "QA21 关浏览器继续");
  await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => current.status === "running",
    "任务进入本地执行（running）",
  );

  const context = await browser.newContext();
  const page = await context.newPage();
  await openApp(page, "/jobs");
  await login(page);
  const row = page.locator('[data-testid="job-list-row"]').filter({ hasText: "QA21 关浏览器继续" });
  await expect(row.getByTestId("job-list-status")).toContainText("本地阶段进行中", {
    timeout: 15_000,
  });
  await captureTo("t17-qa", page, "03-list-running-local");

  // 费用区块的「预留中」：进行中任务的已占用预算必须可见且非 0（UI-027）。
  await page.goto(`${WEB_BASE}/jobs/${jobId}`);
  await expect(page.getByTestId("job-detail-status")).toContainText("本地阶段进行中");
  await expect(page.getByText("预留中（已占用预算）").first()).toBeVisible();
  const reservedAmount = await page
    .locator('[data-testid^="cost-amount-"][data-testid$="-reserved"]')
    .first()
    .innerText();
  expect(reservedAmount, "预留金额必须带单位").toMatch(/credits|USD/);
  expect(reservedAmount.replace(/[^0-9.]/g, ""), "预留金额不得显示成 0").not.toMatch(/^0(\.0+)?$/);
  await captureTo("t17-qa", page, "03b-detail-reserved");
  await context.close();

  // 浏览器关闭：服务器必须继续推进直到缺项（服务端事实）。
  const afterClose = await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => current.status === "needs_input",
    "浏览器关闭期间任务推进到 needs_input",
  );
  expect(stageOf(afterClose, "manual_extract").status).toBe("needs_input");

  const context2 = await browser.newContext();
  const page2 = await context2.newPage();
  await openApp(page2, `/jobs/${jobId}`);
  await login(page2);
  await expect(page2.getByTestId("job-detail-status")).toContainText("等待人工补齐");
  await expect(
    page2.locator('[data-stage-kind="manual_extract"] [data-testid="job-stage-missing"]'),
  ).toContainText("拒答");
  await captureTo("t17-qa", page2, "04-detail-after-reopen");
  await context2.close();
});

test("QA-T17-3 断网显示网络问题（不误报业务失败），恢复后显示服务端新状态", async ({
  page,
  request,
}) => {
  fixture.state.manualMode = "refuse";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const external = await blockExternal(page);
  const { jobId } = await seedJob(request, backend, "QA21 断网");
  await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => current.status === "needs_input",
    "任务进入 needs_input",
  );

  const bridge = await openApp(page, `/jobs/${jobId}`);
  await login(page);
  await expect(page.getByTestId("job-detail-status")).toContainText("等待人工补齐");

  bridge.setBlocked("all");
  await expect(page.getByTestId("network-notice")).toBeVisible({ timeout: 15_000 });
  const offlineText = await bodyText(page);
  expect(offlineText).toContain("网络连接异常");
  expect(
    (await page.getByTestId("job-detail-status").innerText()).trim(),
    "断网不得把任务写成失败",
  ).not.toContain("失败");
  await captureTo("t17-qa", page, "05-offline-network-notice");

  // 断网期间在服务端改状态（取消）：恢复后界面必须显示服务端事实「已取消」。
  const status = await apiCancel(request, jobId);
  expect(status, "断网期间的服务端取消应成功").toBe(200);
  bridge.setBlocked("off");
  await expect(page.getByTestId("job-detail-status")).toContainText("已取消", { timeout: 20_000 });
  await expect(page.getByTestId("network-notice")).toBeHidden({ timeout: 15_000 });
  await expect(page.getByTestId("cancel-not-needed")).toBeVisible();
  await captureTo("t17-qa", page, "06-recovered-server-fact");

  expect(external(), "不应访问外部地址").toEqual([]);
});

test("QA-T17-4 unknown：无重试按钮、对账入口、未决预留=服务端金额（非 0）、recordNoTask 需证据", async ({
  page,
  request,
}) => {
  fixture.state.manualMode = "success";
  fixture.state.submitMode = "http500";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const paidBefore = fixture.paidSubmissions();
  const { jobId } = await seedJob(request, backend, "QA21 结果未知");
  const detail = await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => stageOf(current, "tripo_submit").status === "submission_unknown",
    "付费提交结果未知",
  );
  expect(detail.status).toBe("submission_unknown");
  const tripoReservation = detail.reservations.find((entry) => entry.provider === "tripo");
  expect(tripoReservation, "unknown 必须保留 tripo 预留").toBeTruthy();
  expect(tripoReservation?.state).toBe("unknown");
  const rawUnknown = await rawJobDetail(request, jobId);
  const rawTripo = rawUnknown.data.reservations.find((entry) => entry.provider === "tripo");
  expect(rawTripo?.reservedMinor, "未决预留金额不得为 0").toBeGreaterThan(0);
  const unknownDisplay = tripoReservation?.reservedDisplay ?? "";
  expect(unknownDisplay).toContain("credits");
  // 同步链路（说明书 AI）不提供 attachRemoteTask：由服务端 submissionStyle 决定。
  expect(stageOf(detail, "manual_extract").submissionStyle).toBe("syncResponse");
  expect(stageOf(detail, "tripo_submit").submissionStyle).toBe("asyncRemoteTask");

  await openApp(page, `/jobs/${jobId}`);
  await login(page);
  await expect(page.getByTestId("job-detail-status")).toContainText("付费提交结果未知");
  await expect(page.getByTestId("job-detail-status")).not.toContainText("失败");
  const submitStage = page.locator('[data-stage-kind="tripo_submit"]');
  await expect(submitStage.getByTestId("job-stage-status")).toContainText("结果未知（等待对账）");
  await expect(page.locator('[data-testid="stage-retry-button"]')).toHaveCount(0);
  await expect(submitStage.getByTestId("reconcile-panel")).toBeVisible();
  // 未决预留：金额等于服务端，不是 0。
  await expect(page.getByTestId("cost-amount-tripo-unknown")).toHaveText(unknownDisplay);
  await expect(page.getByText("未决预留（等待对账）")).toBeVisible();
  await expect(page.getByTestId("cost-amount-tripo-unknown")).not.toHaveText(/^0/);
  await expect(page.getByLabel(/附加账户中查到的远端任务/)).toBeVisible();

  // recordNoTask 要求核查证据（空证据不可提交）。
  await page.getByLabel(/记录「账户中未找到该任务」/).check();
  const submit = page.getByTestId("reconcile-submit");
  await expect(submit).toBeDisabled();
  await page.getByLabel(/核查证据（必填）/).fill("QA21：已按对账要求核对账户任务列表，未找到对应任务（2026-09-12）");
  await expect(submit).toBeEnabled();
  await submit.click();
  await expect(
    page.locator('[data-stage-kind="tripo_submit"] [data-testid="job-stage-status"]'),
  ).toContainText("缺项", { timeout: 20_000 });
  const afterReconcile = await fetchJobDetail(request, backend.base, jobId);
  expect(stageOf(afterReconcile, "tripo_submit").status).toBe("needs_input");
  const reservationAfter = afterReconcile.reservations.find((entry) => entry.provider === "tripo");
  expect(reservationAfter?.state, "unknown 的预留不因对账自动释放/清零").toBe("unknown");
  expect(fixture.paidSubmissions() - paidBefore, "对账不得产生新的付费提交").toBe(1);
  await captureTo("t17-qa", page, "07-unknown-reconcile");
});

test("QA-T17-5 T15 P3①：详情拒绝态与端点 422 同源；允许的重试真的生效；草稿文案不承诺重试", async ({
  page,
  request,
}) => {
  fixture.state.manualMode = "refuse";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "truncated";
  fixture.state.manualDelayMs = 0;
  const { jobId } = await seedJob(request, backend, "QA21 缺项与重试");
  const detail = await waitForJob(
    request,
    backend.base,
    jobId,
    (current) =>
      stageOf(current, "model_validate").status === "needs_input" &&
      stageOf(current, "manual_extract").status === "needs_input",
    "两条分支各自阻塞",
  );
  // 付费提交基线：建单本身会产生一次提交；此后的动作（被拒重试/被接受重试）都不得再增加。
  const paidAfterSeed = fixture.paidSubmissions();

  // (a) 详情判定：模型分支被预算拒绝、知识分支可重试。
  const blockedStage = stageOf(detail, "model_validate");
  expect(blockedStage.retry.allowed).toBe(false);
  expect(blockedStage.retry.reason).toBe("budgetNotHolding");
  const retryableStage = stageOf(detail, "manual_extract");
  expect(retryableStage.retry.allowed).toBe(true);

  // (b) 端点同源：真的 POST 一次被拒，reason/message 必须与详情逐字一致，且无副作用。
  const denied = await apiRetry(request, detail, blockedStage.id, `qa21-denied-${Date.now()}`);
  expect(denied.status, JSON.stringify(denied.body)).toBe(422);
  expect(denied.body.error?.details?.reason).toBe(blockedStage.retry.reason);
  expect(denied.body.error?.message).toBe(blockedStage.retry.message);
  expect(fixture.paidSubmissions(), "被拒的重试不得产生付费提交").toBe(paidAfterSeed);

  // (d) 界面：模型分支不渲染重试按钮、原因可读；知识分支渲染按钮。
  await openApp(page, `/jobs/${jobId}`);
  await login(page);
  const validate = page.locator('[data-stage-kind="model_validate"]');
  await expect(validate.getByTestId("stage-retry-denied")).toBeVisible();
  await expect(validate.getByTestId("stage-retry-reason")).toContainText("重新获取报价");
  await expect(validate.locator('[data-testid="stage-retry-button"]')).toHaveCount(0);
  const batch = page.locator('[data-stage-kind="manual_extract"]');
  await expect(batch.getByTestId("job-stage-missing")).toContainText("拒答");
  const pageText = await bodyText(page);
  expect(pageText).not.toContain("可对该阶段重试");
  expect(pageText).not.toContain("可对失败阶段重试");
  await captureTo("t17-qa", page, "08-p3-1-denied-retry");

  // (e) 允许的重试必须真的生效：换成成功脚本后点按钮 → 该批真的重新请求并完成。
  //     注意顺序：拒答脚本下重试会立刻回到 needs_input（fixture 零延迟），
  //     "离开 needs_input"不是稳定中间态，因此先切脚本再点。
  const manualBefore = fixture.counts.manual;
  fixture.state.manualMode = "success";
  await batch.getByTestId("stage-retry-button").click();
  await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => stageOf(current, "manual_extract").status === "succeeded",
    "重试后知识分支完成",
    90_000,
  );
  const rawRecovered = await rawJobDetail(request, jobId);
  const recoveredBatch = rawRecovered.data.stages.find((stage) => stage.stageKind === "manual_extract");
  expect(recoveredBatch?.knowledgeProduced, "重试后该批必须真的产出知识").toBe(true);
  expect(fixture.counts.manual, "重试确实重新请求了说明书 AI（不是沉默按钮）").toBeGreaterThan(
    manualBefore,
  );
  expect(fixture.paidSubmissions(), "重试不得新增 tripo 付费提交").toBe(paidAfterSeed);

  // (f) 草稿缺项文案（T15 P3① 的文案分叉）：该任务现在是"知识完整、模型分支缺项"，
  //     组装应产出 partial 草稿且缺项说明不再承诺"可重试"。
  const drafted = await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => current.draftId !== null && current.draftId !== undefined,
    "部分成功草稿已产出",
  );
  expect(drafted.status, "模型分支仍缺项：父 job 不得冒充 succeeded").toBe("needs_input");
  const draftResponse = await request.get(
    `${backend.base}/api/v1/items/${drafted.item.id}/drafts/${drafted.draftId}`,
  );
  expect(draftResponse.status(), await draftResponse.text()).toBe(200);
  const draftText = await draftResponse.text();
  expect(draftText, "草稿缺项不得承诺可重试").not.toContain("可对该阶段重试");
  expect(draftText, "草稿缺项必须指向任务中心的恢复动作").toContain("恢复动作");
  // UI 侧：模型分支仍不渲染必然被拒的重试入口（预算未背书）。
  await page.goto(`${WEB_BASE}/jobs/${jobId}`);
  await expect(
    page.locator('[data-stage-kind="model_validate"] [data-testid="stage-retry-denied"]'),
  ).toBeVisible();
  await captureTo("t17-qa", page, "11-partial-draft-no-retry");
});

test("QA-T17-6 轮询频率：可见约 2 秒 / 后台约 15 秒；重启后仍从数据库恢复", async ({
  page,
  request,
}) => {
  fixture.state.manualMode = "refuse";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const { jobId } = await seedJob(request, backend, "QA21 轮询频率");
  await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => current.status === "needs_input",
    "任务进入 needs_input（非终态，轮询继续）",
  );

  const stamps: number[] = [];
  page.on("request", (request_) => {
    if (request_.url().includes("/api/v1/jobs")) {
      stamps.push(Date.now());
    }
  });
  await openApp(page, `/jobs/${jobId}`);
  await login(page);
  await expect(page.getByTestId("job-detail-status")).toContainText("等待人工补齐");
  await page.waitForTimeout(1500); // 让首次请求落定

  // 可见：连续观测 8 秒，请求间隔应集中在约 2 秒（4 秒的间隔会说明是 4s+ 轮询）。
  const visibleStart = stamps.length;
  await page.waitForTimeout(8000);
  const visibleStamps = stamps.slice(visibleStart);
  expect(visibleStamps.length, "可见页 8 秒内应有约 4 次轮询").toBeGreaterThanOrEqual(3);
  const gaps = visibleStamps.slice(1).map((stamp, index) => stamp - (visibleStamps[index] ?? stamp));
  const maxGap = Math.max(...gaps);
  expect(maxGap, `可见页轮询间隔应约 2 秒（实测最大间隔 ${maxGap}ms）`).toBeLessThanOrEqual(3500);

  // 后台：`document.visibilityState` 改写 + `visibilitychange`。
  // 说明：headless Chromium 下 `page.bringToFront()` 不会真的产生 hidden（QA 回合 21 探针实测
  // 两个标签页都是 visible，证据 `artifacts/web-mvp/t17-qa/r21-visibility-probe.log`），
  // 因此这里用同一 API 面（visibilitychange 事件）触发观察点。
  const hiddenAt = Date.now();
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  const hiddenStart = stamps.length;
  await page.waitForTimeout(10_000);
  const hiddenFirst10 = stamps.length - hiddenStart;
  expect(
    hiddenFirst10,
    `转为后台后 10 秒内不应再按 2 秒轮询（实际 ${hiddenFirst10} 次）`,
  ).toBe(0);
  await page.waitForTimeout(10_000);
  const hiddenTotal = stamps.length - hiddenStart;
  expect(hiddenTotal, `后台 20 秒内应约 1 次（实际 ${hiddenTotal} 次）`).toBeGreaterThanOrEqual(1);
  expect(hiddenTotal, `后台轮询必须显著降频（实际 ${hiddenTotal} 次）`).toBeLessThanOrEqual(3);
  const firstHiddenGap = (stamps[hiddenStart] ?? 0) - hiddenAt;
  expect(firstHiddenGap, `后台间隔应约 15 秒（实测 ${firstHiddenGap}ms）`).toBeLessThanOrEqual(20_500);

  // 恢复可见：继续按 2 秒轮询（界面状态不变）。
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  const resumeStart = stamps.length;
  await page.waitForTimeout(5000);
  expect(stamps.length - resumeStart, "恢复可见后应继续轮询").toBeGreaterThanOrEqual(1);

  // 服务重启：同一 data-dir，页面刷新后仍从数据库恢复同一状态。
  const beforeRestart = await fetchJobDetail(request, backend.base, jobId);
  await backend.restart(fixture);
  await page.reload();
  await expect(page.getByTestId("job-detail-status")).toContainText("等待人工补齐", {
    timeout: 20_000,
  });
  const afterRestart = await fetchJobDetail(request, backend.base, jobId);
  expect(afterRestart.status).toBe(beforeRestart.status);
  expect(afterRestart.id).toBe(beforeRestart.id);

  // 列表页的断网语义（UI-029/UI-033）：保留旧数据、标注可能过期，不写成任务失败。
  // 会话已存在：直接打开列表页（不要再次走登录表单）。
  const listBridge = await attachBridge(page);
  await page.goto(`${WEB_BASE}/jobs`);
  await expect(page.getByTestId("job-list-row").first()).toBeVisible();
  listBridge.setBlocked("all");
  await expect(page.getByTestId("network-notice")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("jobs-updated-at")).toContainText("数据可能已过期");
  expect(await bodyText(page)).not.toContain("任务失败");
  await captureTo("t17-qa", page, "12-list-offline-notice");
  listBridge.setBlocked("off");
  await expect(page.getByTestId("network-notice")).toBeHidden({ timeout: 15_000 });
});

test("QA-T17-7 412 走统一刷新组件（UI-008）：陈旧 ETag 不覆盖、刷新后见服务端状态", async ({
  page,
  request,
}) => {
  fixture.state.manualMode = "refuse";
  fixture.state.submitMode = "success";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const { jobId } = await seedJob(request, backend, "QA21 并发 412");
  const before = await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => current.status === "needs_input",
    "任务进入 needs_input",
  );

  const bridge = await openApp(page, `/jobs/${jobId}`);
  await login(page);
  await expect(page.getByTestId("job-detail-status")).toContainText("等待人工补齐");

  // 只掐 GET（保留页面上的陈旧 ETag），POST 仍放行。
  bridge.setBlocked("gets-only");
  const cancelled = await apiCancel(request, jobId);
  expect(cancelled, "服务端取消应成功（版本 +1）").toBe(200);

  await page.getByTestId("cancel-button").click();
  await page.getByTestId("cancel-confirm").click();
  const conflict = page.locator(".conflict-notice");
  await expect(conflict).toBeVisible({ timeout: 15_000 });
  await expect(conflict).toContainText("该内容已被其他操作更新");
  await expect(conflict).toContainText(`r${before.revision + 1}`);
  await expect(page.getByTestId("job-detail-status")).toContainText("等待人工补齐");
  await captureTo("t17-qa", page, "09-conflict-412");

  bridge.setBlocked("off");
  await conflict.getByRole("button", { name: /刷新后重试/ }).click();
  await expect(page.getByTestId("job-detail-status")).toContainText("已取消", { timeout: 20_000 });
  const after = await fetchJobDetail(request, backend.base, jobId);
  expect(after.status, "陈旧的取消尝试不得改变服务端状态").toBe("cancelled");
  expect(after.revision, "只应发生一次服务端版本推进").toBe(before.revision + 1);
});

test("QA-T17-8 失败状态：与 unknown 明确分开、错误摘要可读、不可重试原因如实", async ({
  page,
  request,
}) => {
  fixture.state.manualMode = "success";
  fixture.state.submitMode = "business400";
  fixture.state.modelMode = "valid";
  fixture.state.manualDelayMs = 0;
  const { jobId } = await seedJob(request, backend, "QA21 失败摘要");
  const detail = await waitForJob(
    request,
    backend.base,
    jobId,
    (current) => current.status === "failed",
    "任务因业务错误失败",
  );
  expect(stageOf(detail, "tripo_submit").retry.allowed).toBe(false);

  await openApp(page, `/jobs/${jobId}`);
  await login(page);
  await expect(page.getByTestId("job-detail-status")).toContainText("失败");
  await expect(page.getByTestId("job-detail-status")).not.toContainText("结果未知");
  await expect(page.getByTestId("failed-summary")).toBeVisible();
  const submit = page.locator('[data-stage-kind="tripo_submit"]');
  await expect(submit.getByTestId("job-stage-status")).toContainText("失败");
  await expect(submit.getByTestId("stage-retry-button")).toHaveCount(0);
  await expect(submit.getByTestId("stage-retry-reason")).toContainText("重新获取报价");
  await expect(page.locator('[data-testid="reconcile-panel"]'), "失败不是 unknown：不出现对账入口").toHaveCount(0);
  await expect(page.getByTestId("cancel-not-needed")).toBeVisible();
  await expectNoShortcutControls(page);

  // 列表：该行状态标签是「失败」，与「付费提交结果未知（等待对账）」区分。
  await page.goto(`${WEB_BASE}/jobs`);
  const row = page.locator('[data-testid="job-list-row"]').filter({ hasText: "QA21 失败摘要" });
  await expect(row.getByTestId("job-list-status")).toHaveText("失败");
  await captureTo("t17-qa", page, "10-failed-state");
});
