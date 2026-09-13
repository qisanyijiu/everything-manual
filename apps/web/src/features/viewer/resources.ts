/**
 * 3D 资源账本（T18 / REQ-032：「卸载时清理 geometry/material/texture」、
 * 「换模型不串入旧资源」；PRD §5.5 / AC-062 的资源趋势观测）。
 *
 * 为什么需要显式账本：
 * - three 的 `renderer.info` 只反映"当前 GPU 上传"的数量，且随 R3F 内部缓存
 *   波动，不能直接证明"我加载的那份几何/材质/贴图被释放了"；
 * - 测试与 QA 需要可观察证据：`created` 与 `disposed` 必须一一对应，`alive`
 *   在卸载/换模型后回落到基准值（见 `tests/e2e/viewer.spec.ts`）。
 *
 * 账本只记录**本观看器加载的模型资源**（不含 R3F 自建的内置几何，例如
 * OrbitControls 或辅助线），计数在模块作用域，页面内跨组件存活。
 */

import type { BufferGeometry, Material, Object3D, Texture } from "three";

export interface ResourceCounter {
  created: number;
  disposed: number;
  alive: number;
}

export interface ViewerResourceStats {
  geometries: ResourceCounter;
  materials: ResourceCounter;
  textures: ResourceCounter;
  modelsLoaded: number;
  modelsDisposed: number;
  modelsAlive: number;
}

const counters: ViewerResourceStats = {
  geometries: { created: 0, disposed: 0, alive: 0 },
  materials: { created: 0, disposed: 0, alive: 0 },
  textures: { created: 0, disposed: 0, alive: 0 },
  modelsLoaded: 0,
  modelsDisposed: 0,
  modelsAlive: 0,
};

/** 收集场景图中的几何/材质/贴图（三者的"应释放集合"必须与收集口径一致）。 */
export function collectSceneResources(root: Object3D): {
  geometries: Set<BufferGeometry>;
  materials: Set<Material>;
  textures: Set<Texture>;
} {
  const geometries = new Set<BufferGeometry>();
  const materials = new Set<Material>();
  const textures = new Set<Texture>();

  root.traverse((object) => {
    const candidate = object as unknown as {
      geometry?: { isBufferGeometry?: boolean } & BufferGeometry;
      material?: Material | Material[];
    };
    const geometry = candidate.geometry;
    if (geometry !== undefined && geometry.isBufferGeometry === true) {
      geometries.add(geometry);
    }
    const material = candidate.material;
    if (Array.isArray(material)) {
      for (const item of material) {
        materials.add(item);
      }
    } else if (material !== undefined) {
      materials.add(material);
    }
  });

  for (const material of materials) {
    for (const value of Object.values(material as unknown as Record<string, unknown>)) {
      const texture = value as Texture | null;
      if (texture !== null && typeof texture === "object" && texture.isTexture === true) {
        textures.add(texture);
      }
    }
  }
  return { geometries, materials, textures };
}

export type TrackedResources = ReturnType<typeof collectSceneResources>;

/** 一份被记账的模型资源集合（`dispose` 幂等：重复调用不会重复计数）。 */
export interface TrackedModel {
  readonly tracked: TrackedResources;
  readonly counts: { geometries: number; materials: number; textures: number };
  dispose(): void;
}

/**
 * 记入"加载了一份模型"：资源数来自对场景图的收集结果（不猜测）。
 * `dispose()` 释放 geometry/material/texture 并回写账本。
 */
export function trackModelResources(root: Object3D): TrackedModel {
  const tracked = collectSceneResources(root);
  counters.geometries.created += tracked.geometries.size;
  counters.geometries.alive += tracked.geometries.size;
  counters.materials.created += tracked.materials.size;
  counters.materials.alive += tracked.materials.size;
  counters.textures.created += tracked.textures.size;
  counters.textures.alive += tracked.textures.size;
  counters.modelsLoaded += 1;
  counters.modelsAlive += 1;

  let disposed = false;
  return {
    tracked,
    counts: {
      geometries: tracked.geometries.size,
      materials: tracked.materials.size,
      textures: tracked.textures.size,
    },
    dispose(): void {
      if (disposed) {
        return;
      }
      disposed = true;
      // 先贴图（材质引用它们），再材质，最后几何：three 的 `dispose()` 只释放
      // GPU 侧资源；场景对象本身随后整体丢弃，不需逐个断开引用。
      for (const texture of tracked.textures) {
        texture.dispose();
      }
      for (const material of tracked.materials) {
        material.dispose();
      }
      for (const geometry of tracked.geometries) {
        geometry.dispose();
      }
      counters.geometries.disposed += tracked.geometries.size;
      counters.geometries.alive -= tracked.geometries.size;
      counters.materials.disposed += tracked.materials.size;
      counters.materials.alive -= tracked.materials.size;
      counters.textures.disposed += tracked.textures.size;
      counters.textures.alive -= tracked.textures.size;
      counters.modelsDisposed += 1;
      counters.modelsAlive -= 1;
    },
  };
}

/** 快照（测试/QA 读取；不暴露可变引用）。 */
export function viewerResourceStats(): ViewerResourceStats {
  return {
    geometries: { ...counters.geometries },
    materials: { ...counters.materials },
    textures: { ...counters.textures },
    modelsLoaded: counters.modelsLoaded,
    modelsDisposed: counters.modelsDisposed,
    modelsAlive: counters.modelsAlive,
  };
}

/** 重置账本（仅测试使用；页面运行时不调用）。 */
export function resetViewerResources(): void {
  counters.geometries = { created: 0, disposed: 0, alive: 0 };
  counters.materials = { created: 0, disposed: 0, alive: 0 };
  counters.textures = { created: 0, disposed: 0, alive: 0 };
  counters.modelsLoaded = 0;
  counters.modelsDisposed = 0;
  counters.modelsAlive = 0;
}
