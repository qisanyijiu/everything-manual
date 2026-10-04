/** Delivery regression: real Rust/API/PDF.js/Chrome, local providers only, no response mocks. */
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

import {
  expect,
  request as playwrightRequest,
  test,
  type APIRequestContext,
  type Page,
} from "@playwright/test";

import { apiLogin, loginViaUi } from "./helpers";
import {
  BACKEND_PASSWORD,
  LocalFixture,
  TestBackend,
  buildTestServerBinary,
  seedJob,
} from "./job-recovery-harness";
import { E2E_WEB_PORT, REPO_ROOT, fixturePath } from "./runtime";

const WEB = `http://127.0.0.1:${E2E_WEB_PORT}`;
const EVIDENCE = process.env.EM_E2E_EVIDENCE_DIR
  ?? path.join(REPO_ROOT, "var/delivery-20261004/fixture-chrome");
const CORRECTED_NAME = "已核对的后盖";
const CORRECTED_ACTION = "先确认电源已关闭，再松开固定件。";
const CORRECTED_SAFETY = "复核后的安全说明：操作前取出电池。";
const CORRECTED_SPEC = "复核后的供电规格";

interface DraftData {
  id: string;
  knowledge: {
    model: { revisionId: string; sha256: string; assetId: string };
    knowledge: {
      parts: { id: string; name: string }[];
      steps: { id: string; title: string }[];
      specs: { id: string; label: string }[];
    };
  };
}

let fixture: LocalFixture;
let backend: TestBackend;

test.describe.configure({ mode: "serial", timeout: 300_000 });

test.beforeAll(async () => {
  test.setTimeout(900_000);
  fs.mkdirSync(EVIDENCE, { recursive: true });
  if (!process.env.EM_E2E_SERVER_BINARY) buildTestServerBinary();
  fixture = new LocalFixture();
  await fixture.start();
  backend = new TestBackend("delivery-standalone", process.env.EM_E2E_SERVER_BINARY);
  await backend.start(fixture);
});

test.afterAll(async () => {
  await backend?.stop();
  if (backend !== undefined && fs.existsSync(backend.logPath)) {
    fs.copyFileSync(backend.logPath, path.join(EVIDENCE, "fixture-server.log"));
  }
  if (fixture !== undefined) {
    fs.writeFileSync(path.join(EVIDENCE, "fixture-requests.json"), JSON.stringify({
      counts: fixture.counts,
      paths: fixture.lines,
    }, null, 2));
  }
  await backend?.cleanup(fixture);
});

/** Rewrite only the local origin; API bodies and asset bytes always come from Rust. */
async function browserLogin(page: Page): Promise<string[]> {
  const external: string[] = [];
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      await route.continue();
    } else if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") {
      external.push(url.href);
      await route.abort();
    } else if (url.origin === WEB && url.pathname.startsWith("/api/v1")) {
      await route.continue({ url: `${backend.base}${url.pathname}${url.search}` });
    } else {
      await route.continue();
    }
  });
  await loginViaUi(page, WEB, BACKEND_PASSWORD);
  return external;
}

async function mutate(
  context: APIRequestContext,
  csrf: string,
  method: string,
  endpoint: string,
  data: unknown,
  etag?: string,
) {
  const response = await context.fetch(`${backend.base}/api/v1${endpoint}`, {
    method,
    headers: { "x-csrf-token": csrf, ...(etag === undefined ? {} : { "if-match": etag }) },
    data,
  });
  expect(response.ok(), `${method} ${endpoint}: ${response.status()} ${await response.text()}`).toBe(true);
  return response;
}

/** A self-authored one-page vector PDF: two separate product-sized figures, no remote assets. */
function figurePdf(): Buffer {
  const drawing = "0.2 g 55 140 190 230 re f 0.5 g 355 140 190 230 re f\n";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 500] /Resources << >> /Contents 4 0 R >>",
    `<< /Length ${Buffer.byteLength(drawing)} >>\nstream\n${drawing}endstream`,
  ];
  let pdf = "%PDF-1.4\n";
  const offsets = [0];
  for (const [index, object] of objects.entries()) {
    offsets.push(Buffer.byteLength(pdf));
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
  }
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  pdf += offsets.slice(1).map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("");
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(pdf);
}

