# PRD completion · 独立 QA 验收报告

## Round 1 · PC-01 · PASS

- 日期：2026-10-02；角色：独立 QA，未参与本卡生产实现。
- 范围：`scope=slice`，仅 PC-01，REQ-PC-001～005 / AC-PC-001～010 / UI-PC-001～005。
- 依据：[PRD §2～6](prd.md) 冻结的 PRD revision 1 / UI revision 1；当前文件 revision 2 的后续卡不改变 PC-01 合同。[RD_READY 交接](implementation.md) 已读取。
- 结论：10/10 必选 AC 通过，未关闭验收缺陷 0。**只签发 PC-01 PASS；不签发全项目、T22/T23、其它卡或浏览器支持矩阵通过。**

### 环境与交付版本

- 工作树基线提交：`66cbdf6011ea6021e043bc688df0c5d88ac9e7fe`，验收对象包含 PC-01 未提交改动；逐生产文件 SHA-256 见 [环境指纹](../../../var/pc01-qa-round1/environment.json)。PC-06 在另一路实施，QA 未修改任何生产文件。
- macOS / Darwin 25.6.0 arm64；Playwright 1.60.0 的 Chromium 148.0.7778.96（build 1223），headless，zh-CN。375 / 1024 / 1440px 三档。这里记录实测版本，不声称为当前稳定浏览器支持矩阵。
- 在 RD_READY 后复制测试后端到忽略目录；SHA-256：`5c180815c777a29618857b95ce5ee6bd46afd7b5ec1eb58652ff342f74e78532`。QA 复用该副本，没有运行 Cargo 构建或占用后续 RD 的 target。
- 独立 Vite 端口 15473、随机本机后端/fixture 端口，临时 data-dir，固定虚构凭据，非本机浏览器请求拦截。未读取真实设置或用户配置，未访问/启停 8080/5173 用户预览。临时数据与服务由测试收尾；下载包、截图、摘要和日志均在 `var/pc01-qa-round1/`，已用 `git check-ignore` 确认忽略。
- 数据通过真实 HTTP 产生：本机 fixture 完成一次生成、确认并发布两个知识内容不同的版本，另建一个有不同 PDF 资产的其它物品用于排除性验证。原件为仓库原创 2 页 PDF，模型为有效 GLB；没有复制公开第三方说明书到仓库 fixture。
- trace、HAR、video 关闭。截图仅含隔离测试物品，无真实用户数据。浏览器没有请求收费服务；本机 fixture 的造数调用与导出期间零新增调用分开计数。

### 实际命令与结果

在 `apps/web` 执行以下独立 QA 浏览器命令，测试后端读取已复制的二进制，不重新构建：

```sh
PLAYWRIGHT_NO_COPY_PROMPT=1 EM_E2E_WEB_PORT=15473 EM_E2E_API_PORT=18480 \
  /Users/qsyj/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node \
  node_modules/@playwright/test/cli.js test --config playwright.pc01-qa.config.ts
```

| 验证 | 实际结果 | 证据 |
| --- | --- | --- |
| 独立浏览器套件，最终运行 | **5/5 PASS，27.1s**；首轮 4/4 后补足阅读选择保留/Object URL 释放，再运行全部 5 项形成一致证据 | [最终日志](../../../var/pc01-qa-round1/browser-run-final.log)、[独立测试](../../../apps/web/tests/e2e/qa-pc01-download.spec.ts)、[隔离后端](../../../apps/web/tests/e2e/qa-pc01-backend.ts) |
| `node node_modules/vitest/vitest.mjs run src/api/release-download.test.ts src/features/manual/ReleaseDownload.test.tsx src/api/client.test.ts` | **31/31 PASS，3 文件，1.33s**；独立重跑 RD 定向测试 | [单测日志](../../../var/pc01-qa-round1/unit-run.log) |
| 在仓库根运行已复制测试可执行文件：`var/pc01-qa-round1/backup-restore-tests export_contains_only_release_assets_and_no_secrets --exact` | **1/1 PASS，0.82s，10 filtered out**；真实导出白名单、401/404、秘密排除及临时文件清理定向回归 | [后端日志](../../../var/pc01-qa-round1/backend-export-run.log)；测试文件 SHA-256 `631687f75407f06bdd0590cb520f4ebd8e05cede49354eda9bdd8a0abf6aca67` |
| `node node_modules/eslint/bin/eslint.js playwright.pc01-qa.config.ts tests/e2e/qa-pc01-backend.ts tests/e2e/qa-pc01-download.spec.ts --max-warnings 0` | **PASS，0 warnings** | 本回合 QA 新增测试/配置的定向静态检查 |

本回合使用 Codex bundled Python 的 `zipfile`、`pypdf` 及独立 GLB chunk/JSON/buffer 检查器；实现见 [ZIP 检查器](../../../var/pc01-qa-round1/inspect_zip.py)。没有把只有 PDF/GLB 魔数正确当成内容有效。

### AC 验收矩阵

| AC ID | 期望与独立实测 | 结果 | 主要证据 |
| --- | --- | --- | --- |
| AC-PC-001 | 从旧版阅读器、新版列表分别通过浏览器真实下载，建议名与 ZIP 中 release ID 分别匹配所选版本；两个冻结 manifest 哈希不同。没有用最新草稿覆盖旧版 | PASS | 浏览器第 1 项；`older-reader.zip` / `newer-list.zip`；[摘要 archives](../../../var/pc01-qa-round1/evidence/summary.json) |
| AC-PC-002 | 真实空版本列表、延迟实体读取的 loading、真实不存在 release 的读取错误均无下载按钮；正常恢复后才出现已知实体入口 | PASS | 浏览器第 2 项；摘要 `emptyLoadingMissing` |
| AC-PC-003 | 两入口 × 三档宽度，真实导出响应门控等待，Tab 到入口，Enter 启动后重复 Space/Enter；每个 pending 均只发 1 请求、只触发 1 次该下载，显示文字等待状态，完成恢复同一操作 | PASS | 浏览器第 3 项；摘要 `keyboardAndFilenameMatrix`；[24 组几何/状态](../../../var/pc01-qa-round1/evidence/geometry.json) |
| AC-PC-004 | 网络错误、403、500、JSON、HTML、错误 ZIP MIME 内容、截断 ZIP 均无下载；两入口三档另执行真实后端 404，显示类别及 requestId。内部路径/HTML未回显，错误关联当前按钮，显式重试仍是原 release，成功清除错误 | PASS | 浏览器第 3/4 项；摘要 `recovery`；[375 阅读器真实404](../../../var/pc01-qa-round1/evidence/reader-real404-375.png) |
| AC-PC-005 | 真实默认名正确；长安全名在浏览器建议名和页面提示一致；无文件名、`../` 路径名、UTF-8 编码控制字符名均回退为同一 release 派生名。额外单测覆盖 UTF-8 中文、Windows 路径、NUL、双向字符、设备名及错误扩展名。仅显示已发起下载 | PASS | 浏览器第 1/3 项；31 项单测；[375 长文件名](../../../var/pc01-qa-round1/evidence/list-long-success-375.png) |
| AC-PC-006 | 失败时离开返回、等待时离开且晚响应释放后返回均恢复显式可下载，无旧页面成功提示/自动下载；401进入带原路径的登录，登录后无重放，显式操作才产生下载，pageerror=0 | PASS | 浏览器第 4 项；摘要 `recovery`；组件晚响应/取消定向单测 |
| AC-PC-007 | 两入口在首次下载之前即显示冻结用途全文，含原件/模型/manifest/哈希、数据便携与灾备、非整库备份及不承诺双击运行网站；等待/成功 role=status、失败 role=alert，状态均有文字 | PASS | 浏览器第 1/3 项及静态审查；24 组状态记录与 375 截图 |
| AC-PC-008 | 旧版本下载后修改草稿，再停掉 fixture 供应商后下载，release ID、冻结 manifest、PDF/GLB 哈希完全相同。隔离库 jobs/provider_attempts/cost_ledger 为 1/2/2 前后不变，金额/状态分组完全不变；所有 fixture 调用计数不变 | PASS | 浏览器第 1 项；摘要 `archives.before/after`；`older-offline-after-edit.zip` |
| AC-PC-009 | 两入口三档 idle/pending/长名 success/真实404共 24 组：无页面横向溢出，最小按钮 44px高/88px宽，用途14px；真实Tab到达且focus-visible至少2px，Enter/Space下载和重试保留焦点，aria-describedby均指向存在的用途/状态节点。375截图目视无控件遮挡。部件选中和原文第2页在1440→375下载→1440后保留 | PASS | 浏览器第 3/5 项；`geometry.json`；摘要 `readingContext`；375各状态截图 |
| AC-PC-010 | 核心成功响应未被合成或修改：浏览器取得非空真实 ZIP，独立验证CRC、唯一路径集合、顶层manifest、冻结release manifest哈希和文件哈希；PDF严格解析2页，GLB解析为v2/2chunks/1mesh，bufferViews界限有效，无外链buffer/image。包只含两manifest+PDF+GLB，无外来物品资产/草稿新内容/配置/会话/本地绝对路径/供应商临时URL | PASS | 浏览器第 1 项；[ZIP 摘要](../../../var/pc01-qa-round1/evidence/summary.json)；独立检查器；后端导出定向回归 |

### 关键事实与取证边界

- 最终隔离 release：旧 `01a0f8aa-d2bf-773a-81c9-5d1b09225e8a`，新 `01a0f8aa-d2d5-7221-8c84-074af646e2f3`；此临时库已由测试清理，身份和下载成果保留在摘要/ZIP中，不是用户预览数据。
- 每个核心导出包正好 4 个条目：两份 manifest、1216 字节的 2 页 PDF、2912 字节的 GLB。源资产摘要和实际解析信息均在 `summary.json`，没有只依靠页面成功文案判定导出内容。
- 顶层 `manifest.json.exportedAtMillis` 按导出时间变化，**不要求整个 ZIP 字节固定**。不可变判据是 release 身份、`release/manifest.json` 的原始字节哈希、资产集合与资产哈希；旧版本在草稿变更和供应商离线后均保持。
- 文件名边界测试明确使用真实后端 ZIP 响应体，仅注入不同 Content-Disposition 头；错误测试明确注入失败体。AC-PC-001/008/010 的核心成功链路完全透传真实后端，未用合成 ZIP 冒充。
- Object URL：完成后记录到 1 次 ZIP URL 创建/1 次释放；卸载取消及晚结果丢弃另由组件测试和真实站内导航覆盖。没有新后台打包任务/持久包缓存或自动重试。
- 服务端 fixture 的前置造数产生 1 job / 2 attempts / 2 ledger 行，它们是本机虚构链路；导出期间新增均为 0，不能把前置计数写成“整个环境从未有 job”。未验证真实供应商、费用模型或真实生成质量。
- 本轮未做 Linux、Firefox、Edge、Safari、Windows、最新浏览器矩阵或全站辅助技术认证；环境补充/登录 smoke 不能替代其验收。原 T22/T23 与后续 PC 卡保持各自状态。
- 现有 Node `NO_COLOR`、THREE.Clock deprecated 日志没有造成测试失败。本卡未扩大范围处理这些告警。

### 缺陷与交接

- 阻断缺陷：无。没有需要 RD 修复或重开的 PC-01 AC。
- 本轮没有修改生产代码、PRD、implementation、state 或用户配置；新增 QA 配置、隔离后端测试辅助与独立验收套件。报告及证据只陈述已经运行的验证。
- 交接协调者：可将 PC-01 按冻结 PRD/UI revision 1 记为 accepted，并继续下一卡；不能将本回合结果提升为全项目完成。

### Round 1 测试代码类型补正（2026-10-02）

- 集成检查发现独立 QA 测试在严格索引类型下未收窄 ETag 与首个部件；仅修正 `qa-pc01-download.spec.ts`，增加 ETag / 首个 fixture 部件必须存在的显式检查。下载、版本身份、内容与可用性断言未改变，未修改生产代码，不重开已通过的 PC-01 AC。
- 在 `apps/web` 执行 `/Users/qsyj/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node node_modules/typescript/bin/tsc --noEmit`：**PASS，exit 0，无诊断输出**。该结果是当前合并工作区的静态检查，不构成尚在实施的 PC-06 验收结论。

## Round 2 · PC-06 · PASS

- 日期：2026-10-02；角色：独立 QA，未参与 PC-06 生产实现。
- 范围：`scope=slice`，PRD/UI revision 2 §10 / §10.5，REQ-PC6-001～005、AC-PC6-001～010。
- 结论：**10/10 必选 AC 通过，未关闭验收缺陷 0，仅签 PC-06 PASS**。保留 round 1 的 PC-01 结论；不签发全项目、PC-02、T22/T23 或全浏览器支持矩阵通过。

### 冻结对象与隔离

- 真正提供页面的是协调者冻结的 `var/prd-completion/web-snapshots/pc06-rd-ready`，Playwright `webServer.cwd` 固定指向该目录，端口 **15476**。最终对 `source-hashes.json` 所列 **110 个文件**逐个复核，哈希差异 0；没有以 PC-02A 热改源码进行 PC-06 验收。
- 浏览器/HTTP 使用 `var/pc06-qa-round2/everything-manual-fixture`，SHA-256 `cdf2a8adf31522836efdef9b09fa041bf1a5fe81b90e6023144ced0fddea02a5`，debug + job-failpoints。仅是隔离测试构建，不冒称普通发行/preview 产物验证。
- Rust 回归先核验 7 个复制的测试二进制以及可能被 compile-time 路径引用的主 binary 指纹，再运行；结束后已告知协调者释放构建限制。本轮没有调用 Cargo。
- 所有后端、模型服务、CDN 使用新建 localhost fixture 与临时目录；环境变量采用允许列表，注入随机隔离主密钥和虚构 key。未读取用户配置/真实 model，未访问或重启用户 8080/5173 预览，未调用真实收费服务或用户 Keychain。
- 浏览器为 Playwright 1.60 / Chromium 148.0.7778.96；trace/HAR/video/自动截图关闭，手工证据只在问题值已由服务端隐藏、输入为空时保存。两张 375/1440 截图均已目视检查。[环境及指纹](../../../var/pc06-qa-round2/environment.json)、[二进制清单](../../../var/pc06-qa-round2/binaries.json)。

### 实际独立验证

| 命令 / 位置 | 结果与证据 |
| --- | --- |
| 仓库根：bundled Python 执行 `var/pc06-qa-round2/run-rust-regression.py`，内部直接运行复制 binary，完整命令在摘要 | **69 passed / 0 failed / 2 ignored**：model_guard_rd 6、generation_requests 30、pipeline `pc06_` 1、api_settings_rd 4、api_settings_qa 7、encrypted_secrets_rd 10、encrypted_secrets_qa 11。忽略的 2 项均为原有显式 native Keychain 集成，不使用 `--ignored`。[执行摘要](../../../var/pc06-qa-round2/rust-regression/summary.json) |
| `apps/web`：`PLAYWRIGHT_NO_COPY_PROMPT=1 node node_modules/@playwright/test/cli.js test --config playwright.pc06-qa.config.ts` | **7/7 PASS，24.6s**，完全独立编写的 HTTP / 浏览器用例。[最终日志](../../../var/pc06-qa-round2/browser-run-final.log)、[测试套件](../../../apps/web/tests/e2e/qa-pc06-models.spec.ts)、[隔离后端](../../../apps/web/tests/e2e/qa-pc06-backend.ts)、[冻结前端配置](../../../apps/web/playwright.pc06-qa.config.ts) |
| 冻结前端目录：`node node_modules/vitest/vitest.mjs run src/features/settings src/features/import/ConfirmStepPage.test.tsx src/features/jobs` | **36/36 PASS，5 文件，2.76s**；重跑交付版本的组件/分类/任务及确认页测试。[日志](../../../var/pc06-qa-round2/frozen-unit-run.log) |
| `apps/web`：`node node_modules/typescript/bin/tsc --noEmit`；`node node_modules/eslint/bin/eslint.js playwright.pc06-qa.config.ts tests/e2e/qa-pc06-backend.ts tests/e2e/qa-pc06-models.spec.ts --max-warnings 0` | 均 **exit 0，无诊断 / warning**。类型检查为当前合并工作区；PC-06 的运行和组件证据仍来自冻结前端。 |

上述 `node` 实际使用 `/Users/qsyj/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node`，Python 使用同运行时的 `python/bin/python3`。Rust 数量属于定向回归补充，不作为下列独立 10 AC 的替代。

### 逐 AC 结论

| AC | 独立观察及判据 | 结果 / 证据 |
| --- | --- | --- |
| AC-PC6-001 | 两家各 7 种高置信形态均由真实 PUT 拒绝，界面即时 password 遮蔽并 aria-invalid；包含 16 位边界、外层空白、Bearer 多空格/大小写、sk-proj、BOM、NEL。两家各 8 种非匹配自定义名可保存且界面恢复 text；15 位与 sk-local 未误伤。Bearer 后 tab 不分类为秘密但仍由控制字符规则拒绝 | **PASS**；第 1 场景，[classification](../../../var/pc06-qa-round2/evidence/classification.json) |
| AC-PC6-002 | 直接 PUT 问题模型并同时替换 key 返回固定字段 422；overlay 字节、revision、active 不变。匿名 401、错误 CSRF 403、过期修订 409 不绕过；坏 deployment restore 真实 422。无供应商调用，错误不回显输入 | **PASS**；第 1/3 场景；AS 原子写/权限/CAS 回归 |
| AC-PC6-003 | 使用真实服务进程加载三种旧 overlay key 模式 inherit/clear/encrypted replace，两家的 active/saved 均 model:null + modelIssue，来源 web 和 keyConfigured 保真。重复读取与 CLI check 前后整个 overlay 字节不变，pending=false。新正常值成功 PUT 后 active 仍遮蔽；没有利用读取隐式清空或改变模式 | **PASS**；第 2 场景，[legacy-modes](../../../var/pc06-qa-round2/evidence/legacy-modes.json) |
| AC-PC6-004 | 真正 TOML 问题配置可启动，CLI check 无原值且文件不变；环境模型问题在既有任务/阅读场景可启动。网页正常覆盖坏部署值，重启后显示正常 web 来源；恢复坏部署值被拒，恢复合法部署值后来源回 deployment | **PASS**；第 2/3/6/7 场景，CLI 冻结回归；[ui-correction](../../../var/pc06-qa-round2/evidence/ui-correction.json) |
| AC-PC6-005 | 初次空框非 dirty，保存禁用。只改另一家后保存仍明确定位问题模型，网络 PUT 数为 0；null/空字符串/空白直接请求均不充当清空授权。填写正常模型、明确 clearModel、恢复合法部署分别成功；清空确认/撤销保留独立 key 选择。恢复失败聚焦可见恢复区，取消后其它编辑与新 key 仍在本页。DOM 属性/文本、URL、storage、console 未出现 canary | **PASS**；第 1/2/3 场景，清空键盘及恢复错误证据 |
| AC-PC6-006 | key 仍已配置时，问题 provider 在 status 明确不可生成，页面给修正状态，health/ready 可用；新报价 422 providerModelInvalid，不触发登录。真实原件 content 返回 PDF 字节，发布 manifest 正常读取，真实阅读路由的 PDF.js canvas 绘制成功；只读期间 job/attempt/ledger 和供应商计数不增 | **PASS**；第 2/7 场景，[existing-reading-and-frozen-worker](../../../var/pc06-qa-round2/evidence/existing-reading-and-frozen-worker.json) |
| AC-PC6-007 | 活跃问题模型与目录中的问题 preset 模型分别在报价前 422，固定原因无原值插值。当前配置合法时，七类历史来源的确认重放/建单仍 422，jobs/attempts/ledger 为 0 且供应商为 0。单独坏 snapshot 场景：HTTP retry 422 quoteModelInvalid，手动排队的 worker 到 needs_input，不增加 attempt/预留/外呼。旧 unknown 的真实 HTTP attachRemoteTask 返回 200；readonly receipt 恢复与查询完成，替代购买仍拒绝 | **PASS**；第 4～7 场景，[price-interpolation](../../../var/pc06-qa-round2/evidence/price-interpolation.json)、[remote-reconciliation](../../../var/pc06-qa-round2/evidence/remote-reconciliation.json) |
| AC-PC6-008 | 五个 quote_json 公开模型位置、独立 provider_config、历史 confirmation_json 共七个来源逐项注入旧问题值。真实 GET 均输出安全模型状态；原始 quote/provider/confirmation 字符串及确认/消费事实不变，金额与报价身份不变。确认页使用上述真实 GET 安全副本作展示适配后显示不可用及设置入口；此 UI 适配明确不代替真实 GET/确认/建单断言 | **PASS**；第 5 场景，[historical-quotes](../../../var/pc06-qa-round2/evidence/historical-quotes.json) |
| AC-PC6-009 | 三种旧 key 模式的显式纠正均保留模式，replace 仍为密文；重启前 active 问题/saved 正常/pending 保真，重启后新 model 生效。浏览器真实 replacement 与模型纠正一并保存，密钥输入不公开。未决 unknown 下改配置拒绝 providerConfigBusy；旧报价/配置代次、CAS/保存并发与 keep/replace/clear 实际 key 语义由 AS/ES 和 model_guard 冻结回归补充 | **PASS**；第 2/3/6 场景，69 项定向回归 |
| AC-PC6-010 | 375/1440：页面 scrollWidth 分别等于 375/1440；真实 Tab 到达清空操作，Enter 打开，Tab/Space 确认，撤销后焦点返回模型。取消/确认关键按钮均 ≥44px，字段 label/aria-invalid/aria-describedby 可用；错误及提示换行、目视无关键控件遮挡。浏览器 external/pageerror 均 0；全部假值、本机 fixture | **PASS**；第 1/3 场景，[375 截图](../../../var/pc06-qa-round2/evidence/clear-375.png)、[1440 截图](../../../var/pc06-qa-round2/evidence/clear-1440.png) |

### 关键计数、审查闭环与限制

- 最终真实 HTTP 对账 job `01a0f8d9-9936-76a1-965c-edca847cd6a1`：准备阶段 fixture 计数 upload=2 / submit=1 / manual=1；坏模型启动后替代购买拒绝、attach 成功与 receipt 恢复令 task 查询 0→2、CDN 0→1，**submit 仍为 1**，manual/upload 不增加。这是本机虚构购买事实，不能写成真实供应商购买或收费证明。
- 最终坏配置阅读的 release `01a0f8d9-a47e-77e2-9d44-53a5a98c82ab`、source asset `01a0f8d9-a1f6-7478-b06f-ee047e512073`；临时库在测试结束清理，不是用户预览数据。该例先正常生成并发布，再切换问题部署模型，验证已存在资料仍可用。
- 历史数据测试只在 QA 临时库中植入旧格式事实；注入 quote/snapshot 时临时移除并立即恢复原 immutable triggers。冻结 worker 用例还在独立已完成任务中植入坏 snapshot、删除该阶段测试 attempt 并重排队以模拟尚未提交的旧阶段。零新增判据以注入后的明确基线比较；不是生产写入路径或真实用户数据迁移。
- 原始问题 overlay/quote 中的值属于有意输入事实；本卡不后台删除历史字节。新公开响应、诊断、DOM/URL/storage/console/服务日志采用内存布尔检查；证据只保存分类、结果、计数及测试 ID。没有保存用户秘密、响应 trace/HAR 或含输入值的自动截图。
- RD_READY 前只读 finding 已闭环：`verify_remote_task` 的 configured-model 误拦由第 6 场景实际 HTTP 对账验证修复；Rust/JS BOM/NEL trim 差异由第 1 场景两端逐项验证修复。它们是交付前审查发现，不伪造为本轮已交付版本 FAIL。
- 自有测试首次使用了错误的发布详情/阶段重试 URL，收到真实 404；已依据既有 API 合同修正为带 item 的 release 路由与 `/jobs/{id}/retry` + stageId，未放松断言。最后整套 7 项重新通过。一次组件日志 tee 相对路径错误导致包装命令 exit 1；组件本身 36 项通过，随后修正日志路径并重新运行得到 exit 0，以上仅计后次证据。
- 本轮未做真实付费 API、任意供应商秘密识别、历史秘密全库清理、普通发行 smoke、Linux/Firefox/Edge/Windows/最新浏览器矩阵或全站辅助技术认证。规则仅承诺冻结 PRD 的有限高置信匹配，公开访问和执行保护不意味着旧副本已被擦除。

