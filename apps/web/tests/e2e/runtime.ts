/**
 * e2e 运行时约定（配置、global-setup/teardown 与用例共享）。
 *
 * 端口与目录集中在这里，避免三处硬编码漂移。
 */

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

/** 仓库根（`apps/web/tests/e2e/runtime.ts` → 上溯三级）。 */
export const REPO_ROOT = path.resolve(import.meta.dirname, "../../../..");

/** e2e 后端端口（避开开发默认 8080）。 */
export const E2E_API_PORT = Number(process.env.EM_E2E_API_PORT ?? 18080);

/** e2e 前端端口（避开开发默认 5173）。 */
export const E2E_WEB_PORT = Number(process.env.EM_E2E_WEB_PORT ?? 15173);

/** e2e 工作目录（临时 data-dir、口令文件、后端日志、运行时 JSON）。 */
export const E2E_WORK_DIR =
  process.env.EM_E2E_WORK_DIR ?? path.join(os.tmpdir(), "em-web-mvp-e2e");

/** 失败证据与后端日志目录（仓库内，随 artifacts 留存）。 */
export const EVIDENCE_DIR = path.join(REPO_ROOT, "artifacts", "web-mvp", "t09-rd");

/** globalSetup 写入、用例读取的运行时信息。 */
export interface E2eRuntime {
  readonly apiBase: string;
  readonly password: string;
  readonly dataDir: string;
  readonly serverPid: number;
  readonly serverLog: string;
}

export const RUNTIME_FILE = path.join(E2E_WORK_DIR, "runtime.json");

export function writeRuntime(runtime: E2eRuntime): void {
  fs.mkdirSync(E2E_WORK_DIR, { recursive: true });
  fs.writeFileSync(RUNTIME_FILE, JSON.stringify(runtime, null, 2));
}

export function readRuntime(): E2eRuntime {
  return JSON.parse(fs.readFileSync(RUNTIME_FILE, "utf8")) as E2eRuntime;
}

/** 后端二进制路径（debug 构建；由 globalSetup 负责先构建）。 */
export function serverBinary(): string {
  return path.join(REPO_ROOT, "target", "debug", "everything-manual");
}

/** 仓库样例资产路径（T05 原创 fixture）。 */
export function fixturePath(name: string): string {
  return path.join(REPO_ROOT, "tests", "fixtures", "assets", name);
}
