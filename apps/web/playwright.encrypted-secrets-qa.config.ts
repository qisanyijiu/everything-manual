/** ES-01 independent QA. No shared server, globalSetup or historical evidence writes. */
import path from "node:path";
import { defineConfig, devices } from "@playwright/test";

process.env.PLAYWRIGHT_NO_COPY_PROMPT = "1";
const port = Number(process.env.EM_ES_QA_WEB_PORT ?? 15187);
const evidence = path.resolve(import.meta.dirname, "../../artifacts/encrypted-secrets/qa");
// Reuse the established AS restart/worker fixture with a separate preview and evidence root.
process.env.EM_AS_QA_WEB_PORT = String(port);
process.env.EM_AS_QA_ARTIFACT_DIR = path.join(evidence, "as-regression");

export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: ["encrypted-secrets-qa.spec.ts", "api-settings-qa.spec.ts"],
  fullyParallel: false,
  workers: 1,
  forbidOnly: true,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  outputDir: path.join(evidence, "playwright-output"),
  reporter: [["list"]],
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    trace: "off",
    screenshot: "off",
    video: "off",
    locale: "zh-CN",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: "npm run dev",
    cwd: import.meta.dirname,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
    timeout: 120_000,
    env: { EM_WEB_PORT: String(port), EM_API_PROXY_TARGET: "http://127.0.0.1:18187" },
    stdout: "pipe",
    stderr: "pipe",
  },
});
