/**
 * T17 `job-recovery.spec.ts` 的自管测试设施（本机 fixture + 自管后端 + 造数）。
 *
 * 为什么需要它（与 T09/T16 的 global-setup 并存，互不影响）：
 * - 任务中心必须验证"**浏览器关闭后服务器继续**"与"**服务重启后从数据库恢复**"，
 *   这要求用例能自己启停后端进程——共享的 globalSetup 后端做不到；
 * - 需要把 Tripo / 说明书 AI 指向**本机 fixture**，才能在零真实外网、零真实付费的
 *   前提下产生 `needs_input` / `submission_unknown` / `succeeded` 等状态与"付费提交计数"。
 *
 * 隔离与安全：
 * - fixture 只绑定 `127.0.0.1`、随机端口；后端配置的 provider `base_url` 指向它，
 *   凭据用本套件专用的假环境变量；模型 CDN 也指向本机 fixture；
 * - 后端是**测试构建**（`--features job-failpoints`）：只有测试构建才放行
 *   "明文 http + 回环"的模型下载（T13 的两道门，生产构建拒绝）；
 * - 所有数据落在临时 data-dir，用例结束删除。
 *
 * 造数全部走公开 HTTP 合同（`helpers.ts` 的复用函数），不直接改数据库。
 */

import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";

import { expect, type APIRequestContext } from "@playwright/test";

import { apiLogin, seedItemWithDocument, seedPhoto, seedReadyPreparation } from "./helpers";
import { E2E_WEB_PORT, REPO_ROOT, fixturePath, serverBinary } from "./runtime";

export const MODEL_PRESET = "tripo-h-v3.1-standard";
/** 冻结的授权上限（与 e2e 价格目录一致：Tripo 30 credits、说明书 AI 上界）。 */
export const JOB_LIMITS = { tripoCreditMinor: 3000, manualAiUsdMicros: 500000 };
const TASK_ID = "t17-fixture-task-0001";

/** 建单幂等键的序号（键值必须 ASCII：HTTP 头不允许非 ASCII 字符）。 */
let seedCounter = 0;

// ---------------------------------------------------------------------------
// 本机 fixture（Tripo v3 + 说明书 AI + 模型 CDN 三段合一）
// ---------------------------------------------------------------------------

export interface FixtureState {
  /** 说明书 AI：正常产出结构化结果，或拒答（`needs_input` 的真实产品路径）。 */
  manualMode: "success" | "refuse";
  /**
   * 付费提交：`success` 返回 task_id；`http500` 无法证明未被接受 → `submission_unknown`；
   * `business400` 业务错误（明确未计费 → 释放预留，阶段 `failed`）。
   */
  submitMode: "success" | "http500" | "business400";
  /** 模型 CDN 返回完整 GLB，或截断字节（校验失败 → `model_validate` needs_input）。 */
  modelMode: "valid" | "truncated";
  /** 说明书 AI 响应延迟（毫秒）：给"浏览器关闭后服务器继续"留出观察窗口。 */
  manualDelayMs: number;
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        reject(new Error("无法取得空闲端口"));
        return;
      }
      const { port } = address;
      server.close(() => resolve(port));
    });
  });
}

/** 缺脚本必须显式失败（不返回通用成功），与 T05 fixture 的约定一致。 */
export class LocalFixture {
  readonly state: FixtureState = {
    manualMode: "success",
    submitMode: "success",
    modelMode: "valid",
    manualDelayMs: 0,
  };
  readonly counts = { upload: 0, submit: 0, task: 0, manual: 0, cdn: 0 };
  readonly lines: string[] = [];
  private server: http.Server | null = null;
  port = 0;

  get base(): string {
    return `http://127.0.0.1:${this.port}`;
  }

  /** 付费提交计数（"重复操作不多收费"的观察点）。 */
  paidSubmissions(): number {
    return this.counts.submit;
  }

  async start(): Promise<void> {
    this.port = await freePort();
    this.server = http.createServer((request, response) => {
      void this.handle(request, response);
    });
    await new Promise<void>((resolve) => this.server?.listen(this.port, "127.0.0.1", resolve));
  }