### 缺陷与交接

- 正式验收阻断缺陷：**0**；未修改生产代码、PRD、state 或用户配置。
- 新增独立 QA 配置、隔离后端辅助和 7 场景测试；证据位于 Git 忽略目录 `var/pc06-qa-round2/`。Round 1 已保留。
- 交协调者：PC-06 可按 PRD/UI revision 2、round 2、scope=slice 记录 accepted；普通预览部署与后续卡仍由协调者按各自验证推进。

## Round 3 — PC-02A 原件独立阅读与出处导航（2026-10-02）

角色 QA；`scope=slice`；PRD/UI revision 2 §11/§11.3；REQ-PC2-001～003、AC-PC2-001～004、UI-PC2-001～004。结论：**PASS**，仅 PC-02A；不包含 PC-02B、后续卡或全项目发行验收。Round 1/2 保留。

### 冻结对象、隔离与真实命令

- Root 的 RD_READY 前端快照为 `var/prd-completion/web-snapshots/pc02a-rd-ready`，测试前后核对 `source-hashes.json` **114 文件全部匹配**；Vite cwd 固定该目录、端口 **15477**。生产 RD 并行推进下一卡，未使用 mutable `apps/web/src` 作为浏览器或组件验收对象。
- 复用冻结后端 `var/pc06-qa-round2/everything-manual-fixture`，SHA-256 `cdf2a8adf31522836efdef9b09fa041bf1a5fe81b90e6023144ced0fddea02a5`。只在新建临时目录初始化数据；两家 Provider 都未配置。未执行 Cargo、未读取用户 preview/settings、未停止 8080/5173；临时进程与数据在测试后清理。
- 独立测试为 `apps/web/tests/e2e/qa-pc02a-originals.spec.ts`、`qa-pc02a-backend.ts` 及 `playwright.pc02a-qa.config.ts`。真实 HTTP 完成物品创建、PDF 上传/绑定、文档分页、受保护资产和 404；没有 `seedReadyPreparation`、生成任务或伪 PDF 页图。跨文档场景仅拦截 draft/release **引用元数据**，资产始终是真实后端字节；这是导航合同测试，不是多 PDF 供应商生成证明。
- 浏览器阻止非 localhost HTTP(S)，没有 trace/HAR/video；官方原件、截图及执行元数据均仅在 Git 忽略目录 `var/`。官方样本沿用来源清单，公开访问不授予再分发许可。

以下命令使用 `/Users/qsyj/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node`：

| 真实执行命令与目录 | 结果 |
| --- | --- |
| `node node_modules/@playwright/test/cli.js test -c playwright.pc02a-qa.config.ts`，cwd=`apps/web`，实际 webServer cwd 为上述快照 | **6/6 passed，39.2s**，Chromium；含 6 个 consumer/width 组合中的 12 次部件/步骤导航 |
| `node node_modules/vitest/vitest.mjs run src/features/manual src/features/shell/PageLayout.test.tsx src/features/shell/Drawer.test.tsx src/features/viewer/OriginalDocumentPanel.test.tsx src/features/viewer/draft-view.test.ts`，cwd=冻结快照 | **6 文件 / 30 项通过，1.56s**；补充 PDF destroy/render cancel/cleanup、迟到文字、零非法 getPage、单抽屉 trap/Esc 与焦点 fallback。此处明确是既有冻结组件回归，不代替独立真实路由证据 |
| `node node_modules/typescript/bin/tsc --noEmit`，cwd=`apps/web` | exit 0；用于自有 QA 文件类型检查，不把该命令当作 mutable 生产功能验收 |
| `node node_modules/eslint/bin/eslint.js playwright.pc02a-qa.config.ts tests/e2e/qa-pc02a-backend.ts tests/e2e/qa-pc02a-originals.spec.ts --max-warnings 0`，cwd=`apps/web` | exit 0、无 warning |
| Python SHA-256 对冻结前端清单与固定 binary 复核 | 114/114、binary 一致；[执行摘要](../../../var/pc02a-qa-round3/frozen-verification.json) |

首次浏览器启动受沙箱 localhost `listen EPERM` 限制，获准以相同本机范围运行后启动成功。加入第二份官方原件后，自有「文件校验信息」选择器匹配两个节点，已收窄到 LACK 所在资料行；这是测试选择器修正，没有生产改动或放松断言。之后完整六场景通过。

### 逐项验收

| AC / UI | 独立执行证据 | 结论 |
| --- | --- | --- |
| AC-PC2-001 / UI001、002 | 在未配置 Provider、无 draft/release/preparation 的临时物品中，经真实概览「查看原件 · 标题」链接打开官方 LACK。实际标题、1/8→8/8、不同 canvas、LACK 文字层、末页「下一页」禁用、Tab/Enter 及返回原资料行焦点通过。官方 TR-808 另经真实入口读取 42 页，抽检第 1/42 页不同页图，明确无可读取文字层，不伪造文字。来源 URL 和 SHA 信息保留。另一个物品绑定 22 文档，两份目标位于默认 20 条之外；真实概览全部显示，分别打开不同原件并返回焦点 | **PASS**；场景 1～3，[官方数字原件](../../../var/pc02a-qa-round3/evidence/official-entry.json)、[扫描原件](../../../var/pc02a-qa-round3/evidence/official-scan.json)、[分页与不同资产](../../../var/pc02a-qa-round3/evidence/pagination-assets.json) |
| AC-PC2-002 / UI003 | 阅读器与复核页各在 375/1024/1440，从部件和步骤经真实 Tab/Enter 打开出处，标题聚焦；文档名、页图、文字与引用一致，两个实际 PDF 的第 1 页 canvas hash 不同。375 同刻一个 Drawer，1024 选中「原文」标签；键盘返回恢复精确来源按钮。1024→375→1440 保留手动选择的 document/page，返回仍定位原来源 entity。出处/返回按钮实测 ≥44px，页面无横向溢出；两张 375 截图目视控件可用 | **PASS**；场景 4，[引用矩阵](../../../var/pc02a-qa-round3/evidence/reference-matrix.json)、[复核页 375](../../../var/pc02a-qa-round3/evidence/draft-375.png)、[阅读器 375](../../../var/pc02a-qa-round3/evidence/release-375.png) |
| AC-PC2-003 / UI002～004 | 两份原件同页号内容不同，按 evidence.documentId 分别打开；缺 document 明确不可用且没有 canvas，页 99 越界显示固定错误且隐藏 canvas，不替换第一页/末页或绘制伪 bbox。显式手动选择另一份原件回第 1 页。输入 0/1.5/9/text 后 aria-invalid/字段错误、旧有效第 8 页及 URL 不变；上述四种直接非法 URL 先显示不可用并隐藏页图，只有明确输入合法页才恢复。合法页码及前后页内容一致 | **PASS**；场景 1/2/4/5；冻结 `OriginalDocumentPanel` 回归补充非法引用零 getPage |
| AC-PC2-004 / UI001、002、004 | 把原件读取转到真实后端缺资产端点，局部 404 有诊断 requestId；Tab/Enter 重试仍是同文档/页。禁用 WebGL 后 3D 显示不可用，PDF 正常绘制、键盘翻页。375 Esc 原文→原来源（一个 Drawer）→关闭，焦点正确。延迟第一份真实 PDF 响应，切到第二份后释放迟到响应，选中标题/文字/canvas 不被旧资产覆盖。每次只有一个原件 canvas；PDF destroy/cancel/cleanup 由实际执行的冻结单测补充。六项均无 preparation/job/attempt/ledger/draft/release 新增，供应商与外网请求为零 | **PASS**；场景 1/3/5/6，[404/WebGL/键盘](../../../var/pc02a-qa-round3/evidence/local-failure.json)、[迟到响应](../../../var/pc02a-qa-round3/evidence/late-response.json) |

### 数据身份、边界与交接

- 最终官方样本临时物品 `01a0f8ed-3fde-72eb-894e-90c8452711a0`；LACK 文档 `01a0f8ed-4018-7529-a002-2fe273467fa0`、SHA `963c6e96c0c6773085769764df3aa73f74ffbd2841ef4f7dbc1b4b0269a21a8d`；TR-808 文档 `01a0f8ed-4173-765e-b0fd-d9502b34eac5`、SHA `bf5e15408c3aee59fd43135ee51834daaeec6cd54a7fb118984727d15e22e7fb`。身份属于已清理的 QA 库，不是用户预览物品。
- 多原件临时物品 `01a0f8ed-4280-72e8-bce0-ecfb8db1bc40`；目标文档 `01a0f8ed-4295-705c-9df4-fd67a7bca23a` / `01a0f8ed-42a9-77fd-9cd2-e0fb14bb9c9a`。资产是仓库自制 `sample-manual-text.pdf`（2 页）与 `sample-manual-nonlatin.pdf`（1 页）。跨文档、缺失/越界引用的 release/draft 元数据明确为 synthetic；数据库 draft/release 仍为 0。
- 真实官方入口六项数据库计数前后均 0；全套 fake-provider upload/submit/task/manual/CDN 都为 0，浏览器 external/pageerror 为 0。只有 PDF.js 本地 worker、字体/字符资源参与阅读。扫描手册只验证抽样页原件阅读和空文字层；没有 OCR/LLM 解析质量或全部 42 页语义正确的结论。
- 冻结主题样式明确正文 14px、页码/辅助 12px、翻页/跳转/返回/出处 44px，截图及浏览器实际来源/返回控件检查一致。本轮不是完整辅助技术认证、性能长时压测、最新浏览器矩阵、普通发行构建或真实供应商验收；不得据此关闭 T22/T23。
- 正式验收阻断缺陷：**0**。没有修改生产、PRD、state 或用户配置。交协调者：可按 PRD/UI revision 2、round 3、scope=slice 记录 **PC-02A accepted**；后续原件 preview 部署与 PC-02B 由各自交付继续推进。

## Round 4 — PC-03A 准备记录发现与恢复（2026-10-02）

角色 QA；`scope=slice`；PRD/UI revision 2 §§12.1/12.3/12.4；REQ-PC3-001～003、AC-PC3-001～005、UI-PC3-001～004。结论：**FAIL**，阻断缺陷 **BUG-PC3-001（P2）**；不得接受 PC-03A。Round 1～3 保留。本结论不涉及 PC-03B、PC-02B 或全项目发行验收。

### 冻结对象、隔离与真实命令

- 浏览器与组件测试均以 `var/prd-completion/web-snapshots/pc03a-rd-ready` 为目标；Vite cwd 固定该快照、端口 **15485**。前后复核 `source-hashes.json` **115/115 文件匹配**，未以并行 RD 的 mutable 前端作验收。
- 后端为 `var/pc03a-qa-round4/everything-manual-fixture`，SHA-256 `d7ec4440e17ba35cc33eb6b6e6f52325d304e63b4daea3df20e529edce9984f7`，schema 8、job-failpoints、无 embedded-ui。主程序及 4 个 Rust 测试副本与 `binaries.json` **5/5 指纹匹配**；Rust 来源清单由 root 冻结。先核对编译期 `target/debug/everything-manual` 相同指纹，再执行可能使用该路径的 CLI/storage 测试；结束即通知 root 释放 Cargo/target 锁，后续只使用 QA 副本。
- 独立测试：`qa-pc03a-api.spec.ts`（5 项）、`qa-pc03a-browser.spec.ts`（3 项）、`qa-pc03a-backend.ts`、`playwright.pc03a-qa.config.ts`。每组使用私有临时库、最小假值环境和 localhost fixture；不读用户 preview/settings，不访问 8080/5173，不运行 Cargo，不调用真实供应商。禁用 trace/HAR/video，浏览器阻止外网请求，测试进程和临时库在结束时清理。
- API 历史兼容与排名数据是隔离库 SQL fixture，引用经真实 HTTP 上传的仓库自有 JPEG/文字；它验证存储格式与归属，不冒称 PDF 提取。浏览器恢复测试另由真实 PDF.js worker 处理自制 PDF，真实上传/PUT/complete，无 `seedReadyPreparation` 或成功准备元数据 mock。

以下 Node 命令使用 `/Users/qsyj/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node`：

| 真实执行命令与目录 | 结果 |
| --- | --- |
| `var/pc03a-qa-round4/config_cli-tests --test-threads=1`，cwd=仓库根；其余分别替换为 `storage-tests`、`preparations-tests`、`generation_requests-tests` | **12 + 15 + 11 + 31 = 69/69 passed**；日志在同目录的 `config_cli.log`、`storage.log`、`preparations.log`、`generation_requests.log` |
| `node node_modules/vitest/vitest.mjs run --config /Users/qsyj/Code/rust/everything-manual/apps/web/vitest.pc03a-qa.config.ts src/features/import/ConfirmStepPage.test.tsx src/features/import/pdf/prepare.test.ts src/features/import/pdf/errors.test.ts src/features/import/messages.test.ts`，cwd=冻结快照 | **4 文件 / 24 项通过，2.48s**；既有冻结回归，仅作补充 |
| `node node_modules/@playwright/test/cli.js test -c playwright.pc03a-qa.config.ts`，cwd=`apps/web` | **7 passed / 1 failed，38.8s**：5 API 与前 2 个浏览器场景通过；第 3 个在正常键盘切换后失败 |
| 同上附加 `--grep 'mobile radios'`，最小复现两次 | 均失败；最终 **21.3s**。补充 activeElement、details 展开状态和旧节点连通性证据，没有修改生产或放松键盘断言；复现次数不计作新增验收项目 |
| `node node_modules/typescript/bin/tsc --noEmit -p ../../var/pc03a-qa-round4/qa-pc03a-tsconfig.json`，cwd=`apps/web` | exit 0；仅检查独立 QA 文件，不替代生产静态验收 |
| `node node_modules/eslint/bin/eslint.js playwright.pc03a-qa.config.ts vitest.pc03a-qa.config.ts tests/e2e/qa-pc03a-backend.ts tests/e2e/qa-pc03a-api.spec.ts tests/e2e/qa-pc03a-browser.spec.ts --max-warnings 0`，cwd=`apps/web` | exit 0、无 warning |
| Python SHA-256 复核固定快照与 5 个 binary 副本 | 全部匹配；[执行摘要](../../../var/pc03a-qa-round4/frozen-verification.json) |

冻结组件首次运行有 11 项通过、2 个 suite 因 symlink 依赖的 PDF worker `?url` 被 Vite 文件访问范围拒绝而未加载。新增 **QA 专用** `vitest.pc03a-qa.config.ts`，只允许冻结目录和 node_modules 的实际路径，重新执行后得到上述 24/24；没有更改冻结源码或把环境加载失败记成产品通过。

### 逐项验收

| AC / UI | 独立执行证据 | 结论 |
| --- | --- | --- |
| AC-PC3-001 / UI001 | 111 条真实 HTTP 资产支撑的历史记录，跨服务端 100 条批次及 HTTP 游标共 16 页，ID 无重复遗漏；每页推荐一致。分别验证 ready 优先、有效页数、更新时间、ID 排序，推荐不取当前页局部结果。认证后比较所有数据库表的 logical digest 与计数，重复 GET 前后零变化。全新无缓存 context 从准备页及确认页发现 preparing/ready | **PASS**；[全局推荐](../../../var/pc03a-qa-round4/evidence/api-global-ranking.json)、[真实恢复](../../../var/pc03a-qa-round4/evidence/browser-real-resume.json) |
| AC-PC3-002 / UI002、003 | 真实 2 页 PDF：首个页 PUT 等待时停止，释放后仅第 1 页持久化；新 context 显示「已完成 1 页，总页数待读取原件」，无 progressbar/伪 0%，无 PDF/资产读取或写入。用户继续后仅 PUT 第 2 页，旧第 1 页 DB 事实与资产 hash 不变，实际显示 2/2；未自动新建或封存，显式封存后进入确认。新 context 读取 ready，不重新准备、写页或读取资产 | **PASS**；[真实恢复](../../../var/pc03a-qa-round4/evidence/browser-real-resume.json)、[375 截图](../../../var/pc03a-qa-round4/evidence/real-prepared-375.png) |
| AC-PC3-003 / UI001、002、004 | API 验证旧 NULL 版本有可核实页时兼容 v1、空 NULL 明确不兼容且不回填；未来版本、原件 SHA 错、viewport/rotation 错、缺页、页超总数、错 purpose、跨物品图/文字、缺失/截断实体文件均拒绝复用。GET 不修库；不兼容 estimate 真 422、跨物品真 404。默认 POST 复用不变，显式 createNew 生成 v1 且旧 ready/NULL 记录不变。浏览器已验证另一 document 的缓存 ready 不能带到新原件，后者真实独立准备成功。缺失缓存、显式错误 URL preparationId、迟到旧 document 响应的后续步骤被键盘缺陷阻断，尚未执行 | **部分通过，未完成**；[兼容](../../../var/pc03a-qa-round4/evidence/api-compatibility.json)、[实体资产](../../../var/pc03a-qa-round4/evidence/api-physical-assets.json)、[新建与 CAS](../../../var/pc03a-qa-round4/evidence/api-create-cas.json) |
| AC-PC3-004 / UI002～004 | Stop 等待在途 PUT 并保留真实第一页；active 时原件选择禁用且提示先停止。同内容并发 HTTP PUT 返回 200/200、旧 ETag 的改页与 complete 均 412、ready 后 PUT/complete 均 422 且事实不变。浏览器封存前由第二真实 API 客户端改 revision，原请求得到真实 412，界面停止并要求重新读取；第二客户端封存成功后，键盘重读恢复 ready，仅 GET，不覆写或重建。所有执行到的场景 job/attempt/ledger/quote 和供应商计数均 0，原上传/complete Rust 回归通过 | **PASS**；[新建与 CAS](../../../var/pc03a-qa-round4/evidence/api-create-cas.json)、[真实 412](../../../var/pc03a-qa-round4/evidence/browser-real-conflict.json)；并发覆盖为 UI + 独立 API 客户端，不冒称两个完整 UI 标签页同时渲染 |
| AC-PC3-005 / UI001、002、004/键盘 | 非法/重复/未知 query 422、跨 document cursor 422、缺 document 404、匿名 401、CSRF 403、source 不符 422，完整 DB logical digest 不变；独立 QA 使用生成 DTO 且类型检查通过。375 下真实 Tab/Enter 可开始、停止、继续、封存、下一步、展开与加载 22 条记录，检查到的关键控件 ≥44px，成功准备页无横向溢出。**ArrowDown 选择另一兼容记录后 radio 被卸载，焦点落 BODY，展开列表关闭，无法正常 ArrowUp 返回**；后续真实 404 读取失败/重试步骤尚未执行 | **FAIL：BUG-PC3-001**；[安全读取](../../../var/pc03a-qa-round4/evidence/api-read-security.json)、[焦点证据](../../../var/pc03a-qa-round4/evidence/browser-radio-focus.json) |

### BUG-PC3-001 — 切换准备记录卸载 radio，丢失键盘焦点（P2）

- 位置：冻结前端 `src/features/import/preparation-discovery.tsx:54` 把选中 detail 的 pending 合并进 `loading`；同文件 `:115` 用 `!state.loading` 包住整个推荐卡与其他记录列表。对应生产源路径为 `apps/web/src/features/import/preparation-discovery.tsx`。不是分页返回空数据或模拟成功响应造成。
- 最小步骤：375px 打开真实 ready 的原件，展开「其他准备记录」，加载 22 条记录；Tab 聚焦推荐 radio，按 **ArrowDown** 选择下一个兼容 preparing 记录，等待 detail GET 返回，再按 **ArrowUp**。
- 预期：仅切换选择并读取对应详情；列表保持展开，焦点仍在选中 radio，ArrowUp 可以回到推荐 ready；等待详情期间不允许不安全动作。
- 实际：详情 pending 导致整个列表卸载再挂载。真实证据为 `activeTag=BODY`、`checkedValue=qa-ui-empty-20`、`otherRecordsExpanded=false`、`originalRadioNodeConnected=false`；原正常 ArrowUp 返回 ready 的断言失败，补充 `toBeFocused` 的两次独立复现同样失败。测试位置 `qa-pc03a-browser.spec.ts:80`～`:82`。固定 [JSON 证据](../../../var/pc03a-qa-round4/evidence/browser-radio-focus.json) 不含用户数据。
- 影响：违反 §12.4 原生 radio 键盘操作和「焦点不因后台发现结果自行跳走」要求，阻断 AC-PC3-005；记录仍可通过重新展开/鼠标操作选择，不涉及已保存页丢失或费用。已告知 root/RD；QA 未改生产代码。

### 数据身份、限制与交接

- 真实 PDF.js 恢复临时物品 `01a0fa64-3b8b-76d1-9c1a-7d20f6658f33`、文档 `01a0fa64-3ba2-7072-b855-9c6659d36653`、准备 `01a0fa64-4574-709e-a0b5-e85bf5af6e79`。两页 JPEG/文字实际回读，JPEG magic、MIME、viewport 1190×1684/rotation 0、页文字及 asset blob SHA 全部核对；逐页资产 ID/hash 在 evidence JSON。临时库已清理，不是用户 preview 数据。
- 真 412 场景准备 `01a0fa64-6086-7097-b40f-e0921610e5f9`；第二客户端只修改隔离记录 viewport 来产生真实 CAS 冲突，随后实际封存；恢复前后 ready DB 事实一致。没有拦截并伪造 412 或成功准备响应。
- 历史 SQL clones/不兼容坏事实只存在 QA 私有库。实体文件缺失和截断检查不代表检测了任意同长度内容损坏；浏览器页提取另以真实 PDF.js hash 证据证明。没有 OCR/官方手册解析质量、真实付费 API、普通发行构建、完整浏览器矩阵或全站辅助技术认证的结论。
- **待修复后补验**：选中 radio 的焦点与列表展开保留、正常 ArrowDown/ArrowUp；最后场景余下的失效缓存 ID、显式跨 document preparationId 恢复、真实 404 重试、迟到旧 document 响应及其焦点保持。不得把这些未执行步骤写成通过。
- 所有冻结回归与独立结果均如上保存；正式阻断缺陷 **1**。交协调者：保持 PC-03A 未接受，修复后提供新的前端冻结对象，再复验独立浏览器场景及受影响组件；不得由本轮 7/8 或 RD 自测替代全部 5 AC 闭合。QA 没有修改 PRD/state/生产/用户配置。

## Round 5 — PC-03A A2 修复复验（2026-10-02）

