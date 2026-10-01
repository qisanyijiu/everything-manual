import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Link, MemoryRouter } from "react-router";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ProviderSettingsForm } from "./ProviderSettingsForm";
import { errorResponse, jsonResponse } from "../../test/render";

const tripo = { baseUrl: "https://tripo.example/v3", model: "tripo-model", keyConfigured: true, baseUrlSource: "deployment", modelSource: "deployment", keySource: "deployment" };
const manualAi = { ...tripo, baseUrl: "https://responses.example/v1", model: "manual-model" };
const initial = { revision: "deployment", pending: false, active: { tripo, manualAi }, saved: { tripo, manualAi } };
beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute("open"); };
});
afterEach(() => { vi.unstubAllGlobals(); });
function setup(write: (init: RequestInit) => Response | Promise<Response>, read = () => jsonResponse({ data: initial })) {
  const calls: RequestInit[] = [];
  vi.stubGlobal("fetch", vi.fn(async (_url: string, init: RequestInit = {}) => {
    if (init.method === "PUT") { calls.push(init); return write(init); }
    return read();
  }));
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}><MemoryRouter initialEntries={["/settings"]}><ProviderSettingsForm /><Link to="/jobs">任务中心</Link></MemoryRouter></QueryClientProvider>);
  return calls;
}
async function replaceKey() {
  await screen.findByLabelText("Tripo Base URL");
  fireEvent.click(within(screen.getByRole("group", { name: "Tripo 密钥操作" })).getByLabelText("替换"));
  const input = screen.getByLabelText("新的 Tripo 密钥");
  fireEvent.change(input, { target: { value: "local-test-new-key" } });
  return input;
}

describe("API 配置保存与秘密生命周期", () => {
  it("读取失败不创建可保存的猜测表单", async () => {
    const calls = setup(() => jsonResponse({}), () => errorResponse(500, "INTERNAL_ERROR", "读取失败"));
    expect(await screen.findByRole("button", { name: "重新读取" })).toBeInTheDocument();
    expect(screen.queryByLabelText("Tripo Base URL")).not.toBeInTheDocument();
    expect(calls).toHaveLength(0);
  });
  it("密钥不写DOM属性，服务器字段错误解除保存锁后聚焦并保留输入", async () => {
    setup(() => errorResponse(422, "VALIDATION_FAILED", "输入无效", { fields: [{ field: "tripo.apiKey", message: "密钥格式错误" }] }));
    const input = await replaceKey();
    expect(input.getAttribute("value")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(input).toHaveFocus());
    expect(input).toHaveValue("local-test-new-key");
    expect(input).not.toBeDisabled();
    expect(input.getAttribute("value")).toBeNull();
    expect(localStorage.length).toBe(0); expect(sessionStorage.length).toBe(0);
  });
  it("保存中防重，成功清空新密钥且分别展示运行和已保存值", async () => {
    let resolve!: (r: Response) => void;
    const promise = new Promise<Response>((done) => { resolve = done; });
    const calls = setup(() => promise);
    await replaceKey();
    fireEvent.change(screen.getByLabelText("Tripo Base URL"), { target: { value: "https://changed.example/v3" } });
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    fireEvent.click(screen.getByRole("button", { name: "正在保存…" }));
    expect(calls).toHaveLength(1);
    expect(screen.getByLabelText("Tripo Base URL")).toBeDisabled();
    await act(async () => resolve(jsonResponse({ data: { ...initial, revision: "next", pending: true, saved: { ...initial.saved, tripo: { ...tripo, baseUrl: "https://changed.example/v3", keySource: "web" } } } })));
    expect(screen.queryByLabelText("新的 Tripo 密钥")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Tripo Base URL")).toHaveValue("https://changed.example/v3");
    expect(screen.getByText("https://tripo.example/v3")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "保存配置" })).toBeDisabled();
  });
  it("退出替换与取消恢复均清空密钥，另一家编辑保留", async () => {
    setup(() => jsonResponse({ data: initial }));
    await replaceKey();
    fireEvent.change(screen.getByLabelText("说明书 AI 模型"), { target: { value: "edited-other" } });
    const card = screen.getByRole("region", { name: "Tripo · 模型生成" });
    fireEvent.click(within(card).getByRole("button", { name: "恢复部署配置" }));
    expect(screen.queryByLabelText("新的 Tripo 密钥")).not.toBeInTheDocument();
    fireEvent.click(within(card).getByRole("button", { name: "取消恢复" }));
    fireEvent.click(within(screen.getByRole("group", { name: "Tripo 密钥操作" })).getByLabelText("替换"));
    expect(screen.getByLabelText("新的 Tripo 密钥")).toHaveValue("");
    expect(screen.getByLabelText("说明书 AI 模型")).toHaveValue("edited-other");
  });
  it("冲突保留输入且冻结保存，重读经明确丢弃确认", async () => {
    setup(() => errorResponse(409, "REVISION_CONFLICT", "配置已更新"));
    const input = await replaceKey();
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "重新加载已保存配置" }));
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "继续编辑" })).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "继续编辑" }));
    expect(input).toHaveValue("local-test-new-key");
    expect(screen.getByRole("button", { name: "保存配置" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "重新加载已保存配置" }));
    fireEvent.click(screen.getByRole("button", { name: "丢弃并重新加载" }));
    await waitFor(() => expect(screen.queryByLabelText("新的 Tripo 密钥")).not.toBeInTheDocument());
  });
});
