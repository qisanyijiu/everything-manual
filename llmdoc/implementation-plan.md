# 实施路径与小任务卡

历史基线：1.0 · 2026-09-11。第 1～5 节保留初始工程规划，任务状态以各需求的 state/QA 和当前交付记录为准，不应由早期“待执行”描述推断现有实现状态。

新增规划：2026-10-04，[第 6 节 UI 反馈第 3 和第 6 项待办](#6-ui-反馈第-3-和第-6-项待办)已并入本文件，包含两份 PRD、任务依赖和后续 agent 入口。**2026-10-05 视觉 VS-01～05 已全部接受，QA6 全部12AC与正式发行门禁PASS；社区按用户决定取消本轮实施。** 用户后续新增 E2E-01 的当前来源、新包与状态见[完整复验记录](requirements/ui-feedback-3-6/e2e-delivery-2026-10-05.md)；历史 QA6 产物、范围与部署前提见[生产交付记录](requirements/ui-feedback-3-6/production-delivery-2026-10-05.md)；其他需求状态不变。

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

E04 中的“同实例多用户与社区贡献”现由第 6 节 CI-00～06 细化；公网 SaaS、跨实例与多租户仍属远期范围，不因本轮 PRD 自动启动。

## 6. UI 反馈第 3 和第 6 项待办

日期：2026-10-04。来源：[用户需求与反馈边界](requirements/ui-feedback-3-6/request.md)。用户明确要求把 PRD 加入已有规划，方便 agent 接续。本节是新增事项的统一 todo-list；既有 T01～T23 和其他需求的 QA 状态不变。

### 6.1 产物与状态

- [x] 输出[视觉风格优化 PRD](requirements/ui-feedback-3-6/visual-style-prd.md)：8 项需求、6 项交互合同、12 项验收条件。
- [x] 输出[社区与激励 PRD](requirements/ui-feedback-3-6/community-incentives-prd.md)：12 项需求、7 项交互合同、15 项验收条件。
- [x] 保存[反馈原件与附图](../artifacts/ui-feedback-20261004/README.md)、[PRD 索引](requirements/ui-feedback-3-6/prd.md)和[协调状态](requirements/ui-feedback-3-6/state.yaml)。
- [x] 完成视觉方案复核、实施和独立 QA。2026-10-05 VS-01～05全部接受；[QA6](requirements/ui-feedback-3-6/vs05-qa-report.md)全部12AC与[正式交付](requirements/ui-feedback-3-6/production-delivery-2026-10-05.md)通过。
- [ ] 完成社区架构/交互合同、实施和独立 QA。**用户已取消本轮实施**；PRD保留，未经重新授权不派发。

原始排期为P1视觉→P2社区；后续用户决定只实施视觉，社区取消。视觉已依赖验收顺序完成并保持一个生产代码写入者。若未来重新授权社区，须先冻结CI-00，不能以已完成的视觉页面代替多用户权限实现。

### 6.2 视觉风格任务

所有任务依据[视觉 PRD 修订 1](requirements/ui-feedback-3-6/visual-style-prd.md)，AC 前缀为 `AC-VS-`。每片结果写入本需求的 `visual-implementation.md` 和 `visual-qa-report.md`，由实际实施与 QA 时创建。

- [x] **VS-01 · 设计基线复核，P1。**（2026-10-04 设计门禁完成：`design/vs-01/` 六件产物 + UI 修订 1 冻结；运行 AC 由 VS-05 验收） 输入：原反馈第三节、视觉 PRD、现有主题/阅读器/表单；交付三类样页、设计变量与组件全状态说明，并复核全部 REQ/UI/AC 映射。允许修改本 PRD、设计附件及必要展示样页，不改变业务逻辑。完成条件：说明样页如何覆盖 AC-001/002/012，并冻结 UI 修订；本卡是设计门禁，不宣称运行验收通过。依赖：无。
- [x] **VS-02 · 主题、外壳与资料库，P1。**（2026-10-04 接受：QA 回合 1 发现 BUG-VS02-001 → RD 修复 → [回合 2 PASS](requirements/ui-feedback-3-6/visual-qa-report.md)，AC-VS-001/002/003/004/009/010 通过） 依赖 VS-01；修改主题、共享组件、导航、登录、资料库/详情展示。验收：AC-001～004、009、010 的对应页面，现有搜索/分页/路由可用，失败态无假计数。允许前端样式/结构和定向测试，不改后端/生成/发布规则。
- [x] **VS-03 · 阅读器、复核与独立 HTML，P1。**（2026-10-05 接受：QA 回合 3 PASS，0 缺陷；非阻断 N-VS03-1～5 留 VS-05 处置） 依赖 VS-02 验收；修改 viewer/manual/原件与 standalone viewer 展示层。验收：AC-005/006/008/009/010/011，热点/步骤/出处与焦点回归、新导出断网阅读、历史发布哈希不变。候选和确认热点不能视觉混同。
- [x] **VS-04 · 上传、任务、设置和状态，P1。**（2026-10-05 接受：独立 QA4 FAIL → 两处移动触控尺寸修复 → [QA5 PASS](requirements/ui-feedback-3-6/vs04-qa-report.md)）依赖 VS-02 验收；修改 import/jobs/settings/版本列表展示。验收：AC-007/008/009/010，包括未保存保护、报价过期、确认未完成、提交未知、412/422 和密钥保护；样式交互不增加供应商请求。
- [x] **VS-05 · 整体视觉验收，P1。**（2026-10-05接受：[QA6](requirements/ui-feedback-3-6/vs05-qa-report.md)全部12AC PASS；46项去重独立接受、141普通浏览器全回归、工程/正式Mac及Linux运行/包装修订2独立审计全部通过，[生产包及部署前提](requirements/ui-feedback-3-6/production-delivery-2026-10-05.md)已交付）依赖 VS-03/04 验收；运行类型/lint/受影响组件与浏览器检查，核对 375/768/1024/1440、真实 200% 缩放、键盘、离线 HTML、页面截图和 gzip 增量。5秒识别研究未实施，独立列为未验证，没有捏造用户数据；本轮未执行公网部署。
- [x] **E2E-01 · 全量复验与缺陷修复，P1。**（2026-10-05 QA7 full独立验收PASS：209不同用例、兼容性/CLI/GPU及新版双平台发行与部署包闭环；旧QA6记录保留）普通/PC/原生恢复共 209 个不同用例、Edge/Firefox 兼容性、Linux CLI 与真实 GPU 300 秒性能，以及修改后的正式 Mac/Linux 包。原失败保留，新版包已完成独立验收；见[本轮记录](requirements/ui-feedback-3-6/e2e-delivery-2026-10-05.md)。

### 6.3 社区与激励任务

所有任务依据[社区 PRD 修订 1](requirements/ui-feedback-3-6/community-incentives-prd.md)，AC 前缀为 `AC-CI-`。每片结果写入 `community-implementation.md` 和 `community-qa-report.md`，不修改旧需求的通过状态。

> **2026-10-04 用户决定：本轮不实施社区与激励部分（CI-00～06 暂停）**；PRD 与本节文档保留，未经用户重新授权不派发。视觉流 VS-01～VS-05 继续，见 [state.yaml](requirements/ui-feedback-3-6/state.yaml)。

- [ ] **CI-00 · 架构、权限和交互合同，P2。** 依赖本 PRD；复核全部成员页面及状态，形成完整路由/资产权限表、旧管理员迁移、对象归属、共享可见性、审核状态机、幂等事件/积分规则和接口草案；新增有状态的架构决策，明确何时替代单管理员假设。允许文档与设计附件；完成条件：产品与 UI 冻结同一修订，所有 AC 有实现/验证入口。先冻结合同，再允许成员登录。
- [ ] **CI-01 · 成员身份与资料可见性，P2。** 依赖 CI-00；实现邀请、登录/重置/停用、角色、私人/成员共享隔离、社区开关与迁移。允许 core/server、追加迁移、生成合同及成员前端。验收：AC-001/002/014/015 基础部分；直接调用旧管理、生成、导出、Range、HEAD、备份接口也不得绕权，旧资料默认私人。
- [ ] **CI-02 · 投稿、去重与审核，P2。** 依赖 CI-01 验收；实现来源和分享确认、PDF 投稿快照、重复检查、审核/撤回/撤销、共享入库与审计，持久化可重放的采纳事件。验收：AC-003～006；禁止自审、并发只有一个结果、重试不重复，不自动生成/发布/计费。奖励落账在 CI-05 复验 AC-006。
- [ ] **CI-03 · 共建资料库与贡献署名，P2。** 依赖 CI-02 验收；实现共享搜索/筛选、资料详情、原件/3D 阅读、本人贡献列表和公开范围内的档案；沿用 VS 主题合同。验收：AC-003/007/014；无模型仍可读、审核状态真实、私人稿与登录信息不外泄。
- [ ] **CI-04 · 评论、补充、举报与通知，P2。** 依赖 CI-03 验收；实现评论先审、编辑/删除/回复、举报处理、证据化补充、版本冲突、独立社区补充区和站内通知。验收：AC-008/009/013；不可改写旧 release，不向他人泄露通知。成就通知由 CI-05 接入后复验。
- [ ] **CI-05 · 成就、贡献积分和权益，P2。** 依赖 CI-04 验收；实现事件账本、有效阅读/自报使用成就、采纳计分/限额、等级、收藏夹权益、撤销/恢复和规则版本，按 PRD 幂等处理 C1 历史有效贡献。验收：AC-006/010/011/012/013；伪造/重复/重启不多发，降级不删收藏，身份不授予管理或付费权限。
- [ ] **CI-06 · 试点与整体社区验收，P2。** 依赖 CI-05 验收；执行跨角色/多会话权限、并发与断线、配额跨日、迁移备份恢复、键盘/屏宽和单二进制门禁，交付开启/关闭、审核与积分规则操作说明。完成条件：AC-CI-001～015 全部独立 QA 通过；增长指标无真实试点数据时保留待观测，不以 fixture 宣称真实增长。

### 6.4 后续 agent 接续规则

1. 先读 `requirements/ui-feedback-3-6/request.md`、`prd.md`、`state.yaml` 与本节，再按当前任务读取对应完整 PRD；先核对工作区现有改动与代码实际状态，保留无关用户改动。
2. 2026-10-05视觉VS-01～05已接受；Claude交付VS-03后中断，Codex完成VS-04/05。用户新增E2E-01接续旧QA6发行，整体当前状态与新版包以[state.yaml](requirements/ui-feedback-3-6/state.yaml)和[完整复验记录](requirements/ui-feedback-3-6/e2e-delivery-2026-10-05.md)为准，旧[分工记录](requirements/ui-feedback-3-6/codex-handoff-2026-10-05.md)/[QA6交付记录](requirements/ui-feedback-3-6/production-delivery-2026-10-05.md)保留历史。不重复派发完成卡；目标机按新版包完成前置校验；社区不进入CI-00。
3. 依项目 PM→UI→RD→QA 门禁推进，不重复从 T00 开始，不把历史构建结果当本轮通过。协调者在 `streams` 分别记录两条工作线的 UI 修订；只派当前已冻结且依赖通过的切片。
4. 实际编码前给出本卡文件范围和 AC；实施记录与独立 QA 报告保存实际命令、版本、截图/日志索引、失败和未覆盖项。禁止付费调用作为默认验证路径；采用隔离 fixture，不修改用户当前资料库。
5. 仅在卡级验收完成后勾选相应待办，同时从 `pending_tasks` 移除，并写入 `accepted_tasks`/`qa_history`；产品功能未验收前保留未勾选。进入社区时重置对应阶段与当前 QA 状态，不把视觉通过复用为社区通过。
6. 任一PRD或产品修订都同步索引/state，并将受影响任务标为需复验。每条已授权实施流须有当前修订的全量结论才可标done；本轮视觉QA6全量通过，社区为用户取消，不能将其标为功能已实现。重新授权社区时另走CI-00～06门禁，不复用视觉PASS。
