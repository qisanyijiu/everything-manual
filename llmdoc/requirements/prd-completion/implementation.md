# PRD completion · 实现交接

## PC-01 · RD_READY（待独立 QA）

- 日期：2026-10-02；依据：PC-01 冻结的 PRD revision 1 / UI revision 1；范围：REQ-PC-001～005、AC-PC-001～010、UI-PC-001～005。
- 本轮只实现发布资料包下载，不改变后端导出格式、发布语义或后续卡；未修改协调 state、用户配置或预览数据。

### 实现与边界

- 阅读器版本信息下方新增常驻下载区；版本列表每行新增下载，按钮辅助名称含草稿 revision 和发布时间。两处复用 `ReleaseDownload`，实例绑定已读取的 release ID；空态、实体读取失败/加载中没有可用下载入口。
- `api/release-download.ts` 经既有 `requestBytes` 请求 `/api/v1/releases/{id}/export`，沿用 Cookie、401 广播和错误合同。通用字节响应只增加原始响应头供安全消费，既有消费者不变。
- 接收 ZIP MIME、首个 local file record、central directory、EOCD 的一致性，拒绝 JSON/HTML/伪装 MIME/截断包；这属于下载传输检查，完整 manifest/资产/哈希仍由既有后端负责。不会在前端重组包、查询最新草稿替换历史内容或调用供应商。
- 文件名优先使用安全 `filename` / UTF-8 `filename*`；路径、控制/双向字符、设备名、非 ZIP 后缀等整体退回 `release-{safeReleaseId}.zip`，不展示不安全候选。下载状态展示的名字等于实际传给浏览器的建议名。
- 等待同步防重、保留键盘焦点；失败显示分类原因及安全 requestId，未知服务端 message/HTML/内部路径不显示。重试是显式动作，仍请求原 release。401 使用全局重登录和原路径返回，不自动重放。
- 卸载/切换 release 中止请求，丢弃晚响应；Object URL 在浏览器消费后 1 秒释放，卸载也释放。页面只声明“已发起下载”，不宣称磁盘保存成功。
- 新下载/重试控件三档均至少 44px，说明和状态 14px，长名称/诊断 ID 可折行；用途常驻，文字与 `role=status/alert`、`aria-describedby` 配合。

生产文件：

- `apps/web/src/api/client.ts`
- `apps/web/src/api/release-download.ts`
- `apps/web/src/features/manual/ReleaseDownload.tsx`
- `apps/web/src/features/manual/ReleaseListPage.tsx`
- `apps/web/src/features/manual/ReleaseReaderPage.tsx`
- `apps/web/src/theme.css`

### 实际验证（RD 自检，非 QA 签发）

| 命令 / 环境 | 结果与证据 |
| --- | --- |
| 在 `apps/web` 执行 `npm run typecheck`、`npm run lint` | 均通过；TypeScript 无输出错误，ESLint 零 warning |
| `npm run test -- src/api/release-download.test.ts src/features/manual/ReleaseDownload.test.tsx src/api/client.test.ts` | 3 文件 / 31 项通过；包含安全文件名、伪 ZIP、401 广播、防重、焦点、显式重试、卸载/晚响应和 Object URL 清理 |
| `PLAYWRIGHT_NO_COPY_PROMPT=1 CARGO_NET_OFFLINE=true EM_E2E_WEB_PORT=15373 EM_E2E_API_PORT=18380 npm run test:e2e -- --config playwright.release-download.config.ts` | 最终 5 项通过（17.2s）；自管临时后端、localhost fixture、虚构密钥；无 trace/HAR/video，截图仅测试物品。未启停 8080/5173 预览。此配置跳过原 globalSetup 的额外后端构建，测试自身只构建一次 `job-failpoints` |
| 仓库根 `cargo test --offline -p everything-manual --features job-failpoints --test backup_restore export_contains_only_release_assets_and_no_secrets -- --exact` | 1 项通过；既有后端导出资产白名单、其它物品排除、秘密/会话/路径/临时 URL 排除、真实 401/404 与临时文件清理回归 |

浏览器证据目录：`artifacts/prd-completion/pc01-rd/`（当前为未跟踪文件，未命中 Git 忽略规则；本卡没有改全仓忽略规则）。其中 `old-reader.zip`、`old-after-draft-edit.zip`、`new-list.zip` 由真实后端正常响应生成；Python `zipfile` 独立读取 CRC、包内文件集合、顶层 manifest、冻结 `release/manifest.json` 及 PDF/GLB 的 sha256、相对路径/来源、秘密排除，摘要见 `zip-summary.json`。`layout-summary.json` 与 `list-error-{375,1024,1440}.png` / `reader-pending-{375,1024,1440}.png` 对应尺寸与视觉证据，RD 已查看 375px 两入口截图。

重要验收口径：ZIP 顶层导出清单含 `exportedAtMillis`，因此两次导出的 ZIP 整体字节可不同。AC-PC-008 要比较的是 release 身份、冻结 manifest 与资产哈希，实际均不随草稿编辑改变。初次自检错误地比较整个 ZIP，已修正测试而未修改后端格式。浏览器“离开后返回”用例初稿在路由提交前命中旧列表同名控件，已补等待阅读器标题；修正后完整 5 项通过，未弱化晚响应丢弃/返回显式操作断言。

### AC → 取证位置

| AC | 当前实现 / RD 证据 |
| --- | --- |
| AC-PC-001 | 两入口绑定独立 old/new release ID；浏览器第 1 项真实两版本下载，冻结清单哈希不同且身份各自匹配 |
| AC-PC-002 | 页面成功分支才渲染控件；浏览器第 5 项真实空列表 / 不存在 release 无下载；QA 可另延迟实体读取复核 loading |
| AC-PC-003 | 控件 controller ref 同步防重；组件 pending 测试与浏览器第 3 项 Enter/Space 多次仅 1 请求 |
| AC-PC-004 | 安全文案 / 关联 alert / requestId；浏览器第 2 项真实 404， 第 4 项网络/403/500/JSON/HTML/伪 ZIP，均无下载后可重试 |
| AC-PC-005 | 正常文件名与真实浏览器 suggestedFilename 一致；API 单测覆盖 UTF-8、缺失、路径、控制/双向字符、保留名与不正确后缀；只声明发起 |
| AC-PC-006 | 组件取消与晚响应丢弃；浏览器第 3 项等待离开/返回，第 4 项 401 原路径恢复，均需显式重新下载 |
| AC-PC-007 | 两入口常驻完整用途文案；等待/成功 status 与错误 alert 均带文字；截图可检查不依赖 tooltip/颜色 |
| AC-PC-008 | 浏览器第 1 项草稿修改后旧 release/资产哈希不变，fixture 供应商停止后仍可导出，调用计数不变；后端定向测试补资产白名单 |
| AC-PC-009 | 浏览器第 2/3 项 375/1024/1440 无横向溢出，44px 控件、Enter/Space 和焦点保持；组件 ARIA 关联；最终逐项完整可访问性判定留 QA |
| AC-PC-010 | 浏览器真实下载 + Python ZIP/CRC/manifest/资产 sha256 检查 + 既有 Rust 导出回归；PDF 为原创 fixture 真实文件，GLB 为后端 validated fixture；未冒称官方 PDF 已发布/供应商真实生成 |

### QA 交接与限制

- 请求独立 QA 对本卡全部 10 AC 验收，尤其实体 loading、长安全文件名、两入口三档键盘、401 与离开后的资源行为，不以 RD 自检替代签发。
- 正常成功链路没有 synthetic `route.fulfill`；异常响应由明确的故障用例注入，真实后端错误另有独立覆盖。
- 官方 PDF 导入/准备由协调者另行验证，未为本卡再复制第三方 PDF 到 tracked fixture。真实供应商、Linux/其它浏览器、五分钟性能和发行门禁不属于此卡通过范围。
- 前端 ZIP 下载沿用内存字节请求，没有新增后台打包任务或持续缓存；未来超大包资源策略应另评估，不把这次入口交付说成后台流式下载能力。
- 已有构建告警：Node `module.register`、THREE.Clock deprecated、Cargo config 文件名弃用；未导致本卡失败，未扩范围改动。
- llmdoc 本次新增：本文件 PC-01 实现/验证/交接。状态仍由协调者推进；没有代签 QA PASS。

## PC-06 · RD_READY（待独立 QA）

- 日期：2026-10-02；依据：PRD revision 2 §10、UI revision 2 §10.5；范围：REQ-PC6-001～005、AC-PC6-001～010。
- 未读取用户 provider-overrides、真实 model 或密钥，未停启 8080/5173 预览，未自动修正用户配置；全部验证使用临时目录、虚构 canary、固定测试主密钥或随机隔离主密钥和 localhost fixture。PC-01 生产文件/测试保持冻结，本文件仅纠正其证据目录的 Git 状态误述。

### 行为与合同

- 新增 Rust `config/model_guard.rs` 和前端 `settings/model-guard.ts`：完整值匹配 `sk-` 加至少 16 位 ASCII 字母/数字/`_`/`-`，只可剥离一次不区分大小写的 `Bearer` + ASCII 空格。两端显式使用 Unicode White_Space 加 BOM 的首尾空白集合，覆盖 Rust/JS 默认 trim 的 BOM/NEL 差异；不扫描任意子串、不查询模型目录，短 `sk-local` 与常规自定义模型仍合法。
- 两家 active/saved 各自分类，问题 model 返回 `null` + `modelIssue: "suspectedCredential"`，来源与 keyConfigured 保真；普通空值 modelIssue 为 null。`settings/status.providerModelIssues` 提供不含原值的原因；问题 provider 不具备 generation 条件。旧配置加载允许该形态，读取不写文件、不切换 key 模式；CLI check 与 ProviderSettings Debug 安全展示。
- PUT 拒绝新疑似值、未明确处理的 masked null、恢复到问题 deployment；固定字段错误不包含输入。新增 `clearModel: true` 明确清空模型，普通 null/空字符串不是清空旧问题值的授权。填正常模型、明确清空、恢复合法部署三条路径均保留 CAS、busy/pending/配置代次门禁和 key keep/replace/clear 的加密语义。
- 当前配置与冻结配置分别校验：报价/确认/建单、付费重试、authorizeReplacement 和 worker 新提交前均拒绝问题模型。冻结快照及引用 quote 的 providerConfig/sendScope 已知模型位置都检查；不会通过改模型、身份、金额、hash 或确认事实继续旧报价。既有幂等 job 重放仍返回原 job，不产生第二次任务/预留。
- 历史 QuoteDto 的新增 modelIssue 有 serde default，旧 JSON 缺字段可读；公开 GET 只在副本中替换问题模型为固定隐藏文本并标为不可用，持久字节不变。确认重放也先检查冻结模型，不能借已确认状态回显疑似值。
- Tripo 旧任务的查询、取回及 attachRemoteTask 不依赖模型名；问题模型时注册只允许已接受 receipt 恢复的 submit handler 和查询/取回 handler，Fresh submit 仍拒绝。unknown 不自动重试、不释放未决账务；authorizeReplacement 仍受新提交门禁。Manual AI 没有虚构查询接口。
- 设置页保留初始空框非 dirty，立即遮蔽新粘贴疑似输入，不写 input value 属性/URL/storage；显示固定错误、清空确认/撤销、可见恢复错误与聚焦、独立 key 操作。恢复取消保留尚未提交的其它输入和新密钥，成功或明确丢弃才重置。报价/确认/任务失败提供设置与重新报价路径，旧问题报价禁用确认和生成。

主要生产文件：`crates/server/src/config/{model_guard,mod,provider_overrides}.rs`、`http/dto/{settings,generation}.rs`、`http/{settings,estimates,jobs}.rs`、`generation/{estimate,jobs}.rs`、`jobs/{control,executor}.rs`、`providers/mod.rs` 与两家 handlers；前端 `features/settings/{model-guard,ProviderSettingsForm,SettingsPage}`、`features/import/ConfirmStepPage.tsx`、`features/jobs/{JobStageList,status}`；生成合同 `contracts/openapi.json` / `apps/web/src/api/generated.ts`。

### 实际验证（RD 自检，不代签 QA）

| 实际命令 | 最终结果 / 范围 |
| --- | --- |
| `CARGO_NET_OFFLINE=true cargo check -p everything-manual --features job-failpoints` | 通过；后续生产修改由下列测试重新编译 |
| `CARGO_NET_OFFLINE=true cargo xtask contracts`，随后 `CARGO_NET_OFFLINE=true cargo xtask contracts --check` | 生成及一致性检查通过，OpenAPI/TS 与 Rust DTO 一致 |
| `CARGO_NET_OFFLINE=true cargo test -p everything-manual --features job-failpoints --lib config::model_guard::tests::finite_rule_and_legitimate_custom_models -- --exact` | 1/1，通过有限规则、16/15 边界、Bearer/大小写/子串/非ASCII与 BOM/NEL |
| `CARGO_NET_OFFLINE=true cargo test -p everything-manual --features job-failpoints --test model_guard_rd --test generation_requests` | 35/35：model_guard_rd 当时 5 项、generation_requests 30 项；含三种旧 key 模式不改盘、写/恢复拒绝、三条纠正路径、加密重载、五个公开 quote 模型位置、冻结 job retry/worker 拦截，以及正常报价/预算/幂等/旧配置代次回归 |
| `CARGO_NET_OFFLINE=true cargo test -p everything-manual --features job-failpoints --test model_guard_rd pc06_cli_check` | 后追加 CLI 项 1/1：实际 init/check 读取临时 TOML 及环境覆盖，诊断无 canary、文件不变、无隐式网页配置 |
| `CARGO_NET_OFFLINE=true cargo test -p everything-manual --features job-failpoints --test pipeline pc06_` | 1/1，本机假供应商：原 unknown 1 次提交、问题配置拒绝替代购买、attachRemoteTask + accepted 恢复后 2 次查询，提交总数仍为 1 |
| `CARGO_NET_OFFLINE=true cargo test -p everything-manual --features job-failpoints --test api_settings_rd --test api_settings_qa --test encrypted_secrets_rd --test encrypted_secrets_qa` | 32 通过，2 个原有 native Keychain 用例按原声明忽略；无失败。包含 CAS、未决任务/残留、保存/建单锁边界、key动作、密文重启与主密钥故障 |
| 在 `apps/web` 执行 `npm run typecheck`、`npm run lint` | 通过；没有 TypeScript 诊断或 lint warning |
| `npm run test -- src/features/settings src/features/import/ConfirmStepPage.test.tsx src/features/jobs` | 5 文件 / 36 项：设置即时遮蔽/非dirty/明确清空/恢复失败焦点/其它编辑保留，旧报价禁用与任务恢复入口，以及已有行为回归 |
| `PLAYWRIGHT_NO_COPY_PROMPT=1 EM_AS_QA_WEB_PORT=15373 CARGO_NET_OFFLINE=true npm run test:e2e -- --config playwright.model-guard.config.ts` | Chromium 1/1（4.7s），真实隔离后端；375/1440 无横向溢出、清空动作 ≥44px、Enter/Tab/Space/焦点、DOM属性/URL/storage 无 canary、一次正常纠正保存和真实重启后生效。两家供应商请求数均为 0、日志泄漏检查 false；trace/HAR/video/自动失败截图关闭 |
| `git diff --check` | 通过 |

浏览器安全证据：`artifacts/prd-completion/pc06-rd/summary.json`、`correction-375.png`、`correction-1440.png`；RD 已查看两图。图片在初始问题值已由后端遮蔽、输入为空时拍摄，只含虚构环境。证据目录没有声称 Git 忽略。原始后端日志/临时配置由隔离 harness 正常清理，只留下匹配结果与尺寸摘要。

自检过程中修正了测试夹具的 UUID、不可变快照构造和幂等响应预期，未放松生产不可变约束；旧快照夹具只在私有测试 DB 中临时移除并立即还原原 trigger 后植入历史字段，再调用生产读取/控制入口。普通 sandbox 的 localhost 监听曾报 EPERM，已通过工具审批重跑本机 fixture 与浏览器并通过。此前单项 CLI 命令误加 exact 导致 0 项筛选，已按上表重跑实际 1 项；不计作验收通过证据。

### AC 对应与 QA 交接

| AC | 实现 / 当前 RD 证据 |
| --- | --- |
| AC-PC6-001 | 同规则 Rust/TS 单测、两家直接 PUT canary 拒绝、常规模型保存；BOM/NEL 边界已按只读审查修正 |
| AC-PC6-002 | model_guard_rd 原子拒绝/恢复问题部署；无文件/revision/key变化；AS 权限/CAS/422回归 |
| AC-PC6-003 | 旧磁盘 inherit/clear/encrypted replace 两家模型：只读加载后 active/saved 遮蔽、字节保持；无新迁移 |
| AC-PC6-004 | 实际 CLI TOML/env check 安全输出，正常网页覆盖问题部署仍显示合法值；恢复问题部署拒绝、合法部署可恢复 |
| AC-PC6-005 | 未纠正的 masked null 禁止整页保存；填模型/clearModel/restore 三路径；组件验证聚焦、初始非dirty、key独立 |
| AC-PC6-006 | status 模型问题状态、能力false、key事实保留、实际health ready正常；CLI不要求重新登录；原资料路由未增加模型门禁 |
| AC-PC6-007 | generation_requests 当前/冻结报价、确认、建单、retry/worker零新attempt/预留；pipeline unknown替代购买拒绝与旧任务查询不受阻 |
| AC-PC6-008 | 五个公开 QuoteDto 模型位置遮蔽，GET前后quote_json完全相等；旧缺字段可反序列化，金额/身份保持；浏览器及CLI日志无canary |
| AC-PC6-009 | 三种key纠正/加密重载、active坏/saved正常/pending状态、浏览器真实restart；AS/ES 32项及generation_requests配置代次回归 |
| AC-PC6-010 | 375/1440两图/几何JSON、键盘与ARIA组件断言、输入无DOM属性/URL/storage副本；全部使用本机虚构数据 |