  async stop(): Promise<void> {
    if (this.server === null) {
      return;
    }
    await new Promise<void>((resolve) => this.server?.close(() => resolve()));
    this.server = null;
  }

  private log(line: string): void {
    this.lines.push(line);
  }

  private json(response: http.ServerResponse, status: number, body: unknown): void {
    const payload = JSON.stringify(body);
    response.writeHead(status, {
      "content-type": "application/json",
      "content-length": Buffer.byteLength(payload),
    });
    response.end(payload);
  }

  private async handle(request: http.IncomingMessage, response: http.ServerResponse): Promise<void> {
    const url = new URL(request.url ?? "/", `http://127.0.0.1:${this.port}`);
    const chunks: Buffer[] = [];
    for await (const chunk of request) {
      chunks.push(chunk as Buffer);
    }
    const raw = Buffer.concat(chunks);
    const method = request.method ?? "GET";
    this.log(`${method} ${url.pathname}`);

    if (method === "POST" && url.pathname === "/v3/files") {
      this.counts.upload += 1;
      this.json(response, 200, { code: 0, data: { file_token: `t17-token-${this.counts.upload}` } });
      return;
    }
    if (method === "POST" && url.pathname === "/v3/generation/multiview-to-model") {
      this.counts.submit += 1;
      this.log(`付费提交 #${this.counts.submit}`);
      if (this.state.submitMode === "http500") {
        this.json(response, 500, { error: { message: "fixture: 供应商 5xx（结果未知）" } });
        return;
      }
      if (this.state.submitMode === "business400") {
        this.json(response, 400, { code: 1001, message: "fixture: 业务错误（明确未计费）" });
        return;
      }
      this.json(response, 200, { code: 0, data: { task_id: TASK_ID } });
      return;
    }
    if (method === "GET" && url.pathname.startsWith("/v3/tasks/")) {
      this.counts.task += 1;
      this.json(response, 200, {
        code: 0,
        data: {
          task_id: TASK_ID,
          status: "success",
          progress: 100,
          credits_consumed: 30,
          output: {
            model_url: `${this.base}/cdn/model.glb`,
            rendered_image_url: "https://cdn.example.invalid/preview.png",
          },
        },
      });
      return;
    }
    if (method === "GET" && url.pathname === "/cdn/model.glb") {
      this.counts.cdn += 1;
      const bytes = fs.readFileSync(fixturePath("sample-model.glb"));
      const body = this.state.modelMode === "truncated" ? bytes.subarray(0, Math.floor(bytes.length / 3)) : bytes;
      response.writeHead(200, { "content-type": "model/gltf-binary", "content-length": body.length });
      response.end(body);
      return;
    }
    if (method === "POST" && url.pathname === "/v1/responses") {
      this.counts.manual += 1;
      if (this.state.manualDelayMs > 0) {
        await new Promise((resolve) => setTimeout(resolve, this.state.manualDelayMs));
      }
      if (this.state.manualMode === "refuse") {
        this.json(response, 200, {
          id: `resp_t17_refusal_${this.counts.manual}`,
          object: "response",
          status: "completed",
          model: "gpt-5-mini",
          output: [
            {
              type: "message",
              role: "assistant",
              content: [
                {
                  type: "refusal",
                  refusal: "e2e fixture：本次拒答（用于产生 needs_input 的真实产品路径）。",
                },
              ],
            },
          ],
          usage: { input_tokens: 100, output_tokens: 10, total_tokens: 110 },
        });
        return;
      }
      const requestBody = JSON.parse(raw.toString("utf8")) as {
        input?: Array<{ content?: Array<{ text?: string }> }>;
      };
      const prompt = requestBody.input?.[0]?.content?.[0]?.text ?? "";
      const pages = [...prompt.matchAll(/\[第 (\d+) 页\]/g)].map((match) => Number(match[1]));
      if (pages.length === 0) {
        this.json(response, 500, { error: { message: "fixture 请求里没有页标记" } });
        return;
      }
      this.json(response, 200, {
        id: `resp_t17_${this.counts.manual}`,
        object: "response",
        status: "completed",
        model: "gpt-5-mini",
        output: [
          {
            type: "message",
            role: "assistant",
            content: [
              {
                type: "output_text",
                annotations: [],
                text: JSON.stringify(buildKnowledge(pages)),
              },
            ],
          },
        ],
        usage: { input_tokens: 1000, output_tokens: 200, total_tokens: 1200 },
      });
      return;
    }
    this.log(`未命中路由：${method} ${url.pathname}（501，不返回通用成功）`);
    this.json(response, 501, { error: { message: `fixture has no route for ${url.pathname}` } });
  }
}

