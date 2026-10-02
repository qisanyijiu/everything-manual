import path from "node:path";
import { defineConfig } from "@playwright/test";
process.env.PLAYWRIGHT_NO_COPY_PROMPT = "1";
export default defineConfig({
  testDir: "./tests/e2e", testMatch: "qa-pc04-*.spec.ts", workers: 1,
  fullyParallel: false, forbidOnly: true, retries: 0, timeout: 180000,
  expect: { timeout: 15000 }, reporter: [["list"]],
  outputDir: path.resolve(import.meta.dirname, "../../var/pc04-qa-round9/runner"),
  use: { trace: "off", video: "off", screenshot: "off" },
});
