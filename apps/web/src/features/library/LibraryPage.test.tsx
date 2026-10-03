import { MemoryRouter, useLocation, useNavigate } from "react-router";
import { QueryClientProvider } from "@tanstack/react-query";
import { createAppQueryClient } from "../../App";
import { LibraryPage } from "./LibraryPage";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { errorResponse, jsonResponse, renderApp } from "../../test/render";

const item = (id: string) => ({ id, name: `物品 ${id}`, model: "MODEL", brand: null, variant: null, revision: 1,
  createdAt: "2026-10-01T00:00:00Z", updatedAt: "2026-10-01T00:00:00Z", archivedAt: null });
function setup(read: (params: URLSearchParams) => Promise<Response> | Response, route = "/", renderPage = true) {
  const calls: URLSearchParams[] = [];
  vi.stubGlobal("scrollTo", vi.fn());
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(String(input), "http://local.test");
    if (url.pathname.endsWith("/auth/session")) return jsonResponse({ data: { admin: { id: "admin" }, csrfToken: "fixture", expiresAt: "2099-01-01T00:00:00Z" } });
    if (url.pathname.endsWith("/items/summaries")) return jsonResponse({ data: (url.searchParams.get("ids") ?? "").split(",").map((id) => ({ itemId: id, action: "readRelease", targetId: `release-${id}`, latestReleaseId: `release-${id}`, steps: {} })) });
    if (url.pathname === "/api/v1/items") { calls.push(url.searchParams); return read(url.searchParams); }
    throw new Error(`Unexpected ${url.pathname}`);
  }));
  if (renderPage) renderApp({ route }); return calls;
}
afterEach(() => { vi.unstubAllGlobals(); });
async function search(q: string) {
  fireEvent.change(screen.getByLabelText("按名称或型号搜索"), { target: { value: q } });
  fireEvent.submit(screen.getByRole("search"));
}
describe("PC05A server search and retained results", () => {
  it("submits trimmed input only explicitly; rejects 201 chars without truncating a request", async () => {
    const calls = setup((q) => jsonResponse({ data: [item(q.get("q") ?? "first")], nextCursor: null }));
    await screen.findByRole("link", { name: "物品 first" });
    fireEvent.change(screen.getByLabelText("按名称或型号搜索"), { target: { value: "  中文%_  " } });
    expect(calls).toHaveLength(1);
    fireEvent.submit(screen.getByRole("search"));
    await screen.findByRole("link", { name: "物品 中文%_" });
    expect(calls.at(-1)?.get("q")).toBe("中文%_");
    const count = calls.length;
    await search("中".repeat(201));
    expect(screen.getByRole("alert")).toHaveTextContent("最多200");
    expect(calls).toHaveLength(count);
    expect(screen.getByRole("link", { name: "阅读说明书" })).toHaveAttribute("href", "/items/%E4%B8%AD%E6%96%87%25_/releases/release-%E4%B8%AD%E6%96%87%25_");
  });
  it("keeps known results through loading and failure; a late old response cannot replace the applied query", async () => {
    let finish!: (response: Response) => void;
    const pending = new Promise<Response>((resolve) => { finish = resolve; });
    setup((q) => q.get("q") === "slow" ? pending : q.get("q") === "bad" ? errorResponse(500, "INTERNAL", "fixture failure") : jsonResponse({ data: [item(q.get("q") ?? "first")], nextCursor: null }));
    await screen.findByRole("link", { name: "物品 first" });
    await search("slow");
    expect(await screen.findByText("正在查找…，下方为上次结果")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "物品 first" })).toBeInTheDocument();
    await search("new"); await screen.findByRole("link", { name: "物品 new" });
    await act(async () => finish(jsonResponse({ data: [item("slow")], nextCursor: null })));
    expect(screen.queryByRole("link", { name: "物品 slow" })).not.toBeInTheDocument();
    await search("bad");
    expect(await screen.findByRole("alert")).toHaveTextContent("仍显示上次结果");
    expect(screen.getByRole("link", { name: "物品 new" })).toBeInTheDocument();
  });
  it("advances successful pagination without dropping the prefix and resets scope on archive change", async () => {
    const calls = setup((q) => jsonResponse({ data: [item(q.get("archived") === "true" ? "archived" : q.get("cursor") ? "second" : "first")], nextCursor: q.get("cursor") || q.get("archived") === "true" ? null : "next" }));
    await screen.findByRole("link", { name: "物品 first" });
    fireEvent.click(screen.getByRole("button", { name: "加载更多" }));
    await screen.findByRole("link", { name: "物品 second" });
    expect(screen.getByRole("link", { name: "物品 first" })).toBeInTheDocument();
    await waitFor(() => expect(calls).toHaveLength(2));
    fireEvent.click(screen.getByLabelText("显示已归档"));
    await screen.findByRole("link", { name: "物品 archived" });
    expect(calls.at(-1)?.has("cursor")).toBe(false);
    expect(screen.queryByRole("link", { name: "物品 first" })).not.toBeInTheDocument();
  });
  it("offers an explicit reset for a real cursor-field error and clear within empty archived search", async () => {
    const calls = setup((q) => q.has("cursor") ? errorResponse(422, "VALIDATION_FAILED", "条件不匹配", { fields: [{ field: "cursor", message: "请从头开始分页" }] }) : jsonResponse({ data: [], nextCursor: null }), "/?q=x&archived=true&cursor=wrong");
    expect(await screen.findByRole("alert")).toHaveTextContent("列表条件已变化，请回到开头");
    fireEvent.click(screen.getByRole("button", { name: "回到列表开头" }));
    await screen.findByText("当前范围没有匹配物品");
    fireEvent.click(screen.getAllByRole("button", { name: "清除搜索" })[0]!);
    await screen.findByText("没有已归档的物品");
    expect(calls.at(-1)?.get("archived")).toBe("true");
    expect(calls.at(-1)?.has("q")).toBe(false);
  });
});

