# 实施路径与小任务卡

版本：1.0 · 2026-09-11 · 状态：待执行。**本文件中的源码路径、脚本和测试目标是要创建的交付物，不是已存在／已通过的命令。** 当前只有文档与 Claude Code 配置。

## 1. 如何派发给能力较弱的 Agent

先 `/team web-mvp`：PM 将 [request](requirements/web-mvp/request.md) 写成详细 PRD，UI 补进同一 PRD。以下是工程基线任务卡，PM 必须映射到稳定 REQ/AC，UI 为页面与状态给 UI ID；不要让 RD 自行猜产品行为。

每次只派一个卡或卡内更小的闭环，包含“输入、允许文件、具体动作、必测项、交接路径”。依赖通过后才能开始。RD 先读现状、做最小实现、实际验证，写 implementation 和 llmdoc；QA 独立验收。卡级 PASS 不代表产品 PASS，末尾另有真实供应商／单二进制验收。

统一停止条件：需求解释不清 → PM；交互缺失 → UI；权限／付费预算／外部凭据缺失 → 协调者记录 BLOCKED；不能用 mock 冒充解决。开发期间允许只做已确定的小切片，不能一次“重写整个项目”。

## 2. 里程碑

| 阶段 | 任务 | 看得见的交付与门禁 |
| --- | --- | --- |
| M0 协作和基础 | T00–T05 | PRD+UI、锁定版本、最小 React 页面从 Rust 二进制加载、可测认证和数据库 |
| M1 数据入口 | T06–T11 | 资料落盘、PDF 可恢复准备、任务持久化、输入／预算快照 |
| M2 生成闭环 | T12–T15 | 用真实适配器对本地 HTTP fixture 跑通知识+模型→草稿；崩溃不重复购买 |
| M3 用户闭环 | T16–T20 | 向导、任务、阅读器、热点、发布、导出备份 |
| M4 交付 | T21–T23 | 故障与安全回归、干净环境单文件启动、授权真实 API 和目标平台验收 |

不按固定“几天完工”承诺；M0 与一次真实资料垂直验证后再依据卡耗时估算。不要等到最后才验证静态资源能否内嵌，T01 必须先证明最小二进制模式。

## 3. 任务卡

### T00 — PM 明细 PRD与 UI 设计门禁

- 依赖：无。执行者 PM→UI，非 RD。
- 输入：request、architecture、contracts、prd 模板。
- 产物：`llmdoc/requirements/web-mvp/prd.md`；REQ/AC、页面路由、组件、交互状态、键盘／移动端降级、费用告知、错误与恢复、必选发布平台、任务映射。
- 验证：每个必需需求至少一个可测 AC，交互需求对应 UI ID；未决问题和默认假设显式；UI 补同一个 PRD。PM/UI 不提前编生产代码。
- 门禁：PM_READY + UI_READY 的同一 PRD 修订，协调者保存版本。

### T01 — 工作区、合同生成与最小单二进制

- 依赖：T00。
- 允许范围：根 Cargo/npm 工程配置、`.cargo/config.toml`、`rust-toolchain.toml`、`crates/core`、server 最小入口、`apps/web` 最小页面、`xtask`、`contracts`。
- 实现：按架构建 workspace；固定实际可用版本与锁文件；Rust HTTP health 返回 JSON；创建最小 DTO、导出 OpenAPI→TS；定义 cargo xtask 别名；React 页面访问 health；生产 embedded-ui 编译缺 dist 必须报错，普通后端单测不要求 dist。
- 验证：`cargo test -p everything-manual --test bootstrap`；`npm --prefix apps/web run typecheck`；`cargo xtask contracts --check`；`cargo xtask dist --target <本机target>` 后在空目录运行 `cargo xtask smoke-bootstrap --binary <绝对路径>`，仅验证页面、静态资源和 health，不要求后续卡尚未实现的认证／生成功能。
- 交接：锁定版本／MSRV、构建命令、最小包路径与哈希、实际响应；没有这些证据不进入大功能开发。

### T02 — CLI、配置、日志与数据目录锁

