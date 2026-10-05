import { Box3, Matrix4, Vector3, type Mesh, type Object3D } from "three";

export interface InspectedPartNode {
  readonly name: string;
  readonly center: Vector3;
  /** 外观观察位移，asset-root 局部空间；不表示实际拆卸方向。 */
  readonly offset: Vector3;
  readonly bounds: Box3;
}

/** 根据原始几何计算固定的展开向量；不读取已播放的动作或姿态。 */
export function inspectPartNodes(root: Object3D, names: readonly string[]): InspectedPartNode[] {
  root.updateWorldMatrix(true, true);
  const rootInverse = root.matrixWorld.clone().invert();
  const byName = new Map<string, Object3D>();
  root.traverse((object) => {
    if (!byName.has(object.name)) byName.set(object.name, object);
  });
  const nodes = [...new Set(names)].flatMap((name) => {
    const object = byName.get(name);
    if (!object || object === root) return [];
    const bounds = new Box3();
    object.traverse((child) => {
      const mesh = child as Mesh;
      if (!mesh.isMesh || mesh.userData.emHotspot) return;
      mesh.geometry.computeBoundingBox();
      if (mesh.geometry.boundingBox) {
        bounds.union(mesh.geometry.boundingBox.clone().applyMatrix4(new Matrix4().multiplyMatrices(rootInverse, mesh.matrixWorld)));
      }
    });
    return bounds.isEmpty() ? [] : [{ name, bounds, center: bounds.getCenter(new Vector3()) }];
  });
  const total = nodes.reduce((box, node) => box.union(node.bounds), new Box3());
  const center = total.getCenter(new Vector3());
  const span = Math.max(total.getSize(new Vector3()).length(), 0.001);
  return nodes.map((node, index) => {
    const direction = node.center.clone().sub(center);
    // 同心分件也必须可展开：用固定球面方向分开，不随机、不累积漂移。
    if (direction.length() < span * 0.015) {
      const y = 1 - (2 * (index + 0.5)) / Math.max(nodes.length, 1);
      const radius = Math.sqrt(Math.max(0, 1 - y * y));
      const angle = index * Math.PI * (3 - Math.sqrt(5));
      direction.set(Math.cos(angle) * radius, y, Math.sin(angle) * radius);
    }
    return { ...node, offset: direction.normalize().multiplyScalar(span * 0.38) };
  });
}

/** 射线命中子网格时，找到最近的登记分件祖先。 */
export function findPartNode(object: Object3D, names: ReadonlySet<string>): string | null {
  for (let current: Object3D | null = object; current !== null; current = current.parent) {
    if (names.has(current.name)) return current.name;
  }
  return null;
}