交付测试 binary：`/Users/qsyj/Code/rust/everything-manual/target/debug/everything-manual`，包版本 `0.1.0`，debug + `job-failpoints`（含测试用本机下载放行能力，不作为发行/preview产物）；2026-10-02 02:38:37 CST 构建，61,020,864 bytes，SHA-256 `cdf2a8adf31522836efdef9b09fa041bf1a5fe81b90e6023144ced0fddea02a5`。协调者可复制该 binary 与前端源码作独立 QA 冻结；PC-02A 尚未开始。

限制与待 QA：此规则仅覆盖 PRD 的高置信完整形态，不能识别所有供应商秘密；旧错误 model 的原始存储不会被本卡后台清理，只有用户明确纠正后才替换该配置。历史记录只遮蔽公开副本，不清库、不宣称解决既往已泄漏副本。仅上述 Chromium 尺寸场景，本卡未新增真实付费测试，也未把其它浏览器/发行矩阵/native Keychain计为通过。请独立 QA 按全部 10 AC 验证，尤其原件/发布版在坏配置下的读取、HTTP 对账与并发门禁、字段恢复焦点及历史 confirmation/frozen 混合来源；本文件只签 RD_READY，state 由协调者推进。

## PC-02A 原件独立阅读与出处导航（2026-10-02）

角色：RD；依据产品/UI revision 2 §11/§11.3，REQ-PC2-001～003、AC-PC2-001～004、UI-PC2-001～004。当前交接标记：`RD_READY`。仅 A 子卡，不包含 PC-02B 待办处理，不签 QA 结论；PC-01 下载语义和 PC-06 安全规则保持原合同。

### 实现范围

- `ItemOverviewPage` 每份已绑定 PDF 的真实「查看原件 · 文档标题」入口指向 `/items/:itemId/documents/:documentId?page=1`。新增懒加载 `DocumentReaderPage`，从已有受保护资产读取，无 preparation/draft/release/provider 前提；标题、顶部/底部返回物品资料、返回原资料行焦点均接入真实路由，原来源 URL/哈希仍保留于概览。
- `OriginalDocumentPanel` 为独立页与嵌入面板共用：实际 PDF 页数、前后页、带标签的页码输入和 Enter/跳转；非法输入只报字段错误并保留旧有效页。直接非法 URL/越界出处不会 clamp；在明确选择有效页前隐藏原页图。切页/切文档/重试使用渲染身份，避免新页标签短暂配旧页图；旧异步结果不能覆盖当前页，取消 render、destroy PDF、清空 canvas 和页 cleanup 保留。
- `document-navigation` 由阅读器/复核页稳定持有选中 document、page、来源 entity 控件身份。部件及步骤均以 `evidence.documentId` 精确查询当前文档或发布 manifest 的冻结文档；原文显示文档标题及多文档选择框，手动切换到第 1 页。缺 document 显示「此出处的原件不可用」并保留引用身份，越界显示「此出处页码超出原件范围」，不绘制猜测 bbox。
- `PageLayout` 支持显式 `{panelId, focusId, serial}` 导航；1440 右栏原文、1024 单侧栏原文标签、375 单个 Drawer 内替换内容。返回出处恢复来源面板/按钮，来源按钮已消失时退到对应可聚焦面板；Drawer close/returnFocus 使用稳定回调。每个导航 serial 只执行一次焦点定位，普通重开面板/翻页不重放旧焦点；切断点按身份恢复。
- HTTP/PDF 失败仅影响原文区，显示原因/可用 requestId 和「重新加载原文」，重试仍为同一文档/页。3D HTTP/WebGL 失败与 PDF 无耦合。局部 CSS 保证出处/返回/翻页/跳转 ≥44px、正文 14px、页码/辅助 ≥12px；375 工具条换行、页图适配容器。
- 最后检查发现旧 `listDocuments` 默认只读 20 条，会让第 21 条后的原件没有入口且引用误报缺失。因此仅为 PC-02A 三个消费者新增 `reader-documents` 查询，遍历已有 cursor 合同并传递 AbortSignal；`listDocuments` 增加可选 cursor/signal，旧默认调用不变。未新增后端 DTO/API。独立查询 key 保留既有资料前缀，上传失效规则仍有效。
- 测试 harness 的 `TestBackend` 构造器增加可选 binary 路径，原有默认行为不变，PC-02A 使用已冻结副本，不触发 Rust 构建或覆盖 preview binary。

### 已执行验证

所有命令在仓库根执行，以下 npm 命令用 `npm --prefix apps/web` 时等价；浏览器配置自行启动 15373 Vite 和随机端口的临时后端，预览 8080/5173 未停止。

| 命令 / 范围 | RD 结果 |
| --- | --- |
| 在 `apps/web` 执行 `npm run typecheck`、`npm run lint` | 通过，无 TypeScript 诊断或 lint warning |
| `npm test -- src/features/manual src/features/shell/PageLayout.test.tsx src/features/shell/Drawer.test.tsx src/features/viewer/OriginalDocumentPanel.test.tsx src/features/viewer/draft-view.test.ts` | 6 文件 / 30 项通过：保留发布条件/下载状态、draft 读取、Drawer trap/Esc、标签 Home/End/roving；新增一次性显式导航、缺失来源焦点 fallback、非法引用零 getPage、迟到 PDF 释放、旧 render 取消与不覆盖新文字、卸载清理 |
| 最后渲染身份保护后 `npm test -- src/features/viewer/OriginalDocumentPanel.test.tsx` | 3/3 通过；其后 typecheck/lint 再通过 |
| `PLAYWRIGHT_NO_COPY_PROMPT=1 EM_E2E_WEB_PORT=15373 npm run test:e2e -- --config playwright.original-reader.config.ts` | 最终跨分页回归 3/3（26.6s）：22 个真实原件入口/翻页/返回、双阅读器跨文档与三个断点、真实 404 局部重试/WebGL禁用；预览未停，零真实供应商调用 |
| `git diff --check` | 通过 |

浏览器夹具为测试源码生成的两页 `ALPHA` / `BRAVO` 原创 PDF，两个文档同页号有不同页图像素摘要与不同文字；PDF 字节上传、绑定、会话、资产 GET、真实 404、资料查询均走真实隔离 Rust 后端。后端供应商显式未配置，preparations/jobs/provider_attempts/cost_ledger 前后均 0，供应商各类请求均 0。追加 20 份资料元数据，让两个目标文档落到服务端默认 20 条之后，验证概览仍显示 22 个真实入口、直接路由及校准出处均可读取后续分页。

跨文档、缺失引用、越界引用的 draft/release **仅元数据使用显式合成 HTTP fixture**；真实生产当前发布流程只冻结一个 document，本卡不改发布语义。该夹具用于验证读取端能正确消费多文档/异常引用，不能声称通过真实多文档生成/发布链路。PDF、物品、文档列表和读取错误没有伪造成功响应；3D asset 404 和 WebGL getContext 故障只作失败注入。

浏览器证据：`artifacts/prd-completion/pc02a-rd/original-counts.json`、`navigation-summary.json`、`draft-375.png`、`release-375.png`、`draft-1440.png`、`release-1440.png`。RD 已查看上述手机图和桌面图；这些是虚构本地夹具证据，未声称目录被 Git 忽略。没有复制任何官方 PDF 到 tracked artifacts；没有 trace/HAR/video/自动失败截图。临时后端数据/日志/口令正常清理。

自检中先修正浏览器测试未等待页面加载就查 tab 的竞态，再将原件选择框从包裹 label 改为明确 htmlFor，确保可访问名称只有「原件」；视觉复核发现旧 release 样式覆盖出处按钮高度，已补更精确的局部 44px 规则并在全部断点实际测量。失败轮次不计通过；各修正后按上表重新验证。

### AC 映射与独立 QA 注意项

| AC | 实现 / RD 证据 |
| --- | --- |
| AC-PC2-001 | 概览真实文档行入口→独立读取→顶部/底部返回及原行焦点；无 provider/preparation/job 的真实上传绑定 PDF 可读，页数/页图/文字准确；DB 4 类记录零新增。分页覆盖第 21/22 条资料 |
| AC-PC2-002 | 草稿和发布器各自从部件、步骤一次 Enter 进入正确文档第 2 页；375/1024/1440，标题焦点、原文 tab、单 Drawer、返回按钮/关闭恢复来源；切尺寸保持原文/页码/实体。单测另证来源失效 fallback 与旧 serial 不重放 |
| AC-PC2-003 | ALPHA/BRAVO 同页号图像摘要不同且文字吻合；缺文档无 canvas，99 页引用显示明确越界且 canvas 隐藏；0/1.5/3/abc 字段错误留在第 2 页、URL 不改，合法 Enter、前后页与手动换文档正确 |
| AC-PC2-004 | 真实资产 404 显示 requestId，键盘「重新加载原文」回同文档第 2 页；WebGL2/1 均禁用时 PDF 正常。Esc 从原文回出处，再关闭唯一 Drawer；无外部请求/供应商费用，单页资源取消/释放由定向单测验证 |

复用测试 binary：`/Users/qsyj/Code/rust/everything-manual/var/pc06-qa-round2/everything-manual-fixture`，包版本 0.1.0，debug + job-failpoints，SHA-256 `cdf2a8adf31522836efdef9b09fa041bf1a5fe81b90e6023144ced0fddea02a5`。本卡没有 Rust/生成合同变更，没有 Cargo 构建。可用 `EM_PC02A_BINARY` 显式指定 QA 副本；本 binary 不作 preview/发行构建。

限制：本卡 RD 浏览器验证为 Chromium，未计入完整浏览器/操作系统矩阵；官方 BILLY/LACK/TR808 的实际 preview 入口及数字/扫描 PDF 复核由协调者/独立 QA 补验，本卡不读取真实用户配置、不触发说明书 LLM/Tripo。PC-02B 未实施，不借 A 通过宣称待办完成。独立 QA 按四 AC 验证并决定 PASS/FAIL，协调者维护 state。

## PC-03A 从服务端发现并恢复 preparation（2026-10-02）

角色：RD；依据产品/UI revision 2 §12.1/§12.3/§12.4，REQ-PC3-001～003、AC-PC3-001～005、UI-PC3-001～004。交接标记：`RD_READY`。仅 A 子卡；没有实现 PC-03B 建单恢复/摘要或 PC-02B/PC-04，不签 QA PASS，不改协调 state。生产写入在本交接后停止，等待协调者冻结/分配下一卡。

### 实现与兼容边界

- 新增受保护的 `GET /api/v1/documents/{id}/preparations`：严格解析 `limit/cursor`，默认 20、最大 100；cursor 绑定 document，顺序为 updatedAt / ID 降序。响应包含 bounded `data`、`nextCursor`、全局 `recommendedPreparationId` 与该推荐完整摘要（即便不在当前页），摘要含 record/document/source sha、状态、已完成有效页数/页号、已知总数、时间、兼容原因。认证/404/422沿用既有合同。
- 仓储每批至多 100 records，另一次联表 SQL 取得这些 records 的每条最多 101 页资产元数据（第 101 行用于识别超过 v1 页数上限）；在一个只读事务中逐批扫描全部候选，只保留一页响应与当前最佳推荐。没有逐条 record SQL 查询、全量结果堆入内存或读取 PDF/大 blob 内容。推荐为兼容完整 ready 优先；其次兼容 preparing 有效页最多；同等取 updatedAt 再 ID 降序。ready 之间不按总页数偏向更长说明书。
- 新迁移 `0008_preparation_format.sql` 仅添加 nullable `format_version` 与列表索引。新建声明 1，旧记录保持 NULL；旧有页记录仅当当前实际页号/viewport/资产均符合 v1 才可推断兼容。旧空 NULL 记录不可核实，不按日期猜格式。兼容核验包括同 document+sha、1..100 页/有效 viewport、资产同 item/正确 purpose、规范 MIME（JPEG、UTF-8 text）、blob stored 与物理普通文件存在/大小匹配。扫描页允许无 text。缺文件、坏格式、原件不符、ready 不完整均有固定原因，不能作为可用 ready。
- **健康检查边界**：发现只读取文件 metadata，不重新解码或重算全部 blob sha；不能据此声称发现同长度内容篡改，也不证明 clientDerived 页图确实由所指 PDF 生成。原件仍可读取复核。GET 不迁移记录、不补页、不封存、不创建 quote/job/attempt/cost；测试比较 business rows/revision/更新时间/页数据与计数。
- POST body 增加 optional `createNew`；旧调用默认保持「复用 preparing 或创建」合同。只有用户在不兼容情形点「重新准备」才传 true，新建一条格式明确的记录，保留旧记录（含旧 ready），没有后台清空或自动修复。
- 现有准备详情追加同源 `readiness`。报价及新建 job 在读取准备输入时复用兼容门禁，坏 ready 返回 `preparationIncompatible` + `compatibilityReason`，不创建新 quote/job/费用；历史报价 GET 和既有幂等 job replay 读取语义不改。生成 OpenAPI/TS 同步更新，没有改费用、导出或发布语义。
- 前端 `preparation-discovery.tsx` 为准备/确认共用：查询 key 含 item/doc/sha，GET 使用取消信号；默认采用服务端推荐，其他记录为原生 radio、details 和 cursor 加载更多。显式 record ID 也先 GET 验证归属/兼容；sessionStorage 只作可验证提示，错误指针说明「之前的记录不适用于当前原件」，不作为完成事实。切 document 立即换作用域，旧响应不回填；切 record 清理上一轮局部进度/错误。两页均读取完整 document 分页。
- 准备页显式 continue 前先 GET 验证状态，已 ready 即短路，无 PDF fetch/create/PUT/complete。preparing 未知总数显示「已完成 x 页，总页数待读取原件」；用户继续才解析实际 PDF 总数，按有效已有页集合只处理缺页，失败页仍有单页重试。全部页齐仍需点「封存资料」。创建前保留加密/超 100 页 PDF 拒绝行为。
- 停止设置 stopping 并 abort 渲染/上传，但已发出的 PUT 可以完成；等待本轮落定后 GET 回读已保存页，显示保留说明，再允许继续或换文档。运行中禁用原件/record 选择并注册 beforeunload。412/428/ready 冲突停止流水线，显示「记录已更新，请重新读取进度」，只读核对后才恢复动作；不静默覆盖/重建。
- 确认页即使没有本地 pointer 也发现兼容 ready，进入既有预算流程；只有 preparing 时链接到明确 doc/preparation 的继续入口，不制作 PDF。读取失败与空记录/不兼容分别显示；更换选中准备后旧报价和确认失效，晚返回确认不改变新选择，生成继续要求 quote.preparationId 与当前验证 ID 相同。
- 局部样式让 375px 卡片单列、状态/ID换行、原生选择键盘可达、继续/停止/重读/下一步 ≥44px；未知总数不画百分比。没有新增图片素材或复制官方 PDF。

### 已执行验证（RD）

Cargo 均复用现有 `target`，设置 `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0`；前端命令在 `apps/web`。测试数据仅本机临时目录和原创仓库 fixture，未读取真实 provider settings/model/key，未接触 8080/5173 preview。

