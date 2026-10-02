import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setCsrfToken } from "../../api/client";
import { errorResponse, jsonResponse } from "../../test/render";
import { ManualAiModels } from "./ManualAiModels";

afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });

describe("显式读取说明书 AI 模型", () => {
  it("只在点击时 POST，携带 CSRF，无配置请求体；读取不自动选择且阻止重复点击", async () => {
    let resolve!: (response: Response) => void;
    const fetch = vi.fn(() => new Promise<Response>((done) => { resolve = done; }));
    vi.stubGlobal("fetch", fetch); setCsrfToken("synthetic-csrf");
    const onSelect = vi.fn();
    render(<ManualAiModels keyConfigured pending={false} onSelect={onSelect} />);
    expect(fetch).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "读取可用模型" }));
    fireEvent.click(screen.getByRole("button", { name: "正在读取模型…" }));
    expect(fetch).toHaveBeenCalledTimes(1);
    const [url, options] = (fetch.mock.calls as unknown as [string, RequestInit][])[0]!;
    expect(url).toBe("/api/v1/settings/providers/manual-ai/models");
    expect(options.method).toBe("POST"); expect(options.body).toBeUndefined();
    expect(new Headers(options.headers).get("x-csrf-token")).toBe("synthetic-csrf");
    expect(options.cache).toBe("no-store");
    await act(async () => resolve(jsonResponse({ data: ["org/model-v2", "local:model"] })));
    expect(onSelect).not.toHaveBeenCalled();
    fireEvent.change(screen.getByRole("combobox", { name: "可用的说明书 AI 模型" }), { target: { value: "org/model-v2" } });
    expect(onSelect).toHaveBeenCalledExactlyOnceWith("org/model-v2");
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it("pending 明确读取生效配置；生效密钥缺失时不发送请求", () => {
    const fetch = vi.fn(); vi.stubGlobal("fetch", fetch);
    render(<ManualAiModels keyConfigured={false} pending onSelect={vi.fn()} />);
    expect(screen.getByText(/存在待重启配置；本次仍从当前已生效的地址和密钥读取/)).toBeVisible();
    const button = screen.getByRole("button", { name: "读取可用模型" });
    expect(button).toBeDisabled(); fireEvent.click(button); expect(fetch).not.toHaveBeenCalled();
  });

  it("错误保留手填路径，只有再次点击才重试", async () => {
    const fetch = vi.fn().mockResolvedValueOnce(errorResponse(502, "INTERNAL", "模型服务拒绝认证"))
      .mockResolvedValueOnce(jsonResponse({ data: ["valid"] }));
    vi.stubGlobal("fetch", fetch);
    render(<ManualAiModels keyConfigured pending={false} onSelect={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "读取可用模型" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("模型服务拒绝认证");
    expect(screen.getByRole("alert")).toHaveTextContent("手动填写");
    expect(fetch).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "读取可用模型" }));
    expect(await screen.findByRole("combobox")).toBeVisible();
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it("空列表不创建猜测选项", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ data: [] })));
    render(<ManualAiModels keyConfigured pending={false} onSelect={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "读取可用模型" }));
    expect(await screen.findByRole("status")).toHaveTextContent("服务未返回可选模型");
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  });

  it.each([{ data: ["safe", "sk-01234567890123456789"] }, { data: ["safe", 42] }, { data: { data: [] } }])("无效返回不把任何模型写入 DOM：%j", async ({ data }) => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse({ data })));
    render(<ManualAiModels keyConfigured pending={false} onSelect={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "读取可用模型" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("模型列表格式无效"));
    expect(document.body.innerHTML).not.toContain("sk-01234567890123456789");
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  });
});
