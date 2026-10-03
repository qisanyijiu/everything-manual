/**
 * 全局「生成完成」提示：只对本会话见过的进行中任务弹提示；历史已结束任务不弹；
 * 成功提示常驻并直达结果页，失败提示用 alert。
 */

import { screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { jsonResponse, renderApp } from "../../test/render";

const SESSION = {
  data: {
    admin: { id: "01993000-0000-7000-8000-000000000001" },
    csrfToken: "csrf-token-abc",
    expiresAt: "2099-01-01T00:00:00Z",
  },
};

function summary(id: string, status: string, draftId: string | null = null): Record<string, unknown> {
  return {
    id,
    itemId: "item-1",
    itemName: "相机",
    itemModel: "PENTAX 17",
    status,
    draftId,
    revision: 1,
    reservations: [],
    stageSummary: {},
    createdAt: "2026-10-02T09:00:00Z",
    updatedAt: "2026-10-02T09:00:00Z",
  };
}

/** 每次 `/jobs?limit=20` 依次返回下一帧（最后一帧之后保持不变）。 */
function stubJobFrames(frames: Record<string, unknown>[][]) {
  let index = 0;
  const mock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === "/api/v1/auth/session") {
      return jsonResponse(SESSION);
    }
    if (url === "/api/v1/jobs?limit=20") {
      const frame = frames[Math.min(index, frames.length - 1)] ?? [];
      index += 1;
      return jsonResponse({ data: frame, nextCursor: null });
    }
    if (url.startsWith("/api/v1/items")) {
      return jsonResponse({ data: [], nextCursor: null });
    }
    throw new Error(`未预期的请求：${url}`);
  });
  vi.stubGlobal("fetch", mock);
  return mock;
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("JobCompletionWatcher", () => {
  it("进行中 → 成功：弹出常驻提示，链接直达本次生成结果页", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    stubJobFrames([[summary("job-1", "running")], [summary("job-1", "succeeded", "draft-1")]]);
    renderApp({ route: "/" });

    await vi.advanceTimersByTimeAsync(2500);
    const link = await screen.findByRole("link", { name: "查看生成结果" });
    expect(link).toHaveAttribute("href", "/jobs/job-1/result");
    expect(screen.getByText(/「相机（PENTAX 17）」生成完成/)).toBeTruthy();

    // 常驻：超过普通提示的自动消失时间后仍在。
    await vi.advanceTimersByTimeAsync(8000);
    expect(screen.getByRole("link", { name: "查看生成结果" })).toBeTruthy();
  });

  it("首次加载时已经结束的历史任务不弹提示", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const fetchMock = stubJobFrames([[summary("job-old", "succeeded", "draft-old")]]);
    renderApp({ route: "/" });

    await vi.waitFor(() =>
      expect(fetchMock.mock.calls.some(([url]) => String(url) === "/api/v1/jobs?limit=20")).toBe(true),
    );
    await vi.advanceTimersByTimeAsync(5000);
    expect(screen.queryByText(/生成完成/)).toBeNull();
    // 全部终态 → 停止轮询（只请求了一次）。
    expect(fetchMock.mock.calls.filter(([url]) => String(url) === "/api/v1/jobs?limit=20")).toHaveLength(1);
  });

  it("进行中 → 失败：用 alert 提示并链接到任务详情", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    stubJobFrames([[summary("job-2", "waiting_provider")], [summary("job-2", "failed")]]);
    renderApp({ route: "/" });

    await vi.advanceTimersByTimeAsync(2500);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("生成失败");
    expect(screen.getByRole("link", { name: "查看任务" })).toHaveAttribute("href", "/jobs/job-2");
  });
});
