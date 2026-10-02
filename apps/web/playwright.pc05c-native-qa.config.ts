import path from "node:path";
import { defineConfig } from "@playwright/test";
import base from "./playwright.pc05c-qa.config";
export default defineConfig({ ...base, testMatch: "qa-pc05c-native.native.ts", timeout: 150000,
  projects: [{ name: "firefox157-native-bidi", use: {} }],
  outputDir: path.join(process.env.EM_PC05C_QA_OUTPUT_DIR!, "native-browser"),
});