角色 QA；`scope=slice`；PRD/UI revision 2 §§12.1/12.3/12.4；REQ-PC3-001～003、AC-PC3-001～005。结论：**PASS，仅 PC-03A**；**BUG-PC3-001 已关闭**。Round 4 的失败与证据原样保留，本节是新冻结对象的复验，不追溯改写 A1 结论。不包含 PC-02B、PC-03B、普通发行或全项目验收。

### 冻结范围与执行结果

- A2 前端：`var/prd-completion/web-snapshots/pc03a-a2-ready`，manifest SHA-256 `b0348e56c1fcf98cb5e897cc5addea3bd518c08670db895035da4fa97ae62b2a`；测试前后 **115/115 文件匹配**。与 A1 比较仅 `src/features/import/preparation-discovery.tsx` 改变；未接入的 PC-02B 文件不在冻结对象中。Vite cwd 固定 A2、端口 15485。
- 后端继续使用 round4 副本，SHA-256 `d7ec4440e17ba35cc33eb6b6e6f52325d304e63b4daea3df20e529edce9984f7`，前后指纹一致。后端/Rust 无变更，**沿用 round4 已执行的 69/69 Rust 证据，本轮未重复运行或 Cargo 构建**。
- 新增独立 `playwright.pc03a-a2-qa.config.ts`、`vitest.pc03a-a2-qa.config.ts`；后端 helper 仅允许由 QA 配置指定 round5 证据目录，默认 round4 行为不变。重跑原 5 API + 3 browser 场景；原键盘断言保留，并增加真实 detail GET 等待期间的 DOM/焦点/动作门禁断言。所有新证据独立写入 Git 忽略目录 `var/pc03a-qa-round5/`。
- 私有临时库、本机假值 fixture、非 localhost 请求阻断和 trace/HAR/video 禁用与 round4 相同；不访问用户 preview/settings、8080/5173 或真实供应商。所有独立测试进程已退出，临时数据已清理。

以下 Node 命令仍使用已记录的 Codex Node 绝对路径：

| 真实命令 | 结果 |
| --- | --- |
| `node node_modules/@playwright/test/cli.js test -c playwright.pc03a-a2-qa.config.ts`，cwd=`apps/web` | **8/8 passed，27.5s**，Chromium；无重试 |
| `node node_modules/vitest/vitest.mjs run --config /Users/qsyj/Code/rust/everything-manual/apps/web/vitest.pc03a-a2-qa.config.ts src/features/import/ConfirmStepPage.test.tsx src/features/import/pdf/prepare.test.ts src/features/import/pdf/errors.test.ts src/features/import/messages.test.ts`，cwd=A2 快照 | **4 文件 / 24 项通过，3.68s** |
| `node node_modules/typescript/bin/tsc --noEmit -p ../../var/pc03a-qa-round5/qa-pc03a-a2-tsconfig.json`，cwd=`apps/web` | exit 0；含新 QA 配置和全部独立测试 |
| `node node_modules/eslint/bin/eslint.js playwright.pc03a-a2-qa.config.ts vitest.pc03a-a2-qa.config.ts tests/e2e/qa-pc03a-backend.ts tests/e2e/qa-pc03a-api.spec.ts tests/e2e/qa-pc03a-browser.spec.ts --max-warnings 0`，cwd=`apps/web` | exit 0、无 warning |
| Python 对 A2 manifest、115 文件、A1→A2 差异和固定 binary 的 SHA-256 复核 | 一致；[最终指纹和执行摘要](../../../var/pc03a-qa-round5/frozen-verification.json) |

### 逐 AC 复验

| AC / UI | 本轮实际证据 | 结论 |
| --- | --- | --- |
| AC-PC3-001 / UI001 | 再次建立 111 条历史准备记录，完整跨 batch/游标验证全局推荐和 ready/有效页数/时间/ID 优先级；每页一致，GET 全库 logical digest 零变化。全新无缓存 context 在准备/确认页发现服务端已有 preparing 和 ready；22 条 UI 记录可键盘展开、加载更多 | **PASS**；[全局推荐](../../../var/pc03a-qa-round5/evidence/api-global-ranking.json)、[发现边界](../../../var/pc03a-qa-round5/evidence/browser-discovery-boundaries.json) |
| AC-PC3-002 / UI002、003 | 真实 PDF.js 第 1 页在停止后保留；新 context 未读取 PDF 前显示未知总页数且零写，继续只 PUT 第 2 页，第 1 页 DB/资产不变；真实 x/2、显式封存、ready 新 context 确认均重过。回读两页 JPEG/文字与 blob SHA 一致，不依赖伪页 | **PASS**；[真实继续与页 hash](../../../var/pc03a-qa-round5/evidence/browser-real-resume.json) |
| AC-PC3-003 / UI001、002、004 | 全部旧 NULL/未来版本/SHA/viewport/缺页/错 purpose/跨物品资产/实体文件缺失与截断、estimate 拒绝、createNew 保留旧 ready 的 API 场景再过。浏览器跨 document 缓存不会带入旧 ready；本次完整走到缺失缓存提示、显式跨 document URL preparationId 阻断并由明确按钮采用当前推荐、迟到旧 document 真实响应不覆盖新选择；第二份 PDF 的 ready 事实不变 | **PASS**；[兼容](../../../var/pc03a-qa-round5/evidence/api-compatibility.json)、[实体资产](../../../var/pc03a-qa-round5/evidence/api-physical-assets.json)、[发现边界](../../../var/pc03a-qa-round5/evidence/browser-discovery-boundaries.json) |
| AC-PC3-004 / UI002～004 | Stop 保留和 active 原件切换门禁、同页并发 200/200、stale PUT/complete 412、ready 写拒绝均重过。UI 与独立 API 客户端实际 revision 冲突后真实 412，第二客户端封存，UI 只读取恢复 ready；不新增或覆盖已封存准备。执行结束 job/attempt/ledger/quote 与 localhost 供应商计数均 0；未变后端的 69 Rust 通过记录保留 | **PASS**；[新建与 CAS](../../../var/pc03a-qa-round5/evidence/api-create-cas.json)、[真实 412](../../../var/pc03a-qa-round5/evidence/browser-real-conflict.json) |
| AC-PC3-005 / UI001、002、004/键盘 | 非法 query/cursor、认证/CSRF/归属错误及零写 API 场景重过。375px 原生 Tab/Enter/ArrowDown/ArrowUp 全链通过，detail pending 与完成后焦点/展开保持，准备/封存/下一步门禁正确。真实缺 document 端点返回 404，界面是读取失败而非 0 页/未准备，Tab/Enter 重读恢复实际 ready 且无业务写/资产读取；迟到响应返回后原件选择控件仍聚焦。关键操作 ≥44px，已核对截图与无横向溢出 | **PASS**；[安全读取](../../../var/pc03a-qa-round5/evidence/api-read-security.json)、[等待门禁](../../../var/pc03a-qa-round5/evidence/browser-radio-pending.json)、[响应后焦点](../../../var/pc03a-qa-round5/evidence/browser-radio-focus.json)、[375 列表](../../../var/pc03a-qa-round5/evidence/discovery-records-375.png) |

### BUG-PC3-001 关闭依据及限制

- 原生 ArrowDown 触发真实 detail GET 时暂停该请求：原推荐 radio `isConnected=true`、选中 radio 聚焦、`details open=true`；准备/封存按钮隐藏，下一步链接不可用。释放原请求后 `activeTag=INPUT`、`activeValue=checkedValue=qa-ui-empty-20`、原节点仍连接且列表仍展开；**原生 ArrowUp 成功回到 ready**。未使用 `.check()`、强制 focus 或重开列表绕过原问题。
- 本轮先前未执行的四类边界全部完成：失效缓存、显式错误 preparationId、真实 404 读取重试、迟到旧 document 响应。最后场景证据物品 `01a0fa71-a780-7026-99d1-fa8a7839ce4e`，文档/准备分别为 `01a0fa71-a78e-7704-a88a-c87065ab77f2` / `01a0fa71-af30-7033-9e3c-83e18a73e212` 与 `01a0fa71-a7a0-7718-a86e-d72c348f23cf` / `01a0fa71-bb6e-745e-89f3-c49e2c150482`。它们来自本轮已清理的私有 QA 库。
- 真实断点恢复准备为 `01a0fa71-8883-750d-ae7e-a0231d13976c`，仅 PUT 序列 `[1]`→新 context `[2]`，2 张页图/2 份文字均已实际回读并核对 hash。API 排名/兼容历史仍明确为 SQL fixture；并发浏览器证据仍是 UI + 第二真实 API 客户端，不声称两个完整 UI 页面同时准备。
- Round4 的其他范围限制仍适用：无真实付费供应商、官方手册语义/OCR 质量、同长度任意文件损坏、普通发行构建或全部浏览器矩阵结论。没有读取用户数据或修改生产/PRD/state；普通候选构建与部署由 root 单独负责。
- 正式阻断缺陷 **0**。交协调者：可将 **PC-03A A2 / round5 / scope=slice / PRD UI revision2** 记录为 accepted；本轮没有为其它卡签署 PASS。

## Round 6 — PC-02B 发布待办与复核恢复（2026-10-02）

角色 QA；`scope=slice`；PRD/UI revision 2 §§11.2/11.4；REQ-PC2-004～005、AC-PC2-005～008、UI-PC2-005～008。结论：**PASS，仅 PC-02B**；正式阻断缺陷 **0**。同时对受 PageLayout/document-navigation 改动影响的 **PC-02A 完成实际浏览器复验，PASS，可恢复 accepted**。Round 1～5 保留；本轮不签 PC-03B、普通发行或全项目验收。

### 冻结对象、隔离与实际命令

- 前端固定为 `var/prd-completion/web-snapshots/pc02b-rd-ready`，manifest SHA-256 `cdd4561700e236365e4d59f6c14814ebd9ed7f6a6aec5fd4192afbfba47e3145`。测试前后 **118/118 文件匹配**。PC-02B 浏览器端口 15486、PC-02A 受影响回归端口 15477，两个配置的 webServer cwd 均明确指向 **B 快照**；没有使用旧 A 快照或 RD 后续热改源码作验收目标。
- 两套浏览器均使用 `var/pc03a-qa-round4/everything-manual-fixture`，schema8、job-failpoints、无 embedded-ui，SHA-256 `d7ec4440e17ba35cc33eb6b6e6f52325d304e63b4daea3df20e529edce9984f7`。指纹前后一致，Rust/API 合同未变；本轮没有 Cargo 构建或重复计入旧 Rust 结果。
- 新独立测试为 `qa-pc02b-api.spec.ts`（2 项）、`qa-pc02b-browser.spec.ts`（4 项），配套私有后端/真实 HTTP fixture；通过 localhost 假供应商任务链实际生成草稿、validated 模型和知识。后续确认、修订、热点、视角、发布正常成功响应均来自真实 Rust API，没有成功草稿/发布 metadata mock。只在私有临时目录操作假值；不读 preview/settings、8080/5173 或用户配置，不调用真实供应商。
- PC-02A 复验使用原 6 项 `qa-pc02a-originals.spec.ts` 和新 `playwright.pc02a-on-pc02b-qa.config.ts`；自有旧 helper 仅增加可选 binary/output 参数，默认旧轮次行为保留。本轮原件回归也使用 schema8 副本、独立 round6 输出，供应商未配置。
- 无 trace/HAR/video，外网请求阻断；官方原件仅复用已有 Git 忽略目录中的本机缓存，未新增下载或提交第三方 PDF。全部测试进程结束并清理自有临时库；前轮证据未覆盖。最终指纹及计数摘要：[frozen-verification.json](../../../var/pc02b-qa-round6/frozen-verification.json)。

以下 Node 命令使用前轮记录的 Codex bundled Node 绝对路径：

| 真实执行命令与目录 | 结果 |
| --- | --- |
| `node node_modules/@playwright/test/cli.js test -c playwright.pc02b-qa.config.ts`，cwd=`apps/web` | 最终完整 **6/6 passed，56.4s**，Chromium，无重试；此前定向调试不重复计数 |
| `node node_modules/@playwright/test/cli.js test -c playwright.pc02a-on-pc02b-qa.config.ts`，cwd=`apps/web`，实际 webServer cwd=B 快照 | **6/6 passed，41.4s**；是本轮真实受影响回归，不借用 round3 旧结果 |
| `node node_modules/vitest/vitest.mjs run --config /Users/qsyj/Code/rust/everything-manual/apps/web/vitest.pc02b-qa.config.ts src/features/manual/review-tasks.test.ts src/features/manual/useDraftMutations.test.tsx src/features/manual/review-state.test.ts src/features/shell/PageLayout.test.tsx src/features/shell/Drawer.test.tsx src/features/viewer/OriginalDocumentPanel.test.tsx`，cwd=B 快照 | **6 文件 / 26 项通过，2.91s**；任务映射/顺序、PATCH+GET 锁和失败后核对、发布/几何规则、Drawer 焦点及原文释放/非法引用回归 |
| `node node_modules/typescript/bin/tsc --noEmit -p ../../var/pc02b-qa-round6/qa-pc02b-tsconfig.json`，cwd=`apps/web` | 最终 exit 0；覆盖自有 B 测试、冻结 unit 配置和 A 回归配置/helper |
| `node node_modules/eslint/bin/eslint.js playwright.pc02b-qa.config.ts vitest.pc02b-qa.config.ts playwright.pc02a-on-pc02b-qa.config.ts tests/e2e/qa-pc02b-backend.ts tests/e2e/qa-pc02b-fixture.ts tests/e2e/qa-pc02b-api.spec.ts tests/e2e/qa-pc02b-browser.spec.ts tests/e2e/qa-pc02a-backend.ts tests/e2e/qa-pc06-backend.ts --max-warnings 0`，cwd=`apps/web` | 最终 exit 0、无 warning |

### 逐 AC 结论

| AC / UI | 独立验收事实 | 结果 / 证据 |
| --- | --- | --- |
| AC-PC2-005 / UI005、006 | 真实草稿的部件、步骤、规格共 3 个未确认事实、2 个模型声明、缺热点及可选视角分别显示正确分组计数，默认只看未完成。375/1024/1440 用 Tab/Enter 逐项定位实际实体及控件，原文来源打开真实 PDF 后返回原按钮；缺热点/视角在 375 聚焦 ≥768px 说明且几何控件禁用，导航零 PATCH/POST、草稿 revision/内容不变。另在私有库植入旧模型 SHA/revision 的 stale 锚点，小屏只解释并禁止重绑，不冒充有效热点。断点切换保留当前实体，单一 Drawer；关键操作 ≥44px，无横向溢出，375 截图已目视检查 | **PASS**；[导航矩阵](../../../var/pc02b-qa-round6/evidence/navigation-matrix.json)、[375 stale 说明](../../../var/pc02b-qa-round6/evidence/stale-task-375.png) |
| AC-PC2-006 / UI005、007 | 部件→步骤→规格每次真实确认后 GET 回读，revision 各 +1，计数 3→2→1→0，只有目标实体 review 改变，其他实体/知识/模型/几何不被代办。当前区域保留「此项已处理」；切换筛选可见真实已确认项且不丢上下文；只有显式下一项才前进。外部 API 完成当前项后显式重读显示目标变化，保持当前实体、无随机下一项。模型打开/核对分别提交，第一次 userConfirmed=false；末项给返回发布区。全部满足仍 release=0，375 键盘明确发布才变 1。单独 API 场景验证可选视角未设置也能发布、同键重放仍只有同一 release，后续草稿编辑不改 manifest 字节 | **PASS**；[显式下一项](../../../var/pc02b-qa-round6/evidence/explicit-next.json)、[真实显式发布及冻结 manifest](../../../var/pc02b-qa-round6/evidence/api-explicit-release.json) |
| AC-PC2-007 / UI008 | 不完整草稿真实 publish 422 包含 modelReviewMissing、三个 knowledgeUnreviewed、hotspotMissing，且无 release/草稿变更。将该真实错误作为明确负向重放交 UI，已知实体可定位；未知 code、无目标及带 HTML/远程地址的恶意 identity/message 单独负向注入，仍保留问题/requestId、无虚构去处理按钮、无注入 DOM 或外网请求。**GET 后诊断仍在并说明未确认解决**；仅显式「重新检查并发布（通过后生成不可变版本）」才再 POST，真实成功后清错误。保存与发布的 412 均由第二真实 API 会话提升 revision 产生；本地名称/说明不丢，取消唯一确认保留输入，接受后遇真实 404 的重读失败仍保留工作区/编辑/恢复入口，成功 GET 后才清编辑。失败不降低待办数或自动发布 | **PASS**；[真实字段独立与已知 422](../../../var/pc02b-qa-round6/evidence/api-independent-review.json)、[412 与未知问题恢复](../../../var/pc02b-qa-round6/evidence/conflicts-and-issues.json) |
| AC-PC2-008 / UI005～008 | 实际 WebGL 模型中，待办定位不进入拾取；普通旋转/点击及绑定中的拖动不创建热点。明确开始后显示当前部件，取消后点击仍零写；再次开始并真实点击才有 confirmed 热点，当前模型 revision/SHA、有限局部坐标正确，文字 review 不变。旧模型锚点 API 强行确认真实 422 且不写；显式重绑后同 hotspot ID 使用当前模型。实际保存/清除步骤视角均不改变热点。非几何任务、下一项、模型声明、原文/返回及 375 发布有真实键盘链；旧原文/Drawer 回归本轮完整执行通过。所有复核阶段相对造数后基线的 job/attempt/ledger/供应商计数增量为 0 | **PASS**；[实际鼠标/视角/旧锚点重绑](../../../var/pc02b-qa-round6/evidence/geometry-regression.json)、API 独立场景及下述 PC-02A 复验 |

### PC-02A 受影响回归

在 **PC-02B 快照与 schema8 副本** 上完整重跑原 6 项，全部通过：官方 LACK 实际入口/8 页与合法非法页码；22 文档分页及两份真实不同 PDF；官方 TR-808 扫描原件的图像页与无伪文字；阅读器/复核页 **375/1024/1440 × 部件/步骤** 出处、焦点返回和 resize；真实局部 404 重试、WebGL 不可用仍 PDF、键盘 Drawer 往返；迟到第一份 PDF 不覆盖第二份。准备/job/attempt/ledger/draft/release 均无新增，供应商与外网为零。

本轮证据独立保存在 `var/pc02b-qa-round6/pc02a-regression/evidence/`：[出处矩阵](../../../var/pc02b-qa-round6/pc02a-regression/evidence/reference-matrix.json)、[404/WebGL](../../../var/pc02b-qa-round6/pc02a-regression/evidence/local-failure.json)、[迟到响应](../../../var/pc02b-qa-round6/pc02a-regression/evidence/late-response.json)。跨文档引用仍明确是 synthetic draft/release 元数据，原件资产是实际后端字节；不冒称多 PDF 生成。该证据足以关闭本轮 PageLayout/document-navigation 导致的 PC-02A `needs_retest`，恢复其既有 accepted，QA 未自行修改 state。

### 数据身份、审查闭环与限制

- 本轮真实显式发布物品 `01a0fa95-249b-7230-b12d-0625b1612534`、草稿 `01a0fa95-26fc-739e-b942-27ac2c45f957`、release `01a0fa95-2773-74f2-b1ba-81df60228996`；manifest SHA-256 `4048e45415bf3d48aba35d7701e1f00364819d95336d3909e3fabaa63e32a8e1`。首发与幂等重放均 201，重放有 `x-idempotent-replay: true`，release 数始终 1；后续草稿修改前后 manifest 字节一致。
- 鼠标实际拾取 hotspot `01a0fa95-eaec-7493-ad3f-ec24a9ab7a93`，落库 positionLocal `[0, 0.000009094461029525536, 0.999999999999999]`；之后旧模型 SQL 历史夹具及真实重绑仍使用同 ID。历史 SQL 仅改变自有临时草稿的旧 anchor/status，不是生产迁移或真实第二次模型购买。
- 前置 `seedReadyPreparation` 是经真实 HTTP 保存的 32×32 JPEG/自制文字**格式夹具**，不是 PDF.js 提取；它只服务于本卡生成草稿前置，准备提取质量由 PC-03A 单独验收。本轮出处导航另读取真实原件 PDF。任务生成通过 localhost fake provider，造数阶段确有假 upload/submit/manual/CDN 请求；“零新增”均与草稿生成成功后的实际基线比较，不把造数称为零调用或真实付款。
- 422 展示测试有且仅有两次负向响应注入：一次捕获的真实不完整草稿错误，一次人为未来/恶意详情；未知规则不是自然生成的服务端故障。正常草稿、真实 412、重读 404、发布成功与其数据库/资产事实没有伪造。
- 交付前只读 finding 已闭环：有旧数据时 refetch 失败原可能进入整页 error；冻结版通过真实 404 重读验证编辑及恢复控件保留。未知 422 的最终合同也实际验证为 GET 不清诊断、明确再次发布才请求最终校验；均不是把未交付版本记录为正式 FAIL。
- 初次运行修正了自有测试的协议/选择器与等待：幂等 replay 保留 201 且头名为 `x-idempotent-replay`；textarea label 含初始内容，以「说明」前缀定位；当前绑定显示实体名；实际按钮为「清除视角」；resize 后等待按钮可见再量尺寸。确认了这些差异后最终全套 6 项重新通过，没有放宽功能断言、强制点击、跳过分支或修改生产。
- 范围限 Chromium、本机虚构供应商、有限原文/几何回归；没有全浏览器矩阵、真实供应商质量/付费、官方说明书语义/OCR、全部坐标模型的精度或普通发行候选结论。所有 ID 属已清理的私有 QA 库，不是用户 preview 数据。普通候选构建/部署仍由协调者独立推进。
- 交协调者：可将 **PC-02B / round6 / scope=slice / PRD UI revision2** 记录 accepted，并按上述实际回归恢复 **PC-02A accepted**。未修改生产、PRD、state 或用户数据；其余卡不在本轮签署范围。

## Round 7 — PC-03B 流程摘要与建单恢复（2026-10-02）

角色 QA；`scope=slice`；PRD/UI revision 2 §§12.2 / 12.3 B / 12.5，REQ-PC3-004～006、AC-PC3-006～009。结论：**FAIL，仅 PC-03B；BUG-PC3-002（P2）阻断 AC-PC3-006**。其余三项 AC 的本轮独立场景通过。保留 round1～6；不改变此前 PC02A/B、PC03A 结论，不签普通发行或全项目。

### 冻结对象与实际执行

- 前端 `var/prd-completion/web-snapshots/pc03b-rd-ready`，122 文件在测试前后逐一匹配；manifest SHA-256 `061ce14a9f041aa53ce8070f6b0cccaa5024ece555e8d1b707ff1ac72614cb35`。Playwright/Vitest 均固定该快照，web 端口15487，无 mutable 前端验收。
- `var/pc03b-qa-round7/everything-manual-fixture`：schema8、job-failpoints、无 embedded-ui；SHA-256 `bec41094b445d57092845d7aa181ac31de900d9be95fa73d05ae9d7b9a302441`。同目录3个复制 Rust 测试 binary 指纹均与 binaries.json 前后一致；没有 Cargo、target/debug 执行依赖或预览访问。
- 全部后端使用自己创建的临时目录、假配置与 localhost fake provider/CDN；不读取用户 settings/config/key 或8080/5173。浏览器阻断非localhost HTTP，无 trace/HAR/video。HTTP 正常成功响应、报价/任务/发布事实来自真实服务端；负向响应投递和历史/时钟夹具在下文明确说明。测试结束关闭自有服务、浏览器并清理自有临时数据。
- 自有计划、日志、JSON证据在 `var/pc03b-qa-round7/`；[最终指纹与结果](../../../var/pc03b-qa-round7/frozen-verification.json)。Node为前轮记录的 Codex bundled Node 绝对路径，以下简写 `node`；Playwright运行时强制设置 `EM_PC03B_QA_BINARY_SHA256` 为上述指纹。

