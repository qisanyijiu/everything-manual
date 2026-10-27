import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../../api/client";
import { chatRelease } from "../../api/endpoints";
import { ManualChat } from "./ManualChat";

vi.mock("../../api/endpoints", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../api/endpoints")>(),
  chatRelease: vi.fn(),
}));

const chat = vi.mocked(chatRelease);
beforeEach(() => chat.mockReset());

function show(onOpenPage = vi.fn()) {
  render(<ManualChat itemId="item" releaseId="rel" title="示例说明书" onOpenPage={onOpenPage} />);
  fireEvent.click(screen.getByTestId("manual-chat-toggle"));
  return onOpenPage;
}

describe("ManualChat", () => {
  it("sends full history, shows answer and opens cited page", async () => {
    chat.mockResolvedValueOnce({ data: { answer: "插入快充线。", citedPages: [2], documentId: "doc" }, etag: null });
    chat.mockResolvedValueOnce({ data: { answer: "约 1 小时。", citedPages: [], documentId: "doc" }, etag: null });
    const onOpenPage = show();
    const input = screen.getByTestId("manual-chat-input");
    expect(input).toHaveFocus();
    fireEvent.change(input, { target: { value: "怎么充电？" } });
    await act(async () => fireEvent.keyDown(input, { key: "Enter" }));
    expect(chat).toHaveBeenLastCalledWith("item", "rel", [{ role: "user", content: "怎么充电？" }], expect.any(AbortSignal));
    expect(await screen.findByText("插入快充线。")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("manual-chat-page-2"));
    expect(onOpenPage).toHaveBeenCalledWith("doc", 2);

    fireEvent.change(input, { target: { value: "要多久" } });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "发送" })));
    expect(chat).toHaveBeenLastCalledWith("item", "rel", [
      { role: "user", content: "怎么充电？" },
      { role: "assistant", content: "插入快充线。" },
      { role: "user", content: "要多久" },
    ], expect.any(AbortSignal));
    expect(await screen.findByText("约 1 小时。")).toBeInTheDocument();
  });

  it("shift+enter does not send; failure restores the question and shows the error", async () => {
    chat.mockRejectedValueOnce(new ApiError(409, "PROVIDER_NOT_CONFIGURED", "供应商未配置", "rid"));
    show();
    const input = screen.getByTestId("manual-chat-input");
    fireEvent.change(input, { target: { value: "规格？" } });
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(chat).not.toHaveBeenCalled();
    await act(async () => fireEvent.keyDown(input, { key: "Enter" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(input).toHaveValue("规格？");
    expect(screen.queryByTestId("manual-chat-user")).toBeNull();
  });

  it("escape closes and returns focus to the toggle", () => {
    show();
    fireEvent.keyDown(screen.getByTestId("manual-chat-input"), { key: "Escape" });
    expect(screen.queryByTestId("manual-chat")).toBeNull();
    expect(screen.getByTestId("manual-chat-toggle")).toHaveFocus();
  });
});
