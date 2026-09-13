import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

import { pdfjsVendor } from "./vite.pdfjs-vendor.ts";

// 开发：Vite 127.0.0.1:5173 代理 /api 到 Rust 127.0.0.1:8080；
// 后端不开放宽泛 CORS（architecture.md §2）。
//
// Playwright e2e 用独立端口（`EM_WEB_PORT` / `EM_API_PROXY_TARGET`）拉起自己的
// 后端与前端实例，避免与开发者正在运行的 dev 服务互相干扰。
const webPort = Number(process.env.EM_WEB_PORT ?? 5173);
const apiTarget = process.env.EM_API_PROXY_TARGET ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [react(), pdfjsVendor()],
  server: {
    host: "127.0.0.1",
    port: webPort,
    strictPort: true,
    proxy: {
      "/api": { target: apiTarget },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    restoreMocks: true,
    // e2e 规格是 Playwright 文件（在真实浏览器里跑），不进 Vitest 收集范围。
    exclude: ["node_modules/**", "dist/**", "tests/e2e/**"],
  },
});
