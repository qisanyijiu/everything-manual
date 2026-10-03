import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { BACKEND_PASSWORD, LocalFixture, TestBackend } from "./job-recovery-harness";
import { E2E_WEB_PORT, REPO_ROOT } from "./runtime";

/** QA uses a frozen binary copy, never builds or starts the user's preview. */
export class Pc01QaBackend extends TestBackend {
  private qaProcess: ChildProcess | null = null;

  override async start(fixture: LocalFixture): Promise<void> {
    const binary = path.join(REPO_ROOT, "var/pc01-qa-round1/everything-manual-fixture");
    if (!fs.existsSync(binary)) throw new Error("Copy the RD_READY fixture binary before QA");
    this.port = await new Promise<number>((resolve, reject) => {
      const server = net.createServer();
      server.once("error", reject);
      server.listen(0, "127.0.0.1", () => {
        const address = server.address();
        if (address === null || typeof address === "string") return reject(new Error("No isolated port"));
        server.close(() => resolve(address.port));
      });
    });
    const passwordFile = path.join(this.workDir, "password.txt");
    fs.writeFileSync(passwordFile, BACKEND_PASSWORD + "\n", { mode: 0o600 });
    const catalog = path.join(this.workDir, "prices.toml");
    fs.copyFileSync(path.join(REPO_ROOT, "price-catalog.example.toml"), catalog);
    fs.writeFileSync(path.join(this.workDir, "config.toml"), [
      `price_catalog_path = ${JSON.stringify(catalog)}`,
      `public_origin = "http://127.0.0.1:${E2E_WEB_PORT}"`,
      "[providers.tripo]", `base_url = "${fixture.base}/v3"`, 'api_key_env = "EM_T17_TRIPO_KEY"',
      "[providers.manual_ai]", `base_url = "${fixture.base}/v1"`, 'model = "gpt-5-mini"', 'api_key_env = "EM_T17_MANUAL_AI_KEY"',
      "[download]", 'allowed_hosts = ["127.0.0.1"]', "allow_local_fixture = true", "",
    ].join("\n"));
    execFileSync(binary, ["init", "--data-dir", this.dataDir, "--password-file", passwordFile], { stdio: "ignore" });
    const log = fs.createWriteStream(this.logPath, { flags: "a" });
    this.qaProcess = spawn(binary, ["serve", "--data-dir", this.dataDir, "--listen", `127.0.0.1:${this.port}`], {
      cwd: this.workDir,
      env: { ...process.env, EM_T17_TRIPO_KEY: "t17-fake-tripo-key", EM_T17_MANUAL_AI_KEY: "t17-fake-manual-ai-key" },
    });
    this.qaProcess.stdout?.pipe(log);
    this.qaProcess.stderr?.pipe(log);
    this.base = `http://127.0.0.1:${this.port}`;
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
      try { if ((await fetch(`${this.base}/api/v1/health/ready`)).ok) return; } catch { /* starting */ }
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error("Isolated QA backend failed to become ready; inspect only its temporary log");
  }

  override async stop(): Promise<void> {
    const child = this.qaProcess;
    if (child === null) return;
    this.qaProcess = null;
    await new Promise<void>(resolve => {
      const timer = setTimeout(() => { child.kill("SIGKILL"); resolve(); }, 5000);
      child.once("exit", () => { clearTimeout(timer); resolve(); });
      child.kill("SIGTERM");
    });
  }
}