/** 构造 `manual_extract_v1` 结果（evidence 只引用本批输入页；与 T15 冒烟同一形状）。 */
function buildKnowledge(pages: number[]): unknown {
  const first = pages[0] ?? 1;
  const last = pages[pages.length - 1] ?? first;
  return {
    schemaVersion: "manual_extract_v1",
    parts: pages.map((page) => ({
      id: `p-${page}`,
      name: page === first ? "后盖" : `页${page}部件`,
      description: `第 ${page} 页描述的部件`,
      evidence: [{ pageNumber: page, quote: null }],
    })),
    steps: [
      {
        id: "s-1",
        title: "取下后盖",
        orderedActions: ["松开固定件", "取下后盖"],
        partIds: [`p-${first}`],
        evidence: [{ pageNumber: first, quote: "Loosen the four captive screws." }],
        safetyNotes: ["操作前断电。"],
      },
    ],
    specs: [{ id: "sp-1", label: "供电", value: "DC 12 V / 2.5 A", evidence: [{ pageNumber: last, quote: null }] }],
    uncertainties: [],
  };
}

// ---------------------------------------------------------------------------
// 自管后端（测试构建；临时 data-dir）
// ---------------------------------------------------------------------------

export const BACKEND_PASSWORD = "t17-e2e-password-9f21";

export class TestBackend {
  readonly workDir: string;
  readonly dataDir: string;
  readonly logPath: string;
  base = "";
  port = 0;
  private process: ChildProcess | null = null;

  constructor(tag: string) {
    this.workDir = fs.mkdtempSync(path.join(os.tmpdir(), `em-t17-${tag}-`));
    this.dataDir = path.join(this.workDir, "data");
    this.logPath = path.join(this.workDir, "server.log");
  }

  async start(fixture: LocalFixture): Promise<void> {
    if (this.port === 0) {
      this.port = await freePort();
      const passwordFile = path.join(this.workDir, "password.txt");
      fs.writeFileSync(passwordFile, `${BACKEND_PASSWORD}\n`, { mode: 0o600 });
      const priceCatalog = path.join(this.workDir, "price-catalog.toml");
      fs.copyFileSync(path.join(REPO_ROOT, "price-catalog.example.toml"), priceCatalog);
      // 测试构建 + 显式测试配置才放行"明文 http + 回环"的模型下载（T13 两道门）。
      fs.writeFileSync(
        path.join(this.workDir, "config.toml"),
        [
          `price_catalog_path = ${JSON.stringify(priceCatalog)}`,
          // 浏览器请求经用例的路由改写到达本后端：`Origin` 仍是 Vite 页面源
          // （http://127.0.0.1:<E2E_WEB_PORT>），因此必须显式声明 public_origin，
          // 否则修改请求会被 Origin 校验拒绝（403）。
          `public_origin = "http://127.0.0.1:${E2E_WEB_PORT}"`,
          "",
          "[providers.tripo]",
          `base_url = "${fixture.base}/v3"`,
          'api_key_env = "EM_T17_TRIPO_KEY"',
          "",
          "[providers.manual_ai]",
          `base_url = "${fixture.base}/v1"`,
          'model = "gpt-5-mini"',
          'api_key_env = "EM_T17_MANUAL_AI_KEY"',
          "",
          "[download]",
          'allowed_hosts = ["127.0.0.1"]',
          "allow_local_fixture = true",
          "",
        ].join("\n"),
      );
      execFileSync(serverBinary(), ["init", "--data-dir", this.dataDir, "--password-file", passwordFile], {
        stdio: "inherit",
      });
    }
    const log = fs.createWriteStream(this.logPath, { flags: "a" });
    const child = spawn(
      serverBinary(),
      [
        "serve",
        "--data-dir",
        this.dataDir,
        "--listen",
        `127.0.0.1:${this.port}`,
      ],
      {
        cwd: this.workDir,
        env: {
          ...process.env,
          EM_T17_TRIPO_KEY: "t17-fake-tripo-key",
          EM_T17_MANUAL_AI_KEY: "t17-fake-manual-ai-key",
        },
      },
    );
    child.stdout?.pipe(log);
    child.stderr?.pipe(log);
    this.process = child;
    this.base = `http://127.0.0.1:${this.port}`;
    await waitForReady(`${this.base}/api/v1/health/ready`, this.logPath);
  }

