/**
 * T18 e2e：GLB 阅读器与资源恢复（PRD 修订 2 / ui_revision 2；REQ-032、REQ-036/039 的读取侧；
 * AC-050、AC-051、AC-057 的阅读侧、AC-060 前端侧；UI-043/UI-044/UI-045/UI-059）。
 *
 * 环境：真实 Chrome（Playwright chromium）+ 真实 Rust 后端（globalSetup 启动：临时
 * data-dir + `init` + 假凭据与保留端口 base_url，绝不外呼）+ Vite 前端；
 * **不依赖任何已运行的外部服务**。
 *
 * 数据来源（见 `viewer-harness.ts` 的说明）：
 * - 会话/物品/document/preparation/页资产/原 PDF 字节全部是**真实后端**数据（全部用例）；
 * - **真实链路用例（9）**：草稿 DTO 与模型字节也来自真实后端（测试构建 + 本机 fixture
 *   供应商走真实流水线；`GET /items/{id}/drafts/{draftId}` 与 `GET /assets/{id}/content`
 *   服务端已可用——QA 回合 22 实测，此前"必须拦截"的表述已订正）；
 * - 其余用例的草稿 JSON 与 GLB 字节按真实合同形态在浏览器侧提供（拦截**仅**用于
 *   合成 DTO 的注入场景：stale 热点、500 故障注入）。
 * 断言的可观察证据来自只读桥 `window.__EM_VIEWER__`（帧数/位姿/锚点/资源账本）。
 *
 * 覆盖清单（命令 ↔ AC/UI）：
 * 1. 3D 懒加载：资料库首屏不下载 three，进入阅读页按需加载并渲染（AC-050/REQ-040）；
 * 2. OrbitControls 旋转/缩放/复位/适配 + 键盘等价控件（AC-050/UI-043）；
 * 3. WebGL 上下文丢失 → 可见提示 → 自动恢复/手动重建后渲染继续、状态与控件回到
 *    可用态、位姿保留（AC-050/UI-044；BUG-007 的回归断言）；
 * 4. 卸载/换模型释放资源；换模型不串旧资源/旧热点（AC-050/AC-051）；
 * 5. 连续切换 10 次模型：资源数不增长（AC-062/REQ-040 的资源趋势）；
 * 6. 3D 失败（模型内容不可用）时文字与 PDF 仍可读（AC-050/UI-045）；
 * 7. WebGL 不可用时给出可读原因 + 文字/PDF 可用（UI-044/UI-045）；
 * 8. 同一局部点在不同旋转/缩放下的世界位置一致、锚点不漂移（AC-051 浏览器侧）；
 * 9. **真实链路**：真实草稿 + 真实资产字节（零拦截零伪造）驱动阅读器（AC-050/REQ-032）。
 */

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

import {
  expect,
  request as playwrightRequest,
  test,
  type APIRequestContext,
  type Page,
} from "@playwright/test";

import { apiLogin, captureTo, loginViaUi, seedItemWithDocument, seedReadyPreparation } from "./helpers";
import {
  BACKEND_PASSWORD,
  LocalFixture,
  TestBackend,
  buildTestServerBinary,
  seedJob,
  type SeededJob,
} from "./job-recovery-harness";
import { E2E_WEB_PORT, REPO_ROOT, fixturePath } from "./runtime";
import {
  blockExternalRequests,
  canvasHasPixels,
  draftPayload,
  fixtureModel,
  heapBytes,
  installRealBackendRouting,
  installViewerRoutes,
  loseWebglContext,
  modelInfo,
  recordModuleRequests,
  restoreWebglContext,
  roundTrip,
  spaNavigate,
  viewerAnchors,
  viewerFrames,
  viewerPose,
  viewerStats,
  apiBase,
} from "./viewer-harness";
import { runtime } from "./helpers";

const WEB_BASE = `http://127.0.0.1:${E2E_WEB_PORT}`;

const password = (): string => runtime().password;

/**
 * 视口固定在 1440×900（不用默认的 1280×720）。
 *
 * 原因（2026-09-12 BUG-007 修复回合实测，属 T08/T16 布局壳的边界现象，非 T18 代码）：
 * 默认视口宽度**正好等于** `wide ≥1280px` 断点；而 `fullPage` 截图等操作会瞬时改变
 * 滚动条/视口状态，使 `(min-width: 1280px)` 在 wide/mid 之间抖动——`PageLayout` 的
 * wide 与 mid 是两棵不同的树，抖动会导致**整页（含 3D Canvas 与原文面板）反复重挂载**，
 * 落在测量窗口内时表现为 `cameraPose()` 短暂为 null（用例 2 曾以 NaN 形式偶发失败）。
 * 1440 宽度下即使出现滚动条（1440−15 ≥ 1280）也不会跨断点，用例因此确定性可重跑。
 * 布局边界本身的抖动已作为发现项记录（见 implementation.md §T18-13），供 T21 处理。
 */
test.use({ viewport: { width: 1440, height: 900 } });

interface Scenario {
  readonly itemId: string;
  readonly documentId: string;
  readonly preparationId: string;
  readonly modelA: ReturnType<typeof fixtureModel>;
  readonly modelB: ReturnType<typeof fixtureModel>;
  readonly drafts: Record<string, Record<string, unknown>>;
  readonly draftIdA: string;
  readonly draftIdB: string;
}