| 实际命令 / 范围 | 结果 |
| --- | --- |
| `cargo check -p everything-manual --features job-failpoints`（同上环境变量） | 通过 |
| `cargo xtask contracts`，随后 `cargo xtask contracts --check` | OpenAPI 与 generated.ts 一致 |
| `cargo test -p everything-manual --features job-failpoints --test preparations --test generation_requests` | 11 preparation + 30 generation 通过；含 >100 条跨 SQL batch 的推荐/分页、安全/GET 零业务写、旧 NULL/新版/物理缺资产、排序、既有 PUT/CAS/封存/费用回归、PC06 AS/ES及历史模型回归 |
| `cargo test -p everything-manual --features job-failpoints --test generation_requests pc03a` | 新增坏 ready 在报价后变化时拒绝新 quote/job，保留历史 GET、零新增费用/attempt，1/1 通过；以上独立计 42 项 |
| `cargo test -p everything-manual --features job-failpoints --test storage --test config_cli` | 15 storage + 12 config_cli 通过；schema8内嵌/初始化/幂等/旧库升级/失败回滚/未来schema拒绝，CLI配置及秘密排除/启动无外呼等 |
| `cargo build -p everything-manual --features job-failpoints` | 测试 binary 构建通过；无 embedded-ui，不作生产预览产物 |
| `node node_modules/typescript/bin/tsc --noEmit` | 最终通过，全项目无类型诊断 |
| `node node_modules/eslint/bin/eslint.js src/features/import tests/e2e/preparation-discovery.spec.ts tests/e2e/pdf-preparation.spec.ts tests/e2e/global-setup.ts playwright.preparation-discovery.config.ts playwright.preparation-regression.config.ts` | 最终通过 |
| `node node_modules/vitest/vitest.mjs run src/features/import/ConfirmStepPage.test.tsx src/features/import/pdf/prepare.test.ts src/features/import/pdf/errors.test.ts src/features/import/messages.test.ts` | 4 文件 / 24 项通过，含现有报价/确认时序和 PC06 模型门禁；最终相同范围再通过 |
| `PLAYWRIGHT_NO_COPY_PROMPT=1 EM_E2E_WEB_PORT=15513 node node_modules/@playwright/test/cli.js test --config=playwright.preparation-discovery.config.ts --workers=1` | 首 3 场 3/3；追加的 `--grep 'changing document\|T09 regression'` 两场 2/2；最后补 radio 手选并单跑 `--grep 'changing document'` 1/1。合计 5 个不同场景全部通过，未把重复轮次计为新增场景 |
| `PLAYWRIGHT_NO_COPY_PROMPT=1 EM_E2E_SERVER_BINARY=/Users/qsyj/Code/rust/everything-manual/target/debug/everything-manual EM_E2E_WEB_PORT=15513 EM_E2E_API_PORT=18513 EM_E2E_WORK_DIR=/private/tmp/em-pc03a-rd-t09-20261002 node node_modules/@playwright/test/cli.js test --config=playwright.preparation-regression.config.ts --workers=1 --grep 'PDF 准备与续传'` | 原 T09 9/9：数字/扫描/旋转/CJK、中文本地 CMap、加密/101页拒绝、失败续传、停止/离开/单canvas；29.2s |
| `git diff --check` | 通过 |

T09 原断言按本卡明确文案和停止语义更新（ready 不再显示数据库枚举，按钮封存后隐藏；准备 GET 发现不再当作创建请求）；原逐页真实资产/MIME/文字/viewport/PUT序列/未知总数/拒绝零写/单canvas/离开确认检查保留。global-setup 仅增加 optional `EM_E2E_SERVER_BINARY`，显式指定时复用隔离候选，其余默认自动构建行为保持。

首次自检修正了从 SQL 原始资产行读取 purpose/MIME 时未使用实际存储规范的问题（camelCase 线上值不同于 snake_case SQL 值；text MIME 带 charset）。修正后上述准备与报价回归通过。浏览器和 CLI 初次启动在普通沙箱因禁止本机监听 EPERM 未能运行；按授权使用本机隔离监听重跑后通过，不将环境失败算产品成功。无自动失败截图、trace、HAR、video；全部禁止真实供应商调用。

### AC 映射与证据

| AC | 实现 / RD 证据 |
| --- | --- |
| AC-PC3-001 | 104 记录、HTTP limit1、全局 ready 在后续 SQL batch 仍推荐；有效页/updated/id 排序；全新 browser context无Storage发现 legacy partial，进入时零 preparation/page/asset/quote/job/attempt/cost变化 |
| AC-PC3-002 | 3 页原 PDF 预存第1页，新context只 PUT2/3，无新建prep；未知总数先1页后1/3→3/3，显式封存；再新context确认 ready，零 PDF内容请求/准备写入；原T09断线只补第2页继续通过 |
| AC-PC3-003 | legacy NULL有事实可确认、NULL空无版本不可核实、v2/坏viewport/物理文件丢失不可用；明确重新准备生成新record保留旧原文/revision；两document不同PDF/SHA、失效hint、迟到旧发现不覆盖新文档；可单选其他兼容记录；报价/建单端仍拒坏ready |
| AC-PC3-004 | held真实PUT2→点停止→仍stopping不能开新run→落定GET保存1/2→继续只PUT3；complete实际与另一HTTP客户端竞争先封存→当前422ready冲突→仅重读转ready无再次PUT/complete；原T09同页幂等/CAS及停止上传回归通过，零job/cost/provider调用 |
| AC-PC3-005 | Rust匿名401/不存在404/非法limit及cursor422/跨doc cursor422/无CSRF403；前端发现500无开始/假空状态，按钮重读恢复；375px键盘Enter继续、真实radio选择、API合同生成一致；原读取/auth归属门禁保持 |

留存证据：`artifacts/prd-completion/pc03a-rd/prepared-375.png`（已视觉查看）、`stop-conflict-counts.json`，另 T09 原capture沿用 `artifacts/web-mvp/t09-rd/` 的原创 fixture 截图。仅虚构数据与本机计数，未声称 artifacts 被 Git 忽略。临时服务正常停止，测试正常清理临时数据。未读取或修改真实说明书样本目录/provider覆盖文件。

### 冻结交接

以下均来自仓库根 `target/debug/`，package 0.1.0、schema8、debug + `job-failpoints`；没有 embedded-ui，不能替换普通 preview/发行 binary。测试 binary 61,238,272 bytes，构建时间 2026-10-02 03:44:15 CST。最后 Cargo 已结束，协调者可复制 binary、测试可执行文件和当前前端源码冻结，之后另卡写入不影响本轮独立 QA。

| 路径（相对仓库根） | SHA-256 |
| --- | --- |
| `target/debug/everything-manual` | `d7ec4440e17ba35cc33eb6b6e6f52325d304e63b4daea3df20e529edce9984f7` |
| `target/debug/deps/preparations-8a0e48540c011b3c` | `56382ae07bb527646191fcf40d2bb3abbdd545af86a83b05493b758ae4e542db` |
| `target/debug/deps/generation_requests-1f807fc19ff42b49` | `f204117a90551a3dc77ae14a37a6a59d16452b41dde5bf204df6e95531fbe792` |
| `target/debug/deps/storage-795e2efae5f23f93` | `9a8a9b4089a4a8019cfb15ab6067e48abc511077be537206ffe1fc4052306024` |
| `target/debug/deps/config_cli-e60866c02b53e42f` | `7597ca56c5b358baae7da13ae52694398cffc4e5979e2c996203adc7f07044c1` |

独立 QA 仍需按全部 5 AC 作自身验收，尤其其独立全逻辑DB摘要、错误purpose/跨item/原资产删除、分页边界与两个浏览器客户端并发。RD竞态证据为一个浏览器加一个独立HTTP客户端先封存，未声称两个实际UI标签页都完成操作。协调者计划的官方 TILLREDA旧partial32页、旧LACK/TR808/WiiU ready迁移复用，以及非Chromium/发行矩阵，均不记作本卡 RD 已通过；原件健康检查边界见上文。PC-03B/PC-02B 尚未实施，由协调者另行分配。

### PC-03A A2：BUG-PC3-001 键盘焦点修复（2026-10-02）

交接标记：`RD_READY`，仅修独立 QA round4 的 P2 缺陷 BUG-PC3-001（AC-PC3-005 / UI-PC3-001、004），不代签复验 PASS。PC-02B 保持暂停，其未接入的新文件 `review-tasks.ts` 不属于本次补丁或 A2 冻结。

原因：单选记录触发 detail GET 后，共享 `loading` 为 true，整个已成功发现的列表被条件卸载，原生 radio 节点、焦点和 details 展开状态一起丢失。修复只让发现列表取决于列表查询自身的 pending/error；选中详情读取与失败状态仍单独展示，并保留准备/确认页已有的安全动作门禁。未新增自动选择、准备写入或报价动作，没有后端/API/合同变更。

精确补丁文件：

- `apps/web/src/features/import/preparation-discovery.tsx`：保留发现列表/radio DOM。
- `apps/web/tests/e2e/preparation-discovery.spec.ts`：新增真实原生 ArrowDown/ArrowUp 回归；仅增加 optional `EM_E2E_SERVER_BINARY` 以复用冻结测试 binary。
- `llmdoc/requirements/prd-completion/implementation.md`：本交接段。

已执行命令（前端目录 `apps/web`）：

| 命令 | 实际结果 |
| --- | --- |
| `node node_modules/typescript/bin/tsc --noEmit` | 通过，无诊断 |
| `node node_modules/eslint/bin/eslint.js src/features/import/preparation-discovery.tsx tests/e2e/preparation-discovery.spec.ts` | 通过 |
| `PLAYWRIGHT_NO_COPY_PROMPT=1 EM_E2E_SERVER_BINARY=/Users/qsyj/Code/rust/everything-manual/var/pc03a-qa-round4/everything-manual-fixture EM_E2E_WEB_PORT=15513 node node_modules/@playwright/test/cli.js test --config=playwright.preparation-discovery.config.ts --workers=1 --grep 'BUG-PC3-001\|discovery failure\|changing document'` | Chromium 3/3 通过，9.4s；375px ready 推荐与空 preparing 的原生方向键切换，held detail GET 期间原节点 isConnected、焦点与展开状态保留，准备/封存按钮隐藏，响应后 ArrowUp 回推荐；切换无业务写入或供应商调用。另回归读取失败重试/不兼容显式重建、跨文档迟到响应隔离 |
| `node node_modules/vitest/vitest.mjs run src/features/import/ConfirmStepPage.test.tsx` | 1 文件 / 8 项通过，确认/报价时序及模型门禁保持 |
| `git diff --check` | 通过 |

复用 round4 冻结 binary（SHA-256 `d7ec4440e17ba35cc33eb6b6e6f52325d304e63b4daea3df20e529edce9984f7`），没有 Cargo 构建或修改 target，也未触碰真实预览/用户数据。测试使用临时数据和本机 fixture，无 trace/HAR/video/自动截图。原轮次的 `.check()` 测试只验证直接选择，并未覆盖原生方向键焦点连续性；本回归补齐该遗漏。独立 QA 应从 A1 仅覆盖上述前端补丁后复核，本段不替代其结果。

## PC-02B 发布前待办与复核恢复（2026-10-02）

角色：RD；依据产品/UI revision 2 §11.2 / §11.4，REQ-PC2-004～005、AC-PC2-005～008、UI-PC2-005～008；PC-02A 独立 QA round3、PC-03A A2 独立 QA round5 已通过。交接标记：`RD_READY`，不代签 QA PASS，不改协调 state。交接后停止生产写入，等待协调者冻结。本卡不实施 PC-03B/PC-04，不改变后端发布不变量、收费或供应商合同。

### 实现与精确文件

生产文件（均相对 `apps/web/`）：

- `src/features/manual/review-tasks.ts`：从已读取的草稿事实生成稳定任务身份；分为文字事实、模型核对、热点与视角。完成来自实体确认/人工修订、当前模型两项独立声明、有效热点/明确仅文本及真实保存视角。可选视角与额外 stale 记录不会成为新的发布硬门禁。已知发布问题必须同时匹配允许的 code、实体类型与当前实体 ID，未知项不制造导航。
- `src/features/manual/PublishPanel.tsx`：分组未完成计数、默认「只看未完成」、实体名/问题说明/可访问「去处理」，关闭筛选显示真实已完成状态。保留原发布条件摘要；422 原文与 requestId 持续可见，已知问题走相同定位，未知问题仅给重新读取；412 给核对最新版本。实际重读成功前不放行重提；422 问题即使重读成功仍保留原消息/诊断，明确服务端问题尚未确认解决，用户只能显式「重新检查并发布（通过后生成不可变版本）」请求最终校验，成功才清除。412 版本冲突在实际重读成功后清理；发布仍需用户独立点击。
- `src/features/manual/CalibrationWorkspace.tsx`：由稳定父层保存当前任务、实体选择、编辑 buffers 和已处理状态；每个 item/draft 单独作用域。待办定位只选择/导航，不自动进入绑定或写入。处理完成仍留原区域，GET 回读成功才显示「此项已处理」与显式「下一项」；下一项依稳定列表选择仍未完成项，末项可返回发布区。外部完成/删除或模型身份变化不随机跳转，提示重新核对。刷新入口统一为一个流程，有本地修订时一次确认说明丢弃本草稿所有未保存文字修订；取消不丢，GET 失败仍保留，成功才清理。已有数据的 GET 失败不替换整页。几何操作显式开始/取消，显示当前部件；小屏聚焦 ≥768px 说明。
- `src/features/manual/KnowledgeReviewPanel.tsx`：表单值与正在编辑实体受控，换面板/抽屉/尺寸不丢失；保存必须等待 PATCH 与 GET 都成功后关闭。处理中锁住重复保存、输入及取消，失败保留编辑。模型声明以当前 revision+sha 匹配，已打开与核对一致两个动作保持独立。
- `src/features/manual/useDraftMutations.ts`：全部写操作返回可等待结果，单次锁覆盖 PATCH+GET；成功 GET 后才更新缓存与计数。PATCH 已成功但回读失败时只报告结果待核对并禁止盲目重提；412 不自动覆盖。浏览器已加载模型资格绑定当前模型身份，旧资格不能替新模型声明。
- `src/features/viewer/document-navigation.tsx`：开放共用 `navigate`，待办、原文、返回发布区使用同一个递增 serial；可携带窄屏焦点目标。
- `src/features/shell/PageLayout.tsx`：复用 A 的受控面板/焦点；`main` 导航关闭侧栏/抽屉并聚焦发布标题，窄屏几何说明与宽屏动作共用稳定实体导航。目标禁用时回实体/面板可聚焦容器，断点变化按同一实体重新聚焦。
- `src/styles.css`：局部任务组/状态布局、断行和 ≥44px 主要动作。

测试文件：新增 `src/features/manual/review-tasks.test.ts`、`src/features/manual/useDraftMutations.test.tsx`、`tests/e2e/review-tasks.spec.ts`、`playwright.review-tasks.config.ts`。既有 `tests/e2e/manual-review.spec.ts` 仅增加 optional `EM_E2E_SERVER_BINARY` 以跳过构建复用冻结 binary，并把冲突刷新按钮选择器更新为「核对最新版本」；原真实链路/旋转拾取/落库/发布后不可变断言未弱化。

工作树其它既存卡文件不归本卡改动。没有 Rust、迁移、生成 TS/OpenAPI、PDF 引擎或供应商配置修改，没有 Cargo 构建。PC03A A2 源码及 frozen snapshot 未改。

### 实际验证

前端命令在 `apps/web`。仅本机临时 data-dir 与仓库原创 PDF/GLB/图片 fixtures，通过本机 fake provider 构建真实草稿；未读取或修改用户 settings、8080/5173 preview、官方样本。所有浏览器配置 trace/HAR/video/自动失败截图关闭。

| 实际命令 / 范围 | 结果 |
| --- | --- |
| `node node_modules/typescript/bin/tsc --noEmit` | 最终通过，全项目无诊断 |
| `node node_modules/eslint/bin/eslint.js src/features/manual src/features/shell/PageLayout.tsx src/features/viewer/document-navigation.tsx tests/e2e/review-tasks.spec.ts tests/e2e/manual-review.spec.ts playwright.review-tasks.config.ts` | 最终通过 |
| `node node_modules/vitest/vitest.mjs run src/features/manual/review-tasks.test.ts src/features/manual/useDraftMutations.test.tsx src/features/manual/review-state.test.ts src/features/shell/PageLayout.test.tsx` | 4 文件 / 20 项通过；任务排序/完成事实/可选项/未知422映射、stale目标、当前模型身份、PATCH+GET锁/回读失败防盲重提及既有热点发布/面板焦点回归 |
| `PLAYWRIGHT_NO_COPY_PROMPT=1 EM_E2E_SERVER_BINARY=/Users/qsyj/Code/rust/everything-manual/var/pc03a-qa-round4/everything-manual-fixture EM_E2E_WEB_PORT=15523 node node_modules/@playwright/test/cli.js test --config=playwright.review-tasks.config.ts --workers=1 --grep 'PC2B-'` | 首轮3个新场景 3/3，通过13.3s |
| 同上浏览器命令，`--grep 'PC2B-\|T19-1 \|T19-4 \|T19-6 '` | 追加发布412、stale场景和旧回归后，8/8通过39.0s：5个不同新场景＋3个旧T19场景 |
| 同上浏览器命令，`--grep 'real counts\|real stale\|model declarations'` | 最后窄屏/宽屏焦点适配与独立模型双声明补验，3/3通过14.4s；只针对最后变更重验。累计6个不同新场景＋3个旧回归通过，不把重复轮次算新增 |
| 同上浏览器命令，`--grep 'known and unknown'` | 收紧422恢复后1/1通过5.6s：成功GET仍保留未知规则/诊断ID及尚待核对说明；仅明确点击「重新检查并发布（通过后生成不可变版本）」才发POST，真实成功后清错误。随后tsc/scoped lint再次通过 |
| `git diff --check` | 通过 |

浏览器真实数据流：seedJob 经真实 Rust API、临时数据库、fake provider 产出 validated GLB 和知识；review/publish 的成功 PATCH/GET/POST 都来自真实后端。仅 negative 422 和 GET500 使用明确错误注入。保存412与发布412均由独立 API 会话先真实更新 revision 触发；没有伪造成功草稿或发布结果。审核导航/刷新/发布阶段 provider 计数相对造数后基线零新增，未把必要 fixture 造数称为零 provider 调用。

### AC 映射与限制

