import { fireEvent, render, screen } from "@testing-library/react";
import { InteractionPanel } from "./InteractionPanel";
import { useInteractive } from "./useInteractive";
import type { InteractiveView } from "./interactive-view";

const view: InteractiveView = {
  partsModel: { assetId: "parts", sha256: "b".repeat(64), modelRevisionId: "rev", modelSha256: "a".repeat(64), nodeNames: Array.from({ length: 76 }, (_, index) => `tripo_part_${index}`) },
  bindings: [{ partId: "finder", nodes: ["tripo_part_0"], status: "confirmed" }],
  actions: [{ id: "finder-action", label: "取景器展开", description: null, triggerPartIds: ["finder"], mode: "toggle", durationMs: 400, steps: [], stepIds: [] }],
  poses: [],
};

function Harness({ data = view, partId = null }: { data?: InteractiveView; partId?: string | null }) {
  const controller = useInteractive(data, partId)!;
  return <><InteractionPanel {...controller.panel} selectedPartId={partId} /><output data-testid="stage-state">{JSON.stringify({ highlighted: controller.viewerProp.stage.highlightNodes, expanded: controller.viewerProp.stage.expandedNodes })}</output></>;
}

describe("分件列表及控制器", () => {
  it("没有绑定的 76 个编号仍可选择、高亮、过滤和独立复原", () => {
    render(<Harness />);
    expect(screen.getByRole("group", { name: "编号分件列表" }).querySelectorAll("button")).toHaveLength(76);
    fireEvent.change(screen.getByRole("searchbox", { name: "查找编号分件" }), { target: { value: "075" } });
    fireEvent.click(screen.getByTestId("inspect-node-tripo_part_75"));
    expect(screen.getByTestId("stage-state").textContent).toContain('"highlighted":["tripo_part_75"]');
    fireEvent.click(screen.getByTestId("part-inspection-toggle"));
    expect(screen.getByTestId("parts-expanded-count")).toHaveTextContent("已展开 1 / 76");
    expect(screen.getByTestId("stage-state").textContent).toContain('"expanded":["tripo_part_75"]');
    fireEvent.click(screen.getByTestId("part-inspection-toggle"));
    expect(screen.getByTestId("parts-expanded-count")).toHaveTextContent("已展开 0 / 76");
  });
  it("全部展开覆盖所有节点，复原分件保留已触发语义动作", () => {
    render(<Harness />);
    fireEvent.click(screen.getByTestId("action-finder-action"));
    fireEvent.click(screen.getByTestId("parts-expand-all"));
    expect(screen.getByTestId("parts-expanded-count")).toHaveTextContent("已展开 76 / 76");
    fireEvent.click(screen.getByTestId("parts-restore-all"));
    expect(screen.getByTestId("parts-expanded-count")).toHaveTextContent("已展开 0 / 76");
    expect(screen.getByTestId("action-finder-action")).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByTestId("interaction-reset"));
    expect(screen.getByTestId("action-finder-action")).toHaveAttribute("aria-pressed", "false");
  });
  it("说明书部件选择能接回语义高亮，切换模型不带入旧分件状态", () => {
    const result = render(<Harness />);
    fireEvent.click(screen.getByTestId("inspect-node-tripo_part_75"));
    fireEvent.click(screen.getByTestId("part-inspection-toggle"));
    result.rerender(<Harness partId="finder" />);
    expect(screen.getByTestId("stage-state").textContent).toContain('"highlighted":["tripo_part_0"]');
    result.rerender(<Harness data={{ ...view, partsModel: { ...view.partsModel, assetId: "new-parts" } }} />);
    expect(screen.getByTestId("parts-expanded-count")).toHaveTextContent("已展开 0 / 76");
    expect(screen.getByTestId("stage-state").textContent).toContain('"highlighted":[]');
  });
});
