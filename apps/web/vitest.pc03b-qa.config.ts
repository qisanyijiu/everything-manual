/** QA-only runtime: frozen source tree, not mutable apps/web/src. */
import fs from "node:fs";
import path from "node:path";
import { loadConfigFromFile } from "vite";
import { defineConfig, mergeConfig } from "vitest/config";
const root = path.resolve(import.meta.dirname, "../..");
const snapshot = process.env.EM_PC03B_QA_WEB_ROOT ?? path.join(root, "var/prd-completion/web-snapshots/pc03b-rd-ready");
if (!path.resolve(snapshot).startsWith(path.join(root, "var/prd-completion/web-snapshots") + path.sep)) throw new Error("PC03B QA requires an explicit frozen snapshot");
const dependencies = fs.realpathSync(path.resolve(import.meta.dirname, "node_modules"));
export default defineConfig(async () => {
  const frozen = await loadConfigFromFile({ command: "serve", mode: "test" }, path.join(snapshot, "vite.config.ts"), snapshot);
  if (!frozen) throw new Error("PC03B frozen Vite config is required");
  return mergeConfig(frozen.config, { root: snapshot, server: { fs: { allow: [snapshot, dependencies] } } });
});