/** 造一份真实数据 + 两份"模型 A/B"草稿（A 含一个 stale 热点，验证不会显示）。 */
async function seedScenario(page: Page, request: APIRequestContext, tag: string): Promise<Scenario> {
  await apiLogin(request, apiBase(), password());
  const seed = await seedItemWithDocument(
    request,
    apiBase(),
    password(),
    "sample-manual-text.pdf",
    `T18 ${tag}`,
  );
  const csrf = await apiLogin(request, apiBase(), password());
  const preparationId = await seedReadyPreparation(request, apiBase(), csrf, seed);

  const modelA = fixtureModel("viewer-asymmetric", "e2e-model-a");
  const modelB = fixtureModel("viewer-asymmetric-b", "e2e-model-b");
  const pointA0 = modelA.localPoints[0] as readonly [number, number, number];
  const pointA1 = modelA.localPoints[1] as readonly [number, number, number];
  const pointB0 = modelB.localPoints[0] as readonly [number, number, number];

  const draftIdA = "draft-a";
  const draftIdB = "draft-b";
  const drafts: Record<string, Record<string, unknown>> = {
    [draftIdA]: draftPayload({
      itemId: seed.itemId,
      documentId: seed.documentId,
      preparationId,
      model: modelA,
      modelRevision: 1,
      partNames: ["T18 部件-机身", "T18 部件-镜头"],
      stepTitles: ["T18 步骤-安装镜头"],
      hotspots: [
        { id: "hotspot-a-0", partId: "part-revision-viewer-asymmetric-0", status: "confirmed", positionLocal: pointA0 },
        { id: "hotspot-a-1", partId: "part-revision-viewer-asymmetric-1", status: "candidate", positionLocal: pointA1 },
        {
          // stale：锚点属于旧模型版本，必须**不**作为有效热点显示（AC-051/REQ-033）。
          id: "hotspot-a-stale",
          partId: "part-revision-viewer-asymmetric-0",
          status: "confirmed",
          positionLocal: pointA0,
        },
      ],
    }),
    [draftIdB]: draftPayload({
      itemId: seed.itemId,
      documentId: seed.documentId,
      preparationId,
      model: modelB,
      modelRevision: 2,
      partNames: ["T18 部件B-外壳"],
      stepTitles: ["T18 步骤B-更换底座"],
      hotspots: [
        { id: "hotspot-b-0", partId: "part-revision-viewer-asymmetric-b-0", status: "confirmed", positionLocal: pointB0 },
      ],
    }),
  };
  // stale 热点的锚点改成"另一个 revision + 另一个哈希"（读取器据此判 stale）。
  const draftA = drafts[draftIdA] as Record<string, unknown>;
  const knowledgeA = draftA.knowledge as Record<string, unknown>;
  const hotspotsA = knowledgeA.hotspots as Record<string, unknown>[];
  const staleAnchor = hotspotsA[2]?.anchor as Record<string, unknown>;
  staleAnchor.modelRevisionId = "revision-old";
  staleAnchor.modelSha256 = "0".repeat(64);

  await installViewerRoutes(page, { drafts, models: [modelA, modelB] });
  return { itemId: seed.itemId, documentId: seed.documentId, preparationId, modelA, modelB, drafts, draftIdA, draftIdB };
}

/** 需要注入失败（模型内容 500）时重新安装拦截。 */
async function installRoutesFor(
  page: Page,
  scenario: Scenario,
  options: { failModelContent?: boolean } = {},
): Promise<void> {
  await page.unrouteAll({ behavior: "wait" });
  await installViewerRoutes(page, {
    drafts: scenario.drafts,
    models: [scenario.modelA, scenario.modelB],
    ...(options.failModelContent === true ? { failModelContent: true } : {}),
  });
}

const reviewPath = (scenario: Scenario, draftId: string): string =>
  `/items/${scenario.itemId}/drafts/${draftId}/review`;

/** 等待 3D 就绪（模型加载 + 至少渲染过一帧）。 */
async function waitForViewer(page: Page): Promise<void> {
  await expect(page.getByTestId("viewer-canvas")).toBeVisible();
  await expect
    .poll(async () => (await viewerStats(page)).modelsAlive, { timeout: 30_000 })
    .toBe(1);
  await expect.poll(async () => await viewerFrames(page), { timeout: 30_000 }).toBeGreaterThan(0);
  await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
}

/** 读取数组分量（`noUncheckedIndexedAccess` 下的显式默认值，避免 NaN 静默传播）。 */
const at = (values: readonly number[] | undefined, index: number): number =>
  values?.[index] ?? Number.NaN;

/** 两个三维点的距离（缺失分量为 NaN → 断言会失败，不会被当作 0 掩盖）。 */
function distance3(a: readonly number[] | undefined, b: readonly number[] | undefined): number {
  return Math.hypot(at(a, 0) - at(b, 0), at(a, 1) - at(b, 1), at(a, 2) - at(b, 2));
}

function distanceToTarget(page: Page): Promise<number> {
  return viewerPose(page).then((pose) =>
    pose === null ? Number.NaN : distance3(pose.positionLocal, pose.targetLocal),
  );
}

