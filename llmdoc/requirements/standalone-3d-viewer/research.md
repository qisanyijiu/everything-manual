# 生成产物格式调研 & 离线 3D 页面（standalone-3d-viewer）

状态：已实现（分支 `feat/standalone-3d-viewer`），未经独立 QA。日期：2026-10-02。

## 背景

用户要求：用测试说明书跑一次完整流程，调研生成产物格式，并让结果能"像 three.js 一样直接展示在网页上"。原有交付只有两种：需要登录的在线阅读器，和 `GET /releases/{id}/export` 的 ZIP（导出说明明确写着"不承诺可直接双击运行网站"，且前端没有入口）。

## 实测流程（真实 Provider）

- 输入：`tests/fixtures/assets/sample-manual-text.pdf`（2 页，虚构 Model X100）+ 4 张 AI 生成的多视图照片（front/left/back/right）。
- 结果：9 个阶段全部成功，用时约 3.5 分钟，花费 30 Tripo credits；说明书 AI（`gpt-5.5`）提取出 4 个部件、2 个步骤、3 条规格，每条都带页码证据。
- 前置条件：`[download] allowed_hosts` 必须包含 Tripo CDN 域名，否则 `model_download` 会被拒（默认为空 = 拒绝一切）。

## 产物格式

| 产物 | 格式 | 要点 |
| --- | --- | --- |
| 3D 模型 | glTF 2.0 二进制（GLB，`asset.generator = https://tripo3d.ai`） | 单 mesh、单 primitive；属性 POSITION/NORMAL/TEXCOORD_0；1 个 PBR 材质（baseColor JPEG、metallicRoughness JPEG、normal PNG，均为 2048²，内嵌在 BIN）；无扩展、无 Draco；约 8.7 万三角面、4.4 MB。服务端校验后归一到 ±0.5 包围盒。 |
| 知识 | `manual_extract_v1` JSON（在 release manifest 的 `knowledge.knowledge` 下） | `parts[] / steps[] / specs[] / uncertainties[]`，每条带 `evidence[{pageNumber, quote, documentId}]`；`coverage` 记录批次与页码覆盖。 |
| 热点 | `knowledge.hotspots[]` | `anchor = {modelRevisionId, modelSha256, positionLocal}`，`positionLocal` 是 GLB 场景根的局部坐标（[coordinates.ts](../../../apps/web/src/features/viewer/coordinates.ts)）。 |
| 发布版本 | `manual_release_v1` manifest（不可变） | 冻结知识 + 复核记录 + 模型/原件资产引用。 |
| 导出包 | ZIP（STORE，字节确定） | `manifest.json`、`release/manifest.json`、`assets/model/<sha>.glb`、`assets/document/<sha>.pdf`。 |

结论：GLB 是标准 glTF 2.0 + 核心 PBR，three.js 的 `GLTFLoader` 无需任何扩展解码器即可直接加载。

## 决策：浏览器端生成单文件离线 HTML

- **做法**：阅读页新增「下载离线 3D 页面」。浏览器取 GLB 字节与 manifest，拼出一个 HTML：内联 three.js 阅读器（IIFE）、知识 JSON、base64 GLB。双击即可打开，不需要服务端、登录或网络。
- **原因**：满足"直接在网页上展示"，同时不改服务端合同、不新增公开端点、不放宽鉴权；GLB 本身就是 three.js 的原生格式，无需转换。
- **否决方案**：免登录分享链接——单管理员模型下需要新的权限设计与公开面，超出本次授权；ZIP 里加阅读器——`file://` 下 `fetch` 相邻 GLB 会被浏览器拦截，仍然不能双击打开。
- **安全**：HTML 带 CSP `default-src 'none'`，没有任何网络来源；知识文本只经 `textContent` 写入；内嵌 JSON 转义 `<`、U+2028/2029；不含会话、密钥、服务端 URL。
- **代价**：文件比 GLB 大约 1/3（base64），实测 6.2 MB；离线脚本约 645 KB（three.js + loader + controls），只在点击导出时 lazy 加载，不进首屏。离线版没有原文 PDF 面板（只显示页码）。

## 实现与证据

- 打包插件：[vite.standalone-viewer.ts](../../../apps/web/vite.standalone-viewer.ts)（rolldown 把 `src/features/standalone/viewer/main.ts` 打成 IIFE 字符串，作为虚拟模块）。
- 组装与载荷：[build-html.ts](../../../apps/web/src/features/standalone/build-html.ts)、[payload.ts](../../../apps/web/src/features/standalone/payload.ts)；单测 [build-html.test.ts](../../../apps/web/src/features/standalone/build-html.test.ts)（7 项）。
- 浏览器验证（系统 Chrome headless + SwiftShader）：在阅读页点击导出 → 下载 → 在拦截全部网络的新上下文用 `file://` 打开 → 模型渲染、4 个部件、2 步导航、3 条规格、点击联动正常，0 次网络请求，0 条控制台错误。截图：[offline-viewer.png](../../../artifacts/standalone-3d-viewer/offline-viewer.png)、[reader-export-button.png](../../../artifacts/standalone-3d-viewer/reader-export-button.png)。

## 真实说明书测试：PENTAX 17（2026-10-02）

- 输入：`pentax17_om_sc_web.pdf`（42 页、全部有文字层）；视图取自第 8 页（第 6 页码）的正面/背面线稿——直接删除 PDF 中的文字对象、引线（≤12 段路径）与标注圆点后渲染，**不经过 AI 重绘**，保证与说明书一致。
- 第 1 次：9 个说明书 AI 批次中批次 0 网关超时（`manual_ai` 请求超时 180s）→ `submission_unknown`，同分支 3–8 被暂停；按对账流程 `authorizeReplacement` 并逐批 retry 后完成。说明书 AI 预留状态保持 unknown（账务可能重复，属设计行为）。
- 第 2 次：一次成功（约 6 分钟）。两次产物都保留，历史页可分别回看。
- 结果：112 部件、50 步骤、109 规格、29 不确定项，覆盖 42/42 页；GLB 约 8.9 万三角面。截图：[completion-toast.png](../../../artifacts/standalone-3d-viewer/completion-toast.png)、[generation-result.png](../../../artifacts/standalone-3d-viewer/generation-result.png)、[generation-history.png](../../../artifacts/standalone-3d-viewer/generation-history.png)。
- 风险：长批次在网关上接近 180s 超时；超时会触发人工对账。可考虑把 `manual_ai` 请求超时做成配置项（当前为代码常量）。

## 未解决

- 未走 PM → UI → RD → QA 流程，未经独立 QA；未覆盖 Playwright e2e 用例。
- 超大模型（接近 150 MB 上限）的 base64 拼接在浏览器内存里进行，未做压力测试。