function HistoryControls() {
  const navigate = useNavigate(), location = useLocation();
  return <><button onClick={() => navigate(-1)}>测试后退</button><button onClick={() => navigate(1)}>测试前进</button><output data-testid="history-url">{location.search}</output></>;
}
it("BUG-PC5-001 keeps committed URL text when immediate Back cancels a clear transition", async () => {
  setup(() => jsonResponse({ data: [item("same archived result")], nextCursor: null }), "/", false);
  render(<QueryClientProvider client={createAppQueryClient()}><MemoryRouter initialEntries={["/?q=aZ19&archived=true"]}><LibraryPage /><HistoryControls /></MemoryRouter></QueryClientProvider>);
  await screen.findByRole("link", { name: "物品 same archived result" });
  // Same rows/cache cannot be used as a promise that a Router transition committed.
  act(() => {
    fireEvent.click(screen.getByRole("button", { name: "清除搜索" }));
    fireEvent.click(screen.getByRole("button", { name: "测试后退" }));
  });
  await waitFor(() => expect(screen.getByTestId("history-url")).toHaveTextContent("?q=aZ19&archived=true"));
  expect(screen.getByLabelText("按名称或型号搜索")).toHaveValue("aZ19");
  expect(screen.getByLabelText("显示已归档")).toBeChecked();
  fireEvent.click(screen.getByRole("button", { name: "测试前进" }));
  await waitFor(() => expect(screen.getByLabelText("按名称或型号搜索")).toHaveValue(""));
  expect(screen.getByTestId("history-url")).toHaveTextContent("?archived=true");
  fireEvent.change(screen.getByLabelText("按名称或型号搜索"), { target: { value: "new-query" } });
  act(() => {
    fireEvent.submit(screen.getByRole("search"));
    fireEvent.click(screen.getByRole("button", { name: "测试后退" }));
  });
  await waitFor(() => expect(screen.getByTestId("history-url")).toHaveTextContent("?archived=true"));
  expect(screen.getByLabelText("按名称或型号搜索")).toHaveValue("");
  fireEvent.click(screen.getByRole("button", { name: "测试前进" }));
  await waitFor(() => expect(screen.getByLabelText("按名称或型号搜索")).toHaveValue("new-query"));
});