| 实际命令 / cwd | 结果 |
| --- | --- |
| `var/pc03b-qa-round7/generation_requests --test-threads=2`，仓库根 | **34/34 PASS，13.93s**，复制产物；[日志](../../../var/pc03b-qa-round7/rust-generation_requests.log) |
| `var/pc03b-qa-round7/items --test-threads=2`，仓库根 | **11/11 PASS，3.72s**，含真实本机计数监听器；[日志](../../../var/pc03b-qa-round7/rust-items.log) |
| `var/pc03b-qa-round7/preparations --test-threads=2`，仓库根 | **11/11 PASS，3.95s**；三套合计56个不同Rust用例；[日志](../../../var/pc03b-qa-round7/rust-preparations.log) |
| `node node_modules/vitest/vitest.mjs run --config vitest.pc03b-qa.config.ts src/features/import/ConfirmStepPage.test.tsx src/features/library/workflow.test.ts src/features/import/quote-recovery.test.tsx src/features/import/messages.test.ts`，apps/web；实际root=冻结快照 | **4文件 / 22项 PASS，3.02s**，含StrictMode、显式quote URL、坏缓存正常化、恢复锁、412授权保留；[日志](../../../var/pc03b-qa-round7/unit.log) |
| `node node_modules/@playwright/test/cli.js test --config playwright.pc03b-qa.config.ts`，apps/web | 首次完整 **5 passed / 4 failed，40.6s**；一项生产失败、三项自有测量/错误来源问题，未将首次失败计为通过；[日志](../../../var/pc03b-qa-round7/playwright-run1.log) |
| `node apps/web/node_modules/@playwright/test/cli.js test --config apps/web/playwright.pc03b-qa.config.ts --grep 'priority and deterministic\|one library batch\|unreceived submit\|actual stale-input'`，仓库根 | 自有测试修正后定向 **3 passed / 1 failed，12.2s**；原发布摘要门槛第二次失败。合并不同场景为 **8/9 PASS、1/9 FAIL**，不重复计算重跑；[日志](../../../var/pc03b-qa-round7/playwright-targeted2.log) |
| `node apps/web/node_modules/typescript/bin/tsc --noEmit -p var/pc03b-qa-round7/qa-pc03b-tsconfig.json`，仓库根；ESLint自有2配置及4个qa-pc03b文件 `--max-warnings 0`，apps/web | 最终均exit0，无warning；只签自有QA代码静态检查 |

### 逐 AC 结论

| AC / UI | 独立证据与结果 |
| --- | --- |
| AC-PC3-006 / UI005 | **FAIL — BUG-PC3-002**。真实发布后主动作仍继续复核，详下节。其余批量边界独立通过：101个真实物品、100上限、重复ID去重、非法/重复/未知query422，混合不存在/跨物品document和quote404，匿名401，全表logical digest证明GET零写。375资料库9行在稳定测量区间仅1个summary batch，无逐行job/draft/release详情请求，旧发布版次入口保留，真实404投递后仅概览fallback。原API排序组合在发布主门槛处失败，后续SQL历史job/draft/release tie分支**未到达，不冒称独立通过**；修复后须完整跑到。参考[batch](../../../var/pc03b-qa-round7/evidence/api-batch.json)、[资料库](../../../var/pc03b-qa-round7/evidence/browser-library.json)。 |
| AC-PC3-007 / UI006 | **PASS**。API及375真实向导区别已保存基本信息、缺原件/视图/准备、旧ready与新document、确认/过期。显式旧document仍用其ready；新document不借旧ready，后续单独准备归属正确。真实侧视图PATCH成detail使视图和确认需重查，新增有效侧视图后原quote仍inputChanged；明确新报价重新要求勾选确认。到期夹具使quote和步骤同步重查，不能生成。无新增job/cost。消费后修改当前型号/照片，原任务snapshot/release不变。参考[API资料事实](../../../var/pc03b-qa-round7/evidence/api-current-facts.json)、[向导](../../../var/pc03b-qa-round7/evidence/browser-wizard.json)。 |
| AC-PC3-008 / UI007、008 | **PASS**。真实建单POST经后端202后截断响应，quote真实404反复核对时无新报价/提交；清空存储的新context由服务端找到同一consumed job，未依赖旧sessionStorage/localStorage，最终1job/1次fake paid submit。另一场未送达请求+读取失败，GET未消费只开放显式同一提交；原预算锁定，第二次key/body完全一致，延迟原业务身份再经真实API重放仍202+幂等标记，不增加job。没有把GET未消费当作旧POST永不会到达的证明。参考[已受理丢响应](../../../var/pc03b-qa-round7/evidence/browser-accepted-loss.json)、[同一操作恢复](../../../var/pc03b-qa-round7/evidence/browser-same-operation.json)。 |
| AC-PC3-009 / UI007、008 | **PASS，错误注入范围如下**。真实cookie清除/401后原next回到保存quote，不自动确认；独立假配置真实pending阻止未消费quote的新生成，但已消费quote优先恢复job。真实输入变化建单422不建任务；确认页收到真实CAS来源412投递后GET核对且保留42 credits/0.9 USD授权，新报价仍重新同意。localhost provider实际HTTP500使job持久化submission_unknown，fresh context恢复同一任务，详情明确待对账；ledger预留行逐字相同，无重新购买。375无横向溢出，同一提交按钮≥44px并由Enter触发。参考[401/pending](../../../var/pc03b-qa-round7/evidence/browser-auth-pending.json)、[422/412/unknown](../../../var/pc03b-qa-round7/evidence/browser-412-unknown.json)。 |

### BUG-PC3-002 — 正常发布后处理摘要仍推荐继续复核

- **优先级P2，状态OPEN，阻断AC-PC3-006。** 冻结 `crates/server/src/storage/repo/item_summaries.rs:35` 用 `r.draft_revision=d.revision` 判断草稿当前版本是否已发布。但 `releases/service.rs:259` 正常发布先CAS递增草稿revision，`:307` 保存被发布内容的 `expected_revision`；发布自身就造成相差1，并非用户修改。
- 真实HTTP路径：创建资料→本机fake任务成功→草稿事实/模型/热点明确确认→POST publish返回201→立即GET summary。没有SQL历史夹具参与失败点。发布前revision4、发布后revision5、release.draft_revision4；服务端错误返回 `action=reviewDraft`、target草稿。预期没有后续编辑时为 `readRelease`、target新release；真实后续编辑则应恢复reviewDraft。
- 复现身份：item `01a0fac2-875f-704a-8bce-7b3ca6a4d404`，draft `01a0fac2-8a25-70d5-a320-6dbf72eb49bc`，release `01a0fac2-8b53-76a4-aa5b-8198d57e61aa`。两次原断言稳定失败；[精简DB/API证据](../../../var/pc03b-qa-round7/evidence/bug-PC3-002-published-summary.json)，独立测试 `qa-pc03b-api.spec.ts` 的发布后readRelease断言保持原值。
- 用户影响：已完成发布仍被引回复核，最新发布版不能成为正确主动作。旧发布版次入口仍可访问，不是资料丢失。修复应依实际发布事实区分发布自身bump与后续编辑，不能无条件以±1猜；复验需覆盖发布、幂等重放、后续编辑、再发布及原组合尚未到达的稳定tie分支。

### 测试修正与范围限制

- 首次额外三失败属于自有测试：登录后资料库首个异步batch落入下一次导航计数，已等待初始状态稳定再清测量数组，仍严格要求被测导航1个batch；37回读规范化为37.00，改比较同一数额并严格比较最终原key/body/整数预算，未降低授权门槛；真实输入变化合同为422，不能误写412。
- 412专门负向测试先让实际建单因inputChanged返回422，再把独立item过期If-Match产生的**真实412 envelope投递至提交响应边界**，验证前端该分支；不是自然job-create 412，也不是正常成功响应mock。401/pending/unknown走真实原生路径；quote/summary失败则把真实隔离404投递到相应读取边界。
- 过期quote由私有时钟夹具调整：同一SQLite事务内临时去除并原样恢复quote immutable trigger后才进行HTTP，未修改生产迁移/运行规则。历史排序SQL分支本轮未到达；原件与准备是已明确的格式前置，不冒称本轮PDF.js提取、OCR或真实供应商质量验收。
- 本轮调用仅本机fake provider；造数有真实fake upload/submit/manual/CDN请求，“恢复零新增”以基线比较，不把整场写成零provider调用。Chromium范围，未扩大矩阵；无用户数据修改、生产修复、PRD/state修改或预览部署。普通候选cold smoke由root单列，不属于本轮结果。
- 交协调者：PC03B保持 **QA FAIL / needs-fix**，请修BUG-PC3-002并重新冻结再派复验；不得因56 Rust/22组件及其余8场通过将本切片签accepted。

## Round 8 — PC-03B B2 / BUG-PC3-002 复验（2026-10-02）

角色 QA；`scope=slice`；PRD/UI revision2 §§12.2 / 12.3 B / 12.5，AC-PC3-006～009。结论：**PASS，仅 PC03B；BUG-PC3-002 CLOSED**。Round7的FAIL、原始失败日志与缺陷记录原样保留；本节是新冻结交付的复验，不回改历史结论。没有新生产缺陷。

### 对象与真实命令

- 浏览器/API和新后端回归固定 **B2**：`var/prd-completion/web-snapshots/pc03b-b2-ready`，122文件，manifest `061ce14a9f041aa53ce8070f6b0cccaa5024ece555e8d1b707ff1ac72614cb35`；主binary `var/pc03b-qa-round8/everything-manual-fixture`，SHA-256 `57a0650f0101a34d4bedc20b03e0028129444268f32af6569dc57ea57493462c`，schema8/job-failpoints/无embedded-ui。生产变更仅 `storage/repo/item_summaries.rs`，识别同事务发布审计中明确的发布后revision；测试变更为publishing回归。前端生产与B1一致。
- 本轮unit最终采用 **仅测试等待修正快照** `var/prd-completion/web-snapshots/pc03b-b2-tests-ready`，manifest `fd9ac7ddb40fc19d6c309935005b629331b20d03ba82dd2ae33551a289ff2ab7`。与B2唯一差异是 `src/features/import/ConfirmStepPage.test.tsx`，SHA-256 `a17f80b634bbd20b186ca4ff3fd2ba169ffb0dbdc8b415b100f6b57fbc4c2f8a`；其余121文件逐一相同，生产字节全不变。独立核对：[测试快照](../../../var/pc03b-qa-round8/test-wait-qa-verification.json)。
- 配置仍 `playwright.pc03b-qa.config.ts` / `vitest.pc03b-qa.config.ts`；通过 `EM_PC03B_QA_WEB_ROOT`、`EM_PC03B_QA_BINARY`、`EM_PC03B_QA_OUTPUT` 指向对应冻结树及round8目录，`EM_PC03B_QA_BINARY_SHA256`强制匹配上述新binary。只允许仓库var内复制binary、web-snapshots内前端，无target或实时源码fallback；web15487。各产物/前端前后hash一致，[最终汇总](../../../var/pc03b-qa-round8/frozen-verification.json)。
- 仍使用自己创建的临时数据、假配置与localhost provider/CDN，无真实付费、用户preview/settings/key读取、trace/HAR/video、Cargo或生产/PRD/state修改。下面Node为既有Codex bundled Node，环境路径按上段设置；自有服务与进程已结束。

| 实际命令 / cwd | 结果 |
| --- | --- |
| `var/pc03b-qa-round8/generation_requests --test-threads=2`，仓库根 | **34/34 PASS，13.81s**，B2新复制产物；[日志](../../../var/pc03b-qa-round8/rust-generation_requests.log) |
| `var/pc03b-qa-round8/publishing --test-threads=2`，仓库根 | **7/7 PASS，3.70s**；本轮新Rust合计 **41**，没有借用round7旧items/preparations产物冒称新编译回归；[日志](../../../var/pc03b-qa-round8/rust-publishing.log) |
| `node node_modules/@playwright/test/cli.js test --config playwright.pc03b-qa.config.ts`，apps/web，webServer=B2 | **完整9/9 PASS，27.1s，无重试**；原3API+6browser，扩充原排序组合而未删除原发布主门槛；[日志](../../../var/pc03b-qa-round8/playwright-run1.log) |
| `node node_modules/vitest/vitest.mjs run --config vitest.pc03b-qa.config.ts src/features/import/ConfirmStepPage.test.tsx src/features/library/workflow.test.ts src/features/import/quote-recovery.test.tsx src/features/import/messages.test.ts`，apps/web，root=最终仅测试差异快照 | **4文件 / 22项 PASS，3.26s**；[最终正式日志](../../../var/pc03b-qa-round8/unit-final-test-snapshot.log)。首次原B2失败及诊断过程详下节，不混计为通过 |
| 自有scoped `tsc --noEmit -p var/pc03b-qa-round7/qa-pc03b-tsconfig.json` 与2配置/4个qa-pc03b文件 ESLint `--max-warnings 0` | 最终均exit0；复用同一scope定义不代表读取round7运行目标 |

### 逐 AC 与 BUG 关闭证据

| AC | 本轮独立结果 |
| --- | --- |
| AC-PC3-006 | **PASS**。原真实发布后readRelease硬断言通过；实际发布revision4→5，release记录4，由同事务明确事实识别。原key/旧ETag重放仍同release且全库logical digest不变；实际PATCH至6回reviewDraft，重新确认后revision8→再次发布9回新readRelease，第一release行不变；再编辑仍回reviewDraft。7种隔离坏审计（缺metadata、错item/draft、afterRevision字符串、错release身份、错事务时间、缺失整行）都保守回reviewDraft，GET不修复写库，恢复原审计后readRelease。此前未到达的attention→running→review→release优先级和job/draft/release稳定tie分支本轮全部执行通过。批量边界、GET零写、375资料库单batch/无逐行详情请求、旧release次入口和404fallback也完整重跑。证据：[真实发布生命周期与审计](../../../var/pc03b-qa-round8/evidence/published-lifecycle-and-audit.json)、[排序](../../../var/pc03b-qa-round8/evidence/api-priority.json)、[batch](../../../var/pc03b-qa-round8/evidence/api-batch.json)、[资料库](../../../var/pc03b-qa-round8/evidence/browser-library.json)。 |
| AC-PC3-007 | **PASS**。新旧document的ready归属、真实侧视图缺失/替换、quote输入变化和明确重新确认、私有过期时钟夹具及五步状态完整通过；已消费job在当前型号/视图变化后仍可读，原snapshot/release保持。无新增job/cost的界限仍按实际基线验证。证据：[API事实](../../../var/pc03b-qa-round8/evidence/api-current-facts.json)、[向导](../../../var/pc03b-qa-round8/evidence/browser-wizard.json)。 |
| AC-PC3-008 | **PASS**。实际202之后响应丢失、quote真实404保持只读锁、空localStorage新context恢复同一任务，最终1job/1次fake paid submit。GET尚未消费时只显式同key/body/quote重试，原授权37 credits/0.8 USD锁定；延迟原业务身份再次实际投递为幂等202，仍1job，无重新报价。证据：[已受理丢响应](../../../var/pc03b-qa-round8/evidence/browser-accepted-loss.json)、[同一身份恢复](../../../var/pc03b-qa-round8/evidence/browser-same-operation.json)。 |
| AC-PC3-009 | **PASS**。真实401原next恢复、真实假配置pending阻止新生成而已消费job优先、真实inputChanged422、真实CAS来源412负向投递保留42 credits/0.9 USD、重新报价再同意、fake provider实际HTTP500持久化unknown及freshcontext对账入口/预留行不变均重跑通过。412来源边界与round7一致，不声称job-create天然返回412。375布局与键盘操作保留。证据：[401/pending](../../../var/pc03b-qa-round8/evidence/browser-auth-pending.json)、[422/412/unknown](../../../var/pc03b-qa-round8/evidence/browser-412-unknown.json)。 |

**BUG-PC3-002关闭**：真实物品 `01a0face-1971-75e1-91c7-e240e5934e4c`，草稿 `01a0face-1bcc-7605-a757-1a8f5d06fa6c`；首次release `01a0face-1c40-7609-a457-7c30a1f4aec8`，再次release `01a0face-1d57-70f9-ae13-6d4a6a1fc735`。上述publish/replay/edit/republish都是正常HTTP，审计故障注入发生在已通过真实发布门槛之后；没有以历史metadata mock取代原失败点。修复既识别发布本身bump，又保留后续实质编辑的复核入口。

### 首次组件失败、修正与限制

- 原B2冻结组件首跑 **21/22**，旧 `ConfirmStepPage.test.tsx` 的丢响应场景在 `confirm()` 等generate enabled前失败；原样限定单项复核仍失败。日志 [首次](../../../var/pc03b-qa-round8/unit.log)、[限定复核](../../../var/pc03b-qa-round8/unit-targeted2.log)保留；没有反复原样重跑寻绿，也没有把真实9场通过当作自动豁免。
- 经root授权，QA只在自有副本有限诊断。添加观察的副本通过不能单独证明产品无缺陷；最终去掉诊断打印，只使helper `await waitFor(() => expect(checkbox).toBeEnabled())` 后再click，仍保留原generate enabled、提交计数、读取失败锁定及恢复所有断言，该副本组合22/22通过。诊断不是正式unit结果，临时代码已移入忽略目录 `diagnostics/`，不污染正式测试发现。
- 唯一RD按上述可操作前提只修源测试helper，root派生仅一测试文件差异的新冻结树；QA对新树正式22/22通过，并核验其余121文件和所有生产字节未变。没有改确认业务门槛、跳过单测、放宽超时或增加重试；原失败作为测试等待不充分的历史限制保留，不登记未经证实的第二个产品缺陷。
- 当前结论仍限PC03B、Chromium和localhost fake provider。准备前置为自制格式资产、排序历史为标注SQL metadata；坏审计和quote过期仅在自有临时库，全部GET按全表digest核对无写入。不签真实供应商付款/质量、全部浏览器矩阵、普通发行候选或全项目。
- 交协调者：可将 **PC03B round8 accepted，BUG-PC3-002 closed**。普通B2候选部署由root单独核验/执行，QA未修改state或用户数据。

## Round 9 — PC-04 受控本机验证命令（2026-10-02）

角色 QA；`scope=slice`；PRD/UI revision2 §13，REQ-PC4-001～006、AC-PC4-001～007、UI-PC4-001～005。结论：**PASS，仅 PC04 的受控命令与本机fixture链路**。没有新生产缺陷；前期只读发现的secret-like具名ID回显入口已由RD修正，本轮实际拒绝复验通过。**真实供应商、AC-042、T23仍为NOT_RUN**，不由此结果推导全项目或正式发行包通过。Round1～8原样保留。

### 验收对象、隔离与命令

- root正式GO后的对象固定为 `var/prd-completion/pc04-rd-ready/`。QA对11个复制物逐一SHA校验后执行，结束再次逐一校验一致；没有Cargo、共享target fallback或依赖root并行构建出的新程序。主工具 `xtask` SHA-256 **`df05f92aac23619e1cd53fa998b3b586a4a6f3d266b8322bfa0078749f59a8db`**；fixture server **`086f4e0a5f9e6f05be1cd7a7b6cce1fed3a0ee9810e0ff341c9fa7812461f10a`**；含CLI回归的publishing testbin **`3f781a7597ac8daab7aaacbc87aa2456fa43bf873ad875e8be384a27846c9f99`**。均schema8/debug+job-failpoints；服务无embedded-ui。本卡不需要浏览器/Vite。
- 211项Rust源manifest、25项卡片文件manifest与root冻结记录保留于同目录；QA核对其清单文件hash并将动态结论绑定复制binary，未把随后可变的PC05A工作树冒充运行对象。前端生产沿用已接受PC03B，唯一相关Playwright发现配置SHA与本卡冻结记录一致。
- 独立测试 `qa-pc04-cli.spec.ts` / `qa-pc04-harness.ts`，配置 `playwright.pc04-qa.config.ts`：每场以真实HTTP上传/绑定原件、照片和声明的准备格式资产，获取同源quote后停掉自有服务，再执行真实CLI子进程。准备页为一页自撰文本/项目图片格式fixture，不声称PDF.js解析；没有读取用户preview/settings/真实配置或官方手册。全部供应商、代理、下载地址均127.0.0.1；假Key仅注入子进程环境。trace/HAR/video关闭，没有真实费用。
- `EM_PC04_QA_SERVER` / `_SERVER_SHA256`、`EM_PC04_QA_XTASK` / `_XTASK_SHA256`明确指定上述路径与指纹，缺失即拒绝测试执行；自有进程/临时库在每场结束清理。日志、哈希和安全事实留在忽略目录 `var/pc04-qa-round9/`。前后核验：[初始](../../../var/pc04-qa-round9/freeze-before.json)、[最终](../../../var/pc04-qa-round9/freeze-final.json)。

| 实际命令 / cwd | 独立实际结果 |
| --- | --- |
| bundled `node node_modules/@playwright/test/cli.js test --config playwright.pc04-qa.config.ts`，apps/web，上述4个freeze环境变量 | 首次 **7/8通过，31.4s**；成功组因QA将已知假Key用作普通远端ID而触发原安全丢弃逻辑，详下节。原失败 [日志](../../../var/pc04-qa-round9/playwright-run1.log)保留。 |
| 同命令加 `--grep 'actual adapters\|failure, over-budget'`，仅修正自有fixture后 | **2/2通过，14.8s**：成功组完整执行原产物/预算/hash/重放门槛，失败组保留原4变体并新增真实凭据回显负向。此前其余6组无需重复，八组最终全部完成通过，不称原始全套8/8一次通过。[复核](../../../var/pc04-qa-round9/playwright-corrected-fixture.log) |
| bundled `python3 var/pc04-qa-round9/run-copied-regressions.py`，仓库根 | **119个不同Rust用例通过**。只spawn已核验的复制testbins、最小环境，CLI子进程显式 `EM_PC04_XTASK_BINARY=<冻结xtask>`。2合同 + 6显式ignored CLI + jobs_recovery25 + manual_ai_contract20 + model_assets24 + pipeline14 + tripo_contract18 + publishing7 + preparation2 + secret分类1；各命令参数/耗时/exit0见 [结果](../../../var/pc04-qa-round9/rust-results.json)。未使用另含编译期server路径的非选中测试。 |
| `node apps/web/node_modules/typescript/bin/tsc --noEmit -p var/pc04-qa-round9/qa-pc04-tsconfig.json`；apps/web下 ESLint自有2测试文件+QA配置 `--max-warnings=0` | 最终均exit0；初次authoring的可空Buffer索引类型诊断已在运行前修正，没有生产/断言门槛变化。 |
| `node node_modules/@playwright/test/cli.js test --list --config=playwright.config.ts`，apps/web | **138 tests / 23 files**，仅发现列表，不启动后端/browser。专用qa-pc套件0项，qa-t09/t16/t18/t19等既有基线仍在。验证默认入口修复未用旧快照混进当前基线；不把list当实际完整e2e通过。[清单](../../../var/pc04-qa-round9/default-discovery-list.log) |

### 七项 AC / UI 证据

