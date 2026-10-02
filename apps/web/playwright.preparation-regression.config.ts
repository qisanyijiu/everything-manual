import path from "node:path";
import { defineConfig } from "@playwright/test";
import base from "./playwright.config";
export default defineConfig(base, {
  testMatch: "pdf-preparation.spec.ts",
  outputDir: path.resolve(import.meta.dirname, "../../artifacts/prd-completion/pc03a-rd/pdf-regression"),
  use: { ...base.use, trace: "off", screenshot: "off", video: "off" },
});
