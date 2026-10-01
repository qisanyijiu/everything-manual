/** AS-01 QA 的隔离设施。密钥只在内存/临时私有文件，证据仅写匹配布尔值。 */
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";

import { expect, type APIRequestContext, type Page } from "@playwright/test";

import type { components } from "../../src/api/generated";
import { apiLogin, seedItemWithDocument, seedPhoto, seedReadyPreparation } from "./helpers";
import { LocalFixture, JOB_LIMITS } from "./job-recovery-harness";
import { REPO_ROOT, serverBinary } from "./runtime";

export const QA_WEB_PORT = Number(process.env.EM_AS_QA_WEB_PORT ?? 15186);
export const QA_WEB_BASE = `http://127.0.0.1:${QA_WEB_PORT}`;
export const QA_DIR = process.env.EM_AS_QA_ARTIFACT_DIR
  ?? path.join(REPO_ROOT, "artifacts", "api-settings", "qa");
export const QA_PASSWORD = "api-settings-qa-local-password";

export async function freePort(): Promise<number> {
  const server = net.createServer();
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("QA fixture 没有监听地址");
  await new Promise<void>((resolve) => server.close(() => resolve()));
  return address.port;
}

/** 复用成熟响应脚本，但在入口独立检查 Authorization 与实际 wire model。
 * 不把 header/body/随机密钥加入日志、断言预期值或证据。
 */
export class CheckedProviderFixture {
  readonly upstream = new LocalFixture();
  readonly keys = { tripo: `qa-${randomBytes(24).toString("hex")}`, manualAi: `qa-${randomBytes(24).toString("hex")}` };
  readonly checks = { tripoRequests: 0, manualRequests: 0, authMatches: true, modelMatches: true, unknownRequests: 0 };
  private server: http.Server | null = null;
  base = "";
  /** ES-01 isolated reflection cases; transformed response bytes stay in memory. */
  responseTransform: ((pathname: string, response: { status: number; body: Buffer }) => { status: number; body: Buffer }) | null = null;

  constructor(readonly models: { tripo: string; manualAi: string }) {}

  async start(): Promise<void> {
    await this.upstream.start();
    this.server = http.createServer((req, res) => {
      void this.handle(req, res).catch(() => {
        if (!res.headersSent) res.writeHead(500, { "content-type": "application/json" });
        res.end('{"error":{"message":"QA fixture local forwarding failed"}}');
      });
    });
    await new Promise<void>((resolve) => this.server!.listen(0, "127.0.0.1", resolve));
    const address = this.server.address();
    if (!address || typeof address === "string") throw new Error("QA fixture 没有监听地址");
    this.base = `http://127.0.0.1:${address.port}`;
  }

  private async handle(req: http.IncomingMessage, res: http.ServerResponse) {
    const chunks: Buffer[] = [];
    for await (const chunk of req) chunks.push(chunk as Buffer);
    const raw = Buffer.concat(chunks);
    const pathname = new URL(req.url ?? "/", this.base).pathname;
    const isTripo = pathname.startsWith("/v3/");
    const isManual = pathname === "/v1/responses";
    if (isTripo || isManual) {
      this.checks[isTripo ? "tripoRequests" : "manualRequests"] += 1;
      this.checks.authMatches &&= req.headers.authorization === `Bearer ${isTripo ? this.keys.tripo : this.keys.manualAi}`;
      if (pathname === "/v3/generation/multiview-to-model" || isManual) {
        const body = JSON.parse(raw.toString("utf8")) as { model?: unknown };
        this.checks.modelMatches &&= body.model === (isTripo ? this.models.tripo : this.models.manualAi);
      }
    } else this.checks.unknownRequests += 1;
    // 目标为本机响应夹具固定 origin，不由请求数据决定。
    const headers: Record<string, string> = {};
    if (typeof req.headers["content-type"] === "string") headers["content-type"] = req.headers["content-type"];
    const response = await fetch(`${this.upstream.base}${pathname}`, {
      method: req.method,
      headers,
      body: raw.length ? raw : undefined,
      redirect: "error",
    });
    const original = { status: response.status, body: Buffer.from(await response.arrayBuffer()) };
    const sent = this.responseTransform?.(pathname, original) ?? original;
    res.writeHead(sent.status, { "content-type": response.headers.get("content-type") ?? "application/json" });
    res.end(sent.body);
  }

