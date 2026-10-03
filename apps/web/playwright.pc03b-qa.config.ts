import path from "node:path";
import { defineConfig, devices } from "@playwright/test";
process.env.PLAYWRIGHT_NO_COPY_PROMPT = "1";
process.env.EM_E2E_WEB_PORT = "15487";
const root = path.resolve(import.meta.dirname, "../..");
const snapshot = process.env.EM_PC03B_QA_WEB_ROOT ?? path.join(root, "var/prd-completion/web-snapshots/pc03b-rd-ready");
if (!path.resolve(snapshot).startsWith(path.join(root, "var/prd-completion/web-snapshots") + path.sep)) throw new Error("PC03B QA requires an explicit frozen snapshot");
export default defineConfig({
  testDir: "./tests/e2e", testMatch: "qa-pc03b-*.spec.ts", workers: 1, fullyParallel: false,
  forbidOnly: true, retries: 0, timeout: 120000, expect: { timeout: 15000 },
  outputDir: path.join(process.env.EM_PC03B_QA_OUTPUT ?? path.join(root, "var/pc03b-qa-round7"), "browser"), reporter: [["list"]],
  use: { baseURL: "http://127.0.0.1:15487", trace: "off", video: "off", screenshot: "off", locale: "zh-CN" },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: { command: "npm run dev", cwd: snapshot,
    url: "http://127.0.0.1:15487", reuseExistingServer: false, timeout: 120000,
    env: { EM_WEB_PORT: "15487", EM_API_PROXY_TARGET: "http://127.0.0.1:18487" }, stdout: "pipe", stderr: "pipe" },
});
