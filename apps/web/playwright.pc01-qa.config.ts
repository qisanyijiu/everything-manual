import path from "node:path";
import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

export default defineConfig(base, {
  testMatch: "qa-pc01-download.spec.ts",
  globalSetup: undefined,
  globalTeardown: undefined,
  outputDir: path.resolve(import.meta.dirname, "../../var/pc01-qa-round1/browser"),
  use: { ...base.use, trace: "off", screenshot: "off", video: "off" },
});
