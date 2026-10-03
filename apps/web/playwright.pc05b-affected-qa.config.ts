/** Preserve existing PC02B/AS contracts while executing only root-frozen PC05B inputs. */
import path from "node:path";
import { defineConfig } from "@playwright/test";
import base from "./playwright.pc05b-qa.config";
const root = path.resolve(import.meta.dirname, "../..");
const out = path.resolve(process.env.EM_PC05B_QA_OUTPUT_DIR ?? path.join(root, "var/pc05b-qa-round12"), "affected");
process.env.EM_PC02B_QA_WEB_PORT = "15490";
process.env.EM_PC02B_QA_BINARY = process.env.EM_PC05B_QA_BINARY!;
process.env.EM_PC02B_QA_BINARY_SHA256 = process.env.EM_PC05B_QA_BINARY_SHA256!;
process.env.EM_PC02B_QA_OUTPUT_DIR = path.join(out, "pc02b");
process.env.EM_AS_QA_WEB_PORT = "15490";
process.env.EM_AS_QA_BINARY = process.env.EM_PC05B_QA_BINARY!;
process.env.EM_AS_QA_BINARY_SHA256 = process.env.EM_PC05B_QA_BINARY_SHA256!;
process.env.EM_AS_QA_ARTIFACT_DIR = path.join(out, "api-settings");
process.env.EM_AS_QA_NO_SCREENSHOTS = "1";
export default defineConfig({
  ...base,
  testMatch: ["qa-pc02b-browser.spec.ts", "qa-pc02b-api.spec.ts", "api-settings-qa.spec.ts"],
  grep: /(QA PC2-|AS-QA-0[28] )/,
  outputDir: path.join(out, "browser"),
  globalTeardown: "./tests/e2e/qa-pc05b-freeze.ts",
});