- 依赖：T01。
- 允许范围：server 的 config/cli/logging/启动、集成测试 `config_cli.rs`、环境配置示例。
- 实现：`init/serve/check/backup/restore` 的参数分派（backup/restore 先明确未实现返回非零，T20 完成）；默认 loopback；配置优先级 CLI 非密钥项 > 环境变量 > TOML > 默认；密码交互或受限文件；数据目录排他锁；结构化日志脱敏；未知配置报错而不忽略。
- 验证：`cargo test -p everything-manual --test config_cli`，覆盖缺密钥不启 mock、重复进程锁失败、错误路径、敏感字段不进入日志；`check` 不调用收费 API。
- 交接：配置键和有效示例、目录权限、退出码；说明内置 TLS 证书输入与反向代理边界。

### T03 — SQLite 迁移和 Repository

- 依赖：T02。
- 允许范围：`migrations`、core 数据类型、server/storage、`tests/storage.rs`。
- 实现：建合同核心表、唯一键、外键、索引；WAL/FULL/busy_timeout；迁移内嵌与 rerun-if-changed；旧 schema 升级／新 schema 拒绝；Sqlx 使用静态 SQL 和 bind，动态 SQL 用 QueryBuilder，不拼用户字符串。
- 验证：`cargo test -p everything-manual --test storage`，空库初始化、重复迁移幂等、外键拒绝、revision CAS、回滚、数据目录重开保留数据。
- 交接：迁移版本、不变量对应测试、恢复限制；不能用运行时只存在内存的 HashMap 代替持久层。

### T04 — 认证和 API 基础

- 依赖：T03。
- 允许范围：server/http/auth/error/router、OpenAPI DTO、`tests/auth_api.rs`。
- 实现：管理员初始化、Argon2、会话哈希、登录／注销／恢复、CSRF+Origin、登录限速；统一错误／requestId；404 API 与 SPA 分离；If-Match 工具；settings 仅显示配置状态。
- 验证：`cargo test -p everything-manual --test auth_api`，无登录401、跨站修改403、登出失效、过期会话、缺If-Match428、冲突412、`/api/unknown` JSON404；生成合同不得漂移。
- 交接：前端会话和 CSRF 用法、认证测试证据；无匿名“临时后门”。

### T05 — 无费用 HTTP Fixture 测试设施

- 依赖：T01。
- 允许范围：`crates/test-support`、`tests/fixtures`、server 测试挂载、测试说明。
- 实现：本机 Tripo／Manual AI HTTP fixture，按脚本记录调用次数与请求体，支持延迟／断连／429／5xx／畸形JSON／成功；原创小 GLB、文字PDF、扫描PDF和图片，保存来源与许可。测试开关不能启用到生产默认。
- 验证：`cargo test -p everything-manual --test fixture_harness`；缺少 fixture 脚本必须失败，不返回通用成功；测试进程无真实外网调用。
- 交接：场景名、样例资产 hash、调用断言 API；后续测试必须调用真实 HTTP 适配器而非只 stub 业务结果。

### T06 — 上传和安全资产服务

- 依赖：T04、T05。
- 允许范围：server/assets/storage/http assets、`tests/assets.rs`。
- 实现：流式 multipart、magic／像素／体积校验、tmp→fsync→rename→元数据、内容去重、owner 校验、Range／HEAD／ETag；剩余空间检查、崩溃孤儿隔离；文件名不作路径。
- 验证：`cargo test -p everything-manual --test assets`，伪造类型、超大、路径穿越、未授权、重复上传、206/416/HEAD、fsync 后 DB 失败不删共享文件。
- 交接：资产状态与清理策略；不能把整个 data-dir 做静态服务。

### T07 — 物品、原始资料和版本

- 依赖：T06。
- 允许范围：core item/document/photo、server 对应服务与路由、`tests/items.rs`。
- 实现：物品增查改归档；绑定 PDF 与照片、视图选择；校验资产同物品；乐观锁；物品编辑不改旧快照，归档不破坏已发布资料。
- 验证：`cargo test -p everything-manual --test items`，空型号、错误归属、并发编辑412、同型号不同配置、归档后引用仍完整。
- 交接：完整 API 样例、错误码和前端生成类型。

### T08 — React 应用框架与基础交互

- 依赖：T04。
- 允许范围：web 路由、api、布局、基础组件、登录、空态、错误边界；`src/features/shell/shell.test.tsx`。
- 实现：按 UI PRD 建路由和布局；Typed fetch 注入 CSRF／If-Match，Query 管服务器状态；401 回登录并保留安全内部返回路由；全局通知／可访问表单／加载骨架；不调用假 API 假装业务完成。
- 验证：`npm --prefix apps/web run test -- --run src/features/shell/shell.test.tsx`、typecheck；键盘登录、错误信息关联表单、412 可恢复、不泄露 token。
- 交接：路由表和组件复用方法，UI 截图由真实浏览器产出。

