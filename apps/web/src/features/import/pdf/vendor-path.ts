/**
 * PDF.js 本地资源路径前缀（唯一来源）。
 *
 * 同时被两处引用：
 * - 应用运行时代码（`pdf/vendor.ts`）——拼接 CMaps/字体/WASM 的请求地址；
 * - 构建插件（`vite.pdfjs-vendor.ts`）——dev 中间件匹配与 build 复制目标。
 *
 * 放在 `src/` 下（而不是插件文件里）是为了让两端导入同一个常量：插件文件含 node API，
 * 应用代码不能导入它；本文件无任何依赖，可被两边安全引用。
 */
export const PDFJS_VENDOR_PREFIX = "/vendor/pdfjs/";
