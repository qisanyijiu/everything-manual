import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RetryCountdown } from "./RetryCountdown";

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });
describe("authoritative safe retry presentation", () => {
  it("uses the current deadline, clamps zero, accepts rescheduling, and makes no HTTP call", () => {
    vi.useFakeTimers(); vi.setSystemTime(new Date("2026-10-02T00:00:00Z")); const fetch = vi.spyOn(globalThis, "fetch");
    const view = render(<RetryCountdown safeRetry={{ number: 4, limit: 5 }} nextRunAt="2026-10-02T00:01:02Z" />);
    expect(screen.getByText("第 4/5 次安全重试")).toBeInTheDocument();
    expect(screen.getByTestId("safe-retry-countdown")).toHaveTextContent("1:02");
    act(() => vi.advanceTimersByTime(3000)); expect(screen.getByTestId("safe-retry-countdown")).toHaveTextContent("0:59");
    view.rerender(<RetryCountdown safeRetry={{ number: 5, limit: 5 }} nextRunAt="2026-10-02T00:00:05Z" />);
    act(() => vi.advanceTimersByTime(5000)); expect(screen.getByTestId("safe-retry-countdown")).toHaveTextContent("等待调度更新");
    expect(vi.getTimerCount()).toBe(0); expect(fetch).not.toHaveBeenCalled();
    expect(screen.getByTestId("safe-retry-countdown").closest('[aria-live="off"]')).not.toBeNull();
  });
  it("missing time stays explicit; visibility recovery recalculates from wall clock", () => {
    vi.useFakeTimers(); vi.setSystemTime(new Date("2026-10-02T00:00:00Z"));
    const visible = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    const view = render(<RetryCountdown safeRetry={{ number: 1, limit: 5 }} nextRunAt={null} />);
    expect(screen.getByTestId("safe-retry-countdown")).toHaveTextContent("重试时间暂不可用"); expect(vi.getTimerCount()).toBe(0);
    view.rerender(<RetryCountdown safeRetry={{ number: 1, limit: 5 }} nextRunAt="2026-10-02T00:00:20Z" />);
    visible.mockReturnValue("hidden"); act(() => document.dispatchEvent(new Event("visibilitychange"))); expect(vi.getTimerCount()).toBe(0);
    vi.setSystemTime(new Date("2026-10-02T00:00:15Z")); visible.mockReturnValue("visible"); act(() => document.dispatchEvent(new Event("visibilitychange")));
    expect(screen.getByTestId("safe-retry-countdown")).toHaveTextContent("0:05"); view.unmount(); expect(vi.getTimerCount()).toBe(0);
    // This is a component event test; actual OS hidden-page behavior is separately measured.
  });
});
