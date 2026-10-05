import { BoxGeometry, Group, Mesh, MeshStandardMaterial, Vector3 } from "three";
import { PartAnimator } from "./interactive";
import { findPartNode, inspectPartNodes } from "./part-inspection";
import type { ModelActionView } from "./interactive-view";

function numberedScene() {
  const root = new Group();
  const nodes = Array.from({ length: 76 }, (_, index) => {
    const mesh = new Mesh(new BoxGeometry(0.06, 0.06, 0.06), new MeshStandardMaterial());
    mesh.name = `tripo_part_${index}`;
    mesh.position.set((index % 8) * 0.1 - 0.35, Math.floor(index / 8) * 0.1 - 0.45, (index % 3) * 0.05);
    root.add(mesh);
    return mesh;
  });
  return { root, nodes, names: nodes.map((node) => node.name) };
}

describe("全部编号分件的独立观察", () => {
  it("76 个分件均有有限非零位移，逐个展开后复原严格回到初始矩阵", () => {
    const { root, nodes, names } = numberedScene();
    const descriptions = inspectPartNodes(root, names);
    expect(descriptions).toHaveLength(76);
    descriptions.forEach((part) => expect(part.offset.length()).toBeGreaterThan(0));
    const original = nodes.map((node) => node.position.clone());
    const animator = new PartAnimator(root, true);
    animator.configureParts(names);
    nodes.forEach((node, index) => {
      animator.setExpandedNodes([node.name], index * 2);
      animator.update(index * 2);
      nodes.forEach((other, otherIndex) => {
        if (other === node) expect(other.position.distanceTo(original[otherIndex]!)).toBeGreaterThan(0);
        else expect(other.position.distanceTo(original[otherIndex]!)).toBe(0);
      });
      animator.setExpandedNodes([], index * 2 + 1);
      animator.update(index * 2 + 1);
      nodes.forEach((other, otherIndex) => expect(other.position.toArray()).toEqual(original[otherIndex]!.toArray()));
    });
    animator.setExpandedNodes(names, 200);
    animator.update(200);
    expect(animator.expandedNodeNames).toHaveLength(76);
    nodes.forEach((node, index) => expect(node.position.distanceTo(original[index]!)).toBeGreaterThan(0));
  });

  it("同心几何使用稳定方向，忽略缺失、重复与空节点", () => {
    const root = new Group();
    for (const name of ["a", "b"]) { const mesh = new Mesh(new BoxGeometry(), new MeshStandardMaterial()); mesh.name = name; root.add(mesh); }
    const first = inspectPartNodes(root, ["a", "b", "a", "missing"]);
    const second = inspectPartNodes(root, ["a", "b"]);
    expect(first).toHaveLength(2);
    expect(first.map((part) => part.offset.toArray())).toEqual(second.map((part) => part.offset.toArray()));
    expect(first[0]!.offset.distanceTo(first[1]!.offset)).toBeGreaterThan(0);
  });

  it("外观展开叠加在现有语义动作上，单独复原不会清除动作", () => {
    const { root, nodes, names } = numberedScene();
    const animator = new PartAnimator(root, true);
    animator.configureParts(names);
    const action: ModelActionView = { id: "original", label: "已有动作", description: null, triggerPartIds: [], mode: "toggle", durationMs: 0, steps: [{ nodes: [names[0]!], kind: "translate", vector: [0, 0.2, 0] }], stepIds: [] };
    const original = nodes[0]!.position.clone();
    animator.trigger(action, 0);
    animator.setExpandedNodes([names[0]!], 0);
    animator.update(0);
    const moved = original.clone().add(new Vector3(0, 0.2, 0));
    expect(nodes[0]!.position.distanceTo(moved)).toBeGreaterThan(0);
    animator.setExpandedNodes([], 1);
    animator.update(1);
    expect(nodes[0]!.position.toArray()).toEqual(moved.toArray());
    animator.resetActions(2);
    animator.update(2);
    expect(nodes[0]!.position.toArray()).toEqual(original.toArray());
  });

  it("根节点旋转缩放与嵌套父分件不会令子件重复展开", () => {
    const root = new Group(); root.position.set(5, 2, 1); root.rotation.z = 0.6; root.scale.set(2, 1.5, 0.5);
    const parent = new Group(); parent.name = "parent"; parent.rotation.y = 0.4; parent.scale.set(0.5, 2, 1);
    const child = new Mesh(new BoxGeometry(), new MeshStandardMaterial()); child.name = "child"; child.position.set(0.4, 0, 0);
    parent.add(child); root.add(parent);
    const original = root.worldToLocal(child.getWorldPosition(new Vector3()));
    const parts = inspectPartNodes(root, ["parent", "child"]);
    const animator = new PartAnimator(root, true); animator.configureParts(["parent", "child"]);
    animator.setExpandedNodes(["parent"], 0); animator.update(0);
    const collapsedChild = root.worldToLocal(child.getWorldPosition(new Vector3()));
    expect(collapsedChild.distanceTo(original)).toBeLessThan(1e-9);
    animator.setExpandedNodes(["parent", "child"], 1); animator.update(1);
    const expandedChild = root.worldToLocal(child.getWorldPosition(new Vector3()));
    expect(expandedChild.distanceTo(original.clone().add(parts.find((part) => part.name === "child")!.offset))).toBeLessThan(1e-9);
    expect(findPartNode(child, new Set(["parent"]))).toBe("parent");
    expect(findPartNode(child, new Set(["parent", "child"]))).toBe("child");
  });

  it("连续更换高亮会释放旧材质副本", () => {
    const { root, nodes } = numberedScene();
    const animator = new PartAnimator(root); animator.highlight([nodes[0]!.name]);
    const tinted = nodes[0]!.material as MeshStandardMaterial;
    let disposed = 0; tinted.addEventListener("dispose", () => { disposed += 1; });
    animator.highlight([nodes[1]!.name]);
    expect(disposed).toBe(1);
    animator.dispose();
  });

  it("已绑定热点跟随观察分件，复原后不改变原始锚点", () => {
    const { root, nodes, names } = numberedScene();
    const marker = new Mesh(new BoxGeometry(0.01, 0.01, 0.01), new MeshStandardMaterial());
    marker.userData = { emHotspot: true, partId: "known" }; marker.position.copy(nodes[0]!.position);
    root.add(marker);
    const original = marker.position.clone();
    const animator = new PartAnimator(root, true); animator.configureParts(names);
    animator.attachMarkers(root, new Map([["known", names[0]!]]));
    animator.setExpandedNodes([names[0]!], 0); animator.update(0);
    expect(marker.position.toArray()).toEqual(nodes[0]!.position.toArray());
    const state = animator.inspectParts();
    expect(state).toHaveLength(76);
    expect(state.find((part) => part.name === names[0])?.expanded).toBe(true);
    expect(state.every((part) => part.matrixRoot.length === 16 && part.matrixRoot.every(Number.isFinite))).toBe(true);
    animator.setExpandedNodes([], 1); animator.update(1);
    expect(marker.position.toArray()).toEqual(original.toArray());
  });
});