test("PDF 候选图经真实提取、排列、自动保存与刷新后仍保留", async ({ page }) => {
  const api = await playwrightRequest.newContext();
  try {
    const csrf = await apiLogin(api, backend.base, BACKEND_PASSWORD);
    const itemResponse = await mutate(api, csrf, "POST", "/items", { name: "候选图回归物品", model: "delivery-vector" });
    const itemId = ((await itemResponse.json()) as { data: { id: string } }).data.id;
    const upload = await api.post(`${backend.base}/api/v1/items/${itemId}/assets`, {
      headers: { "x-csrf-token": csrf },
      multipart: { purpose: "document", file: { name: "vector-views.pdf", mimeType: "application/pdf", buffer: figurePdf() } },
    });
    expect(upload.status(), await upload.text()).toBe(201);
    const assetId = ((await upload.json()) as { data: { id: string } }).data.id;
    await mutate(api, csrf, "POST", `/items/${itemId}/documents`, { sourceAssetId: assetId, title: "向量视图说明书" });
    const external = await browserLogin(page);
    await page.goto(`${WEB}/items/${itemId}/import/views`);
    await expect(page.getByTestId("classify-view-candidates")).not.toBeChecked();
    const candidateRequests: { classify?: boolean }[] = [];
    page.on("request", (request) => {
      if (request.method() === "POST"
        && new URL(request.url()).pathname === `/api/v1/items/${itemId}/view-candidates`) {
        candidateRequests.push(request.postDataJSON() as { classify?: boolean });
      }
    });
    const manualCallsBefore = fixture.counts.manual;
    await page.getByTestId("extract-candidates").click();
    await expect(page.locator('[data-testid^="arrange-card-"]')).toHaveCount(2, { timeout: 90_000 });
    await expect(page.getByTestId("extract-candidates")).toBeEnabled();
    const candidatesResponse = await api.get(`${backend.base}/api/v1/items/${itemId}/view-candidates`);
    const candidates = ((await candidatesResponse.json()) as { data: { assetId: string; pageNumber: number; source: string }[] }).data;
    expect(candidates).toHaveLength(2);
    expect(candidates.every((candidate) => candidate.pageNumber === 1 && candidate.source === "region")).toBe(true);
    expect(candidateRequests).toHaveLength(2);
    expect(candidateRequests.every((request) => request.classify === false)).toBe(true);
    expect(fixture.counts.manual).toBe(manualCallsBefore);
    const [front, left] = candidates;
    expect(front).toBeDefined();
    expect(left).toBeDefined();
    await page.getByTestId(`arrange-card-${front?.assetId ?? ""}`).getByRole("combobox").selectOption("front");
    await page.getByTestId(`arrange-card-${left?.assetId ?? ""}`).getByRole("combobox").selectOption("left");
    await expect(page.getByTestId("arrangement-unsaved")).toHaveCount(0);
    const savedPhotos = async () => {
      const response = await api.get(`${backend.base}/api/v1/items/${itemId}/photos`);
      return ((await response.json()) as { data: { assetId: string; view: string }[] }).data.map((photo) => `${photo.view}:${photo.assetId}`).sort();
    };
    await expect.poll(savedPhotos).toEqual([`front:${front?.assetId ?? ""}`, `left:${left?.assetId ?? ""}`]);
    await page.reload();
    await expect(page.getByTestId("arrange-slot-front").locator("img")).toBeVisible();
    await expect(page.getByTestId("arrange-slot-left").locator("img")).toBeVisible();
    await expect(page.getByTestId(`arrange-card-${front?.assetId ?? ""}`).getByRole("combobox")).toHaveValue("front");
    // Swapping occupied slots is atomic and keeps both assignments after reload.
    await page.getByTestId(`arrange-card-${front?.assetId ?? ""}`).getByRole("combobox").selectOption("left");
    await expect.poll(savedPhotos).toEqual([`front:${left?.assetId ?? ""}`, `left:${front?.assetId ?? ""}`]);
    await expect(page.getByTestId("arrangement-unsaved")).toHaveCount(0);
    await page.reload();
    await expect(page.getByTestId(`arrange-card-${front?.assetId ?? ""}`).getByRole("combobox")).toHaveValue("left");
    await page.screenshot({ path: path.join(EVIDENCE, "candidate-arrangement.png"), fullPage: true });
    expect(external).toEqual([]);
  } finally {
    await api.dispose();
  }
});

