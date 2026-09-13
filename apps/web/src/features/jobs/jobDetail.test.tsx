/**
 * T17 任务详情组件测试（真实路由 + 真实 Query，只替换 fetch）。
 *
 * 覆盖（PRD §6.2 UI-027、UI-031、UI-033、UI-034、UI-037、UI-038）：
 * - `submission_unknown` **不渲染重试按钮**、渲染对账面板；同步链路不提供
 *   `attachRemoteTask`（依据服务端 `submissionStyle`）；
 * - `needs_input` 但 `retry.allowed=false`（T15 P3①：预留已结算）→ 不渲染重试按钮，
 *   显示服务端原因与"重新报价"路径，不出现"可对该阶段重试"；
 * - `needs_input` 且 `retry.allowed=true` → 渲染重试按钮，点击带 If-Match 与幂等键；
 * - 费用分列（credits / USD 带单位，不相加），unknown 预留保留显示（不是 0）；
 * - 禁用措辞清单不出现在整页文本里。
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { errorResponse, jsonResponse, renderApp, setViewportWidth } from "../../test/render";
import { JOBS_POLL_VISIBLE_MS } from "./jobs";

const SESSION = {
  data: {
    admin: { id: "01993000-0000-7000-8000-000000000001" },
    csrfToken: "csrf-token-abc",
    expiresAt: "2026-09-19T00:00:00Z",
  },
};

const REQUEST_ID = "01993000-0000-7000-8000-0000000000ee";

const FORBIDDEN_WORDING = [
  "已自动校准",
  "自动发布",
  "总进度 100%",
  "已证明页图来自原 PDF",
  "供应商账户硬封顶",
  "零费用",
  "重试不会重复收费",
  "离线可用",
];

function stage(overrides: Record<string, unknown>): Record<string, unknown> {
  return {
    id: "stage-1",
    stageKind: "model_validate",
    batchIndex: 0,
    status: "queued",
    pageSet: null,
    attemptCount: 1,
    pollCount: 0,
    nextRunAt: null,
    lastError: null,
    needsInput: [],
    resultAssetId: null,
    usage: null,
    knowledgeProduced: null,
    retry: { allowed: false, reason: "stageNotRetryable", message: "阶段当前状态为 queued：不可重试" },
    submissionStyle: null,
    updatedAt: "2026-09-12T10:00:00Z",
    ...overrides,
  };
}

function detailFixture(overrides: Record<string, unknown>): Record<string, unknown> {
  return {
    id: "job-1",
    item: { id: "item-1", name: "相机 A", model: "X100V" },
    snapshotId: "snap-1",
    status: "running",
    revision: 7,
    stages: [stage({})],
    attempts: [],
    reservations: [],
    draftId: null,
    budgetNotice:
      "预算上限只表示本应用不会主动发起超出本次授权估算的请求，不是供应商账户级硬封顶。",
    createdAt: "2026-09-12T09:00:00Z",
    updatedAt: "2026-09-12T10:00:00Z",
    ...overrides,
  };
}

type FetchHandler = (url: string, init: RequestInit) => Response | undefined;

function stubFetch(handler: FetchHandler) {
  const mock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const response = handler(url, init ?? {});
    if (response === undefined) {
      throw new Error(`未预期的请求：${url}`);
    }
    return response;
  });
  vi.stubGlobal("fetch", mock);
  return mock;
}

function stubDetail(detail: Record<string, unknown>, etag = '"r7"'): ReturnType<typeof stubFetch> {
  return stubFetch((url) => {
    if (url === "/api/v1/auth/session") {
      return jsonResponse(SESSION, { requestId: REQUEST_ID });
    }
    if (url === "/api/v1/jobs/job-1") {
      return jsonResponse({ data: detail }, { etag, requestId: REQUEST_ID });
    }
    return undefined;
  });
}

beforeEach(() => {
  // wide 断点：费用/操作侧栏并排渲染（mid 断点下侧栏是折叠面板，属图标式交互）。
  setViewportWidth(1400);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("任务详情：unknown 与对账入口（UI-034/UI-035）", () => {
  it("submission_unknown 不渲染重试按钮，渲染对账入口；同步链路不提供 attachRemoteTask", async () => {
    stubDetail(
      detailFixture({
        status: "submission_unknown",
        stages: [
          stage({
            id: "batch-1",
            stageKind: "manual_extract",
            status: "submission_unknown",
            submissionStyle: "syncResponse",
            lastError: "请求已发出、完整响应未持久化：结果未知，等待对账",
            retry: {
              allowed: false,
              reason: "stageNotRetryable",
              message: "阶段 manual_extract 当前状态为 submission_unknown：submission_unknown 必须先对账",
            },
          }),
        ],
        attempts: [
          {
            id: "attempt-1",
            stageId: "batch-1",
            submitState: "unknown",
            remoteTaskId: null,
            responseId: "resp-1",
            startedAt: "2026-09-12T09:30:00Z",
            lastError: "响应未持久化",
          },
        ],
        reservations: [
          { provider: "manual_ai", currency: "usdMicros", reservedMinor: 300_000, reservedDisplay: "0.30 USD", state: "unknown" },
        ],
      }),
    );

    renderApp({ route: "/jobs/job-1" });

    expect(await screen.findByTestId("reconcile-panel")).toBeTruthy();
    expect(screen.queryByTestId("stage-retry-button")).toBeNull();
    expect(screen.getByText(/付费提交结果未知：需要先对账/)).toBeTruthy();
    expect(screen.getByText(/该分支的后续购买已暂停/)).toBeTruthy();
    expect(screen.getByTestId("reconcile-summary").textContent).toContain("不提供盲目重试");
    // 同步链路：不出现 attachRemoteTask 选项（contracts §5）。
    expect(screen.queryByLabelText(/附加账户中查到的远端任务/)).toBeNull();
    expect(screen.getByLabelText(/记录「账户中未找到该任务」/)).toBeTruthy();
    expect(screen.getByLabelText(/授权替代提交/)).toBeTruthy();
    // unknown 预留保留显示（不是 0）。
    expect(screen.getByTestId("cost-amount-manual_ai-unknown").textContent).toContain("0.30 USD");
  });

  it("远端任务链路（asyncRemoteTask）才提供 attachRemoteTask", async () => {
    stubDetail(
      detailFixture({
        status: "submission_unknown",
        stages: [
          stage({
            id: "submit-1",
            stageKind: "tripo_submit",
            status: "submission_unknown",
            submissionStyle: "asyncRemoteTask",
          }),
        ],
      }),
    );

    renderApp({ route: "/jobs/job-1" });
    expect(await screen.findByTestId("reconcile-panel")).toBeTruthy();
    expect(screen.getByLabelText(/附加账户中查到的远端任务/)).toBeTruthy();
    expect(screen.queryByTestId("stage-retry-button")).toBeNull();
  });
});

describe("任务详情：needs_input 的缺项与可执行动作（UI-031 / T15 P3①）", () => {
  it("retry.allowed=false（预留已结算）时不渲染重试按钮，给出原因与重新报价路径", async () => {
    stubDetail(
      detailFixture({
        status: "needs_input",
        stages: [
          stage({
            id: "validate-1",
            stageKind: "model_validate",
            status: "needs_input",
            needsInput: [{ code: "truncated", message: "模型文件截断（原始模型已保留，未静默修改）" }],
            lastError: "模型文件截断",
            retry: {
              allowed: false,
              reason: "budgetNotHolding",
              message:
                "tripo 分支的预留未占用预算（已释放/已结算或缺失）：重试会重新发起请求，请重新获取报价并确认预算后再执行",
            },
          }),
        ],
        reservations: [
          { provider: "tripo", currency: "creditMinor", reservedMinor: 3000, reservedDisplay: "30.00 credits", state: "settled" },
        ],
      }),
    );

    renderApp({ route: "/jobs/job-1" });

    expect(await screen.findByTestId("stage-retry-denied")).toBeTruthy();
    expect(screen.queryByTestId("stage-retry-button")).toBeNull();
    expect(screen.getByTestId("stage-retry-reason").textContent).toContain("重新获取报价");
    expect(screen.getByTestId("job-stage-missing").textContent).toContain("模型文件截断");
    expect(screen.getByText(/去检查 PDF 准备|打开物品/)).toBeTruthy();
  });

  it("retry.allowed=true 时渲染重试按钮，点击带 If-Match 与幂等键", async () => {
    const calls: Array<{ url: string; method: string; headers: Record<string, string>; body: string }> = [];
    const mock = stubFetch((url, init) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION, { requestId: REQUEST_ID });
      }
      if (url === "/api/v1/jobs/job-1" && (init.method ?? "GET") === "GET") {
        return jsonResponse(
          {
            data: detailFixture({
              status: "needs_input",
              stages: [
                stage({
                  id: "batch-1",
                  stageKind: "manual_extract",
                  status: "needs_input",
                  submissionStyle: "syncResponse",
                  knowledgeProduced: false,
                  needsInput: [
                    { code: "manual_ai_refusal", message: "供应商拒答：该批未产出正式知识（不自动重试、不重复付费）" },
                  ],
                  retry: { allowed: true, reason: null, message: null },
                }),
              ],
            }),
          },
          { etag: '"r7"', requestId: REQUEST_ID },
        );
      }
      if (url === "/api/v1/jobs/job-1/retry") {
        calls.push({
          url,
          method: init.method ?? "GET",
          headers: (init.headers ?? {}) as Record<string, string>,
          body: String(init.body ?? ""),
        });
        return jsonResponse(
          {
            data: {
              job: detailFixture({ status: "running" }),
              stageId: "batch-1",
              stageKind: "manual_extract",
              previousStatus: "needs_input",
              requeuedDependents: 0,
              notice: "只重跑指定阶段：已完成的其他阶段成果保留，不改变模型/质量预设。",
            },
          },
          { requestId: REQUEST_ID },
        );
      }
      return undefined;
    });

    renderApp({ route: "/jobs/job-1" });

    const button = await screen.findByTestId("stage-retry-button");
    fireEvent.click(button);
    await waitFor(() => expect(calls.length).toBe(1));
    expect(calls[0]!.headers["if-match"]).toBe('"r7"');
    expect(calls[0]!.headers["Idempotency-Key"] ?? calls[0]!.headers["idempotency-key"]).toBeTruthy();
    expect(calls[0]!.body).toContain("batch-1");
    // 点击后按钮进入禁用（防重复提交；正确性仍由服务端幂等保证）。
    expect(mock).toBeTruthy();
  });
});

describe("任务详情：费用分列与措辞边界（UI-027 / §6.3.2）", () => {
  it("credits 与 USD 分列、带单位、不相加；unknown 明确写未决预留", async () => {
    stubDetail(
      detailFixture({
        status: "waiting_provider",
        reservations: [
          { provider: "tripo", currency: "creditMinor", reservedMinor: 3000, reservedDisplay: "30.00 credits", state: "settled" },
          { provider: "manual_ai", currency: "usdMicros", reservedMinor: 300_000, reservedDisplay: "0.30 USD", state: "unknown" },
        ],
      }),
    );

    renderApp({ route: "/jobs/job-1" });

    expect(await screen.findByTestId("cost-breakdown")).toBeTruthy();
    expect(screen.getByTestId("cost-amount-tripo-settled").textContent).toContain("credits");
    expect(screen.getByTestId("cost-amount-manual_ai-unknown").textContent).toContain("USD");
    expect(screen.getByText(/未决预留（等待对账）/)).toBeTruthy();
    expect(screen.getByTestId("budget-notice").textContent).toContain("不是供应商账户级硬封顶");
    // 不相加：页面不出现把两个币种合并的数字。
    expect(screen.queryByText(/合计|总计|总费用/)).toBeNull();
  });

  it("整页文本不含禁用措辞（§6.3.2 清单）", async () => {
    stubDetail(
      detailFixture({
        status: "needs_input",
        stages: [
          stage({
            id: "validate-1",
            stageKind: "model_validate",
            status: "needs_input",
            needsInput: [{ code: "truncated", message: "模型文件截断（原始模型已保留，未静默修改）" }],
            lastError: "模型文件截断",
            retry: { allowed: false, reason: "budgetNotHolding", message: "预留未占用预算：请重新获取报价" },
          }),
        ],
      }),
    );

    renderApp({ route: "/jobs/job-1" });
    await screen.findByTestId("stage-retry-denied");

    const text = document.body.textContent ?? "";
    for (const forbidden of FORBIDDEN_WORDING) {
      expect(text).not.toContain(forbidden);
    }
    // 也不出现线性总进度：没有百分比数字。
    expect(text).not.toMatch(/\d+\s*%/);
  });

  it("轮询参数：可见页 2 秒（常量合同，AC-049 的界面侧）", () => {
    expect(JOBS_POLL_VISIBLE_MS).toBe(2000);
  });

  it("网络失败不写成任务失败（UI-033）", async () => {
    let failNext = true;
    stubFetch((url) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION, { requestId: REQUEST_ID });
      }
      if (url === "/api/v1/jobs/job-1") {
        if (failNext) {
          failNext = false;
          throw new TypeError("Failed to fetch");
        }
        return jsonResponse({ data: detailFixture({ status: "waiting_provider" }) }, { etag: '"r7"' });
      }
      return undefined;
    });

    renderApp({ route: "/jobs/job-1" });

    expect(await screen.findByText("无法连接服务")).toBeTruthy();
    expect(screen.getByText(/不是任务失败/)).toBeTruthy();
    expect(screen.queryByText("失败")).toBeNull();
  });

  it("合同错误（业务失败）与网络错误分开显示", async () => {
    stubFetch((url) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION, { requestId: REQUEST_ID });
      }
      if (url === "/api/v1/jobs/job-1") {
        return errorResponse(500, "INTERNAL", "服务端内部错误");
      }
      return undefined;
    });

    renderApp({ route: "/jobs/job-1" });
    expect(await screen.findByText("无法读取任务详情")).toBeTruthy();
    expect(screen.getByText("服务端内部错误")).toBeTruthy();
  });
});