  /** 停止后端（SIGTERM；等待退出，避免下一个进程拿不到数据目录锁）。 */
  async stop(): Promise<void> {
    const child = this.process;
    if (child === null) {
      return;
    }
    this.process = null;
    await new Promise<void>((resolve) => {
      child.once("exit", () => resolve());
      child.kill("SIGTERM");
      setTimeout(() => {
        child.kill("SIGKILL");
        resolve();
      }, 10_000);
    });
  }

  async restart(fixture: LocalFixture): Promise<void> {
    await this.stop();
    await this.start(fixture);
  }

  /** 收尾：停止后端并删除临时工作目录（data-dir/日志/口令文件都不留在机器上）。 */
  async cleanup(fixture: LocalFixture): Promise<void> {
    await this.stop();
    fs.rmSync(this.workDir, { recursive: true, force: true });
    await fixture.stop();
  }
}

async function waitForReady(url: string, logPath: string): Promise<void> {
  const deadline = Date.now() + 30_000;
  let lastError = "";
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
      lastError = `HTTP ${response.status}`;
    } catch (error) {
      lastError = String(error);
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  const tail = fs.existsSync(logPath)
    ? fs.readFileSync(logPath, "utf8").split("\n").slice(-20).join("\n")
    : "(无日志)";
  throw new Error(`T17 测试后端未就绪（${url}）：${lastError}\n日志尾部：\n${tail}`);
}

/** 构建测试构建的后端二进制（仅测试构建放行本机 fixture 的模型下载）。 */
export function buildTestServerBinary(): void {
  execFileSync("cargo", ["build", "-p", "everything-manual", "--features", "job-failpoints"], {
    cwd: REPO_ROOT,
    stdio: "inherit",
  });
}

// ---------------------------------------------------------------------------
// 造数与读取（全部走公开 HTTP 合同）
// ---------------------------------------------------------------------------

export interface SeededJob {
  readonly itemId: string;
  readonly jobId: string;
}

async function api<T>(
  request: APIRequestContext,
  options: {
    method: "GET" | "POST";
    url: string;
    csrf?: string;
    data?: unknown;
    ifMatch?: string | null;
    idempotencyKey?: string;
  },
): Promise<{ status: number; body: T; etag: string | null }> {
  const headers: Record<string, string> = {};
  if (options.csrf !== undefined) {
    headers["x-csrf-token"] = options.csrf;
  }
  if (options.ifMatch !== undefined && options.ifMatch !== null) {
    headers["if-match"] = options.ifMatch;
  }
  if (options.idempotencyKey !== undefined) {
    headers["idempotency-key"] = options.idempotencyKey;
  }
  const response = await request.fetch(options.url, {
    method: options.method,
    headers,
    data: options.data,
  });
  const text = await response.text();
  return {
    status: response.status(),
    body: (text === "" ? undefined : JSON.parse(text)) as T,
    etag: response.headers()["etag"] ?? null,
  };
}

/**
 * 造一个完整可执行的 job：物品 + 说明书 + ready 准备 + front/left 照片 + 报价确认建单。
 *
 * 必须在调用前把 fixture 设成目标场景（manual/submit/model 模式）。
 */
