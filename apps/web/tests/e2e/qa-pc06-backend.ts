import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { expect, type APIRequestContext, type Page } from "@playwright/test";
import { freePort, CheckedProviderFixture } from "./api-settings-qa-harness";
import { apiLogin, seedItemWithDocument, seedPhoto, seedReadyPreparation } from "./helpers";
import { JOB_LIMITS } from "./job-recovery-harness";
import { REPO_ROOT } from "./runtime";

export const WEB = "http://127.0.0.1:15476";
export const PASSWORD = "pc06-independent-qa-password";
export const OUT = path.join(REPO_ROOT, "var/pc06-qa-round2/evidence");
const BINARY = process.env.EM_PC06_QA_BINARY ?? path.join(REPO_ROOT, "var/pc06-qa-round2/everything-manual-fixture");
const PYTHON = "/Users/qsyj/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3";
export const GOOD = { tripo: "v3.1-20260211", manualAi: "gpt-5-mini" };
export type Provider = "tripo" | "manualAi";
export const names: Provider[] = ["tripo", "manualAi"];
export function canary() { return "sk-" + randomBytes(16).toString("hex"); }
export function evidence(name: string, value: unknown) { fs.mkdirSync(OUT, { recursive: true }); fs.writeFileSync(path.join(OUT, name + ".json"), JSON.stringify(value, null, 2) + "\n"); }

/** Only this class's new temp directory and explicit fake environment are used. */
export class Pc06QaBackend {
  readonly workDir = fs.mkdtempSync(path.join(os.tmpdir(), "em-pc06-independent-"));
  readonly dataDir = path.join(this.workDir, "data");
  readonly overlay = path.join(this.dataDir, "provider-overrides.json");
  readonly logPath = path.join(this.workDir, "server.log");
  readonly masterKey = randomBytes(32).toString("hex");
  readonly fixture = new CheckedProviderFixture({ ...GOOD });
  readonly markers: string[] = [];
  models = { ...GOOD };
  source: "env" | "toml" = "env";
  base = "";
  private port = 0;
  private initialized = false;
  private child: ChildProcess | null = null;
  private log: fs.WriteStream | null = null;

