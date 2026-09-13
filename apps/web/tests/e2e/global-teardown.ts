/**
 * e2e 全局收尾：结束后端进程并保留临时 data-dir 供复查（含 e2e 失败证据）。
 *
 * 只结束本套件启动的进程（按运行时 JSON 记录的 PID），不触碰开发者自己的服务。
 */

import fs from "node:fs";

import { E2E_WORK_DIR, RUNTIME_FILE, type E2eRuntime } from "./runtime";

export default async function globalTeardown(): Promise<void> {
  if (!fs.existsSync(RUNTIME_FILE)) {
    return;
  }
  const runtime = JSON.parse(fs.readFileSync(RUNTIME_FILE, "utf8")) as E2eRuntime;
  if (runtime.serverPid > 0) {
    try {
      process.kill(runtime.serverPid, "SIGTERM");
    } catch {
      // 进程已退出：忽略。
    }
  }
  // 运行时信息与日志保留在 E2E_WORK_DIR / artifacts 中，便于失败复查；
  // 临时 data-dir 也保留（不含真实用户数据，只有本次 e2e 自建的样例资料）。
  fs.writeFileSync(
    `${RUNTIME_FILE}.done`,
    `teardown 完成：${new Date().toISOString()}（data-dir 保留于 ${runtime.dataDir}）\n`,
  );
  void E2E_WORK_DIR;
}