### T09 — 浏览器 PDF 准备与续传

- 依赖：T07、T08。
- 允许范围：web/import/pdf、server preparations/pages、PDF vendor 构建、`tests/preparations.rs`、`tests/e2e/pdf-preparation.spec.ts`。
- 实现：上传原件后顺序提取／渲染并上传页；同版本本地 worker/fonts/cmaps/wasm；manifest 校验后封存；从服务端查询缺页续传；取消／关闭标签页明确提示；加密和超页数拒绝。
- 验证：`cargo test -p everything-manual --test preparations`；`npm --prefix apps/web run test:e2e -- pdf-preparation.spec.ts`，文字／扫描／旋转／非拉丁字体、中断后只补缺页、全部页号1-based、断外网仍加载 PDF 本地资源。
- 交接：浏览器依赖阶段与后台阶段的界限、页图坐标与尺寸、内存观测；未完成的 preparation 不可生成。

### T10 — 持久任务执行器

- 依赖：T03、T05。
- 允许范围：core 状态机、server/jobs、`tests/jobs_recovery.rs`。
- 实现：阶段 DAG、leaseEpoch、nextRunAt、限并发、退避、关机恢复；事务领取但不跨 HTTP 持事务；attempt intent/submitting/receipt 分离；支持注入测试 failpoint（仅测试构建）。
- 验证：`cargo test -p everything-manual --test jobs_recovery`，双 worker 竞争、lease 过期、旧 worker 晚到 receipt 保存但不能推进、未知提交不重购、SIGKILL 后已知 ID 继续查询。
- 交接：状态转换表对应测试与日志例子；这张卡先用 fixture 阶段，不假称 Tripo 已接通。

### T11 — 输入快照、报价、预算与幂等

- 依赖：T07、T09、T10。
- 允许范围：server generation/estimate/ledger、core snapshot、`tests/generation_requests.rs`。
- 实现：校验 ready 准备+至少两有效视图、报价快照、云端资料告知确认、冻结输入、事务预留+job；幂等重放与不同 body 冲突；精确小数换算；余额 unknown 不自动释放。
- 验证：`cargo test -p everything-manual --test generation_requests`，重复20次同键只建1job、过期报价、版本变动、无预算／缺配置、并发额度竞争、decimal 转换、事务中断不留半笔预留。
- 交接：报价与真实账单区别、支持模型价格配置；不写无来源“必定只花某金额”。

### T12 — Tripo v3 HTTP 适配器

- 依赖：T05、T11。
- 允许范围：server/providers/tripo、provider DTO、`tests/tripo_contract.rs`。
- 实现：图片 file 上传、view-key generation、查询任务、状态归一化、响应 code 校验；明确连接／整体超时；未返回 ID 的付费 POST 禁止通用自动重试中间件；记录计费与错误。
- 验证：`cargo test -p everything-manual --test tripo_contract`，断言 endpoint/header/multipart/body、200非零code、success缺模型、未知枚举、429、POST已接受后断连；真实收费调用留到 T23。
- 交接：官方文档日期、请求样例、脱敏响应映射；不混入 v2 字段。

### T13 — 模型下载、校验与不可变版本

- 依赖：T06、T12。
- 允许范围：server/assets/glb/download、model revisions、`tests/model_assets.rs`。
- 实现：下载独立 client 无 bearer 默认头；HTTPS 域／实际连接 IP／每跳重定向验证；流式大小与 hash；GLB 结构／内嵌资源／扩展／面数／贴图限制；成功本地落盘后生成 revision。
- 验证：`cargo test -p everything-manual --test model_assets`，DNS重绑定模拟、重定向私网、签名 URL 失效后重新查询、截断GLB、超面数、外链资源、磁盘满、不支持扩展；不重新购买模型来修复下载失败。
- 交接：实际支持 GLB 能力清单、非支持资产的 needs_input 提示、原始模型保留策略。

### T14 — 说明书 AI 适配与证据校验

