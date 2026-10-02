/** Independent PC04 CLI QA. Execution requires root-supplied frozen binaries and explicit GO. */
import { createHash, randomBytes, randomUUID } from "node:crypto";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { expect, type APIRequestContext } from "@playwright/test";
import { apiLogin, fetchPhotos, seedItemWithDocument, seedPhoto, seedReadyPreparation } from "./helpers";
import { LocalFixture, MODEL_PRESET } from "./job-recovery-harness";
import { REPO_ROOT } from "./runtime";

export const OUT = path.join(REPO_ROOT, "var/pc04-qa-round9/evidence");
const PYTHON = "/Users/qsyj/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3";
export const SUBMIT = "/v3/generation/multiview-to-model";
export const MANUAL = "/v1/responses";
export const FAKE_CANARY = "sk-proj-PC04_QA_ONLY_NOT_A_REAL_CREDENTIAL_123456789";
export const REMOTE_CANARY = "qa-pc04-opaque-remote-id-canary";
export const SIGNED_CANARY = "qa-pc04-fake-download-signature";
export type Dict = Record<string, unknown>;
export interface CaseFile {
  schemaVersion: 1; caseId: string; mode: "loopbackFixture";
  instance: { instanceId: string; dataDir: string; configFile: string };
  material: { itemId: string; itemModel: string; documentId: string; sourceSha256: string; preparationId: string; photos: { id: string; sha256: string; view: string }[] };
  generation: { modelPreset: string; tripo: { identity: "tripo"; model: string }; manualAi: { identity: "manual_ai"; model: string }; priceVersion: string };
  outputDirectory: string;
}
export interface Plan extends Dict {
  caseHash: string; planHash: string; inputHash: string; requiredLimits: { creditMinor: number; usdMicros: number };
  stages: { stageKind: string; batchIndex: number; stageInputHash: string }[];
  views: Dict[]; providers: Dict; pages: Dict[]; batches: Dict[];
}
export interface BudgetFile {
  schemaVersion: 1; authorizationId: string; caseId: string; caseHash: string; planHash: string;
  allowed: boolean; expiresAt: string; limits: { creditMinor: number; usdMicros: number }; maxInitialGenerations: 1;
  retryScopes: { stageKind: string; batchIndex: number; stageInputHash: string; operation: "safeRetry"; maxAdditionalAttempts: number }[];
}
export interface CliResult { code: number | null; signal: NodeJS.Signals | null; stdout: string; stderr: string }
export function sha(bytes: Buffer | string) { return createHash("sha256").update(bytes).digest("hex"); }
export function object(value: unknown): Dict { if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Expected QA JSON object"); return value as Dict; }
export function outputJson(result: CliResult): Dict {
  const at = result.stdout.indexOf("{\n"); if (at < 0) throw new Error("CLI output has no report JSON; raw output withheld");
  return object(JSON.parse(result.stdout.slice(at)));
}
export function evidence(name: string, value: unknown) { fs.mkdirSync(OUT, { recursive: true }); fs.writeFileSync(path.join(OUT, name + ".json"), JSON.stringify(value, null, 2) + "\n"); }
async function freePort(): Promise<number> { return new Promise((resolve, reject) => { const s = net.createServer(); s.on("error", reject); s.listen(0, "127.0.0.1", () => { const a = s.address(); if (!a || typeof a === "string") { reject(new Error("No loopback port")); return; } s.close(() => resolve(a.port)); }); }); }
function frozenBinary(kind: "SERVER" | "XTASK") {
  const file = process.env[`EM_PC04_QA_${kind}`]; const hash = process.env[`EM_PC04_QA_${kind}_SHA256`];
  if (!file || !hash || !/^[0-9a-f]{64}$/.test(hash)) throw new Error(`Root must supply frozen PC04 ${kind} path and SHA before QA`);
  if (!fs.realpathSync(file).startsWith(path.join(REPO_ROOT, "var") + path.sep)) throw new Error("QA refuses a mutable/build fallback binary");
  expect(sha(fs.readFileSync(file)), "copied binary matches supplied freeze hash").toBe(hash); return file;
}

/** Loopback-only instrumentation; raw request bodies stay in memory, authentication headers are never recorded. */
export class Pc04Proxy {
  readonly fixture = new LocalFixture();
  readonly requests: { method: string; path: string; body: Buffer }[] = [];
  private server: http.Server | null = null;
  private unblock: (() => void) | null = null;
  private holdRoute: string | null = null;
  held = false; dropAcceptedOnce: string | null = null; credits = 30; remoteCanary = false; echoCredential = false;
  base = "";
  async start() {
    await this.fixture.start(); const port = await freePort(); this.base = `http://127.0.0.1:${port}`;
    this.server = http.createServer((req, res) => { void this.handle(req, res).catch(() => res.destroy()); });
    await new Promise<void>(resolve => this.server!.listen(port, "127.0.0.1", resolve));
  }
  holdNext(route: string) { this.holdRoute = route; this.held = false; }
  release() { this.holdRoute = null; this.unblock?.(); this.unblock = null; }
  count(route: string) { return this.requests.filter(r => r.path.startsWith(route)).length; }
  private async handle(req: http.IncomingMessage, res: http.ServerResponse) {
    const chunks: Buffer[] = []; for await (const part of req) chunks.push(part as Buffer);
    const body = Buffer.concat(chunks), route = new URL(req.url ?? "/", this.base).pathname, method = req.method ?? "GET";
    this.requests.push({ method, path: route, body });
    if (this.holdRoute && route.startsWith(this.holdRoute)) { this.holdRoute = null; this.held = true; await new Promise<void>(resolve => { this.unblock = resolve; }); }
    const headers: Record<string, string> = {}; if (req.headers["content-type"]) headers["content-type"] = req.headers["content-type"];
    const upstream = await fetch(this.fixture.base + route, { method, headers, ...(body.length ? { body: new Uint8Array(body) } : {}) });
    let bytes = Buffer.from(await upstream.arrayBuffer());
    if (this.dropAcceptedOnce === route) { this.dropAcceptedOnce = null; res.destroy(); return; }
    if (route.startsWith("/v3/tasks/")) {
      const payload = JSON.parse(bytes.toString()) as { data: { credits_consumed: number; task_id: string; output: { model_url: string } } };
      payload.data.credits_consumed = this.credits; payload.data.output.model_url = this.base + "/cdn/model.glb" + (this.remoteCanary ? "?signature=" + SIGNED_CANARY : "");
      if (this.remoteCanary || this.echoCredential) payload.data.task_id = this.echoCredential ? FAKE_CANARY : REMOTE_CANARY;
      bytes = Buffer.from(JSON.stringify(payload));
    } else if (route === SUBMIT && upstream.ok && (this.remoteCanary || this.echoCredential)) {
      const payload = JSON.parse(bytes.toString()) as { data: { task_id: string } }; payload.data.task_id = this.echoCredential ? FAKE_CANARY : REMOTE_CANARY; bytes = Buffer.from(JSON.stringify(payload));
    }
    res.writeHead(upstream.status, { "content-type": upstream.headers.get("content-type") ?? "application/octet-stream", "content-length": bytes.length }); res.end(bytes);
  }
  async stop() { this.release(); this.server?.closeAllConnections(); await new Promise<void>(resolve => this.server ? this.server.close(() => resolve()) : resolve()); await this.fixture.stop(); }
}

export class Pc04Harness {
  readonly workDir = fs.mkdtempSync(path.join(os.tmpdir(), "qa-pc04-local-only-"));
  readonly dataDir = path.join(this.workDir, "data"); readonly outputDir = path.join(this.workDir, "results");
  readonly casePath = path.join(this.workDir, "case.json"); readonly budgetPath = path.join(this.workDir, "fixture-authorization.json");
  readonly configPath = path.join(this.dataDir, "config.toml"); readonly catalogPath = path.join(this.dataDir, "price-catalog.toml");
  readonly proxy = new Pc04Proxy(); readonly password = "PC04-only-local-fixture-password"; readonly master = randomBytes(32).toString("hex");
  readonly logs: string[] = []; readonly cliResults: CliResult[] = [];
  private server: ChildProcess | null = null; private children = new Set<ChildProcess>();
  private serverBinary = ""; private xtaskBinary = "";
  base = ""; case!: CaseFile; quote!: Dict; missingManualKey = false;
  private env(): NodeJS.ProcessEnv { return { PATH: process.env.PATH, TMPDIR: process.env.TMPDIR, EM_SECRETS_MASTER_KEY: this.master, EM_PC04_QA_TRIPO: FAKE_CANARY, ...(this.missingManualKey ? {} : { EM_PC04_QA_MANUAL: FAKE_CANARY + "_manual" }) }; }
  async setup(api: APIRequestContext) {
    this.serverBinary = frozenBinary("SERVER"); this.xtaskBinary = frozenBinary("XTASK"); await this.proxy.start();
    fs.mkdirSync(this.outputDir, { mode: 0o700 }); const pw = path.join(this.workDir, "password.txt"); fs.writeFileSync(pw, this.password + "\n", { mode: 0o600 });
    const init = spawnSync(this.serverBinary, ["init", "--data-dir", this.dataDir, "--password-file", pw], { cwd: this.workDir, env: this.env(), encoding: "utf8", timeout: 30000 });
    expect(init.status, "private fixture init (raw output withheld)").toBe(0);
    fs.copyFileSync(path.join(REPO_ROOT, "price-catalog.example.toml"), this.catalogPath);
    const port = await freePort(); this.base = `http://127.0.0.1:${port}`;
    fs.writeFileSync(this.configPath, [
      `public_origin = "${this.base}"`, `price_catalog_path = ${JSON.stringify(this.catalogPath)}`,
      "[providers.tripo]", `base_url = "${this.proxy.base}/v3"`, 'api_key_env = "EM_PC04_QA_TRIPO"',
      "[providers.manual_ai]", `base_url = "${this.proxy.base}/v1"`, 'api_key_env = "EM_PC04_QA_MANUAL"', 'model = "gpt-5-mini"',
      "[download]", 'allowed_hosts = ["127.0.0.1"]', "allow_local_fixture = true", "[jobs]", "lease_seconds = 3", "renew_seconds = 1", "",
    ].join("\n"), { mode: 0o600 });
    this.server = spawn(this.serverBinary, ["serve", "--data-dir", this.dataDir, "--config", this.configPath, "--listen", `127.0.0.1:${port}`], { cwd: this.workDir, env: this.env(), stdio: ["ignore", "pipe", "pipe"] });
    this.server.stdout?.on("data", v => this.logs.push(String(v))); this.server.stderr?.on("data", v => this.logs.push(String(v)));
    await expect.poll(async () => { if (this.server?.exitCode !== null) return false; try { return (await fetch(this.base + "/api/v1/health/ready")).ok; } catch { return false; } }, { message: "owned backend ready", timeout: 20000 }).toBe(true);
    const seed = await seedItemWithDocument(api, this.base, this.password, "sample-manual-text.pdf", "QA PC04 local fixture only");
    const csrf = await apiLogin(api, this.base, this.password); const prep = await seedReadyPreparation(api, this.base, csrf, seed);
    await seedPhoto(api, this.base, csrf, seed.itemId, "front", "sample-photo-front.jpg"); await seedPhoto(api, this.base, csrf, seed.itemId, "left", "sample-photo-left.png");
    const photos = await fetchPhotos(api, this.base, seed.itemId); const bound = [];
    for (const p of photos) { const r = await api.get(`${this.base}/api/v1/assets/${p.assetId}/content`); expect(r.status()).toBe(200); bound.push({ id: p.id, sha256: sha(await r.body()), view: p.view }); }
    const r = await api.post(`${this.base}/api/v1/items/${seed.itemId}/estimates`, { headers: { "x-csrf-token": csrf }, data: { preparationId: prep, photoIds: bound.map(p => p.id), modelPreset: MODEL_PRESET } }); expect(r.status()).toBe(201); this.quote = object((await r.json()).data);
    this.case = { schemaVersion: 1, caseId: "qa-pc04-local-only-" + randomUUID(), mode: "loopbackFixture", instance: { instanceId: "qa-pc04-local-only", dataDir: this.dataDir, configFile: this.configPath }, material: { itemId: seed.itemId, itemModel: "T09-sample-manual-text.pdf", documentId: seed.documentId, sourceSha256: seed.sourceSha256, preparationId: prep, photos: bound }, generation: { modelPreset: MODEL_PRESET, tripo: { identity: "tripo", model: "v3.1-20260211" }, manualAi: { identity: "manual_ai", model: "gpt-5-mini" }, priceVersion: "2026-09-11" }, outputDirectory: this.outputDir };
    this.writeCase(); await this.stopServer(); expect(this.proxy.requests.length).toBe(0);
  }
  writeCase(value: unknown = this.case) { fs.writeFileSync(this.casePath, JSON.stringify(value), { mode: 0o600 }); }
  writeBudget(value: unknown, mode = 0o600) { fs.writeFileSync(this.budgetPath, JSON.stringify(value), { mode }); fs.chmodSync(this.budgetPath, mode); }
  budget(plan: Plan): BudgetFile { return { schemaVersion: 1, authorizationId: "qa-pc04-local-only-" + randomUUID(), caseId: this.case.caseId, caseHash: plan.caseHash, planHash: plan.planHash, allowed: true, expiresAt: new Date(Date.now() + 300000).toISOString(), limits: { ...plan.requiredLimits }, maxInitialGenerations: 1, retryScopes: [] }; }
  start(args = ["--case", this.casePath, "--budget-file", this.budgetPath]) {
    const child = spawn(this.xtaskBinary, ["test-live", ...args], { cwd: this.workDir, env: this.env(), stdio: ["ignore", "pipe", "pipe"] }); this.children.add(child);
    let stdout = "", stderr = ""; child.stdout?.on("data", v => { stdout += String(v); }); child.stderr?.on("data", v => { stderr += String(v); });
    const done = new Promise<CliResult>((resolve, reject) => { child.once("error", reject); child.once("exit", (code, signal) => { this.children.delete(child); const result = { code, signal, stdout, stderr }; this.cliResults.push(result); resolve(result); }); });
    return { child, done };
  }
  async cli(args?: string[]) { return this.start(args).done; }
  async plan(): Promise<Plan> { const r = await this.cli(["--plan", "--case", this.casePath]); expect(r.code, "plan exit (raw output withheld)").toBe(0); expect(r.stdout.includes("计划预览 · 未执行")).toBe(true); return outputJson(r) as Plan; }
  db(sql: string, params: unknown[] = []): Dict[] {
    const script = "import sqlite3,json,sys\np=json.load(sys.stdin)\nc=sqlite3.connect(p['db']);c.row_factory=sqlite3.Row\nr=c.execute(p['sql'],p['params']);rows=[dict(x) for x in r.fetchall()];c.commit()\nprint(json.dumps(rows))";
    const r = spawnSync(PYTHON, ["-c", script], { input: JSON.stringify({ db: path.join(this.dataDir, "manual.sqlite3"), sql, params }), encoding: "utf8", env: { PATH: process.env.PATH }, timeout: 10000 });
    if (r.status !== 0) throw new Error("Owned QA DB operation failed; raw rows withheld"); return JSON.parse(r.stdout) as Dict[];
  }
  facts() { return this.db("SELECT (SELECT count(*) FROM jobs) AS jobs,(SELECT count(*) FROM provider_attempts) AS attempts,(SELECT count(*) FROM cost_ledger) AS ledger,(SELECT count(*) FROM manual_releases) AS releases")[0]; }
  snapshot() {
    const script = "import sqlite3,json,hashlib,sys\nc=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True)\nt=[x[0] for x in c.execute(\"SELECT name FROM sqlite_master WHERE type='table' ORDER BY name\")]\nr={n:c.execute('SELECT * FROM '+chr(34)+n.replace(chr(34),chr(34)*2)+chr(34)).fetchall() for n in t}\np=json.dumps(r,sort_keys=True,default=lambda b:b.hex(),separators=(',',':')).encode()\nprint(json.dumps({'sha256':hashlib.sha256(p).hexdigest(),'counts':{n:len(v) for n,v in r.items()}}))";
    const r = spawnSync(PYTHON, ["-c", script, path.join(this.dataDir, "manual.sqlite3")], { encoding: "utf8", env: { PATH: process.env.PATH }, timeout: 10000 }); if (r.status !== 0) throw new Error("QA digest failed"); return JSON.parse(r.stdout) as { sha256: string; counts: Record<string, number> };
  }
  scan(text: string, includePaths = true) {
    const needles = [FAKE_CANARY, REMOTE_CANARY, SIGNED_CANARY, this.password, this.master, ...(includePaths ? [this.workDir, this.dataDir] : [])];
    expect(needles.filter(n => text.includes(n)).length, "known fake-secret/private-path canary hits (values withheld)").toBe(0);
  }
  savedReport() { const dirs = fs.readdirSync(this.outputDir); expect(dirs.length).toBe(1); const dir = path.join(this.outputDir, dirs[0]!); const file = path.join(dir, "report.json"); return { dir, report: object(JSON.parse(fs.readFileSync(file, "utf8"))) }; }
  async stopServer() { if (this.server && this.server.exitCode === null && this.server.signalCode === null) { const s = this.server; await new Promise<void>(resolve => { const timer = setTimeout(() => s.kill("SIGKILL"), 8000); s.once("exit", () => { clearTimeout(timer); resolve(); }); s.kill("SIGTERM"); }); } this.server = null; }
  async cleanup() {
    this.proxy.release(); await Promise.all([...this.children].map(child => new Promise<void>(resolve => { if (child.exitCode !== null || child.signalCode !== null) { resolve(); return; } child.once("exit", () => resolve()); child.kill("SIGKILL"); }))); await this.stopServer(); await this.proxy.stop();
    if (this.serverBinary) frozenBinary("SERVER"); if (this.xtaskBinary) frozenBinary("XTASK"); fs.rmSync(this.workDir, { recursive: true, force: true });
  }
}
