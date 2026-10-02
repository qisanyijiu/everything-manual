/** Independent QA: load only the exact root-frozen PC05C frontend. */
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { loadConfigFromFile } from "vite";
import { defineConfig, mergeConfig } from "vitest/config";
import verifyCFreeze from "./tests/e2e/qa-pc05c-freeze";
verifyCFreeze();
const snapshot = process.env.EM_PC05C_QA_WEB_ROOT, sha = process.env.EM_PC05C_QA_WEB_MANIFEST_SHA256;
if (!snapshot || !sha) throw new Error("PC05C frozen component target and manifest SHA required");
const root = path.resolve(import.meta.dirname, "../..");
if (!fs.realpathSync(snapshot).startsWith(path.join(root, "var/prd-completion/web-snapshots") + path.sep)) throw new Error("No mutable frontend fallback");
const manifest = fs.readFileSync(path.join(snapshot, "source-hashes.json"));
if (createHash("sha256").update(manifest).digest("hex") !== sha) throw new Error("PC05C frozen component manifest differs");
for (const [file, hash] of Object.entries(JSON.parse(manifest.toString()) as Record<string, string>)) if (createHash("sha256").update(fs.readFileSync(path.join(snapshot, file))).digest("hex") !== hash) throw new Error("PC05C frozen component file differs: " + file);
const dependencies = fs.realpathSync(path.resolve(import.meta.dirname, "node_modules"));
export default defineConfig(async () => {
  const frozen = await loadConfigFromFile({ command: "serve", mode: "test" }, path.join(snapshot, "vite.config.ts"), snapshot);
  if (!frozen) throw new Error("Frozen Vite config missing");
  return mergeConfig(frozen.config, { root: snapshot, test: { globalSetup: [path.resolve(import.meta.dirname, "tests/e2e/qa-pc05c-component-freeze.ts")] }, server: { fs: { allow: [snapshot, dependencies] } } });
});
