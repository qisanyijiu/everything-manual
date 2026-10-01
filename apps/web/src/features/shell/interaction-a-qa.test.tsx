/** Independent QA edge cases for IA-01 panel semantics; no production mocks. */
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { PageLayout } from "./PageLayout";
import { setViewportWidth } from "../../test/render";

beforeEach(() => { setViewportWidth(1024); });
afterEach(() => { setViewportWidth(1024); });

describe("interaction-a 独立 QA 标签边界", () => {
  it("只有文字的面板成为 Tab 落点，切到含控件面板取消额外停靠", async () => {
    render(<PageLayout
      rail={{ id: "read-only", label: "纯文字", content: <p>只有阅读说明</p> }}
      aside={{ id: "editable", label: "有控件", content: <button type="button">面板操作</button> }}
    ><p>页面内容</p></PageLayout>);
    fireEvent.click(screen.getByRole("button", { name: "显示有控件" }));
    const text = screen.getByRole("tab", { name: "纯文字" });
    const editable = screen.getByRole("tab", { name: "有控件" });
    expect(screen.getByRole("tabpanel")).toHaveAttribute("tabindex", "0");
    text.focus();
    fireEvent.keyDown(text, { key: "End" });
    expect(editable).toHaveFocus();
    expect(editable).toHaveAttribute("aria-selected", "true");
    await waitFor(() => expect(screen.getByRole("tabpanel")).toHaveAttribute("tabindex", "-1"));
    expect(screen.getByRole("tabpanel")).toHaveAttribute("aria-labelledby", editable.id);
    expect(text).toHaveAttribute("tabindex", "-1");
    fireEvent.keyDown(editable, { key: "ArrowRight" });
    expect(text).toHaveFocus();
    await waitFor(() => expect(screen.getByRole("tabpanel")).toHaveAttribute("tabindex", "0"));
    fireEvent.keyDown(text, { key: "ArrowLeft" });
    expect(editable).toHaveFocus();
    fireEvent.keyDown(editable, { key: "Home" });
    expect(text).toHaveFocus();
  });

  it("单面板直接呈现，不添加标签；普通 Tab 不由标签导航处理", () => {
    const single = render(<PageLayout aside={{ id: "one", label: "单面板", content: <button type="button">阅读原文</button> }}><p>内容</p></PageLayout>);
    fireEvent.click(screen.getByRole("button", { name: "显示单面板" }));
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "阅读原文" })).toBeInTheDocument();
    single.unmount();
    render(<PageLayout rail={{ id: "a", label: "A", content: <p>A 内容</p> }} aside={{ id: "b", label: "B", content: <p>B 内容</p> }}><p>内容</p></PageLayout>);
    fireEvent.click(screen.getByRole("button", { name: "显示B" }));
    expect(fireEvent.keyDown(screen.getByRole("tab", { name: "A" }), { key: "Tab" })).toBe(true);
    expect(fireEvent.keyDown(screen.getByRole("tab", { name: "A" }), { key: "Tab", shiftKey: true })).toBe(true);
    expect(screen.getByRole("tab", { name: "A" })).toHaveAttribute("aria-selected", "true");
  });
});