export async function seedJob(
  request: APIRequestContext,
  backend: TestBackend,
  name: string,
): Promise<SeededJob> {
  const seed = await seedItemWithDocument(
    request,
    backend.base,
    BACKEND_PASSWORD,
    "sample-manual-text.pdf",
    name,
  );
  // `seedItemWithDocument` 内部登录过：CSRF token 与会话 cookie 绑定，必须取
  // **当前 cookie jar 那一次会话**的 token（否则 403 CSRF_REJECTED）。
  const csrf = await apiLogin(request, backend.base, BACKEND_PASSWORD);
  const preparationId = await seedReadyPreparation(request, backend.base, csrf, seed);
  await seedPhoto(request, backend.base, csrf, seed.itemId, "front", "sample-photo-front.jpg");
  await seedPhoto(request, backend.base, csrf, seed.itemId, "left", "sample-photo-left.png");
  const photos = await api<{ data: { id: string }[] }>(request, {
    method: "GET",
    url: `${backend.base}/api/v1/items/${seed.itemId}/photos`,
  });
  const photoIds = photos.body.data.map((photo) => photo.id);

  const estimate = await api<{ data: { id: string } }>(request, {
    method: "POST",
    url: `${backend.base}/api/v1/items/${seed.itemId}/estimates`,
    csrf,
    data: { preparationId, photoIds, modelPreset: MODEL_PRESET },
  });
  expect(estimate.status, JSON.stringify(estimate.body)).toBe(201);
  const confirm = await api(request, {
    method: "POST",
    url: `${backend.base}/api/v1/items/${seed.itemId}/estimates/${estimate.body.data.id}/confirm`,
    csrf,
  });
  expect(confirm.status, JSON.stringify(confirm.body)).toBe(200);
  // 幂等键只允许 ASCII（HTTP 头值）：用序号而不是物品名。
  const job = await api<{ data: { id: string } }>(request, {
    method: "POST",
    url: `${backend.base}/api/v1/items/${seed.itemId}/jobs`,
    csrf,
    idempotencyKey: `t17-seed-${seedCounter++}`,
    data: {
      quoteId: estimate.body.data.id,
      preparationId,
      photoIds,
      limits: JOB_LIMITS,
    },
  });
  expect(job.status, JSON.stringify(job.body)).toBe(202);
  return { itemId: seed.itemId, jobId: job.body.data.id };
}

export interface JobStageView {
  id: string;
  stageKind: string;
  status: string;
  needsInput: { code: string; message: string }[];
  retry: { allowed: boolean; reason: string | null; message: string | null };
  submissionStyle: string | null;
  lastError: string | null;
}

export interface JobDetailView {
  id: string;
  status: string;
  revision: number;
  item: { id: string; name: string; model: string };
  stages: JobStageView[];
  reservations: { provider: string; state: string; reservedDisplay: string }[];
  draftId: string | null;
  etag: string | null;
}

export async function fetchJobDetail(
  request: APIRequestContext,
  base: string,
  jobId: string,
): Promise<JobDetailView> {
  const response = await request.get(`${base}/api/v1/jobs/${jobId}`);
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: Omit<JobDetailView, "etag"> };
  return { ...body.data, etag: response.headers()["etag"] ?? null };
}

/** 轮询任务详情直到断言成立（服务端事实；用例自己的等待，不依赖界面轮询）。 */
export async function waitForJob(
  request: APIRequestContext,
  base: string,
  jobId: string,
  predicate: (detail: JobDetailView) => boolean,
  description: string,
  timeoutMs = 60_000,
): Promise<JobDetailView> {
  const deadline = Date.now() + timeoutMs;
  let last: JobDetailView | null = null;
  while (Date.now() < deadline) {
    last = await fetchJobDetail(request, base, jobId);
    if (predicate(last)) {
      return last;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(
    `等待超时：${description}；最后状态 ${last?.status}，阶段 ${JSON.stringify(last?.stages.map((stage) => [stage.stageKind, stage.status]))}`,
  );
}

export function stageOf(detail: JobDetailView, kind: string): JobStageView {
  const stage = detail.stages.find((candidate) => candidate.stageKind === kind);
  expect(stage, `找不到阶段 ${kind}：${JSON.stringify(detail.stages)}`).toBeTruthy();
  return stage as JobStageView;
}
