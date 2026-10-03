import path from "node:path";
import { defineConfig } from "@playwright/test";
import base from "./playwright.config";
export default defineConfig(base, {
  testMatch: ["workflow-recovery.spec.ts"], globalSetup: undefined, globalTeardown: undefined,
  outputDir: path.resolve(import.meta.dirname,"../../artifacts/prd-completion/pc03b-rd/browser"),
  use: {...base.use,trace:"off",screenshot:"off",video:"off"},
});
