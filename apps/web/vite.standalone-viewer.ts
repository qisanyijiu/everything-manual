/**
 * 离线阅读器脚本内联插件（导出单文件 HTML 用）。
 *
 * 为什么需要它：
 * - 导出的 HTML 必须**双击即可打开**：不依赖本服务、不访问 CDN、不需要登录；
 * - 因此 three.js + GLTFLoader + OrbitControls + 阅读器 UI 要被打成**一段 IIFE 文本**，
 *   由前端在导出时原样写进 `<script>`；
 * - 本插件把 `src/features/standalone/viewer/main.ts` 用 rolldown 打包（压缩、单文件、
 *   无动态 import），以虚拟模块 `virtual:standalone-viewer` 的**字符串默认导出**提供给
 *   SPA。SPA 只在用户点击导出时才 lazy import 它（不进首屏包）。
 *
 * dev 与 build 走同一条路径；入口目录下的文件变化会让虚拟模块失效重建。
 */

import path from "node:path";

import { build } from "rolldown";
import type { Plugin } from "vite";

const VIRTUAL_ID = "virtual:standalone-viewer";
const RESOLVED_ID = `\0${VIRTUAL_ID}`;

export function standaloneViewer(): Plugin {
  let root = process.cwd();
  return {
    name: "em-standalone-viewer",
    configResolved(config) {
      root = config.root;
    },
    resolveId(id) {
      return id === VIRTUAL_ID ? RESOLVED_ID : null;
    },
    async load(id) {
      if (id !== RESOLVED_ID) {
        return null;
      }
      const entryDir = path.join(root, "src/features/standalone/viewer");
      const result = await build({
        input: path.join(entryDir, "main.ts"),
        cwd: root,
        write: false,
        platform: "browser",
        logLevel: "warn",
        output: {
          format: "iife",
          minify: true,
          // 离线文件只有一个 <script>：禁止拆 chunk。
          codeSplitting: false,
        },
      });
      const chunk = result.output.find((item) => item.type === "chunk");
      if (chunk === undefined || chunk.type !== "chunk") {
        this.error("离线阅读器打包失败：没有产出脚本");
      }
      for (const file of chunk.moduleIds) {
        if (file.startsWith(entryDir)) {
          this.addWatchFile(file);
        }
      }
      return `export default ${JSON.stringify(chunk.code)};`;
    },
  };
}
