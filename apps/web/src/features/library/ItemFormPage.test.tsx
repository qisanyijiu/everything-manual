import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { errorResponse, jsonResponse, renderApp } from "../../test/render";

const ITEM = { id: "item-created", name: "相机", model: "IA", brand: null, variant: null, revision: 1,
  createdAt: "2026-09-19T00:00:00Z", updatedAt: "2026-09-19T00:00:00Z", archivedAt: null };
function setup(write: (init: RequestInit) => Promise<Response> | Response, edit = false) {
  const calls: RequestInit[] = [];
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = String(input);
    if (url.endsWith("/auth/session")) return jsonResponse({ data: { admin: { id: "admin" }, csrfToken: "fixture", expiresAt: "2099-01-01T00:00:00Z" } });
    if (init.method === "POST" || init.method === "PATCH") { calls.push(init); return write(init); }
    if (url === `/api/v1/items/${ITEM.id}`) return jsonResponse({ data: ITEM }, { etag: '"r1"' });
    if (/\/documents$|\/photos$|\/jobs\?/.test(url)) return jsonResponse({ data: [], nextCursor: null });
    if (url.endsWith("/settings/status")) return jsonResponse({ data: { limits: { pdfMaxBytes: 52428800 } } });
    throw new Error(`Unexpected ${url}`);
  }));
  renderApp({ route: edit ? `/items/${ITEM.id}/edit` : "/items/new" });
  return calls;
}
afterEach(() => { vi.unstubAllGlobals(); });

describe("interaction-a：创建后连续进入上传", () => {
  it("等待创建响应期间禁用，成功后直接上传且仅创建一次", async () => {
    let resolve!: (value: Response) => void;
    const pending = new Promise<Response>((done) => { resolve = done; });
    const calls = setup(() => pending);
    await screen.findByRole("heading", { name: "新建物品" });
    fireEvent.change(screen.getByLabelText(/^名称/), { target: { value: ITEM.name } });
    fireEvent.change(screen.getByLabelText(/^准确型号/), { target: { value: ITEM.model } });
    fireEvent.click(screen.getByRole("button", { name: "创建并继续" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "保存中…" })).toBeDisabled());
    fireEvent.click(screen.getByRole("button", { name: "保存中…" }));
    expect(screen.getByRole("heading", { name: "新建物品" })).toBeInTheDocument();
    await act(async () => resolve(jsonResponse({ data: ITEM }, { status: 201 })));
    expect(await screen.findByRole("heading", { name: "说明书原件" })).toBeInTheDocument();
    expect(screen.getByLabelText("选择 PDF 文件")).toBeInTheDocument();
    expect(calls).toHaveLength(1);
  });

  it("创建失败保留字段和值，不进入上传", async () => {
    const calls = setup(() => errorResponse(422, "VALIDATION_FAILED", "名称无效", { fields: [{ field: "name", message: "名称需调整" }] }));
    await screen.findByRole("heading", { name: "新建物品" });
    fireEvent.change(screen.getByLabelText(/^名称/), { target: { value: ITEM.name } });
    fireEvent.change(screen.getByLabelText(/^准确型号/), { target: { value: ITEM.model } });
    fireEvent.click(screen.getByRole("button", { name: "创建并继续" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("名称需调整");
    expect(screen.getByLabelText(/^名称/)).toHaveValue(ITEM.name);
    expect(screen.getByLabelText(/^准确型号/)).toHaveValue(ITEM.model);
    expect(screen.getByRole("heading", { name: "新建物品" })).toBeInTheDocument();
    expect(screen.queryByLabelText("选择 PDF 文件")).not.toBeInTheDocument();
    expect(calls).toHaveLength(1);
  });

  it("编辑成功继续返回物品概览", async () => {
    const calls = setup(() => jsonResponse({ data: { ...ITEM, revision: 2 } }, { etag: '"r2"' }), true);
    await screen.findByRole("heading", { name: "编辑物品" });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    expect(await screen.findByRole("link", { name: "绑定说明书原件（向导第 2 步）" })).toHaveAttribute("href", `/items/${ITEM.id}/import/document`);
    expect(screen.queryByLabelText("选择 PDF 文件")).not.toBeInTheDocument();
    expect(calls).toHaveLength(1);
    expect(calls[0]?.method).toBe("PATCH");
  });
});
