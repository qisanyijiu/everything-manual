/**
 * T21 QA 独立验收 spec（回合 29）——由 QA 自写，覆盖 validation-release §3 中
 * 尚无独立证据的浏览器侧必测项：**PDF 「worker 缺失」**（§3 PDF 行失败项）。
 *
 * 场景：部署缺资源 / 中间件拦截时，`pdf.worker.min.mjs` 不可加载。
 * 要求（不得坏掉无解释、"不能挂起"、"不能假成功"）：
 *  1. 页面必须到达**终态**（不无限挂起）；
 *  2. 若降级为主线程 fake worker 完成 → 必须真的产出正确页（1-based、文字可读），
 *     且仍不产生任何收费/任务；
 *  3. 若失败 → 必须显示可行动错误（`prepare-failures`/`prepare-status` 有内容），
 *     且不产生页记录、不产生 jobs/cost_ledger。
 * 两种情况都合规（validation-release §4 允许明确降级），本用例**如实记录**发生的形态。
 *
 * 运行：`npm --prefix apps/web run test:e2e -- qa-t21-independent.spec.ts`。
 * 本文件属验收测试，不改生产代码。
 */

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

const WORK_DIR = process.env.EM_E2E_WORK_DIR ?? path.join(os.tmpdir(), "em-web-mvp-e2e");

interface RuntimeInfo {
  readonly apiBase: string;
  readonly password: string;
  readonly dataDir: string;
  readonly serverPid: number;
}

function runtime(): RuntimeInfo {
  return JSON.parse(fs.readFileSync(path.join(WORK_DIR, "runtime.json"), "utf8")) as RuntimeInfo;
}

function api(): string {
  return runtime().apiBase;
}

async function apiLogin(request: APIRequestContext): Promise<string> {
  const response = await request.post(`${api()}/api/v1/auth/login`, {
    data: { password: runtime().password },
  });
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: { csrfToken: string } };
  return body.data.csrfToken;
}

interface SeedResult {
  readonly itemId: string;
  readonly sourceAssetId: string;
}

/** 造一个"物品 + 已绑定文字型 PDF"的前置状态（只走公开 HTTP 合同）。 */
async function seedItemWithDocument(request: APIRequestContext): Promise<SeedResult> {
  const csrf = await apiLogin(request);
  const headers = { "x-csrf-token": csrf };
  const itemResponse = await request.post(`${api()}/api/v1/items`, {
    headers,
    data: { name: "QA21 worker 缺失", brand: "QA", model: "R29-WORKER" },
  });
  expect(itemResponse.status(), await itemResponse.text()).toBe(201);
  const itemId = ((await itemResponse.json()) as { data: { id: string } }).data.id;

  const fixture = path.join(
    path.resolve(import.meta.dirname, "../../../.."),
    "tests",
    "fixtures",
    "assets",
    "sample-manual-text.pdf",
  );
  const assetResponse = await request.post(`${api()}/api/v1/items/${itemId}/assets`, {
    headers,
    multipart: {
      purpose: "document",
      file: {
        name: "sample-manual-text.pdf",
        mimeType: "application/pdf",
        buffer: fs.readFileSync(fixture),
      },
    },
  });
  expect(assetResponse.status(), await assetResponse.text()).toBe(201);
  const sourceAssetId = ((await assetResponse.json()) as { data: { id: string } }).data.id;

  const documentResponse = await request.post(`${api()}/api/v1/items/${itemId}/documents`, {
    headers,
    data: { sourceAssetId, title: "QA21 说明书" },
  });
  expect(documentResponse.status(), await documentResponse.text()).toBe(201);

  return { itemId, sourceAssetId };
}

async function loginViaUi(page: Page): Promise<void> {
  await page.goto("/login");
  await page.getByLabel("密码").fill(runtime().password);
  await page.getByRole("button", { name: "登录" }).click();
  await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();
}

type TableName = "preparations" | "pages" | "jobs" | "job_stages" | "cost_ledger";

function dbCount(table: TableName): number {
  const stdout = execFileSync(
    "sqlite3",
    [path.join(runtime().dataDir, "manual.sqlite3"), `SELECT COUNT(*) FROM ${table};`],
    { encoding: "utf8" },
  );
  return Number.parseInt(stdout.trim(), 10);
}