| AC | RD 证据 |
| --- | --- |
| AC-PC2-005 | 375px真实待办三分组/默认筛选/正确计数；事实项 Enter 聚焦具体实体控件，跨375→1024→1440保留输入；完成后关闭筛选可见「已保存人工修订」。窄屏缺热点定位文字说明，控件禁用且revision不增；1024模型项聚焦正确独立声明 |
| AC-PC2-006 | 保存人工修订后 GET 才减数/移除未完成行，并在当前处理区显示完成；显式下一项按当前真实列表定位，下一项自身不写revision。单测 held GET 期间缓存不提前更新、重复动作只1次PATCH。模型两声明分别写入并回读，第一次的服务器userConfirmed仍false。返回发布标题有焦点；满足条件仍无自动发布 |
| AC-PC2-007 | known422有且只有真实目标按钮，unknown422消息/诊断ID保留并阻止盲目发布；显式重读后未知问题及requestId仍保留，明确服务端问题未确认解决，用户可显式重新检查并发布，最终真实服务端成功才清除。真实PATCH412保留修订，取消刷新不丢；同一次确认后GET500仍保留、再成功重读才清buffer。真实发布412同样保留编辑，取消/确认均独立验证 |
| AC-PC2-008 | 真实API创建stale状态记录，小屏只能说明，宽屏导航不自动拾取；显式重新绑定/取消/再次绑定，当前部件可见，真实点击后同一hotspot ID变confirmed。T19-1旋转/缩放后命中、拖动不建点、坐标逐轴/回投影/落库全部通过；T19-4真实412恢复、T19-6真实发布→阅读器→改草稿后旧版本不变通过。键盘覆盖任务/出处往返/下一项/发布，保留PC02A受控原文导航 |

stale 浏览器夹具通过公共 PATCH 创建明确 stale 状态，anchor 合法且属于当前 fixture 模型；用于验证 stale 无论锚点匹配与否都不可作为有效热点、重新绑定保留 ID。旧模型 revision/hash 不匹配的状态归一化、目标映射及当前模型声明门禁另由定向单测覆盖，未声称本卡浏览器跑了一次真实旧模型再生成流程。独立 QA 可另测旧模型继承。

留存人工截图 `artifacts/prd-completion/pc02b-rd/tasks-375.png`、`editing-1440.png`，已查看手机任务组和桌面修订布局；桌面图包含模块重新挂载时的加载状态，不作为 GLB/PDF 完成证据。旧T19截图沿原 `artifacts/web-mvp/t19-rd/`。仅虚构数据，未声称这些 artifacts 已被 Git 忽略；无真实密钥截图/日志和官方 PDF 复制。浏览器范围为 Chromium，本卡未扩展完整平台矩阵，交独立 QA 验收。

复用 binary：`var/pc03a-qa-round4/everything-manual-fixture`，package0.1.0、schema8、debug+job-failpoints，无embedded-ui；SHA-256 `d7ec4440e17ba35cc33eb6b6e6f52325d304e63b4daea3df20e529edce9984f7`，完全未改。全部自管测试服务已正常结束/清理，无本卡 Cargo 或 target 清理任务。协调者可冻结上述生产/测试文件并交 round6 独立 QA；后续卡须另行授权。

## PC-03B 真实流程摘要与建单恢复（2026-10-02）

角色 RD；产品/UI revision 2 §12.2 / §12.3 B / §12.5，REQ-PC3-004～006、AC-PC3-006～009、UI-PC3-005～008；依赖 PC03A A2 independent QA round5 PASS，PC02B independent QA round6 PASS。本段为 `RD_READY` 开发交接，不代签独立 QA，不修改协调 state。停止生产写入，等待协调者冻结；不进入 PC04/PC05。

### 实现与精确范围

服务端新增 `GET /api/v1/items/summaries?ids=<逗号分隔 UUID>`。会话保护，原始输入 1～100 个 ID，再去重；未知/重复参数、非法 UUID、超过上限 422。混合不存在的 ID 整批 404，不能把遗漏行猜成空状态。只查询单个物品时可给 `documentId`；文档不存在或跨物品同为 404。返回按 item ID 稳定排列的类型化动作和目标实体 ID，客户端映射站内路径，不下发任意 URL。默认 document 取 updatedAt+ID 最新，显式 URL documentId 在五步条、准备与确认页一致保留。

推荐优先级为 needs_input/submission_unknown/failed 任务 → queued/running/waiting_provider/retry_wait 任务 → 尚未发布当前 revision 的草稿 → 最新发布版 → 文档/正面侧面/准备缺项或预算确认。同类按 updatedAt DESC、ID DESC；不可变发布和报价以 createdAt 作为其唯一更新时间。旧发布版独立作为次入口；归档不替代处理状态。

仓储基础读取固定四个批量 SQL（每物品最多一个目标文档/报价、最多四张多视图照片），不读完整 draft knowledge、release manifest、job stage 详情。所选文档准备记录按每批最多 100 条扫描，每批准备/页资产事实两条 SQL；复用 A 的 document+sha+v1/资产 metadata 兼容判据，推荐在全部候选中选择，不由当前页猜选。内存有界，但历史候选多时扫描时间随候选数量增长；仅文件 metadata 检查，不读取完整 blob。摘要处于只读事务，不落库缓存或修复旧数据。

五步状态为 complete/missing/needsReview；当前步骤 aria-current 与完成状态分开。准备基于所选文档存在的兼容完整 ready；报价基于当前输入指纹/视图集合、有效期、配置代次、价格版本与明确确认/消费。新旧 PDF、照片变更使相关状态重查，不改既有 frozen job 或 release。`quoteExpiresAt` 让已打开步骤条在到期边界转为重查，不每秒请求 API。读取失败显示未知，不打完成勾。输入写入、封存、报价/确认/建单成功均失效相关摘要；进入列表/向导重读服务端事实。

报价 GET 仅为公开副本增加可选 `inputIssue`：inputChanged/preparationIncompatible/providerConfigChanged/providerUnavailable/priceVersionChanged。缺字段的旧冻结 QuoteDto 仍可反序列化，`quote_json` 不回写；**已有 consumedJobId 先于当前输入/配置/过期门禁恢复**。模型信息继续沿用 PC06 遮蔽。未改变确认、费用计算、建单事务、同报价单任务、幂等重放或 unknown 预留的服务端最终校验。

确认页先发现/回读已保存报价，已有任务恢复「任务已受理」与详情链接，不自动确认、重报或建单。提交前可在同源 localStorage 保存非秘密 quote ID、一次操作 key 和原请求体（资料 ID/授权整数金额），只作提示；只正常化受限ASCII标识、2～4个不同photo ID及非负安全整数预算；损坏/旧形状提示忽略，不能让存储对象直接进入渲染或重放。无存储也能由服务端最新报价发现恢复。未知结果/GET 失败只允许核对，主生成和重报价锁定；GET 明确未消费且持有在途操作时，只能用户显式重试同 key、同 body 的原授权，不旋转业务键。已消费清提示；明确业务拒绝且 GET 成功未消费后才释放恢复锁。401 沿原认证恢复，原提示保留；412 核对后保留同报价已输入授权（金额规范化显示），不自动第二次提交。跨标签 storage/focus 通知发现原操作；显式 quote URL 变化触发读取，不能绕过未核对操作。StrictMode effect replay 和晚返回输入签名/卸载均有门禁。

生产文件（相对仓库根；其它既存卡文件不归本卡）：

- 新增 `crates/server/src/item_summaries.rs`、`crates/server/src/storage/repo/item_summaries.rs`。
- 修改 `crates/server/src/storage/repo/preparations/discovery.rs`（复用有界多文档批量）、`storage/repo/{items,documents,photos,quotes}.rs`（既有 row decoder crate 可见）、`storage/repo/mod.rs`、`lib.rs`。
- 修改 `crates/server/src/http/{items,estimates,openapi}.rs`、`http/dto/{items,generation}.rs`、`generation/estimate.rs`。
- 生成 `contracts/openapi.json`、`apps/web/src/api/generated.ts`；前端没有手抄服务端 DTO。
- 修改 `apps/web/src/api/endpoints.ts`；新增 `src/features/library/workflow.tsx`；修改同目录 `LibraryPage.tsx`、`ItemOverviewPage.tsx`、`items.ts`。
- 新增 `apps/web/src/features/import/quote-recovery.ts`；修改同目录 `ConfirmStepPage.tsx`、`WizardSteps.tsx`、`PreparePage.tsx`、`DocumentStepPage.tsx`、`ViewsStepPage.tsx`；局部 `src/styles.css`。

测试：修改 `crates/server/tests/generation_requests.rs`（新增三组 API 场景）和 `apps/web/src/features/import/ConfirmStepPage.test.tsx`（原八项继续验证，新增消费/未知/412场景）；新增 `src/features/import/quote-recovery.test.tsx`、`src/features/library/workflow.test.ts`、`tests/e2e/workflow-recovery.spec.ts`、`playwright.workflow.config.ts`。没有迁移，schema 仍为 8；没有修改费用/执行引擎、用户 provider 配置、官方 PDF 或预览数据。

### 实际命令与验证

Rust 根目录；前端命令目录 `apps/web`，node 实际路径 `/opt/homebrew/bin/node`。全部临时 SQLite、原创小型 PDF/图片及 localhost fake provider。浏览器 Vite 15533，与用户 5173/8080 分离；trace/HAR/video/自动失败截图关闭。无真实外网或费用。

| 命令 | 实际结果 |
| --- | --- |
| `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo check -p everything-manual --features job-failpoints` | 初次批量服务编译通过 |
| `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo xtask contracts` 与最终 `cargo xtask contracts --check` | DTO→OpenAPI→TS 生成及逐字节检查通过 |
| `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo test -p everything-manual --features job-failpoints --test generation_requests pc03b -- --test-threads=1` | 三组新增真实 API 用例 3/3：100物品边界/去重/鉴权/404；103条 preparing 跨 SQL batch 不遮住旧 ready，选定旧 PDF 与默认新 PDF 状态不同，视图变更、GET不改quote/audit；任务优先级/同时间ID tie/旧发布次入口及 expired+unknown 消费恢复 |
| `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo test -p everything-manual --features job-failpoints --test generation_requests --test preparations --test items -- --test-threads=2` | generation_requests 34/34通过（含原金额/并发幂等/unknown预留/PC06/PC03A）；items 10项通过，唯一计数监听器用例被沙箱禁止绑定 localhost，非断言失败。该命令因此提前结束，未冒称 preparations 已运行 |
| `target/debug/deps/items-0565a023ab2beda4 document_source_url_is_never_fetched_by_the_server --exact`（获准本机监听后） | 受阻单项 1/1通过，items 合计11个不同用例通过 |
| `target/debug/deps/preparations-8a0e48540c011b3c --test-threads=2` | preparations 11/11通过，包括A的兼容/全局推荐/分页/GET零写和页幂等/CAS/封存；三测试套件合计56个不同用例通过 |
| `node node_modules/vitest/vitest.mjs run src/features/import/ConfirmStepPage.test.tsx src/features/library/workflow.test.ts src/features/import/quote-recovery.test.tsx src/features/import/messages.test.ts` | 四文件22项通过；含既有显式确认/过期/费用门禁，新旧报价消费恢复、GET未知不重报、同key/body、412授权保留、StrictMode初始恢复与显式报价链接读取、站内目标及URL上下文 |
| `PLAYWRIGHT_NO_COPY_PROMPT=1 EM_E2E_WEB_PORT=15533 EM_E2E_SERVER_BINARY=/Users/qsyj/Code/rust/everything-manual/target/debug/everything-manual node node_modules/@playwright/test/cli.js test --config playwright.workflow.config.ts` | 最终完整 Chromium 4/4通过，13.1s；预算保持/显式URL读取收尾后同命令加 `--grep 'PC3B-008'` 两恢复场景2/2通过8.0s |
| `node node_modules/typescript/bin/tsc --noEmit`；`node node_modules/eslint/bin/eslint.js . --max-warnings=0`；`git diff --check` | 最终通过 |
| `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo build -p everything-manual --features job-failpoints`，随后上述三个 Rust suite `--no-run` | 最终源码格式化后重建测试候选及测试可执行文件，均通过；未扩展未授权业务范围 |

首次浏览器运行确实暴露 StrictMode cleanup 使保存操作的初始 GET 结果失效，已修复且专门单测及真实 reload/new context 回归通过；不把首轮失败写成通过。一次测试等待只等错误链接未等消费 GET 完成，已改为等待真实生成门禁开放再手动触发；412整数金额规范化为42.00与输入42同一授权，不把展示格式差异称为预算丢失。

### AC 映射与证据边界

| AC | RD 证据 |
| --- | --- |
| AC-PC3-006 | 服务端明确优先级/tie目标，历史发布次入口；100物品批量、103候选跨batch。浏览器20行只调用有界summary，不逐行请求job/draft/release/preparation；加载下一页目标仍真实。summary500后静态身份/概览保留，不出现假「继续准备」，375px Enter重读恢复 |
| AC-PC3-007 | Rust当前输入指纹、准备资产兼容与视图集合；浏览器确认后五项已完成，新绑定PDF默认选新原件使准备/确认重查，显式旧doc仍使用原ready/quote，视图步骤链接保留doc。把侧视图改detail后视图/确认重查，无新增quote/job；原服务端冻结/发布不变量回归通过 |
| AC-PC3-008 | 真实POST已202建单后截断响应、GET500→只读核对；刷新仍未知，清存储的新context由服务端最新报价恢复唯一任务，用户自行进入详情。另一场在POST尚未到后端时断网，GET明确未消费后只显式原key/body重试；预算编辑锁定，2次浏览器请求正文/key一致，最终仅1job/1quote |
| AC-PC3-009 | 新增/原Rust认证/配置代次pending/expired/model guard/unknown预留回归；消费优先不因当前输入变化或过期丢job。单测过期+pending消费、读取失败、412保留原授权且无自动提交；浏览器375px键盘完成提交/核对与旧资料入口。未声称本卡浏览器造过所有401/412/pending组合，独立QA继续交叉取证 |

`artifacts/prd-completion/pc03b-rd/consumption-counts.json` 记录丢响应场景：jobs=1、quotes=1、建单POST=1；fixture upload=2、submit=1、task=1、manual=1、cdn=1，任务完成后重读/新context恢复这些计数完全不变。造数与首次真实本机fixture任务本身有这些调用，恢复阶段零新增，不冒称整场零provider调用。截图 `summary-375.png`、`recovery-375.png` 已人工查看；仅虚构资料，非密钥截图，未声称artifacts被Git忽略。范围为Chromium，未扩大最终平台矩阵。

服务端批量是有界内存/批量SQL，不是历史无限量时固定耗时保证；localStorage是可缺失提示，无法读取保存报价时仍保持结果待核对。系统默认文档仅在无显式URL选择时取最新；历史已消费报价允许找回其冻结任务，当前资料的五步状态仍可要求重新核对。未知结果不会自动重购或解除预留。

### 冻结候选

package0.1.0，schema8，debug + `job-failpoints`，**无 embedded-ui**，只供隔离QA；普通预览部署由协调者另构建。最终可执行文件 SHA-256：

- `target/debug/everything-manual`：`bec41094b445d57092845d7aa181ac31de900d9be95fa73d05ae9d7b9a302441`
- `target/debug/deps/generation_requests-1f807fc19ff42b49`：`09f176115b3f9677fd53c6accc64067d8a81ffaa1bd1206c1c6b473e42fd106f`
- `target/debug/deps/items-0565a023ab2beda4`：`5c7033e205660a66f504de349a66f8c831bec5be76b041c3323848b5b510f77d`
- `target/debug/deps/preparations-8a0e48540c011b3c`：`0056f080d75844146e0026b7b4ce0599049bf93917304ce5397af9901b75d1eb`

生成合同 SHA-256：OpenAPI `9176e62f24d1e9588554f1dea0bafc694b7ba97534cc65c6166e30650e97db37`；TS `efff2b627d79403202e09b775b0b5e2bb913eab1379f2349b290d35c3363801d`。精确生产/测试文件及 SHA 清单另存 `var/prd-completion/pc03b-rd-files.json` 供协调冻结，不包含用户配置或秘密。本卡自管测试服务正常结束；无继续Cargo任务，无target清理。`RD_READY` 后停止生产写入，待root冻结并派下一卡。

## BUG-PC3-002 发布后摘要入口修复（2026-10-02）

角色 RD，PC03B / AC-PC3-006，产品/UI revision2。独立 QA round7 以真实 HTTP publish 复现：草稿内容 revision4 发布后因 CAS bump 成5，release 保留内容 revision4；原摘要只比较两者相等，错误保留「复核草稿」。本补丁为 `RD_READY`，等待独立 QA round8；不替代 QA 结论。PC04 在只读设计边界暂停，未写其生产代码或合同，未混入发行 smoke 工具调整。

只改一份生产文件 `crates/server/src/storage/repo/item_summaries.rs`：除了既有内容 revision 相等的历史形式，识别发布事务内已存在的 `release_published` 审计事实。必须匹配 release 实体/物品/草稿、事务时间、发布前内容 revision，以及审计明确记录的发布后 revision 与当前草稿 revision。未使用 revision±1 推测；真实后续 PATCH 再增 revision 后自然回到复核入口。`json_valid` 的 CASE 分支内再核对 JSON 字段与整数类型，异常、缺失、错引用或字符串版本的审计保守返回复核，不让单条历史记录造成整批500。保持四条批量SQL、原排序和最新发布次入口；不读取 manifest/knowledge 大内容，不改发布事务、版本、账本或历史数据，无 schema/DTO/前端变动。