| AC | 独立结论与实际观察 |
| --- | --- |
| AC-PC4-001 / UI001、003 | **PASS**。真实help区分plan/执行、两个币种及独立授权；缺budget、未知CLI参数、坏JSON/嵌套未知字段、0644、allowed=false、过期、secret-like caseId/instanceId/authorizationId逐项非零，固定门禁码可区分且不回显canary。所有前置拒绝provider0、全库logical digest不变。11个拒绝分支 [证据](../../../var/pc04-qa-round9/evidence/01-parsing.json)。 |
| AC-PC4-002 / UI001～003 | **PASS**。两次plan完全一致、无逻辑写/外呼；输入hash与真实HTTP quote的存储事实相等，providers/preset/parameters/price/page范围/视图及两个整数上界均同源。修改原件声明hash、照片view、provider model、planHash均拒绝；实际磁盘原件字节损坏触发materialAsset；目录版本不变但credits内容30→31使planHash变化，旧授权planChanged拒绝，恢复字节后原plan一致。[证据](../../../var/pc04-qa-round9/evidence/02-plan.json) |
| AC-PC4-003 / UI002、003 | **PASS**。creditMinor、usdMicros分别比实际所需少1即各自拒绝；每币种负数/小数/字符串/i64溢出拒绝，缺价格目录、缺假Key、PC06问题模型与不存在批次的retryScope均拒绝；全部零外呼、零逻辑写。金额来自当前测试目录同源计算，不以另一币种余额抵扣，不猜缺失价格。[14个负向事实](../../../var/pc04-qa-round9/evidence/03-budget.json)；复制纯合同单测补最大合法i64及嵌套字段。 |
| AC-PC4-004 / UI004、005 | **PASS**。真实适配器发front/left两个槽位、冻结模型及standard参数；AI为严格json_schema、store=false且本批仅第1页，与plan批数一致。实际GLB magic/version/length与导出SHA通过；结构化知识parts/steps/specs均有真实页号，草稿needs_review，产物0600，release0。job `01a0fb05-c264-757b-85f0-7c8527ccbe74`、draft `01a0fb05-c362-77b2-ae0b-41f44302d19b`；只有1job/2attempt/2ledger、Tripo paid POST1、ManualAI POST1。计划3000 creditMinor/8504 usdMicros；Tripo实际3000，ManualAI实际未知保持null而非0。报告remoteID仅hash、无原ID/签名/假Key/实例秘密路径，canary0。[证据](../../../var/pc04-qa-round9/evidence/04-success.json) |
| AC-PC4-005 / UI003～005 | **PASS**。成功同授权重放同job、业务行及paid计数不增；第一CLI持锁时第二CLI instanceBusy。实际remoteID落库后在poll处SIGKILL，重新同授权恢复成功，安全查询2次、paid submit仍1、job仍1。[并发/中断](../../../var/pc04-qa-round9/evidence/05-concurrent-interrupted.json)。已受理paid响应在代理处丢失形成真实submission_unknown；即使有精确可选retry授权，重跑仍同job/原3000预留/actual=null、paid1，没有自动重购。[unknown](../../../var/pc04-qa-round9/evidence/06-unknown.json)。复制6CLI补receipt前崩溃、明确429有/无scope和其它expired任务逐行不动；不是只用RD历史测试数代替本轮执行。 |
| AC-PC4-006 / UI003～005 | **PASS**。真实本机business400、截断GLB、实际31 credits超过30 credits上限、持有请求期间授权过期均非成功并保留已有资产。budgetRisk记录实际3100与授权3000；过期stop=authorizationExpired，释放已发响应后请求数量不再增加。已知Key回显丢弃、unknown/null/no trusted receipt，无秘密输出。[失败/部分成果](../../../var/pc04-qa-round9/evidence/07-failures.json)。实际第一upload已发时改变指定stage inputHash或目录质量参数，分别stageScopeChanged/planChanged停止，paid0；显式私有SQL另建queuedjob，其行/阶段逐字段不动且无attempt，授权job正常完成。[任务范围](../../../var/pc04-qa-round9/evidence/08-scope.json) |
| AC-PC4-007 / UI001、003、005 | **PASS**。全部动态对象是假Key与localhost fixture；已知Key回显不能作为可信成功receipt，普通opaque ID和签名URL不进入公开报告；正常case/budget、stdout/stderr/report/自有服务日志的canary检查0。报告与终端区分本机验证及真实验收未运行，AC-042/T23/realSupplierAcceptance均NOT_RUN。冻结CLI集成默认ignored；默认Playwright只排除依赖私有冻结物的qa-pc且保留138原基线发现。未执行完整default check/e2e或真实供应商。 |

### 首次失败、测试修正与限制

- 首次成功场景把**已配置假API Key本身**作为Tripo remoteTaskId，错误期待它被接纳后生成成功。原 `secret::response_requires_discard` 会检查raw/decoded JSON中的已知key；Tripo客户端对2xx/code0的泄漏响应按Unexpected处理，安全保持unknown。这是QA夹具与既有凭据保护合同冲突，未确认为产品缺陷。原7/8日志和 [诊断](../../../var/pc04-qa-round9/authoring-diagnostic.json)保留。
- 仅将成功场景的receipt改为不同的普通opaque ID，新增独立假签名marker，仍断言报告只含remote-ID hash且所有marker不可见；原key回显保留在失败组，实际断言unknown、预留/actual=null及remote_task_id未存。只重跑受影响2组，不改生产、不降成功阈值、不放宽重试/超时、不盲重跑找绿。测试夹具修正后的scoped typecheck/lint再通过。
- 八组是有限组合而非完整真实供应商覆盖：独立准备前置一页格式fixture；复制CLI用例另覆盖三页同源批次。大PDF/完整浏览器矩阵/模型真实质量/计费真实性与人工复核发布不在本轮。ManualAI reserved/actual=null按既有引擎未知实际金额如实报告，不能称实际免费。unknown ledger可为reserved或unknown，都必须保留预算并无再次购买。
- source/quality/stage损坏与另一个queuedjob仅在QA拥有的临时实例，SQL坏fixture明确标注，未替代真实HTTP准备/真实CLI生成成功门槛。合法业务流程没有自动复核/发布。真实付费授权没有被创建或开启。
- 结束后再次扫描21份持久日志/安全事实文件，已知假Key、密码、opaque-ID与签名marker **0命中**，[扫描结果](../../../var/pc04-qa-round9/persisted-evidence-scan.json)。附加 `pgrep` 进程枚举因环境 `sysmond service not found` 不可用，不冒称完成全系统进程检查；测试持有的子进程均由自己的句柄等待退出/清理，两个Playwright运行与复制回归session均已正常结束。
- 交协调者：**PC04 / AC-PC4-001～007可accepted**；普通embedded-ui候选、部署、全项目与真实T23由root另行核验。QA未修改state、生产、PRD或用户资料。

## Round 10 — PC-05A 服务端搜索、列表位置与发布入口（2026-10-02）

角色 QA；`scope=slice`；PRD/UI revision2 §§14.1/14.2/14.4，REQ-PC5-001/002、AC-PC5-001～003、UI-PC5-001～003。结论：**FAIL，BUG-PC5-001 / P2 / OPEN 阻断 PC05A**。清除搜索后 Back 的 URL 已恢复原 q，但搜索框持续为空；默认 Chromium148 与当前 Chrome154 均真实复现。既有 Round1～9原样保留，不由其它通过项推导本切片、全项目或当前浏览器矩阵通过。

### 冻结对象与实际执行

- 主副本 `var/prd-completion/pc05a-rd-ready/everything-manual-fixture` SHA **`f1eddd3b78c3c24b806741ea78fd848581a4e440466b71964493a23b2ba65f73`**，schema8/debug+job-failpoints，无embedded-ui；前端 `var/prd-completion/web-snapshots/pc05a-rd-ready` **125文件**，manifest SHA **`2e852b3a0f4c817f9397d0c4c8ab305ac30168d8d1bd24043978f5139645b1bf`**。QA前后校验4复制物及125前端文件，均一致；Rust211项冻结manifest SHA `d314435dc76b19d55f56861e4d9b638fbadd689ee28f2daeeede0d07f2c47e98`。动态只用这些副本，未读取并行PC05B热改前端作为目标，没有Cargo/target fallback。
- 自有 `qa-pc05a-{api,browser}.spec.ts`、backend/fixture、`playwright.pc05a-qa.config.ts`、`vitest.pc05a-qa.config.ts`；15489独立Vite、随机localhost后台、自有临时库。主binary和web路径/指纹是4个强制 `EM_PC05A_QA_*` 环境变量。格式准备fixture仅用于生成知识草稿，不宣称本卡完成新的PDF.js准备；成功数据/两次发布/ZIP均真实HTTP和本机适配器，没有真实供应商、用户preview/config/key读取。
- 原六组浏览器默认实际为 Playwright bundled **Chromium148.0.7778.96**，不能称当前Chrome全套通过。缺陷定向另用 installed **Chrome154.0.8037.93** headed、独立QA profile重现；该次沿用Desktop Chrome preset的模拟UA148，实际引擎版本由 `browser.version()`记录，未把UA当引擎。最终QA配置已明确 `channel: "chrome"`、不覆盖原生UA，不再默默回退148；完整当前Chrome覆盖留A2正式复验。
- 运行器 [run-frozen.py](../../../var/pc05a-qa-round10/run-frozen.py)仅spawn复制bin或Node，最小环境；日志和JSON在忽略目录 `var/pc05a-qa-round10/`。trace/HAR/video关闭。最终 [冻结核验](../../../var/pc05a-qa-round10/freeze-after-verify.json)通过；15489最终无监听。

| 实际命令 / cwd | QA实际结果 |
| --- | --- |
| bundled `python3 var/pc05a-qa-round10/run-frozen.py browser`，仓库根；内部在apps/web执行 `node node_modules/@playwright/test/cli.js test --config playwright.pc05a-qa.config.ts` | **原始5/6通过、1失败，37.0s**；两API + 浏览器第4/5/6组通过，第3组Back恢复搜索值失败。无skip/retry。[原日志](../../../var/pc05a-qa-round10/playwright-run1.log)，原error-context保留`browser-first-run/`。 |
| `... run-frozen.py history-diagnostic`；相同命令加 `--grep '3/6'` | **0/1，原样定向复现失败**。仅加入URL/history/input安全事实，原15s门槛不变。[日志](../../../var/pc05a-qa-round10/playwright-history-diagnostic.log)、[148事实](../../../var/pc05a-qa-round10/chromium148-evidence/03-history-diagnostic.json)。 |
| `... run-frozen.py history-chrome`；Chrome channel、同一第3组 | **0/1，当前Chrome154同门槛失败**。[日志](../../../var/pc05a-qa-round10/playwright-history-chrome.log)、[完整历史事实](../../../var/pc05a-qa-round10/history-chrome154-original.json)、[真实引擎身份](../../../var/pc05a-qa-round10/browser-chrome154-original.json)。未据老版本独有现象开缺陷。 |
| `... run-frozen.py rust`：copied `items-test --test-threads=1` + `publishing-test --test-threads=1`，仓库根 | **14/14 + 7/7 = 21个不同Rust用例通过**；publishing的6个PC04显式CLI场景默认ignored，不计本卡通过数。items SHA `ff4d2bde4e7852720953c1aae2b328d047bab29adc20f41b9120f01cb48d3a78`；publishing SHA `20d91a90b1a6479f0c07d5103685b550144ad13abef2e1871a5c38dbdb2a458e`。日志`rust-items.log`/`rust-publishing.log`及对应result.json。 |
| `... run-frozen.py components`：`node node_modules/vitest/vitest.mjs run --config vitest.pc05a-qa.config.ts`，选择LibraryPage、library-navigation、workflow、ItemFormPage四文件 | **10/10，4文件通过**，cwd目标为冻结PC05A；含StrictMode取消首帧后的滚动恢复组件回归。[日志](../../../var/pc05a-qa-round10/components.log)。 |
| apps/web：bundled `node node_modules/typescript/bin/tsc --noEmit -p ../../var/pc05a-qa-round10/qa-pc05a-tsconfig.json`；ESLint自有qa-pc05a测试及Playwright/Vitest配置 `--max-warnings=0` | 最终均exit0。最早authoring的`page.unroute`多参TS2554已在执行前仅修自有测试，未改生产/业务断言。 |

### 三项 AC / UI 证据

| AC | 本轮结论与证据 |
| --- | --- |
| AC-PC5-001 / UI001、002 | **独立搜索合同通过**。真实HTTP建立45 active、12 archived；最旧目标在默认20行外，通过名称中文/型号aZ19直接找到，`%`/`_`字面与decoy区分，非ASCII大小写保持原字符，trim/空串语义正确。两归档范围10种q逐页limit7无重漏；同时间tie仅对HTTP创建的3行私有修改created_at，真实分页顺序与owned DB `(created_at,id) DESC`全表事实一致。200/201 ASCII/中文/emoji按字符接受/拒绝；合法同义q续页、跨q/archive/sort/旧scope/非法cursor/重复参数/401均取证。所有GET前后全库logical digest相同，jobs/attempts/ledger/provider均0。[搜索](../../../var/pc05a-qa-round10/evidence/01-literal-search.json)、[校验/零写](../../../var/pc05a-qa-round10/evidence/02-cursor-and-read-only.json)。浏览器输入不即时发查询、Enter/按钮真实查询已过，但UI全套当前Chrome复核仍在A2范围。 |
| AC-PC5-002 / UI001、002及URL恢复 | **FAIL / BUG-PC5-001 OPEN**。成功load-more后URL才提交，保留20→40唯一行；提交q/切归档/清除重置cursor。pending显示旧结果说明、显式500失败保留原行/requestId、重试真实恢复；实际错scope422只给重置入口，无结果可清除，独立空归档库能切回active；201字符字段错误0请求，迟到的真实旧响应不覆盖当前字面查询。[边界证据](../../../var/pc05a-qa-round10/evidence/04-error-and-late-response.json)。但搜索aZ19→归档→清除→Back后URL/idx正确，input持续空15秒。原第3组后续第二次Back/Forward及fresh context URL cursor分支**未作为正式通过证据**；A2必须从原hard断言继续跑完，不能跳过失败称闭合。 |
| AC-PC5-003 / UI003 | **本轮已执行路径通过，当前Chrome完整覆盖待A2**。本机fake pipeline真实草稿、显式review/hotspot/model PATCH、两次HTTP publish得到R1 `01a0fb21-7bb5-7242-8574-977cfb0c348f` / R2 `01a0fb21-7bf8-71a3-9b2d-4865c9534d26`；summary同源r7/latestID。再真实编辑草稿使主任务reviewDraft、再真实提交第二个job并由localhost business400明确失败使handleJob，已发布入口始终R2，历史R1可读，旧release行不变。阅读→资料库/历史→旧阅读→资料库/概览返回筛选一致；实际原行top255.97px返回相同且可见，StrictMode滚动修复浏览器路径通过。[真实版本/任务/返回](../../../var/pc05a-qa-round10/evidence/05-real-latest-and-history.json)。375/1024/1440输入→搜索/清除→归档Tab次序、主要操作44px、无横向溢出/查询抢焦点；每20行只有1 list+1 batch summary，无逐行版本详情。两个版本通过375键盘真实下载，与HTTP ZIP的冻结manifest/每资产SHA一致（只忽略顶层exportedAtMillis），R1/R2冻结manifest不同；读/下载前后2jobs/4attempts/4ledger/2releases及供应商计数完全不变。[布局/ZIP](../../../var/pc05a-qa-round10/evidence/06-keyboard-zip.json)。这些浏览器通过来自148，不冒称154整组通过。 |

### BUG-PC5-001（P2 / OPEN）

- **归属与影响**：AC-PC5-002、UI-PC5-001；搜索框显示与已恢复查询不一致，用户无法从控件判断当前筛选，阻断当前slice PASS。
- **复现**：本轮第3组登录→中文旧目标→QA5A/加载更多→按钮搜索aZ19→显示已归档→清除搜索→浏览器Back。清除前`/?q=aZ19&archived=true`/idx5/input=aZ19；清除后`/?archived=true`/idx6/input空；Back后URL和idx恢复5，等待15秒input仍空。148首次、148定向、Chrome154定向同门槛一致；不是未发生历史导航或单次时序抖动。
- **期望**：Back恢复应用q/范围时，搜索框应显示aZ19；原Back/Forward和fresh-context恢复门槛都需保留。实际缺陷与请求失败保留旧结果是不同分支。
- **位置**：冻结 `src/features/library/LibraryPage.tsx:25–28` 的`search→input`同步、搜索input绑定、清除handler（约101行）。浏览器历史表单值回填可能相关，仅列为RD分析方向，QA未把关联代码当已证明根因。
- **完整信息**：[BUG-PC5-001.md](../../../var/pc05a-qa-round10/BUG-PC5-001.md)。RD修复后待新freeze/QA11，QA未修改生产或提前关闭。

### 过程限制与后续

- 最初自有配置沿用默认Desktop Chrome，实际跑到旧Chromium148。识别后保留五组通过的真实版本边界，并使用当前Chrome154复现缺陷；最终配置改为明确Chrome channel/原生UA。A2全部六组必须按新的当前Chrome配置逐项执行，不能复用148的UI通过数冒称当前支持矩阵完成。
- root要求停止补余分支前，QA刚启动一轮当前Chrome四浏览器组、临时捕获已知错误并最终rethrow以取未到达分支；收到调度后立即Ctrl-C，**exit130 / INTERRUPTED_NOT_COUNTED**。该轮日志/事实隔离在`interrupted-native-chrome/`，不计任何通过或覆盖；临时continuation已删除，原hard断言与原执行次序恢复。不得拿中断日志里的局部步骤替代原第3组通过。
- 仅明确的500错误时序边界被注入；成功列表/summary/401/422/版本/ZIP均真实服务端。timestamp tie明确为私有SQLfixture；新failed job是实际HTTP+假provider业务拒绝，没有伪造job/release状态。知识生成准备仍为声明的格式fixture，本卡未新验PDF.js或真实供应商质量/费用。
- 对55份持久日志/JSON及ZIP解包字节扫描本机假Key/假密码，**0命中**；只保存命中数量、不打印canary原文。[扫描](../../../var/pc05a-qa-round10/canary-scan.json)。
- 没有执行所有下载异常、所有浏览器、PC05B/C、真实AC-042/T23或完整release gate；只保留必要PC01双版本成功导出回归。QA未改生产/PRD/state/用户资料。**交协调者：PC05A不得accepted；保留BUG-PC5-001，修复后QA11按原六组完整复验。**

## Round 11 — PC-05A A2 / BUG-PC5-001 独立复验（2026-10-02）

角色 QA；`scope=slice`；PRD/UI revision2 §§14.1/14.2/14.4，AC-PC5-001～003、UI-PC5-001～003。结论：**PASS，仅 PC05A；BUG-PC5-001 由 QA 标记 CLOSED**。原六组在真实当前 Chrome 上一次全部通过，原第3组的硬断言及后续分支没有跳过。Round10的FAIL、148/154失败复现和中断记录均保留；本轮不签全项目、PC05B/C或多浏览器完整矩阵。

### A2身份、差异及命令

- 固定前端 `var/prd-completion/web-snapshots/pc05a-a2-ready`，125文件；manifest SHA **`b5cfb55ce19d5ae5d19eda7849b4f9b50aaae2b11a4929341eef9743a67b4646`**。QA独立比较A1/A2清单，只有 `LibraryPage.tsx`（`bc9f4bda…dc32af`）与 `LibraryPage.test.tsx`（`095bb516…75014`）变化，其余123文件一致，PC05B排除。[差异](../../../var/pc05a-qa-round11/source-delta.json)。
- 后端仍为copied `var/prd-completion/pc05a-rd-ready/everything-manual-fixture`，SHA **`f1eddd3b78c3c24b806741ea78fd848581a4e440466b71964493a23b2ba65f73`**；没有Cargo/target fallback。前后4个复制物及全部125前端源文件SHA一致，[最终核验](../../../var/pc05a-qa-round11/freeze-after-browser.json)。Rust211项冻结manifest未变；items-test/publishing-test的字节SHA与QA10一致，因此引用QA10实际通过的**14+7=21 Rust用例**，本轮未重复执行或增加计数。
- Playwright配置固定 `channel: "chrome"`、headed、新QA profile、无模拟UA；实际引擎 **Chrome154.0.8037.93**、原生Mac UA中的Chrome154。[身份](../../../var/pc05a-qa-round11/evidence/browser-identity.json)。15489只对冻结cwd启动Vite，随机端口自有fixture，环境假值/localhost provider。新增 `EM_PC05A_QA_OUTPUT_DIR=var/pc05a-qa-round11` 对应绝对路径，仅隔离输出，不改变业务断言；没有读取用户preview/settings/config或真实Key，没有真实供应商调用。
- 运行器 [run-frozen.py](../../../var/pc05a-qa-round11/run-frozen.py)固定A2路径和新manifest；4个 `EM_PC05A_QA_*` freeze变量强制传入，缺失即拒绝。所有结果保存独立round11目录，trace/HAR/video关闭。

| 实际命令 / cwd | 结果 |
| --- | --- |
| bundled `python3 var/pc05a-qa-round11/run-frozen.py browser`，仓库根；内部 `node node_modules/@playwright/test/cli.js test --config playwright.pc05a-qa.config.ts`，apps/web | **6/6一次通过，20.6s，0skip/0retry**：2API+4浏览器组合。第3组3.4s走完原Back输入断言、第二Back/Forward、fresh context URL分页。无soft/catch继续或门槛放宽。[日志](../../../var/pc05a-qa-round11/playwright-run1.log)。 |
| `... run-frozen.py components`；`node node_modules/vitest/vitest.mjs run --config vitest.pc05a-qa.config.ts`选择LibraryPage/library-navigation/workflow/ItemFormPage | **11/11，4文件**，全部来自A2冻结；含新增同act“清除/提交→立即Back→Forward”回归及原StrictMode滚动恢复。[日志](../../../var/pc05a-qa-round11/components.log)。 |
| apps/web下 bundled `node node_modules/typescript/bin/tsc --noEmit -p ../../var/pc05a-qa-round11/qa-pc05a-tsconfig.json`；ESLint `playwright.pc05a-qa.config.ts vitest.pc05a-qa.config.ts tests/e2e/qa-pc05a-*.ts --max-warnings=0` | 均exit0；只验证自有测试/配置及类型依赖，不把mutable B生产检查当A2验收。 |

### 三项AC与缺陷闭环