  async stop(): Promise<void> {
    if (this.server) await new Promise<void>((resolve) => this.server!.close(() => resolve()));
    this.server = null;
    await this.upstream.stop();
  }
}

/** 每例独立真实进程，同 data-dir 重启；不继承任何 EM_* 部署覆盖。 */
export class ApiSettingsQaBackend {
  readonly workDir = fs.mkdtempSync(path.join(os.tmpdir(), "em-api-settings-qa-"));
  readonly dataDir = path.join(this.workDir, "data");
  readonly logPath = path.join(this.workDir, "server.log");
  // ES-01: regular QA processes must never create/read the user's default Keychain item.
  // Random per backend, kept stable only for its own same-data-dir restart.
  readonly masterKey = randomBytes(32).toString("hex");
  private process: ChildProcess | null = null;
  private initialized = false;
  private log: fs.WriteStream | null = null;
  port = 0;
  base = "";

  constructor(readonly deployment: CheckedProviderFixture, readonly alternative: CheckedProviderFixture) {}

  private environment(): NodeJS.ProcessEnv {
    return {
      PATH: process.env.PATH,
      TMPDIR: process.env.TMPDIR,
      EM_SECRETS_MASTER_KEY: this.masterKey,
      EM_API_SETTINGS_QA_TRIPO: this.deployment.keys.tripo,
      EM_API_SETTINGS_QA_MANUAL: this.deployment.keys.manualAi,
      // 明确环境 > TOML：下方 TOML 使用另一模型名，运行应采用这里的值。
      EM_PROVIDERS__TRIPO__MODEL: this.deployment.models.tripo,
      EM_PROVIDERS__MANUAL_AI__MODEL: this.deployment.models.manualAi,
    };
  }

  async start(): Promise<void> {
    if (!this.initialized) {
      this.port = await freePort();
      this.base = `http://127.0.0.1:${this.port}`;
      const passwordFile = path.join(this.workDir, "password.txt");
      fs.writeFileSync(passwordFile, `${QA_PASSWORD}\n`, { mode: 0o600 });
      const catalog = path.join(this.workDir, "price-catalog.toml");
      fs.writeFileSync(catalog, [
        'version = "api-settings-qa"', 'snapshot_date = "2026-09-21"',
        ...[this.deployment, this.alternative].flatMap((fixture, index) => [
          "[[tripo.presets]]", `preset = "api-settings-qa-${index}"`, `model = ${JSON.stringify(fixture.models.tripo)}`, 'credits = "30"',
        ]),
        ...[...new Set([this.deployment.models.manualAi, this.alternative.models.manualAi])].flatMap((model) => [
          `[manual_ai.models.${JSON.stringify(model)}]`, 'input_usd_per_million_tokens = "0.25"',
          'output_usd_per_million_tokens = "2.00"', 'image_usd_per_image = "0.01"',
        ]),
      ].join("\n"));
      fs.writeFileSync(path.join(this.workDir, "config.toml"), [
        `price_catalog_path = ${JSON.stringify(catalog)}`, `public_origin = ${JSON.stringify(QA_WEB_BASE)}`,
        "[providers.tripo]", `base_url = ${JSON.stringify(`${this.deployment.base}/v3`)}`,
        'model = "qa-toml-tripo"', 'api_key_env = "EM_API_SETTINGS_QA_TRIPO"',
        "[providers.manual_ai]", `base_url = ${JSON.stringify(`${this.deployment.base}/v1`)}`,
        'model = "qa-toml-manual"', 'api_key_env = "EM_API_SETTINGS_QA_MANUAL"',
        "[download]", 'allowed_hosts = ["127.0.0.1"]', 'allow_local_fixture = true', "",
      ].join("\n"));
      execFileSync(serverBinary(), ["init", "--data-dir", this.dataDir, "--password-file", passwordFile], {
        cwd: this.workDir, env: this.environment(), stdio: "pipe",
      });
      this.initialized = true;
    }
    this.log = fs.createWriteStream(this.logPath, { flags: "a", mode: 0o600 });
    const child = spawn(serverBinary(), ["serve", "--data-dir", this.dataDir, "--listen", `127.0.0.1:${this.port}`], {
      cwd: this.workDir, env: this.environment(), stdio: ["ignore", "pipe", "pipe"],
    });
    child.stdout?.pipe(this.log, { end: false });
    child.stderr?.pipe(this.log, { end: false });
    this.process = child;
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
      if (child.exitCode !== null) throw new Error(`QA backend exited: ${child.exitCode}; inspect private log locally`);
      try {
        const response = await fetch(`${this.base}/api/v1/health/ready`);
        if (response.ok) return;
      } catch { /* wait for listening without copying raw logs */ }
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    throw new Error("QA backend readiness timeout; inspect private log locally");
  }

