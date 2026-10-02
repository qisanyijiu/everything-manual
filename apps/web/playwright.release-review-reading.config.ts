import path from "node:path";
import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

// Use an explicitly supplied, already-built fixture binary; this suite never runs Cargo.
export default defineConfig(base, {
  testMatch: "release-review-reading.spec.ts",
  globalSetup: undefined,
  globalTeardown: undefined,
  outputDir: path.resolve(import.meta.dirname, "../../var/prd-completion/qa15-reader-fix/browser"),
  use: { ...base.use, trace: "off", screenshot: "off", video: "off" },
});