| AC | 本轮独立结论 |
| --- | --- |
| AC-PC5-001 / UI001、002 | **PASS**。六组原门槛完整重跑：45 active+12 archived真实创建，最旧目标在首20行外，名称中文/型号ASCII直接命中，字面%/_与decoy区分、非ASCII不折叠，trim/空q/两归档范围正确；limit7分页及明确时间tie无漏重且稳定。200/201三类字符、规范化q续游标、跨q/archive/sort/legacy拒绝、字段422/401实际通过；GET全库digest和0job/0attempt/0ledger/provider0不变。[搜索](../../../var/pc05a-qa-round11/evidence/01-literal-search.json)、[合同](../../../var/pc05a-qa-round11/evidence/02-cursor-and-read-only.json)。当前Chrome实际输入不查询、Enter/按钮请求真实目标；没有前端拉全库过滤。 |
| AC-PC5-002 / UI001、002/URL | **PASS，BUG-PC5-001 CLOSED**。原序列搜索aZ19→归档→清除→Back后，URL/idx回5且input=aZ19，原 `toHaveValue('aZ19')` 硬断言通过；没有延迟/重新填值/刷新替代。继续第二Back切回active、Forward到archive、全新context按真实cursor读取对应页，均完成。hold更多请求期间URL不先推进，成功后20→40且无重复；q/archive/clear重置cursor。原pending/500旧数据标注、安全诊断/真实重试、真实错scope422/回到开头、无匹配/空archive、201字符零查询、旧请求迟到不覆盖新q全过。[历史事实](../../../var/pc05a-qa-round11/evidence/03-history-diagnostic.json)、[完整后半段](../../../var/pc05a-qa-round11/evidence/03-search-url.json)、[失败边界](../../../var/pc05a-qa-round11/evidence/04-error-and-late-response.json)。 |
| AC-PC5-003 / UI003 | **PASS**。当前Chrome重新执行真实两次publish：R1 `01a0fb33-1674-77a0-922d-3352df25d252`、R2 `01a0fb33-16ba-7782-a000-4fa1a6460ef5`；summary latestID/r7同源。真实待复核编辑与新的localhost business400失败job分别提升主任务，但R2阅读入口保留；历史R1可读/旧行不变。发布阅读/历史/概览返回保持筛选并恢复目标行可见（实测top256.62px前后一致）。375/1024/1440键盘次序、44px主要控件、无水平溢出、搜索不抢焦点；20行1列表+1批量摘要，无N套版本详情。375键盘下载两个真实ZIP，与各自HTTP导出的冻结manifest/每资产SHA一致，忽略的仅顶层exportedAtMillis；浏览/下载前后2jobs/4attempts/4ledger/2releases及fake provider counters不变。[真实版本/返回](../../../var/pc05a-qa-round11/evidence/05-real-latest-and-history.json)、[布局/ZIP](../../../var/pc05a-qa-round11/evidence/06-keyboard-zip.json)。 |

RD已用可重复诊断将原因定位为同步清input与Router transition不同步，快速Back取消transition时原search依赖未变化；A2把未提交草稿绑定location.key，提交后显示直接取URL。本轮原可达失败序列及冻结同act回归均通过，足以关闭BUG；QA10提出的“可能浏览器自动恢复”仅是当时排查假设，不作为最终根因。未引入新history项、DOM填值或测试等待来规避原门槛。

本轮没有测试作者失败/重试；唯一QA代码变化为将结果目录参数化到round11，真实Chrome配置与hard断言沿用QA10最终版本。准备输入仍为声明格式fixture，仅用于生成本机草稿，不宣称新的PDF.js准备或真实provider质量/计费；timestamp tie和明确500响应时序边界仍按原计划标明来源。没有扩大下载错误矩阵、PC05B/C、AC-063全浏览器或真实AC-042/T23。

23份持久日志/JSON/ZIP字节的假Key/假密码canary **0命中**，自有临时目录0，15489无监听；[安全/清理事实](../../../var/pc05a-qa-round11/canary-cleanup-scan.json)。**交协调者：PC05A / AC-PC5-001～003可accepted，BUG-PC5-001 CLOSED；普通embedded候选/部署与全项目门禁由root继续。** QA未改生产、PRD、state或用户资料。

## Round 12 — PC-05B B3 / 受影响 PC02B 与 AS 独立验收（2026-10-02）

角色 QA；`scope=slice`，PRD/UI revision2 §14.5，AC-PC5-004/005、UI-PC5-004～006。**PASS：PC05B 两项 AC；PC02B 受影响六场及 AS02/08 回归通过，可恢复 PC02B accepted。** 没有发现需登记的生产缺陷；不签 full gate、其它 PC05 子卡或真实供应商验收。Round1～11 保留。6 个主场景为分轮通过，不能写成整套单轮通过。

### 冻结、执行与作者修正

目标仅 `var/prd-completion/web-snapshots/pc05b-b3-ready`，127 文件 manifest SHA **`cb4419fc5abe1f2818babe90d6cc06ab3eba0b1b290d7d01b12b5a0ad1018a60`**；copied fixture `var/prd-completion/pc05b-rd-ready/everything-manual-fixture` SHA **`f1eddd3b78c3c24b806741ea78fd848581a4e440466b71964493a23b2ba65f73`**。四个 `EM_PC05B_QA_*` freeze 环境变量强制校验，15490 对应冻结 cwd；无 Cargo/target fallback、用户预览或真实 Provider。实际 **headed Chrome154.0.8037.93，原生 UA、新 QA context**；[身份](../../../var/pc05b-qa-round12/evidence/browser-identity.json)。前后127源文件、4复制物与211 Rust源指纹一致。

仓库根执行 `python3 var/pc05b-qa-round12/run-frozen.py --suite main|affected`，内部为 Playwright `playwright.pc05b[-affected]-qa.config.ts`；过滤重跑显式追加 `--grep`。所有实际执行均0skip/0自动retry，分轮详情见 [run-summary.json](../../../var/pc05b-qa-round12/run-summary.json)。

| 执行轮次 | 实际结果与证据 |
| --- | --- |
| main 首轮六场 | B1/B2通过，B3～B6失败，475.5s；[原日志](../../../var/pc05b-qa-round12/initial-main/playwright.log)。B3错误地在核对预读前注入404；B4误以为夹具有两个部件；B5/B6已见真实原生取消但PW导航promise不结束。均为测试作者问题。 |
| 有限作者修正后 `--grep 'B[3-6] '` | B3/B5/B6通过；B4在知识Drawer下DOM角色计数2失败，95.5s；[日志](../../../var/pc05b-qa-round12/main-author-fix1/playwright.log)。失败GET改在确认框打开后注入，两个buffer改用真实部件+规格；原生取消只限等待3s，并新增同document证明，接受必须实际新document。 |
| B4只读诊断→最终复验 | 原失败保留在 [b4-dom-vs-ax](../../../var/pc05b-qa-round12/b4-dom-vs-ax/playwright.log)。Chrome AX树实际只有1个非ignored modal；原DOM locator把后台Drawer也计入。经root认可改测单一`:modal`+单一有效AX dialog，保留双向Tab、Esc同field/buffer及URL/history全部硬断言；最终 **1/1，9.4s**，[日志](../../../var/pc05b-qa-round12/playwright.log)。另有一次错误锚定grep导致0条，保留在author-filter-error，不计覆盖。 |
| affected 既有回归 | **8/8一次通过，46.1s**：PC02B 4 browser+2 API、AS02/08；[日志](../../../var/pc05b-qa-round12/playwright-affected.log)。只适配旧HTML按钮/确认交互，业务断言保留；AS02两段真实原生刷新代码逐字相同。[交互差异及SHA](../../../var/pc05b-qa-round12/affected-author-delta.json)。 |
| B3冻结组件 | **53/53，8文件，一次通过**：WorkProtection6、Drawer4、PageLayout4、shell19、ProviderSettingsForm9、ItemFormPage3、useDraftMutations3、LibraryPage5。apps/web下 `vitest run --config vitest.pc05b-qa.config.ts` 显式选择上述文件；[日志](../../../var/pc05b-qa-round12/components.log)。 |

自有最终 scoped `tsc -p var/pc05b-qa-round12/qa-pc05b-tsconfig.json`、相关QA文件/config ESLint `--max-warnings=0` 均exit0；[命令与结果](../../../var/pc05b-qa-round12/final-static-checks.json)。作者修正精确差异在 [runtime-author-corrections.patch](../../../var/pc05b-qa-round12/runtime-author-corrections.patch)，未改生产/PRD/state。Rust未变化，本轮未执行或增加Rust计数。

### AC 与影响复核

| 范围 | 独立事实与结论 |
| --- | --- |
| AC-PC5-004 / UI005 | **PASS**。真实Link/Back/Forward取消/接受，history无额外项；1440/375首尾Tab/ShiftTab循环、Esc恢复来源；375知识Drawer保留同一field/buffer。真实原生beforeunload覆盖物品刷新取消/接受、知识/上传/准备刷新取消及活动准备关闭接受。原件/发布阅读、无变化表单、已实际受理完成的本机job详情无误弹；dirty程序导航另由冻结Router组件覆盖。[B4](../../../var/pc05b-qa-round12/evidence/b4-navigation.json)、[原生/布局](../../../var/pc05b-qa-round12/evidence/b6-native-layout.json)。真实PDF.js两页：第1页已PUT、第2页hold；离开后原件/第1页DB和资产SHA保留，新context只PUT第2页、无自动seal、无新增job/attempt/ledger/provider调用。[准备事实](../../../var/pc05b-qa-round12/evidence/b5-partial-retention.json)。 |
| AC-PC5-005 / UI004、006 | **PASS**。真实延迟201/PATCH及必要GET期间pending、防重、成功才导航/收起；真实422焦点；送达前失败、实际提交后丢响应及PATCH成功/GET失败保留编辑。未知创建跨实际401登录仍不盲POST；item/knowledge各真实401同文档内存返回、各真实412保留原基线，核对与显式丢弃分离，取消/GET失败保留，成功GET后再次显式保存。事实快照不变、overlay独立，普通编辑和假新Key未写local/sessionStorage。[保存](../../../var/pc05b-qa-round12/evidence/b1-save.json)、[未知结果](../../../var/pc05b-qa-round12/evidence/b2-uncertainty.json)、[401/CAS](../../../var/pc05b-qa-round12/evidence/b3-auth-cas.json)。 |
| PC02B / AS影响回归 | **PASS**。PC02B三宽度导航/真实计数/显式下一项、真实保存与发布412、known422/恶意unknown422诊断、显式发布、真实几何绑定和不可变版本API全部原门槛通过；AS02原生取消/接受与密钥不缓存、AS08字段/写失败/防重/真实配置CAS/取消及显式重读通过。历史QA6保持原结论，本轮提供B上的新复核证据。 |

### 边界、扫描与清理

[最终核验](../../../var/pc05b-qa-round12/freeze-canary-cleanup.json)：15490无监听，三类自有临时实例目录均0；trace/HAR/video关闭。已有PC02B回归手工保留一张375px假夹具热点图，经查看无密钥/设置/用户资料；AS截图关闭。51份运行日志/JSON/失败上下文扫描中有5处固定假Key命中，**全部为Playwright在“Test source”复制的同一个测试源码常量**，并非接口/UI/存储/后台回显；原失败没有静默删除。运行数据部分无固定秘密命中，AS每例另在清理前检查随机假Key日志及URL/storage/DOM；浏览器编辑canary写入与残留均0。不会把上述扫描表述为所有原始文件“零命中”。

知识草稿setup使用明确格式准备夹具+真实localhost pipeline；只有B5/B6准备路径宣称真实PDF.js。未做真实供应商调用或质量/费用验收，未扩大多浏览器、Linux、性能或发行矩阵。**交协调者：PC05B可accepted，PC02B可由needs_retest恢复accepted；普通预览部署与后续PC05C由root继续。**

## Round 13 — PC05C / scope=slice / PASS（2026-10-02）

PRD/UI revision 2 §14.6，REQ-PC5-005/006、AC-PC5-006～008、UI-PC5-007～009。**本卡 PASS；PC05B 与 PC06 本次受影响合同复核 PASS，可恢复 accepted。** 未发现冻结 C 的产品缺陷，不签 full gate、全部浏览器矩阵、正常调度性能或真实供应商质量。没有更改生产/PRD/state/旧测试，没有 Cargo、用户预览或真实付费调用。

### 冻结对象与执行结果

前端 `var/prd-completion/web-snapshots/pc05c-rd-ready`：131 文件，manifest `6cb9b763b6f9cf4470e784ee19c2d50cb1cb097514f28a66a9de976738df6fac`；copied fixture `var/prd-completion/pc05c-rd-ready/everything-manual-fixture`：`976ef85eb0f67498285c20eb6061e15ea62ae8c583274641eb5a73c23498b469`。211 项 Rust manifest `78c1372390325c25d65d5c4afcea6ee8666614a4670f69838229bf88a0a96c3d`；五份复制物与全部源码前后逐项不变，证据 `var/pc05c-qa-round13/freeze-canary-cleanup.json`。

独立主场景为 **2 API + 4 Chrome 浏览器，六项分轮通过**；另有 **1 项 Firefox 原生后台产品补充**。Chrome **154.0.8037.93**、headed/native UA；Firefox 官方 **157.0**、Puppeteer **25.12.0**/原生 BiDi。全部 **0 skip、0 自动 retry**，不能表述为六场一次通过。

| 实际轮次 / 命令 | 结果 | 原始证据（均在 `var/pc05c-qa-round13/`） |
| --- | --- | --- |
| `python3 var/pc05c-qa-round13/execute.py main` | 首轮 3/6，148.84s；C1/C2/C4 通过，C3/C5/C6 保留失败 | `initial-main/playwright.log/json`、browser/error-context |
| 同命令 `main --grep 'C3 \|C5 \|C6 '` | 2/3，78.23s；C5/C6 通过，C3 原门槛再次失败并取得时序 | `diagnostic-author-fix1/` |
| 同命令 `main --grep 'C3 '` | 1/1，63.41s；无伪时钟的 C3 全部原业务门槛通过 | `playwright.log/json`、`evidence/c3-*.json` |
| 同命令 `native` | 1/1，25.59s；真实产品页面隐藏/恢复 | `playwright-native.log/json`、`firefox-native-product/result.json` |
| 同命令 `components` | 冻结 12 文件 **83/83**，一次 4.16s | `components.log/json` |
| 同命令 `rust`，仅执行 copied test bins | **8/8**：DTO4、真实 activity API1、Retry-After2、poll1 | `rust-results.json`、`rust-{1..4}.log` |

Rust 精确过滤为 `jobs-lib-test http::jobs::tests`、`generation-requests-test pc05c_activity`、`jobs-recovery-test retry_after`、`jobs-recovery-test poll_pace_ramps_from_three_to_fifteen_seconds --exact`，均 `--test-threads=1`；未借用 target 产物。冻结组件覆盖 RetryCountdown/JobActivityLink/jobDetail/status、shell/client、work-protection、ItemFormPage/useDraftMutations、ProviderSettingsForm/model-guard/ConfirmStepPage。自有 scoped tsc 与 ESLint 最终 exit0；完整命令与逐轮统计见 `run-summary.json`、`var/pc05c-qa/plan.md`。

### 逐项结论与影响复核

| AC / UI | 结论 | 独立事实 |
| --- | --- | --- |
| AC-PC5-006 / UI007 | PASS | C1 实际执行器：先两次正常远端查询，再六次本机429；实际 Retry-After7/3/1/1/1/1。DTO安全重试1～5、limit5与 attemptCount 同源，pollCount 始终2，最后 failed/safeRetry=null，付费提交计数仍1。C3实际20→25秒 deadline变化、自然归零不POST；缺时间明确等待，poll89不混为安全次数；running/failed/终态移除倒计时，终态后原严格0新详情GET通过；真实unknown只显示对账，不自动重购。C7在同冻结产品上补真实后台恢复，见下段。 |
| AC-PC5-007 / UI008 | PASS | C2真实 COUNT 28（四态各7），列表第一页仅20；五种非在途状态均不计，读取前后逻辑DB摘要相同；真实401。C4顶栏28→27→0→1，0仍发现新任务；失败显示不可用而非0，入口可键盘进入并恢复；shell不拉全任务列表，4500ms只有既有2s汇总节奏；375入口≥44px且无横向溢出。初次pending/旧数据错误另由冻结组件取证。 |
| AC-PC5-008 / UI009 | PASS | C5初访普通登录、错误密码不说会话过期、安全next/恶意next；真实注销使物品及知识PATCH各401，同SPA登录回原路由，375重新打开知识Drawer后原buffer仍在，无自动重放及持久存储。C6实际错误分别 model422/providerModelInvalid、price503/PRICE_CATALOG_MISSING、provider409/PROVIDER_NOT_CONFIGURED；未登出，auth仍200，原PDF真实canvas及既有不可变发布版均可读，未新增job/attempt/ledger/供应商调用。 |
| PC05B 受影响复核 | PASS | C5真实物品+知识401的内存/无重放/无持久正文，配合冻结保护、物品与知识组件；保护框/Drawer/保存核心源码与B3相同。没有冒称重跑B全部六组。 |
| PC06 受影响复核 | PASS | C6真实三个配置业务错误与阅读保持，加冻结模型遮蔽/确认/客户端组件；模型guard、provider override与秘密处理核心Rust未变。没有重开全部配置持久化/加密/远端对账验收。 |

影响依据和逐项未变SHA：`affected-contract-map.json`。场景事实：`evidence/c1-real-retry.json`、`c2-activity.json`、`c3-countdown.json`、`c4-activity-ui.json`、`c5-auth-memory.json`、`c6-config-read.json`。

### 原失败、作者修正与真实后台边界

- C5初轮在375登录返回后直接找已关闭Drawer内字段；实际页面显示“修改仍在本页”。改为从真实“步骤与原文”重新打开，再按原值/零重放/零存储门槛验证通过。C6初轮用了已消费报价物品；产品正确恢复旧job，没有自动新报价。改为另建未报价的真实资料物品触发错误，原发布版仍用于阅读复核，没有删除/绕过已消费恢复事实。
- C3保留两次精确 `/jobs/<id>` 请求失败，确认并非activity或已在途响应：终态200后仍每2秒新发详情GET。原因是作者在应用原生interval已经创建后首次 `page.clock.setSystemTime`；Playwright1.60会在此时安装模拟时钟，而其 clearTimer 只清自己timer表，不能清此前原生ID。代码摘录、安装时点及requestStart/responseEnd见 `clock-author-diagnosis.json` 与 `diagnostic-author-fix1/evidence/c3-terminal-read-diagnostic.json`。最终去掉伪时钟：先读实际 tripo_poll→tripo_submit 依赖及 succeeded 原值，在**私有库**暂置needs_input，证明真实deadline已过仍retry_wait且无外呼增量；观测自然0后还原原值，实际执行成功，再通过未改变的终态0GET断言。仅复跑C3，未修改产品门槛；此隔离展示窗口不代表正常调度性能。原稿与精确变更见 `author-original/`、`visibility-split.patch`、`final-author-delta.patch`。
- 已知Chrome工具无法产生hidden的RD原attempt2/3保留，没有重复同方法或宣称Chrome隐藏PASS。root明确授权独立Firefox产品等价补充：真实任务页 normal/visible→minimized/hidden→normal/visible，原生窗口bounds分别回读minimized/normal，两条visibilitychange均trusted=true；真实隐藏约18.6秒，倒计时1:28→1:09。后台前后jobs1/attempts2/ledger2、供应商计数相同，无非auth业务POST及全jobs列表读取。没有dispatchEvent、visibility属性覆写，也没有把此前静态工具probe充当产品验收。只签该产品后台场景，不签Firefox全部baseline。

### 隔离、限制与清理

生成准备是明确的格式页夹具，随后走真实localhost生成/人工复核/发布；不冒称本轮新PDF.js准备或官方内容质量。跨页计数用私有DB复制有效job身份（没有阶段），只验真实汇总读取，不称38次真实生成。缺时间/running/failed补充状态亦明确是受阻依赖的私有读状态夹具；MAX5和Retry-After来自未改造的实际执行器链路。

前后131前端/211Rust/5复制物hash一致。15491/18491无监听，11个独立实例均停止并删除，自有Firefox profile/runtime/proxy已清理；**75份运行日志/JSON/error-context文本固定秘密canary匹配0**，11次后端日志canary检查均通过，浏览器编辑canary持久化写入0。无截图、trace、HAR或video。证据汇总：`freeze-canary-cleanup.json`、`run-summary.json`。只有协调者可据此更新state/部署；本轮没有操作用户预览、真实密钥或真实付费API。

## Round 14 — 最终原始回归 / scope=full / FAIL（2026-10-02）

协调者执行原七组 `cargo xtask check`：6组通过、Rust workspace组失败；前端35文件/220测试通过，格式、clippy、lint、typecheck、合同一致。全源502文件清单 `var/prd-completion/final-source-20261002-qa14.json` SHA `1a757f2a1fd34e1a0b6f509d8b2c38578033fc2f82b08ce1a8f9f99075a14899` 前后完全一致。使用 `CARGO_PROFILE_TEST_DEBUG=0`、incremental关闭、Cargo离线，未改变release配置。日志与分组结果 `var/prd-completion/final-check-qa14/`。Rust首个失败后未抵达的storage/tripo/core/test-support/xtask/doc目标独立补跑通过（87通过、1个原有doc ignore）；证据 `final-check-qa14-unreached/`。这不改变原完整命令FAIL。

- **BUG-PCF-001 / P2 / OPEN**：设置页进入“恢复部署配置”仅切restore布尔值，取消后复活同卡未保存地址、模型、新密钥及replace动作。原AS-QA-09在真实Chrome154.0.8037.93复现“保留现有”未选中。原api-settings PRD §恢复交互要求进入时立即清新密钥、取消回最近saved+keep；PC06仅对拒绝恢复要求保留其他编辑，并未改写正常取消合同。两位独立只读复核一致，详见 `var/prd-completion/as09-readonly-diagnosis.md`。最小修复须只重置该卡，保留另一卡编辑；服务器/网络失败不自动丢弃其他编辑；不得借旧错误组件断言降低原合同。
- **BUG-PCF-002 / P2 / OPEN**：`redaction_surface::write_functions_either_redact_or_are_exempt` 报 `claim_next_for_job` 未登记。实际写入从旧 `claim_next` 移到该函数，只有worker身份、状态、epoch、时间和NULL，job scope只参与筛选；无供应商自由文本写入。修复是将显式纯状态豁免精准从旧wrapper名迁到实际函数并注明依据，不增加宽泛豁免、不假调用脱敏函数。原守卫失败与真实脱敏持久化测试PASS均保留。

Chrome原始138条矩阵尚未完成：私有拒绝代理的CONNECT socket未处理ECONNRESET，Node异常中断；属于工具故障，不是产品PASS或产品网络错误。原输出留在 `browser-matrix/qa14-original-chrome/`，QA将追加精确已执行计数和清理记录。Edge/Firefox未启动。工具仅补socket错误处理，源码修复后重新冻结并继续完整回归。PC06/PC05B暂needs_retest，其他slice结论保留；发行、性能、Linux、真实T23仍未签full通过。

### QA 独立补充：原始 Chrome partial 的可核实边界

使用 actual **Google Chrome154.0.8037.93**（不宣称最新 patch），原23spec精确字节、预期138场。CONNECT工具中断时没有最终Playwright JSON，buffered stdout未落盘；只可确认**5份失败error-context工件**，已完成PASS/FAIL/SKIP总数均未知，不能表述为“5/138失败”。五项为AS-QA-09、import-flow正常向导、IA-QA-03、IA-QA-05、IA-QA-06；后三类文案/旧恢复交互/三tab假设的归属详情保留在 [partial草稿](../../../var/prd-completion/browser-matrix/qa14-original-chrome/qa14-partial-draft.md) 与 `partial-failure-index.json`。IA06自行启动bundled Chromium并120s超时，不属于actual Chrome/Edge缩放通过证据。

原runner及工具错误、最小CONNECT修复均保留；仅`node --check`静态通过，未动态重跑。502全源/C131前端/原23spec/复制fixture/Chrome可执行文件hash核对不变；21份运行文本工件secret-pattern扫描0，原settings显式截图mask了password且全部来自合成实例。自有PGID4554已空、15540/18540无监听；Edge/Firefox均尚未启动，已通知后者等待新freeze。精确事实见 `var/prd-completion/browser-matrix/qa14-original-chrome/partial-integrity-cleanup.json`。未改原AS09或产品以绕过失败。