  private environment(): NodeJS.ProcessEnv {
    return { PATH: process.env.PATH, TMPDIR: process.env.TMPDIR, EM_SECRETS_MASTER_KEY: this.masterKey,
      EM_PC06_QA_TRIPO: this.fixture.keys.tripo, EM_PC06_QA_MANUAL: this.fixture.keys.manualAi,
      ...(this.source === "env" ? { EM_PROVIDERS__TRIPO__MODEL: this.models.tripo, EM_PROVIDERS__MANUAL_AI__MODEL: this.models.manualAi } : {}) };
  }
  config() {
    const catalog = path.join(this.workDir, "price-catalog.toml");
    if (!fs.existsSync(catalog)) fs.copyFileSync(path.join(REPO_ROOT, "price-catalog.example.toml"), catalog);
    fs.writeFileSync(path.join(this.workDir, "config.toml"), [
      `price_catalog_path = ${JSON.stringify(catalog)}`, `public_origin = ${JSON.stringify(WEB)}`,
      "[providers.tripo]", `base_url = ${JSON.stringify(this.fixture.base + "/v3")}`, `model = ${JSON.stringify(this.source === "toml" ? this.models.tripo : GOOD.tripo)}`, 'api_key_env = "EM_PC06_QA_TRIPO"',
      "[providers.manual_ai]", `base_url = ${JSON.stringify(this.fixture.base + "/v1")}`, `model = ${JSON.stringify(this.source === "toml" ? this.models.manualAi : GOOD.manualAi)}`, 'api_key_env = "EM_PC06_QA_MANUAL"',
      "[download]", 'allowed_hosts = ["127.0.0.1"]', "allow_local_fixture = true", "",
    ].join("\n"), { mode: 0o600 });
  }
  command(args: string[]) {
    const result = spawnSync(BINARY, args, { cwd: this.workDir, env: this.environment(), stdio: "pipe", timeout: 30000 });
    return { code: result.status, output: Buffer.concat([result.stdout ?? Buffer.alloc(0), result.stderr ?? Buffer.alloc(0)]).toString() };
  }
  async start() {
    if (!this.initialized) {
      await this.fixture.start(); this.port = await freePort(); this.base = `http://127.0.0.1:${this.port}`;
      this.config(); const password = path.join(this.workDir, "password.txt");
      fs.writeFileSync(password, PASSWORD + "\n", { mode: 0o600 });
      const initialized = this.command(["init", "--no-sample", "--data-dir", this.dataDir, "--password-file", password]);
      expect(initialized.code, "isolated init exit status only").toBe(0); this.noLeak(initialized.output);
      this.initialized = true;
    }
    this.log = fs.createWriteStream(this.logPath, { flags: "a", mode: 0o600 });
    this.child = spawn(BINARY, ["serve", "--data-dir", this.dataDir, "--listen", `127.0.0.1:${this.port}`], { cwd: this.workDir, env: this.environment(), stdio: ["ignore", "pipe", "pipe"] });
    this.child.stdout?.pipe(this.log, { end: false }); this.child.stderr?.pipe(this.log, { end: false });
    for (let tries = 0; tries < 150; tries++) {
      if (this.child.exitCode !== null) throw new Error("Isolated PC06 backend exited; no raw output attached");
      try { if ((await fetch(this.base + "/api/v1/health/ready")).ok) return; } catch { /* startup */ }
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error("Isolated PC06 backend readiness timeout");
  }
  async stop() {
    const child = this.child; this.child = null;
    if (child && child.exitCode === null && child.signalCode === null) await new Promise<void>(resolve => {
      const timer = setTimeout(() => child.kill("SIGKILL"), 8000); child.once("exit", () => { clearTimeout(timer); resolve(); }); child.kill("SIGTERM");
    });
    if (this.log) await new Promise<void>(resolve => this.log!.end(resolve)); this.log = null;
  }
  async restart() { await this.stop(); this.config(); await this.start(); }
  noLeak(value: unknown) {
    const text = typeof value === "string" ? value : JSON.stringify(value);
    const markers = [...this.markers, this.masterKey, ...Object.values(this.fixture.keys)];
    expect(markers.some(marker => text.includes(marker)), "private marker leak boolean").toBe(false);
  }
  counts() { return { ...this.fixture.upstream.counts }; }
  db(sql: string, params: unknown[] = [], mutate = false): unknown {
    const script = `import sqlite3,json,sys\np=json.load(sys.stdin)\nc=sqlite3.connect(p['db'])\nc.row_factory=sqlite3.Row\ntriggers=[]\nif p['mutate']:\n triggers=c.execute("select name,sql from sqlite_master where type='trigger' and tbl_name in ('quotes','generation_snapshots')").fetchall()\n for t in triggers:c.execute('drop trigger "'+t['name'].replace('"','""')+'"')\nr=c.execute(p['sql'],p['params'])\nrows=[dict(v) for v in r.fetchall()]\nfor t in triggers:c.execute(t['sql'])\nc.commit()\nprint(json.dumps(rows))`;
    const result = spawnSync(PYTHON, ["-c", script], { input: JSON.stringify({ db: path.join(this.dataDir, "manual.sqlite3"), sql, params, mutate }), encoding: "utf8", env: { PATH: process.env.PATH }, timeout: 10000 });
    if (result.status !== 0) throw new Error("Private QA database operation failed; no raw SQL/values emitted");
    return JSON.parse(result.stdout);
  }
  async cleanup() { await this.stop(); if (fs.existsSync(this.logPath)) this.noLeak(fs.readFileSync(this.logPath, "utf8")); await this.fixture.stop(); fs.rmSync(this.workDir, { recursive: true, force: true }); }
}

export async function settings(api: APIRequestContext, b: Pc06QaBackend) {
  const r = await api.get(b.base + "/api/v1/settings/providers"); expect(r.status()).toBe(200);
  const json = await r.json(); b.noLeak(json); return json.data;
}
export function body(view: { revision: string; saved: Record<Provider, { baseUrl: string; model: string | null }> }) {
  const edit = (name: Provider) => ({ action: "update", baseUrl: view.saved[name].baseUrl, model: view.saved[name].model, keyAction: "keep" });
  return { revision: view.revision, tripo: edit("tripo"), manualAi: edit("manualAi") };
}
export async function put(api: APIRequestContext, b: Pc06QaBackend, csrf: string, data: unknown) {
  const r = await api.put(b.base + "/api/v1/settings/providers", { headers: { "x-csrf-token": csrf }, data }); b.noLeak(await r.text()); return r;
}
export async function attach(page: Page, b: Pc06QaBackend) {
  const state = { external: 0, pageErrors: 0, consoleLeak: false, writes: 0 };
  page.on("pageerror", () => state.pageErrors++);
  page.on("console", message => { state.consoleLeak ||= b.markers.some(value => message.text().includes(value)); });
  page.on("request", r => { if (r.method() === "PUT" && r.url().includes("/settings/providers")) state.writes++; });
  await page.route("**/*", async route => {
    const url = new URL(route.request().url());
    if (["http:", "https:"].includes(url.protocol) && !["127.0.0.1", "localhost"].includes(url.hostname)) { state.external++; await route.abort(); return; }
    await route.continue({ url: url.origin === WEB && url.pathname.startsWith("/api/v1/") ? b.base + url.pathname + url.search : url.href });
  });
  return state;
}
export async function quote(api: APIRequestContext, b: Pc06QaBackend) {
  const seed = await seedItemWithDocument(api, b.base, PASSWORD, "sample-manual-text.pdf", "PC06 independent fixture");
  const csrf = await apiLogin(api, b.base, PASSWORD);
  const preparationId = await seedReadyPreparation(api, b.base, csrf, seed);
  const photoIds = (await Promise.all([seedPhoto(api, b.base, csrf, seed.itemId, "front", "sample-photo-front.jpg"), seedPhoto(api, b.base, csrf, seed.itemId, "left", "sample-photo-left.png")])).map(photo => photo.photoId);
  const response = await api.post(`${b.base}/api/v1/items/${seed.itemId}/estimates`, { headers: { "x-csrf-token": csrf }, data: { preparationId, photoIds, modelPreset: "tripo-h-v3.1-standard" } });
  expect(response.status()).toBe(201); const payload = (await response.json()).data;
  return { ...seed, csrf, preparationId, photoIds, payload, quoteId: payload.id as string, jobBody: { quoteId: payload.id, preparationId, photoIds, limits: JOB_LIMITS } };
}
