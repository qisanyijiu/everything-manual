import { Group, Mesh, MeshStandardMaterial, BoxGeometry, Vector3 } from "three";

import { PartAnimator, stepMatrices } from "./interactive";
import { readInteractive, type ModelActionView } from "./interactive-view";

const MODEL = { revisionId: "rev-1", sha256: "a".repeat(64) };

function shell(overrides: Record<string, unknown> = {}) {
  return {
    interactive: {
      partsModel: {
        assetId: "parts-1",
        sha256: "b".repeat(64),
        modelRevisionId: "rev-1",
        modelSha256: "a".repeat(64),
        nodeNames: ["body", "cover"],
        source: "test",
      },
      bindings: [{ partId: "part-a", nodes: ["cover"], status: "auto" }],
      actions: [
        {
          id: "open",
          label: "取下电池盖",
          description: null,
          triggerPartIds: ["part-a"],
          mode: "toggle",
          durationMs: 0,
          steps: [{ nodes: ["cover"], kind: "translate", vector: [0, -0.2, 0] }],
          stepIds: [],
        },
      ],
      poses: [],
      ...overrides,
    },
  };
}

describe("readInteractive", () => {
  it("读取与当前模型版本匹配的交互层", () => {
    const view = readInteractive(shell(), MODEL);
    expect(view?.partsModel.assetId).toBe("parts-1");
    expect(view?.bindings[0]?.nodes).toEqual(["cover"]);
    expect(view?.actions[0]?.label).toBe("取下电池盖");
  });

  it("分件附件属于旧模型版本时不可用（不让旧分件驱动新模型）", () => {
    expect(readInteractive(shell(), { revisionId: "rev-2", sha256: "a".repeat(64) })).toBeNull();
    expect(readInteractive({}, MODEL)).toBeNull();
  });
});

describe("stepMatrices", () => {
  it("rotate 绕枢轴旋转：枢轴自身不动，端点按角度移动", () => {
    const mats = stepMatrices([{ nodes: ["n"], kind: "rotate", pivot: [1, 0, 0], axis: [0, 0, 1], angleDeg: 90 }], 1);
    const m = mats.get("n");
    expect(m).toBeDefined();
    const pivot = new Vector3(1, 0, 0).applyMatrix4(m!);
    expect(pivot.distanceTo(new Vector3(1, 0, 0))).toBeLessThan(1e-9);
    const end = new Vector3(2, 0, 0).applyMatrix4(m!);
    expect(end.distanceTo(new Vector3(1, 1, 0))).toBeLessThan(1e-9);
  });

  it("进度 t 线性缩放位移，t=0 为恒等", () => {
    const steps = [{ nodes: ["n"], kind: "translate" as const, vector: [0, -0.2, 0] as const }];
    const half = new Vector3().applyMatrix4(stepMatrices(steps, 0.5).get("n")!);
    expect(half.y).toBeCloseTo(-0.1);
    expect(new Vector3().applyMatrix4(stepMatrices(steps, 0).get("n")!).length()).toBe(0);
  });
});

describe("PartAnimator", () => {
  function scene() {
    const root = new Group();
    const cover = new Mesh(new BoxGeometry(), new MeshStandardMaterial());
    cover.name = "cover";
    cover.position.set(0.1, 0.2, 0.3);
    root.add(cover);
    return { root, cover };
  }
  const action: ModelActionView = {
    id: "open",
    label: "取下电池盖",
    description: null,
    triggerPartIds: [],
    mode: "toggle",
    durationMs: 100,
    steps: [{ nodes: ["cover"], kind: "translate", vector: [0, -0.2, 0] }],
    stepIds: [],
  };

  it("toggle 动作在初始位姿与目标位姿之间切换，复原后回到初始位姿", () => {
    const { root, cover } = scene();
    const animator = new PartAnimator(root, true);
    animator.trigger(action, 0);
    animator.update(10);
    expect(cover.position.y).toBeCloseTo(0.0);
    expect(cover.position.x).toBeCloseTo(0.1);
    animator.trigger(action, 20);
    animator.update(30);
    expect(cover.position.y).toBeCloseTo(0.2);
    animator.trigger(action, 40);
    animator.resetActions(50);
    animator.update(60);
    expect(cover.position.y).toBeCloseTo(0.2);
  });

  it("高亮替换材质副本，dispose 后恢复原材质", () => {
    const { root, cover } = scene();
    const original = cover.material;
    const animator = new PartAnimator(root);
    animator.highlight(["cover"]);
    expect(cover.material).not.toBe(original);
    animator.dispose();
    expect(cover.material).toBe(original);
  });
});