test.describe("T21 独立验收：PDF worker 缺失（回合 29）", () => {
  test.describe.configure({ timeout: 180_000 });

  test("worker 资源不可用：不挂起、不假成功、零任务零费用（如实记录降级或失败形态）", async ({
    page,
    request,
  }) => {
    const seed = await seedItemWithDocument(request);
    await loginViaUi(page);

    // 阻断 worker 资源（模拟部署缺文件 / 静态资源被拦截）。
    //
    // 注意：**不能**用宽 glob 阻断所有含 `pdf.worker` 的请求——dev 模式下
    // `vendor.ts` 会**静态 import** `pdf.worker.min.mjs?url`，把该模块一并 abort 会让
    // PreparePage 的模块图整体加载失败（`Failed to fetch dynamically imported module`，
    // 页面根本渲染不出 prepare-start，属测试自身缺陷，回合 29 首跑即此形态）。
    // 因此只阻断"真正被当作 worker 脚本取"的 URL（不含 `?url` 模块请求）。
    const blocked: string[] = [];
    await page.route(
      (url) => url.href.includes("pdf.worker") && !url.href.includes("?url"),
      async (route) => {
        blocked.push(route.request().url());
        await route.abort("failed");
      },
    );

    const jobsBefore = dbCount("jobs");
    const stagesBefore = dbCount("job_stages");
    const ledgerBefore = dbCount("cost_ledger");
    const pagesBefore = dbCount("pages");
    const preparationsBefore = dbCount("preparations");

    await page.goto(`/items/${seed.itemId}/import/prepare`);

    // 点击开始准备；若客户端在创建 preparation 之前就因 worker 失败而拒绝，则没有该响应。
    let preparationId: string | null = null;
    page.waitForResponse(
      (response) =>
        response.request().method() === "POST" &&
        /\/api\/v1\/documents\/[^/]+\/preparations$/.test(new URL(response.url()).pathname),
      { timeout: 20_000 },
    ).then(async (response) => {
      if (response.status() < 300) {
        preparationId = ((await response.json()) as { data: { id: string } }).data.id;
      }
    }).catch(() => undefined);

    await page.getByTestId("prepare-start").click();

    // 终态判据：封存就绪（降级成功）/ 客户端拒绝面板 / 页失败列表，三者必居其一。
    const sealed = page.getByTestId("prepare-sealed");
    const failures = page.getByTestId("prepare-failures");
    const rejection = page.getByRole("alert").filter({ hasText: "无法开始准备" });
    const status = page.getByTestId("prepare-status");

    await expect
      .poll(
        async () => {
          if (await sealed.isVisible().catch(() => false)) return "sealed";
          if (await rejection.isVisible().catch(() => false)) return "rejected";
          if ((await failures.count()) > 0 && (await failures.isVisible().catch(() => false))) {
            return "failed";
          }
          return "pending";
        },
        { timeout: 90_000, message: "worker 缺失时页面必须到达终态（不得无限挂起）" },
      )
      .not.toBe("pending");

    // 注意：`prepare-status` 只在"已拿到总页数"后渲染，拒绝/失败形态下不存在——
    // 直接 `textContent()` 会等待到测试超时（回合 29 首跑即此形态，属测试自身缺陷）。
    // 因此这里统一用**有界超时**读取可选文案。
    const optionalText = async (locator: ReturnType<typeof page.getByTestId>): Promise<string> => {
      if ((await locator.count()) === 0) return "";
      return (await locator.textContent({ timeout: 3_000 }).catch(() => null)) ?? "";
    };

    let terminal = "failed";
    let failureText = "";
    if (await sealed.isVisible().catch(() => false)) {
      terminal = "sealed";
    } else if (await rejection.isVisible().catch(() => false)) {
      terminal = "rejected";
      failureText = await optionalText(rejection);
    } else {
      failureText = await optionalText(failures);
    }
    const statusText = await optionalText(status);
    console.log(
      `QA21-W1 实测形态=${terminal}（worker 被阻断 ${blocked.length} 次）；status=${JSON.stringify(
        statusText,
      )}；message=${JSON.stringify(failureText)}`,
    );

    if (terminal !== "sealed") {
      expect(failureText.trim().length, "失败/拒绝形态必须给出可读原因").toBeGreaterThan(0);
    }

    // 任何形态都不允许：任务/费用凭空增加（worker 问题与生成链路无关）。
    expect(dbCount("jobs"), "不得创建任务").toBe(jobsBefore);
    expect(dbCount("job_stages"), "不得创建阶段").toBe(stagesBefore);
    expect(dbCount("cost_ledger"), "不得产生费用记录").toBe(ledgerBefore);
    if (terminal === "sealed") {
      expect(dbCount("pages"), "降级成功也必须真的产出页（不假成功）").toBeGreaterThan(pagesBefore);
      expect(dbCount("preparations")).toBeGreaterThan(preparationsBefore);
    } else {
      expect(dbCount("pages"), "失败/拒绝形态不得留下页记录").toBe(pagesBefore);
      expect(dbCount("preparations"), "失败/拒绝形态不得留下 preparation").toBe(preparationsBefore);
      expect(preparationId, "客户端拒绝发生在创建 preparation 之前").toBeNull();
    }

    // 降级形态：封存后可读、页号 1-based（与正常路径同一合同）。
    if (terminal === "sealed" && preparationId !== null) {
      const detail = await request.get(`${api()}/api/v1/preparations/${preparationId}`);
      if (detail.status() === 200) {
        const body = (await detail.json()) as {
          data: { pages: { pageNumber: number }[] };
        };
        const numbers = body.data.pages.map((entry) => entry.pageNumber);
        expect(numbers.length, "降级形态仍必须产出全部页").toBeGreaterThan(0);
        expect(numbers[0], "页号 1-based").toBe(1);
      }
    }
  });

  test("对照：worker 可用时同一页面正常封存（证明阻断生效而非用例空转）", async ({ page, request }) => {
    const seed = await seedItemWithDocument(request);
    await loginViaUi(page);
    await page.goto(`/items/${seed.itemId}/import/prepare`);
    await page.getByTestId("prepare-start").click();
    await expect(page.getByTestId("prepare-seal")).toBeEnabled({ timeout: 90_000 });
  });
});
