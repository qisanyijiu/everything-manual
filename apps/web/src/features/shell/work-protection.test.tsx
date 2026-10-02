import { act, fireEvent, render, screen } from "@testing-library/react";
import { BrowserRouter, Link, MemoryRouter, Route, Routes, useLocation, useNavigate } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { WorkProtection, useMemoryEdit, usePageWork, useWorkProtection } from "./work-protection";

function Editor() {
  const [value, setValue, clear] = useMemoryEdit("test-edit", "");
  const { bypass } = useWorkProtection(), navigate = useNavigate();
  usePageWork({ active: value !== "", message: "物品修改未保存", discard: clear });
  usePageWork({ active: value !== "", message: "另一个编辑仍在本页" });
  return <><input aria-label="内容" value={value} onChange={e => setValue(e.target.value)} /><Link to="/read">阅读</Link>
    <button onClick={() => navigate("/read")}>程序导航</button><button onClick={() => navigate(-1)}>程序后退</button><button onClick={() => navigate(1)}>程序前进</button>
    <button onClick={() => bypass(() => navigate("/login"))}>模拟失效跳转</button>
    <button onClick={() => { clear(); bypass(() => navigate("/read")); }}>模拟成功返回</button></>;
}
function Elsewhere() {
  const { memory } = useWorkProtection(), location = useLocation();
  return <><h1>{location.pathname}</h1><Link to="/edit">返回编辑</Link><button onClick={() => memory.clear()}>清空登录会话</button></>;
}
function setup() {
  return render(<MemoryRouter initialEntries={["/read", "/edit"]} initialIndex={1}><WorkProtection><Routes><Route path="/edit" element={<Editor />} /><Route path="*" element={<Elsewhere />} /></Routes></WorkProtection></MemoryRouter>);
}
function dirty() { fireEvent.change(screen.getByLabelText("内容"), { target: { value: "ordinary-text" } }); }
describe("one shared work guard", () => {
  it("aggregates work, defaults to continue, cancels with focus and retains input", () => {
    setup(); dirty(); const trigger = screen.getByRole("link", { name: "阅读" }); trigger.focus(); fireEvent.click(trigger);
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "继续处理" })).toHaveFocus();
    fireEvent.keyDown(screen.getByRole("button", { name: "继续处理" }), { key: "Tab", shiftKey: true });
    expect(screen.getByRole("button", { name: "离开页面" })).toHaveFocus();
    fireEvent.keyDown(screen.getByRole("button", { name: "离开页面" }), { key: "Tab" });
    expect(screen.getByRole("button", { name: "继续处理" })).toHaveFocus();
    fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(); expect(trigger).toHaveFocus();
    expect(screen.getByLabelText("内容")).toHaveValue("ordinary-text");
  });
  it("intercepts programmatic push and history go, discards only on explicit acceptance", async () => {
    setup(); dirty(); fireEvent.click(screen.getByRole("button", { name: "程序导航" }));
    fireEvent.click(screen.getByRole("button", { name: "继续处理" }));
    fireEvent.click(screen.getByRole("button", { name: "程序后退" }));
    fireEvent.click(screen.getByRole("button", { name: "离开页面" }));
    expect(await screen.findByRole("heading", { name: "/read" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("link", { name: "返回编辑" }));
    expect(await screen.findByLabelText("内容")).toHaveValue("");
  });
  it("an unguarded history go without a POP cannot bypass the next dirty Back", () => {
    const previousUrl = window.location.href, previousState: unknown = window.history.state;
    window.history.replaceState({ idx: 1 }, "", "/edit");
    // Deterministic component event test: model the browser's out-of-range no-op.
    const go = vi.spyOn(window.history, "go").mockImplementation(() => undefined);
    const view = render(<BrowserRouter><WorkProtection><Routes><Route path="/edit" element={<Editor />} /><Route path="*" element={<Elsewhere />} /></Routes></WorkProtection></BrowserRouter>);
    try {
      fireEvent.click(screen.getByRole("button", { name: "程序前进" }));
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(); dirty();
      window.history.replaceState({ idx: 0 }, "", "/read");
      act(() => window.dispatchEvent(new PopStateEvent("popstate", { state: { idx: 0 } })));
      expect(screen.getByRole("dialog")).toBeVisible(); expect(screen.getByLabelText("内容")).toHaveValue("ordinary-text");
    } finally { view.unmount(); go.mockRestore(); window.history.replaceState(previousState, "", previousUrl); }
  });
  it("keeps only memory edits across expiry and clears them on explicit logout", async () => {
    setup(); dirty(); fireEvent.click(screen.getByRole("button", { name: "模拟失效跳转" }));
    expect(await screen.findByRole("heading", { name: "/login" })).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("link", { name: "返回编辑" }));
    expect(await screen.findByLabelText("内容")).toHaveValue("ordinary-text");
    fireEvent.click(screen.getByRole("button", { name: "模拟失效跳转" }));
    await screen.findByRole("heading", { name: "/login" }); fireEvent.click(screen.getByRole("button", { name: "清空登录会话" }));
    fireEvent.click(screen.getByRole("link", { name: "返回编辑" }));
    expect(await screen.findByLabelText("内容")).toHaveValue("");
    expect(localStorage.length + sessionStorage.length).toBe(0);
  });
  it("expiry supersedes an already open leave dialog without discarding memory edits", async () => {
    setup(); dirty();
    const expiry = screen.getByRole("button", { name: "模拟失效跳转" });
    fireEvent.click(screen.getByRole("link", { name: "阅读" }));
    expect(screen.getByRole("dialog")).toBeVisible();
    // Models the asynchronous global 401 callback while a modal choice is pending.
    fireEvent.click(expiry);
    await screen.findByRole("heading", { name: "/login" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("link", { name: "返回编辑" }));
    expect(await screen.findByLabelText("内容")).toHaveValue("ordinary-text");
  });
  it("registers native protection only while work exists; successful bypass has no SPA prompt", async () => {
    setup(); const clean = new Event("beforeunload", { cancelable: true }); act(() => window.dispatchEvent(clean)); expect(clean.defaultPrevented).toBe(false);
    dirty(); const changed = new Event("beforeunload", { cancelable: true }); act(() => window.dispatchEvent(changed)); expect(changed.defaultPrevented).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "模拟成功返回" }));
    await screen.findByRole("heading", { name: "/read" }); expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    const reading = new Event("beforeunload", { cancelable: true }); act(() => window.dispatchEvent(reading)); expect(reading.defaultPrevented).toBe(false);
  });
});
