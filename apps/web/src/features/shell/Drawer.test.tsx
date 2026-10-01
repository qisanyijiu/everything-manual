import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Drawer } from "./Drawer";

describe("IA-BUG-001：抽屉内原生折叠控件的键盘顺序", () => {
  it("下一页后的 summary 保留原生 Tab 路径，末端与关闭按钮双向循环", () => {
    render(<Drawer open onClose={vi.fn()} title="步骤与原文">
      <button>下一页</button>
      <details><summary>本页文字</summary><pre>PDF 文字层</pre></details>
    </Drawer>);
    const close = screen.getByRole("button", { name: "关闭" });
    const next = screen.getByRole("button", { name: "下一页" });
    const summary = screen.getByText("本页文字");
    next.focus();
    // jsdom 不实现原生 Tab 移焦；验证陷阱不取消这一跳，真实浏览器由 QA 验证。
    expect(fireEvent.keyDown(next, { key: "Tab" })).toBe(true);
    summary.focus();
    expect(summary).toHaveFocus();
    expect(fireEvent.keyDown(summary, { key: "Tab" })).toBe(false);
    expect(close).toHaveFocus();
    fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(summary).toHaveFocus();
  });

  it("隐藏、禁用、负tabIndex与未展开details内部控件不能成为回绕落点", () => {
    render(<Drawer open onClose={vi.fn()} title="步骤与原文">
      <button>下一页</button>
      <details><summary>本页文字</summary><button>未展开的操作</button></details>
      <button disabled tabIndex={0}>禁用的操作</button>
      <fieldset disabled><button>禁用字段组操作</button></fieldset>
      <button tabIndex={-2}>跳过的操作</button>
      <div hidden><button>隐藏属性操作</button></div>
      <div style={{ display: "none" }}><button>隐藏容器操作</button></div>
      <button style={{ visibility: "hidden" }}>不可见操作</button>
      <div inert><button>不活动操作</button></div>
      <input type="hidden" tabIndex={0} />
    </Drawer>);
    const close = screen.getByRole("button", { name: "关闭" });
    fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(screen.getByText("本页文字")).toHaveFocus();
    fireEvent.keyDown(document, { key: "Tab" });
    expect(close).toHaveFocus();
  });

  it("details 展开后内部操作加入顺序，折叠后再次排除", () => {
    const { rerender } = render(<Drawer open onClose={vi.fn()} title="原文">
      <details open><summary>本页文字</summary><button>复制文字</button></details>
    </Drawer>);
    const close = screen.getByRole("button", { name: "关闭" });
    fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(screen.getByRole("button", { name: "复制文字" })).toHaveFocus();
    rerender(<Drawer open onClose={vi.fn()} title="原文">
      <details><summary>本页文字</summary><button>复制文字</button></details>
    </Drawer>);
    close.focus();
    fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(screen.getByText("本页文字")).toHaveFocus();
  });
});
