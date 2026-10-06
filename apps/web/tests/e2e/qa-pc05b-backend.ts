/** Standalone PC05B independent QA: explicit copied binary + SHA, never Cargo or user preview. */
import { createHash, randomBytes } from "node:crypto";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { expect, type Page } from "@playwright/test";
import { LocalFixture } from "./job-recovery-harness";
import { REPO_ROOT } from "./runtime";
export const WEB = "http://127.0.0.1:15490", PASSWORD = "PC05B-independent-local-only";
const outputRoot = path.resolve(process.env.EM_PC05B_QA_OUTPUT_DIR ?? path.join(REPO_ROOT, "var/pc05b-qa-round12"));
if (!outputRoot.startsWith(path.join(REPO_ROOT, "var") + path.sep)) throw new Error("PC05B evidence must remain under owned repo/var");
export const OUT = path.join(outputRoot, "evidence");
export const PYTHON = "/Users/qsyj/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3";
export type Row = Record<string, unknown>;
export function digest(bytes: Buffer | string) { return createHash("sha256").update(bytes).digest("hex"); }
export function evidence(name: string, value: unknown) { fs.mkdirSync(OUT, { recursive: true }); fs.writeFileSync(path.join(OUT, name + ".json"), JSON.stringify(value, null, 2) + "\n"); }
async function freePort(): Promise<number> { return new Promise((resolve, reject) => { const s = net.createServer(); s.on("error", reject); s.listen(0, "127.0.0.1", () => { const a = s.address(); if (!a || typeof a === "string") { reject(new Error("No loopback port")); return; } s.close(() => resolve(a.port)); }); }); }
function binary() {
  const file = process.env.EM_PC05B_QA_BINARY, sha = process.env.EM_PC05B_QA_BINARY_SHA256;
  if (!file || !sha || !/^[a-f0-9]{64}$/.test(sha)) throw new Error("PC05B root-frozen binary and SHA are mandatory");
  if (!fs.realpathSync(file).startsWith(path.join(REPO_ROOT, "var") + path.sep)) throw new Error("PC05B refuses target/current binary fallback");
  expect(digest(fs.readFileSync(file))).toBe(sha); return file;
}
function verifyWebFreeze() {
  const root = process.env.EM_PC05B_QA_WEB_ROOT, expected = process.env.EM_PC05B_QA_WEB_MANIFEST_SHA256;
  if (!root || !expected) throw new Error("Missing PC05B web freeze for final verification");
  const manifest = fs.readFileSync(path.join(root, "source-hashes.json")); expect(digest(manifest)).toBe(expected);
  for (const [file, sha] of Object.entries(JSON.parse(manifest.toString()) as Record<string, string>)) expect(digest(fs.readFileSync(path.join(root, file))), `unchanged frozen ${file}`).toBe(sha);
}
export class Pc05bBackend {
  readonly workDir = fs.mkdtempSync(path.join(os.tmpdir(), "em-pc05b-independent-"));
  readonly dataDir = path.join(this.workDir, "data"); readonly fixture = new LocalFixture(); readonly master = randomBytes(32).toString("hex");
  base = ""; private child: ChildProcess | null = null; private log: fs.WriteStream | null = null;
  private env(): NodeJS.ProcessEnv { return { PATH: process.env.PATH, TMPDIR: process.env.TMPDIR, EM_SECRETS_MASTER_KEY: this.master, EM_PC05B_QA_TRIPO: "pc05b-local-fake-tripo", EM_PC05B_QA_MANUAL: "pc05b-local-fake-manual" }; }
  async start() {
    const executable = binary(); await this.fixture.start(); const port = await freePort(); this.base = `http://127.0.0.1:${port}`;
    const pw = path.join(this.workDir, "password.txt"); fs.writeFileSync(pw, PASSWORD + "\n", { mode: 0o600 });
    const init = spawnSync(executable, ["init", "--no-sample", "--data-dir", this.dataDir, "--password-file", pw], { cwd: this.workDir, env: this.env(), stdio: "pipe", timeout: 30000 }); expect(init.status, "owned fixture init; raw log withheld").toBe(0);
    const catalog = path.join(this.dataDir, "price-catalog.toml"); fs.copyFileSync(path.join(REPO_ROOT, "price-catalog.example.toml"), catalog);
    const config = path.join(this.dataDir, "config.toml"); fs.writeFileSync(config, [`public_origin = "${WEB}"`, `price_catalog_path = ${JSON.stringify(catalog)}`, "[providers.tripo]", `base_url = "${this.fixture.base}/v3"`, 'api_key_env = "EM_PC05B_QA_TRIPO"', "[providers.manual_ai]", `base_url = "${this.fixture.base}/v1"`, 'api_key_env = "EM_PC05B_QA_MANUAL"', 'model = "gpt-5-mini"', "[download]", 'allowed_hosts = ["127.0.0.1"]', "allow_local_fixture = true", ""].join("\n"), { mode: 0o600 });
    this.log = fs.createWriteStream(path.join(this.workDir, "server.log"), { mode: 0o600 }); this.child = spawn(executable, ["serve", "--data-dir", this.dataDir, "--config", config, "--listen", `127.0.0.1:${port}`], { cwd: this.workDir, env: this.env(), stdio: ["ignore", "pipe", "pipe"] });
    this.child.stdout?.pipe(this.log, { end: false }); this.child.stderr?.pipe(this.log, { end: false });
    await expect.poll(async () => { if (this.child?.exitCode !== null) return false; try { return (await fetch(this.base + "/api/v1/health/ready")).ok; } catch { return false; } }, { message: "isolated backend ready", timeout: 20000 }).toBe(true);
  }
  db(sql: string, params: unknown[] = []): Row[] {
    const script = "import sqlite3,json,sys\np=json.load(sys.stdin);c=sqlite3.connect(p['db']);c.row_factory=sqlite3.Row\nr=c.execute(p['sql'],p['params']);v=[dict(x) for x in r.fetchall()];c.commit();print(json.dumps(v))";
    const r = spawnSync(PYTHON, ["-c", script], { input: JSON.stringify({ db: path.join(this.dataDir, "manual.sqlite3"), sql, params }), encoding: "utf8", env: { PATH: process.env.PATH }, timeout: 10000 }); if (r.status !== 0) throw new Error("Owned QA DB operation failed; raw rows withheld"); return JSON.parse(r.stdout) as Row[];
  }
  snapshot() {
    const script = "import sqlite3,json,hashlib,sys\nc=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True)\nt=[r[0] for r in c.execute(\"SELECT name FROM sqlite_master WHERE type='table' ORDER BY name\")]\nr={n:c.execute('SELECT * FROM '+chr(34)+n.replace(chr(34),chr(34)*2)+chr(34)).fetchall() for n in t}\nb=json.dumps(r,sort_keys=True,default=lambda v:v.hex(),separators=(',',':')).encode();print(json.dumps({'sha256':hashlib.sha256(b).hexdigest(),'counts':{n:len(v) for n,v in r.items()}}))";
    const r = spawnSync(PYTHON, ["-c", script, path.join(this.dataDir, "manual.sqlite3")], { encoding: "utf8", env: { PATH: process.env.PATH }, timeout: 10000 }); if (r.status !== 0) throw new Error("Owned read-only digest failed"); return JSON.parse(r.stdout) as { sha256: string; counts: Record<string, number> };
  }
  facts() { return { db: this.db("SELECT (SELECT count(*) FROM jobs) AS jobs,(SELECT count(*) FROM provider_attempts) AS attempts,(SELECT count(*) FROM cost_ledger) AS ledger,(SELECT count(*) FROM manual_releases) AS releases"), wire: { ...this.fixture.counts } }; }
  async cleanup() {
    if (this.child && this.child.exitCode === null && this.child.signalCode === null) { const child = this.child; await new Promise<void>(resolve => { const timer = setTimeout(() => child.kill("SIGKILL"), 8000); child.once("exit", () => { clearTimeout(timer); resolve(); }); child.kill("SIGTERM"); }); }
    if (this.log) await new Promise<void>(resolve => this.log!.end(resolve)); await this.fixture.stop(); binary(); verifyWebFreeze(); fs.rmSync(this.workDir, { recursive: true, force: true });
  }
}
export async function attach(page: Page, b: Pc05bBackend) {
  const state = { external: 0, pageErrors: 0, writes: [] as string[], reads: [] as string[] };
  page.on("pageerror", () => state.pageErrors++); page.on("request", r => { const u = new URL(r.url()); if (!u.pathname.startsWith("/api/v1/")) return; if (r.method() === "GET") state.reads.push(u.pathname + u.search); else state.writes.push(r.method() + " " + u.pathname); });
  await page.route("**/*", async r => { const u = new URL(r.request().url()); if (["http:", "https:"].includes(u.protocol) && !["localhost", "127.0.0.1"].includes(u.hostname)) { state.external++; await r.abort(); return; } await r.continue({ url: u.origin === WEB && u.pathname.startsWith("/api/v1/") ? b.base + u.pathname + u.search : u.href }); }); return state;
}