/**
 * 等到位姿可读再取距离（返回有限数值）。
 *
 * 为什么需要：`cameraPose()` 在"模型/相机尚未就绪"的窗口里返回 null；而 NaN 会
 * **骗过** `not.toBeCloseTo` 这类否定式断言（NaN 与任何值都不接近），造成假通过、
 * 随后以难懂的 NaN 失败（用例 2 的偶发失败现场）。这里显式等待，并给出可读原因。
 */
async function finiteDistance(page: Page, timeoutMs = 15_000): Promise<number> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await distanceToTarget(page);
    if (Number.isFinite(value)) {
      return value;
    }
    if (Date.now() > deadline) {
      throw new Error("3D 位姿在超时内不可用（模型或相机未就绪）");
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}

test.describe("GLB 阅读器与资源恢复（T18）", () => {
  test("3D 懒加载：资料库首屏不下载 three，进入阅读页按需加载并渲染（AC-050/REQ-040）", async ({
    page,
    request,
  }) => {
    const external = await blockExternalRequests(page);
    const modules = recordModuleRequests(page);
    const scenario = await seedScenario(page, request, "懒加载");
    await loginViaUi(page, "", password());

    // --- 首屏（资料库）：不加载 three/R3F/阅读器模块 -----------------------------
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();
    expect(modules, `首屏不该请求 3D 模块：${modules.join(", ")}`).toEqual([]);

    // --- 进入阅读页：3D 模块按需加载 ---------------------------------------------
    await page.goto(reviewPath(scenario, scenario.draftIdA));
    await expect(page.getByRole("heading", { name: "阅读与复核" })).toBeVisible();
    await waitForViewer(page);
    expect(modules.length, "进入阅读页后应下载 3D 模块").toBeGreaterThan(0);
    expect(modules.some((url) => /three/.test(url))).toBe(true);

    const info = await modelInfo(page);
    expect(info?.assetId).toBe(scenario.modelA.assetId);
    expect(info?.sha256).toBe(scenario.modelA.sha256);
    // fixture 是 12 三角面的不对称六面体（含 1 张内嵌贴图）。
    expect(info?.triangles).toBe(12);
    expect(info?.textures).toBe(1);

    // 3D 成功时的固定提示（UI-043）。
    await expect(page.getByText("模型为外观资产，不表示机械结构", { exact: false })).toBeVisible();
    await captureTo("t18-rd", page, "01-viewer-loaded");
    expect(external).toEqual([]);
  });

  test("OrbitControls：旋转/缩放/复位/适配与键盘等价控件（AC-050/UI-043）", async ({
    page,
    request,
  }) => {
    const scenario = await seedScenario(page, request, "操作");
    await loginViaUi(page, "", password());
    await page.goto(reviewPath(scenario, scenario.draftIdA));
    await waitForViewer(page);

    const initialPose = await viewerPose(page);
    expect(initialPose).not.toBeNull();
    const canvas = page.getByTestId("viewer-canvas");
    const box = await canvas.boundingBox();
    expect(box).not.toBeNull();
    const centerX = (box?.x ?? 0) + (box?.width ?? 0) / 2;
    const centerY = (box?.y ?? 0) + (box?.height ?? 0) / 2;

    // --- 旋转：拖动改变相机方位 ---------------------------------------------------
    await page.mouse.move(centerX, centerY);
    await page.mouse.down();
    await page.mouse.move(centerX + 120, centerY + 40, { steps: 12 });
    await page.mouse.up();
    const rotated = await viewerPose(page);
    expect(rotated).not.toBeNull();
    const rotateDelta = distance3(rotated?.positionLocal, initialPose?.positionLocal);
    expect(rotateDelta, "拖动后相机位姿必须变化").toBeGreaterThan(0.01);
    await captureTo("t18-rd", page, "02-viewer-rotated");

    // --- 缩放：滚轮改变相机到目标的距离 -------------------------------------------
    const distanceBefore = await finiteDistance(page);
    await page.mouse.move(centerX, centerY);
    await page.mouse.wheel(0, -480);
    await expect
      .poll(async () => await distanceToTarget(page), { timeout: 10_000 })
      .not.toBeCloseTo(distanceBefore, 2);
    const distanceAfter = await finiteDistance(page);
    expect(Math.abs(distanceAfter - distanceBefore)).toBeGreaterThan(0.05);

    // --- 复位（键盘等价控件）：回到初始取景 ---------------------------------------
    const resetButton = page.getByRole("button", { name: "复位视角" });
    await expect(resetButton).toBeEnabled();
    await resetButton.focus();
    await page.keyboard.press("Enter");
    await expect
      .poll(async () => {
        const pose = await viewerPose(page);
        return pose === null ? Number.NaN : distance3(pose.positionLocal, initialPose?.positionLocal);
      })
      .toBeLessThan(1e-3);

    // --- 适配：先转开再点「适配模型」，模型重新进入视野（距离回到可看全的量级） ----
    await page.mouse.move(centerX, centerY);
    await page.mouse.down();
    await page.mouse.move(centerX - 140, centerY - 30, { steps: 12 });
    await page.mouse.up();
    await page.getByRole("button", { name: "适配模型" }).click();
    const fitted = await distanceToTarget(page);
    const initialDistance = distance3(initialPose?.positionLocal, initialPose?.targetLocal);
    expect(fitted).toBeGreaterThan(0.5);
    expect(Math.abs(fitted - initialDistance)).toBeLessThan(0.05);
    // 目标点仍是"模型中心"：targetLocal 必须等于局部包围盒中心（显示居中把该点
    // 映射到世界原点；注意模型自身的局部中心不一定是原点）。
    const fittedPose = await viewerPose(page);
    const bounds = (await modelInfo(page))?.bounds;
    expect(bounds).toBeDefined();
    for (let axis = 0; axis < 3; axis += 1) {
      const center = (at(bounds?.min, axis) + at(bounds?.max, axis)) / 2;
      expect(at(fittedPose?.targetLocal, axis)).toBeCloseTo(center, 3);
    }
    await captureTo("t18-rd", page, "03-viewer-fit");
  });

  test("WebGL 上下文丢失 → 提示 → 恢复后继续渲染并保留相机（AC-050/UI-044）", async ({
    page,
    request,
  }) => {
    test.setTimeout(180_000);
    const scenario = await seedScenario(page, request, "上下文");
    await loginViaUi(page, "", password());
    await page.goto(reviewPath(scenario, scenario.draftIdA));
    await waitForViewer(page);
    // 初始取景（用于断言重建后的「复位视角」仍回到同一取景）。
    const poseAtLoad = await viewerPose(page);

    // 先转一下相机，验证恢复后位姿保留（UI-044）。
    const canvas = page.getByTestId("viewer-canvas");
    const box = await canvas.boundingBox();
    await page.mouse.move((box?.x ?? 0) + (box?.width ?? 0) / 2, (box?.y ?? 0) + (box?.height ?? 0) / 2);
    await page.mouse.down();
    await page.mouse.move((box?.x ?? 0) + (box?.width ?? 0) / 2 + 90, (box?.y ?? 0) + 10, { steps: 8 });
    await page.mouse.up();
    const poseBeforeLoss = await viewerPose(page);
    const framesBeforeLoss = await viewerFrames(page);

    // --- 丢失：可见提示 + 交互禁用（不是只能刷新页面） -----------------------------
    await loseWebglContext(page);
    await expect(page.getByTestId("viewer-status")).toContainText("3D 显示已中断，正在重建…");
    await expect(page.getByRole("button", { name: "复位视角" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "适配模型" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "立即重建" })).toBeVisible();

    // --- 恢复：渲染继续（帧数增长），相机位姿保留 ---------------------------------
    await restoreWebglContext(page);
    await expect
      .poll(async () => await viewerFrames(page), { timeout: 30_000 })
      .toBeGreaterThan(framesBeforeLoss);
    await expect(page.getByTestId("viewer-status")).toContainText("3D 显示已恢复。");
    await expect(page.getByRole("button", { name: "复位视角" })).toBeEnabled();
    const poseAfterRestore = await viewerPose(page);
    for (let axis = 0; axis < 3; axis += 1) {
      expect(at(poseAfterRestore?.positionLocal, axis)).toBeCloseTo(
        at(poseBeforeLoss?.positionLocal, axis),
        5,
      );
    }
    await captureTo("t18-rd", page, "04-viewer-restored");

    // --- 手动重建：再次丢失后用「立即重建」换新上下文并继续渲染 -------------------
    await loseWebglContext(page);
    const framesAtLoss = await viewerFrames(page);
    const poseBeforeRebuild = await viewerPose(page);
    await page.getByRole("button", { name: "立即重建" }).click();
    await expect
      .poll(async () => await viewerFrames(page), { timeout: 30_000 })
      .toBeGreaterThan(framesAtLoss);
    await expect.poll(async () => (await viewerStats(page)).modelsAlive, { timeout: 30_000 }).toBe(1);
    await expect(page.getByTestId("viewer-canvas")).toBeVisible();

    // BUG-007：重建成功后必须回到可用态——状态文案、键盘等价控件、重建入口三处都要
    // 恢复（只查帧数/模型存活会漏掉这个缺陷；QA 回合 22 由此立案）。
    await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
    await expect(page.getByRole("button", { name: "复位视角" })).toBeEnabled();
    await expect(page.getByRole("button", { name: "适配模型" })).toBeEnabled();
    await expect(page.getByRole("button", { name: "立即重建" })).toHaveCount(0);

    // 相机位姿与自动 restored 分支一致地保留（重建前捕获的 asset-root 局部位姿由新
    // 舞台套用；UI-044「能重建时保留当前相机」）。
    const poseAfterRebuild = await viewerPose(page);
    for (let axis = 0; axis < 3; axis += 1) {
      expect(
        at(poseAfterRebuild?.positionLocal, axis),
        `重建后相机位置应保留（轴 ${axis}）`,
      ).toBeCloseTo(at(poseBeforeRebuild?.positionLocal, axis), 3);
      expect(
        at(poseAfterRebuild?.targetLocal, axis),
        `重建后观察目标应保留（轴 ${axis}）`,
      ).toBeCloseTo(at(poseBeforeRebuild?.targetLocal, axis), 3);
    }

    // 8 秒后仍不得误报「浏览器 3D 上下文不可用」（丢失时的 8s 计时必须在重建成功
    // 后清除；BUG-007 的第二个症状）。
    await page.waitForTimeout(8_500);
    await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
    await expect(page.getByTestId("viewer-status")).not.toContainText("不可用");
    await expect(page.getByRole("button", { name: "复位视角" })).toBeEnabled();

    // 证据留档（BUG-007 复验的人工可读现场：状态行 + 控件 + 8.5 秒后的渲染事实）。
    const evidenceDir = path.join(REPO_ROOT, "artifacts", "web-mvp", "t18-rd-fix");
    fs.mkdirSync(evidenceDir, { recursive: true });
    fs.writeFileSync(
      path.join(evidenceDir, "after-fix-rebuild-state.json"),
      JSON.stringify(
        {
          说明:
            "BUG-007 修复后：手动「立即重建」→ 等待 8.5 秒的现场（RD viewer.spec.ts 用例 3；修复前为 statusText=3D 显示已中断，正在重建…、resetButtonDisabled=true、rebuildButtonVisible=true）",
          statusText: (await page.getByTestId("viewer-status").textContent()) ?? "",
          resetButtonDisabled: await page.getByRole("button", { name: "复位视角" }).isDisabled(),
          fitButtonDisabled: await page.getByRole("button", { name: "适配模型" }).isDisabled(),
          rebuildButtonCount: await page.getByRole("button", { name: "立即重建" }).count(),
          modelsAlive: (await viewerStats(page)).modelsAlive,
          frames: await viewerFrames(page),
        },
        null,
        2,
      ),
    );

    // 恢复后交互照常：「适配模型」可用，「复位视角」回到初始取景。
    await page.getByRole("button", { name: "适配模型" }).click();
    await page.getByRole("button", { name: "复位视角" }).click();
    const poseAfterReset = await viewerPose(page);
    for (let axis = 0; axis < 3; axis += 1) {
      expect(
        at(poseAfterReset?.positionLocal, axis),
        `重建后「复位视角」应回到初始取景（轴 ${axis}）`,
      ).toBeCloseTo(at(poseAtLoad?.positionLocal, axis), 3);
    }
    await expect.poll(async () => await viewerFrames(page), { timeout: 15_000 }).toBeGreaterThan(0);
    await captureTo("t18-rd", page, "04b-viewer-manual-rebuild-recovered");
  });

  test("卸载/换模型：资源被释放、旧资源与旧热点不串入（AC-050/AC-051）", async ({
    page,
    request,
  }) => {
    const scenario = await seedScenario(page, request, "换模型");
    await loginViaUi(page, "", password());
    await page.goto(reviewPath(scenario, scenario.draftIdA));
    await waitForViewer(page);

    const statsA = await viewerStats(page);
    expect(statsA.geometries.alive).toBe(1);
    expect(statsA.materials.alive).toBe(1);
    expect(statsA.textures.alive).toBe(1);
    const anchorsA = await viewerAnchors(page);
    expect(anchorsA.map((anchor) => anchor.id).sort()).toEqual(["hotspot-a-0", "hotspot-a-1"]);
    await expect(page.getByTestId("parts-list")).toContainText("T18 部件-机身");
    // stale 锚点不计入"有效热点"（面板按 part 显示"（1 个已失效）"）。
    await expect(
      page.getByTestId("part-hotspot-part-revision-viewer-asymmetric-0"),
    ).toContainText("已失效");

    // --- 换模型（同一页面内的客户端路由）：B 成为唯一存活资源，A 的部件/热点消失 ----
    await spaNavigate(page, reviewPath(scenario, scenario.draftIdB));
    await expect(page.getByTestId("parts-list")).toContainText("T18 部件B-外壳");
    await waitForViewer(page);
    const statsB = await viewerStats(page);
    expect(statsB.modelsAlive).toBe(1);
    expect(statsB.modelsDisposed).toBeGreaterThanOrEqual(1);
    expect(statsB.geometries.alive).toBe(1);
    expect(statsB.materials.alive).toBe(1);
    expect(statsB.textures.alive).toBe(1);
    // A 的资源确实被释放过（created 总数 > 存活数）——不是"从未加载 A"。
    expect(statsB.geometries.created).toBeGreaterThan(statsB.geometries.alive);
    const anchorsB = await viewerAnchors(page);
    expect(anchorsB.map((anchor) => anchor.id)).toEqual(["hotspot-b-0"]);
    await expect(page.getByTestId("parts-list")).not.toContainText("T18 部件-机身");
    await expect(page.getByTestId("steps-list")).toContainText("T18 步骤B-更换底座");
    await expect(page.getByTestId("steps-list")).not.toContainText("T18 步骤-安装镜头");
    const infoB = await modelInfo(page);
    expect(infoB?.assetId).toBe(scenario.modelB.assetId);
    await captureTo("t18-rd", page, "05-viewer-model-b");

    // --- 卸载（SPA 内离开阅读页）：模型资源全部释放 --------------------------------
    await spaNavigate(page, "/");
    await expect(page.getByRole("heading", { name: "资料库", exact: true })).toBeVisible();
    await expect
      .poll(async () => (await viewerStats(page)).geometries.alive, { timeout: 10_000 })
      .toBe(0);
    const statsAfterUnmount = await viewerStats(page);
    expect(statsAfterUnmount.geometries.disposed).toBe(statsAfterUnmount.geometries.created);
    expect(statsAfterUnmount.materials.disposed).toBe(statsAfterUnmount.materials.created);
    expect(statsAfterUnmount.textures.disposed).toBe(statsAfterUnmount.textures.created);
    expect(statsAfterUnmount.modelsAlive).toBe(0);
  });

  test("连续切换 10 次模型：资源数不增长（AC-062/REQ-040 的资源趋势）", async ({
    page,
    request,
  }) => {
    test.setTimeout(180_000);
    const scenario = await seedScenario(page, request, "切换趋势");
    await loginViaUi(page, "", password());
    await page.goto(reviewPath(scenario, scenario.draftIdA));
    await waitForViewer(page);

    const heapSamples: (number | null)[] = [];
    const aliveSamples: string[] = [];
    for (let round = 0; round < 10; round += 1) {
      const draftId = round % 2 === 0 ? scenario.draftIdB : scenario.draftIdA;
      await spaNavigate(page, reviewPath(scenario, draftId));
      await waitForViewer(page);
      const stats = await viewerStats(page);
      // 每一次切换后：只有一份模型的资源存活（模型不串、也不堆积）。
      expect(stats.modelsAlive, `第 ${round + 1} 次切换后 modelsAlive`).toBe(1);
      expect(stats.geometries.alive).toBe(1);
      expect(stats.materials.alive).toBe(1);
      expect(stats.textures.alive).toBe(1);
      expect(stats.modelsLoaded - stats.modelsDisposed).toBe(1);
      aliveSamples.push(
        `${stats.geometries.alive}/${stats.materials.alive}/${stats.textures.alive}`,
      );
      heapSamples.push(await heapBytes(page));
    }
    // 存活资源数在 10 次切换中保持恒定（硬断言）；堆用量仅作补充证据打印。
    expect(new Set(aliveSamples).size).toBe(1);
    console.log(`T18 资源趋势：存活(geometry/material/texture)=${aliveSamples.join(" ")}`);
    console.log(`T18 堆用量采样（字节，Chrome 专有）：${heapSamples.join(", ")}`);
    const firstHeap = heapSamples[0];
    const lastHeap = heapSamples[heapSamples.length - 1];
    if (firstHeap !== null && lastHeap !== null && firstHeap !== undefined && lastHeap !== undefined) {
      // 明确标注为"趋势观察"而不是硬指标：Chrome 的堆用量受 GC 时机影响。
      console.log(
        `T18 堆用量首末比：${(lastHeap / firstHeap).toFixed(2)}（1.0 附近为正常；不设阈值）`,
      );
    }
  });

  test("3D 失败时文字与 PDF 仍可读（AC-050/UI-045）", async ({ page, request }) => {
    const scenario = await seedScenario(page, request, "失败降级");
    await loginViaUi(page, "", password());
    await installRoutesFor(page, scenario, { failModelContent: true });
    await page.goto(reviewPath(scenario, scenario.draftIdA));

    // 3D 区域给出可读原因与重试入口。
    await expect(page.getByTestId("viewer-error")).toBeVisible();
    await expect(page.getByTestId("viewer-error")).toContainText("模型加载失败");
    await expect(page.getByRole("button", { name: "重试加载" })).toBeVisible();

    // 文字与 PDF 不受影响：部件/步骤可读，原文页真实渲染出来。
    await expect(page.getByTestId("parts-list")).toContainText("T18 部件-机身");
    await expect(page.getByTestId("steps-list")).toContainText("T18 步骤-安装镜头");
    await expect(page.getByTestId("original-page-label")).toContainText("第 1 /");
    await expect.poll(async () => await canvasHasPixels(page, '[data-testid="original-canvas"]'), {
      timeout: 20_000,
    }).toBe(true);
    await captureTo("t18-rd", page, "06-viewer-failed-text-pdf-ok");

    // 「改用文字阅读」把焦点交给文字面板（键盘路径；UI-059）。
    await page.getByRole("button", { name: "改用文字阅读" }).click();
    await expect
      .poll(() => page.evaluate(() => document.activeElement?.getAttribute("data-testid") ?? ""))
      .toBe("parts-panel");
  });

  test("WebGL 不可用：给出可读原因，文字与 PDF 仍可用（UI-044/UI-045）", async ({
    page,
    request,
  }) => {
    const scenario = await seedScenario(page, request, "无 WebGL");
    // 让浏览器拒绝一切 WebGL 上下文（真实降级路径：探测失败 → 不挂载 Canvas）。
    await page.addInitScript(() => {
      const original = HTMLCanvasElement.prototype.getContext;
      HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, type: string) {
        if (type === "webgl" || type === "webgl2" || type === "experimental-webgl") {
          return null;
        }
        return original.call(this, type as never) as never;
      } as typeof HTMLCanvasElement.prototype.getContext;
    });
    await loginViaUi(page, "", password());
    await page.goto(reviewPath(scenario, scenario.draftIdA));

    await expect(page.getByTestId("viewer-unavailable")).toBeVisible();
    await expect(page.getByTestId("viewer-unavailable")).toContainText("浏览器 3D 上下文不可用");
    await expect(page.getByTestId("viewer-canvas")).toHaveCount(0);
    // 降级路径下文字与原文照常。
    await expect(page.getByTestId("parts-list")).toContainText("T18 部件-机身");
    await expect.poll(async () => await canvasHasPixels(page, '[data-testid="original-canvas"]'), {
      timeout: 20_000,
    }).toBe(true);
    await captureTo("t18-rd", page, "07-viewer-webgl-unavailable");
  });

  test("AC-051 浏览器侧：同一局部点在不同旋转/缩放下世界位置一致、锚点不漂移", async ({
    page,
    request,
  }) => {
    const scenario = await seedScenario(page, request, "坐标一致");
    await loginViaUi(page, "", password());
    await page.goto(reviewPath(scenario, scenario.draftIdA));
    await waitForViewer(page);

    // 1) 局部↔世界往返：fixture 给出的 3 个 asset-root 局部采样点全部一致。
    for (const point of scenario.modelA.localPoints) {
      const result = await roundTrip(page, point);
      expect(result, "桥的 roundTrip 必须可用").not.toBeNull();
      expect(result?.error ?? 1).toBeLessThan(1e-6);
      expect(result?.back[0]).toBeCloseTo(point[0], 6);
      expect(result?.back[1]).toBeCloseTo(point[1], 6);
      expect(result?.back[2]).toBeCloseTo(point[2], 6);
    }

    const anchorsBefore = await viewerAnchors(page);
    expect(anchorsBefore).toHaveLength(2);
    // 锚点必须落在"显示归一化后"的模型范围内（半径 ≤ 1 的显示空间）。
    for (const anchor of anchorsBefore) {
      expect(Math.hypot(at(anchor.world, 0), at(anchor.world, 1), at(anchor.world, 2))).toBeLessThanOrEqual(1.001);
    }
    expect(distance3(anchorsBefore[0]?.world, anchorsBefore[1]?.world)).toBeGreaterThan(0.05);

    // 2) 相机旋转 + 缩放（显示变换不变）后：锚点的局部坐标与**世界坐标**都不变。
    const canvas = page.getByTestId("viewer-canvas");
    const box = await canvas.boundingBox();
    const cx = (box?.x ?? 0) + (box?.width ?? 0) / 2;
    const cy = (box?.y ?? 0) + (box?.height ?? 0) / 2;
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 150, cy + 60, { steps: 12 });
    await page.mouse.up();
    await page.mouse.wheel(0, -400);
    await expect
      .poll(async () => await viewerFrames(page))
      .toBeGreaterThan(0);

    const anchorsAfter = await viewerAnchors(page);
    expect(anchorsAfter).toHaveLength(2);
    for (const anchor of anchorsAfter) {
      const before = anchorsBefore.find((candidate) => candidate.id === anchor.id);
      expect(before, `换相机后仍应有热点 ${anchor.id}`).toBeDefined();
      expect(anchor.local).toEqual(before?.local);
      for (let axis = 0; axis < 3; axis += 1) {
        expect(at(anchor.world, axis)).toBeCloseTo(at(before?.world, axis), 6);
      }
    }
  });
});

