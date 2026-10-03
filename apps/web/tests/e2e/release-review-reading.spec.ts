/** BUG-PCF-003: actual local HTTP pipeline, UI edits/publications, frozen release reading.
 * Setup uses synthetic provider input and API review declarations; no real provider calls.
 * Browser routes only rewrite the local API origin and block external requests.
 */
import { createHash } from "node:crypto";
import http from "node:http";
import { expect, request, test, type APIRequestContext, type Page } from "@playwright/test";
import type { DraftPart, DraftSpec, DraftStep } from "../../src/features/viewer/draft-view";
import { apiLogin, loginViaUi } from "./helpers";
import { BACKEND_PASSWORD, LocalFixture, seedJob, TestBackend, waitForJob } from "./job-recovery-harness";
import { E2E_WEB_PORT } from "./runtime";
import { installRealBackendRouting } from "./viewer-harness";

test.describe.configure({ timeout: 300_000, mode: "serial" });
const web = `http://127.0.0.1:${E2E_WEB_PORT}`;
let fixture: LocalFixture;
let backend: TestBackend;
let api: APIRequestContext;
let itemId: string;
let draftId: string;
let source: Knowledge;
let part: DraftPart;
let textPart: DraftPart;
let step: DraftStep;
let spec: DraftSpec;

interface Knowledge {
  model: { revisionId: string; sha256: string; assetId: string };
  knowledge: { parts: DraftPart[]; steps: DraftStep[]; specs: DraftSpec[] };
  hotspots: unknown[];
  stepPoses: unknown;
}
interface Manifest {
  knowledge: Knowledge;
  review: { entities: Record<string, { userEdited?: Record<string, unknown> }> };
  assets: { assetId: string; sha256: string }[];
  documents: { documentId: string; title: string }[];
}
interface Release {
  id: string;
  manifestAssetId: string;
  manifestSha256: string;
  manifest: Manifest;
}
const draftUrl = () => `${backend.base}/api/v1/items/${itemId}/drafts/${draftId}`;
const readerPath = (releaseId: string) => `/items/${itemId}/releases/${releaseId}`;
const digest = (bytes: Buffer) => createHash("sha256").update(bytes).digest("hex");

/** The shared seed prepares exactly one page, so its default provider has only one part.
 * This suite defines two synthetic parts in the provider's original extraction response.
 * The proxy forwards other fixture endpoints unchanged; no stored knowledge is patched.
 */
class ReadingFixture extends LocalFixture {
  private readingServer: http.Server | null = null;
  private readingPort = 0;

  override get base() { return this.readingPort === 0 ? super.base : `http://127.0.0.1:${this.readingPort}`; }

  override async start() {
    await super.start();
    const upstream = super.base;
    this.readingServer = http.createServer((incoming, response) => {
      if (incoming.method !== "POST" || incoming.url !== "/v1/responses") {
        const forwarding = http.request(`${upstream}${incoming.url ?? "/"}`, { method: incoming.method, headers: incoming.headers }, (result) => {
          response.writeHead(result.statusCode ?? 502, result.headers); result.pipe(response);
        });
        forwarding.on("error", () => { response.writeHead(502); response.end(); });
        incoming.pipe(forwarding);
        return;
      }
      void (async () => {
        const chunks: Buffer[] = [];
        for await (const chunk of incoming) chunks.push(Buffer.from(chunk));
        const body = JSON.parse(Buffer.concat(chunks).toString("utf8")) as { input?: { content?: { text?: string }[] }[] };
        const prompt = body.input?.[0]?.content?.[0]?.text ?? "";
        const pages = [...prompt.matchAll(/\[第 (\d+) 页\]/g)].map((match) => Number(match[1]));
        if (pages.length !== 1 || pages[0] !== 1) throw new Error("Expected the declared one-page synthetic input");
        this.counts.manual += 1;
        const evidence = [{ pageNumber: 1, quote: null }];
        const extraction = {
          schemaVersion: "manual_extract_v1",
          parts: [
            { id: "p-1", name: "后盖", description: "合成资料中的后盖", evidence },
            { id: "p-text", name: "电源接头", description: "合成资料中的仅文本部件", evidence },
          ],
          steps: [{ id: "s-1", title: "取下后盖", orderedActions: ["松开固定件", "取下后盖"], partIds: ["p-1"], safetyNotes: ["操作前断电。"], evidence }],
          specs: [{ id: "sp-1", label: "供电", value: "DC 12 V / 2.5 A", evidence }],
          uncertainties: [],
        };
        const result = JSON.stringify({
          id: `resp_reader_${this.counts.manual}`, object: "response", status: "completed", model: "gpt-5-mini",
          output: [{ type: "message", role: "assistant", content: [{ type: "output_text", annotations: [], text: JSON.stringify(extraction) }] }],
          usage: { input_tokens: 1000, output_tokens: 200, total_tokens: 1200 },
        });
        response.writeHead(200, { "content-type": "application/json" }); response.end(result);
      })().catch(() => { response.writeHead(500); response.end("Invalid synthetic reader fixture input"); });
    });
    await new Promise<void>((resolve) => this.readingServer!.listen(0, "127.0.0.1", resolve));
    const address = this.readingServer.address();
    if (address === null || typeof address === "string") throw new Error("Missing reader fixture port");
    this.readingPort = address.port;
  }

