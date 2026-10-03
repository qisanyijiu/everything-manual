import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OriginalDocumentPanel } from "./OriginalDocumentPanel";
import { fetchAssetContent } from "../../api/endpoints";
import { openPdfDocument } from "../import/pdf/vendor";
vi.mock("../../api/endpoints", () => ({ fetchAssetContent: vi.fn() }));
vi.mock("../import/pdf/vendor", () => ({ openPdfDocument: vi.fn(), MAX_RENDER_SCALE: 2 }));
vi.mock("../import/pdf/prepare", () => ({ textItemsToText: (items: { str: string }[]) => items.map((item) => item.str).join(" ") }));
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((yes) => { resolve = yes; }); return { promise, resolve }; }
function documentFixture(label: string, firstRender = Promise.resolve()) {
  const cleanup = vi.fn(), cancel = vi.fn(), destroy = vi.fn().mockResolvedValue(undefined);
  const getPage = vi.fn(async (number: number) => ({
    getViewport: () => ({ width: 300, height: 400 }), cleanup,
    render: () => ({ promise: number === 1 ? firstRender : Promise.resolve(), cancel }),
    getTextContent: async () => ({ items: [{ str: `${label} ${number}` }] }),
  }));
  return { numPages: 2, getPage, loadingTask: { destroy }, cleanup, cancel };
}
beforeEach(() => {
  vi.mocked(fetchAssetContent).mockResolvedValue({ bytes: new ArrayBuffer(1) } as never);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ setTransform: vi.fn(), fillRect: vi.fn() } as never);
});
afterEach(() => vi.restoreAllMocks());

describe("single-page original resources and stale responses", () => {
  it("does not substitute page 1/last page for an invalid reference; valid input recovers", async () => {
    const pdf = documentFixture("FIRST"); vi.mocked(openPdfDocument).mockResolvedValue(pdf as never);
    const onPageChange = vi.fn();
    render(<OriginalDocumentPanel assetId="a" pageNumber={99} fromEvidence onPageChange={onPageChange} />);
    await screen.findByText(/此出处页码超出原件范围/);
    expect(pdf.getPage).not.toHaveBeenCalled();
    expect(screen.getByTestId("original-canvas")).not.toBeVisible();
    fireEvent.change(screen.getByLabelText("页码"), { target: { value: "1.5" } });
    fireEvent.click(screen.getByRole("button", { name: "跳转" }));
    expect(onPageChange).not.toHaveBeenCalled();
    expect(screen.getByLabelText("页码")).toHaveFocus();
    fireEvent.change(screen.getByLabelText("页码"), { target: { value: "2" } });
    fireEvent.click(screen.getByRole("button", { name: "跳转" }));
    expect(onPageChange).toHaveBeenCalledWith(2);
  });
  it("late document open is destroyed and cannot replace current content; unmount destroys current PDF", async () => {
    const old = documentFixture("OLD"), current = documentFixture("CURRENT");
    const pending = deferred<typeof old>();
    vi.mocked(openPdfDocument).mockReturnValueOnce(pending.promise as never).mockResolvedValueOnce(current as never);
    const view = render(<OriginalDocumentPanel assetId="old" pageNumber={1} onPageChange={vi.fn()} />);
    await waitFor(() => expect(openPdfDocument).toHaveBeenCalled());
    view.rerender(<OriginalDocumentPanel assetId="current" pageNumber={2} onPageChange={vi.fn()} />);
    await screen.findByText("CURRENT 2");
    await act(async () => pending.resolve(old));
    expect(old.loadingTask.destroy).toHaveBeenCalledTimes(1);
    expect(old.getPage).not.toHaveBeenCalled();
    expect(screen.queryByText("OLD 1")).not.toBeInTheDocument();
    view.unmount();
    expect(current.loadingTask.destroy).toHaveBeenCalledTimes(1);
  });
  it("changing page cancels the previous render; its late text cannot overwrite the new page", async () => {
    const pending = deferred<void>();
    const pdf = documentFixture("PAGE", pending.promise);
    vi.mocked(openPdfDocument).mockResolvedValue(pdf as never);
    const view = render(<OriginalDocumentPanel assetId="a" pageNumber={1} onPageChange={vi.fn()} />);
    await waitFor(() => expect(pdf.getPage).toHaveBeenCalledWith(1));
    view.rerender(<OriginalDocumentPanel assetId="a" pageNumber={2} onPageChange={vi.fn()} />);
    await screen.findByText("PAGE 2");
    expect(pdf.cancel).toHaveBeenCalled();
    await act(async () => pending.resolve());
    expect(screen.queryByText("PAGE 1")).not.toBeInTheDocument();
    expect(screen.getByText("PAGE 2")).toBeInTheDocument();
    view.unmount();
    expect(pdf.cleanup).toHaveBeenCalledTimes(2);
  });
});
