/**
 * PDF.js 静态资源本地化插件（T09 / REQ-014；架构 §3「PDF」与 §7「vendor 内嵌」）。
 *
 * 为什么需要它：
 * - pdfjs-dist 运行时需要的 CMaps（169 个 `.bcmap`）、standard fonts（`.pfb`）、
 *   WASM（jbig2/openjpeg/qcms/quickjs）与 ICC 档案不是 JS 模块，import 无法引入；
 * - 这些资源必须由**构建内嵌**（随 dist 进 Rust 二进制），运行时**不访问 CDN**；
 * - 资源必须与 `pdfjs-dist` 主包**同版本**：直接从 `node_modules/pdfjs-dist` 复制，
 *   版本漂移不可能发生（`dev` 由中间件实时读取同一目录，`build` 复制同一目录）。
 *
 * 路径约定：`/vendor/pdfjs/<cmaps|standard_fonts|wasm|iccs>/<file>`。
 * 前端只用 `import.meta.env.BASE_URL + "vendor/pdfjs/..."` 拼这些**稳定路径**；
 * PDF.js 主包与 worker 走 Vite 的 `?url` import（带内容哈希，见 `pdf/vendor.ts`），
 * 不手工拼接可能被改名的资产路径。
 */

import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

import type { Plugin } from "vite";

import { PDFJS_VENDOR_PREFIX } from "./src/features/import/pdf/vendor-path.ts";

/** 需要复制／服务的资源子目录（pdfjs-dist 包内名称）。 */
const VENDOR_DIRS = ["cmaps", "standard_fonts", "wasm", "iccs"] as const;

const CONTENT_TYPES: Record<string, string> = {
  ".bcmap": "application/octet-stream",
  ".pfb": "application/octet-stream",
  ".wasm": "application/wasm",
  ".icc": "application/octet-stream",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".txt": "text/plain; charset=utf-8",
  ".md": "text/markdown; charset=utf-8",
};

/** 解析 `node_modules/pdfjs-dist` 的实际目录（兼容提升与嵌套安装）。 */
function resolvePdfjsDir(root: string): string {
  const require = createRequire(path.join(root, "package.json"));
  return path.dirname(require.resolve("pdfjs-dist/package.json"));
}

/** 列出某个资源子目录下的全部文件（相对该子目录，含子目录中的文件）。 */
function listFiles(dir: string, base: string = dir): string[] {
  const entries: string[] = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      entries.push(...listFiles(full, base));
    } else if (entry.isFile()) {
      entries.push(path.relative(base, full));
    }
  }
  return entries;
}

/**
 * 校验资源目录存在且非空；缺失即构建失败（不能"运行时到 CDN 下载"兜底）。
 * 返回每个目录的文件数，供构建日志核对。
 */
export function verifyPdfjsVendor(pdfjsDir: string): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const dir of VENDOR_DIRS) {
    const full = path.join(pdfjsDir, dir);
    if (!fs.existsSync(full)) {
      throw new Error(
        `pdfjs-dist 缺少资源目录 ${dir}（${full}）：PDF.js 离线资源不可缺失，` +
          `请检查 pdfjs-dist 版本与 package.json 锁定值`,
      );
    }
    const files = listFiles(full);
    if (files.length === 0) {
      throw new Error(`pdfjs-dist 资源目录 ${dir} 为空：离线渲染会失败`);
    }
    counts[dir] = files.length;
  }
  return counts;
}

export function pdfjsVendor(): Plugin {
  let root = process.cwd();
  let outDir = "dist";
  let pdfjsDir = "";

  return {
    name: "em-pdfjs-vendor",
    configResolved(config) {
      root = config.root;
      outDir = config.build.outDir;
      pdfjsDir = resolvePdfjsDir(root);
      const counts = verifyPdfjsVendor(pdfjsDir);
      const version = JSON.parse(
        fs.readFileSync(path.join(pdfjsDir, "package.json"), "utf8"),
      ).version as string;
      config.logger.info(
        `  pdfjs-dist ${version} 本地资源：` +
          Object.entries(counts)
            .map(([dir, count]) => `${dir}=${count}`)
            .join(" "),
      );
    },
    // 开发服务器：直接服务 node_modules/pdfjs-dist 下的文件（同版本、无复制）。
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const url = request.url ?? "";
        if (!url.startsWith(PDFJS_VENDOR_PREFIX)) {
          next();
          return;
        }
        const relative = decodeURIComponent(url.slice(PDFJS_VENDOR_PREFIX.length).split("?")[0]!);
        const target = path.normalize(path.join(pdfjsDir, relative));
        if (!target.startsWith(pdfjsDir) || !fs.existsSync(target) || !fs.statSync(target).isFile()) {
          response.statusCode = 404;
          response.end("not found");
          return;
        }
        response.setHeader(
          "content-type",
          CONTENT_TYPES[path.extname(target)] ?? "application/octet-stream",
        );
        response.setHeader("cache-control", "no-cache");
        fs.createReadStream(target).pipe(response);
      });
    },
    // 生产构建：把资源目录复制进 dist（随后由 rust-embed 内嵌进二进制）。
    closeBundle() {
      const counts = verifyPdfjsVendor(pdfjsDir);
      for (const dir of VENDOR_DIRS) {
        const from = path.join(pdfjsDir, dir);
        const to = path.join(root, outDir, "vendor", "pdfjs", dir);
        fs.mkdirSync(to, { recursive: true });
        fs.cpSync(from, to, { recursive: true });
      }
      this.info(
        `  vendor/pdfjs 已写入 ${path.join(outDir, "vendor", "pdfjs")}（` +
          Object.entries(counts)
            .map(([dir, count]) => `${dir}=${count}`)
            .join(" ") +
          "）",
      );
    },
  };
}