  override async stop() {
    const server = this.readingServer;
    this.readingServer = null;
    if (server !== null) await new Promise<void>((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
    this.readingPort = 0;
    await super.stop();
  }
}

async function readDraft() {
  const response = await api.get(draftUrl());
  expect(response.status(), await response.text()).toBe(200);
  const { data } = await response.json() as { data: { knowledge: Knowledge } };
  return { ...data, etag: response.headers().etag ?? "" };
}
async function readRelease(releaseId: string) {
  const response = await api.get(`${backend.base}/api/v1/items/${itemId}/releases/${releaseId}`);
  expect(response.status(), await response.text()).toBe(200);
  return (await response.json() as { data: Release }).data;
}
async function readBytes(assetId: string) {
  const response = await api.get(`${backend.base}/api/v1/assets/${assetId}/content`);
  expect(response.status()).toBe(200);
  return response.body();
}
async function frozenBytes(release: Release) {
  const bytes = await readBytes(release.manifestAssetId);
  expect(digest(bytes)).toBe(release.manifestSha256);
  const assets: Record<string, string> = {};
  for (const asset of release.manifest.assets) {
    assets[asset.assetId] = digest(await readBytes(asset.assetId));
    expect(assets[asset.assetId]).toBe(asset.sha256);
  }
  return { manifestBytes: bytes.toString("base64"), sha256: digest(bytes), assets };
}

test.beforeAll(async () => {
  const binary = process.env.EM_E2E_SERVER_BINARY;
  if (!binary) throw new Error("Supply the frozen job-failpoints binary via EM_E2E_SERVER_BINARY; no automatic build is allowed.");
  fixture = new ReadingFixture();
  await fixture.start();
  backend = new TestBackend("release-review-reading", binary);
  await backend.start(fixture);
  api = await request.newContext();
  const seeded = await seedJob(api, backend, "人工修订发布阅读回归（合成资料）");
  itemId = seeded.itemId;
  const job = await waitForJob(api, backend.base, seeded.jobId, (entry) => entry.status === "succeeded", "release review fixture", 180_000);
  draftId = job.draftId ?? "";
  expect(draftId).not.toBe("");
  const csrf = await apiLogin(api, backend.base, BACKEND_PASSWORD);
  const current = await readDraft();
  part = current.knowledge.knowledge.parts[0]!;
  textPart = current.knowledge.knowledge.parts[1]!;
  step = current.knowledge.knowledge.steps[0]!;
  spec = current.knowledge.knowledge.specs[0]!;
  expect(part).toBeDefined(); expect(textPart).toBeDefined(); expect(step).toBeDefined(); expect(spec).toBeDefined();
  const entities = Object.fromEntries([
    ...current.knowledge.knowledge.parts.map((entry) => [entry.id, {
      reviewStatus: "confirmed", textOnly: entry.id !== part.id,
      // A partial existing edit exercises fallback in the real published reader too.
      ...(entry.id === textPart.id ? { userEdited: { description: "仅修改说明，部件名保留原文" } } : {}),
    }]),
    ...[...current.knowledge.knowledge.steps, ...current.knowledge.knowledge.specs].map((entry) => [entry.id, { reviewStatus: "confirmed" }]),
  ]);
  const response = await api.patch(draftUrl(), {
    headers: { "x-csrf-token": csrf, "if-match": current.etag },
    data: {
      entities, modelReview: { loaded: true, userConfirmed: true },
      hotspots: { upsert: [{ partId: part.id, status: "confirmed", anchor: {
        modelRevisionId: current.knowledge.model.revisionId, modelSha256: current.knowledge.model.sha256, positionLocal: [0.4, 0.3, 1],
      } }] },
    },
  });
  expect(response.status(), await response.text()).toBe(200);
  source = (await readDraft()).knowledge;
});
test.afterAll(async () => { await api?.dispose(); await backend?.cleanup(fixture); });

async function editAndPublish(page: Page, version: "A" | "B") {
  await page.goto(`${web}/items/${itemId}/drafts/${draftId}/review`);
  await expect(page.getByTestId("knowledge-panel")).toBeVisible();
  for (const [id, fields] of [
    [part.id, { 部件名: `${version}人工部件`, 说明: `${version}人工说明` }],
    [step.id, { 步骤标题: `${version}人工步骤`, "操作（每行一步）": `${version}操作一\n${version}操作二` }],
    [spec.id, { 规格名: `${version}人工规格`, 规格值: `${version}人工数值` }],
  ] as const) {
    const entry = page.getByTestId(`knowledge-${id}`);
    await entry.getByRole("button", { name: "复制为本地修订" }).click();
    for (const [label, value] of Object.entries(fields)) {
      // React textarea content may be included in its wrapping label's text.
      const control = label === "说明" ? entry.getByLabel(/^说明/)
        : label === "操作（每行一步）" ? entry.getByLabel(/^操作（每行一步）/)
        : entry.getByLabel(label, { exact: true });
      await control.fill(value);
    }
    await entry.getByRole("button", { name: "保存人工修订（并确认事实）" }).click();
    await expect(page.getByTestId(`knowledge-edited-${id}`)).toContainText(`${version}人工`);
    await expect(entry.getByRole("button", { name: "复制为本地修订" })).toBeEnabled();
  }
  const publishing = page.waitForResponse((response) => response.request().method() === "POST" && response.url().endsWith(`/drafts/${draftId}/publish`));
  await page.getByRole("button", { name: "发布（生成不可变版本）" }).click();
  const response = await publishing;
  expect(response.status(), await response.text()).toBe(201);
  const releaseId = (await response.json() as { data: { id: string } }).data.id;
  await expect(page.getByTestId("publish-success")).toContainText(releaseId);
  await page.getByRole("link", { name: "打开阅读器", exact: true }).click();
  await expect(page.getByRole("heading", { name: "已发布说明书" })).toBeVisible();
  return readRelease(releaseId);
}

async function assertReader(page: Page, version: "A" | "B", release: Release) {
  const partRow = page.getByTestId(`reader-part-${part.id}`);
  const stepRow = page.getByTestId(`reader-step-${step.id}`);
  const specRow = page.getByTestId(`reader-spec-${spec.id}`);
  const name = `${version}人工部件`;
  await expect(partRow.getByRole("button", { name, exact: true })).toBeVisible();
  await expect(partRow).toContainText(`${version}人工说明`);
  await expect(stepRow.getByRole("button", { name: `${version}人工步骤`, exact: true })).toBeVisible();
  await expect(stepRow.locator(".step-detail")).toContainText(`${version}操作一`);
  await expect(stepRow.locator(".step-detail")).toContainText(`${version}操作二`);
  await expect(specRow).toContainText(`${version}人工规格：${version}人工数值`);
  await expect(page.getByTestId("reader-current-step")).toContainText(`${version}人工步骤`);
  await expect(stepRow.getByRole("button", { name, exact: true })).toBeVisible();
  await expect(page.getByTestId(`reader-part-${textPart.id}`).getByRole("button", { name: textPart.name, exact: true })).toBeVisible();
  await expect(page.getByTestId(`reader-part-${textPart.id}`)).toContainText("仅修改说明，部件名保留原文");
  await expect(page.getByTestId("text-only-note")).toContainText(textPart.name);
  await partRow.getByRole("button", { name, exact: true }).click();
  await expect(page.getByTestId("reader-notice")).toContainText(name);
  for (const id of [part.id, step.id, spec.id]) {
    const revision = page.getByTestId(`reader-revision-${id}`);
    await expect(revision).toContainText("已修订（人工）");
    expect((await revision.locator("summary").boundingBox())?.height).toBeGreaterThanOrEqual(44);
    const details = revision.locator("details");
    if (await details.getAttribute("open") === null) {
      await revision.locator("summary").focus();
      await revision.locator("summary").press("Enter");
    }
    const paragraphs = details.locator(":scope > p");
    const expectedParagraphs = id === part.id ? [part.name, ...(part.description !== "" ? [part.description] : [])]
      : id === step.id ? [step.title] : [`${spec.label}：${spec.value}`];
    await expect(paragraphs).toHaveText(expectedParagraphs);
    for (let index = 0; index < expectedParagraphs.length; index += 1) await expect(paragraphs.nth(index)).toBeVisible();
    if (id === step.id) {
      const actions = details.locator(":scope > ol > li");
      await expect(actions).toHaveText(step.orderedActions);
      for (let index = 0; index < step.orderedActions.length; index += 1) await expect(actions.nth(index)).toBeVisible();
    }
  }
  for (const [id, kind, evidence] of [[part.id, "part", part.evidence], [step.id, "step", step.evidence], [spec.id, "spec", spec.evidence]] as const) {
    const first = evidence[0]!;
    expect(first).toBeDefined();
    const document = release.manifest.documents.find((entry) => entry.documentId === first.documentId)!;
    expect(document).toBeDefined();
    const button = page.getByTestId(`reader-${kind}-${id}`).getByRole("button", { name: `查看出处 · ${document.title} · 第 ${first.pageNumber} 页`, exact: true });
    await button.focus(); await button.press("Enter");
    await expect(page.getByTestId("original-page-label")).toContainText(`第 ${first.pageNumber} /`);
    await expect(page.getByTestId("original-canvas")).toBeVisible();
    await expect(page.locator("#original-heading")).toHaveText(`原文 · ${document.title}`);
    await page.getByRole("button", { name: "返回出处", exact: true }).click();
    await expect(button).toBeFocused();
  }
}

test("PCF-003 UI edits → publish A/B → each release keeps its own text, source, IDs and bytes", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  const routing = await installRealBackendRouting(page, backend.base);
  await loginViaUi(page, web, BACKEND_PASSWORD);
  const calls = { ...fixture.counts };
  const releaseA = await editAndPublish(page, "A");
  await assertReader(page, "A", releaseA);
  expect(releaseA.manifest.knowledge).toEqual(source);
  const bytesA = await frozenBytes(releaseA);
  await page.screenshot({ path: testInfo.outputPath("release-A.png"), fullPage: true });
  const releaseB = await editAndPublish(page, "B");
  await assertReader(page, "B", releaseB);
  expect(releaseB.manifest.knowledge).toEqual(source);
  expect(releaseB.manifest.assets).toEqual(releaseA.manifest.assets);
  expect(releaseB.manifest.review.entities[part.id]?.userEdited?.name).toBe("B人工部件");
  expect(releaseA.manifest.review.entities[part.id]?.userEdited?.name).toBe("A人工部件");
  expect((await readDraft()).knowledge).toEqual(source);
  const bytesB = await frozenBytes(releaseB);
  expect(bytesB.sha256).not.toBe(bytesA.sha256);
  expect(await frozenBytes(await readRelease(releaseA.id))).toEqual(bytesA);

  // Actual links/history navigate the same client app; the reader must never fetch a draft.
  const draftReads: string[] = [];
  page.on("request", (entry) => { if (entry.method() === "GET" && /\/drafts\//.test(entry.url())) draftReads.push(entry.url()); });
  for (const [version, release] of [["A", releaseA], ["B", releaseB], ["A", releaseA]] as const) {
    await page.getByRole("link", { name: "返回版本列表", exact: true }).click();
    await page.locator(`a[href="${readerPath(release.id)}"]`).click();
    await assertReader(page, version, release);
  }
  expect(draftReads).toEqual([]);
  expect(fixture.counts).toEqual(calls);
  expect(routing.external).toEqual([]);
  expect(routing.fulfilled).toBe(0);
  expect(routing.assetRequests.length).toBeGreaterThan(0);
  await page.screenshot({ path: testInfo.outputPath("release-A-after-B.png"), fullPage: true });
  await testInfo.attach("release-review-evidence", { contentType: "application/json", body: JSON.stringify({
    releaseA: { id: releaseA.id, sha256: bytesA.sha256, assets: bytesA.assets },
    releaseB: { id: releaseB.id, sha256: bytesB.sha256, assets: bytesB.assets },
    sourceKnowledgeUnchanged: true, draftReadsDuringReading: draftReads, externalRequests: routing.external,
  }, null, 2) });
});
