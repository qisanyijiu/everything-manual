/**
 * e2e 全局准备：构建并启动真实 Rust 后端（临时 data-dir，自动 `init` 管理员）。
 *
 * 约定（validation-release.md §2）：`test:e2e` 必须自动管理测试后端与临时目录，
 * 不得依赖外部已运行服务。这里做的每一步都能在 `artifacts/web-mvp/t09-rd/`
 * 或 e2e 工作目录找到证据（后端日志、运行时 JSON、临时 data-dir）。
 *
 * T16 起额外写入**测试用价格目录与非密钥供应商配置**（避免向真实供应商发起任何调用）：
 * - `price-catalog.example.toml` 复制到 e2e 工作目录（Tripo 30 credits / 说明书 AI 示例单价）；
 * - `providers.*.base_url` 指向 `127.0.0.1:1`（保留端口、不会监听成功 → 立即连接被拒），
 *   API key 用本套件专用的假环境变量 `EM_E2E_TRIPO_KEY` / `EM_E2E_MANUAL_AI_KEY`。
 *   这样 estimate/建单（T16 向导的必需路径）可用，而执行器对供应商阶段的调用**不可能**
 *   离开本机（且不会被误当成真实凭据：环境变量名与部署者的真实密钥名无关）。
 */

import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import {
  E2E_API_PORT,
  E2E_WORK_DIR,
  EVIDENCE_DIR,
  serverBinary,
  writeRuntime,
  type E2eRuntime,
} from "./runtime";

const PASSWORD = "e2e-t09-password-6b31";

export default async function globalSetup(): Promise<void> {
  const binary = serverBinary();
  // 1) 构建后端（debug；e2e 用测试构建，正式包由 T22 验证）。
  execFileSync("cargo", ["build", "-p", "everything-manual"], {
    cwd: path.resolve(import.meta.dirname, "../../../.."),
    stdio: "inherit",
  });

  // 2) 全新临时 data-dir + 受限口令文件（不通过命令行传口令）。
  fs.rmSync(E2E_WORK_DIR, { recursive: true, force: true });
  fs.mkdirSync(E2E_WORK_DIR, { recursive: true });
  const dataDir = path.join(E2E_WORK_DIR, "data");
  const passwordFile = path.join(E2E_WORK_DIR, "password.txt");
  fs.writeFileSync(passwordFile, `${PASSWORD}\n`, { mode: 0o600 });
  execFileSync(binary, ["init", "--data-dir", dataDir, "--password-file", passwordFile], {
    stdio: "inherit",
  });

  // 3) 价格目录与供应商配置（见文件头：base_url 指向本机保留端口，绝不外呼）。
  const priceCatalog = path.join(E2E_WORK_DIR, "price-catalog.toml");
  fs.copyFileSync(
    path.join(path.resolve(import.meta.dirname, "../../../.."), "price-catalog.example.toml"),
    priceCatalog,
  );
  const configPath = path.join(E2E_WORK_DIR, "config.toml");
  fs.writeFileSync(
    configPath,
    [
      `price_catalog_path = ${JSON.stringify(priceCatalog)}`,
      "",
      "[providers.tripo]",
      'base_url = "http://127.0.0.1:1"',
      'model = "v3.1-20260211"',
      'api_key_env = "EM_E2E_TRIPO_KEY"',
      "",
      "[providers.manual_ai]",
      'base_url = "http://127.0.0.1:1"',
      'model = "gpt-5-mini"',
      'api_key_env = "EM_E2E_MANUAL_AI_KEY"',
      "",
    ].join("\n"),
  );

  // 4) 启动后端；日志留档（失败时用于定位，不含口令与密钥）。
  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });
  const serverLog = path.join(EVIDENCE_DIR, "e2e-server.log");
  const logStream = fs.createWriteStream(serverLog, { flags: "w" });
  const child = spawn(
    binary,
    [
      "serve",
      "--data-dir",
      dataDir,
      "--config",
      configPath,
      "--listen",
      `127.0.0.1:${E2E_API_PORT}`,
    ],
    {
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...process.env,
        EM_E2E_TRIPO_KEY: "e2e-fake-tripo-key-not-used",
        EM_E2E_MANUAL_AI_KEY: "e2e-fake-manual-ai-key-not-used",
      },
    },
  );
  child.stdout.pipe(logStream, { end: false });
  child.stderr.pipe(logStream, { end: false });
  child.on("exit", (code) => {
    if (!logStream.writableEnded) {
      logStream.write(`\n[e2e] server exited with code ${code}\n`);
      logStream.end();
    }
  });

  const apiBase = `http://127.0.0.1:${E2E_API_PORT}`;
  await waitForHealth(`${apiBase}/api/v1/health/ready`, serverLog);

  const runtime: E2eRuntime = {
    apiBase,
    password: PASSWORD,
    dataDir,
    serverPid: child.pid ?? 0,
    serverLog,
  };
  writeRuntime(runtime);
  process.env.EM_E2E_PASSWORD = PASSWORD;
}

/** 轮询就绪探针；超时抛错并附带后端日志尾部（不打印口令）。 */
async function waitForHealth(url: string, serverLog: string): Promise<void> {
  const deadline = Date.now() + 30_000;
  let lastError = "";
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
      lastError = `HTTP ${response.status}`;
    } catch (error) {
      lastError = String(error);
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  const tail = fs.existsSync(serverLog)
    ? fs.readFileSync(serverLog, "utf8").split("\n").slice(-20).join("\n")
    : "(无日志)";
  throw new Error(`e2e 后端未就绪（${url}）：${lastError}\n后端日志尾部：\n${tail}`);
}
