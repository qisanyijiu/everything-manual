import { WorkProtection } from "../shell/work-protection";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider, QueryObserver } from "@tanstack/react-query";
import { Link, MemoryRouter } from "react-router";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { ProviderSettingsForm } from "./ProviderSettingsForm";
import { errorResponse, jsonResponse } from "../../test/render";
import { fetchSettingsStatus } from "../../api/endpoints";

const tripo = { baseUrl: "https://tripo.example/v3", model: "tripo-model", keyConfigured: true, baseUrlSource: "deployment", modelSource: "deployment", keySource: "deployment" };
const manualAi = { ...tripo, baseUrl: "https://responses.example/v1", model: "manual-model" };
const initial = { revision: "deployment", pending: false, active: { tripo, manualAi }, saved: { tripo, manualAi } };
beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute("open"); };
});
afterEach(() => { vi.unstubAllGlobals(); });
function setup(write: (init: RequestInit) => Response | Promise<Response>, read: (url: string) => Response | Promise<Response> = () => jsonResponse({ data: initial }), queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })) {
  const calls: RequestInit[] = [];
  vi.stubGlobal("fetch", vi.fn(async (_url: string, init: RequestInit = {}) => {
    if (init.method === "PUT") { calls.push(init); return write(init); }
    return read(_url);
  }));
  render(<QueryClientProvider client={queryClient}><MemoryRouter initialEntries={["/settings"]}><WorkProtection><ProviderSettingsForm /><Link to="/jobs">任务中心</Link></WorkProtection></MemoryRouter></QueryClientProvider>);
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
  it.each(["替换", "清除"])("%s后恢复清新密钥，取消回最近saved和keep，另一家编辑不变", async (action) => {
    const saved = { ...initial, revision: "saved-newer", pending: true, saved: { ...initial.saved, tripo: { ...tripo, baseUrl: "https://saved.example/v3", model: "saved-model" } } };
    const calls = setup(() => jsonResponse({ data: saved }), () => jsonResponse({ data: saved }));
    await replaceKey();
    fireEvent.click(within(screen.getByRole("group", { name: "Tripo 密钥操作" })).getByLabelText(action));
    fireEvent.change(screen.getByLabelText("Tripo Base URL"), { target: { value: "https://unsaved.example/v3" } });
    fireEvent.change(screen.getByLabelText("Tripo 模型"), { target: { value: "unsaved-model" } });
    fireEvent.change(screen.getByLabelText("说明书 AI 模型"), { target: { value: "edited-other" } });
    fireEvent.click(within(screen.getByRole("group", { name: "说明书 AI 密钥操作" })).getByLabelText("替换"));
    fireEvent.change(screen.getByLabelText("新的说明书 AI密钥"), { target: { value: "local-test-other-key" } });
    const card = screen.getByRole("region", { name: "Tripo · 模型生成" });
    fireEvent.click(within(card).getByRole("button", { name: "恢复部署配置" }));
    expect(screen.queryByLabelText("新的 Tripo 密钥")).not.toBeInTheDocument();
    expect(document.getElementById("api-tripo.model")).toHaveFocus();
    expect(calls).toHaveLength(0);
    fireEvent.click(within(card).getByRole("button", { name: "取消恢复" }));
    expect(screen.getByLabelText("Tripo Base URL")).toHaveValue("https://saved.example/v3");
    expect(screen.getByLabelText("Tripo Base URL")).toHaveFocus();
    expect(screen.getByLabelText("Tripo 模型")).toHaveValue("saved-model");
    expect(within(screen.getByRole("group", { name: "Tripo 密钥操作" })).getByLabelText("保留现有")).toBeChecked();
    expect(screen.queryByLabelText("新的 Tripo 密钥")).not.toBeInTheDocument();
    expect(screen.getByText("已取消恢复；如需替换密钥，请重新输入。")).toBeInTheDocument();
    fireEvent.click(within(screen.getByRole("group", { name: "Tripo 密钥操作" })).getByLabelText("替换"));
    expect(screen.getByLabelText("新的 Tripo 密钥")).toHaveValue("");
    expect(screen.getByLabelText("说明书 AI 模型")).toHaveValue("edited-other");
    expect(screen.getByLabelText("新的说明书 AI密钥")).toHaveValue("local-test-other-key");
    expect(calls).toHaveLength(0);
  });
  it("冲突保留输入且冻结保存，重读经明确丢弃确认", async () => {
    setup(() => errorResponse(409, "REVISION_CONFLICT", "配置已更新"));
    const input = await replaceKey();
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "重新加载已保存配置" }));
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "继续处理" })).toHaveFocus();
    fireEvent.click(screen.getByRole("button", { name: "继续处理" }));
    expect(input).toHaveValue("local-test-new-key");
    expect(screen.getByRole("button", { name: "保存配置" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "重新加载已保存配置" }));
    fireEvent.click(screen.getByRole("button", { name: "丢弃并重新加载" }));
    await waitFor(() => expect(screen.queryByLabelText("新的 Tripo 密钥")).not.toBeInTheDocument());
  });
});

