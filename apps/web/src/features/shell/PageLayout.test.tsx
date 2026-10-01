import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { setViewportWidth } from "../../test/render";
import { PageLayout } from "./PageLayout";

beforeEach(() => setViewportWidth(1024));

describe("interaction-a：中屏多面板标签", () => {
  it("左右循环、Home/End同步焦点/选择/内容；只有活动标签可Tab进入", () => {
    render(<PageLayout rail={{ id: "parts", label: "部件", content: <button>选择部件</button> }} aside={{ id: "steps", label: "步骤", content: <p>步骤说明</p> }}>主内容</PageLayout>);
    fireEvent.click(screen.getByRole("button", { name: "显示步骤" }));
    const parts = screen.getByRole("tab", { name: "部件" });
    const steps = screen.getByRole("tab", { name: "步骤" });
    expect(parts).toHaveAttribute("tabindex", "0");
    expect(steps).toHaveAttribute("tabindex", "-1");
    expect(screen.getByRole("tabpanel")).toHaveAttribute("tabindex", "-1");
    parts.focus();
    fireEvent.keyDown(parts, { key: "ArrowLeft" });
    expect(steps).toHaveFocus();
    expect(steps).toHaveAttribute("aria-selected", "true");
    expect(parts).toHaveAttribute("tabindex", "-1");
    expect(screen.getByRole("tabpanel")).toHaveAttribute("aria-labelledby", steps.id);
    expect(steps).toHaveAttribute("aria-controls", screen.getByRole("tabpanel").id);
    expect(screen.getByRole("tabpanel")).toHaveAttribute("tabindex", "0");
    fireEvent.keyDown(steps, { key: "ArrowRight" });
    expect(parts).toHaveFocus();
    expect(screen.getByRole("button", { name: "选择部件" })).toBeInTheDocument();
    fireEvent.keyDown(parts, { key: "End" });
    expect(steps).toHaveFocus();
    fireEvent.keyDown(steps, { key: "Home" });
    expect(parts).toHaveFocus();
    fireEvent.click(steps);
    expect(steps).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tabpanel")).toHaveTextContent("步骤说明");
  });

  it("单面板直接显示，不创建标签组", () => {
    render(<PageLayout aside={{ id: "summary", label: "摘要", content: <p>唯一摘要</p> }}>主内容</PageLayout>);
    fireEvent.click(screen.getByRole("button", { name: "显示摘要" }));
    expect(screen.getByText("唯一摘要")).toBeInTheDocument();
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  });
});
