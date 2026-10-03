import path from "node:path";
import { defineConfig, devices } from "@playwright/test";
process.env.PLAYWRIGHT_NO_COPY_PROMPT = "1";
const root = path.resolve(import.meta.dirname, "../..");
export default defineConfig({
  testDir: "./tests/e2e", testMatch: "qa-pc02a-originals.spec.ts", workers: 1, fullyParallel: false,
  forbidOnly: true, retries: 0, timeout: 120000, expect: { timeout: 15000 },
  outputDir: path.join(root, "var/pc02a-qa-round3/browser"), reporter: [["list"]],
  use: { baseURL: "http://127.0.0.1:15477", trace: "off", video: "off", screenshot: "off", locale: "zh-CN" },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: { command: "npm run dev", cwd: path.join(root, "var/prd-completion/web-snapshots/pc02a-rd-ready"),
    url: "http://127.0.0.1:15477", reuseExistingServer: false, timeout: 120000,
    env: { EM_WEB_PORT: "15477", EM_API_PROXY_TARGET: "http://127.0.0.1:18477" }, stdout: "pipe", stderr: "pipe" },
});