测试仅修改 `crates/server/tests/publishing.rs`：在既有 T19 真实适配器→草稿→HTTP显式复核/发布流程加入摘要断言，覆盖发布前复核、首次发布阅读版、同键重放仍阅读版、真实PATCH后复核、显式再次复核/发布后新阅读版；旧manifest/模型/费用/供应商计数不变的原断言保留。专门暂时破坏并恢复该发布审计，验证 malformed JSON/null/空对象/错误draftId/字符串afterRevision/错误afterRevision 均安全回退；该故障注入仅在临时测试库进行。

实际验证（根目录；所有供应商/CDN均 localhost fixture、临时库和假凭据）：

- `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo test -p everything-manual --test publishing publish_is_transactional_idempotent_and_immutable -- --exact` 编译通过，首次运行仅因沙箱禁止绑定127.0.0.1退出；获准后直接运行新测试可执行文件同一单项 **1/1通过**，未把受阻运行记成功。
- `target/debug/deps/publishing-ae1ef50be1299f8f --test-threads=1`（获准本机监听）：**7/7通过**，包括旧发布不可变、模型重生成/stale、显式模型/事实复核、发布字段不变量及新摘要回归。
- `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo test -p everything-manual --test generation_requests pc03b -- --nocapture`：**3/3通过**，原100ID边界/103准备跨batch/GET零写、任务优先级和稳定ID tie/旧发布次入口/消费恢复保持。
- `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo build -p everything-manual --features job-failpoints --bin everything-manual`、两修改 Rust 文件的 `rustfmt --edition 2024 --check` 与 `git diff --check`：通过。

没有重新运行无改动的浏览器/TypeScript/合同矩阵；后端新 binary 供 QA 与原122文件前端快照结合复验。package0.1.0 / schema8 / debug+job-failpoints / 无embedded-ui。精确 SHA-256：

| 文件 | SHA-256 |
| --- | --- |
| `crates/server/src/storage/repo/item_summaries.rs` | `e434995c2f7d82f3d1b781d44669fa5e2111d6c3f2e6c499872be87ac68dae3f` |
| `crates/server/tests/publishing.rs` | `d0f0294088b281c040f9db6395d8d4f4feb982e57fadfd2ceea425248d2aea39` |
| `target/debug/everything-manual` | `57a0650f0101a34d4bedc20b03e0028129444268f32af6569dc57ea57493462c` |
| `target/debug/deps/publishing-ae1ef50be1299f8f` | `dd9a3e9d961aee92b779323d7b12084fd189d572e8035ae4e9e45fdad7d1a7dc` |
| `target/debug/deps/generation_requests-1f807fc19ff42b49` | `7f1c02a70312134c911cb22af426710b35851e3beb01f6412af3ad83a8d8be4e` |

以上为10个不同Rust用例通过，定向单项是7项套件中的重复验证，不另增计数。未读/改用户配置、预览、真实样本；无真实付费。所有自有测试进程已结束，无 Cargo 进行；**RD_READY 后停止生产写入，等待root冻结与独立QA。**

## 发行 smoke HTML 脚本入口发现修复（2026-10-02，独立工具项）

协调者普通 B 构建的七步冷目录检查在步骤3暴露工具缺陷：HTML引用被BTreeSet排序后，原代码只选择首个JS（Skeleton预加载分块），因该分块不引用准备页，误判缺少实际存在的PDF worker。证据 `var/prd-completion/pc03b-normal-build/cold-smoke-tooling-issue.json`。此项不混入BUG-PC3-002或B2后端/前端候选。

只修改 `xtask/src/smoke.rs`（含同文件三个单测）：校验HTML中全部JS/CSS直接引用，并以全部JS引用作为图遍历根，沿构建中的真实引用发现worker；找到worker后仍遍历其他根，实际缺失chunk继续失败。HTML直接引用无论是否含hash都必须200。库源码中不存在的未hash路径文案仍按原规则忽略。资源遍历采用256个唯一引用上限，超过则明确失败，不再像原40项上限静默跳过。未硬编码worker产物名、未修改七步验收流程或跳过缺文件。

实际 `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo test -p xtask resource_tests` **3/3通过**：预加载排序先于module入口仍发现实际worker；另一根有缺失hash chunk/缺失HTML根时仍失败；真实worker缺失与未hash直接入口缺失均失败。`cargo build -p xtask` 与 `git diff --check` 通过。单测使用纯内存响应表，无网络、配置或用户数据。完整真实B2冷目录七步检查由协调者使用新复制工具独立运行，本段不提前声称该门禁已通过。

交付工具 `var/prd-completion/smoke-root-fix/xtask` SHA-256 `64372ca2c229625554a1f8e9a9945fd7c41794547fbbe61999c563b0b352a8eb`；源文件 `xtask/src/smoke.rs` SHA-256 `de75421d586338bebce7e84510101a0ac5458884790ab6f151f33ed2de61d304`。普通B2服务二进制和冻结前端均未修改，工具已独立复制，可与后续PC04编译隔离使用。

## PC-04 受控验证命令（PRD/UI revision 2，2026-10-02）

实现 `cargo xtask test-live --case <本地JSON> --budget-file <受限JSON>` 与显式 `--plan`。机器合同为 `crates/server/src/test_live/contract.rs`，使用说明和可替换模板见 [test-live](../../test-live.md)。只面向已初始化、停服的专用实例，OS目录访问权加现有排他锁是管理边界；不伪造网页会话，不读取开发预览配置，也不自动上传/准备PDF。案例引用既有ready资料，两provider继续使用现有安全配置。

### 实现与安全边界

- `test_live/plan.rs` 复用 `create_estimate_with_revision`、实际页资产检查、`plan_manual_ai` 和原 `build_stage_plan`。plan在短事务中回滚临时报价，零供应商调用/任务/费用预留。绑定实例定位与管理员、案例、原件/页/照片实际磁盘hash、两provider/model/preset/参数、配置代次/凭据不可逆绑定、价格目录内容和版本。不得仅凭同名价格版本忽略价格内容变化。
- 授权逐层拒绝未知字段；受限普通文件，两个币种分别使用非负i64整数，初始生成恰好一次。具名ID/模型字段拒绝高置信密钥形态。授权冻结为私有原子journal中的固定quote、body和幂等key；job/attempt/远端ID/账本仍只用原引擎。重跑按同一业务身份恢复；并发实例锁拒绝第二进程的新执行；journal尚无jobId也可查回已受理任务。
- `ExecutionScope` 是可选执行能力；普通serve无此scope，语义保持原样。领取和过期租约恢复均限于授权job；不恢复或修改同库其它任务。新外呼前重读授权到期/文件/配置/物理资产/计划/阶段集合及分币种actual风险；提交窗口在submitting后再次检查，CDN逐次跳转检查。已发请求允许保存不可变结果，停止不等于供应商取消。
- 可选retry仅限具体付费stage kind+batch+inputHash、明确未受理的safeRetry和1～5次上限；省略即无额外付费提交重试。unknown不使用该预算重购，查询策略沿用原引擎。Tripo已知task ID可继续查询，丢失receipt则保留预留待对账，不擅自把reserved改为settled或填actual=0。
- `report.rs` 白名单报告只列安全身份/hash、分币种planned/reserved/actual/state、阶段/attempt状态、已保存成果与下一步；remote ID只给hash，原值仍在既有任务库。完整结果导出受限GLB/结构化知识/报告，失败保留已存资产，知识仍needs_review，零自动复核/发布。fixture标题明确真实验收未运行，AC-042/T23始终单列NOT_RUN。中断或30分钟等待上限后保留授权记录，同授权恢复。
- fixture模式两provider及CDN只能字面loopback，客户端不跟随提交重定向；下载原SSRF/逐跳白名单保持，loopback禁用环境代理。执行fixture需要显式job-failpoints测试构建；普通发行构建不开放本机模型下载。

CLI计划公开字段在初稿基础上明确增加 `modelPreset`、`parameters`；报告增加 `maxInitialGenerations`、`retryScopes`。这是本卡新CLI合同，不修改HTTP/OpenAPI/前端生成合同，不新增数据库迁移或第二套费用引擎。

### AC 映射与实际验证

| AC | 实现与实际证据 |
| --- | --- |
| AC-PC4-001 / UI001、003 | 实际CLI help/缺预算、坏JSON、未知字段、0644权限、false/expired、secret-like ID等拒绝，固定错误无canary回显，job/quote/provider计数均0 |
| AC-PC4-002 / UI001、002、003 | 两次plan完全一致；同源3页批次/front+left/hash/model/preset/参数和独立上界；plan事务零quote/外呼，原件/准备身份与预设变化拒绝；运行中同版本价格内容变化停止新调用 |
| AC-PC4-003 / UI002、003 | 两币种各自0上限不足，负/小数/字符串/溢出拒绝；缺配置/价格及PC06疑似key模型拒绝；纯合同单测覆盖最大合法i64及未知嵌套字段 |
| AC-PC4-004 / UI004、005 | 实际适配器请求断言两视图token槽位、冻结模型、AI strict schema/输出上限及第1～3页、无第4页；校验实际GLB与有页码知识产物，报告不含假key/URL/实例路径，release计数0 |
| AC-PC4-005 / UI004、005 | 同授权完成重跑及并发只生成1job/quote/付费链；其它expired任务逐行保持；实际进程exit90分别发生于付费响应到达但receipt前/receipt后，租约恢复分别unknown保留预留/已知ID安全查询成功，所有重复执行paid计数不增加；429仅明确scope可再试，disconnect即使有scope也不重购 |
| AC-PC4-006 / UI004、005 | 运行中expiry、同版本价格改动、stage inputHash破坏、actual超授权分别停止；已返回paid成果保留且新Tripo调用0；供应商429无重试授权明确needs_input，unknownactual保持null，既有下载/校验失败部分成果回归通过 |
| AC-PC4-007 / UI001、003、005 | 新CLI集成默认ignored且必须明确指定隔离工具；默认Playwright排除专用qa-pc，原基线保持；所有本卡动态验证只到localhost假供应商，无真实收费、无用户preview/config读取。报告保留真实AC-042/T23 NOT_RUN |

实际命令（根目录，Rust均 `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0`；首次新套件在沙箱内绑定loopback受阻，获准后才执行，不将受阻运行计成功）：

1. `cargo build -p xtask -p everything-manual --features everything-manual/job-failpoints --bins` 通过。工具与fixture server为package0.1.0/schema8/debug+job-failpoints，无embedded-ui。
2. `cargo test -p everything-manual --lib test_live`：2/2纯合同单测通过。
3. 显式 `EM_PC04_XTASK_BINARY=<冻结xtask> <publishing-testbin> test_live_cases::pc04_cli --ignored --test-threads=1 --nocapture`：6/6实际CLI subprocess/localhost场景通过（每组包括多个边界，按6个Rust用例计，不膨胀计数）。
4. 受影响旧回归，真实localhost fixture、隔离data：jobs_recovery 25/25、manual_ai_contract 20/20、model_assets 24/24、pipeline 14/14、tripo_contract 18/18、publishing 7/7，共108个。publishing默认跳过本卡6个显式CLI用例；它们已由上一独立命令实际执行。
5. `cargo clippy -p everything-manual -p xtask --all-targets -- -D warnings` 最终通过，无允许忽略参数。最初全量检查发现两个既有同型转换及9处既有测试Result转换告警，经协调者授权等价修正：preparations的i64范围值去冗余转换；encrypted_secrets_rd的ok/err.expect改为固定诊断expect/expect_err（先把异常值映射到unit，仍不Debug打印供应商响应）。未降低断言阈值。自有两个len判断改is_empty，report参数改具名context。
6. `PATH=/opt/homebrew/bin:$PATH target/debug/xtask contracts --check`：OpenAPI与generated.ts一致；`node node_modules/typescript/bin/tsc --noEmit` 与 `node node_modules/eslint/bin/eslint.js playwright.config.ts --max-warnings=0` 通过。`git diff --check` 通过。

日志位于 `var/prd-completion/pc04-*.log`，均忽略目录中的假fixture证据。最终冻结路径、SHA和最后等价修复的定向复验见下方交付清单。

### 单独工程入口修复与交付范围

默认 `apps/web/playwright.config.ts` 只新增 `testIgnore: "**/qa-pc*.spec.ts"`，专用PC验收依赖其独立冻结快照、复制binary及端口，必须用各自 `--config` 执行，helper没有target/旧快照fallback。动态 `PLAYWRIGHT_NO_COPY_PROMPT=1 node node_modules/@playwright/test/cli.js test --list --config=playwright.config.ts` 实际发现138 tests / 23 files；静态集合当前33spec，仅排除10个qa-pc前缀（包括QA正在准备的PC04），保留qa-t16/t18等全部旧基线。完整前后文件集合 `var/prd-completion/pc04-e2e-discovery.json`；清单日志 `pc04-default-e2e-list.txt`。这不移除验收，最终root按专用配置逐项复验。

本卡生产/测试文件：`test_live/{contract,plan,report,mod}.rs`、`jobs/{scope,executor,submission,mod}.rs`、`generation/jobs.rs`（仅暴露同源stage plan）、`storage/repo/job_stages.rs`、`providers/tripo/{client,handlers}.rs`、`providers/manual_ai/client.rs`、`assets/glb/download.rs`、`lib.rs`、`xtask/{Cargo.toml,src/main.rs}`、Cargo.lock、`tests/{publishing.rs,support/test_live_cases.rs}`；工程等价修复 `preparations/mod.rs`、`tests/encrypted_secrets_rd.rs`；前端仅Playwright默认发现配置。文档更新 `llmdoc/test-live.md`、README索引及本段。此前独立smoke修复和B2测试helper已各自交付/验收，不冒充PC04新增业务。

边界：未运行真实供应商、真实预算或人工发布，不能声称T23/AC-042完成；不实现无浏览器PDF准备或通用流程平台。不把测试构建当正式发行包；普通embedded-ui构建、完整矩阵及独立QA由root后续执行。实例锁要求停服专用库，不能让CLI与网页服务共享活动实例。每次新调用的门禁核对实际内容hash，偏重安全一致性，没有声称大资料高吞吐性能。

### RD_READY 冻结交付

最后定向复验直接使用已复制物：6/6 CLI（21.83s）、2/2合同unit、秘密响应分类固定诊断1/1、准备发现/兼容2/2均通过，输出保存在 `var/prd-completion/pc04-rd-ready/*-final.log`。最后复制xtask的 `contracts --check` 也通过。各轮重复执行不叠加，合计119个不同Rust用例（本卡6CLI+2合同、108受影响旧回归、3个等价lint相关回归）。没有重复无关全矩阵。

固定复制目录：`var/prd-completion/pc04-rd-ready/`（不依赖随后被覆盖的target）。下列SHA-256均为复制文件自身：

| 文件 | SHA-256 |
| --- | --- |
| `xtask` | `df05f92aac23619e1cd53fa998b3b586a4a6f3d266b8322bfa0078749f59a8db` |
| `everything-manual-fixture` | `086f4e0a5f9e6f05be1cd7a7b6cce1fed3a0ee9810e0ff341c9fa7812461f10a` |
| `publishing-ae1ef50be1299f8f`（含6CLI） | `3f781a7597ac8daab7aaacbc87aa2456fa43bf873ad875e8be384a27846c9f99` |
| `everything_manual-39127789337299c0`（unit） | `c8d90fabfe886d77bcf46f3ee3f9a3b152fc954aee9700b05ce45a8a4db0df90` |
| `encrypted_secrets_rd-c342c431af888cff` | `d06ff8b326a5dfe2373c4d698bee48066046c24c617e780386f01171fe6d2c09` |
| `preparations-8a0e48540c011b3c` | `919a2f26e217541e7a20a6bca2874418539f7ca3f98e9534d20489199c664418` |
| `jobs_recovery-ec5c590c74aa7375` | `5aec381103a9b8cb59d556c3575d83f6a05615d529a2135efebb61c2cbc5c14d` |
| `manual_ai_contract-933dd94ad9df66bd` | `3b82147734511b93d346e2bc95d01cc4dceaab8bd359197e3747a5280a8afce1` |
| `model_assets-327247f2c828f066` | `50eff341a7bc03161d8e016253f574545838502a9f76a3ddfbc5d383dce74c79` |
| `pipeline-ce6fb328978023d0` | `d439b7859ed59b9b4019421e39a13ece0cac3b499c21fd65cf5b0dc6815fa045` |
| `tripo_contract-471ca93612e8b6ee` | `a99d16c31ae7849da9e0eb6d9a562b6fd5129dd9a3afd2eec352f738b89aa34e` |

同目录 `binaries.json`（11复制物及来源/字节/SHA）SHA `b29a71e7f31b1687ab002083d5dc6971bf787e1868c6af2a5d3be5e18ecf32bf`；`pc04-files.json`（25项本卡源码/测试/配置/说明文件，不含本交接自引用）SHA `b4326d764c4afd2eca611f080427449ad117bab0c4c70f6528f4d8bf59e33883`；`rust-source-manifest.json`（211项全Rust/Cargo/迁移文件）SHA `b473ce63e608da566a678b06c901bd345e2461a870fd3f55549aeb88cac9136b`。