// ---------------------------------------------------------------------------
// 真实链路（不拦截草稿/模型字节）：测试构建后端 + 本机 fixture 供应商
//
// 为什么单独一个 describe：造出一份带 validated 模型的草稿需要**测试构建**
// （`--features job-failpoints` 放行明文 http + 回环的模型下载）与显式测试配置
// （provider base_url 指向本机 fixture），因此这里自管后端与 fixture；该设施与
// T17 的 `job-recovery.spec.ts` 同源（复用 `job-recovery-harness`）。
// ---------------------------------------------------------------------------

interface RealDraftRef {
  readonly itemId: string;
  readonly draftId: string;
  readonly assetId: string;
  readonly sha256: string;
  readonly revisionId: string;
}

/** 等真实流水线成功并读出**真实草稿**里的模型引用（HTTP 合同，不碰数据库）。 */
async function readRealDraft(
  context: APIRequestContext,
  backend: TestBackend,
  seeded: SeededJob,
  tag: string,
): Promise<RealDraftRef> {
  const deadline = Date.now() + 180_000;
  let last: { status: string; draftId: string | null } | null = null;
  while (Date.now() < deadline) {
    const response = await context.get(`${backend.base}/api/v1/jobs/${seeded.jobId}`);
    expect(response.status(), await response.text()).toBe(200);
    const body = (await response.json()) as { data: { status: string; draftId: string | null } };
    last = body.data;
    if (last.status === "succeeded") {
      break;
    }
    if (last.status === "failed" || last.status === "cancelled") {
      throw new Error(`${tag}：真实流水线任务终态为 ${last.status}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  expect(last?.status, `${tag}：真实流水线必须成功（最后状态 ${last?.status ?? "未知"}）`).toBe(
    "succeeded",
  );
  expect(last?.draftId, `${tag}：成功任务必须产出草稿`).toBeTruthy();
  const detail = await context.get(
    `${backend.base}/api/v1/items/${seeded.itemId}/drafts/${last?.draftId ?? ""}`,
  );
  expect(detail.status(), await detail.text()).toBe(200);
  const body = (await detail.json()) as {
    data: { knowledge: { model?: Record<string, unknown> } };
  };
  const model = body.data.knowledge.model ?? {};
  expect(model.validationState, `${tag}：草稿模型必须是 validated`).toBe("validated");
  return {
    itemId: seeded.itemId,
    draftId: String(last?.draftId ?? ""),
    assetId: String(model.assetId),
    sha256: String(model.sha256),
    revisionId: String(model.revisionId),
  };
}

test.describe("真实链路：真实草稿 + 真实资产字节（不拦截草稿/模型字节）", () => {
  test.describe.configure({ timeout: 300_000 });
  let fixture: LocalFixture;
  let backend: TestBackend;
  let real: RealDraftRef;
  let sampleSha256: string;

  test.beforeAll(async () => {
    test.setTimeout(900_000);
    buildTestServerBinary();
    fixture = new LocalFixture();
    fixture.state.manualMode = "success";
    fixture.state.submitMode = "success";
    fixture.state.modelMode = "valid";
    fixture.state.manualDelayMs = 0;
    await fixture.start();
    backend = new TestBackend("t18-rd-real");
    await backend.start(fixture);
    const context = await playwrightRequest.newContext();
    const seeded = await seedJob(context, backend, "T18 真实链路");
    real = await readRealDraft(context, backend, seeded, "T18 真实链路");
    await context.dispose();
    sampleSha256 = createHash("sha256")
      .update(fs.readFileSync(fixturePath("sample-model.glb")))
      .digest("hex");
  });

  test.afterAll(async () => {
    await backend?.cleanup(fixture);
  });

  test("真实草稿与模型字节驱动阅读器：加载/操作/文字路径（AC-050/REQ-032；零拦截）", async ({
    page,
  }) => {
    const routing = await installRealBackendRouting(page, backend.base);
    await loginViaUi(page, WEB_BASE, BACKEND_PASSWORD);
    await page.goto(`/items/${real.itemId}/drafts/${real.draftId}/review`);
    await expect(page.getByRole("heading", { name: "阅读与复核" })).toBeVisible();
    await waitForViewer(page);

    // 模型事实与草稿一致：assetId/revisionId 来自真实后端，字节哈希等于仓库 fixture GLB
    // 的哈希（该 GLB 由真实流水线下：fixture 供应商 CDN → 下载 → model_validate → 落库）。
    const info = await modelInfo(page);
    expect(info?.assetId).toBe(real.assetId);
    expect(info?.revisionId).toBe(real.revisionId);
    expect(info?.sha256).toBe(real.sha256);
    expect(info?.sha256, "草稿哈希应等于 fixture GLB 的真实哈希").toBe(sampleSha256);
    expect(info?.triangles, "fixture 模型 12 三角面").toBe(12);
    expect(info?.textures).toBe(1);

    // 真实知识（fixture 说明书 AI 输出经真实合并后落库）与真实草稿身份。
    await expect(page.getByTestId("parts-list")).toContainText("后盖");
    await expect(page.getByTestId("steps-list")).toContainText("取下后盖");
    await expect(page.getByTestId("reader-context")).toContainText(`草稿 ${real.draftId}`);

    // 原文（真实 PDF 字节 + 本地 PDF.js）真的画出了像素——文字路径不依赖 3D 数据来源。
    await expect(page.getByTestId("original-page-label")).toContainText("第 1 /");
    await expect
      .poll(async () => await canvasHasPixels(page, '[data-testid="original-canvas"]'), {
        timeout: 30_000,
      })
      .toBe(true);

    // 真实模型字节确实请求了真实端点（`/assets/{id}/content`），且**没有任何伪造**。
    expect(
      routing.assetRequests.some((path) => path.includes(real.assetId)),
      `模型字节必须请求真实资产端点（实测：${routing.assetRequests.join(", ")}）`,
    ).toBe(true);
    expect(routing.fulfilled, "真实链路用例不得使用 route.fulfill").toBe(0);
    expect(routing.external, "全程零真实外网").toEqual([]);
    expect(fixture.counts.cdn, "模型字节应由本机 fixture 供应商的 CDN 分支产出").toBeGreaterThan(0);

    // 三轴操作与键盘等价控件在真实数据上照常（AC-050）。
    const initialPose = await viewerPose(page);
    const canvas = page.getByTestId("viewer-canvas");
    const box = await canvas.boundingBox();
    const cx = (box?.x ?? 0) + (box?.width ?? 0) / 2;
    const cy = (box?.y ?? 0) + (box?.height ?? 0) / 2;
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 120, cy + 50, { steps: 10 });
    await page.mouse.up();
    const rotated = await viewerPose(page);
    expect(distance3(rotated?.positionLocal, initialPose?.positionLocal)).toBeGreaterThan(0.01);
    await page.getByRole("button", { name: "复位视角" }).click();
    await expect
      .poll(async () => {
        const pose = await viewerPose(page);
        return pose === null ? Number.NaN : distance3(pose.positionLocal, initialPose?.positionLocal);
      })
      .toBeLessThan(1e-3);
    await expect.poll(async () => await viewerFrames(page), { timeout: 15_000 }).toBeGreaterThan(0);
    await captureTo("t18-rd", page, "09-viewer-real-draft-link");
  });
});