## Round 15 — 最终修复复验与全量验证 / scope=full / FAIL（2026-10-02）

新冻结502文件 `final-source-20261002-qa15.json` SHA `d77c5ef822ac0153c666203ca618f5eeaf5f533b392afb698892e52bb94d6267`。相对QA14仅设置表单/单测、独立PC06测试和脱敏守卫四文件变化；生产Rust未变化，增量与新131前端/211Rust身份见 `var/prd-completion/qa15-final-ready/freeze-delta.json`。原QA14失败不覆盖。

协调者独立执行原七组 `cargo xtask check`：**7/7 PASS**，Rust **621通过/0失败/10忽略**，前端35文件**224通过**，耗时192.05秒，前后全源MATCH。仍使用测试debug=0、incremental关闭与Cargo离线；日志 `final-check-qa15/`。10忽略分别为手工造数1、显式本机Keychain父/子2、PC04显式CLI6、文档示例1，不冒称这些由默认命令执行。**BUG-PCF-002 CLOSED**：原两项结构守卫全部通过，清单仍精确10项，真实脱敏持久化回归也通过；生产逻辑未加绕过。BUG-PCF-001待独立浏览器复验。

独立QA随后运行最终复制fixture/xtask/publishing-test：PC04 **8/8 CLI + 6/6显式ignored**一次通过，0自动重试/skip，分别33.64/22.7秒。完整502源、镜像、复制物前后不变，业务断言无delta；13份运行工件canary扫描0，自有进程清理。报告 `var/prd-completion/qa15-pc04-final/report.md`。这六项由显式路径实际覆盖，不篡改默认check的10ignored统计，也不当真实T23。

原生命令 `cargo xtask dist --target aarch64-apple-darwin --check-reproducible` 与正常复制runner七步smoke通过：release **26,038,672字节**，两次独立构建SHA均 **`7038a96e292d1b5670658348cb872fbda1e667549b927be381d00f71acbc5684`**；only embedded-ui，无job-failpoints。系统动态依赖合规、禁止构建路径命中0、licenses/build-info/SHA256SUMS齐全；冷目录/离线沙箱/重启/备份恢复smoke3.35秒，未跳过断网步骤。证据 `var/prd-completion/final-macos-dist-qa15/`。未签名公证、未部署到用户实例，正式包实际浏览器编辑链与五分钟性能尚在执行。

T22首次实际Chrome正常包浏览器烟测保留工具失败：私有helper要求含 `/api/v1` 的路径而调用方传资源相对路径，尚未到后续业务；同时静态发现设置状态字段写错。此为未动态演练工具的作者错误，原失败 `t22-final-qa15-chrome-attempt1/` 保留，自有浏览器/服务已清理。仅私有工具按真实合同修正，不修改产品或业务门槛。后续结果另追加。


### QA15 后续实际结果与 BUG-PCF-003（2026-10-02）

独立定向报告 `var/prd-completion/browser-matrix/qa15-targeted-report/report.md`：原 AS02/08/09 + B6 首轮4通过；PC06第三场因作者 exact heading “Tripo”定位超时，保留失败，仅私有修为真实标题“Tripo · 模型生成”后1/1通过。5场分轮闭合，0skip/自动retry，冻结设置13 + 保护6 = **19/19**。进入restore立即清本卡key、取消回最近saved+keep、另一卡保留及失败边界原门槛通过，**BUG-PCF-001 CLOSED**，PC06/PC05B本次影响复核PASS，可恢复accepted。未匹配的Drawer过滤项不计覆盖；14份运行工件秘密扫描0，源502/web131/二进制前后未变，自有实例清理。下一候选源码仅补已证明的PC06标题locator，精确delta `qa16-author/`；不覆盖原失败。

`qa15-pc-matrix/README.md`：PC03B **3/3 API**、PC05A **2/2 API**一次通过；原业务spec/helper字节保留，冻结校验与清理通过。两卡browser仅准备，尚未执行，不得按收集数当覆盖。

正常发行版7038a96e的 **300秒真实M1性能 PASS**：`performance/final-qa15-attempt1/`，持续300010.6ms、18000帧，p95 **18ms** / p99 18.8ms，100000三角形实际可见；items300样本p95 **10.3ms**、jobs100样本p95 **9.6ms**。8991真实指针事件、299资源观测；Chrome154.0.8037.93/ANGLE Metal，16GiB M1。前后source502及DB业务计数不变，无外部转发。root独立按原样本复算nearest-rank并查看截图，结论见独立 `review.json`，不篡改原summary.review。此为本机本候选结论，后继改变候选不自动继承性能PASS。

T22第二轮 `t22-final-qa15-chrome-attempt2/` 原正常包Chrome脚本 **PASS**：真实PDF.js准备、资料上传、raycast校准、人工编辑、发布201、旧manifest字节不变、资产200/HEAD/206、无provider409不新增job/attempt/ledger、重启与正常备份/空目录恢复全部完成。首轮路径/DTO/字段locator私有工具作者修正保留完整delta；没有改产品门槛。**脚本的人工编辑断言仅验证manifest，root视觉复核发现以下新产品遗漏；因此脚本PASS不代表该合同完整通过。** Edge/Firefox正常包链路未执行。

- **BUG-PCF-003 / P2 / OPEN**；关联 REQ034/AC054、REQ035/AC055、REQ036/AC057、UI051 与 ADR030修订原文对照/发布冻结合同的组合语义，并非声称AC057单句逐字段明定overlay。复现：Chrome T22真实UI把部件名改为“ T22 已通过界面保存的合成部件”（保存值无首空格）→保存→发布→重启/恢复后读该版本。manifest.review 的 userEdited 已持久化，但阅读卡与关联chips仍显示原“后盖”。期望：阅读页使用该不可变版本自己的六字段人工修订，标明人工修订且原文/出处可对照，后续草稿或版本不能覆盖历史版；实际：仅取manifest.knowledge原字段，review只用于textOnly。证据 `t22-final-qa15-chrome-attempt2/evidence.json`、`05-restored-new-release.png`，独立静态合同审计 `release-overlay-readonly-audit.md`。修复范围限定release投影及Reader显示：Part name/description、Step title/orderedActions、Spec label/value，保留id/evidence/partIds/safetyNotes/model/hotspot；空字符串/空数组按字段存在性处理。新回归文件验证真实编辑与多版本隔离，不改原23spec字节。

QA15 **full FAIL**：上述新必选产品缺陷未闭合、完整浏览器与Linux/真实T23未完成。已有实测结果和未覆盖项原样保留，进入单一RD修复；不以全量代码测试/发行/性能通过替代产品全量验收。


## Round 16 — 阅读修订复验与最终矩阵 / scope=full / FAIL（2026-10-02）

QA16全源507 manifest `15e75d82075766a452fbc126855487cc0ee1375b2d2e86e364032cf9b56ee7e6`，前端134 manifest `b9276e0ca9f10730fa7773cc438f8f73b0f17286adab051825dc7a27cfb874f9`。Rust211未变，明确复用QA15复制fixture20d1ad2a而非声称重新编译。协调者原七组check **7/7 PASS**，220.707s，Rust **621/0/10ignored**，前端 **37文件232/0**；前后全源MATCH。见 `var/prd-completion/final-check-qa16/`。

独立 `browser-matrix/qa16-reader-report/report.md`：**BUG-PCF-003 CLOSED**，真实UI六字段编辑与A/B发布、A→B→A修订/原文/几何引用/manifest与全部资产SHA不变均通过。新增首轮0/1为测试原文相同“取下后盖”标题与动作的严格locator冲突，不是产品；私有修为全部段落与有序动作逐节点可见，保留键盘展开/44px，第二轮1/1（14.09s）。5个原阅读/出处/下载受影响场景5/5（50.52s）；均0skip/自动retry。4截图独立查看、11运行文本canary0、自有进程/端口/临时目录清理。原失败与精确作者差异保留。

协调者在检查及QA停止后将这唯一已实测spec作者差异同步源码，形成 **QA16b507** manifest `346c83fb778a8b6934773250c2aeacf5aad185c809c562e482408fa8c84b8df3`；产品134web/211Rust完全相同。当前spec SHA `bc04967b5941a7e2390298872f0af1ed1b05b9299d492f9ac957196ff90ef718` 与实际独立运行字节一致；默认tsc和该spec ESLint再检查exit0。上述完整check作为 **REUSED_REVIEWED**（只影响新增测试定位），不称为QA16b重新运行整套。变更与来源在 `qa16-final-ready/qa16b-delta.json`、`qa16-author/`。

QA16b正常 macOS `cargo xtask dist --target aarch64-apple-darwin --check-reproducible` **PASS**，121.43s，两次独立构建SHA **`e0854d698680476cc9e74689ce1d612164c0aaf06773d0f8c32becfad7797184`**，26,038,672B，只embedded-ui。正常复制runner七步smoke3.37s、断网沙箱未跳过、全源前后MATCH，证据 `final-macos-dist-qa16b/`；尚未部署用户实例。原全浏览器清单为 **24spec/139**（原23/138+新增1），收集非执行；Chrome原样矩阵进行中，未宣称full PASS。

### BUG-PCF-004 / P2 / OPEN — 已确认报价在部署密钥缺失后仍可建单

关联 **REQ007/AC012**：无供应商密钥时 estimate/jobs 应明确拒绝、不创建job或费用记录、不外呼。root独立隔离实际HTTP复现：以本机假部署key创建合法ready资料、报价201、确认200；停服后仅把deployment api_key_env改成未设置变量，再重启，settings明确Tripo/ManualAI均false；持原未过期已确认报价、新幂等键POST jobs，**实际202**，jobs **0→1**、cost_ledger **0→2**、snapshots **0→1**，attempts0且fixture全部wire计数0。全部为合成实例，未读用户配置/密钥、无真实供应商或费用。

预期新提交返回明确未配置错误，四类持久数据与wire计数保持0；已受理任务的同键同body重放需保持读取语义，不新增购买。证据 `var/prd-completion/no-key-submission-diagnostic/observation.json`、`runner-attempt3.log`、私有原case。首次两轮为ESM模块/目录导入工具错误（尚未运行用例），日志保留；第三轮真实产品失败，未修改冻结source507。根因初判：jobs提交只有pending/model/revision门禁，deployment密钥缺失并不改变overlay revision，也没有复用estimate的provider配置可用性检查。RD先在私有目录准备最小补丁及each-provider/no-write/replay回归，待Chrome原矩阵结束后开源码编辑窗口；不把本缺陷伪装成价格或真实API未授权阻塞。


### QA16 原 Chrome 完整结果与 QA17 接续

原24spec/139场全部调度完成：**105 PASS /31 FAIL /3 SKIP，0 flaky/自动 retry**，1971.863秒。3项是 workflow-recovery 首场失败后的串行跳过，不算通过。实际 Chrome154.0.8037.93，完整507源、134前端镜像、复制fixture及浏览器前后hash不变；35运行文本和34生成文本canary匹配0，自有进程及15540/18540端口清理。原始结果与逐场清单：`var/prd-completion/browser-matrix/qa16-original-chrome/`；独立分类 `qa16-original-report/`。Edge/Firefox原矩阵尚未执行，不将Chrome结果迁移过去。

31失败包含已经改版的准备页、恢复发现、三面板与来源控件旧断言，以及缩放测试自行启动bundled Chromium、异步GPU清理过早采样等工具问题；viewport/raycast等仍需真实动态诊断，不能统称所有失败均为旧测试。保留原字节、失败输出与业务门槛后，授权QA更新当前canonical旧E2E；每项记录旧门槛到当前交互的映射，不用降低布局可用性、唯一提交、付费、资源释放等合同来通过。BUG-PCF-004实际产品缺陷仍未闭合，故QA16为full FAIL。

## Round 17 — 配置提交边界修复与当前交互回归 / scope=full / FAIL（2026-10-02）

BUG-PCF-004单一RD已交付5生产文件及新增HTTP测试：新job校验两供应商，付费retry/replacement仅校验目标供应商，幂等回执及已受理任务本地恢复保持原语义。精确delta/旧字节在 `var/prd-completion/qa17-api-fix/`。root首次运行5新增测试：4通过，1因initial/replay的reservations数组顺序不同而失败，其他JSON字段相同；原日志 `qa17-api-focused/new-regressions.log` 保留。该测试修正前不称新增测试全通过，也不以静态交付关闭缺陷。全部为本机合成实例，无用户数据/密钥/真实计费。

### BUG-PCF-004 独立复验 CLOSED

先核对合同：JobDto.reservations 没有数组排序要求。作者仅对该新replay测试按(provider,currency)排序后比较完整JSON，数量/金额/状态/币种/显示值及所有任务字段和无写入仍严格比较；五生产文件未动，原失败和差异保留 `qa17-api-fix/replay-order-author-fix/`。root重跑新增5项/26状态组合全部通过，再跑原相关 `api_settings_qa`7 + `generation_requests`40 + `pipeline`14 = **61通过/0失败/0忽略**，211Rust前后不变；`qa17-api-focused-attempt2/`。

从当前Rust重新编译复制fixture SHA `e0be0d57b4a3bc82912bc016c773361f637710659920b71773e416ef0e3aa426`，非正常发行版、无embedded-ui；真实进程独立复验分别丢失Tripo、ManualAI、两者共**3/3通过**：合法报价确认→停服→仅更改deployment引用的环境变量名→重启→原报价提交，全部 **409 / PROVIDER_NOT_CONFIGURED**，jobs/attempts/cost_ledger/snapshots始终0，本机fixture upload/submit/task/manual/cdn始终0。原失败202与新结果并列，证据 `var/prd-completion/no-key-submission-qa17/`，系统沙箱只允许localhost，未读用户数据/密钥。原幂等重放、各目标provider付费retry/replacement、本地恢复与accepted边界由前述独立运行HTTP测试覆盖。此缺陷关闭；不代表完整浏览器、正式包或真实供应商质量验收完成。

### QA17 冻结与完整代码检查 PASS（不代表 full 验收）

新507文件 manifest `51af1eaadd1455879e4ed13a219733eb849938befa6500fb6e7be6c51374646d`，相对QA16b有20代码/测试文件变化（6Rust +14旧E2E/helper）。前端产品134文件manifest仍 `b9276e0ca9f10730fa7773cc438f8f73b0f17286adab051825dc7a27cfb874f9`，211Rust manifest `6af9722eed845698e57cd65f962773540914f3641c847340da90757e06421ee7`；精确delta `qa17-final-ready/freeze-delta.json`。旧测试作者修订说明与原始文件 `revision2-test-author-qa17/`，仍24spec/139，无减少case/新增skip；这些是静态收集，未当动态通过。

协调者原七组 `cargo xtask check` **7/7 PASS**，235.49秒，Rust **626通过/0失败/10原有忽略**，前端 **37文件232通过**，全源before/after MATCH；`final-check-qa17/`。复制正常xtask与publishing test之后，为磁盘空间仅 `cargo clean --package everything-manual --profile dev` 清除1.9GiB可重建缓存，复制fixture/正常runner仍保留，未清用户数据/依赖。

12:53Z 再次只读核对用户本地预览：两provider配置均true、pending=false、模型误填状态均null，价格目录false、生成能力false。`provider-readiness-qa17-corrected.json` 仅保存布尔状态、无密钥/地址、供应商请求0；首个探针误用扁平price/capabilities字段得到null的结果另存保留，没有当作false。真实价格目录/预算仍是T23未验的外部前置条件。

### QA17 正常 macOS 发行构建与两个浏览器等价场景

正常 `cargo xtask dist --target aarch64-apple-darwin --check-reproducible` PASS，122.15秒，两次独立release SHA均 `f80f6fbccad360a8ad3c9105b00f912c0673593afd11abdc7c8cdc4c31eae377`、26,032,864B，只有embedded-ui。正常runner七步smoke3.515秒PASS，断网沙箱未跳过、全源前后MATCH；`final-macos-dist-qa17/`。未部署用户实例，T22浏览器/性能另有prepared输入，未当执行。

root实际Chrome154的 **IA06原生200%产品等价1/1 PASS**，6.46秒：浏览器内置appearance的zoomLevel真实选2，原生windowId/bounds完全不变，innerWidth1440→720、DPR2→4；原确认预算/checkbox/键盘焦点/真实elementFromPoint/生成可操作与新建表单全部业务断言保留，无横向溢出。私有仅替换已失效的bundled Chromium扩展机制，原业务源canonical未改；首轮因单独运行未创建截图父目录失败，在首张截图前的确认断言已通过，补私有mkdir后完整通过；两轮保留。`browser-matrix/qa17-chrome-native-zoom-product-attempt2/` 内原生边界、图片与精确adapter。root已目视复核确认页截图，未声称是所有浏览器缩放验收。Edge原生设置探针为TOOL_GAP：未找到可操作combobox/select，未切到200%，自有浏览器/profile/proxy清理，不计产品失败或通过。

**QA19-1三维实际拾取等价1/1 PASS**，10.48秒。原只加观测的诊断保留FAIL：-500滚轮的radius轨迹4.23→7.52→14.78→37.70→1682.98，画布575px、中心仍在287.5px，但测试按半径求offset841px导致目标出界；不是产品canvas本身溢出。私有只在半径接近30后将真实wheel步长改-50（不改相机/不clamp），轨迹37.70→42.81→49.17，原>45前置保持、offset25>18且<0.9R；另将原纯页码locator改为当前document+page来源控件。所有真实raycast落模型表面、finite、anchor落库/身份、长按/拖动不建点、点热点不新增、1-based来源断言都通过。原两处差异、额外观测、失败及原始结果见 `qa17-raycast-diagnostic-attempt2/` 与 `qa17-raycast-equivalent-attempt1/`；root目视复核最终来源截图。首个诊断用例标题过滤误加^，0场执行的No tests found亦保留，不算产品结果。

上述两个私有等价通过与当前canonical137场矩阵分开计数。137矩阵明确排除这两个场景，原3场serial跳过仍须真实执行；矩阵运行中，不能合并宣称139全部通过。

### BUG-PCF-005 / P2 / OPEN — 资料库操作点击区域不足

QA17当前真实Chrome ROW1/ROW2实测历史链接宽度40px，未达44px，包含375px窄屏；高度已为44px。关联本PRD rev2 PC03B UI-PC3-005～008的375px关键主/次操作≥44、PC05A UI-PC5-003行入口主要操作44（当前正文line416/513）。现有canonical ROW1/ROW2保持44px门槛并独立失败，证据 `browser-matrix/qa17-current-chrome/test-output/library-row-layout-*/error-context.md`；最终统计待矩阵结束。先单一RD只读确认具体选择器并在var准备最小候选；冻结矩阵未结束前不写生产样式，不下调阈值。

BUG-PCF-005测量更正：QA运行中初报误说height40，root曾据此发进度；RD只读逐行复核两份error-context与同轮library-row-geometry.json，断言实际是**width**≥44。六桌面档primaryAction84×44、historyLink40×44；styles.css已有44高度的选择器优先级正确。真实缺口是历史链接点击宽度，候选仅补限定行操作链接min-width44，不重做高度/全站样式。保留此更正说明，最终以原始几何及断言位置为准。

### QA17 当前137场矩阵最终结果 FAIL

实际Chrome154.0.8037.93 **130 PASS /4 FAIL /3 SKIP /0 retry/flaky**，581.786秒。两产品失败是BUG-PCF-005历史链接width40；另两作者遗漏是QA16-QA1后半概览旧「打开说明书」定位（未到后半真实行流程），workflow首场仍期望分页替换1行而当前PC05A合同追加共21行。该serial首场失败后3场未执行，不能沿用旧结果当通过。全源507/web134/spec/fixture/browser前后SHA一致，9运行+42工件文本canary/session/CSRF匹配0，自有PGID29659及15540/18540监听均空。报告 `browser-matrix/qa17-current-chrome/report/report.md`、逐场 `case-index.json`。首轮沙箱EPERM发生在proxy监听，0执行，另存工具失败目录；本轮取得本机执行权限后真正跑完137。

## Round 18 — 资料库点击区及遗漏测试分支 / scope=full / IN_PROGRESS（2026-10-02）

BUG-PCF-005单一RD限定真实资料库行链接min-width44；既有44高度、排版、字号、路由保持。仅有一个CSS规则的候选已获第二人静态复核，执行后仍待真实8档ROW1/ROW2及PC05A检查，不以静态解析代替浏览器。三个测试作者文件限定补现有交互/前置：QA1历史版本与新空物品资料缺项；workflow累加分页21与新增条目定位；QA19同步已实际通过的更细真实滚轮和来源locator。业务assert和case数量保留，不新增skip、不会重新跑已不受影响130场只为改轮号。前述Chrome native缩放与raycast等价结果保留身份与范围，受影响结果必须实测。

### QA18 冻结与正常 macOS 包

507源manifest `6550edc447c3ded4da2c4ba6cb34cf91fd36747911b3524a4506d7599a0fde2b`；134web `bb6e013555eaa58302629c084888a4396230d178def08ce82d06a2ba3d0d4471`，211Rust与QA17字节相同（`6af9722e`）。只变CSS及三个E2E文件，delta见 `qa18-final-ready/freeze-delta.json`；QA17 fixture显式复用，不冒称重新编译。

正常 `cargo xtask dist --target aarch64-apple-darwin --check-reproducible` **PASS**，118.577秒，两次独立SHA `2ccceb9d0ba26cdca76686564e625ff60fa995ecfe6cdaac72b76d98a7aa4d2c`、26,032,864B，仅embedded-ui；复制正常runner七步smoke2.314秒PASS，断网沙箱未跳过，507源前后MATCH。`final-macos-dist-qa18/`保留实际日志、包、构建身份；未部署用户实例。T22三浏览器与300秒性能输入已绑定新包，当前只是prepared。

### BUG-PCF-005 独立动态复验 CLOSED；受影响两卡恢复 accepted

独立QA在实际Chrome154.0.8037.93定向 **baseline8/8 PASS（48.913秒）+ PC05A4/4 PASS（20.522秒）**，0skip/retry/flaky。原workflow第一场后的3项serial这次全部实际执行；分页追加21、真实响应丢失后唯一恢复/未消费同key同body再试、PDF/视图独立导航均通过。原ROW1/ROW2八档375/767/1024/1280/1366/1440/1600/1920的历史链接实测44×44；QA和root分别目视375截图，QA另查1280。PC05A搜索/URL历史/失败保留/两真实发布/不同版本双ZIP亦全部通过。`browser-matrix/qa18-prepared/report.md` 与原两套result/geometry为证。

全部spec字节保持；仅PC05A私有backend init/serve加入OS localhost网络约束（exact patch已存），无成功HTTP伪造。两套source507/web134/bin/browser before/after一致；20份运行/工件文本已知canary/session/CSRF匹配0，外网转发0；自有PGID34180/34671、四端口与临时PC实例清理。PC03B/PC05A仅恢复此次影响范围，未重复执行不受影响API并明确引用前轮证据；不称原卡每项QA18重跑。QA17的130通过/4失败/3跳过历史与两私有等价保留，不能把拼接写成一轮139原样全通过；当前完整release gate仍未完成。

### QA18 普通包 T22 三浏览器实际烟测 PASS

