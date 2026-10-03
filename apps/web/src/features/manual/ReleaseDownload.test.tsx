import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../../api/client";
import { fetchReleaseArchive } from "../../api/release-download";
import { ReleaseDownload } from "./ReleaseDownload";

vi.mock("../../api/release-download", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../api/release-download")>(),
  fetchReleaseArchive: vi.fn(),
}));

const fetchArchive = vi.mocked(fetchReleaseArchive);
let clicked: ReturnType<typeof vi.spyOn>;
beforeEach(() => {
  fetchArchive.mockReset();
  vi.stubGlobal("URL", Object.assign(URL, {
    createObjectURL: vi.fn(() => "blob:test-download"), revokeObjectURL: vi.fn(),
  }));
  clicked = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => undefined);
});
afterEach(() => vi.unstubAllGlobals());

function show() {
  return render(<><p id="purpose">原件、模型、manifest 与哈希</p><ReleaseDownload releaseId="old-release" descriptionId="purpose" versionLabel="草稿 r2" /></>);
}

describe("ReleaseDownload lifecycle", () => {
  it("keeps focus and suppresses duplicate activation while pending; reports exact filename", async () => {
    let finish!: (value: { blob: Blob; filename: string }) => void;
    fetchArchive.mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    const view = show();
    const button = screen.getByRole("button", { name: "下载说明书资料包，草稿 r2" });
    button.focus();
    fireEvent.click(button); fireEvent.click(button);
    expect(fetchArchive).toHaveBeenCalledOnce();
    expect(button).toHaveFocus();
    expect(button).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("status")).toHaveTextContent("正在准备此发布版本");
    await act(async () => finish({ blob: new Blob(["zip"]), filename: "release-old-release.zip" }));
    expect(clicked).toHaveBeenCalledOnce();
    expect(clicked.mock.instances[0]).toHaveAttribute("download", "release-old-release.zip");
    expect(screen.getByRole("status")).toHaveTextContent("已发起下载：release-old-release.zip");
    expect(button).toHaveAttribute("aria-disabled", "false");
    view.unmount();
    expect(URL.revokeObjectURL).toHaveBeenCalledWith("blob:test-download");
  });

  it("shows safe error then explicitly retries the same release", async () => {
    fetchArchive.mockRejectedValueOnce(new ApiError(404, "NOT_FOUND", "/private/internal", "request-id"));
    fetchArchive.mockResolvedValueOnce({ blob: new Blob(["zip"]), filename: "old.zip" });
    show();
    fireEvent.click(screen.getByRole("button"));
    expect(await screen.findByRole("alert")).toHaveTextContent("该发布版本已无法找到");
    expect(screen.getByRole("alert")).toHaveTextContent("request-id");
    expect(screen.getByRole("alert")).not.toHaveTextContent("/private/");
    expect(clicked).not.toHaveBeenCalled();
    const retry = screen.getByRole("button", { name: "重试下载，草稿 r2" });
    retry.focus();
    await act(async () => fireEvent.click(retry));
    expect(fetchArchive.mock.calls.map(([id]) => id)).toEqual(["old-release", "old-release"]);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(retry).toHaveFocus();
  });

  it("aborts on leave and discards even a late result", async () => {
    let finish!: (value: { blob: Blob; filename: string }) => void;
    fetchArchive.mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    const view = show();
    fireEvent.click(screen.getByRole("button"));
    const signal = fetchArchive.mock.calls[0]?.[1];
    view.unmount();
    expect(signal?.aborted).toBe(true);
    await act(async () => finish({ blob: new Blob(["zip"]), filename: "old.zip" }));
    expect(clicked).not.toHaveBeenCalled();
    expect(URL.createObjectURL).not.toHaveBeenCalled();
  });
});
