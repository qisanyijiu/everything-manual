import { fireEvent, render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";

import { NotificationProvider, NotificationRegion, useNotify } from "./notifications";

function PublishNotices() {
  const notify = useNotify();
  return (
    <button type="button" onClick={() => {
      notify("相机生成完成", { sticky: true, action: { label: "查看生成结果", to: "/jobs/camera/result" } });
      notify("打印机待对账", { kind: "alert", action: { label: "去对账", to: "/jobs/printer" } });
    }}>创建通知</button>
  );
}

function renderNotices() {
  render(
    <MemoryRouter>
      <NotificationProvider>
        <NotificationRegion />
        <PublishNotices />
      </NotificationProvider>
    </MemoryRouter>,
  );
  fireEvent.click(screen.getByRole("button", { name: "创建通知" }));
}

describe("global notifications", () => {
  it("keeps separate accessible alerts/actions and lets the user dismiss one notice", () => {
    renderNotices();
    const region = screen.getByRole("region", { name: "全局通知" });
    expect(within(region).getByRole("status")).toHaveTextContent("相机生成完成");
    expect(within(region).getByRole("alert")).toHaveTextContent("打印机待对账");
    expect(within(region).getByRole("link", { name: "去对账" })).toHaveAttribute("href", "/jobs/printer");

    fireEvent.click(within(region).getByRole("button", { name: "关闭通知：打印机待对账" }));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByRole("link", { name: "查看生成结果" })).toHaveAttribute("href", "/jobs/camera/result");
  });

  it("can dismiss the whole stack without disabling the page's own actions", () => {
    renderNotices();
    fireEvent.click(screen.getByRole("button", { name: "关闭全部通知" }));
    expect(screen.queryByRole("region", { name: "全局通知" })).toBeNull();
    expect(screen.getByRole("button", { name: "创建通知" })).toBeEnabled();
  });
});
