import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { defineConfig } from "@playwright/test";
const root = path.resolve(import.meta.dirname, "../..");
const snapshot = process.env.EM_PC05B_QA_WEB_ROOT;
const expected = process.env.EM_PC05B_QA_WEB_MANIFEST_SHA256;
if (!snapshot || !expected || !/^[a-f0-9]{64}$/.test(expected)) throw new Error("PC05B QA requires explicit frozen web root and manifest SHA; no historical/current fallback");
if (!fs.realpathSync(snapshot).startsWith(path.join(root, "var/prd-completion/web-snapshots") + path.sep)) throw new Error("PC05B QA web target must be a root-frozen snapshot");
const manifest = fs.readFileSync(path.join(snapshot, "source-hashes.json"));
if (createHash("sha256").update(manifest).digest("hex") !== expected) throw new Error("PC05B web manifest SHA differs from GO");
for (const [file, hash] of Object.entries(JSON.parse(manifest.toString()) as Record<string, string>)) if (createHash("sha256").update(fs.readFileSync(path.join(snapshot, file))).digest("hex") !== hash) throw new Error("PC05B frozen web file differs: " + file);
const binary = process.env.EM_PC05B_QA_BINARY, binarySha = process.env.EM_PC05B_QA_BINARY_SHA256;
if (!binary || !binarySha || !/^[a-f0-9]{64}$/.test(binarySha)) throw new Error("PC05B copied binary path and SHA are mandatory alongside the web freeze");
if (!fs.realpathSync(binary).startsWith(path.join(root, "var") + path.sep)) throw new Error("PC05B refuses target/current binary fallback");
if (createHash("sha256").update(fs.readFileSync(binary)).digest("hex") !== binarySha) throw new Error("PC05B backend SHA differs from GO");
process.env.EM_E2E_WEB_PORT = "15490";
process.env.PLAYWRIGHT_NO_COPY_PROMPT = "1";
const outputRoot = path.resolve(process.env.EM_PC05B_QA_OUTPUT_DIR ?? path.join(root, "var/pc05b-qa-round12"));
if (!outputRoot.startsWith(path.join(root, "var") + path.sep)) throw new Error("PC05B output must remain under owned repo/var");
export default defineConfig({
  testDir: "./tests/e2e", testMatch: "qa-pc05b-*.spec.ts", workers: 1, fullyParallel: false,
  forbidOnly: true, retries: 0, timeout: 210000, expect: { timeout: 15000 }, reporter: [["list"]],
  outputDir: path.join(outputRoot, "browser"),
  use: { baseURL: "http://127.0.0.1:15490", trace: "off", video: "off", screenshot: "off", locale: "zh-CN" },
  // Explicit supported engine; do not silently fall back to bundled Chromium148 or override native UA.
  projects: [{ name: "chrome-stable", use: { viewport: { width: 1280, height: 720 }, channel: "chrome", headless: false } }],
  webServer: { command: "npm run dev", cwd: snapshot, url: "http://127.0.0.1:15490", reuseExistingServer: false, timeout: 120000,
    env: { EM_WEB_PORT: "15490", EM_API_PROXY_TARGET: "http://127.0.0.1:18490" }, stdout: "pipe", stderr: "pipe" },
});