**RD_READY：本卡生产写入停止，所有自有Cargo及测试session已完成，Cargo交还root。** 本段是RD交付，不代替独立QA签署，不修改state。预览8080/5173、用户数据/配置/官方样本未动；真实付费与T23仍NOT_RUN。

## PC-05A — 服务端搜索、列表位置与发布入口（2026-10-02，RD_READY）

work-id `prd-completion`；PRD/UI revision2 §§14.1、14.2、14.4；REQ-PC5-001/002、AC-PC5-001～003、UI-PC5-001～003。PC03B round8/BUG-PC3-002 CLOSED 为依赖。由原 UI 工具任务接任唯一 RD 后实施；此前工具探针只为测试环境准备，不计本卡产品验收。**本节是 RD 交付，待独立 QA；没有修改原 PRD/state/QA 报告。**

### 实现与理由

- `GET /items` 新增 q。去首尾空格、最多200个 Unicode 字符、ASCII大小写不敏感；SQL `instr(lower(name/model), q)` 字面包含，`%`/`_`无通配语义，非ASCII不折叠。全库查询留在 SQLite，前端不遍历全部页过滤。未知/重复参数及超长字段仍422。
- 新游标绑定归档范围、固定 createdAt DESC/id DESC 排序及规范化 q 的 SHA；同义ASCII大小写/首尾空格可以续页，换条件422字段cursor明确从头分页。旧无q游标只在无筛选查询解释；未开放新排序参数，也不对新q静默重用旧scope。
- 输入文本与已提交条件分离，Enter/搜索才更新URL。搜索/归档/清除重置游标并产生可前后退的历史；加载更多成功后才更新cursor。每个 URL 位置有独立 Query 缓存，保留已加载前缀；Back恢复此前页数，刷新无缓存时从URL cursor继续。等待/失败保留上次结果并标注，错误保留安全requestId；错游标单个重置入口，无匹配可清搜索，空归档可切回使用中。
- 导航位置只存在本页会话内存，最多100个位置/物品返回点，保存滚动和物品ID而非正文。概览/版本列表/阅读器“返回资料库”恢复q/归档/页位置与仍存在的目标行；数据变化则回合理旧滚动位置。没有跨设备/刷新恢复未保存输入的承诺，也未写浏览器持久存储。
- 复用 PC03B `latestReleaseId` 与主任务/旧发布次入口；服务端同一批量摘要增加该版本的 `latestReleaseDraftRevision/latestReleaseCreatedAt`，显示“已发布 · 草稿 rN · 时间”。历史列表和两个下载入口沿用 PC01，不新增逐行版本/manifest请求或第二套导出。UUID仍保留于版本详情。

### AC 与实际验证

| AC / UI | 实际证据 |
| --- | --- |
| 001 / UI001、002 | Rust创建目标后再建45个干扰项，第一页没有目标而名称中文/型号ASCII/字面%_直接命中；归档隔离、非ASCII不折叠、200/201 ASCII/中文/非BMP字符、17行limit3分页与全量期望顺序一致。真实浏览器输入不自动查询，Enter提交trim，切范围，44px搜索按钮和375无横向溢出 |
| 002 / UI001、002 | Rust q/archive错游标及legacy无q兼容/拒绝；组件旧数据等待/失败、迟到响应不覆盖新q。Chrome1440/375真实20→40行、Back20/Forward40、刷新从URL页恢复、概览返回目标行可见；真实后端422与显式重置、负向500安全requestId、重试真实空结果、超长无请求 |
| 003 / UI003 | localhost fake pipeline真实生成草稿，真实复核/绑定与两次HTTP publish得到R1/R2；批量摘要的ID/revision/时间与实际release同源。明确私有历史failed job状态夹具下仍可读R2、历史可读R1；两个宽度实际下载4个ZIP，原ZIP inspector验证版本manifest/PDF/GLB结构与所有文件SHA；无逐行详情请求、无新增job/attempt/ledger或供应商调用。真实发布Rust回归另验证不可变/幂等与摘要字段一致 |

实际命令/结果（根目录；Rust均 `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0`，未启额外测试profile或清target）：

1. `cargo test -p everything-manual --test items -- --test-threads=1`：最终 **14/14**，含3个新PC05搜索用例；最初沙箱内原“source URL不外呼”监听器被EPERM阻止，获准localhost执行后通过，受阻轮不计通过。日志 `var/prd-completion/pc05a-items-ready.log`。
2. `cargo test -p everything-manual --test publishing publish_is_transactional_idempotent_and_immutable -- --exact --test-threads=1`：**1/1**，增加最新版本revision/时间读回断言。先前误用不存在的item_summary筛选跑0条，明确不计测试证据；最终日志 `pc05a-summary-publish.log`。
3. `node node_modules/vitest/vitest.mjs run src/features/library/LibraryPage.test.tsx src/features/library/library-navigation.test.tsx src/features/library/workflow.test.ts src/features/library/ItemFormPage.test.tsx`（apps/web目录）：**10/10**，4文件；包括StrictMode取消首个rAF后仍实际恢复的回归。日志 `pc05a-components-ready.log`。首轮测试期望误写未编码href已修正；另移除重复的错误游标重置按钮，未降低错误门槛。
4. 最终 `node var/prd-completion/pc05a-rd/run-browser.mjs attempt5 pc05a-rd-ready`：**6/6、0skip/0retry，17.97秒**，实际 headed **Chrome154.0.8037.93**，1440/375。新快照125文件+最终copied fixture，15535隔离，原PC03B helpers只在私有镜像适配端口/OS sandbox，原文件hash不变。证据 `var/prd-completion/pc05a-rd/attempt5/{identity.json,summary.json,evidence/}`。
5. `cargo clippy -p everything-manual -p xtask --all-targets -- -D warnings`、`tsc --noEmit`、本卡全部改动TS/TSX的定向ESLint均通过。clippy最初发现新测试未使用csrf，改为`_csrf`后通过；没有加allow规则。日志 `pc05a-clippy-ready.log`。
6. `target/debug/xtask contracts`生成OpenAPI/types；最终 copied `var/prd-completion/pc05a-rd-ready/xtask contracts --check`通过，日志同目录`contracts-check.log`。`git diff --check`通过。`cargo build -p everything-manual -p xtask --features everything-manual/job-failpoints --bins`生成复制物，日志`pc05a-build-ready.log`；无embedded-ui，不把fixture称普通发行。

不同用例计数为 **15个Rust + 10个组件 + 6个浏览器场景**，不把各轮重复相加。首次浏览器用PW.check即时检查受控checkbox失败，改为真实click后重试断言checked，实际状态正常；随后明确复现StrictMode提前标记restored导致取消rAF后不恢复滚动。修复为实际帧执行后标记，并在此之前不记录被短页面限制的位置。原失败attempt1/2保留，定向attempt3两导航场通过，最终冻结attempt5全6场通过；没有sleep/改CSS/放宽可见性代替修复。

### 文件与冻结交付

本卡实现：`http/items.rs`、`storage/repo/items.rs`、`http/dto/items.rs`、`item_summaries.rs`/对应repo；`endpoints.ts`与生成合同；`LibraryPage`/`items`/`workflow`、新`library-navigation`、概览/ReleaseList/ReleaseReader的返回入口、`theme.css`。测试：`items.rs`、`publishing.rs`、新`LibraryPage.test.tsx`/`library-navigation.test.tsx`；私有真实浏览器spec/runner位于`var/prd-completion/pc05a-rd/`。文档只更新本节与`llmdoc/contracts.md`的搜索/摘要合同说明；root同时修改的README/历史进度不计本卡改动。

前端 `var/prd-completion/web-snapshots/pc05a-rd-ready`，**125文件**，`source-hashes.json` SHA **`2e852b3a0f4c817f9397d0c4c8ab305ac30168d8d1bd24043978f5139645b1bf`**。

复制目录 `var/prd-completion/pc05a-rd-ready/`（QA不依赖target）：

| 复制物 | 自身SHA-256 |
| --- | --- |
| everything-manual-fixture | `f1eddd3b78c3c24b806741ea78fd848581a4e440466b71964493a23b2ba65f73` |
| xtask | `3cbe57832f5bc48708d8dc56c8e458034ff3e43cec7c53cef2543edaa1871a02` |
| items-test | `ff4d2bde4e7852720953c1aae2b328d047bab29adc20f41b9120f01cb48d3a78` |
| publishing-test | `20d91a90b1a6479f0c07d5103685b550144ad13abef2e1871a5c38dbdb2a458e` |

`binaries.json` SHA `edd5430f7baa521618432747efedc74044cd99cdf964856b1859d5692f8d686b`；211项直接路径→SHA映射的`rust-source-manifest.json` SHA `d314435dc76b19d55f56861e4d9b638fbadd689ee28f2daeeede0d07f2c47e98`；21项本卡文件（不含本交接自引用）的`pc05a-files.json` SHA `2b28a9e73078e2c2830b3ddab69f3b313a7e78795d050ec7277aab52621701cc`。

**RD_READY：生产及Cargo停止，15535与自有fixture已清理，交root/独立QA。** 无preview/用户配置/官方样本改动、无真实provider/付费、无commit；trace/HAR/video关闭。PC05B/C、最终三浏览器AC063、普通embedded发行及T23均不在本卡签署范围。真实生成输入准备在本卡明确为synthetic格式夹具，不宣称新的PDF.js准备或供应商质量验收。

最终全量测试基础设施另有root已记录事项：原baseline `runtime.ts::serverBinary`及部分构建helper仍固定target/可能自动Cargo。此次私有runner显式复制/校验binary，并不意味着所有baseline已完成同样改造；该工程事项留最终回归前处理。

### PC05A — BUG-PC5-001 修复交付（2026-10-02，RD_FIX_READY，待 QA11）

QA10 的搜索清除→Back 失败已在 RD 的真实 Chrome 1440/375 各复现。私有诊断 attempt7 显示：`setInput("")` 同步提交，而 Router 的 q 清除使用 transition；快速 Back 取消了尚未提交的 transition，组件从未渲染过空 q，原 `[search]` effect 因依赖未变化而无法恢复输入。因此不是浏览器表单自动恢复的推测；仅 `autoComplete=off` 的 attempt6 仍失败，未保留为修复。

修复仅 `LibraryPage.tsx` 与 `LibraryPage.test.tsx`：尚未提交的输入草稿绑定 `location.key`；提交/清除后解除草稿，已提交文字直接来自 URL。没有定时等待、DOM 强制写入、history entry 补造或降低断言。新组件回归在同一 act 中执行清除→立即 Back，并继续覆盖输入新 q→提交→立即 Back 与两次 Forward；后退时 q/归档/文字一致。PC05B 已暂停，其 WorkProtection/App/settings/auth/shell 改动仍留工作树，明确不进入 A2。

验证：A 隔离快照下4组件文件 **11/11**（原10+新1，`var/prd-completion/pc05a-bug001/components.log`）；新测试误用了 Playwright 的 `exact` 选项，tsc 正确报错后仅去掉不支持的选项，最终该文件 **5/5** 与完整冻结前端 `tsc --noEmit` 通过（同目录 `components-final.log/typecheck.log`）。两改动文件 ESLint、copied xtask `contracts --check`、`git diff --check` 均通过。

真实 Chrome **154.0.8037.93**、headful、本机15535、f1eddd3b copied fixture：`node var/prd-completion/pc05a-rd/run-browser.mjs attempt9 pc05a-bug001-ready 'search/paging/history'`，1440/375 **2/2、0skip/retry，8.14秒**；保留原快速 Back 的门槛，并验证 URL/归档/输入、Forward、分页和返回位置。attempt8 两场为中间确认；不叠加次数。所有自有实例已清理，15535无监听。浏览器使用的生产源与最终 A2 字节完全相同，最终变化仅测试的 TS 选项修正；中间快照包含从 A1 复制的可重建 dist/vendor，最终 source manifest 按 A1 的125项重算，不把生成物计入源码。

最终冻结 `var/prd-completion/web-snapshots/pc05a-a2-ready`，125项 manifest SHA **`b5cfb55ce19d5ae5d19eda7849b4f9b50aaae2b11a4929341eef9743a67b4646`**；相对 A1 只有2项变化：LibraryPage.tsx SHA `bc9f4bdae7a6024f2d0869b9ad11227b047dd5a2d6fcb7244e1ce348ffdc32af`，LibraryPage.test.tsx SHA `095bb516af9f253b5dd26b020d2992d1dc8d02257b71fafdb646944ccf875014`。其余123项一致，B改动全部排除。211项 Rust/Cargo/迁移路径全量核验与 A1 manifest `d314435dc…c47e98` 一致，无Cargo执行、无需重建；复用 A1 fixture `f1eddd3b…65f73` 及 copied Rust。精确 delta/路径/哈希：`var/prd-completion/pc05a-bug001/handoff.json`。

**RD_FIX_READY：修复写入停止，等待独立 QA 关闭 BUG-PC5-001，不代签 CLOSED。** root 可直接从 A2 冻结快照构建普通 embedded；不要从含未完成 B 的 mutable apps/web 构建 A2。无用户数据/预览/真实供应商操作，未改 PRD/state/QA报告。

## PC-05B · RD_READY（2026-10-02，待独立 QA12）

依据 PRD/UI revision 2 §14.1、14.2、14.5：REQ-PC5-003/004、AC-PC5-004/005、UI-PC5-004～006。起点是 QA11 已接受的 A2；本卡未重新修改 A 搜索/分页逻辑。只实现编辑与本页工作的保存/离开保护，不包含 PC05C，不修改费用、供应商引擎、加密或 Rust 合同。协调 state 与 QA 结论由对应角色写入。

### 实现与实际边界

- 路由层 `WorkProtection` 汇总当前编辑、上传/登记、PDF 准备中的工作，复用一个原生 HTML dialog。Link、程序 push/replace/go、同文档真实 Back/Forward 均经过保护；默认「继续处理」，Escape 取消并恢复来源焦点，首尾 Tab/Shift+Tab 循环。浏览器刷新/关闭用真实 beforeunload，不承诺定制原生文案。已保存的成功导航和登录失效跳转使用明确 bypass；普通阅读、无变化表单、已经受理的后台 job 不注册本页工作。
- `popstate` relay 在 BrowserRouter 挂载前注册，避免晚注册监听时 Router 已卸载编辑区。拒绝 POP 先恢复原历史项，接受才执行原 delta，不新造历史项；未保护的 `go` 不设置残留 bypass。当前生产没有数字 navigate 调用，组件另覆盖“无 dirty 的越界 go 不发 POP，之后 dirty Back 仍弹框”。这个组件事件用例不冒称原生浏览器越界行为证据。
- 窄屏知识编辑的旧 Drawer 在事件目标属于上层 `dialog[open]` 时让出键盘处理，保持原编辑区挂载；防止旧 capture handler 抢走 Tab、先关闭 Drawer 或截断 Escape。真实 375px Back、双向 Tab、Escape 回到同一字段与保留输入已验证。
- 物品与知识显示真实 dirty/pending/saved/error 状态，操作旁 polite 状态与失败 alert；同步锁和 pending 禁用避免双提交。知识 PATCH 必须随后 GET 成功才收起编辑/显示已保存。物品成功仍按原路径进入资料步或概览，不误弹。字段校验失败保留输入及首错误焦点。
- 非密钥物品/知识草稿只存在 provider 生命周期内的内存 Map；按 item/draft 身份隔离。全局 401 到 SPA 登录后可回到原路径、同一内存编辑与原 ETag，没有自动提交。显式离开/注销清除对应槽位，已失效槽位拒绝晚响应重新写回。没有 localStorage/sessionStorage/IndexedDB 正文草稿；API 配置和新密钥从不进入这个 Map。
- 412/未知保存结果保留本地输入，不在旧输入下悄悄安装新 ETag。「核对最新版本」读取服务器内容供比较；只有明确「丢弃本页修改并加载最新版本」且其 GET 成功才替换本地状态。取消或失败 GET 不替换；401/卸载后的晚读取不再打开过期确认框或更新已离开的页面。未知创建结果禁止再次 POST，提供真实名称搜索核对，并明确同名/未查到都不能证明先前请求是否成功，没有伪造创建幂等承诺。
- 上传保护持续到资料登记完成，接受离开中止仍在本页的上传；PDF.js 准备中止后不继续发后续页，已经成功的页/资产保留，恢复只补缺页。已发出的单个请求可能完成，不承诺回滚。明确 seal 仍由用户点击，无自动准备确认或后台重放。
- 设置页移除旧私有离开监听，接入同一个 guard；原有模型误填遮蔽/加密提交逻辑保持，避免两套离开框。控件延续 44px、说明 14px、375 换行规则。

### AC 与 RD 验证

