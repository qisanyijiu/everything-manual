/**
 * 离线 3D 阅读器（导出单文件 HTML 的运行时；由 `vite.standalone-viewer.ts` 打成 IIFE）。
 *
 * 运行环境：用户双击打开的本地文件（`file://`），**没有服务端、没有网络**。
 * - 模型：从 `<script id="em-model">` 读 base64 GLB → `GLTFLoader.parse`；
 * - 知识：从 `<script id="em-payload">` 读 JSON（schema 见 `../payload.ts`）；
 * - 交互：OrbitControls 旋转/缩放；部件列表 ↔ 3D 热点 ↔ 步骤联动；
 * - 安全：全部文字用 textContent 写入，不拼接 HTML；不发起任何网络请求。
 *
 * 坐标约定与 SPA 一致：热点 `positionLocal` 是 GLB 场景根的局部坐标；居中/缩放放在
 * 外层 display group，不改场景根自身变换（见 `../../viewer/coordinates.ts`）。
 */

import {
  ACESFilmicToneMapping,
  Box3,
  Color,
  Group,
  Mesh,
  MeshBasicMaterial,
  PMREMGenerator,
  PerspectiveCamera,
  Raycaster,
  Scene,
  SphereGeometry,
  SRGBColorSpace,
  Vector2,
  Vector3,
  WebGLRenderer,
  type Object3D,
} from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";

import {
  STANDALONE_MODEL_ELEMENT_ID,
  STANDALONE_PAYLOAD_ELEMENT_ID,
  STANDALONE_PAYLOAD_VERSION,
  type StandaloneEvidence,
  type StandaloneHotspot,
  type StandalonePayload,
} from "../payload";
import { PartAnimator } from "../../viewer/interactive";
import { partNodeLabel, type ModelActionView } from "../../viewer/interactive-view";
import { findPartNode } from "../../viewer/part-inspection";

// 与应用阅读器同一套 token 取值（--color-accent / --color-accent-active）。
// 发布版只含已确认热点（实心圆语义），离线侧不画候选。
const HOTSPOT_COLOR = 0xa63f21;
const HOTSPOT_SELECTED_COLOR = 0x7a2c12;

function byId<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (element === null) {
    throw new Error(`离线阅读器缺少元素 #${id}`);
  }
  return element as T;
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  options: { className?: string; text?: string } = {},
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (options.className !== undefined) {
    node.className = options.className;
  }
  if (options.text !== undefined) {
    node.textContent = options.text;
  }
  return node;
}

function readPayload(): StandalonePayload {
  const raw = byId<HTMLScriptElement>(STANDALONE_PAYLOAD_ELEMENT_ID).textContent ?? "";
  const payload = JSON.parse(raw) as StandalonePayload;
  if (payload.schemaVersion !== STANDALONE_PAYLOAD_VERSION) {
    throw new Error(`不支持的离线包版本：${String(payload.schemaVersion)}`);
  }
  return payload;
}

function readModelBytes(): ArrayBuffer {
  const base64 = (byId<HTMLScriptElement>(STANDALONE_MODEL_ELEMENT_ID).textContent ?? "").trim();
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes.buffer;
}

function evidenceLabel(evidence: readonly StandaloneEvidence[]): string | null {
  const pages = [...new Set(evidence.map((item) => item.pageNumber))].sort((a, b) => a - b);
  return pages.length === 0 ? null : `原文：第 ${pages.join("、")} 页`;
}

/** 发布时间显示：可解析时本地化，否则原样显示（不猜测）。 */
function formatPublished(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString("zh-CN", { hour12: false });
}

interface Viewer {
  selectPart(partId: string | null): void;
  selectNode(name: string | null): void;
  readonly nodeNames: readonly string[];
  partStates(): readonly { name: string; matrix: number[]; expanded: boolean }[];
  resetView(): void;
  /** 交互层（分件模型才有）。 */
  animator: PartAnimator | null;
}

function setStatus(text: string, isError = false): void {
  const status = byId<HTMLParagraphElement>("em-status");
  status.textContent = text;
  status.classList.toggle("is-error", isError);
}

