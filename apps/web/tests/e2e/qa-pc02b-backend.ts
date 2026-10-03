/** PC02B independent QA infrastructure only. The root must provide the frozen binary before execution. */
import { createHash, randomBytes } from "node:crypto";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { expect, type Page } from "@playwright/test";
import { LocalFixture } from "./job-recovery-harness";
import { REPO_ROOT } from "./runtime";
export const WEB = `http://127.0.0.1:${process.env.EM_PC02B_QA_WEB_PORT ?? "15486"}`;
export const OUT = process.env.EM_PC02B_QA_OUTPUT_DIR ?? path.join(REPO_ROOT, "var/pc02b-qa-round6", "evidence");
export const PASSWORD = "PC02B-independent-local-only";
const BINARY = path.join(REPO_ROOT, "var/pc03a-qa-round4/everything-manual-fixture");
function checkedBinary() {
  const override = process.env.EM_PC02B_QA_BINARY;
  if (process.env.EM_PC05B_QA_WEB_ROOT && !override) throw new Error("PC05B affected PC02B run requires explicit copied binary");
  const file = override ?? BINARY, sha = override ? process.env.EM_PC02B_QA_BINARY_SHA256 : "d7ec4440e17ba35cc33eb6b6e6f52325d304e63b4daea3df20e529edce9984f7";
  if (!sha || !/^[a-f0-9]{64}$/.test(sha)) throw new Error("PC02B copied binary SHA required");
  if (!fs.realpathSync(file).startsWith(path.join(REPO_ROOT, "var") + path.sep)) throw new Error("PC02B refuses target binary");
  expect(digest(fs.readFileSync(file))).toBe(sha); return file;
}
const PYTHON = "/Users/qsyj/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3";
async function freePort(): Promise<number> { return new Promise((resolve, reject) => { const server = net.createServer(); server.on("error", reject); server.listen(0, "127.0.0.1", () => { const address = server.address(); if (!address || typeof address === "string") { reject(new Error("No local port")); return; } server.close(() => resolve(address.port)); }); }); }

