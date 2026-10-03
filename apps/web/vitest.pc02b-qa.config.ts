/** QA-only: frozen PC02B sources and its symlinked dependency runtime. */
import fs from "node:fs";
import path from "node:path";
import { defineConfig, mergeConfig } from "vitest/config";
import frozen from "../../var/prd-completion/web-snapshots/pc02b-rd-ready/vite.config.ts";
const snapshot = path.resolve(import.meta.dirname, "../../var/prd-completion/web-snapshots/pc02b-rd-ready");
const dependencies = fs.realpathSync(path.resolve(import.meta.dirname, "node_modules"));
export default mergeConfig(frozen, defineConfig({ root: snapshot, server: { fs: { allow: [snapshot, dependencies] } } }));