- 依赖：T05、T09、T11。
- 允许范围：server/providers/manual_ai、提取schema/提示词、core knowledge、`tests/manual_ai_contract.rs`。
- 实现：Responses HTTP、页批次与 token/预算限制、严格结构输出解析、refusal/incomplete处理；全部页覆盖记录；本地合并去重与冲突；引用只允许输入页，防提示注入越权。
- 验证：`cargo test -p everything-manual --test manual_ai_contract`，扫描页图、引用不存在页、漏页、拒答、截断、畸形 JSON、相互冲突事实、超预算不再请求；断言真实适配器所发 HTTP 格式。
- 交接：model/schema/promptVersion、覆盖率、不确定项与隐私限制；真实模型效果待 T23。

### T15 — 两条分支组装草稿

- 依赖：T10、T13、T14。
- 允许范围：server/jobs/pipeline、draft服务、`tests/pipeline.rs`。
- 实现：模型与知识分支可独立完成；checkpoint恢复；仅必要分支重试；assemble_draft 幂等；部分成功展示；job succeeded 与 draft needs_review 分离；cancel/reconcile 与审计。
- 验证：`cargo test -p everything-manual --test pipeline`，成功全链路、模型成功知识失败仅重提取、重启不重复draft、unknown需管理员对账、取消后不发新付费步骤、草稿未自动发布。
- 交接：HTTP fixture 下端到端请求／费用／次数证据；可由 API 完整创建草稿，无需先等 UI。

### T16 — 资料库和新建向导

- 依赖：T08、T09、T11、T15。
- 允许范围：web/library/import、`tests/e2e/import-flow.spec.ts`。
- 实现：库列表、型号表单、原件／视图上传、准备页、预算／隐私确认、生成按钮；前端不判断远端成功；防重点击同时依赖服务端幂等；刷新恢复工作。
- 验证：`npm --prefix apps/web run test:e2e -- import-flow.spec.ts`，正常向导、缺front、错误型号、上传失败重试、预算变动、返回上一步保留资料、窄屏键盘操作。
- 交接：UI/AC截图与实际 API 联调证据，不止静态 mock 页面。

### T17 — 任务中心与可行动错误

- 依赖：T15、T16。
- 允许范围：web/jobs、`tests/e2e/job-recovery.spec.ts`。
- 实现：阶段状态、已消耗／预留、错误详情、取消、分支重试、unknown对账；轮询可见页2秒／后台15秒，终态停止；网络错不误报业务失败；刷新和重启后读数据库状态。
- 验证：`npm --prefix apps/web run test:e2e -- job-recovery.spec.ts`，浏览器关闭后服务器继续、服务重启恢复、unknown无盲目“重试”按钮、重复操作不多收费、错误摘要可读。
- 交接：每种状态的UI及下一步，不用虚假总进度100%掩盖待校准。

### T18 — GLB 阅读器和资源恢复

- 依赖：T08、T13。
- 允许范围：web/viewer、`src/features/viewer/coordinates.test.ts`、`tests/e2e/viewer.spec.ts`。
- 实现：懒加载 R3F、OrbitControls、透明／基础背景、fit/reset、loading/error、真实canvas contextlost/restored、清理 geometry/material/texture；文本阅读不依赖 WebGL成功；确定asset-root坐标适配层。
- 验证：Vitest 目标 `coordinates.test.ts`，Playwright 目标 `viewer.spec.ts`；不对称测试模型在不同旋转／缩放下局部点一致；forceContextLoss恢复；换模型无旧资源／热点串入。
- 交接：模型预算实测、浏览器支持范围；不能用截图或 `<model-viewer>` 占位冒充约定阅读器。

### T19 — 热点校准、步骤联动与发布

- 依赖：T15、T18。
- 允许范围：web/manual/viewer binding、server drafts/releases、`tests/publishing.rs`、`tests/e2e/manual-review.spec.ts`。
- 实现：部件／步骤／PDF原文联动；raycast校准保存局部点；视角保存；确认知识；发布事务冻结manifest；重生成新模型后旧绑定stale，旧发布版保持可读。
- 验证：`cargo test -p everything-manual --test publishing`；Playwright `manual-review.spec.ts`，缩放旋转后点击位置仍正确、拖动不建点、引用跳到正确1-based页、并发412、不完整／stale发布422、发布后修改draft不改release。
- 交接：事实审核与几何校准两种确认的区别、所有必选交互 AC。

### T20 — 导出、备份、升级与恢复