describe("恢复及配置读取的失败边界", () => {
  it("恢复保存网络结果未知时，保留恢复意图与另一家现有新密钥", async () => {
    const calls = setup(() => { throw new TypeError("Failed to fetch"); });
    await replaceKey();
    fireEvent.click(within(screen.getByRole("group", { name: "说明书 AI 密钥操作" })).getByLabelText("替换"));
    fireEvent.change(screen.getByLabelText("新的说明书 AI密钥"), { target: { value: "local-test-other-key" } });
    fireEvent.change(screen.getByLabelText("说明书 AI 模型"), { target: { value: "edited-other" } });
    fireEvent.click(screen.getAllByRole("button", { name: "恢复部署配置" })[0]!);
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await screen.findByText(/未能保存配置。无法连接服务/);
    expect(calls).toHaveLength(1);
    const body = JSON.parse(String(calls[0]?.body));
    expect(body.tripo).toEqual({ action: "restore" });
    expect(body.manualAi).toMatchObject({ model: "edited-other", keyAction: "replace", apiKey: "local-test-other-key" });
    expect(screen.getByText("保存后恢复部署配置")).toBeInTheDocument();
    expect(screen.getByLabelText("新的说明书 AI密钥")).toHaveValue("local-test-other-key");
    expect(screen.getByText("请求结果可通过重新读取核对；未保存的输入仍保留在当前页。")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "取消恢复" }));
    expect(within(screen.getByRole("group", { name: "Tripo 密钥操作" })).getByLabelText("保留现有")).toBeChecked();
    expect(screen.queryByLabelText("新的 Tripo 密钥")).not.toBeInTheDocument();
    expect(screen.getByLabelText("新的说明书 AI密钥")).toHaveValue("local-test-other-key");
    expect(calls).toHaveLength(1);
  });
  it("确认丢弃后重读失败仍保留输入和旧revision，成功重读才替换", async () => {
    let reads = 0;
    const newer = { ...initial, revision: "newer", saved: { ...initial.saved, tripo: { ...tripo, model: "server-newer" } } };
    const calls = setup(() => errorResponse(409, "REVISION_CONFLICT", "配置已更新"), () => {
      reads += 1;
      return reads === 2 ? errorResponse(500, "INTERNAL_ERROR", "重读失败") : jsonResponse({ data: reads === 1 ? initial : newer });
    });
    const key = await replaceKey();
    fireEvent.change(screen.getByLabelText("Tripo 模型"), { target: { value: "local-edit" } });
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    fireEvent.click(await screen.findByRole("button", { name: "重新加载已保存配置" }));
    fireEvent.click(screen.getByRole("button", { name: "丢弃并重新加载" }));
    await screen.findByText("无法读取 API 配置。重读失败");
    expect(key).toHaveValue("local-test-new-key");
    expect(screen.getByLabelText("Tripo 模型")).toHaveValue("local-edit");
    expect(screen.getByRole("button", { name: "保存配置" })).toBeDisabled();
    expect(JSON.parse(String(calls[0]?.body)).revision).toBe(initial.revision);
    expect(calls).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "重新读取" }));
    fireEvent.click(screen.getByRole("button", { name: "丢弃并重新加载" }));
    await waitFor(() => expect(screen.getByLabelText("Tripo 模型")).toHaveValue("server-newer"));
    expect(screen.queryByLabelText("新的 Tripo 密钥")).not.toBeInTheDocument();
    expect(within(screen.getByRole("group", { name: "Tripo 密钥操作" })).getByLabelText("保留现有")).toBeChecked();
    fireEvent.change(screen.getByLabelText("Tripo 模型"), { target: { value: "next-edit" } });
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(calls).toHaveLength(2));
    expect(JSON.parse(String(calls[1]?.body)).revision).toBe("newer");
  });
  it("成功PUT后状态GET失败仍保持保存成功，不复活新密钥", async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    let statusReads = 0;
    const calls = setup(() => jsonResponse({ data: { ...initial, revision: "saved", pending: true } }), (url) => {
      if (!url.endsWith("/status")) return jsonResponse({ data: initial });
      statusReads += 1;
      return statusReads === 1 ? jsonResponse({ data: {} }) : errorResponse(500, "INTERNAL_ERROR", "状态读取失败");
    }, queryClient);
    const observer = new QueryObserver(queryClient, { queryKey: ["settings", "status"], queryFn: fetchSettingsStatus });
    const unsubscribe = observer.subscribe(() => {});
    try {
      await waitFor(() => expect(observer.getCurrentResult().isSuccess).toBe(true));
      await replaceKey();
      fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
      await waitFor(() => expect(observer.getCurrentResult().isError).toBe(true));
      expect(statusReads).toBe(2);
      expect(calls).toHaveLength(1);
      expect(screen.getByText("所有修改已保存")).toBeInTheDocument();
      expect(screen.queryByLabelText("新的 Tripo 密钥")).not.toBeInTheDocument();
      expect(within(screen.getByRole("group", { name: "Tripo 密钥操作" })).getByLabelText("保留现有")).toBeChecked();
      expect(screen.getByRole("button", { name: "保存配置" })).toBeDisabled();
      expect(screen.queryByText(/未能保存配置/)).not.toBeInTheDocument();
    } finally { unsubscribe(); queryClient.clear(); }
  });
});

