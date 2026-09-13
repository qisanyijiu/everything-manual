# tests/fixtures —— 原创样例资产与 fixture 场景（T05）

本目录是测试专用资料：**不进入生产依赖树**，不包含任何真实个人信息、真实说明书扫描件或第三方受版权素材。

## 来源与许可

- 全部二进制样例资产（GLB / PDF / PNG / JPEG）由本仓库代码
  `crates/test-support/src/bin/generate_fixtures.rs` **现场构造**：只按公开规范
  （glTF 2.0 二进制容器、PDF 1.4、PNG、JPEG baseline）写字节，没有拷贝任何
  厂商响应、说明书 PDF、照片或字体文件。
- 文本内容是自撰的合成示例（"Model X100" 是虚构型号），不是任何真实产品的说明。
- 许可：与仓库源码相同条款（仓库当前未发布，无独立 LICENSE 文件）；仅在本仓库及
  其测试／构建流程内使用。发布者若要单独分发这些文件，需自行复核。
- 脱敏响应样例（`responses/*.json`）同样是**自建构造**，字段形态对齐
  `llmdoc/contracts.md` §6 的接口示意，**不是供应商官方响应原文**；文件内
  `_fixtureNote` 逐条注明。所有 URL 使用 `.invalid` 域或本机 fixture 地址。

## 资产清单与 sha256（生成器输出，测试中另有固定断言）

| 路径 | 说明 | sha256 | 结构校验结论 |
| --- | --- | --- | --- |
| `assets/sample-model.glb` | glTF 2.0 单位立方体，12 三角面，自包含 BIN + 16×16 PNG 贴图 | `a9f884c27da21ec86e27b9f1df92762d2a4bb4b2431086c19eaf7b932db2ddfb` | version=2、chunk 布局合法、bufferView/accessor 在界内、贴图内嵌、面数与贴图尺寸远低于 PRD §5.3 预算（≤100000 面、≤4096 px） |
| `assets/sample-manual-text.pdf` | 文字型 PDF，2 页，Helvetica 标准字体，无压缩内容流 | `e18cf61a61c9f9834a73e7454c9f1c3a50e897908474f21edac4759fd3695cda` | PDF 1.4、startxref/xref/trailer/Root/Pages 合法、`/Count=2`、有文字算子（`BT`/`Tj`）、无图片对象 |
| `assets/sample-manual-scan.pdf` | 扫描型 PDF，2 页，页图为无压缩 8 位灰度栅格（32×32/页），**无文字层** | `4080019bb8f1d0b9db446add474721734d7dbf8f18e0f62bfa5c59c661db2ce7` | 同上结构合法、`/Count=2`、无文字算子、2 个 `/Subtype /Image`（32×32） |
| `assets/sample-photo-front.jpg` | 灰度 baseline JPEG，32×32（自建最小 Huffman 表） | `122e645391038b12830365dd96f125b35a78cedcdab5183fc67476c89992c586` | SOI…SOF0/DHT/DQT/SOS…EOI 完整，1 分量、非渐进 |
| `assets/sample-photo-left.png` | 真彩 PNG，64×64（zlib stored 块） | `0a74e5a5b542aa62440b48aee24838b664802bf5a98cd73fed45123cd7d2c890` | 签名/IHDR/IDAT/IEND 合法、chunk CRC 全对 |
| `assets/sample-manual-rotated.pdf` | 旋转页 PDF，2 页，第 2 页 `/Rotate 90`（T09） | `4a8821d3bf058c24f0378b958ccc98f20feedfbc32bd1f216a1e1a8b83a400fd` | 结构合法、`/Count=2`、有文字层；第 2 页声明 90° 旋转（页图宽高互换） |
| `assets/sample-manual-nonlatin.pdf` | 非拉丁字体 PDF，1 页，Type0/CID 字体 + `/UniGB-UCS2-H` CMap + ToUnicode（中文文字层）（T09） | `d6640acbd6d3d6aec3e5fac59aa366c9263340669d74124849e1216488fb9030` | 结构合法、`/Count=1`、有文字层；使用**非 Identity** 的 CMap，因此 PDF.js 必须从本地 `cmaps/` 读取 `UniGB-UCS2-H.bcmap`（离线证据） |
| `assets/sample-manual-encrypted.pdf` | **真实加密** PDF，1 页，标准安全处理器 V=1/R=2（RC4 40 位，用户口令 `fixture-secret`，按 PDF 规范算法 2/3/4/5 计算 O/U 与文件密钥）（T09） | `2916fa405c1bd33812808ee18d7b1522c019ecf741f4749ba965b1b340e21503` | 结构合法、`/Count=1`；PDF.js 未提供口令时抛 `PasswordException(NEED_PASSWORD)`，提供正确口令可正常打开（Node 复核见 implementation §T09） |
| `assets/sample-manual-many-pages.pdf` | 101 页 PDF（超过 100 页上限），页数用**间接引用** `/Count 3 0 R` 给出（T09） | `da6bc156855dcfe45afc6a223ad9ea0164fe6e6c478a263cb1df392a3e6a6b85` | 结构合法、实际页数 101；T06 上传探针刻意不解析间接引用（`Parsed{3}` 放行），PDF.js 解析出 101 页 → T09 准备阶段权威拒绝 |

重新生成（结果应逐字节一致；不一致时 sha256 与 `fixture_harness.rs` 中的固定断言会一起失败）：

```sh
cargo run -p test-support --bin generate-fixtures
```

平台解码复核（实现记录中给出的原始输出）：macOS `sips -g format -g pixelWidth -g pixelHeight`
可解码 JPEG/PNG，并可把两个 PDF 渲染为 595×842 的 PNG 页图。

T09 起 `crates/test-support/src/assets.rs` 的 PDF 结构校验器做两处小幅增强（只为支持上述样例，
判定口径不变）：①结构解析改用**等长 ASCII 投影**（非 ASCII 字节 → `?`），使含二进制流的 PDF
（加密样例的 RC4 密文）的 startxref/xref 偏移校验不再错位；②`/Count` 为间接引用时跟随一次。

## 场景脚本（`scenarios/*.json`）

- `tripo_happy.json`：上传 → 提交 → 任务查询（第一次 running，之后 `repeatLast` 返回 success）。
- `manual_ai_happy.json`：Responses 形态的单批提取成功响应。
- `behavior_matrix.json`：每种行为一个路由——`success / delay / disconnect / reset / halfClose /
  rate-limited(429+Retry-After) / server-error(503) / malformed-json / timeout / once / repeats`。

脚本语法（`crates/test-support/src/scenario.rs`）：路由含 `method`、`path`、可选
`match`（`exact` 默认 / `prefix`）、`repeatLast`（默认 false）与 `steps`；步骤用
`kind` 标签区分（`respond / delay / disconnect / reset / halfClose / timeout`）。
响应体可为 `{"json": …}`、`{"text": …}` 或 `{"file": "responses/…"}`（相对 `tests/fixtures/`）。
**步骤耗尽且未声明 `repeatLast`、或没有匹配路由时，fixture 返回 501 并记录 script problem**，
不返回通用成功。
