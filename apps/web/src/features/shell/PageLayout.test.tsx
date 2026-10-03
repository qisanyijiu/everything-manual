import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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


describe("PC02A explicit panel navigation", () => {
  it("focuses the requested panel and does not replay old focus on manual reopening", async () => {
    render(<PageLayout rail={{ id: "parts", label: "部件", content: <button>选择部件</button> }}
      original={{ id: "original", label: "原文", content: <h2 id="original-title" tabIndex={-1}>PDF 原文</h2> }}
      navigation={{ panelId: "original", focusId: "original-title", serial: 1 }}>主内容</PageLayout>);
    await waitFor(() => expect(screen.getByRole("heading", { name: "PDF 原文" })).toHaveFocus());
    fireEvent.click(screen.getByRole("button", { name: "隐藏部件" }));
    fireEvent.click(screen.getByRole("button", { name: "显示部件" }));
    const original = screen.getByRole("tab", { name: "原文" });
    original.focus(); fireEvent.click(original);
    await new Promise((resolve) => setTimeout(resolve, 40));
    expect(original).toHaveFocus();
  });
  it("a removed source control falls back to the corresponding panel", async () => {
    render(<PageLayout rail={{ id: "parts", label: "部件", content: <p>实体已变化</p> }}
      aside={{ id: "steps", label: "步骤", content: <p>步骤说明</p> }}
      navigation={{ panelId: "parts", focusId: "removed-evidence", serial: 1 }}>主内容</PageLayout>);
    await waitFor(() => expect(screen.getByRole("tabpanel")).toHaveFocus());
    expect(screen.getByRole("tabpanel")).toHaveTextContent("实体已变化");
  });
});