- 依赖：T19。
- 允许范围：server export/backup/restore/migration guard、xtask检查、`tests/backup_restore.rs`。
- 实现：只导出授权release关联资产与manifest；备份要求停服／独占锁，处理WAL并包含引用blob；restore仅新空目录、校验hash与schema；升级前备份，禁止旧程序打开新schema。
- 验证：`cargo test -p everything-manual --test backup_restore`，运行中拒绝危险直接备份、损坏blob、非空恢复目录拒绝、新空目录恢复后模型／PDF／发布版可用；导出无密钥／绝对路径。
- 交接：管理员操作指南、恢复演练报告；不是只生成一个ZIP就称备份完成。

### T21 — 产品回归、安全与故障矩阵

- 依赖：T16、T17、T19、T20。
- 允许范围：tests、必要缺陷修复（经QA→RD）、CI、validation 文档。
- 实现：完整 fixture E2E、浏览器矩阵、故障注入、上传／SSRF／CSRF／路径／输入注入回归；QA 独立执行，发现问题走缺陷回路。
- 验证：`cargo xtask check`、`npm --prefix apps/web run test:e2e`；覆盖 [验收矩阵](validation-release.md)，所有必选测试无静默skip、无0测试假通过。
- 交接：PRD版本→AC→测试／截图／缺陷闭环；明确真实服务未验的边界，不签发完整发布PASS。

### T22 — 多平台单二进制发布

- 依赖：T21。
- 允许范围：xtask/dist/smoke、CI release矩阵、构建脚本、LICENSE清单、运维文档。
- 实现：先目标平台原生构建，再逐平台扩展；锁文件构建、web dist与vendor内嵌、SQLite bundled、TLS依赖检查、校验和、版本信息；独立 data-dir；不得把构建环境偷偷带进运行包。
- 验证：`cargo xtask dist --target <target>`、`cargo xtask smoke --binary <绝对路径>`；正式binary从合法测试备份恢复资料，不连接本机Provider fixture；从无源码／dist／Node／Python环境启动，PDF字体worker、GLB、API、路由刷新、Range都可用；断外部AI仍读本地。完整fixture生成E2E在T21测试构建完成，正式生成在T23授权执行。
- 交接：每目标binary hash、系统动态依赖清单、安装／启动／升级／回滚说明。某平台未跑不能贴该平台支持标签。

### T23 — 授权真实链路与最终 QA

- 依赖：T22。
- 输入：用户允许发送的真实型号资料、Tripo/ManualAI凭据、明确一次生成及可选重试预算；不把凭据写入报告。
- 实现：按明确预算运行真实适配器；从多视图／说明书到本地GLB／知识草稿／人工校准／发布；记录实际费用、数据质量与模型约束差异；无必要不得重复购买。
- 验证：实现后运行显式 `cargo xtask test-live --case <案例> --budget-file <受限文件>`；QA 使用正式二进制完成全量AC、目标平台冷启动与恢复。真实失败按问题分别回RD/PM或外部阻塞。
- 交接：当前PRD修订的全量QA PASS、预算与实际摘要、发布包、已知非阻断限制。缺凭据／预算／平台环境写BLOCKED，不把T21 fixture PASS改名为最终PASS。

## 4. 每张卡的完成定义

1. 实现与当前 PRD／合同一致；生成OpenAPI和TS类型无漂移。
2. 真正执行指定目标测试且测试数大于0；新增行为有正常＋至少一个错误／恢复测试。失败命令不藏在 `|| true` 后。
3. RD 在 `implementation.md` 记录任务、修改文件、实际命令及结果、已知限制、llmdoc更新，不发明测试截图。
4. QA 将任务／AC／PRD修订／回合／报告绑定；所有必选项通过，无未关闭验收缺陷。
5. 协调者更新 state；下一张卡只消费已验证产物。需求变更导致受影响卡重新验收。

## 5. 后续增强，不混入 MVP

- E01 自动热点候选：冻结模型和相机截图→AI二维点→浏览器raycast→确认，先验证遮挡／歧义再谈自动确认。
- E02 分件与机械交互：独立PRD，来源证明部件拓扑、转轴、限位和顺序；Tripo分割不等于机械语义。
- E03 无浏览器资料准备：评估PDFium／其他原生渲染依赖的静态链接和许可证；若破坏单文件边界，需用户选择，不能偷偷添容器服务。
- E04 多用户／公网SaaS：先设计租户隔离、对象存储、配额和审计，可能迁移数据库／队列，另走架构决策。