test("完成提示、历史、发布与离线交互保留修订事实和安全说明", async ({ page, browser }, testInfo) => {
  const api = await playwrightRequest.newContext();
  fixture.state.manualDelayMs = 15_000;
  try {
    const seeded = await seedJob(api, backend, "离线交付回归物品");
    const external = await browserLogin(page);
    await page.goto(`${WEB}/items/${seeded.itemId}/generations`);
    await expect(page.getByTestId("history-entry")).toHaveCount(1);
    await expect(page.getByTestId("history-entry")).not.toContainText("已完成");
    await expect(page.getByText("「离线交付回归物品（T09-sample-manual-text.pdf）」生成完成：3D 模型与说明书草稿已就绪。", { exact: true })).toBeVisible({ timeout: 120_000 });
    const resultLink = page.getByRole("link", { name: "查看生成结果", exact: true });
    await expect(resultLink).toHaveAttribute("href", `/jobs/${seeded.jobId}/result`);
    await page.screenshot({ path: path.join(EVIDENCE, "generation-completion.png"), fullPage: true });
    await resultLink.click();
    await expect(page.getByTestId("viewer-status")).toContainText("模型已加载", { timeout: 60_000 });
    const detail = await api.get(`${backend.base}/api/v1/jobs/${seeded.jobId}`);
    const job = ((await detail.json()) as { data: { status: string; draftId: string } }).data;
    expect(job.status).toBe("succeeded");
    const draftEndpoint = `/items/${seeded.itemId}/drafts/${job.draftId}`;
    const draftResponse = await api.get(`${backend.base}/api/v1${draftEndpoint}`);
    const draft = ((await draftResponse.json()) as { data: DraftData }).data;
    const csrf = await apiLogin(api, backend.base, BACKEND_PASSWORD);
    const partsResponse = await api.post(`${backend.base}/api/v1${draftEndpoint}/parts-model`, {
      headers: { "x-csrf-token": csrf, "if-match": draftResponse.headers().etag ?? "" },
      multipart: { source: "self-authored named cube fixture", file: { name: "parts.glb", mimeType: "model/gltf-binary", buffer: fs.readFileSync(fixturePath("sample-model.glb")) } },
    });
    expect(partsResponse.status(), await partsResponse.text()).toBe(200);
    const entities: Record<string, unknown> = {};
    const facts = draft.knowledge.knowledge;
    for (const fact of [...facts.parts, ...facts.steps, ...facts.specs]) entities[fact.id] = { reviewStatus: "confirmed" };
    const part = facts.parts[0];
    const step = facts.steps[0];
    const spec = facts.specs[0];
    expect(part).toBeDefined();
    expect(step).toBeDefined();
    expect(spec).toBeDefined();
    entities[part?.id ?? ""] = { reviewStatus: "confirmed", userEdited: { name: CORRECTED_NAME, description: "已对照说明书核对" } };
    entities[step?.id ?? ""] = { reviewStatus: "confirmed", userEdited: { title: "已核对的取盖步骤", orderedActions: [CORRECTED_ACTION], safetyNotes: [CORRECTED_SAFETY] } };
    entities[spec?.id ?? ""] = { reviewStatus: "confirmed", userEdited: { label: "已核对的供电", value: CORRECTED_SPEC } };
    const fresh = await api.get(`${backend.base}/api/v1${draftEndpoint}`);
    const transform = { nodes: ["unit-cube"], kind: "translate", vector: [0.25, 0, 0] };
    await mutate(api, csrf, "PATCH", draftEndpoint, {
      entities,
      hotspots: { upsert: facts.parts.map((entry, index) => ({
        partId: entry.id, status: "confirmed", anchor: {
          modelRevisionId: draft.knowledge.model.revisionId,
          modelSha256: draft.knowledge.model.sha256,
          positionLocal: [0.4 - index * 0.2, 0.3, 1],
        },
      })) },
      interactive: {
        bindings: [{ partId: part?.id ?? "", nodes: ["unit-cube"], status: "confirmed" }],
        actions: [{ id: "fixture-open", label: "交互回归动作", triggerPartIds: [part?.id ?? ""], mode: "toggle", durationMs: 500, steps: [transform], stepIds: [step?.id ?? ""] }],
        poses: [],
      },
    }, fresh.headers().etag);
    const ready = await api.get(`${backend.base}/api/v1${draftEndpoint}`);
    await mutate(api, csrf, "PATCH", draftEndpoint, { modelReview: { loaded: true, userConfirmed: true } }, ready.headers().etag);
    await page.goto(`${WEB}/items/${seeded.itemId}/drafts/${job.draftId}/review`);
    const publish = page.getByRole("button", { name: "发布（生成不可变版本）", exact: true });
    await expect(publish).toBeEnabled();
    await publish.click();
    await expect(page.getByTestId("publish-success")).toBeVisible();
    await page.getByRole("link", { name: "打开阅读器", exact: true }).click();
    await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
    await expect(page.getByTestId("parts-list")).toContainText(CORRECTED_NAME);
    await expect(page.getByText(CORRECTED_SAFETY, { exact: false })).toBeVisible();
    const action = page.getByTestId("action-fixture-open");
    const onlineBefore = await page.getByTestId("viewer-canvas").screenshot();
    await action.click();
    await expect(action).toHaveAttribute("aria-pressed", "true");
    await expect.poll(async () => !(await page.getByTestId("viewer-canvas").screenshot()).equals(onlineBefore)).toBe(true);
    await page.getByTestId("interaction-reset").click();
    await expect(action).toHaveAttribute("aria-pressed", "false");
    const releaseId = new URL(page.url()).pathname.split("/").at(-1) ?? "";
    const downloadEvent = page.waitForEvent("download");
    await page.getByTestId("standalone-export").click();
    const download = await downloadEvent;
    const htmlPath = testInfo.outputPath("corrected-release-3d.html");
    await download.saveAs(htmlPath);
    const offlineContext = await browser.newContext({ viewport: { width: 1440, height: 1000 }, offline: true });
    const offline = await offlineContext.newPage();
    const requests: string[] = [];
    const errors: string[] = [];
    offline.on("request", (request) => { if (/^https?:/.test(request.url())) requests.push(request.url()); });
    offline.on("pageerror", (error) => errors.push(error.message));
    offline.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
    await offlineContext.route(/^https?:/, (route) => route.abort());
    try {
      await offline.goto(pathToFileURL(htmlPath).href);
      await expect(offline.locator("#em-status")).toContainText("拖动旋转", { timeout: 60_000 });
      await expect(offline.locator("#em-parts")).toContainText(CORRECTED_NAME);
      await expect(offline.locator("#em-step")).toContainText(CORRECTED_ACTION);
      await expect(offline.locator("#em-step")).toContainText(CORRECTED_SAFETY);
      await expect(offline.locator("#em-specs")).toContainText(CORRECTED_SPEC);
      const correctedPart = offline.getByRole("button", { name: CORRECTED_NAME, exact: true });
      await correctedPart.click();
      await expect(correctedPart).toHaveAttribute("aria-pressed", "true");
      const offlineAction = offline.getByRole("button", { name: "交互回归动作", exact: true });
      const offlineBefore = await offline.locator("#em-canvas").screenshot();
      await offlineAction.click();
      await expect(offlineAction).toHaveAttribute("aria-pressed", "true");
      await expect.poll(async () => !(await offline.locator("#em-canvas").screenshot()).equals(offlineBefore)).toBe(true);
      await offline.getByRole("button", { name: "复原", exact: true }).click();
      await expect(offlineAction).toHaveAttribute("aria-pressed", "false");
      await offline.getByRole("button", { name: "复位视角", exact: true }).click();
      await offline.screenshot({ path: path.join(EVIDENCE, "offline-corrected-release.png"), fullPage: true });
      expect(requests, "离线 HTML 不得尝试 HTTP(S) 请求").toEqual([]);
      expect(errors, "离线 HTML 不得产生控制台或页面错误").toEqual([]);
      fs.writeFileSync(path.join(EVIDENCE, "offline-verification.json"), JSON.stringify({
        browser: browser.version(),
        project: testInfo.project.name,
        releaseId,
        correctedFacts: true,
        correctedSafetyNotes: true,
        canvasChangedOnInteraction: true,
        httpRequests: requests.length,
        errors: errors.length,
      }, null, 2));
    } finally {
      await offlineContext.close();
    }
    await page.goto(`${WEB}/items/${seeded.itemId}/generations`);
    await expect(page.getByTestId("history-entry").getByRole("link", { name: "查看结果", exact: true })).toHaveAttribute("href", `/jobs/${seeded.jobId}/result`);
    await expect(page.getByTestId("history-entry").locator(`a[href="/items/${seeded.itemId}/releases/${releaseId}"]`)).toHaveCount(1);
    await page.screenshot({ path: path.join(EVIDENCE, "published-generation-history.png"), fullPage: true });
    expect(external).toEqual([]);
  } finally {
    fixture.state.manualDelayMs = 0;
    await api.dispose();
  }
});