| AC | 实现及真实证据 |
| --- | --- |
| AC-PC5-004 / UI005 | B1 375 item Link/Back 取消、Back 接受后 Forward、原生刷新取消与接受；B3 知识原生取消；B4 实际 XHR 上传中取消与离开停止、已有照片不变；B5 实际两页 PDF.js 准备中原生关闭接受、另 context 只 PUT 第2页、原第1页数据库事实不变及两页 JPEG/text SHA；B8 窄屏 Drawer 内 Back 弹框、真实 Tab/Shift+Tab/Escape 保留编辑与恢复焦点。 |
| AC-PC5-005 / UI004、006 | B1 延迟真实201响应且仅一次POST/pending；B2 实际201后丢响应、真实另一会话412、401登录内存返回，核对不盲发；B3 PATCH实际成功而GET失败不收起、多个编辑buffer、真实412/401及迟到失效读取；B7 PATCH发送前失败/真实提交后丢响应均保留输入且明确核对。真实后端读取、版本/业务计数比对，故障响应明确为测试注入。B6 全fake设置遮蔽、统一离开、原生刷新取消、实际PUT成功并清密钥字段及无浏览器存储。 |

实际命令与结果（全部无 Cargo）：

1. `node var/prd-completion/pc05b-rd/run-browser.mjs attempt7 pc05b-b2-ready`：实际 headed **Chrome 154.0.8037.93**、15537、自管临时后端及 localhost 假供应商，**B1～B7 7/7、0skip/retry，48.16s**。`attempt7/{identity.json,summary.json,evidence/}` 记录身份、HTTP事实、真实 PDF 页 hash 与计数；正常成功响应均来自真实后端，部分测试转发真实响应以制造延迟或丢包，未伪造正常成功。准备场景用真实 PDF.js；知识场景的准备造数是明确的 synthetic-ready fixture + 真实本机 pipeline，不冒称官方资料/OCR效果。
2. `node var/prd-completion/pc05b-rd/run-browser.mjs attempt8 pc05b-b3-ready 'B8 375'`：**1/1，6.23s**。最终 B3 比 B2 仅 `Drawer.tsx`/`Drawer.test.tsx` 两项变化；新增 B8 验证该差异。前7场结果属于 B2 原记录，不写成重新跑过 B3 全7场。
3. `node var/prd-completion/pc05b-rd/run-review-regression.mjs review1 pc05b-b2-ready`：**3/3，15.20s**，真实 PC02B 并发 PATCH412/发布412，以及原样 T19-4 无buffer刷新。只把 RD `review-tasks.spec.ts` 两条旧 `page.once(native dialog)` 交互改为共享 HTML dialog/明确丢弃；真实冲突、取消保留、接受后 GET500 保留、成功 GET 才更新/清编辑和发布仍禁用的断言都保留。原 T19-4 无需弹框且断言逐字不变。私有镜像中的三个用例与当前 tracked spec 字节相同；helper 仅显式 prebuilt 跳过 Cargo，默认 target 路径指向 OS localhost sandbox wrapper，确实执行复制二进制。
4. 冻结组件：在 apps/web 使用 `EM_PC03B_QA_WEB_ROOT=<绝对B2路径> node node_modules/vitest/vitest.mjs run --config vitest.pc03b-qa.config.ts`，选择 WorkProtection、shell、ProviderSettingsForm、ItemFormPage、useDraftMutations、LibraryPage 共 **6文件45项通过**；B3 对 WorkProtection、Drawer、PageLayout **3文件14项通过**。共8个不同文件/53个不同用例，不把重复6项 guard 累加。日志 `var/prd-completion/pc05b-components-{b2,b3}.log`。复用的 QA config 只提供冻结根与依赖 fs.allow，不修改 QA 文件或断言。
5. apps/web 完整 `node node_modules/typescript/bin/tsc --noEmit`、`node node_modules/eslint/bin/eslint.js .` 均通过；日志 `pc05b-typecheck-final.log`/`pc05b-lint-final.log` 零错误。copied `var/prd-completion/pc05a-rd-ready/xtask contracts --check` 与 `git diff --check` 通过。Rust/Cargo/迁移211项逐项 hash 与接受的 A2 完全一致，复用已验 backend/testbins，不无因重跑 Rust。

自检失败证据均保留：初始 jsdom 缺 showModal/close 只在 test/setup 补 shim；真实浏览器另证明原生行为。第一次 Back 暴露 relay 注册顺序问题并修正；一轮测试误把跨文档 page.goto 的 Back 当 SPA 已改用真实站内 Link。原生刷新取消时 Playwright 导航 promise 不一定 resolve，helper 只接受明确取消/超时工具表现，仍核对 beforeunload 事件、同一 document 与输入；接受刷新必须实际新 document，不吞异常。attempt6 新增反向 Tab 环测失败后补 dialog 首尾键盘处理，attempt7 原门槛通过。冻结 Vitest 首次 worker URL 被 fs.allow 拒绝，随后使用既有冻结配置通过，不把该环境失败算成产品PASS。QA authoring中的短暂TS错误由QA自行修正，未改其源文件。

### 文件、冻结与交接

相对接受 A2 的前端20项变化：App；AssetUploadCard/DocumentStepPage/PreparePage；ItemFormPage；CalibrationWorkspace/KnowledgeReviewPanel/useDraftMutations及其test；ProviderSettingsForm及其test；AppShell/SessionExpiryWatcher；新增 work-protection 及其test；Drawer及其test；shell.test、test/setup、theme.css。另外 RD `tests/e2e/review-tasks.spec.ts` 更新两条受影响旧UI交互。精确21项路径→SHA见 `var/prd-completion/pc05b-rd-ready/pc05b-files.json`，SHA **`f7d929cf7d1869f4160d642c7032138e5129e7e63679fc9f9271692b4dd7460c`**。root文档、独立QA文件不计本卡生产差异。

最终前端：`var/prd-completion/web-snapshots/pc05b-b3-ready`，**127文件**，manifest SHA **`cb4419fc5abe1f2818babe90d6cc06ab3eba0b1b290d7d01b12b5a0ad1018a60`**；逐项与当前 apps/web 源码一致。先前 `pc05b-rd-ready`/`pc05b-b2-ready` 候选与失败日志保留，最终以 **b3-ready** 为准。

`var/prd-completion/pc05b-rd-ready/` 已独立复制 A 的4物，QA不依赖target：fixture **`f1eddd3b78c3c24b806741ea78fd848581a4e440466b71964493a23b2ba65f73`**；xtask **`3cbe57832f5bc48708d8dc56c8e458034ff3e43cec7c53cef2543edaa1871a02`**；items-test **`ff4d2bde4e7852720953c1aae2b328d047bab29adc20f41b9120f01cb48d3a78`**；publishing-test **`20d91a90b1a6479f0c07d5103685b550144ad13abef2e1871a5c38dbdb2a458e`**。binaries.json SHA **`f540029d48f27866b8c2c944326568334b63bcca20c8498fed5f95f008cf116c`**；211项 rust-source-manifest SHA仍 **`d314435dc76b19d55f56861e4d9b638fbadd689ee28f2daeeede0d07f2c47e98`**。机器可读身份/适用范围见同目录 `handoff.json`。

**RD_READY：生产写入及测试已停止，15537无监听，自有临时实例已清理；没有占用Cargo。** 交root由最终冻结前端构建普通 embedded、再由 QA12 独立验收。本轮未操作8080/5173、真实配置/秘密、官方样本、真实供应商或付费；trace/HAR/video关闭，测试正文和凭据均虚构。浏览器原生确认文案和跨刷新丢弃的限制保持真实，不承诺持久恢复未保存正文。不代签最终 AC063/五分钟性能/Linux/真实模型质量或发行验收。

## PC-05C · RD_READY（2026-10-02，待独立 QA）

依据 PRD/UI revision 2 §14.1、14.2、14.6，REQ-PC5-005/006、AC-PC5-006～008、UI-PC5-007～009；起点为独立 QA12 接受的 B3。只增加任务展示所需的读取合同及会话提示，未改供应商执行器、付费、轮询策略、未知提交重购或密钥加密语义。

### 实现与 AC 映射

| AC | 实现与证据 |
| --- | --- |
| AC-PC5-006 / UI007 | 阶段 DTO 的 `safeRetry:{number,limit}` 从同一持久化 `attempt_count` 和 core `MAX_SAFE_RETRIES` 输出，仅正在安排的 retry_wait、非终态/非 unknown 父任务有效；provider 的 `poll_count` 独立。`RetryCountdown` 每秒本地计算 nextRunAt 与 Date.now，不请求接口；0 显示等待调度、缺时间明确不可用，变化时重算，离开等待/终态移除，aria-live=off。实际 Chrome 真实本机 CDN429 的 Retry-After12秒、1/5、倒计时变化、归零无业务提交、缺时间/改时戳、running/failed及终态停详情轮询已通过。**真实 hidden/resume 在 RD Chrome 中未完成，限制见下文；组件时钟/visibility 测试不能替代原生覆盖。** |
| AC-PC5-007 / UI008 | 会话保护的 `GET /api/v1/jobs/activity → {data:{active}}` 使用单条全库 COUNT，仅 queued/running/retry_wait/waiting_provider；不含 needs_input、submission_unknown 或终态，不展开 stages/attempts/ledger，不使用当前列表页数量。顶栏保持任务入口，加载中不假报0，读取失败即使有旧数据也显示不可用；375 使用紧凑图标+数字并保留完整无障碍名称。复用可见2秒/不可见15秒的读取节奏，单一 summary query；0仍读取以发现新任务。真实24个进行中/列表仅20行、24→25→24、故障→不可用→恢复、375键盘与1440展示通过。 |
| AC-PC5-008 / UI009 | 非秘密 WeakSet 记录当前 QueryClient 是否实际恢复或登录成功；初次401普通登录，已验证会话401才显示过期。沿用安全next及B的provider内存编辑，登录后不自动重放写请求，登出清除此标记。业务配置错误仍非401，不触发登录跳转；确认页补齐实际可操作的设置/部署价格说明。真实初次匿名、实际清cookie后POST401、返回原物品输入、无自动POST/浏览器正文存储、恶意next留站内通过；真实 PROVIDER_NOT_CONFIGURED409仍可读原PDF且供应商0调用。模型/价格分支由已有组件回归及独立QA继续核验，不将该409场景说成三个业务错误均已实测。 |

`JobDetailPage` 对非401读取失败也保留最后状态并标注未更新；没有将读取失败改写为任务失败。B的物品/知识内存、保护框、设置页秘密生命周期保持。接口与字段已同步 Rust DTO/OpenAPI/generated types 及 llmdoc/contracts.md；没有 migration。

### 实际验证与失败证据

Rust命令均 `CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0`，只定向构建，未清理target：

1. `cargo test -p everything-manual --lib http::jobs::tests -- --test-threads=1`：4/4，包含新增安全重试次数、poll/unknown/terminal区分，以及原脱敏测试。`cargo test -p everything-manual --test generation_requests pc05c_activity -- --test-threads=1`：1/1，真实路由401/0/25与20行分页、终态24、同源4/5和timestamp；声明的私有SQL状态夹具，不伪造HTTP成功，读取前后attempt/账本不变。
2. `cargo test -p everything-manual --test jobs_recovery retry_after -- --test-threads=1`：2/2；`cargo test -p everything-manual --test jobs_recovery poll_pace_ramps_from_three_to_fifteen_seconds -- --exact --test-threads=1`：1/1。保留既有执行器2/4/8/16/32与Retry-After上限、正常poll不消耗safeRetry的门槛。合计8个不同Rust用例，日志为 `var/prd-completion/pc05c-{rust-dto,rust-api,retry-after,poll-count}.log`。
3. `cargo build -p everything-manual -p xtask --features everything-manual/job-failpoints --bins` 与 `cargo clippy -p everything-manual -p xtask --all-targets -- -D warnings` 通过。`target/debug/xtask contracts`生成；最终 `target/debug/xtask contracts --check`、`cargo fmt --all -- --check`、`git diff --check`通过。日志 `pc05c-build-fixture.log`、`pc05c-clippy.log`、`pc05c-contracts-{generate,check}.log`。
4. apps/web完整 `node node_modules/typescript/bin/tsc --noEmit`、`node node_modules/eslint/bin/eslint.js .` 通过。`node node_modules/vitest/vitest.mjs run src/features/import/ConfirmStepPage.test.tsx src/features/jobs/RetryCountdown.test.tsx src/features/jobs/JobActivityLink.test.tsx src/features/jobs/jobDetail.test.tsx src/features/shell/shell.test.tsx src/features/shell/work-protection.test.tsx`：6文件50/50（最终日志 `pc05c-components-ready.log`）。首次原阅读测试精确GET集合未包含新增全局activity读取而失败，只增这一个预期读取，保留无N查询门槛；首次结果保留。组件visibility是受控事件，只证明状态逻辑。
5. 私有 `node var/prd-completion/pc05c-rd/run-browser.mjs <attempt> pc05c-probe1 [grep]`，headed **Chrome154.0.8037.93 / Playwright1.60.0**、端口15539：attempt1的C3通过；attempt2定向C1/C2中C2通过；attempt6 `C1 real`通过，48.30秒。最终三个不同业务场景均在与最终freeze逐字相同的131项前端上完成，不冒称整套一次通过。身份、各轮原spec、summary和业务证据见各attempt目录；准备造数明确为synthetic-ready，再走真实本机HTTP pipeline，不冒称新PDF.js准备或真实供应商质量。

保留的浏览器失败及边界：attempt1错把假CDN503当429的Retry-After分支，实际既有5xx退避2秒正确；仅修夹具为429，未改adapter。C2初次误选job-row，实际控件job-list-row，修私有定位。attempt2同一context新页面bringToFront、attempt3单页CDP最小化/恢复均持续document.visibilityState=visible；后者已去掉三个PW背景抑制参数，但没有回读窗口bounds，不能说窗口确已隐藏。严格断言失败原样保存，不模拟visibility、不再盲试。原生后台恢复标记 **NOT_VERIFIED_TOOL_LIMIT**，交独立QA用已证明可产生trusted hidden/visible的实际浏览器工具补验；其他场景不据此宣称AC006全部通过。

C1归零需要观察调度尚未推进的状态：attempt4私有lease hold不符合retry_wait领取条件，attempt5误写model_poll而未阻挡，均因真实第二次CDN读取正确失败；最终改为明确、断言已生效的私有 `tripo_poll=needs_input` 前置阻挡，保留真实第一次429与nextRunAt。这只隔离展示观察窗口，不改生产调度器。随后归零无新CDN/付费、2.1秒内只有既有详情+summary读取、不读全jobs、不发重试POST均校验；缺时间、重新安排和终态使用同一私有状态夹具。不得据此推断正常调度性能。

### 冻结与交接

最终前端 `var/prd-completion/web-snapshots/pc05c-rd-ready`，**131项**，manifest SHA **`6cb9b763b6f9cf4470e784ee19c2d50cb1cb097514f28a66a9de976738df6fac`**。与浏览器用的pc05c-probe1全部131项hash一致。相对B3有16项前端变化（API/types、auth/shell、jobs新增两组件及测试、确认页文案、theme）；Rust功能/测试5项：dto/jobs、http/jobs、http/openapi、repo/jobs、generation_requests。另cargo fmt仅将repo/mod既有item_summaries排序、storage测试既有MIGRATION_0008单行展开；逆变换哈希均准确等于B3，证明无行为变化，见format-only-proof.json。再含contracts/openapi与llmdoc/contracts，共25路径清单 `pc05c-files.json` SHA **`e9073d959858706dcbc47d45cf3cff12d2a11f7c708893dacf2bc2d9d8b16e79`**；不含本节自引用、root文档或QA作者文件。

`var/prd-completion/pc05c-rd-ready/` 已复制5物，QA不依赖target，binaries.json逐项保存源复制前/后与目标相同SHA：

| 复制物 | SHA-256 |
| --- | --- |
| everything-manual-fixture | `976ef85eb0f67498285c20eb6061e15ea62ae8c583274641eb5a73c23498b469` |
| xtask | `bb3856f7ace384b947d94beef0867af97dafde8892f1ff9e7a849d6ee539b48d` |
| jobs-lib-test | `b819f74d8d04d08b837e399f841cd7af29111c1e7927bfabd7b6a5aed9fc4e6b` |
| generation-requests-test | `7d11994dfb1bc65de579f21c6a73ddffd30e0ef43392d4989a6f30dcb90c0add` |
| jobs-recovery-test | `b01c8e615e5f8a6b6103d8aaff0fd96f41f60a3a7617860dd03be8f1f52f94e2` |

211项Rust/Cargo/迁移直接map `rust-source-manifest.json` SHA **`78c1372390325c25d65d5c4afcea6ee8666614a4670f69838229bf88a0a96c3d`**；binaries.json SHA **`9b35c2bfc514469397c4700a5ded7986b07d60ebf215478517406de7dd6ee872`**。机器交接见handoff.json。fixture启用job-failpoints、不含embedded，普通发行候选由root另行冻结构建。

**RD_READY：生产/Cargo/浏览器测试停止，15539无监听，自有临时实例清理。** 没有操作用户预览、真实配置、官方样本、真实供应商或费用；trace/HAR/video关闭。独立QA仍须签本卡，尤其原生隐藏恢复；不代签最终跨浏览器、性能、Linux或真实内容验收。


## QA15 · BUG-PCF-001/002 最小修复 RD_READY（2026-10-02，待独立复验）

依据原 api-settings PRD §6.3 UI-AS-003/AC-AS-006/007/014，以及 prd-completion PRD/UI revision 2 §10.5、§14.5；本次不变更需求。QA14 原完整 FAIL 与原 source freeze 保留，PC06/B 的最终复验由 root/独立 QA 调度。