同一正常embedded二进制 `2ccceb9d…`，实际 headed Chrome154.0.8037.93 **1/1，14.991s**、Edge154.0.4258.53 **1/1，16.751s**、官方Firefox157.0 BiDi **1/1，17.971s**，各0skip/retry；每场包含真实UI PDF/两照片上传与PDF.js逐页准备、模型旋转与表面raycast/持久化热点、人工文字修改/新发布201、旧版manifest不变、实际停启和backup→空目录restore→重登录/嵌套路由刷新。新增阅读修订断言在发布后/重启后/恢复后各实际执行一遍，正确显示修改名、说明、人工徽标、原文展开及选择提示。GLB/PDF实际hash与HEAD/206保持；缺供应商409且jobs/attempts/ledger无新增。三个cold目录已删除，原备份未改，普通包无failpoints；路径 `t22-final-qa18-{chrome,edge,firefox}-attempt1/`。

这是T22本机普通包浏览器完整烟测，非T21全部异常矩阵、依赖未安装目标机证明、Linux或T23真实供应商质量。无provider外呼、未访问用户数据或密钥。独立截图/工件复核另存，不由通过一场推导full release。

### QA18 当前普通包 AC-061 具名设备性能 PASS

同QA18 normal `2ccceb9d…`、full源`6550edc4…`，Macmini9,1/Apple M1/16GiB/macOS26.6.2、实际Chrome154.0.8037.93/ANGLE Metal。100000三角面、内嵌16×16纹理、2,215,892B合成发布模型；独占本任务CPU/浏览器窗口连续 **300016.9ms**，18000真实viewer帧，p95 **18.5ms** /p99 **19.2ms**，阈值≤33ms；资料库300次/任务100次真实接口各p95 **10ms**，阈值≤200ms。8996条trusted指针事件，最大间隔66.6ms；新Chrome进程首次进入模型到rendered ready858.6ms，明确不是操作系统文件缓存冷启动。

root逐条重算原始JSONL统计、核对同模型真实资源/可见焦点、任务/账本前后不变和零provider/外部转发/页面错误，并实际查看final-reader.png。独立后续review在 `performance/final-qa18-attempt1/review.json`；原summary的NOT_RECORDED保留不改，后续复核单独记录。后台/profile/proxy均清理。这只签该具名机器/模型的AC061，不称4K最坏纹理、150MiB加载保证、AC062十次切模型或其他浏览器性能通过。首次执行权限审核超时发生在进程创建前，0执行；自动允许的一次相同安全命令重试实际完成，没有隐藏产品失败。

### QA18 AC001 bootstrap 与工具子进程观察补证

原正常smoke日志“pgrep不可用/跳过”来自xtask对退出码1的误判：该码也表示当前进程无匹配子进程，不能推断工具缺失。未改运行中的源码冻结；使用明确复制normal runner187143…对同normal2ccceb9d…执行 `smoke-bootstrap`，冷目录根页/实际JS/health/404等通过exit0；另起同SHA普通包在仅binary/data/合成password的新临时目录、最小系统PATH、随机测试master、OS仅localhost下实际init/serve/health。`qa18-bootstrap-childcheck/attempt4/result.json` 三次核验父进程仍运行、ps路径规范化后和文件SHA正是复制normal，`/usr/bin/pgrep -P`均exit1且stdout/stderr空，证明观测时无子进程；自有服务和cold目录已清理。

前三次私有观察器尝试均保留bootstrap exit0，后续工具断言未通过：早期路径文本比较未处理macOS `/var`与`/private/var`同一对象，第三次TextIO缓冲与select组合漏读就绪行。分别记录精确delta后以双侧resolve+SHA和原始字节读管道修正，第四次实际完成。全部不修改产品，不以外部Python驱动自身的存在推导产品依赖Python，也不声称本机物理未安装Node。

Edge当前原可移植138/138实际PASS、575.244秒、0skip/retry，source/web/spec/bin/browser前后一致；`browser-matrix/qa18-current-edge/report.md`。原IA06仍独立工具缺口，附加qa-pc不在此138库存，不泛称AC063全通过。Firefox首轮发现私有default网关改Host导致无public_origin的global登录403，产品安全校验符合预期；已中止重复前置失败，保留原始unit结果，后继只修私有传输，重新执行受影响未验项。

### QA18 原66项AC逐条台账

采纳独立提案到 `var/prd-completion/final-acceptance-ledger.json`，原planned台账另存。187个具名Rust测试在QA17完整日志的218处AC引用及当前函数/依赖身份已复核；61输入文件SHA验证，唯一变化为root继续追加的qa-report，审计所读153523B前缀SHA逐字匹配，追加单独留痕。当前4个个别AC完整通过、12审核复用、37具名目标测试通过（不等于全条款）、11部分覆盖、2阻塞。Edge138、T22三浏览器、性能/bootstrap和三个缺陷闭环已逐条映射，没有按626总数批签。Firefox后继/100页新增结果仍待条款级合入，不提前签full release。

### QA18 AC062 精确100页与PDF/3D懒加载新增局部PASS

独立作者准备、root实际Chrome154执行1/1 PASS19.619秒、0skip/retry，证据 `ac062-exact100-prepared/attempt1/`。从仓库原101页合成PDF确定性截前100页，全部保留页内容字节/媒体框/旋转/文字一致、输出SHA8ff58fb4…；真实UI处理到第4页暂停传输、取消后服务端精确保留1..3，旧canvas尺寸归零；恢复只补4..100，实际PUT总序列恰1..100，显式seal前仍preparing，按钮封存后ready/pageCount100/clientDerived。整轮最多一个有尺寸canvas/一个JPEG编码/一笔pagePUT、DOM渲染canvas为0；两轮复用canvas总创建2而非100。

同会话库首屏无PDF.js/worker/three/R3F/ViewerStage请求；实际100页准备与之后独立两页夹具模型渲染分别提供PDF/3D正控制，100页本身未送供应商。root复算关键原始字段、看ready100截图、运行文本canary扫描0；全源/前端/原spec/新增spec/PDF/bin/browser前后一致，外转发0。明确仍未在这一个PDF会话内连续切10模型/采堆，不将其他会话通过拼接为原AC062完整同会话证据；台账保持PARTIAL并将缺口收窄到该组合。


### QA18 Firefox 受影响重跑与真实 wire / 原生下载补证

修正私有代理默认Host后，67受影响或尚未执行场完成 **64PASS/2FAIL/1SKIP**，0retry；前轮57不受Host影响的通过依据逐文件/依赖/浏览器冻结复核保留。合计原可移植124项为121PASS/2FAIL/1SKIP；失败分别是QA19与stale重绑的画布点击坐标落到视口外，后续model声明被serial跳过，不把跳过计为通过。输出 `browser-matrix/qa18-firefox-portable-attempt2/`；原失败与精确139 disposition保留。

root独立执行私有真实wire适配 **3/3PASS15.007s +4/4PASS16.631s +2/2PASS9.965s**，官方Firefox157、0skip/retry，补G01/02/03/05/06/07/08/10/11。原UI/预算/唯一建单/恢复断言保留；实际非空请求Buffer和raw key只在内存比较，首请求未到后端、显式重试实际202；已受理响应丢失则通过GET恢复，不重复UI提交。真实Rust201报价只注入原expiry故障，真实旧confirm200在新quote201之后交付。root逐项核验原始wire摘要、时序、body/key相等、zero upstream负例，源码/镜像/原spec/bin/browser未变，外部转发0、canary0、原始正文/密钥不落盘、测试进程与端口退出。`browser-matrix/qa18-nine-wire-root-review.json` 记录范围与三组结果。首次无权限ps观察失败未产生报告，随后经只读执行核实清理。

原生下载等价 **5/5PASS**：官方Firefox真实begin/complete事件与6个实际ZIP逐资产hash匹配；历史版本不变、新版本不同、375/1024/1440点击区域达标。独立只读复核 `qa18-firefox-viewport-prepared/independent-evidence-review.json`；第五场与portable第五重叠，139库存只新增关闭4个缺口，不能加5再重复计数。negative窗口由实际执行断言证明，原窗口JSONL随runtime清除，保留结果/适配器/总原生协议，证据限制明确记录。

原生heap与zoom各第一轮未到验收：heap因重新查询target时actor身份变化而停；zoom在真正缩放前等待库标题失败。新zoom诊断证明产品已登录、visible/focus=true且h1内容为“资料库.”，其点号aria-hidden=true；私有工具直接textContent严格比较与原accessible name不同，后台RAF假设没有得到支持。保留两次失败，不改产品标题或放宽缩放门槛。视口3项、heap身份和标题语义适配继续定向诊断，尚未签Firefox全矩阵。

### QA18 AC062 同会话组合完整目标 PASS

独立作者准备、root在实际headed Chrome154.0.8037.93执行 **1/1PASS47.327s**，0skip/retry；新证据 `ac062-combined-prepared/attempt1/`，先前PDF局部结果不改。原PDF/lazy73条断言保留，新增原T18十轮资源/堆观察体。同一浏览器context内精确100页处理、第4页取消后保留1..3、恢复仅补4..100、显式封存ready100；最多1个有尺寸canvas/编码/PUT，首屏不加载PDF/3D引擎；之后两个真实服务端草稿往返10次，每轮活动资源1/1/1且实际deleteBuffer/deleteTexture发生。

root重算保留堆首末三样本中位数比 **1.0849627583 <1.5原门槛**，10个原始样本完整；12context、等待后lost10，最终卸载11/11几何体/材质/纹理/模型、modelsAlive0。该比值并非内存绝对不增长或无限时长无泄漏的证明。实际查看ready100截图、12文本已知canary匹配0；全源/镜像/spec/PDF/bin/browser守卫通过，外转发0，测试只用本机合成fixture。原memoryObservation保留初始化pending文字，后续完成字段/原始trend及独立review明确实际通过，不改写历史输出。台账AC062在具名Chrome完整条款范围改为AC_FULLPASS，AC063其他浏览器的资源/交互门槛另列；当前5个个别AC完整通过，不表示full release。


## 2026-10-03 用户指定Chrome单浏览器真实Wii U流程

用户明确取消其他浏览器验证要求，独立真实样本成为当前目标；旧Firefox/Edge失败/准备工件保留，不继续扩展矩阵，不把其未验状态涂为PASS。用户确认LLM内部免费额度，并在实际30 credits /0 USD报价后授权本任务最多9000 credits（用完后再审批）。新普通QA18包2ccceb9d运行于独立8082实例，通过现有密文和用户macOS钥匙串授权启动；SQLite在线backup+APFS克隆原资料，原8080/5173及原data-dir不改。

Chrome154.0.8037.93真实读取Wii U34页ready及front/back两照片，原件/全部页资产hash本轮核验。当前34页有文字层，所以实际quote发送34页提取文字、7个LLM批次，不发送页图；Tripo发送两张原始实拍。页面真实报价3000 creditMinor /0 usdMicros后显式提交job01a0fd7f-0528-7440-a34d-d75279cd18c4返回202。

首次真实任务未完成：LLM7批均HTTP400明确拒绝模型GPT-6-Astra并提示GET/v1/models；Tripo照片上传HTTP401/code2/Invalid API key，付费建模提交从未运行，远端task ID为0。已通过UI取消这条失败任务，保留全部错误/attempt/账本；初始30 credits仍显示reserved且actual=null，不能篡改为实际账单0，未读取账户账单。现无有效生成结果，不称真实端到端PASS。证据var/prd-completion/chrome-live-wiiu/下quote/submission/status/first-attempt-result。

协议只读核对：POST v3/files、Bearer、multipart字段file与官方文档一致，401可能无效key或全球.ai/中国.com区域错配，不据此改上传协议；已请用户在新版设置核对。唯一RD正在新增显式读取已生效网关可用模型列表的受保护设置操作，避免继续盲猜模型名。当前仅推进该修正所需的Chrome与局部检查，旧全矩阵不再作为本次样本闭环门槛。

### Chrome 真实流程续跑：配置修复与第二次实际提交

用户确认已在新版设置保存 Tripo 配置。新增显式模型读取接口通过 Rust 集成 7/7、前端目标 21/21、lint、全前端 typecheck 与生产构建；Rust 测试初次因 SQLx 静态 SQL 约束编译失败，改为静态字面量后重跑通过。普通 embedded-ui 包 SHA `b9c24fd4ab6d8f11e48d6733cac08c7a2ab83eef0137acda6906fca24b3d2adc` 已在 8082 启动，保留用户加密配置；系统钥匙串提示由用户处理。Chrome 实际读取 `/models` 成功，网关支持 `gpt-6-astra`，旧的大小写 `GPT-6-Astra` 无效；明确保存更正并重启，免费价格依据沿用用户内部额度确认。

第二次真实报价 `01a0fd9f-d335-7701-bb48-834b1e74f199` 为 30 credits / 0 USD。发现已消费报价在取消任务后永久显示 202 的前端恢复缺口，正在单独修复；本次先通过受认证的本地报价 API 创建报价（无供应商调用），之后在实际 Chrome 中核对、勾选资料发送并提交，返回 202 job `01a0fda1-0a8f-77a6-acf2-31f76b0164fb`。Tripo 两图上传与付费提交实际成功，远端任务 `488220ac-ca08-4867-8fba-d1bd326f0622`；说明书 AI 前两批开始处理。此时尚无最终模型/知识/草稿，不记端到端通过。

用户明确允许任务使用 9,000 credits，额度内不再逐次索权。保守将前次未结算预留 30 与本次 30 合计占用 60，剩余保守额度 8,940；此数是执行预算台账，不是供应商账户余额或实际账单。详细证据保存在 `var/prd-completion/chrome-live-wiiu/`。

### Chrome 真实结果：34页解析完成，3D已生成，复核修订准备

第二次任务7个说明书批次全部成功，共232个原始条目、去重后229个（73部件/67步骤/89规格）。初始batch0/1实际HTTP502并进入unknown；通过现有显式替代授权及后续原阶段重试完成，保留unknown的0美元账本，不伪造“供应商未受理”证明。用户已确认内部免费额度。Tripo真实任务成功，返回credits_consumed=30，账本actual=3000 creditMinor且settled；未再次提交付费建模。

实际模型输出位于 tripo-data.rg1.data.tripo3d.com，原默认下载名单只含cdn.tripo3d.ai，已按真实受认证任务返回的精确主机扩充隔离实例名单。同时修复已结算后下载重试错误地要求held预算的问题：仅可能新增计费的ManualExtract/TripoUpload/TripoSubmit保留预算门禁，现有远端查询、下载和本地处理可恢复；未知远端/取消/配置/DAG约束保留。3项control、15项pipeline、11项独立pipeline回归通过，测试证明下载恢复不新增已支付的Tripo提交，账本与知识不变。已消费报价的重新报价入口36项前端目标测试通过，实际Chrome复验仍待后续。

原说明书已逐项校对并生成7份未应用审计：91项需要文字修订，138项可确认，68个外部附件/流程角色/不可见内部部件仅文本。3个重复ID均原样confirmed决定，合并后的229项由第二人16条结构检查核验。按钮图标丢失、电池步骤、安全提示和规格适用对象的错误均记录；物理页与印刷页分别保留，供应商原事实不可变。新增注意事项编辑和覆盖字段合并修复正在最终回归；仅在新包验证部署后按实际UI逐项应用，当前没有事实确认、模型声明、热点或发布动作。

当前普通包启动阻塞在macOS NativeKeychain授权调用，用户需操作系统弹窗；尚无本次真实模型下载/Chrome外观验证/发布/导出结果。代码与审计完成不视为真实流程PASS。新包验证结果及最终服务身份后续追加。

注意事项完整修复的最终常规publishing套件 **9 PASS /0 FAIL /6既有显式门控live-CLI用例未运行**，新字段合同已重新生成。部分PATCH按提供字段合并，安全only保留已有标题动作、title-only保留安全修订、[]明确清空；旧版本与供应商原文不变。前端初始5目标文件37项通过，草稿步骤预览修复后的3相关文件20项通过（重叠不相加）；tsc/目标lint/diff-check通过。审计应用计划16项独立结构核查通过，只有旧编辑器阻塞的关机等待提示按新功能转为待应用修订，其第二条正确Wii Menu例外保留。

最终前端生产build与普通release embedded-ui包构建通过，SHA `a2e7c4a2045f770def0a6a2feaedbd98857a12dfdb0c7549bda37f0fc72ba113`，26,151,024B。首次就地覆盖执行在macOS被杀（版本探针exit137）；相同字节通过新文件原子替换后版本检查exit0，未修改签名/钥匙串ACL或系统安全设置。最终服务session87473/PID57163仍无8082监听且等待NativeKeychain；此前等待进程已正常中止，实际生成任务未重购。后续用同一新包恢复，Chrome驱动v3及229项核查计划已准备但未应用。用户完成当前系统授权前，不能标记真实样本闭环通过。

### Chrome 样本恢复：系统代理 fake-IP 与下载安全保护

用户完成前一包的钥匙串授权后，原任务下载阶段经认证 API 定向重试返回200（只重新排队 `model_download`，未重提Tripo生成）。阶段随后因本机Shadowrocket把精确Tripo下载域名解析到保留段 `198.18.0.28` 而进入 `needs_input`；应用SSRF保护正确拒绝。Tripo上传/生成/轮询及7批LLM仍成功，30credits账本settled，draft仍不存在。

只读网络验证：公共DoH直连返回Tripo下载主机的CloudFront CNAME及四个公网A地址；对其中两地址做保持原主机名/TLS校验的无签名HEAD，证书验证成功且根路径返回预期403。新增窄范围下载器恢复：仅已精确允许的HTTPS主机且系统所有答案均为198.18/15时，使用固定DoH查询，限制超时/大小/CNAME，逐项检查公网地址并固定直连IP；混合或真实私网结果仍拒绝，TLS/SNI与逐跳重定向校验保留。模型客户端明确无代理，避免HTTPS CONNECT绕开IP固定。没有放行fake-IP或修改系统代理设置。

下载器目标单测8/8、已结算下载恢复集成1/1、正式embedded-ui release build通过；新包SHA `b462a3783d5309f7fbc8306a64ea74f1f738235070a178dfd989fe31b4a1e5a3`。当前该包进程PID5829仍等待macOS钥匙串授权，8082未监听；真实签名模型字节尚未下载，不能记端到端通过。

### 预览口令轮换及 Chrome 续验

按用户要求，使用内置 `init --password-file` 分别轮换真实样本和原本地预览两个数据目录的管理员口令；受限临时文件随即删除，既有会话全部撤销，数据和加密 API 配置保留。受认证 HTTP 实测新口令登录 200、旧口令 401；Chrome 154 headed 实际用新口令登录 8082，打开设置和 Wii U 物品页，页面脚本错误 0。口令值不写入仓库文件。网页登录与 macOS 钥匙串是独立机制，口令轮换不改变加密主密钥。

使用已结算任务对 `model_download` 单阶段定向重试一次，HTTP 200，Tripo submit/poll 保持成功且没有重新购买。该阶段再次被本机 Shadowrocket 假 IP 阻挡；诊断发现系统同时返回 `198.18.0.28` 及其 IPv4-translated IPv6 表示，旧 DoH 门槛把后者误判为独立 IPv6。最小修复仅将映射/转换后的 198.18/15 视为同一假地址，其他私网、公网混合答案仍不进入回退；下载模块 8/8 测试、格式与差异检查、release 构建通过。新包 SHA `0a272250d38f5a28bfe47c97a1ef6547da6dd39222e99914ca23436b06fbf9fd`。本轮新包首次启动等待系统钥匙串授权，尚无 8082 监听；不将代码测试写成真实模型下载成功。

### Chrome 真实模型与钥匙串身份（2026-10-03）

用户授权现有 `everything-manual-dns-translated` 后，8082 实际恢复监听；只定向重试原已结算任务的 `model_download`，旧任务随后成功并组装草稿。真实 GLB 5,393,272B、SHA-256 `63910f02e10b1e28f240f4290ddbfb41118500ceb5346abb6dca1335d1150738` 通过头、长度和 v2 校验；Chrome 截图发现两张原照片中的人手被融合进主机模型，故没有复核或发布这份草稿。

本机预览管理员口令现为用户指定值（不在文档重复口令）；新口令经真实 HTTP/Chrome 登录、旧口令被拒。默认钥匙串是当前系统用户的 `login.keychain-db`，应用条目 service=`org.everything-manual.secrets.v1`、account=`master`，只存随机 32 字节 AES-256-GCM 主密钥；网页 API key 密文仍在 gitignored、0600 的 `provider-overrides.json`。当前二进制 `codesign` 为 ad-hoc 且 designated requirement 含 cdhash，本机 `security find-identity -v -p codesigning` 为 0 valid identities，故每次后端新构建可能再次要求授权。已准备只使用现有身份的 `scripts/macos-sign-local.sh` 与运维说明；没有读取/导出主密钥或密钥明文，没有开放条目给所有应用。

为去除人手，在 Chrome 中将两张照片换为 Wikimedia Commons 的无遮挡 Wii U 主机正反照片（各 3700×2500，PD-self；来源、哈希见忽略目录 `var/prd-completion/chrome-live-wiiu/clean-photos/metadata.json`）。新报价 `01a10183-2384-7103-8230-e82ca80dd4a8` 明列两张新照片及 34 页文字，仍为 30 Tripo credits / 0 USD；Chrome 显式勾选资料发送并提交新任务 `01a10185-6cda-77af-8ead-c85764c932cc`，用户已有 9000 credits 授权。Tripo **仅一次新付费提交**，模型下载/校验成功、账本 actual=30 credits settled；新 GLB 4,861,236B、SHA-256 `a8bd9d05095f3bdc40622b5a7eb675d05794eb3d29242a0d1538ede828c34bbf`。Chrome 154 实际 WebGL 渲染正反面无 pageerror，截图 `wiiu-clean-model-standalone.png` / `wiiu-clean-model-back.png`；整机轮廓可用，但接口排列、标识和局部孔位仍有生成失真，不能据此宣称精确接线热点。

LLM 网关先前三批成功，两批在硬编码的 180 秒整体请求超时处落 `submission_unknown`，无法证明上游未处理；按内部免费 0 USD 预算分别走显式替代对账，保留原 unknown 账务，串行恢复后两批成功，未重跑已成功批次或 Tripo。下一批在单独请求时仍 180 秒超时，当前 7 批中 5 批成功、1 批结果未知、1 批等待人工，`manual_merge`/`assemble_draft` 排队，**新任务尚无草稿、未发布、未导出，真实端到端仍未通过**。前次 7 个成功批耗时 117–168 秒，当前成功批耗时 130–159 秒；失败批输入并非最大，超时余量不足是较可能根因，但不宣称网关一定完成或一定未完成请求。

已把 Manual AI 默认整体超时从 180 提高到 600 秒，增加 `EM_MANUAL_AI_REQUEST_TIMEOUT_SECONDS` 的 30..=3600 秒受限覆盖；超时继续保持 unknown、绝不自动重发。定向 5/5 与既有 unknown 不重发集成通过，release 构建通过；新普通包 SHA-256 `c4f8c76c51ed0e860a59acccfb84269009a7a72827d245babfc40b0c3fa9624f` 尚未启动。可同时把 `manual_ai_batches` 从默认 2 降至 1。当前已授权服务继续运行，等待可复用本机签名身份/明确外部主密钥方案以避免再次反复弹窗；不会为消除弹窗降低密钥保护。旧 229 项知识审计固定原 job/实体 ID，不直接应用到新草稿；已准备只读逐项对照器 `compare-drafts.py`，新草稿出现后先核对原文与出处。