async function createViewer(
  payload: StandalonePayload,
  onPick: (partId: string) => void,
  onNodePick: (name: string) => void,
): Promise<Viewer> {
  const host = byId<HTMLDivElement>("em-canvas");
  const renderer = new WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.outputColorSpace = SRGBColorSpace;
  renderer.toneMapping = ACESFilmicToneMapping;
  host.appendChild(renderer.domElement);

  const scene = new Scene();
  // 纸面底色（--color-paper）：与网页阅读器的模型空白区一致。
  scene.background = new Color(0xf4f1e8);
  const pmrem = new PMREMGenerator(renderer);
  scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;

  const camera = new PerspectiveCamera(40, 1, 0.01, 100);
  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = !window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  const gltf = await new GLTFLoader().parseAsync(readModelBytes(), "");
  const assetRoot: Object3D = gltf.scene;
  const fallbackNames: string[] = [];
  assetRoot.traverse((object) => {
    if ((object as Mesh).isMesh && object.name !== "") fallbackNames.push(object.name);
  });
  const nodeNames = payload.interactive === null ? [] : [...new Set(payload.interactive.nodeNames ?? fallbackNames)];
  const registeredNodes = new Set(nodeNames);
  // 显示变换只放在外层：居中 + 统一缩放到单位尺寸。
  const display = new Group();
  display.add(assetRoot);
  scene.add(display);
  const box = new Box3().setFromObject(assetRoot);
  const size = box.getSize(new Vector3());
  const center = box.getCenter(new Vector3());
  const scale = 1 / Math.max(size.x, size.y, size.z, 1e-6);
  display.scale.setScalar(scale);
  display.position.copy(center.multiplyScalar(-scale));

  // 热点挂在 asset-root 下：直接用局部坐标，与 SPA 同一语义；球体反向缩放保持屏幕尺寸稳定。
  const markerRadius = 0.018 / scale;
  const markers = new Map<string, { mesh: Mesh; hotspot: StandaloneHotspot }>();
  for (const hotspot of payload.hotspots) {
    const material = new MeshBasicMaterial({ color: HOTSPOT_COLOR, depthTest: false });
    const mesh = new Mesh(new SphereGeometry(markerRadius, 20, 14), material);
    mesh.position.set(...hotspot.positionLocal);
    mesh.renderOrder = 10;
    mesh.userData.partId = hotspot.partId;
    mesh.userData.emHotspot = true;
    assetRoot.add(mesh);
    markers.set(hotspot.id, { mesh, hotspot });
  }

  const resetView = (): void => {
    assetRoot.updateWorldMatrix(true, true);
    const current = new Box3().setFromObject(assetRoot);
    const target = current.getCenter(new Vector3());
    const radius = current.getSize(new Vector3()).length() * 0.5;
    const verticalHalfFov = (camera.fov * Math.PI) / 360;
    const horizontalHalfFov = Math.atan(Math.tan(verticalHalfFov) * camera.aspect);
    const distance = Math.max(radius / Math.sin(Math.min(verticalHalfFov, horizontalHalfFov)) * 1.1, 0.6);
    camera.position.copy(target).add(new Vector3(0.9, 0.55, 1.35).normalize().multiplyScalar(distance));
    controls.target.copy(target);
    controls.update();
  };
  const resize = (): void => {
    const width = host.clientWidth;
    const height = host.clientHeight;
    renderer.setSize(width, height, false);
    camera.aspect = width / Math.max(height, 1);
    camera.updateProjectionMatrix();
  };
  new ResizeObserver(resize).observe(host);
  resize();
  resetView();

  // 点击热点 → 选中部件（拖动旋转不算点击）。
  let downAt: { x: number; y: number } | null = null;
  renderer.domElement.addEventListener("pointerdown", (event) => {
    downAt = { x: event.clientX, y: event.clientY };
  });
  const raycaster = new Raycaster();
  renderer.domElement.addEventListener("pointerup", (event) => {
    if (downAt === null || Math.hypot(event.clientX - downAt.x, event.clientY - downAt.y) > 4) {
      return;
    }
    const rect = renderer.domElement.getBoundingClientRect();
    const pointer = new Vector2(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1,
    );
    raycaster.setFromCamera(pointer, camera);
    const hit = raycaster.intersectObjects([...markers.values()].map((item) => item.mesh))[0];
    const partId = hit?.object.userData.partId;
    if (typeof partId === "string") {
      onPick(partId);
      return;
    }
    const surfaceHit = raycaster.intersectObject(assetRoot, true).find((entry) => !entry.object.userData.emHotspot);
    const nodeName = surfaceHit ? findPartNode(surfaceHit.object, registeredNodes) : null;
    if (nodeName !== null) onNodePick(nodeName);
  });

  const animator =
    payload.interactive === null
      ? null
      : new PartAnimator(assetRoot, window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  if (animator !== null && payload.interactive !== null) {
    animator.configureParts(nodeNames);
    animator.attachMarkers(
      assetRoot,
      new Map(payload.interactive.bindings.flatMap((b) => (b.nodes[0] ? [[b.partId, b.nodes[0]] as const] : []))),
    );
  }
  renderer.setAnimationLoop(() => {
    animator?.update(performance.now());
    controls.update();
    renderer.render(scene, camera);
  });

  return {
    resetView,
    animator,
    nodeNames,
    selectNode(name) {
      animator?.highlight(name === null ? [] : [name]);
    },
    partStates() {
      assetRoot.updateWorldMatrix(true, true);
      const inverse = assetRoot.matrixWorld.clone().invert();
      const expanded = new Set(animator?.expandedNodeNames ?? []);
      return nodeNames.flatMap((name) => {
        const node = assetRoot.getObjectByName(name);
        return node ? [{ name, matrix: inverse.clone().multiply(node.matrixWorld).toArray(), expanded: expanded.has(name) }] : [];
      });
    },
    selectPart(partId) {
      const nodes = payload.interactive?.bindings.find((b) => b.partId === partId)?.nodes ?? [];
      animator?.highlight(nodes);
      for (const { mesh, hotspot } of markers.values()) {
        const selected = hotspot.partId === partId;
        (mesh.material as MeshBasicMaterial).color.setHex(
          selected ? HOTSPOT_SELECTED_COLOR : HOTSPOT_COLOR,
        );
        mesh.scale.setScalar(selected ? 1.3 : 1);
      }
    },
  };
}

function renderPanels(payload: StandalonePayload, select: (partId: string | null) => void) {
  const partButtons = new Map<string, HTMLButtonElement>();
  const partsList = byId<HTMLUListElement>("em-parts");
  const hotspotCount = new Map<string, number>();
  for (const hotspot of payload.hotspots) {
    hotspotCount.set(hotspot.partId, (hotspotCount.get(hotspot.partId) ?? 0) + 1);
  }
  for (const part of payload.parts) {
    const item = el("li", { className: "part" });
    const button = el("button", { className: "part__name", text: part.name });
    button.type = "button";
    button.setAttribute("aria-pressed", "false");
    button.addEventListener("click", () => select(part.id));
    item.append(button);
    const count = hotspotCount.get(part.id) ?? 0;
    item.append(el("span", { className: "tag", text: count > 0 ? `热点 ${count}` : "无热点" }));
    if (part.description !== "") {
      item.append(el("p", { text: part.description }));
    }
    const source = evidenceLabel(part.evidence);
    if (source !== null) {
      item.append(el("p", { className: "source", text: source }));
    }
    partsList.append(item);
    partButtons.set(part.id, button);
  }

  const specs = byId<HTMLDListElement>("em-specs");
  for (const spec of payload.specs) {
    specs.append(el("dt", { text: spec.label }), el("dd", { text: spec.value }));
  }
  if (payload.specs.length === 0) {
    byId<HTMLElement>("em-specs-section").hidden = true;
  }

  const stepBody = byId<HTMLDivElement>("em-step");
  const stepCounter = byId<HTMLSpanElement>("em-step-counter");
  const prev = byId<HTMLButtonElement>("em-step-prev");
  const next = byId<HTMLButtonElement>("em-step-next");
  let stepIndex = 0;
  const showStep = (index: number, focusPart: boolean): void => {
    if (payload.steps.length === 0) {
      stepBody.replaceChildren(el("p", { text: "该版本没有步骤。" }));
      stepCounter.textContent = "";
      prev.disabled = true;
      next.disabled = true;
      return;
    }
    stepIndex = Math.min(Math.max(index, 0), payload.steps.length - 1);
    const step = payload.steps[stepIndex];
    if (step === undefined) {
      return;
    }
    stepCounter.textContent = `第 ${stepIndex + 1} / ${payload.steps.length} 步`;
    prev.disabled = stepIndex === 0;
    next.disabled = stepIndex === payload.steps.length - 1;
    const list = el("ol");
    for (const action of step.orderedActions) {
      list.append(el("li", { text: action }));
    }
    const nodes: Node[] = [el("h3", { text: step.title }), list];
    for (const note of step.safetyNotes) {
      nodes.push(el("p", { className: "warning", text: `注意：${note}` }));
    }
    const names = step.partIds
      .map((id) => payload.parts.find((part) => part.id === id)?.name)
      .filter((name): name is string => name !== undefined);
    if (names.length > 0) {
      nodes.push(el("p", { className: "source", text: `涉及部件：${names.join("、")}` }));
    }
    const source = evidenceLabel(step.evidence);
    if (source !== null) {
      nodes.push(el("p", { className: "source", text: source }));
    }
    stepBody.replaceChildren(...nodes);
    if (focusPart) {
      select(step.partIds[0] ?? null);
    }
  };
  prev.addEventListener("click", () => showStep(stepIndex - 1, true));
  next.addEventListener("click", () => showStep(stepIndex + 1, true));
  showStep(0, false);

  return {
    markPart(partId: string | null) {
      for (const [id, button] of partButtons) {
        const selected = id === partId;
        button.setAttribute("aria-pressed", String(selected));
        button.parentElement?.classList.toggle("is-selected", selected);
        if (selected) {
          button.parentElement?.scrollIntoView({ block: "nearest" });
        }
      }
    },
  };
}

/** 姿势与动作按钮（离线版；与在线 InteractionPanel 同一语义）。 */
function renderInteractions(payload: StandalonePayload, viewer: Viewer): void {
  const host = byId<HTMLDivElement>("em-interactions");
  const interactive = payload.interactive;
  const animator = viewer.animator;
  if (interactive === null || animator === null) {
    host.hidden = true;
    return;
  }
  const group = (label: string): HTMLDivElement => {
    const row = el("div", { className: "chips" });
    row.setAttribute("role", "group");
    row.setAttribute("aria-label", label);
    row.append(el("span", { className: "chips__label", text: label }));
    host.append(row);
    return row;
  };
  if (viewer.nodeNames.length > 0) renderPartsExplorer(host, viewer);
  if (interactive.poses.length > 0) {
    const row = group("姿势");
    const buttons: HTMLButtonElement[] = [];
    interactive.poses.forEach((pose, index) => {
      const button = el("button", { className: "chip", text: pose.label });
      button.type = "button";
      button.title = pose.description ?? "";
      button.setAttribute("aria-pressed", String(index === 0));
      button.addEventListener("click", () => {
        animator.setPose(pose, performance.now());
        buttons.forEach((b) => b.setAttribute("aria-pressed", String(b === button)));
      });
      buttons.push(button);
      row.append(button);
    });
  }
  if (interactive.actions.length > 0) {
    const row = group("动作");
    const toggles = new Map<string, HTMLButtonElement>();
    for (const action of interactive.actions as readonly ModelActionView[]) {
      const button = el("button", { className: "chip", text: action.label });
      button.type = "button";
      button.title = action.description ?? "";
      if (action.mode === "toggle") {
        button.setAttribute("aria-pressed", "false");
        toggles.set(action.id, button);
      }
      button.addEventListener("click", () => {
        const on = animator.trigger(action, performance.now());
        if (action.mode === "toggle") {
          button.setAttribute("aria-pressed", String(on));
        }
      });
      row.append(button);
    }
    const reset = el("button", { className: "chip", text: "复原" });
    reset.type = "button";
    reset.addEventListener("click", () => {
      animator.resetActions(performance.now());
      animator.setPose(null, performance.now());
      toggles.forEach((b) => b.setAttribute("aria-pressed", "false"));
      host.dispatchEvent(new Event("em-inspection-reset"));
    });
    row.append(reset);
  }
  host.append(el("p", { className: "source", text: "动作与姿势为外观示意（分件模型刚体变换），不代表真实机械结构。" }));
}

/** 独立网格观察与说明书动作分别展示，编号不冒充已确认的部件名称。 */
function renderPartsExplorer(host: HTMLElement, viewer: Viewer): void {
  const animator = viewer.animator;
  if (animator === null) return;
  const section = el("section", { className: "mesh-explorer" });
  section.setAttribute("aria-label", "全部分件观察");
  section.dataset.testid = "offline-parts-explorer";
  section.append(el("h3", { text: `全部分件 · ${viewer.nodeNames.length}` }));
  section.append(el("p", { className: "source", text: "编号对应模型几何分区；展开仅供观察，不表示真实拆卸方向或机械零件。" }));
  const search = el("input");
  search.type = "search";
  search.placeholder = "搜索分件编号";
  search.setAttribute("aria-label", "搜索分件编号");
  const list = el("div", { className: "mesh-explorer__list" });
  list.setAttribute("role", "group");
  list.setAttribute("aria-label", "分件编号");
  const detail = el("p", { className: "source", text: "选择一个编号，观察对应网格。" });
  detail.setAttribute("aria-live", "polite");
  const controls = el("div", { className: "chips" });
  const toggle = el("button", { className: "chip", text: "展开选中分件" });
  toggle.type = "button";
  toggle.disabled = true;
  toggle.dataset.testid = "offline-part-toggle";
  const all = el("button", { className: "chip", text: "全部展开" });
  all.type = "button";
  all.dataset.testid = "offline-parts-expand-all";
  const reset = el("button", { className: "chip", text: "全部复原" });
  reset.type = "button";
  reset.dataset.testid = "offline-parts-reset";
  let selected: string | null = null;
  let expanded = new Set<string>();
  const buttons = new Map<string, HTMLButtonElement>();
  const update = (): void => {
    const active = selected !== null && expanded.has(selected);
    toggle.disabled = selected === null;
    toggle.textContent = active ? "复原选中分件" : "展开选中分件";
    toggle.setAttribute("aria-pressed", String(active));
    detail.textContent = selected === null ? "选择一个编号，观察对应网格。" : `${partNodeLabel(selected)} · ${expanded.has(selected) ? "已展开" : "原位"}`;
    for (const [name, button] of buttons) {
      button.setAttribute("aria-pressed", String(name === selected));
      button.dataset.expanded = String(expanded.has(name));
    }
  };
  const select = (name: string): void => {
    selected = name;
    viewer.selectNode(name);
    update();
  };
  for (const name of viewer.nodeNames) {
    const button = el("button", { className: "chip", text: partNodeLabel(name) });
    button.type = "button";
    button.title = name;
    button.dataset.testid = `offline-inspect-node-${name}`;
    button.addEventListener("click", () => select(name));
    buttons.set(name, button);
    list.append(button);
  }
  search.addEventListener("input", () => {
    const query = search.value.trim().toLowerCase();
    for (const [name, button] of buttons) button.hidden = !`${name} ${partNodeLabel(name)}`.toLowerCase().includes(query);
  });
  toggle.addEventListener("click", () => {
    if (selected === null) return;
    if (expanded.has(selected)) expanded.delete(selected);
    else expanded.add(selected);
    animator.setExpandedNodes([...expanded], performance.now());
    update();
  });
  all.addEventListener("click", () => {
    expanded = new Set(viewer.nodeNames);
    animator.setExpandedNodes([...expanded], performance.now());
    update();
    setTimeout(() => viewer.resetView(), 450);
  });
  reset.addEventListener("click", () => {
    expanded.clear();
    animator.setExpandedNodes([], performance.now());
    update();
  });
  controls.append(toggle, all, reset);
  section.append(search, list, detail, controls);
  host.prepend(section);
  // Local QA can read geometry only. No setters, bytes, credentials or requests.
  if (new URLSearchParams(location.hash.slice(1)).has("em-inspect-qa")) {
    Object.defineProperty(window, "__EM_OFFLINE_PARTS__", { value: { parts: () => viewer.partStates() }, configurable: true });
  }
  host.addEventListener("em-node-pick", (event) => {
    const name = (event as CustomEvent<string>).detail;
    if (buttons.has(name)) select(name);
  });
  host.addEventListener("em-inspection-reset", () => reset.click());
  host.addEventListener("em-part-selected", () => { selected = null; update(); });
  update();
}

async function main(): Promise<void> {
  const payload = readPayload();
  byId<HTMLHeadingElement>("em-title").textContent = payload.title;
  byId<HTMLParagraphElement>("em-subtitle").textContent = payload.subtitle;
  // 版本信息公开可展开（与网页阅读器的「查看版本信息」同语义）；内容为发布时冻结版本。
  const published =
    payload.publishedAt === null
      ? ""
      : ` · 发布于 ${formatPublished(payload.publishedAt)}`;
  byId<HTMLParagraphElement>("em-release").textContent =
    `发布版本 ${payload.releaseId}${published} · 内容为发布时冻结版本（不可变）`;

  let viewer: Viewer | null = null;
  let panels: { markPart(partId: string | null): void } | null = null;
  const select = (partId: string | null): void => {
    panels?.markPart(partId);
    viewer?.selectPart(partId);
    byId<HTMLDivElement>("em-interactions").dispatchEvent(new Event("em-part-selected"));
  };
  panels = renderPanels(payload, select);

  try {
    viewer = await createViewer(payload, select, (name) => {
      byId<HTMLDivElement>("em-interactions").dispatchEvent(new CustomEvent("em-node-pick", { detail: name }));
    });
    byId<HTMLButtonElement>("em-reset").addEventListener("click", () => viewer?.resetView());
    renderInteractions(payload, viewer);
    setStatus("拖动旋转、滚轮缩放；点击热点定位部件，点击模型或分件编号可逐件展开观察。");
  } catch (error) {
    setStatus(`3D 模型无法显示：${error instanceof Error ? error.message : String(error)}`, true);
  }
}

void main();
