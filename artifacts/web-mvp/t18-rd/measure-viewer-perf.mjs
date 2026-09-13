#!/usr/bin/env node
/**
 * T18 阅读器性能实测（REQ-040 / PRD §5.5：100k 三角面、≤4K 贴图预算下桌面连续
 * 旋转目标 p95 帧耗时 ≤ 33ms）。
 *
 * 为什么不用 headless CI 跑这条测量：
 * - Playwright 默认的 headless Chromium 在本机走 SwiftShader（软件光栅），帧耗时
 *   与真实 GPU 相差数量级，结论不可用于达标判定（validation-release §4 也明确
 *   "Headless WebGL 结果不替代目标设备人工复核"）。
 * - 因此本脚本用**真实 Chrome（channel=chrome）+ 真实 GPU + 真实窗口**测量，并在
 *   报告里记录设备/浏览器/WebGL 渲染器字符串，便于复核与复现。
 *
 * 测量对象是**生产路径**：`vite build` 的 dist 由 `cargo build --features
 * embedded-ui` 内嵌进 Rust 二进制，单进程同源提供页面与 API（与发布形态一致）。
 * 模型（约 100k 面）由 `apps/web/tests/e2e/fixtures/generate-viewer-fixtures.mjs
 * --large` 现场确定性生成，草稿与模型字节在浏览器侧按合同形态提供（同 e2e 的
 * 做法：服务端在 T18 尚无"读取模型版本资产"端点，T19 接入发布端点后替换）。
 *
 * 用法（任意目录）：
 *   node artifacts/web-mvp/t18-rd/measure-viewer-perf.mjs [--seconds 20] [--json out.json]
 *
 * 输出：控制台报告 + JSON（帧耗时统计、设备信息、资源账本）+ PNG 截图。
 */

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createWriteStream, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, "../../..");
const WEB_DIR = path.join(REPO_ROOT, "apps", "web");
const API_PORT = Number(process.env.EM_PERF_API_PORT ?? 19090);
const BASE = `http://127.0.0.1:${API_PORT}`;
const PASSWORD = "perf-measure-password-1a2b";

function log(message) {
  process.stdout.write(`${message}\n`);
}

function parseArgs(argv) {
  const args = { seconds: 20, json: path.join(HERE, "perf-result.json") };
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === "--seconds") {
      args.seconds = Number(argv[index + 1]);
      index += 1;
    } else if (argv[index] === "--json") {
      args.json = path.resolve(argv[index + 1]);
      index += 1;
    }
  }
  return args;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", cwd: REPO_ROOT, ...options });
  if (result.status !== 0) {
    throw new Error(`命令失败（${result.status}）：${command} ${args.join(" ")}`);
  }
}

async function waitForHealth(url, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
    } catch {
      // 尚未就绪
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`后端未就绪：${url}`);
}

/** 从会话接口读取 CSRF token（浏览器上下文共享 cookie）。 */
async function readCsrf(context) {
  const response = await context.request.get(`${BASE}/api/v1/auth/session`);
  const body = await response.json();
  return body.data.csrfToken;
}