export class Pc02bQaBackend {
  readonly workDir = fs.mkdtempSync(path.join(os.tmpdir(), "em-pc02b-independent-"));
  readonly dataDir = path.join(this.workDir, "data");
  readonly fixture = new LocalFixture();
  readonly master = randomBytes(32).toString("hex");
  readonly logPath = path.join(this.workDir, "server.log");
  base = "";
  private child: ChildProcess | null = null;
  private log: fs.WriteStream | null = null;
  private env(): NodeJS.ProcessEnv { return { PATH: process.env.PATH, TMPDIR: process.env.TMPDIR, EM_SECRETS_MASTER_KEY: this.master, EM_PC02B_QA_TRIPO: "pc02b-local-fake-tripo", EM_PC02B_QA_MANUAL: "pc02b-local-fake-manual" }; }
  async start() {
    // No fallback to mutable target/debug and no build side effects.
    const executable = checkedBinary();
    await this.fixture.start(); const port = await freePort(); this.base = `http://127.0.0.1:${port}`;
    const catalog = path.join(this.workDir, "price-catalog.toml"); fs.copyFileSync(path.join(REPO_ROOT, "price-catalog.example.toml"), catalog);
    fs.writeFileSync(path.join(this.workDir, "config.toml"), [`public_origin = "${WEB}"`, `price_catalog_path = ${JSON.stringify(catalog)}`, "[providers.tripo]", `base_url = "${this.fixture.base}/v3"`, 'api_key_env = "EM_PC02B_QA_TRIPO"', "[providers.manual_ai]", `base_url = "${this.fixture.base}/v1"`, 'api_key_env = "EM_PC02B_QA_MANUAL"', 'model = "gpt-5-mini"', '[download]', 'allowed_hosts = ["127.0.0.1"]', 'allow_local_fixture = true', ""].join("\n"), { mode: 0o600 });
    const password = path.join(this.workDir, "password.txt"); fs.writeFileSync(password, PASSWORD + "\n", { mode: 0o600 });
    const init = spawnSync(executable, ["init", "--data-dir", this.dataDir, "--password-file", password], { cwd: this.workDir, env: this.env(), stdio: "pipe", timeout: 30000 }); expect(init.status, "private init status").toBe(0);
    this.log = fs.createWriteStream(this.logPath, { mode: 0o600 }); this.child = spawn(executable, ["serve", "--data-dir", this.dataDir, "--listen", `127.0.0.1:${port}`], { cwd: this.workDir, env: this.env(), stdio: ["ignore", "pipe", "pipe"] });
    this.child.stdout?.pipe(this.log, { end: false }); this.child.stderr?.pipe(this.log, { end: false });
    for (let i = 0; i < 150; i++) { if (this.child.exitCode !== null) throw new Error("Private PC02B backend exited; no raw log emitted"); try { if ((await fetch(this.base + "/api/v1/health/ready")).ok) return; } catch { /* startup */ } await new Promise(resolve => setTimeout(resolve, 100)); }
    throw new Error("Private PC02B backend readiness timeout");
  }
  db(sql: string, params: unknown[] = []) {
    const script = "import sqlite3,json,sys\np=json.load(sys.stdin)\nc=sqlite3.connect(p['db'])\nc.row_factory=sqlite3.Row\nr=c.execute(p['sql'],p['params'])\nrows=[dict(v) for v in r.fetchall()]\nc.commit()\nprint(json.dumps(rows))";
    const result = spawnSync(PYTHON, ["-c", script], { input: JSON.stringify({ db: path.join(this.dataDir, "manual.sqlite3"), sql, params }), encoding: "utf8", env: { PATH: process.env.PATH }, timeout: 10000 });
    if (result.status !== 0) throw new Error("Private QA SQL failed; no raw rows emitted"); return JSON.parse(result.stdout);
  }
  /** All persisted logical rows, including sessions/audit: return only digest and counts. */
  snapshot() {
    const script = "import sqlite3,json,sys,hashlib\nc=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True)\ntables=[x[0] for x in c.execute(\"SELECT name FROM sqlite_master WHERE type='table' ORDER BY name\")]\nrows={t:c.execute('SELECT * FROM '+chr(34)+t.replace(chr(34),chr(34)*2)+chr(34)).fetchall() for t in tables}\npayload=json.dumps(rows,sort_keys=True,default=lambda b:b.hex(),separators=(',',':')).encode()\nprint(json.dumps({'sha256':hashlib.sha256(payload).hexdigest(),'counts':{t:len(v) for t,v in rows.items()}}))";
    const result = spawnSync(PYTHON, ["-c", script, path.join(this.dataDir, "manual.sqlite3")], { encoding: "utf8", env: { PATH: process.env.PATH }, timeout: 10000 });
    if (result.status !== 0) throw new Error("Private QA read-only snapshot failed"); return JSON.parse(result.stdout) as { sha256: string; counts: Record<string, number> };
  }
  async cleanup() {
    if (this.child && this.child.exitCode === null && this.child.signalCode === null) await new Promise<void>(resolve => { const timer = setTimeout(() => this.child?.kill("SIGKILL"), 8000); this.child!.once("exit", () => { clearTimeout(timer); resolve(); }); this.child!.kill("SIGTERM"); });
    if (this.log) await new Promise<void>(resolve => this.log!.end(resolve)); await this.fixture.stop(); checkedBinary(); fs.rmSync(this.workDir, { recursive: true, force: true });
  }
}
export function saveEvidence(name: string, value: unknown) { fs.mkdirSync(OUT, { recursive: true }); fs.writeFileSync(path.join(OUT, name + ".json"), JSON.stringify(value, null, 2) + "\n"); }
export function digest(bytes: Buffer) { return createHash("sha256").update(bytes).digest("hex"); }
export async function attach(page: Page, backend: Pc02bQaBackend) {
  const state = { external: 0, pageErrors: 0, pagePuts: [] as number[], writes: [] as string[], assetReads: 0 };
  page.on("pageerror", () => state.pageErrors++);
  page.on("request", request => { const u = new URL(request.url()); if (u.pathname.startsWith("/api/v1/") && !["GET", "HEAD"].includes(request.method())) state.writes.push(request.method() + " " + u.pathname); const put = /\/preparations\/[^/]+\/pages\/(\d+)$/.exec(u.pathname); if (request.method() === "PUT" && put) state.pagePuts.push(Number(put[1])); if (/\/assets\/[^/]+\/content$/.test(u.pathname)) state.assetReads++; });
  await page.route("**/*", async entry => { const u = new URL(entry.request().url()); if (["http:", "https:"].includes(u.protocol) && !["localhost", "127.0.0.1"].includes(u.hostname)) { state.external++; await entry.abort(); return; } await entry.continue({ url: u.origin === WEB && u.pathname.startsWith("/api/v1/") ? backend.base + u.pathname + u.search : u.href }); }); return state;
}