const hidden = { ...tripo, model: null, modelIssue: "suspectedCredential" };
const hiddenData = { ...initial, active: { tripo: hidden, manualAi }, saved: { tripo: hidden, manualAi } };
describe("PC06 模型纠正", () => {
  it("已隐藏值不造成初始dirty；其他字段保存不能隐式清空它", async () => {
    const calls = setup(() => jsonResponse({ data: initial }), () => jsonResponse({ data: hiddenData }));
    const input = await screen.findByLabelText("Tripo 模型");
    expect(input).toHaveValue("");
    expect(screen.getByRole("button", { name: "保存配置" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("说明书 AI 模型"), { target: { value: "org/custom-model" } });
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(input).toHaveFocus());
    expect(calls).toHaveLength(0);
    expect(screen.getByText("模型需修正（疑似误填密钥，已隐藏）")).toBeInTheDocument();
  });
  it("新粘贴的疑似密钥立即掩码且不进入DOM属性、存储或请求", async () => {
    const calls = setup(() => jsonResponse({ data: initial }));
    const input = await screen.findByLabelText("说明书 AI 模型");
    const canary = "Bearer sk-pc06_fake_0123456789";
    fireEvent.change(input, { target: { value: canary } });
    expect(input).toHaveAttribute("type", "password");
    expect(input).not.toHaveAttribute("value");
    expect(document.body.innerHTML).not.toContain(canary);
    expect(input).toHaveAttribute("aria-invalid", "true");
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    expect(calls).toHaveLength(0);
    expect(localStorage.length).toBe(0); expect(sessionStorage.length).toBe(0);
    fireEvent.change(input, { target: { value: "local:model-v2" } });
    expect(input).toHaveAttribute("type", "text");
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(calls).toHaveLength(1));
    expect(JSON.parse(String(calls[0]?.body)).manualAi.model).toBe("local:model-v2");
  });
  it("清空需显式确认，取消不变更；确认与密钥选择独立且可以撤销", async () => {
    const calls = setup(() => errorResponse(422, "VALIDATION_FAILED", "测试保存未完成"), () => jsonResponse({ data: hiddenData }));
    await screen.findByLabelText("Tripo 模型");
    fireEvent.click(screen.getByRole("button", { name: "清空误填模型" }));
    fireEvent.click(screen.getByRole("button", { name: /^取消$/ }));
    expect(screen.getByRole("button", { name: "保存配置" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "清空误填模型" }));
    fireEvent.click(screen.getByRole("button", { name: "确认清空模型" }));
    expect(screen.getByText("待清空模型，保存后生效")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(calls).toHaveLength(1));
    expect(JSON.parse(String(calls[0]?.body)).tripo).toMatchObject({ model: "", clearModel: true, keyAction: "keep" });
    await screen.findByText("未能保存配置。测试保存未完成");
    fireEvent.click(screen.getByRole("button", { name: "撤销清空" }));
    fireEvent.change(screen.getByLabelText("Tripo 模型"), { target: { value: "sk-local" } });
    expect(screen.queryByText("待清空模型，保存后生效")).not.toBeInTheDocument();
  });
  it("恢复被拒时保留其他编辑和密钥，取消回saved问题空框与keep", async () => {
    const calls = setup(() => errorResponse(422, "VALIDATION_FAILED", "输入无效", { fields: [{ field: "tripo.model", message: "部署模型疑似误填密钥，不能恢复。" }] }), () => jsonResponse({ data: hiddenData }));
    await replaceKey();
    fireEvent.change(screen.getByLabelText("Tripo 模型"), { target: { value: "org/discarded-model" } });
    fireEvent.change(screen.getByLabelText("说明书 AI 模型"), { target: { value: "org/custom-model" } });
    fireEvent.click(within(screen.getByRole("group", { name: "说明书 AI 密钥操作" })).getByLabelText("替换"));
    fireEvent.change(screen.getByLabelText("新的说明书 AI密钥"), { target: { value: "local-test-other-key" } });
    const card = screen.getByRole("region", { name: "Tripo · 模型生成" });
    fireEvent.click(within(card).getByRole("button", { name: "恢复部署配置" }));
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(document.getElementById("api-tripo.model")).toHaveFocus());
    expect(document.getElementById("api-tripo.model")).toBeVisible();
    expect(screen.getByText("保存后恢复部署配置")).toBeInTheDocument();
    expect(screen.getByLabelText("新的说明书 AI密钥")).toHaveValue("local-test-other-key");
    expect(calls).toHaveLength(1);
    expect(JSON.parse(String(calls[0]?.body)).tripo).toEqual({ action: "restore" });
    fireEvent.click(screen.getByRole("button", { name: "取消恢复" }));
    expect(screen.getByLabelText("Tripo 模型")).toHaveValue("");
    expect(screen.getByText("已保存的模型疑似误填密钥，已隐藏；请填写正确模型")).toBeInTheDocument();
    expect(within(screen.getByRole("group", { name: "Tripo 密钥操作" })).getByLabelText("保留现有")).toBeChecked();
    expect(screen.queryByLabelText("新的 Tripo 密钥")).not.toBeInTheDocument();
    expect(screen.getByLabelText("说明书 AI 模型")).toHaveValue("org/custom-model");
    expect(screen.getByLabelText("新的说明书 AI密钥")).toHaveValue("local-test-other-key");
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(screen.getByLabelText("Tripo 模型")).toHaveFocus());
    expect(calls).toHaveLength(1);
  });
});
