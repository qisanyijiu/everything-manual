import path from "node:path";
import { defineConfig } from "@playwright/test";
import base from "./playwright.api-settings-qa.config";

// Private backend owned by the test; no globalSetup compilation or secret traces.
export default defineConfig(base, {
  testMatch: "model-guard.spec.ts",
  outputDir: path.resolve(import.meta.dirname, "../../artifacts/prd-completion/pc06-rd/browser"),
});
