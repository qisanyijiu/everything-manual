/** Independent QA: load only the exact root-frozen PC05A frontend. */
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { loadConfigFromFile } from "vite";
import { defineConfig, mergeConfig } from "vitest/config";
const snapshot = process.env.EM_PC05A_QA_WEB_ROOT, sha = process.env.EM_PC05A_QA_WEB_MANIFEST_SHA256;
if (!snapshot || !sha) throw new Error("PC05A frozen component target and manifest SHA required");
const root = path.resolve(import.meta.dirname, "../..");
if (!fs.realpathSync(snapshot).startsWith(path.join(root, "var/prd-completion/web-snapshots") + path.sep)) throw new Error("No mutable frontend fallback");
const manifest = fs.readFileSync(path.join(snapshot, "source-hashes.json"));
if (createHash("sha256").update(manifest).digest("hex") !== sha) throw new Error("PC05A frozen component manifest differs");
const dependencies = fs.realpathSync(path.resolve(import.meta.dirname, "node_modules"));
export default defineConfig(async () => {
  const frozen = await loadConfigFromFile({ command: "serve", mode: "test" }, path.join(snapshot, "vite.config.ts"), snapshot);
  if (!frozen) throw new Error("Frozen Vite config missing");
  return mergeConfig(frozen.config, { root: snapshot, server: { fs: { allow: [snapshot, dependencies] } } });
});