  async stop(): Promise<void> {
    const child = this.process;
    this.process = null;
    if (child && child.exitCode === null && child.signalCode === null) {
      await new Promise<void>((resolve) => {
        const timeout = setTimeout(() => child.kill("SIGKILL"), 10_000);
        child.once("exit", () => { clearTimeout(timeout); resolve(); });
        child.kill("SIGTERM");
      });
    }
    if (this.log) await new Promise<void>((resolve) => this.log!.end(resolve));
    this.log = null;
  }

  async restart(): Promise<void> { await this.stop(); await this.start(); }

  logsContainSecret(): boolean {
    const log = fs.existsSync(this.logPath) ? fs.readFileSync(this.logPath, "utf8") : "";
    return log.includes(this.masterKey)
      || [this.deployment, this.alternative].some((fixture) => Object.values(fixture.keys).some((key) => log.includes(key)));
  }

  async cleanup(): Promise<void> {
    await this.stop();
    fs.rmSync(this.workDir, { recursive: true, force: true });
  }
}

export async function attachQaBackend(page: Page, backend: ApiSettingsQaBackend) {
  const externalHosts: string[] = [];
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (!["http:", "https:"].includes(url.protocol)) { await route.continue(); return; }
    if (!["127.0.0.1", "localhost"].includes(url.hostname)) {
      externalHosts.push(url.hostname);
      await route.abort();
      return;
    }
    const target = url.origin === QA_WEB_BASE && url.pathname.startsWith("/api/v1/")
      ? `${backend.base}${url.pathname}${url.search}` : url.href;
    await route.continue({ url: target });
  });
  return externalHosts;
}

/** 上传、准备及报价均走既有真实 HTTP。可以停在未消费报价以测配置切换。 */
export async function readyQuote(request: APIRequestContext, backend: ApiSettingsQaBackend, preset: 0 | 1) {
  const seed = await seedItemWithDocument(request, backend.base, QA_PASSWORD, "sample-manual-text.pdf", "AS QA fixture item");
  const csrf = await apiLogin(request, backend.base, QA_PASSWORD);
  const preparationId = await seedReadyPreparation(request, backend.base, csrf, seed);
  const photos = [
    await seedPhoto(request, backend.base, csrf, seed.itemId, "front", "sample-photo-front.jpg"),
    await seedPhoto(request, backend.base, csrf, seed.itemId, "left", "sample-photo-left.png"),
  ];
  const photoIds = photos.map((photo) => photo.photoId);
  const data: components["schemas"]["EstimateRequest"] = { preparationId, photoIds, modelPreset: `api-settings-qa-${preset}` };
  const response = await request.post(`${backend.base}/api/v1/items/${seed.itemId}/estimates`, { headers: { "x-csrf-token": csrf }, data });
  expect(response.status()).toBe(201);
  const estimate = await response.json() as components["schemas"]["QuoteResponse"];
  const confirmed = await request.post(`${backend.base}/api/v1/items/${seed.itemId}/estimates/${estimate.data.id}/confirm`, { headers: { "x-csrf-token": csrf } });
  expect(confirmed.status()).toBe(200);
  const jobBody: components["schemas"]["JobCreateRequest"] = { quoteId: estimate.data.id, preparationId, photoIds, limits: JOB_LIMITS };
  return { ...seed, csrf, jobBody, estimateId: estimate.data.id };
}

export function writeQaEvidence(name: string, evidence: unknown): void {
  fs.mkdirSync(QA_DIR, { recursive: true });
  fs.writeFileSync(path.join(QA_DIR, `${name}.json`), JSON.stringify(evidence, null, 2));
}