function percentile(sorted, p) {
  if (sorted.length === 0) {
    return null;
  }
  const index = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[index];
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const workDir = mkdtempSync(path.join(os.tmpdir(), "em-t18-perf-"));
  log(`T18 性能测量工作目录：${workDir}`);

  let server = null;
  let serverLogStream = null;
  let browser = null;
  try {
    // 1) dist + embedded-ui 二进制（生产形态：同源提供页面与 API）
    log("[1/6] 构建前端产物与内嵌 UI 的 Rust 二进制…");
    run("npm", ["--prefix", WEB_DIR, "run", "build"]);
    run("cargo", ["build", "-p", "everything-manual", "--features", "embedded-ui"]);

    // 2) 100k 面模型（确定性生成）
    log("[2/6] 生成 100k 三角面测试模型…");
    run("node", [
      path.join(WEB_DIR, "tests", "e2e", "fixtures", "generate-viewer-fixtures.mjs"),
      "--large",
      "--out",
      workDir,
    ]);
    const largeBytes = readFileSync(path.join(workDir, "viewer-large.glb"));
    const largeSha = createHash("sha256").update(largeBytes).digest("hex");

    // 3) 临时 data-dir + 后端（内嵌 UI，与发布形态一致）
    log("[3/6] 初始化 data-dir 并启动内嵌 UI 的服务…");
    const dataDir = path.join(workDir, "data");
    const passwordFile = path.join(workDir, "password.txt");
    writeFileSync(passwordFile, `${PASSWORD}\n`, { mode: 0o600 });
    const binary = path.join(REPO_ROOT, "target", "debug", "everything-manual");
    run(binary, ["init", "--data-dir", dataDir, "--password-file", passwordFile]);
    serverLogStream = createWriteStream(path.join(workDir, "server.log"));
    server = spawn(binary, ["serve", "--data-dir", dataDir, "--listen", `127.0.0.1:${API_PORT}`], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    server.stdout.pipe(serverLogStream, { end: false });
    server.stderr.pipe(serverLogStream, { end: false });
    await waitForHealth(`${BASE}/api/v1/health/ready`);

    // 4) 真实 Chrome（真实 GPU）
    log("[4/6] 启动真实 Chrome 并准备阅读页…");
    const { chromium } = await import(
      path.join(WEB_DIR, "node_modules", "playwright", "index.mjs")
    );
    browser = await chromium.launch({
      channel: "chrome",
      headless: false,
      args: ["--hide-scrollbars", "--disable-infobars"],
    });
    const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } });
    const page = await context.newPage();
    const consoleErrors = [];
    page.on("pageerror", (error) => consoleErrors.push(String(error)));

    await page.goto(`${BASE}/login`);
    await page.getByLabel("密码").fill(PASSWORD);
    await page.getByRole("button", { name: "登录" }).click();
    await page.getByRole("heading", { name: "资料库", exact: true }).waitFor();

    const itemResponse = await context.request.post(`${BASE}/api/v1/items`, {
      data: { name: "T18 性能测量", model: "PERF-100K" },
      headers: { "content-type": "application/json", "x-csrf-token": await readCsrf(context) },
    });
    if (!itemResponse.ok()) {
      throw new Error(`创建物品失败：${itemResponse.status()} ${await itemResponse.text()}`);
    }
    const itemId = (await itemResponse.json()).data.id;

    // 草稿与模型字节：按合同形态提供（同 e2e 的做法）。
    const modelAssetId = "perf-large-model";
    const draft = {
      id: "draft-perf",
      itemId,
      snapshotId: "snapshot-perf",
      modelRevisionId: "revision-perf-large",
      revision: 1,
      status: "needs_review",
      completeness: "complete",
      missing: [],
      notices: ["生成完成 ≠ 已发布：不存在自动发布路径。"],
      knowledge: {
        schemaVersion: "manual_draft_v1",
        sourceJobId: "job-perf",
        completeness: "complete",
        model: {
          revisionId: "revision-perf-large",
          sha256: largeSha,
          validationState: "validated",
          assetId: modelAssetId,
          bounds: null,
        },
        knowledge: null,
        hotspots: [],
        missing: [],
      },
      review: null,
      createdAt: "2026-09-12T00:00:00Z",
      updatedAt: "2026-09-12T00:00:00Z",
    };
    await page.route(/\/api\/v1\/items\/[^/]+\/drafts\/[^/?]+(\?.*)?$/, async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ data: draft }),
      });
    });
    await page.route(/\/api\/v1\/assets\/[^/]+\/content(\?.*)?$/, async (route) => {
      if (!route.request().url().includes(modelAssetId)) {
        await route.continue();
        return;
      }
      await route.fulfill({ status: 200, contentType: "model/gltf-binary", body: largeBytes });
    });

    await page.goto(`${BASE}/items/${itemId}/drafts/draft-perf/review`);
    await page.getByTestId("viewer-canvas").waitFor({ timeout: 60_000 });
    await page.waitForFunction(() => (window.__EM_VIEWER__?.stats().modelsAlive ?? 0) === 1, null, {
      timeout: 60_000,
    });
    const modelInfo = await page.evaluate(() => window.__EM_VIEWER__.model());

    // 5) 连续旋转 + 帧耗时采样（rAF 间隔；真实输入事件驱动 OrbitControls）
    log(`[5/6] 连续旋转 ${args.seconds}s 并采样帧耗时…`);
    await page.evaluate(() => {
      window.__perf = { samples: [], running: true };
      let last = performance.now();
      const tick = (now) => {
        if (!window.__perf.running) {
          return;
        }
        window.__perf.samples.push(now - last);
        last = now;
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    // 预热 2s：模型上传纹理/编译着色器不进入采样窗口。
    await new Promise((resolve) => setTimeout(resolve, 2000));
    await page.evaluate(() => {
      window.__perf.samples.length = 0;
    });

    const canvasBox = await page.getByTestId("viewer-canvas").boundingBox();
    const cx = canvasBox.x + canvasBox.width / 2;
    const cy = canvasBox.y + canvasBox.height / 2;
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    const startedAt = Date.now();
    let angle = 0;
    while (Date.now() - startedAt < args.seconds * 1000) {
      angle += 0.35;
      await page.mouse.move(cx + Math.cos(angle) * 260, cy + Math.sin(angle) * 120, { steps: 2 });
    }
    await page.mouse.up();

    const samples = await page.evaluate(() => window.__perf.samples.slice());
    const stats = await page.evaluate(() => window.__EM_VIEWER__.stats());
    const rendererInfo = await page.evaluate(() => {
      const canvas = document.querySelector('[data-testid="viewer-canvas"]');
      const gl = canvas.getContext("webgl2") ?? canvas.getContext("webgl");
      const extension = gl.getExtension("WEBGL_debug_renderer_info");
      return {
        vendor: extension ? gl.getParameter(extension.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR),
        renderer: extension
          ? gl.getParameter(extension.UNMASKED_RENDERER_WEBGL)
          : gl.getParameter(gl.RENDERER),
        version: gl.getParameter(gl.VERSION),
        devicePixelRatio: window.devicePixelRatio,
      };
    });
    await page.screenshot({ path: path.join(HERE, "perf-screenshot.png") });

    // 6) 统计与报告
    const sorted = [...samples].sort((a, b) => a - b);
    const mean = sorted.reduce((sum, value) => sum + value, 0) / (sorted.length || 1);
    const result = {
      measuredAt: new Date().toISOString(),
      platform: {
        os: `${os.type()} ${os.release()} ${os.arch()}`,
        cpus: os.cpus()[0]?.model ?? "unknown",
        cpuCount: os.cpus().length,
        memoryGiB: Math.round(os.totalmem() / 1024 ** 3),
        userAgent: await page.evaluate(() => navigator.userAgent),
        webgl: rendererInfo,
        viewport: "1600x1000 (真实窗口，非 headless)",
      },
      model: {
        bytes: largeBytes.byteLength,
        sha256: largeSha,
        triangles: modelInfo?.triangles ?? null,
        textures: modelInfo?.textures ?? null,
      },
      rotationSeconds: args.seconds,
      frameTimeMs: {
        samples: sorted.length,
        p50: percentile(sorted, 50),
        p95: percentile(sorted, 95),
        p99: percentile(sorted, 99),
        max: sorted[sorted.length - 1] ?? null,
        mean,
        targetP95: 33,
        pass: (percentile(sorted, 95) ?? Number.POSITIVE_INFINITY) <= 33,
      },
      resources: stats,
      consoleErrors,
    };
    writeFileSync(args.json, `${JSON.stringify(result, null, 2)}\n`);
    log(`[6/6] 结果（${args.json}）：`);
    log(JSON.stringify(result.frameTimeMs, null, 2));
    log(`设备：${result.platform.os} / ${result.platform.cpus} ×${result.platform.cpuCount} / GPU=${rendererInfo.renderer}`);
    log(`模型：${largeBytes.byteLength} 字节、${modelInfo?.triangles} 三角面、sha256=${largeSha.slice(0, 16)}…`);
    log(`资源账本：${JSON.stringify(stats)}`);
    if (consoleErrors.length > 0) {
      log(`页面错误：${consoleErrors.join(" | ")}`);
    }
  } finally {
    if (browser !== null) {
      await browser.close().catch(() => undefined);
    }
    if (server !== null) {
      server.kill("SIGTERM");
    }
    if (serverLogStream !== null) {
      serverLogStream.end();
    }
    rmSync(workDir, { recursive: true, force: true });
  }
}

main().catch((error) => {
  process.stderr.write(
    `性能测量失败：${error instanceof Error ? (error.stack ?? error.message) : error}\n`,
  );
  process.exit(1);
});
