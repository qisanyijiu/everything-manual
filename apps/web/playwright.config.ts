/**
 * Playwright e2e 配置（T09 起；合同见 llmdoc/validation-release.md §2「test:e2e」）。
 *
 * 自管理（不依赖任何外部已运行服务）：
 * - `globalSetup` 构建并启动**真实 Rust 后端**（临时 data-dir + `init` 写入管理员口令），
 *   把 API 端口与口令写入运行时 JSON；
 * - `webServer` 启动 Vite 前端（独立端口），`/api` 代理到上面的后端；
 * - 失败证据（trace / 截图 / 截图目录）落在仓库 `artifacts/web-mvp/t09-rd/`，
 *   后端日志同目录留存。
 *
 * 端口刻意避开开发默认值（5173/8080）：即使开发者本机正跑着 dev 服务，
 * e2e 也用自己的实例，避免互相污染。
 */

import path from "node:path";

import { defineConfig, devices } from "@playwright/test";

import { E2E_API_PORT, E2E_WEB_PORT, E2E_WORK_DIR } from "./tests/e2e/runtime";

const REPO_ROOT = path.resolve(import.meta.dirname, "../..");
const EVIDENCE_DIR = path.join(REPO_ROOT, "artifacts", "web-mvp", "t09-rd");

export default defineConfig({
  testDir: "./tests/e2e",
  // 每个用例都自建物品/资料；顺序执行避免共享后端上的相互等待。
  fullyParallel: false,
  workers: 1,
  forbidOnly: true,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 15_000 },
  outputDir: path.join(EVIDENCE_DIR, "playwright-output"),
  reporter: [["list"]],
  globalSetup: "./tests/e2e/global-setup.ts",
  globalTeardown: "./tests/e2e/global-teardown.ts",
  use: {
    baseURL: `http://127.0.0.1:${E2E_WEB_PORT}`,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "off",
    locale: "zh-CN",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: `npm run dev`,
    cwd: import.meta.dirname,
    url: `http://127.0.0.1:${E2E_WEB_PORT}`,
    reuseExistingServer: false,
    timeout: 120_000,
    env: {
      EM_WEB_PORT: String(E2E_WEB_PORT),
      EM_API_PROXY_TARGET: `http://127.0.0.1:${E2E_API_PORT}`,
      EM_E2E_WORK_DIR: E2E_WORK_DIR,
    },
    stdout: "pipe",
    stderr: "pipe",
  },
});
