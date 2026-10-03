import path from "node:path";
import { defineConfig } from "@playwright/test";
import base from "./playwright.config";
export default defineConfig(base, {
  testMatch: "original-reader.spec.ts",
  globalSetup: undefined,
  globalTeardown: undefined,
  outputDir: path.resolve(import.meta.dirname, "../../artifacts/prd-completion/pc02a-rd/browser"),
  use: { ...base.use, actionTimeout: 10_000, trace: "off", screenshot: "off", video: "off" },
});
