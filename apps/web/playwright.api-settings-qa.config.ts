/** AS-01 独立 QA：不运行历史 globalSetup，不覆盖 web-mvp 日志。 */
import path from "node:path";

import { defineConfig, devices } from "@playwright/test";

// Installed Playwright lib/index.js::_takePageSnapshot reads this guard before
// writing failure error-context.md; password accessibility values are sensitive too.
process.env.PLAYWRIGHT_NO_COPY_PROMPT = "1";

const port = Number(process.env.EM_AS_QA_WEB_PORT ?? 15186);
const evidence = path.resolve(import.meta.dirname, "../../artifacts/api-settings/qa");

export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: "api-settings-qa.spec.ts",
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
    // 输入/请求中含测试专用随机密钥，禁止 trace/HAR/自动截图保存其副本。
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
    // 用例自己创建随机端口后端，浏览器 API 由独立路由桥转发。
    env: { EM_WEB_PORT: String(port), EM_API_PROXY_TARGET: "http://127.0.0.1:18186" },
    stdout: "pipe",
    stderr: "pipe",
  },
});
