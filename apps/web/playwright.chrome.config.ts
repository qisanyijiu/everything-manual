import path from "node:path";

import { defineConfig, devices } from "@playwright/test";

import base from "./playwright.config";

// The delivery regression owns its Rust instance and fixture; no old QA evidence is overwritten.
export default defineConfig({
  ...base,
  testMatch: "delivery-standalone.spec.ts",
  globalSetup: undefined,
  globalTeardown: undefined,
  outputDir: path.join(
    process.env.EM_E2E_EVIDENCE_DIR
      ?? path.resolve(import.meta.dirname, "../../var/delivery-20261004/fixture-chrome"),
    "browser",
  ),
  reporter: [["list"]],
  projects: [{
    name: "chrome-stable",
    use: {
      ...devices["Desktop Chrome"],
      channel: "chrome",
      viewport: { width: 1440, height: 1000 },
      launchOptions: { args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] },
    },
  }],
});
