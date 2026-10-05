import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, vi } from "vitest";
import { InteractionPanel } from "./InteractionPanel";
import { useInteractive } from "./useInteractive";
import type { InteractiveView } from "./interactive-view";

const view: InteractiveView = {
  partsModel: { assetId: "parts", sha256: "b".repeat(64), modelRevisionId: "rev", modelSha256: "a".repeat(64), nodeNames: ["tripo_part_0", "tripo_part_75"] },
  bindings: [], actions: [], poses: [],
};

function Harness({ nested = false }: { nested?: boolean }) {
  const controller = useInteractive(view, null)!;
  const content = <><section className="viewer-panel"><canvas data-testid="same-model-canvas" /></section><InteractionPanel {...controller.panel} selectedPartId={null} /><p data-testid="extra-context">原有上下文</p></>;
  return <main className="page-layout__main">{nested ? <section>{content}</section> : content}</main>;
}

const properties = [
  [HTMLElement.prototype, "requestFullscreen"],
  [document, "exitFullscreen"],
  [document, "fullscreenElement"],
  [document, "fullscreenEnabled"],
] as const;
const originals = properties.map(([object, key]) => Object.getOwnPropertyDescriptor(object, key));
let activeElement: Element | null = null;
function fullscreenApi(enabled = true) {
  activeElement = null;
  Object.defineProperty(document, "fullscreenElement", { configurable: true, get: () => activeElement });
  Object.defineProperty(document, "fullscreenEnabled", { configurable: true, value: enabled });
  const request = vi.fn(function (this: HTMLElement) {
    activeElement = this.closest(".page-layout__main");
    document.dispatchEvent(new Event("fullscreenchange"));
    return Promise.resolve();
  });
  const exit = vi.fn(() => {
    activeElement = null;
    document.dispatchEvent(new Event("fullscreenchange"));
    return Promise.resolve();
  });
  Object.defineProperty(HTMLElement.prototype, "requestFullscreen", { configurable: true, value: request });
  Object.defineProperty(document, "exitFullscreen", { configurable: true, value: exit });
  return { request, exit };
}
afterEach(() => {
  properties.forEach(([object, key], index) => {
    const descriptor = originals[index];
    if (descriptor) Object.defineProperty(object, key, descriptor);
    else Reflect.deleteProperty(object, key);
  });
});

describe("分件全屏观察", () => {
  it("按钮直接请求模型和分件的共同主栏，进入和按钮退出保留同一Canvas与展开状态", async () => {
    const api = fullscreenApi();
    render(<Harness />);
    const canvas = screen.getByTestId("same-model-canvas");
    fireEvent.click(screen.getByTestId("inspect-node-tripo_part_75"));
    fireEvent.click(screen.getByTestId("part-inspection-toggle"));
    const button = screen.getByTestId("parts-fullscreen");
    fireEvent.click(button);
    expect(api.request).toHaveBeenCalledTimes(1);
    expect(api.request.mock.instances[0]).toBe(canvas.closest(".page-layout__main"));
    await waitFor(() => expect(button).toHaveTextContent("退出全屏"));
    await waitFor(() => expect(button).not.toBeDisabled());
    expect(screen.getByTestId("same-model-canvas")).toBe(canvas);
    expect(screen.getByTestId("parts-expanded-count")).toHaveTextContent("已展开 1 / 2");
    fireEvent.click(button);
    await waitFor(() => expect(button).toHaveTextContent("全屏观察"));
    expect(api.exit).toHaveBeenCalledTimes(1);
    expect(canvas.closest(".page-layout__main")).not.toHaveClass("parts-observation-fullscreen");
    expect(screen.getByTestId("same-model-canvas")).toBe(canvas);
    expect(screen.getByTestId("parts-expanded-count")).toHaveTextContent("已展开 1 / 2");
  });

  it("Escape按键退出后更新按钮和焦点，筛选状态不丢失", async () => {
    const api = fullscreenApi(); render(<Harness />);
    const search = screen.getByRole("searchbox", { name: "查找编号分件" });
    fireEvent.change(search, { target: { value: "075" } });
    const button = screen.getByTestId("parts-fullscreen");
    fireEvent.click(button);
    await waitFor(() => expect(button).toHaveTextContent("退出全屏"));
    expect(screen.getByText(/按 Esc 也可退出/)).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(button).toHaveTextContent("全屏观察"));
    expect(api.exit).toHaveBeenCalledTimes(1);
    expect(button).toHaveFocus();
    expect(search).toHaveValue("075");
  });

  it("普通页面和其他元素的全屏不会被本面板的Escape监听退出", () => {
    const api = fullscreenApi(); render(<Harness />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(api.exit).not.toHaveBeenCalled();
    activeElement = document.createElement("section");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(api.exit).not.toHaveBeenCalled();
  });

  it("Escape退出失败仅在本模型仍全屏时提示，浏览器同时已退出则不误报", async () => {
    const api = fullscreenApi(); render(<Harness />);
    const button = screen.getByTestId("parts-fullscreen");
    fireEvent.click(button);
    await waitFor(() => expect(button).toHaveTextContent("退出全屏"));
    api.exit.mockRejectedValueOnce(new Error("failed"));
    fireEvent.keyDown(document, { key: "Escape" });
    expect(await screen.findByRole("alert")).toHaveTextContent("无法退出全屏");
    // 浏览器原生退出先一步完成，但退出 Promise 拒绝：不覆盖为错误的新状态。
    api.exit.mockImplementationOnce(() => {
      activeElement = null;
      document.dispatchEvent(new Event("fullscreenchange"));
      return Promise.reject(new Error("already exited"));
    });
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(button).toHaveTextContent("全屏观察"));
    expect(document.fullscreenElement).toBeNull();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("浏览器拒绝全屏时提示可读错误并可再次尝试，不影响当前分件操作", async () => {
    const api = fullscreenApi(); api.request.mockRejectedValueOnce(new Error("permission denied"));
    render(<Harness />);
    const button = screen.getByTestId("parts-fullscreen");
    fireEvent.click(button);
    expect(await screen.findByRole("alert")).toHaveTextContent("暂时无法进入全屏");
    expect(screen.getByTestId("same-model-canvas").closest(".page-layout__main")).not.toHaveClass("parts-observation-fullscreen");
    fireEvent.click(screen.getByTestId("inspect-node-tripo_part_75"));
    fireEvent.click(screen.getByTestId("part-inspection-toggle"));
    expect(screen.getByTestId("parts-expanded-count")).toHaveTextContent("已展开 1 / 2");
    fireEvent.click(button);
    await waitFor(() => expect(button).toHaveTextContent("退出全屏"));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("不支持全屏时保留普通观察；模型非主栏直系子项时不提供错误入口", () => {
    fullscreenApi(false);
    const result = render(<Harness />);
    expect(screen.queryByTestId("parts-fullscreen")).not.toBeInTheDocument();
    expect(screen.getByTestId("inspect-node-tripo_part_75")).toBeInTheDocument();
    result.unmount(); fullscreenApi(); render(<Harness nested />);
    expect(screen.queryByTestId("parts-fullscreen")).not.toBeInTheDocument();
  });

  it("组件卸载不会退出不属于当前观察主栏的全屏", () => {
    const api = fullscreenApi(); const result = render(<Harness />);
    activeElement = document.createElement("section");
    result.unmount(); expect(api.exit).not.toHaveBeenCalled();
  });
});
