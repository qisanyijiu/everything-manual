import path from "node:path";
import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

// This suite owns its fixture/backend. Avoid globalSetup's second Rust build
// and keep all credentials, HTTP bodies and browser traces out of artifacts.
export default defineConfig(base, {
  testMatch: "release-download.spec.ts",
  globalSetup: undefined,
  globalTeardown: undefined,
  outputDir: path.resolve(import.meta.dirname, "../../artifacts/prd-completion/pc01-rd/browser"),
  use: { ...base.use, trace: "off", screenshot: "off", video: "off" },
});