- **BUG-PCF-001**：`ProviderSettingsForm.tsx` 增加仅作用目标卡的 `setRestore`。进入恢复立即将该卡 `apiKey` 清空、动作回 `keep`；取消用既有 `editor(data.saved[name])` 重建该卡，恢复最近服务端已保存 URL/model、问题模型遮蔽与明确清空标记，取消不复活新 key。另一卡的编辑、新 key 和字段错误保持；目标卡旧字段错误清理，恢复区/取消后 URL 输入有可见焦点，提示「已取消恢复；如需替换密钥，请重新输入」。不请求部署值，不即时保存。
- 服务端拒绝或网络结果未知时保留仍存在的编辑和另一卡新 key；恢复区继续展示错误、可取消，已按恢复动作清掉的 key 不再恢复。显式重读仍只有 GET 成功才替换编辑；PUT 成功直接使用完整响应清 key/dirty，随后独立 status GET 失败不复活 key。未改 CAS、读写错误路径、401秘密清除或共享离开保护。
- **BUG-PCF-002**：仅 `redaction_surface.rs` 的显式豁免项从无写 wrapper `claim_next` 精准改为实际 SQL 所在 `claim_next_for_job`，清单仍10项；理由明确只写系统 worker 身份、状态/epoch/时间与 NULL，job_id 仅候选 WHERE。不扩大扫描豁免，也不向生产加入无效脱敏调用；执行器生产文件未改。
- `ProviderSettingsForm.test.tsx` 按合同纠正两项同卡秘密复活的旧预期，保持替换/清除、另一卡输入、恢复拒绝可见焦点、masked null 不能隐式保存等门槛；新增网络未知、确认丢弃后 GET 失败/成功与 revision 更新、PUT 成功后 status GET 失败边界。测试均为虚构凭据、组件网络边界响应，不称为真实浏览器/供应商结果。原 AS-QA-09 未改，独立 PC06 E2E 的预期修订由 QA 另行拥有，未计入 RD 改动。

### 实际命令与结果

在 `apps/web` 执行：

1. `node node_modules/vitest/vitest.mjs run src/features/settings/ProviderSettingsForm.test.tsx src/features/settings/model-guard.test.ts src/features/shell/work-protection.test.tsx`：首轮3文件20项通过（2.65秒）；最终3文件20项通过（2.18秒），日志 `var/prd-completion/qa15-rd-fix/components-{attempt1,ready}.log`。
2. `node node_modules/typescript/bin/tsc --noEmit`：首轮仅新增测试的 Testing Library `getByRole` 不接受 Playwright 的 `exact` 选项而失败；删除该无效选项（字符串 accessible name 仍精确匹配），最终 exit0。保留 `typecheck.log` 原失败及 `typecheck-attempt2.log` 成功，不降低断言。
3. `node node_modules/eslint/bin/eslint.js src/features/settings/ProviderSettingsForm.tsx src/features/settings/ProviderSettingsForm.test.tsx --max-warnings=0`：最终 exit0，`lint-ready.log`；改动三源码/测试文件 `git diff --check` exit0。

**未运行 Cargo、浏览器、fixture build 或普通发行构建；Rust 两项结构守卫尚待 root 唯一 Cargo 校验，不凭静态复核签 PASS。** root 将统一冻结、新完整 check/构建，再交独立 QA 运行 AS09/PC06/B 受影响路径及最终矩阵。没有操作用户预览8080/5173、配置、真实秘密、官方样本、真实供应商或费用。

代码身份（完整 before/after 与精确 diff 在 `var/prd-completion/qa15-rd-fix/`；该目录不是全源冻结）：

| 文件 | SHA-256 |
| --- | --- |
| `apps/web/src/features/settings/ProviderSettingsForm.tsx` | `15d788d110457001be5b31924f3bd3f089bcdecebd6852c475c5087cc13eed50` |
| `apps/web/src/features/settings/ProviderSettingsForm.test.tsx` | `58fc1a3da44bd11186493d63dbdf1f289fce599f7b94f451e30a0d2ff805d51f` |
| `crates/server/tests/redaction_surface.rs` | `33cbe2feac26558981cd5a2f6dbd4db9a60f83ee5f8b0a20447a7ccc2d20f172` |

**RD_READY：上述三文件及本 implementation 交付后停止生产/测试写入。** 未改 PRD、state、QA 报告或原 E2E；不自行生成 full freeze/二进制，由 root 接续。

## QA15 · BUG-PCF-003 阅读版人工修订 · RD_READY（2026-10-02，待独立复验）

依据原 web-mvp REQ-034/035/036、AC-054/055/057、UI-051/055，以及 ADR-030：人工文本保存在覆盖层，供应商事实和出处保留供对照；发布同时冻结两者。本次修复人工修订已保存并进入 manifest、阅读页仍只呈现供应商文本的闭环遗漏。未变更 PRD、验收阈值、存储、后端或 manifest 格式；只读独立审计为 `var/prd-completion/release-overlay-readonly-audit.md`。

### 实现与范围

- `features/manual/release-view.ts` 为发布阅读页派生显示模型：只解析选定 release 自己的 knowledge/review，逐实体 ID 覆盖 Part name/description、Step title/orderedActions、Spec label/value。逐字段 `??` 保留合法空字符串和空数组，未修订字段回退；不会用整个 userEdited 对象覆盖实体，也不会从当前草稿或最新版本补内容。
- `ReleaseReaderPage.tsx` 的主条目、部件引用按钮、部件选择提示、仅文本摘要与当前步骤全部消费同一有效文本。各类已修订条目显示「已修订（人工）」与可键盘展开的「查看原文本」；复用原文组件的 44px summary 样式。原部件/步骤出处保留，规格使用同一 EvidenceLinks 导航。保留 ID、evidence、partIds、safetyNotes、模型、热点和视角；没有编辑发布版的入口。
- 阅读页按 itemId/releaseId 重新挂载局部交互状态，避免 A 版的选择提示或出处状态带入 B 版。原始实体作为只读 `original` 留在派生结果供比较，不改写输入知识或 review。
- 生产改动限上述两个文件。新增 `release-view.test.ts`（5项）、`ReleaseReaderPage.test.tsx`（3项）；新增独立 `tests/e2e/release-review-reading.spec.ts` 与 `playwright.release-review-reading.config.ts`（1场真实本机链路，尚未运行）。原 `manual-review.spec.ts` 和通用 `viewer/draft-view.ts` 保持交付前字节不变。

### 实际验证与尚未运行项

在 `apps/web` 执行（日志 `var/prd-completion/qa15-reader-fix/`）：

1. `node node_modules/vitest/vitest.mjs run src/features/manual/release-view.test.ts src/features/manual/ReleaseReaderPage.test.tsx`：首轮新增8/8通过，`components-attempt1.log`。覆盖六字段、部分覆盖、显式空值、未知实体/类型不符/额外字段、防输入改写、来源对照与 ID 导航、A/B 版本切换、旧版本无 overlay。
2. `node node_modules/vitest/vitest.mjs run src/features/manual/release-view.test.ts src/features/manual/ReleaseReaderPage.test.tsx src/features/manual/ReleaseDownload.test.tsx src/features/viewer/draft-view.test.ts`：4文件19/19通过（8新增+11已有），`components-ready.log`。组件模型/PDF绘制被明确替换为测试边界，只验证阅读文本和导航数据，不据此宣称真实 GPU/PDF 或浏览器布局通过。
3. `node node_modules/typescript/bin/tsc --noEmit`：exit0，`typecheck-ready.log`。`node node_modules/eslint/bin/eslint.js src/features/manual/release-view.ts src/features/manual/release-view.test.ts src/features/manual/ReleaseReaderPage.tsx src/features/manual/ReleaseReaderPage.test.tsx tests/e2e/release-review-reading.spec.ts playwright.release-review-reading.config.ts --max-warnings=0`：exit0，`lint-ready.log`。`git diff --check` 定向阅读页通过。
4. `node node_modules/@playwright/test/cli.js test -c playwright.release-review-reading.config.ts --list`：成功收集独立1场；只列用例，未启动浏览器或后端。配置要求显式 `EM_E2E_SERVER_BINARY`，直接使用冻结 job-failpoints 二进制、不调用 Cargo；关闭 trace/HAR/video。待 root 授予浏览器令牌后由 RD/QA实际执行，当前不计 E2E PASS。

新独立浏览器用例的准备阶段用真实本机 fixture 流水线生成草稿，以 API声明复核资格和合成热点；自身 ReadingFixture 在原始合成提取响应中明确定义同一输入第1页的两个部件，第二部件仅 description 预置修订以检查原名回退。随后三类实体六字段修订及 A/B 发布全部通过 UI。用例断言各版本修订/原文本与出处可见、键盘展开和出处返回、同一有效部件标签、仅文本回退、版本列表链接 A→B→A 且无 draft GET、原知识/热点/步骤引用不变、A manifest 实际字节及 SHA 在 B 发布后不变、原件/模型字节 SHA 不变。该用例不替代最终 T22 restart/restore、PC-01导出、375px/多浏览器或完整发布门禁。

交付前协调者静态复核发现测试作者前提错误：共享 `seedReadyPreparation` 实际只准备一页，默认 `LocalFixture.parts=pages.map` 仅一个部件，初稿对 `parts[1]` 的断言无法成立。已仅在新 spec 内定义上述独立合成供应商提取响应；其余模型/上传/任务端点仍原样转发共享本机 fixture，不更改已经存储的知识快照，不修改共享 harness。说明/操作文本域的包装 label 可能包含 textarea 正文，定位改为条目内 `^说明` / `^操作（每行一步）`；非文本域仍精确匹配。`tsconfig.json` 的 include 增加新独立配置的精确路径，确保默认 fullcheck 类型检查覆盖它。修正后 `tsc --noEmit` 和新 E2E/config 的 eslint 均 exit0，日志 `typecheck-author-fix.log`、`lint-author-fix.log`；生产源码和已通过的19项单测未变，不将静态校正称为浏览器通过。

精确 before/after SHA 与阅读页差异见 `var/prd-completion/qa15-reader-fix/{before-hashes.json,source-delta.json,ReleaseReaderPage.diff}`。本批未运行 Cargo、普通发行构建或浏览器；未操作用户预览、真实配置、官方样本、真实供应商或费用。状态、QA报告和全源冻结由 root/独立 QA 接续；BUG-PCF-003 仅标“已修复，待复验”，不代 QA 关闭。

## QA17 · BUG-PCF-004 缺供应商凭据的新购买准入 · RD_READY（2026-10-02，待编译与独立复验）

依据 web-mvp REQ-007 / AC-012，以及 api-settings AC-AS-012/015。root 的隔离实际 HTTP 复现 `var/prd-completion/no-key-submission-diagnostic/observation.json` 表明：先用假凭据取得并确认有效报价，停服后把部署环境变量名称改为不存在的名称再启动，配置代次未变；两家均未配置时新 key 提交仍 202，新增 job、snapshot 和两笔预留。复现供应商 wire/attempt 均为 0，没有证据表明已发生外部付费调用。该缺陷属于新购买准入遗漏，不是价格或幂等协议变化。

- `generation/estimate.rs` 提取既有缺配置字段计算，保留 `409 PROVIDER_NOT_CONFIGURED` 与 `details.missing` 形状、原模型安全优先级。`generation/jobs.rs` 在已有幂等重放、模型安全、pending/配置代次检查之后，在任何写入之前检查两家当前配置。有效旧报价也必须重新满足当前凭据条件；已有同 key/body 的任务回执仍先返回。
- `config/provider_overrides.rs` 只增加内部 active providers 借用；不改变通用 `ensure_available`、`ensure_generation_available` 或读取门禁。部署凭据失效与网页保存导致的代次变更分别处理。
- `jobs/control.rs` 在 retry 原有幂等重放、CAS、状态、unknown、预算门槛之后、写队列之前检查本次新付费动作的目标供应商：manual_extract 检查 Manual AI；未 accepted 的 tripo_submit 检查 Tripo。同步 Manual AI 的 accepted 不构成可轮询任务，人工重试仍检查配置。Tripo accepted 且有 task ID 可本地恢复；failed 即使残留 task ID 仍可能重新提交，必须检查配置；accepted 但无 ID 的既有处理器会转 unknown、不重购，所以不套新增购买门槛。manual_merge、assemble、poll、download 等不加双家缺钥限制。
- `authorizeReplacement` 明确授权再次购买，因此在原字段、取消、预算与模型门槛通过后、改写 unknown attempt 或队列之前，检查该分支供应商配置。另一家缺钥不新增阻断；recordNoTask 与 attachRemoteTask 的现有核查/查询语义不变。GET 任务详情仍 200，复用同一 retry 缺配置判据提供 `providerNotConfigured` 提示。
- 新增 5 个 `generation_requests.rs::pcf004_*` HTTP 集成测试，覆盖 26 个隔离状态组合：三种缺钥的新 job/新 estimate 拒绝且报价完全不变；三种缺钥后原 job 同键回放与冲突；12 种目标供应商/本地修复/accepted 恢复重试（含 failed 残留 task ID 负例与 accepted 无 ID 不新增购买门槛正例）；两种付费 retry 原键重放；六种 replacement 缺目标供应商或只缺另一家。拒绝路径对比任务详情/ETag 与 quotes、jobs、stages、snapshots、ledger、attempts、idempotency、audit 计数；unknown 账务和旧 attempt 不变。测试只启动进程内 HTTP Router 与真实临时 SQLite，不启动 worker，不以此宣称实际 provider wire 已验收。

源码与测试先在 `var/prd-completion/qa17-api-fix/candidate/` 暂存，root 明确 GO 后按逐文件 before SHA 校验再写 canonical。最小差异与身份记录：`candidate.patch`、`source-manifest.json`。原有 generation_requests 测试字节完整保留，新增部分只追加；没有改 frontend、schema、PRD、state 或 QA 报告。

实际静态验证：六个候选 Rust 文件的 `rustfmt --edition 2024 --config skip_children=true` exit 0；canonical 定向 `git diff --check` exit 0。**RD 未运行 Cargo、构建、浏览器或任何供应商请求；新测试仍待 root 唯一 Cargo 调度编译/执行，不签 PASS。** 建议先执行 `cargo test -p everything-manual --test generation_requests pcf004_`，随后完整 generation_requests、pipeline、api_settings_qa 与既有模型安全/恢复门禁，由 root 记录真实结果和最终 source freeze。真实 HTTP 缺钥复现应重跑验证 409、数据库零增量、loopback fixture 零 wire；retry/replacement 可据同样合成状态独立核对。

### BUG-PCF-004 首轮测试作者修正：预留列表顺序

root 首轮执行 `cargo test -p everything-manual --test generation_requests pcf004_`，结果为 4 通过、1 失败；原日志保留于 `var/prd-completion/qa17-api-focused/new-regressions.log`，完整副本与 SHA 另存 `var/prd-completion/qa17-api-fix/replay-order-author-fix/`。失败仅为新 replay 测试把 reservations 的数组位置也视为合同：首次创建按 Tripo/Manual AI 构造，仓储回读按 provider 排序，逐条内容和其他任务字段相同。核对 `contracts/openapi.json` 的 JobDto/ReservationDto、DTO 注释和 `llmdoc/contracts.md` §4，合同要求分列预留与原任务幂等回读，没有规定数组排序。

本次仅在该测试内按 `(provider, currency)` 排序后继续严格比较完整 job JSON；保留数组数量、每条全部字段（币种、预算、显示金额、状态）与其余任务字段、原 key 重放标记、异体冲突、八表计数和完整报价不变断言。没有删除或忽略预算字段，没有改生产响应顺序。RD 仅运行 rustfmt 静态检查与定向 diff 检查，不运行 Cargo；修正后的真实结果待 root 复跑，当前不记 PASS。精确作者 delta 和停写交接见该私有目录。


## QA18 · BUG-PCF-005 资料库行操作目标宽度 · RD_READY（2026-10-02，待独立复验）

依据 prd-completion PRD revision 2 的 PC03B UI-005–008 与 PC05A UI-003。QA17 原矩阵的 ROW1/ROW2 实际失败维度是宽度：历史链接为 40×44px，主要操作为 84×44px；既有 `.workflow-actions a` 已提供 44px 最小高度，未被 theme 的 40px 规则覆盖。原失败及完整矩阵结果保留，不改验收阈值或测试断言。

本次只在 `apps/web/src/styles.css` 增加 `.item-row__actions .workflow-actions > a { min-width: 44px; }` 及一行说明，限定资料库行内工作流链接。保留既有高度、字号、flex 换行和窄屏布局，不扩大全站按钮规则。canonical CSS 与先前只读复核的私有候选逐字一致；精确 before/after SHA、两文件差异与静态解析结果见 `var/prd-completion/qa18-library-action-fix/applied/`。

RD 仅使用已缓存 PostCSS 解析完整 CSS 并检查定向差异；没有运行浏览器、Cargo、构建或新增机械测试。真实 375px 与桌面窗口下的 ROW1/ROW2、PC05A 操作尺寸及不溢出仍由独立 QA 在新 freeze 上复验，当前不代签浏览器 PASS。除本 CSS 与 implementation 记录外未修改生产、测试、PRD 或 state；交付后停止写入。
