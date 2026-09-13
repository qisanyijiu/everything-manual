# web-mvp PRD

状态：pm_ready（修订 2 已裁决 D-1–D-4；第 6 章需 UI 按 §9.2 清单同步后冻结为 ui_revision 2） · 修订：2 · 更新日期：2026-09-12 · 作者：PM（§6 由 UI 补充）

说明：本文是任务卡 T00 的 PM 产物，扩展自 [request.md](request.md)，与 [architecture.md](../../architecture.md)、[contracts.md](../../contracts.md)、[decisions.md](../../decisions.md) 保持一致。文中所有命令、路由、表名都是**待实现合同**，不是已通过结果。第 6 章由 UI 角色在同一修订号内补写，PM 不提前定稿交互；PM 不编写生产代码。

## 1. 需求依据与目标

### 1.1 需求来源

| 来源 | 内容 | 采用方式 |
| --- | --- | --- |
| 用户明确要求（request.md，2026-09-11） | 「万物说明书」：输入物品与型号、说明书、多视图，经 Tripo 生成 3D 并形成可交互说明书；改为前后端分离网站，React 前端 + Rust 后端，最终打包为一个二进制；方案要能指导较弱 agent 编码 | 全部进入本期范围 |
| 用户明确要求 | 项目内定义 PM/UI/RD/QA 四角色与 llmdoc 持久知识约定 | 属协作配置，已在 `llmdoc/collaboration.md`；PRD 只按流程产出 |
| 技术基线（architecture.md / contracts.md / ADR-001～009） | React 19 + R3F 9、Rust Axum + SQLx + SQLite、GLB 为运行资产、浏览器完成 PDF 准备、单管理员自托管、submission_unknown 不盲重购 | 作为固定约束，不重新选型 |
| 旧 macOS 文档（docs/everything-manual-implementation-plan.md） | 产品背景与用户场景 | 只作背景。SwiftUI / SceneKit / USDZ 强制转换 / 恢复旧 Swift 源码均为**非目标** |
| PM 推导（本文） | 可验收 REQ/AC、数值默认值、发布平台、切片映射 | 属默认假设，凡有数值的判断见 §8.1 |

### 1.2 用户、场景与主流程

**目标用户**：单一管理员（物品所有者本人）。他/她拥有具体物品（相机、小家电、工具、仪器等刚性实物）、该型号的说明书 PDF、以及多视图照片，希望得到一份可交互、有出处、可校准、可复读的 3D 说明书，并自行托管数据。

**不服务的用户**：多租户终端用户、匿名访客、需要在线分享/协作的团队。这些属后续增强 E04。

**主流程（端到端，本文档所有 REQ 的骨架）**：

1. 管理员用 `init` 初始化 data-dir 与密码，`serve` 启动，浏览器登录。
2. 新建物品：填写名称、品牌、准确型号、配置（如机身+镜头组合）。
3. 上传说明书 PDF 原件与多视图照片（JPEG/PNG），为每张照片标注视图（front/left/back/right/detail）。
4. 浏览器用 PDF.js 逐页提取页文字与页图并上传到 preparation；中途关闭标签页可续传；全部页完成后封存为 ready。
5. 请求报价：看到分列的 Tripo credits 与说明书 AI USD、价格版本、保守上界与有效期；阅读"将发送给云端供应商的资料"告知并显式确认。
6. 点击生成：Rust 冻结输入快照、在同一事务预留费用并幂等创建本地 job；之后浏览器关闭不影响已提交的后台任务（只要服务进程仍在运行）。
7. 后台两条分支并行：说明书 AI 提取（分批 → 合并，产物为带页码出处的部件/步骤/规格）与 Tripo（上传图片 → 多视图生成 → 轮询 → 下载 GLB → 校验）。
8. 组装出草稿，`draft.status = needs_review`。生成成功**不等于**事实已核验，也**不等于**已发布。
9. 在浏览器：加载模型、确认知识（部件/步骤/规格/警告）、点选模型表面绑定热点、设置步骤视角、必要时修正文案并保留修改来源。
10. 发布：服务端校验发布不变量后生成**不可变**说明书版本。
11. 阅读：3D + 部件 + 步骤 + 原文联动；导出发布包；停服备份、恢复到新目录。

### 1.3 部署与信任边界

- **单管理员自托管**：一个 data-dir 只允许一个进程持有排他锁（architecture.md §6）。默认监听 `127.0.0.1:8080`；非 loopback 监听只允许位于显式受信反向代理之后（配置 `trusted_proxy_cidrs`、TLS 由代理终止）这一种放行路径，否则拒绝启动；**MVP 不包含内置 TLS 监听**（D-4/A-15），配置 `tls.*` 时 fail-closed 拒绝启动、不降级明文。不提供默认密码。
- **data-dir 是用户数据的唯一事实来源**：`manual.sqlite3`（含 WAL/SHM）、`blobs/<sha256前缀>/<sha256>`、`tmp/`、`logs/`、锁文件。密钥不落 data-dir 日志；原始 PDF、照片、GLB 都在 data-dir。
- **单二进制的准确含义**（ADR-002）：每种操作系统/CPU 架构各有一个可执行文件，内含 Rust 服务、React 构建产物、前端运行资源（PDF.js worker/CMaps/standard fonts/WASM、字体图标、允许的 3D 解码器）、数据库迁移与 bundled SQLite 引擎。运行时**不要求** Node、Python、PDF 可执行程序、Tesseract、Chromium、外部数据库或容器；运行时**仍需要**浏览器、操作系统与系统信任根。"一个文件"不表示跨平台通用，也不表示用户数据写进程序本身。
- **浏览器信任边界**：浏览器只持有会话 cookie 与 CSRF token，不接触 Tripo / 说明书 AI 密钥；所有外部 API 由 Rust 调用；后端默认不开放宽泛 CORS（开发期由 Vite 代理 `/api`）。
- **云端边界**：原始资料只在用户显式确认后发送给对应供应商；Rust 不抓取任意用户 URL；供应商下载走允许域 + 每跳地址校验，拒绝私网/回环/链路本地。

### 1.4 成功指标（可观察，不是营销承诺）

本期的成功定义为"可交付的自托管 MVP"，指标全部可观察、可复现：

1. 一个从未接触源码的使用者，能在一台干净的目标平台机器上只放一个可执行文件 + 一个 data-dir，完成 init → 登录 → 建物品 → 准备 PDF → 报价确认 → 生成（fixture 或授权真实链路）→ 校准 → 发布 → 阅读 → 备份恢复，全程不需要 Node/Python/源码。
2. 每个 MVP 必选 AC（§4）在指定命令、HTTP 行为、状态转换或浏览器可观察行为上有证据；退出码 0 但 0 个测试不算通过。
3. 崩溃与未知状态不产生二次付费：模拟 5 个崩溃断点后，本地 job 数、远端请求数、费用预留与不可变版本均可核对。
4. 已发布版本不可变、可持续阅读：改草稿、换模型、断外网都不改变旧发布版内容。
5. 真实数据质量指标（自动提取准确率、真实费用、真实模型效果）本期**不作为承诺**，只作为 T23 授权验证项记录（见 §8.2）。

### 1.5 用户原话、推导要求与建议的区分

| 类型 | 条目 | 说明 |
| --- | --- | --- |
| 用户原话 | 输入型号+说明书+多视图 → Tripo 3D → 交互说明书；React + Rust 网站；一个二进制发布；四角色协作 + llmdoc | 见 request.md，全部为必选 |
| 推导要求（由架构/合同派生） | 单管理员、data-dir、浏览器 PDF 准备、草稿+人工确认发布、submission_unknown 对账、按阶段租约任务、GLB 直接运行（不再默认 USDZ 转换） | 已由 ADR-001～009 accepted，本文只做可验收化 |
| PM 建议/默认值 | 报价有效期 10 分钟、上传与体积上限、会话与限速默认值、必选发布平台选择、未配置 Provider 的 409 语义 | 见 §8.1，可在不改变用户目标的前提下调整 |

## 2. 范围与非目标

### 2.1 本期交付（MVP）

资料库、新建物品向导、浏览器 PDF 准备与续传、费用报价与显式确认、后台任务与崩溃恢复、Tripo GLB 生成、说明书知识提取（带页码出处）、草稿组装、3D 阅读器、热点人工校准、知识确认与人工修订、发布不可变版本、阅读端联动、导出、备份恢复、单二进制发布。对应任务卡 T01–T23（§7）。

### 2.2 明确非目标（本期不做）

1. **多用户、租户、公网 SaaS、分享链接、评论**：E04，需新 PRD 与架构决策。
2. **自动热点候选**：E01。本期热点全部由人工在浏览器点选；自动候选需要"渲染精确模型版本多视图 → AI 2D 候选 → 相机矩阵 raycast → 人工确认"的完整链路，且需要遮挡/歧义验证。
3. **分件、爆炸图、机械运动（旋转/平移/装配动画）**：E02。Tripo 分割结果不等于机械语义，需独立 PRD 说明来源证明、转轴与限位。
4. **无浏览器资料准备（服务端 PDF 渲染、本地 OCR 可执行程序）**：E03。若引入原生渲染依赖会破坏单二进制边界，必须由用户选择。
5. **服务端抓取任意用户 URL 生成资料**：`document.source_url` 只作出处记录，不触发服务器下载；物品级不存在来源链接字段（A-16）。
6. **无限量免费重生成、自动降质量、自动换模型**：任何质量/模型/阶段变化都必须重新报价与确认。
7. **MVP 全文检索**：首版只按物品名称/型号检索（contracts.md §2）；全文索引后续需求再做。
8. **永久删除物品/资产的管理 API、任意磁盘浏览 API**：归档代替删除；不物理删除被引用资产。
9. **PDF 导出/打印、ZIP 导入接口**：导出是数据便携与灾备自包含包，不承诺双击运行网站；导入需另设 zip-slip/解压炸弹验收后才开放。
10. **定时/自动备份**：备份是管理员显式命令（停服 + 独占锁）。
11. **旧 macOS 路线**：不实现 SwiftUI / SceneKit / RealityKit / 强制 USDZ 转换；不要求恢复旧 Swift 源码。
12. **未验证平台的支持声明**：Safari、Windows、Intel macOS 在各自实机验证前不列入支持列表。

### 2.3 后续增强（E01–E04，不得混入 MVP）

| ID | 增强 | 前置条件 | 不计入本 PRD 验收 |
| --- | --- | --- | --- |
| E01 | 自动热点候选（AI 2D 候选 + raycast + 人工确认） | 先验证遮挡、歧义与可接受率；需要浏览器参与，不承诺关闭浏览器可继续 | 本期所有热点均由人工点选 |
| E02 | 分件与机械交互（Joint/Action 定义、转轴、限位、顺序） | 独立 PRD + 来源证明 | 本期不承诺部件独立运动 |
| E03 | 无浏览器资料准备（PDFium 等原生渲染依赖） | 静态链接与许可证评估；若破坏单文件边界需用户选择 | 本期准备必须保持标签页打开 |
| E04 | 多用户 / 公网 SaaS | 租户隔离、对象存储、配额、审计、可能迁移数据库/队列 | 本期单管理员 |

任何 REQ 若被要求并入 E01–E04 的内容，必须回到 PM 修订 PRD，RD 不得顺手实现。

## 3. 明细需求

本表条目**全部为 MVP 必选**（与 §2.3 的 E01–E04 无关）。每条 REQ 的完成标准见 §4 对应 AC，测试命令见各 AC 的"测试方式"列。REQ 与 contracts.md 的状态枚举、错误码、金额单位、页码基准保持一致；如发现冲突，以本文 §5 的显式说明为准并回 PM 记录。

### 3.1 基础与运行

| REQ ID | 场景／触发 | 业务规则与输入输出 | 错误／恢复／权限／费用 | AC IDs |
| --- | --- | --- | --- | --- |
| REQ-001 | 管理员要在一个目标平台机器上运行产品 | 输入：release 二进制 + 新 data-dir。输出：单文件可执行程序（内含 Rust 服务、React 构建产物、PDF.js 运行资源、迁移、bundled SQLite），首次运行可 `init` 初始化，随后 `serve` 提供 SPA 与 `/api/v1/*`。运行环境不需要 Node/Python/PDF 程序/外部数据库 | 目录中缺 dist 时 release 构建必须失败（不得产出空壳）；embedded-ui 仅在发布构建要求，普通后端单测不要求 dist；未知 `/api/*` 返回 JSON 404，SPA fallback 只服务 HTML 导航 | AC-001, AC-002 |
| REQ-002 | 管理员登录与保护所有资料操作 | 输入：`init` 交互输入的密码（或受限文件）、登录密码。输出：Argon2 哈希落库；登录成功返回 HttpOnly、SameSite=Strict（HTTPS 下加 Secure）的会话 cookie 与 CSRF token；`GET /auth/session` 供刷新恢复；`POST /auth/logout` 撤销会话。全部 `/api/v1` 路由受会话保护，仅健康探针与登录例外 | 无默认密码；401 未登录、403 CSRF/Origin 失败、429 登录限速；密码交互输入，不以 `--api-key` 风格参数暴露；登出后旧 cookie 失效；无匿名后门。默认会话 7 天绝对过期（假设 A-05），登录失败限速默认 5 次/分钟 | AC-003, AC-004 |
| REQ-003 | 部署与运维配置 | 输入：CLI `init/serve/check/backup/restore` + 配置（CLI 非密钥项 > 环境变量 > TOML > 默认）。输出：结构化日志（脱敏）、data-dir 排他锁、明确的退出码。配置键至少覆盖 data_dir、listen、public_origin、tls、trusted_proxy_cidrs、providers.*、limits、concurrency、price_catalog_path | 未知配置键报错而不是忽略；缺密钥可启动浏览已有资料，但生成返回未配置；`backup/restore` 在 T20 完成前返回非零并说明未实现；同一 data-dir 第二个进程必须失败退出；非 loopback 仅当显式配置 `trusted_proxy_cidrs` 才放行，未配置或配置 `tls.*`（MVP 无内置监听）一律拒绝启动、不降级明文（A-15）；`check` 不调用任何外部 API、不产生费用 | AC-005, AC-006 |
| REQ-004 | 数据库初始化与升级 | 输入：空 data-dir 或已有旧 schema 的库。输出：迁移随二进制内嵌，启动时自动检测并升级到支持版本；核心表、唯一键、外键、索引按 contracts.md §2 建立；WAL、busy_timeout=5s、synchronous=FULL | 程序版本低于库 schema 时**拒绝打开**并给出可读错误，不修改数据；迁移 SQL 变更必须触发重新编译（rerun-if-changed）；迁移在启动事务内完成，失败不留半升级状态 | AC-007, AC-008 |
| REQ-005 | 灾备：备份与恢复到新目录 | 输入：停服后的 data-dir、明确的新输出路径。输出：一致 SQLite 快照 + 全部被引用 blob + manifest + sha256；`restore` 到**不存在或为空**的目标目录，校验 hash/外键/引用后完成 | 运行中（持锁）请求 backup 必须拒绝并要求先停服；不覆盖已有备份；损坏 blob/hash 不符/非空目标目录 → 失败并保留现场；备份与导出不含密钥、绝对路径、会话、临时云端 URL；恢复后发布版、PDF、GLB 可读 | AC-009, AC-010 |
| REQ-006 | 前后端接口不能各自手抄 | 输入：Rust DTO。输出：`cargo xtask contracts` 生成并提交 `contracts/openapi.json` 与 TS 类型，前端只用生成类型 | 生成结果稳定排序、确定性；`--check` 发现差异必须非零且不改工作树；前端手写 DTO 视为缺陷。费用：无 | AC-011 |
| REQ-007 | 未配置供应商或不就绪时不得假成功 | 输入：无 Provider 密钥/无价格配置的部署。输出：`GET /settings/status` 返回 providersConfigured、limits、capabilities（不含密钥）；已有 release/PDF/GLB 照常可读；estimate/jobs 返回明确的未配置错误（PM 决定：409 `PROVIDER_NOT_CONFIGURED` / `PRICE_CATALOG_MISSING`，见 §8.1 A-13） | 不得回退 mock Provider；不得返回 0 费用假成功；不得创建 job 或费用记录；`/health/ready` 只检查 DB/迁移/数据目录，云端不可达不得使就绪失败 | AC-012, AC-013 |
| REQ-008 | 默认测试禁止真实收费调用 | 输入：测试构建 + 本机 fixture 脚本。输出：Tripo 与 Manual AI 的本机 HTTP fixture（延迟/断连/429/5xx/畸形 JSON/成功），记录调用次数与请求体；原创小型样例资产（小 GLB、文字 PDF、扫描 PDF、图片）并记录来源与许可 | 缺少 fixture 脚本必须失败，不返回通用成功；测试进程不得产生真实外网调用；测试开关不能进入生产默认；真实调用只能经 `cargo xtask test-live --budget-file` 显式入口，且无授权/超预算非零退出 | AC-014, AC-015 |

### 3.2 资料与物品

| REQ ID | 场景／触发 | 业务规则与输入输出 | 错误／恢复／权限／费用 | AC IDs |
| --- | --- | --- | --- | --- |
| REQ-010 | 管理员建立物品档案 | 输入：名称、品牌、准确型号、变体/配置（**不含物品级来源链接**；出处链接仅由绑定说明书时的 `document.source_url` 承载，A-16）。输出：item（服务器生成 UUIDv7 id、整数 revision），创建返回 201，列表返回 `{data, nextCursor}` 分页（默认 20、最多 100） | 名称与型号必填，空白/超长 422；物品级 `sourceUrl` 属未知字段 → 422（不静默忽略）；同品牌型号不强制唯一（允许不同配置并存）；编辑需 `If-Match`（缺 428、冲突 412）；归档用 PATCH 字段，不物理删除被引用资产，已发布资料归档后仍可读；MVP 不提供永久删除 API | AC-016, AC-017 |
| REQ-011 | 上传原始资料文件 | 输入：multipart `file` + `purpose`（document/photo/pageImage/pageText）。输出：asset（含 sha256、size、mime、storage_state），内容按 sha256 唯一存储（重复内容命中同一 blob）。`GET/HEAD /assets/{id}/content` 经授权后提供 ETag、HEAD、单 Range | 流式写 tmp → 校验 → fsync → 原子 rename → 短事务提交元数据；伪造类型 415、超限 413、像素炸弹/解码失败 422、越权资产 404、磁盘预留不足 `413` + `details.reason=insufficientStorage`（A-14）且不半提交；原文件名只作元数据不作路径；崩溃留下的 tmp 文件被隔离，不因 DB 回滚删除共享 blob | AC-018, AC-019 |
| REQ-012 | 绑定说明书原件 | 输入：已上传 PDF asset、title、可选 source_url。输出：document 记录（item_id、source_asset_id、source_sha256） | 校验 mime 与资产归属，跨物品引用被拒；`source_url` 仅保存为出处，服务端不发起抓取；加密 PDF 的拒绝发生在准备阶段（见 REQ-014） | AC-020 |
| REQ-013 | 标注多视图照片 | 输入：照片 asset、view ∈ {front,left,back,right,detail}。输出：photo 记录，同一快照每视图最多一张；view 以物品自身方向为参照（非观察者视角） | 修改需 If-Match；detail（特写）不进入 Tripo 多视图请求体，只用于理解与核对；照片必须属于同一物品；缺 front 或视图不足时在报价/生成处被拒（REQ-020/022） | AC-021 |
| REQ-014 | 浏览器完成 PDF 页准备（ADR-003） | 输入：原 PDF。输出：逐页 1-based `pageNumber` 的页文字与页图资产上传到 `PUT /preparations/{id}/pages/{n}`；页图长边 ≤2000px、白底 JPEG；页图坐标原点为旋转后 viewport 左上角。`GET /preparations/{id}` 返回页状态与缺页，用于断线续传 | 加密 PDF 与 >100 页 PDF 明确拒绝；每次只渲染一页，切换/取消必须销毁 render task 与 canvas；worker 的 ArrayBuffer 可能被转移，不得假设仍可读；PDF.js 主包与 worker/CMaps/standard fonts/WASM 必须同版本且本地内嵌，不访问 CDN；单页内容哈希相同幂等，ready 后禁止写。费用：0（不收费） | AC-022, AC-023, AC-024 |
| REQ-015 | 封存资料（preparation ready） | 输入：`POST /preparations/{id}/complete` + 声明 pageCount。输出：ready preparation，页号连续 1..N，标记 `clientDerived` 并保留原件供复核 | If-Match；事务校验全部连续页、资产归属与哈希，缺页/哈希不符 422 并列出缺项；ready 后不可修改；**complete 不自动发起任何收费任务**；哈希只证明字节一致，不证明页图确实来自原 PDF，UI 与知识出处不得宣称已证明 | AC-025 |
| REQ-016 | 资料库与新建向导（用户入口） | 输入：管理员操作。输出：库列表（名称、型号、状态、最近使用）+ 向导 5 步：基本信息 → 说明书 → 视图排列 → 准备进度 → 预算/隐私确认与生成按钮 | 刷新/返回上一步不丢已上传资料；缺 front 或视图不足时生成按钮不可用并解释缺项；防重复点击，但只有服务端幂等才是最终保证；空态、加载、错误态必须有可行动文案；前端不得自行判断远端成功 | AC-026 |
| REQ-017 | 生成开始后编辑物品/资料 | 输入：对 item、照片、说明书的编辑。输出：已开始任务使用**冻结快照**，编辑不影响其输入；新建快照才能用于新任务；物品 revision 变化不改变已存在 job 的快照引用 | 重生成总是新快照 + 新报价确认；旧发布版不变；输入指纹不兼容的阶段不复用缓存（下游失效规则按架构 §5：照片/视图变化 → 模型/热点失效；说明书变化 → 知识与绑定失效；型号/配置变化 → 两条链全部失效，仅复用字节相同的原始文件） | AC-027 |

### 3.3 报价、生成与任务

| REQ ID | 场景／触发 | 业务规则与输入输出 | 错误／恢复／权限／费用 | AC IDs |
| --- | --- | --- | --- | --- |
| REQ-020 | 生成前查看计划与费用 | 输入：preparationId、photoIds、modelPreset。输出：quote（绑定输入哈希、模型参数、页数、最大输出 token、价格版本与快照日期、分列金额、保守上界、expiresAt）。**只计算计划，不调用生成服务** | 前置校验：名称/型号非空、preparation ready、至少 front + left/back/right 之一、图片同物品且每视图唯一、Provider 与价格配置存在；缺项返回 422 明细或 409 `PRICE_CATALOG_MISSING`；computed 上界不确定的型号不进入首版支持清单；quote 默认有效期 10 分钟（假设 A-02） | AC-028, AC-029 |
| REQ-021 | 云端发送前告知与同意 | 输入：quote + 将发送的资料集合。输出：明确列出发送给 Tripo 的图片视图、发送给说明书 AI 的页范围/页图/型号文本、模型名、价格版本与预算上界；用户显式确认后才允许提交 | 未确认的提交被拒；确认动作写 audit_events；资料视为待分析数据不是指令，模型无工具权限，不能按 PDF 内文字改变预算、访问 URL 或运行命令；不得默认勾选同意 | AC-030 |
| REQ-022 | 冻结输入、预留费用、幂等创建任务 | 输入：quoteId + 输入 IDs + limits + `Idempotency-Key`。输出：同一事务内冻结 generation_snapshot、写 cost_ledger 预留、创建 job（首次 202） | 服务端重新校验引用、报价未过期、输入未变、预算足够，**不接受前端传入的费用数值**；相同 key + 相同 body → 返回同一 job，不新建；同 key 不同 body → 409；事务中断不留半笔预留；重复点击/断连/重启不得产生第二份生成单 | AC-031, AC-032 |
| REQ-023 | 费用展示与预算语义 | 输入：任务/报价数据。输出：Tripo **credits** 与说明书 AI **USD** 分列显示，不相加成无单位数字；显示已消耗、预留中、unknown 仍保留的预留、预估偏差 | 预算限制只保证本应用不主动发起超出授权估算的请求，**不冒充供应商账户级硬封顶**（UI 与文案必须这样表述）；超出上界的请求被应用拒绝并说明；自动降质量/换模型/增加阶段必须先重新确认；重生成需新预算确认；unknown 不得把实际费用填 0 | AC-033, AC-034 |
| REQ-024 | 后台任务可靠执行与恢复 | 输入：已入队 job。输出：阶段 DAG（freeze_inputs → manual_extract_batches → manual_merge / tripo_upload → tripo_submit → tripo_poll → model_download → model_validate → assemble_draft）逐阶段落库，带 leaseOwner/leaseEpoch/leaseUntil/nextRunAt；SQL 条件更新领取，状态推进校验 leaseEpoch | 双 worker 竞争只有一个领取；过期 worker 不得解锁后续阶段（可保存不可变 receipt）；默认租约 120s、20s 续约；临时失败进 retry_wait 按 2/4/8/16/32 秒 + jitter 退避并尊重 Retry-After，超 5 次 → failed；资料/schema 不足 → needs_input 列出可行动缺项；总等待默认 30 分钟后转 needs_input 并保留 task_id；全局远端生成并发 2、说明书批次并发 2（可降）；SIGKILL 后重启按已知远端 ID 继续查询，不重复购买；浏览器关闭不影响已提交任务（服务进程须在运行） | AC-035, AC-036 |
| REQ-025 | 付费提交结果未知时的对账 | 输入：`submission_unknown` 状态。输出：暂停该分支后续购买；记录 attempt 与预留；提供 `POST /jobs/{id}/reconcile` | 仅管理员可对账；`attachRemoteTask` 仅用于 Tripo（附加账户中查到的 ID，须查询验证类型与账号可访问性并二次确认）；`recordNoTask` 要求填写核查证据；`authorizeReplacement` 要求再次预算确认并明确重复收费风险，创建新 attempt、保留旧未决账务；同步 Manual AI 不提供 attachRemoteTask；不能伪造供应商"不存在"证明；unknown 的预留不自动释放；任务中心不得提供一键盲目重试 | AC-037, AC-038 |
| REQ-026 | 取消与按分支重试 | 输入：`POST /jobs/{id}/cancel`、`POST /jobs/{id}/retry`。输出：取消停止未提交阶段；重试只重跑指定可重试阶段 | 取消不声称已取消远端付费操作，已提交阶段保留查询/账务收尾；重试需 If-Match + Idempotency-Key；unknown 状态不得作为重试入口；已完成成果保留；重试不得偷偷改用更便宜模型 | AC-039, AC-040 |
| REQ-027 | Tripo v3 调用（ADR-004） | 输入：多视图图片（front + ≥1 侧视图，JPEG/PNG，views ∈ front/left/back/right）。输出：`POST /files` 取得 token → `POST /generation/multiview-to-model`（`model=v3.1-20260211`、texture=true、pbr=true、texture_quality/geometry_quality=standard、face_limit=100000、quad=false、generate_parts=false）→ 持久化 task_id → `GET /tasks/{id}` 保存原始状态与归一化状态 → 下载产物 | 响应体 `code` 非 0 即业务错误，不能只看 HTTP 200；success 必须带可下载模型，否则不算成功；未知状态保留原值进入待处理；查询失败 ≠ 生成失败；429/5xx 按退避重试读取；未返回 ID 的付费 POST 禁止通用自动重试中间件；不使用 v2 的 `model_version`/`files` 字段形态；缺照片要求用户补齐，不用 AI 造图代替实物照片 | AC-041, AC-042 |
| REQ-028 | 模型下载与不可变模型版本 | 输入：Tripo 成功响应中的模型 URL。输出：下载到本地、流式校验大小与 sha256、通过 GLB 检查后创建不可变 model revision（记录 asset、sha256、bounds、validation_state） | 下载使用独立 client，不带 API Authorization，不把 bearer token 转发到模型 CDN；HTTPS + 允许域 + 每跳重定向与实际连接 IP 校验，拒绝私网/回环/链路本地；关闭自动重定向或逐跳验证；拒绝外链 buffer/image URI 与未支持的 required extension；三角面 ≤100000、贴图单边 ≤4096，超预算进入 needs_input 并保留原始模型与错误；临时供应商 URL 不得当永久地址；链接过期时重新查询已知任务，不重新购买 | AC-043, AC-044 |
| REQ-029 | 说明书知识提取（ManualAiProvider） | 输入：页文字（必要时页图）、型号与 schema。输出：按 ≤5 页/批的批次结果（部件/步骤/规格/证据/不确定项），每批独立持久身份与结果资产；全部批次成功且页覆盖完整才解锁 merge；merge 用确定性本地合并，去重但保留原始出处 | 请求使用 Responses `text.format` JSON Schema（strict）与 `input_image` data URL；解析 `output[].content[]` 的 `output_text`，另行处理 refusal/incomplete/截断/畸形 JSON——这些不产生正式知识；引用必须存在于本次输入页，服务端复核引用页集合与部件引用关系；每项事实必须有 document/preparation/page 出处；不能把模型 confidence 当已验真概率；不能用正则抢救畸形 JSON 当真；超预算不再请求；真实模型效果待 T23 验证 | AC-045, AC-046 |
| REQ-030 | 组装草稿（ADR-005） | 输入：两条分支产物。输出：`assemble_draft` 幂等生成 manual_draft，`draft.status = needs_review`；job `succeeded` 仅表示可复核草稿已产出 | 模型与知识分支可独立完成、可独立重试；重启不重复创建 draft；部分成功可展示；**生成完成不自动发布**，不存在自动 release；模型成功而知识失败只重提取；unknown/needs_input 在父 job 上展示并阻塞对应分支 | AC-047, AC-048 |

### 3.4 复核、发布与阅读

| REQ ID | 场景／触发 | 业务规则与输入输出 | 错误／恢复／权限／费用 | AC IDs |
| --- | --- | --- | --- | --- |
| REQ-031 | 任务中心与可行动错误 | 输入：job 列表/详情。输出：区分本地准备、排队、供应商进度、等待人工、失败、unknown；显示阶段、已消耗/预留、错误摘要与下一步操作 | 可见页轮询约 2 秒、后台 15 秒、终态停止；不用虚假线性百分比（不显示"总进度 100%"掩盖待校准）；断网不误报业务失败；刷新与服务重启后从数据库读状态；unknown 显示对账入口而非重试按钮；每种状态有可执行下一步 | AC-049 |
| REQ-032 | GLB 阅读器与 WebGL 资源恢复 | 输入：validated model revision 的 GLB（自包含 BIN 与 PNG/JPEG 贴图）。输出：R3F 懒加载阅读器，OrbitControls 旋转/缩放/复位，loading/error 状态；模型只作资料资产，不执行脚本 | WebGL context lost/restored 需处理并可重建，不只提供刷新按钮；卸载时清理 geometry/material/texture；换模型不串入旧资源/旧热点；文本与 PDF 阅读不依赖 WebGL 成功；3D 失败仍可读文字和 PDF | AC-050, AC-051 |
| REQ-033 | 人工热点校准 | 输入：用户在模型表面点选 + 部件选择。输出：hotspot 保存 `modelRevisionId + modelSha256 + positionLocal`（asset-root 局部坐标，有限数值，禁止 NaN/Infinity）；状态 `unbound → candidate → confirmed`，人工直接拾取可 `unbound → confirmed`；可保存步骤视角（CameraPose：positionLocal/targetLocal/upLocal/fov） | unbound 时 anchor=null，不用 [0,0,0] 占位；candidate/confirmed 要求非空 anchor；raycast 只包含模型 mesh 并排除热点自身；拖动旋转后不得误触创建热点；显示居中/缩放放外层 group，保存用 asset-root 的 worldToLocal；模型 revision 变化必须进入 `stale` 并重新绑定，旧 anchor 可保留解释但不得当有效热点显示；API 拒绝 confirmed 热点与当前模型 sha 不符；MVP 不自动生成 candidate | AC-052, AC-053 |
| REQ-034 | 知识确认与人工修订 | 输入：草稿中的 Part/Step/Evidence/Review。输出：实体级 `confirmed / needs_review`；人工修改标 `userEdited` 并保留出处；modelReview 经草稿 PATCH 写入（loaded 与 userConfirmed 由用户声明，checkedAt 由服务器赋值） | 引用校验：页码 1-based 且必须存在于本次输入，部件引用关系存在；bbox 无法可靠提供时为 null（仍可跳页，不捏造框）；不能修改供应商事实快照；不能由后台 CPU 校验自动替用户确认；换模型清空 modelReview；改草稿不改变已发布 release | AC-054 |
| REQ-035 | 发布不可变版本 | 输入：`POST /items/{id}/drafts/{draftId}/publish`（If-Match + 幂等键）。输出：manual_release（不可变完整快照，manifest 记录 draftRevision、modelRevisionId、资产 sha256 与来源） | 发布不变量：必需知识已确认或有明确人工修订记录、引用页存在、选中模型 validated 且 modelReview.loaded 与 userConfirmed 均 true 且 revision/hash 匹配、每个要发布的交互部件至少一个 confirmed 热点且 hash 匹配、步骤引用全部存在、无 stale/candidate 冒充 confirmed；不满足返回 422 明细，并发 412；幂等键重放返回同一 release；无法绑定的知识可按"仅文本条目"保留并明显标识，不得为发布自动隐藏必需内容 | AC-055, AC-056 |
| REQ-036 | 阅读端 3D/部件/步骤/原文联动 | 输入：release。输出：3D 模型、部件列表、步骤导航、原文页展示四者双向联动；引用跳转到正确 1-based 页 | 部件列表提供 3D 热点的文字替代路径；步骤前进/后退/跳步不累积错误状态；键盘可达所有非 3D 核心操作且有可见 focus 与错误关联；支持减少动效 | AC-057 |
| REQ-037 | 导出发布包 | 输入：`GET /releases/{releaseId}/export`。输出：自包含包（原件、GLB、manifest、哈希、来源、schemaVersion、相对资产清单） | 只导出有权资产；不含绝对路径、密钥、会话、临时云端 URL；不承诺导出包能直接双击运行网站；本期不开放 ZIP 导入接口 | AC-058 |
| REQ-038 | 供应商不可用/断网时的可用性 | 输入：已有发布版的部署，外部 AI 断网或密钥撤销。输出：3D、部件、步骤、原文与本地 PDF 仍可读；`/health/ready` 不因云端不可达失败 | 供应商关停时历史说明书仍本地可读；不把"服务停止则网站不可用"包装成离线 PWA 能力 | AC-059 |
| REQ-039 | 窄屏与降级路径 | 输入：<768px 视口、键盘操作、减少动效偏好。输出：转抽屉/单栏布局，保留读说明书与步骤 | 移动端不支持热点校准时必须明确禁用并给出解释性提示，不能坏掉无解释；减少动效（prefers-reduced-motion）生效；所有表单错误与字段关联 | AC-060 |

### 3.5 非功能与交付

| REQ ID | 场景／触发 | 业务规则与输入输出 | 错误／恢复／权限／费用 | AC IDs |
| --- | --- | --- | --- | --- |
| REQ-040 | 性能与资源预算 | 输入：100k 三角面 / ≤4K 贴图模型、100 页 PDF、元数据 API。输出：桌面连续旋转 p95 帧耗时 ≤33ms；本地无供应商等待的普通元数据 API p95 ≤200ms（具名测试机）；模型加载耗时单独记录，不承诺 150 MiB 模型瞬开 | 3D/PDF 不在首屏库列表强制加载；连续切换 10 次模型后资源数与内存无持续增长趋势；PDF 逐页渲染、有进度可取消、100 页不同时铺满 canvas；不达标时不得由 RD 私自降低阈值，须回 PM 记录 | AC-061, AC-062 |
| REQ-041 | 浏览器支持基线 | 输入：Chrome / Edge / Firefox 当前支持版本。输出：必须全部通过并在发布报告写精确版本 | Safari 需单独实机测试才可列入支持；Headless WebGL 结果不替代目标设备人工复核；某浏览器未跑不得贴支持标签 | AC-063 |
| REQ-042 | 必选发布平台（PM 在 T00 明确，见 §5.6） | 输入：目标平台原生 runner。输出：每个平台单独的可执行文件 + SHA256 + licenses + 系统动态依赖清单，冷环境启动通过 | 不得以交叉编译退出码 0 替代运行证据；签名/公证需用户账号，未做必须声明；aarch64-apple-darwin 与 x86_64-unknown-linux-musl 为本轮必选 | AC-064 |
| REQ-043 | 安全与隐私回归 | 输入：上传/下载/认证/日志路径。输出：通过上传类型伪造、SSRF（重定向到私网、DNS 重绑定）、CSRF/Origin、路径穿越、越权资产、日志脱敏、未知 `/api/*` JSON 404 回归 | 各项无静默 skip，测试数 >0；密钥、原始 PDF 内容、签名 URL 查询串不进日志；不把 data-dir 整目录暴露为静态服务；密钥只在服务端 | AC-065 |
| REQ-044 | 可观察性与诊断 | 输入：运行中服务。输出：结构化日志含 requestId/jobId/stage/attemptId、耗时与错误码；`/health/live` 与 `/health/ready` 分离；管理页显示 provider 配置是否存在而不返回密钥；任务页区分各阶段而不用虚假百分比 | ready 不依赖云端可达；未知状态可读；错误响应统一 `error.code/message/details/requestId`，不输出堆栈、SQL、密钥或完整供应商签名 URL | AC-066 |

## 4. 验收条件

规则：每条 AC 必须落到具体命令、HTTP 行为、状态转换或可观察的浏览器行为；"体验良好""正常工作"不是 AC。标 [必选] 的 AC 全部通过才允许 `qa_running → done`；未覆盖真实供应商的 AC 由 T23 在授权后补验，缺授权时记 BLOCKED 而不是改成通过。测试方式列给出的命令是验收合同（见 validation-release.md §2），实现后必须真实执行且测试数 >0。fixture 指 T05 建立的本机 HTTP fixture。

### 4.1 基础与运行

| AC ID | REQ ID | Given | When | Then（可观察结果） | 测试方式／必选 |
| --- | --- | --- | --- | --- | --- |
| AC-001 | REQ-001 | 开发机与已解析的版本锁 | 执行 `cargo xtask dist --target <本机target>`，然后在只含该二进制的新临时目录执行 `cargo xtask smoke-bootstrap --binary <绝对路径>` | 产出单个可执行文件 + SHA256 + licenses + build-info；smoke-bootstrap 只验证内嵌页面、静态资源与 health 并退出 0；目录中无源码无 Node 仍可运行 | [必选] xtask dist + smoke-bootstrap（T01/T22） |
| AC-002 | REQ-001 | release 二进制与 T20 生成的脱敏样例备份 | 执行 `cargo xtask smoke --binary <绝对路径>` | 新临时目录 + 新 data-dir restore 后启动成功；首页、嵌套路由刷新、JS/CSS/字体/PDF 本地资源加载；`GET /api/unknown` 返回 JSON 404 而非 index.html；停服重启后数据仍在；全程不连接本机 Provider fixture | [必选] xtask smoke（T22） |
| AC-003 | REQ-002 | 未登录会话 | 请求 `GET /api/v1/items`；随后正确密码登录并注销 | 第一次请求 401 且响应为 `error.code/message/details/requestId`；登录返回 HttpOnly、SameSite=Strict cookie 与 CSRF token；注销后携带旧 cookie 再请求得到 401；`GET /auth/session` 带 `Cache-Control: no-store` | [必选] `cargo test -p everything-manual --test auth_api` |
| AC-004 | REQ-002 | 已登录管理员 | 依次发起：无 CSRF/跨站 Origin 的 PATCH；连续错误密码登录；缺 `If-Match` 的 PATCH；使用过期 revision 的 PATCH | 分别返回 403、429（限速）、428、412；错误响应不泄露堆栈或 SQL | [必选] `cargo test -p everything-manual --test auth_api` |
| AC-005 | REQ-003 | 合法与非法配置组合 | 执行 `cargo test -p everything-manual --test config_cli` | 覆盖 init/serve/check 分派、未知配置键报错、缺密钥不启动 mock、第二个进程持有同一 data-dir 失败退出、日志中不出现敏感字段；backup/restore 未实现期间返回非零并说明 | [必选] `cargo test -p everything-manual --test config_cli` |
| AC-006 | REQ-003 | 非 loopback 监听未获得放行条件：无可信代理配置；或配置了 `tls.*`（MVP 无内置 TLS 监听） | 分别以非 loopback 地址执行 `serve`、`check --listen`；并执行 `check --data-dir <dir>` | serve/check 均拒绝且退出码非零（配置 `tls.*` 时 fail-closed、不降级明文，A-15）；`check --data-dir` 全程不发起任何外部 HTTP 请求（fixture 记录真实调用为 0） | [必选] `cargo test -p everything-manual --test config_cli` |
| AC-007 | REQ-004 | 空 data-dir | 执行 `cargo test -p everything-manual --test storage` | 核心表/唯一键/外键/索引按合同建立；重复迁移幂等；外键拒绝非法引用；revision CAS 生效；回滚不留半状态；关闭并重开同一 data-dir 后数据保留 | [必选] `cargo test -p everything-manual --test storage` |
| AC-008 | REQ-004 | 旧 schema 库一份、比程序更新的 schema 库一份 | 分别启动服务 | 旧 schema 自动迁移成功且数据保留；新 schema 被拒绝打开并输出可读错误，库文件未被修改；修改迁移 SQL 后二进制触发重编译 | [必选] `cargo test -p everything-manual --test storage` + 手工启动验证 |
| AC-009 | REQ-005 | data-dir 被运行中进程持锁 | 执行 `backup`；随后停服再次执行 | 运行中请求被拒绝并说明需先停服；停服后备份成功，产物含一致 SQLite 快照、全部被引用 blob、manifest 与 sha256；已存在的输出路径不被覆盖 | [必选] `cargo test -p everything-manual --test backup_restore` |
| AC-010 | REQ-005 | 合法备份 | 执行 `restore --from <备份> --data-dir <新目录>` | 目标已存在且非空被拒绝；损坏 blob 或 hash 不符时失败并保留现场；成功后同一 release、PDF、GLB 可读；导出与备份内容不含密钥、绝对路径、会话、临时云端 URL | [必选] `cargo test -p everything-manual --test backup_restore` |
| AC-011 | REQ-006 | 干净工作树 | 执行 `cargo xtask contracts --check`；随后人工改动生成文件再执行一次 | 第一次退出 0 且不修改工作树；改动后非零退出；前端类型来自生成文件，`npm --prefix apps/web run typecheck` 通过 | [必选] `cargo xtask contracts --check` |
| AC-012 | REQ-007 | 无 Provider 密钥与无价格配置的 data-dir | 启动服务，访问 settings、已有 release、并提交 estimate/jobs | `GET /settings/status` 返回 providersConfigured=false 且不含任何密钥或完整配置；已有 release/PDF/GLB 可读；estimate/jobs 返回明确未配置错误、不创建 job、不写费用记录、不发起远端请求 | [必选] `cargo test -p everything-manual --test config_cli` + `--test generation_requests` |
| AC-013 | REQ-007 | release 构建、缺 API key | 启动服务并访问 `/health/ready` | 不自动回退 mock Provider，不产生假模型；ready 依据 DB/迁移/数据目录判定为可用；云端不可达不使 ready 失败 | [必选] `cargo test -p everything-manual --test fixture_harness` |
| AC-014 | REQ-008 | 测试构建 | 执行 `cargo test -p everything-manual --test fixture_harness` | fixture 覆盖延迟/断连/429/5xx/畸形 JSON/成功；记录调用次数与请求体；缺少脚本时测试失败而不是通用成功；断言测试进程无真实外网调用；样例资产 hash 与许可见 implementation.md | [必选] `cargo test -p everything-manual --test fixture_harness` |
| AC-015 | REQ-008 | 默认测试入口 | 执行 `cargo xtask check`、`npm --prefix apps/web run test:e2e`；再执行 `cargo xtask test-live --case <case>`（无 budget-file） | 前两者不触发任何真实收费 API；test-live 缺 `--budget-file` 或无授权或超预算时非零退出并拒绝执行 | [必选] `cargo xtask check`；test-live 参数校验在 T23 |

### 4.2 资料与物品

| AC ID | REQ ID | Given | When | Then（可观察结果） | 测试方式／必选 |
| --- | --- | --- | --- | --- | --- |
| AC-016 | REQ-010 | 已登录管理员 | 创建物品 → 编辑 → 归档；另用空白名称/型号创建 | 创建 201 且返回 UUIDv7 id 与整数 revision；空白或超长 422；列表返回 `{data, nextCursor}`；归档物品不出现在默认列表但其已发布资料仍可读；不存在永久删除 API（对删除路由返回 404/405） | [必选] `cargo test -p everything-manual --test items` |
| AC-017 | REQ-010 | 已存在某品牌型号物品 | 创建同品牌型号不同配置的物品；并发对同一物品做两次 PATCH | 两条记录并存互不覆盖；后到 PATCH 使用旧 revision 时返回 412 且 `details.currentRevision` 可见 | [必选] `cargo test -p everything-manual --test items` |
| AC-018 | REQ-011 | 已登录管理员 | `POST /items/{id}/assets` 上传 PDF/JPEG/PNG；重复上传同一文件；`GET`/`HEAD`/带 Range 请求 `/assets/{id}/content` | 返回 asset 且内容按 sha256 去重（同一 blob）；`ETag` 存在且 `If-None-Match` 可 304；HEAD 无 body；合法单区间 206 且 Content-Range/Length 正确；不可满足 416；响应不含磁盘路径 | [必选] `cargo test -p everything-manual --test assets` |
| AC-019 | REQ-011 | 恶意/超限输入 | 上传伪造类型文件、超限文件、像素炸弹、含路径穿越的文件名、访问他人 asset、磁盘预留不足场景 | 分别得到 415 / 413 / 422 / 404；磁盘预留不足（预检/复检判定）→ `413` + `details.reason=insufficientStorage`（A-14）；不产生半提交资产与孤儿元数据；DB 事务回滚不删除已被其他记录引用的共享 blob；tmp 残留被隔离清理 | [必选] `cargo test -p everything-manual --test assets` |
| AC-020 | REQ-012 | 已上传 PDF asset | 创建 document（含 source_url）；再用另一物品的 asset 创建 | 正常创建成功并保存 source_sha256；跨物品引用被拒（404/422）；服务端不对 source_url 发起任何抓取（fixture 记录 0 次外呼） | [必选] `cargo test -p everything-manual --test items` |
| AC-021 | REQ-013 | 物品已有照片 | 添加/修改视图；给同一视图再加一张；用 detail 照片发起多视图请求 | view 只接受 front/left/back/right/detail；同一快照每视图最多一张（第二张被拒或覆盖规则明确）；PATCH 缺 If-Match 428、冲突 412；detail 不出现在 Tripo 多视图请求体中 | [必选] `cargo test -p everything-manual --test items` + `--test tripo_contract` |
| AC-022 | REQ-014 | 已绑定文字型 PDF | 在准备页开始准备并完成全部页 | 每页产生 1-based `pageNumber` 的页文字与页图资产；相同内容重复 PUT 幂等；页图长边 ≤2000px、白底 JPEG；页图坐标原点为旋转后 viewport 左上；`GET /preparations/{id}` 反映已完成页 | [必选] `cargo test -p everything-manual --test preparations` + `npm --prefix apps/web run test:e2e -- pdf-preparation.spec.ts` |
| AC-023 | REQ-014 | 准备中途关闭标签页/断网 | 重新进入同一 document 继续准备 | 只补齐缺失页，不重传已完成页；离线状态下 PDF.js 的 worker/CMaps/standard fonts/WASM 均从本地加载成功（无 CDN 请求）；扫描页走页图上传、旋转页与非拉丁字体页文字提取正确 | [必选] Playwright `pdf-preparation.spec.ts` |
| AC-024 | REQ-014 | 加密 PDF、>100 页 PDF | 触发准备 | 均明确拒绝并给出原因（不支持加密 / 超出 100 页），不创建页记录、不进入 jobs、不产生任何收费请求；错误文案可行动 | [必选] Playwright `pdf-preparation.spec.ts` + `--test preparations` |
| AC-025 | REQ-015 | 全部页已上传 | `POST /preparations/{id}/complete`；再尝试写入已 ready 的页 | 成功时页号连续 1..N 且标记 clientDerived（保留原件）；缺页或哈希不符返回 422 并列出缺项；ready 后写入被拒；**complete 不创建 job、不产生费用** | [必选] `cargo test -p everything-manual --test preparations` |
| AC-026 | REQ-016 | 已登录管理员 | 在向导中完成 新建物品 → 上传 PDF → 上传照片并标注视图 → 完成准备 → 看到预算与隐私确认页；中途刷新页面；缺 front 时查看生成按钮 | 刷新后已上传资料不丢；缺 front 或视图不足时生成按钮不可用并说明缺项；重复点击只产生一个 job（服务端幂等）；所有页面有可观察的空态/加载/失败态与可行动错误 | [必选] Playwright `import-flow.spec.ts` |
| AC-027 | REQ-017 | 已有运行中 job | 编辑物品/替换照片/更换说明书，再查看 job 与草稿 | 已开始任务继续使用冻结快照，输入哈希不变；重生成创建新快照并要求新报价确认；旧发布版内容不变 | [必选] `cargo test -p everything-manual --test generation_requests` |

### 4.3 报价、生成与任务

| AC ID | REQ ID | Given | When | Then（可观察结果） | 测试方式／必选 |
| --- | --- | --- | --- | --- | --- |
| AC-028 | REQ-020 | preparation ready、front + 至少一侧视图、已配置价格 | `POST /items/{id}/estimates` | 返回 Tripo **credits** 与 Manual AI **USD** 分列、价格版本与快照日期、保守上界、expiresAt（默认 10 分钟）；不调用任何生成服务、不产生费用记录；金额用整数单位（creditMinor / usdMicros） | [必选] `cargo test -p everything-manual --test generation_requests` |
| AC-029 | REQ-020 | 缺价格配置 / 未 ready 准备 / 缺 front 三种情况 | 分别请求 estimate | 缺价格 → 409 `PRICE_CATALOG_MISSING`；未 ready 或缺视图 → 422 并列出具体缺项（缺少哪个视图/准备未完成）；任何情况下不返回伪造精确金额 | [必选] `cargo test -p everything-manual --test generation_requests` |
| AC-030 | REQ-021 | 已完成 estimate | 查看确认页后不勾选确认提交 jobs；再勾选确认提交 | 未确认提交被拒（不创建 job）；确认页明确列出发送给 Tripo 的视图、发送给说明书 AI 的页范围/页图与型号文本、模型名、价格版本与预算上界；确认动作写入 audit_events；默认不预勾选 | [必选] `cargo test -p everything-manual --test generation_requests` + Playwright `import-flow.spec.ts` |
| AC-031 | REQ-022 | 已确认报价 | 以同一 `Idempotency-Key` 连续提交 `POST /items/{id}/jobs` 20 次；再用同 key 改 body 提交一次 | 只存在 1 个 job（首次 202，重放返回同一 job id）；同 key 不同 body → 409；job 与 cost_ledger 预留同事务写入，事务中断不留半笔预留 | [必选] `cargo test -p everything-manual --test generation_requests` |
| AC-032 | REQ-022 | 报价已过期 / 输入已变 / 允许上限低于服务端计算上界 | 提交 jobs | 均被拒绝（422/409）且不创建 job、不产生远端请求；服务端不使用前端传入的费用数值；重新报价后才可提交 | [必选] `cargo test -p everything-manual --test generation_requests` |
| AC-033 | REQ-023 | 存在进行中与历史任务 | 查看任务详情与费用区域 | Tripo credits 与 Manual AI USD 分列显示，不相加成无单位数字；显示已消耗、预留、以及 unknown 仍保留的预留；页面明确说明本地预算是应用侧控制、不是供应商账户级硬上限 | [必选] Playwright `job-recovery.spec.ts` + `--test generation_requests` |
| AC-034 | REQ-023 | 预算用尽 / 需要更高成本的处理 | 尝试发起超出授权估算的生成或自动降质量 | 应用拒绝发起并说明原因；不自动降质量、不换模型、不加处理阶段；重生成必须经新报价确认；unknown 场景实际费用不会被填 0 | [必选] `cargo test -p everything-manual --test generation_requests` |
| AC-035 | REQ-024 | 已入队 job；另注入 SIGKILL 断点 | 执行 `cargo test -p everything-manual --test jobs_recovery` | 阶段 DAG 按合同落库（含 batch_index 与 stage_kind 唯一性）；双 worker 竞争只有一个领取；过期 worker 晚到 receipt 可保存但不能推进状态；SIGKILL 后重启按已知远端 ID 继续查询、不产生第二次付费提交；并发上限（生成 2、AI 批次 2）生效 | [必选] `cargo test -p everything-manual --test jobs_recovery` |
| AC-036 | REQ-024 | 阶段遇到 429/5xx/超时；另一条为资料/schema 不足 | 观察阶段状态转换 | 临时失败 → retry_wait，退避 2/4/8/16/32 秒 + jitter 并尊重 Retry-After，超 5 次 → failed；资料/schema/支持能力不足 → needs_input 并列出可行动缺项，不无休止重试；总等待超 30 分钟转 needs_input 且保留 task_id（恢复只查询、不重新购买） | [必选] `cargo test -p everything-manual --test jobs_recovery` |
| AC-037 | REQ-025 | 付费 POST 已发出但响应未到（注入断点） | 重启后查看任务与对账入口 | attempt 标记 `submission_unknown`，该分支后续购买暂停；任务中心不提供"重试"按钮而提供对账；`reconcile` 的 attachRemoteTask（仅 Tripo，验证类型与账号可访问性并二次确认）/ recordNoTask（要求证据）/ authorizeReplacement（再次预算确认 + 重复收费风险，创建新 attempt、保留旧未决账务）行为符合合同；unknown 预留不自动释放；审计保留 | [必选] `cargo test -p everything-manual --test pipeline` + Playwright `job-recovery.spec.ts` |
| AC-038 | REQ-025 | Manual AI 同步批次已发请求但完整响应未持久化 | 重启并检查该批与服务 | 该批进入 `submission_unknown`；系统不假定 response_id 可轮询/重取；已持久化结果的批次不重跑、不重复付费；result 已存而 checkpoint 未推进时恢复补推进而不重新请求；同步链路不提供 attachRemoteTask | [必选] `cargo test -p everything-manual --test manual_ai_contract` + `--test jobs_recovery` |
| AC-039 | REQ-026 | 运行中 job（含已提交远端阶段） | `POST /jobs/{id}/cancel` | 未提交阶段停止领取与推进；已提交阶段停止新业务推进但保留查询与账务收尾；响应与 UI 文案不声称已取消远端付费操作；产生 audit_event | [必选] `cargo test -p everything-manual --test pipeline` |
| AC-040 | REQ-026 | 模型分支成功、知识分支失败；另一 job 处于 unknown | 对失败分支 `POST /jobs/{id}/retry`；对 unknown job 尝试 retry | 只重跑指定可重试阶段，已完成成果保留不被覆盖；retry 需要 If-Match 与 Idempotency-Key，重放不产生第二个 attempt；unknown 状态不提供 retry 入口（被拒且无副作用）；重试不改变模型/质量预设 | [必选] `cargo test -p everything-manual --test pipeline` |
| AC-041 | REQ-027 | 测试构建 + Tripo fixture | 执行 `cargo test -p everything-manual --test tripo_contract` | 断言：上传使用 multipart 字段 `file`；多视图 body 含 `inputs`(front + 侧视图 view-key)、`model=v3.1-20260211`、texture/pbr=true、quality=standard、face_limit=100000、quad=false、generate_parts=false；HTTP 200 但 `code!=0` 视为失败；success 缺可下载模型不算成功；未知状态保留原值；429 退避；POST 已被接受后断连不自动重发；无 v2 字段形态 | [必选] `cargo test -p everything-manual --test tripo_contract` |
| AC-042 | REQ-027 | 用户授权凭据、真实型号资料与预算文件（T23） | 执行 `cargo xtask test-live --case <case> --budget-file <受限文件>` | 记录真实请求/响应摘要（脱敏）、实际 credits、任务结果与差异说明；报告不含密钥；无授权或超预算时命令非零退出；该项缺失记 BLOCKED，不以 fixture 结果冒充真实通过 | [必选，依赖用户授权] `cargo xtask test-live` |
| AC-043 | REQ-028 | Tripo 任务成功、模型 URL 有效 | 下载并校验后查看 model revision | 下载 client 不带 API Authorization；HTTPS + 允许域 + 每跳重定向与实际连接 IP 校验，拒绝私网/回环/链路本地；流式大小与 sha256 校验通过；GLB magic/version/长度/chunk/JSON 结构/accessor 边界/内嵌资源/扩展检查通过；创建不可变 model revision 且临时供应商 URL 未被当作永久地址保存 | [必选] `cargo test -p everything-manual --test model_assets` |
| AC-044 | REQ-028 | 链接过期 / 断连 / 超面数或外链资源模型 | 处理该阶段 | 链接过期时重新查询已知任务取新链接而不重新购买；断连可安全重试下载；超面数、外链 buffer/image、不支持扩展或截断 GLB 进入 needs_input 并保留原始模型与错误，不静默改坏模型、不自动降预算 | [必选] `cargo test -p everything-manual --test model_assets` |
| AC-045 | REQ-029 | ready preparation（含文字页与扫描页） | 执行 `cargo test -p everything-manual --test manual_ai_contract` | 断言真实适配器所发 HTTP：Responses `input` 含 input_text 与必要 input_image（JPEG data URL）、`text.format.type=json_schema`、strict、全部 required 与 additionalProperties=false、max_output_tokens 受限；批次 ≤5 页并记录覆盖率；refusal/incomplete/截断/畸形 JSON 不产生正式知识；引用不存在页被服务端拒绝；扫描页走页图 | [必选] `cargo test -p everything-manual --test manual_ai_contract` |
| AC-046 | REQ-029 | 多批提取（含一批失败） | 查看批次记录与合并结果 | 每批独立持久身份与结果资产；全部批次成功且页覆盖完整才解锁 merge；merge 去重但保留原始出处，冲突事实保留为待复核；PDF 内恶意指令不能改变预算、不能触发 URL 访问或命令执行（模型无工具权限） | [必选] `cargo test -p everything-manual --test manual_ai_contract` |
| AC-047 | REQ-030 | 两条分支完成（fixture 全链路） | 执行 `cargo test -p everything-manual --test pipeline` | 组装出 draft 且 `status=needs_review`；job `succeeded` 不等于 published；模型成功知识失败时只重提取知识分支；重启不重复创建 draft；部分成功状态可展示 | [必选] `cargo test -p everything-manual --test pipeline` |
| AC-048 | REQ-030 | 生成成功 | 查询 releases 列表与草稿 | releases 列表为空；草稿 status=needs_review；界面明确提示需要人工确认后才能发布（无自动发布路径、无后台自动 publish） | [必选] `cargo test -p everything-manual --test pipeline` + Playwright `import-flow.spec.ts` |

### 4.4 复核、发布与阅读

| AC ID | REQ ID | Given | When | Then（可观察结果） | 测试方式／必选 |
| --- | --- | --- | --- | --- | --- |
| AC-049 | REQ-031 | 存在进行中、失败与 unknown 三类任务 | 打开任务中心并断开网络 | 各任务显示本地准备/排队/供应商进度/等待人工/失败/unknown 的区分；可见页轮询约 2 秒、后台 15 秒、终态停止；断网显示网络问题而不误报业务失败；刷新与服务重启后状态从数据库恢复；unknown 显示对账入口而非重试按钮；不显示虚假总进度 100% | [必选] Playwright `job-recovery.spec.ts` |
| AC-050 | REQ-032 | 发布版含 100k 三角面模型 | 打开阅读器并旋转/缩放/复位；用 forceContextLoss 模拟上下文丢失 | 3D 模块懒加载；OrbitControls 三轴操作与复位可用；loading 与 error 状态可观察；context lost 后 restored 能重建渲染而不是只能刷新页面；卸载后 geometry/material/texture 被释放；文本与 PDF 阅读不依赖 WebGL 成功 | [必选] Playwright `viewer.spec.ts` |
| AC-051 | REQ-032 | 不对称测试模型（asset-root 有非均匀缩放/旋转） | 在不同旋转/缩放下读取同一局部点；再切换到另一模型 | 同一局部点在各种旋转缩放下的世界位置计算一致（`coordinates.test.ts` 断言）；换模型后不出现旧模型资源或旧热点串入 | [必选] `npm --prefix apps/web run test -- --run src/features/viewer/coordinates.test.ts` + Playwright `viewer.spec.ts` |
| AC-052 | REQ-033 | 草稿含可绑定部件 | 点击模型表面绑定；拖动旋转模型；为未绑定部件查看状态 | 保存 `modelRevisionId + modelSha256 + positionLocal` 且数值有限（无 NaN/Infinity）；人工直接拾取得到 confirmed；unbound 时 anchor 为 null 而非 [0,0,0]；拖动旋转不误触创建热点；raycast 只命中模型 mesh 不含热点自身 | [必选] Playwright `manual-review.spec.ts` + `--test publishing` |
| AC-053 | REQ-033 | 草稿已绑定热点 | 重新生成模型后打开新草稿；尝试用旧 sha 提交 confirmed；查看旧发布版 | 旧绑定进入 `stale` 且不作为有效热点显示（不得静默复用）；API 拒绝 confirmed 热点与当前模型 sha 不符的提交；旧发布版继续指向旧模型且可读 | [必选] `cargo test -p everything-manual --test publishing` |
| AC-054 | REQ-034 | 草稿含 AI 产出知识 | 人工确认、修改并保存；写入 modelReview | 实体级 confirmed/needs_review 可切换；人工修改标 userEdited 且保留出处；Evidence 页码 1-based 且存在于本次输入，部件引用存在；bbox 无法提供时为 null 仍可跳页；modelReview 的 loaded/userConfirmed 由用户声明，checkedAt 服务器赋值；换模型清空 modelReview；不能修改供应商事实快照 | [必选] `cargo test -p everything-manual --test publishing` |
| AC-055 | REQ-035 | 满足发布不变量的草稿 | `POST publish`；再次用同幂等键发布；发布后修改草稿 | 201 生成不可变 release，manifest 记录 draftRevision、modelRevisionId 与资产 sha256；幂等重放返回同一 release；发布后修改 draft 不改变已发布内容（重读 release 字节与哈希不变） | [必选] `cargo test -p everything-manual --test publishing` |
| AC-056 | REQ-035 | 未确认知识 / 缺 confirmed 热点 / 存在 stale 热点 / modelReview 未完成 / 并发修改 的草稿 | 分别尝试发布 | 返回 422 且 `details` 列出具体不满足项；并发修改返回 412；不得自动隐藏必需内容或伪造确认；只有 modelReview.loaded 与 userConfirmed 均 true 且 revision/hash 匹配时才可发布 | [必选] `cargo test -p everything-manual --test publishing` |
| AC-057 | REQ-036 | 已发布 release（含步骤与原文出处） | 在阅读端点击部件/步骤/原文引用，并仅用键盘操作 | 部件列表与 3D 热点双向联动；步骤前进/后退/跳步不累积错误状态；引用跳转到正确 1-based 页码并与页图/文字一致；部件列表提供 3D 热点的文字替代；所有非 3D 操作键盘可达且有可见 focus 与错误关联 | [必选] Playwright `manual-review.spec.ts` + `viewer.spec.ts` |
| AC-058 | REQ-037 | 已发布 release | `GET /releases/{releaseId}/export` | 下载自包含包（原件、GLB、manifest、哈希、来源、相对资产清单）；只含该 release 有权资产；不含绝对路径、密钥、会话、临时云端 URL；不代表可双击运行网站（文档说明） | [必选] `cargo test -p everything-manual --test backup_restore`（导出部分） |
| AC-059 | REQ-038 | 已有发布版，外部 AI 不可达 | 断外网访问阅读端并请求 `/health/ready` | 3D、部件、步骤、原文与本地 PDF 仍可读；ready 不因云端不可达失败；日志明确记录 provider 不可用而不影响本地读取 | [必选] Playwright/T22 smoke（T22 步骤 5） |
| AC-060 | REQ-039 | <768px 视口、键盘用户、prefers-reduced-motion | 打开阅读与校准界面 | 布局转抽屉/单栏且保留读说明书与步骤；不支持校准的入口明确禁用并显示解释性提示（不出现坏掉无解释）；减少动效偏好生效；表单错误与字段关联并可读 | [必选] Playwright 窄屏用例 + 人工复核 |

### 4.5 非功能与交付

| AC ID | REQ ID | Given | When | Then（可观察结果） | 测试方式／必选 |
| --- | --- | --- | --- | --- | --- |
| AC-061 | REQ-040 | 具名测试机 + 100k 面/≤4K 贴图模型 | 连续旋转 ≥5 分钟并请求普通元数据 API | 旋转 p95 帧耗时 ≤33ms；无供应商等待的本地元数据 API p95 ≤200ms；模型加载耗时单独记录（不承诺 150 MiB 瞬开）；记录设备、系统、样例与浏览器版本 | [必选] T21 性能记录 + 人工复核 |
| AC-062 | REQ-040 | 100 页 PDF；库列表；同一会话 | 执行准备；打开库列表；连续切换 10 次模型 | PDF 逐页渲染、有进度且可取消，不同时创建 100 张 canvas；首屏库列表不强制加载 3D/PDF；10 次切换后资源数与内存无持续增长趋势 | [必选] Playwright `pdf-preparation.spec.ts` + 人工内存观测记录 |
| AC-063 | REQ-041 | Chrome、Edge、Firefox 当前支持版本 | 执行 T21 回归与 T22 smoke | 三者全部通过并在发布报告写精确版本；Safari 未被列入支持（无实机测试前）；不以 Headless WebGL 结果替代目标设备人工复核 | [必选] `npm --prefix apps/web run test:e2e` + 发布报告 |
| AC-064 | REQ-042 | 目标平台原生构建环境 | 分别执行 `cargo xtask dist --target aarch64-apple-darwin`、`cargo xtask dist --target x86_64-unknown-linux-musl` 并执行 `cargo xtask smoke --binary <绝对路径>` | 每个平台各自产出单文件 + SHA256 + licenses 与动态依赖清单；在原生冷环境启动并通过 smoke；签名/公证未做时在报告中声明；未跑平台不贴支持标签 | [必选] xtask dist/smoke 两平台 |
| AC-065 | REQ-043 | 测试构建 | 执行 `cargo xtask check` 与安全回归集 | 覆盖上传类型伪造、SSRF（重定向到私网、DNS 重绑定）、CSRF/Origin、路径穿越、越权资产、日志脱敏、未知 `/api/*` JSON 404；无静默 skip、测试数 >0；无密钥泄露 | [必选] `cargo xtask check`；安全矩阵见 validation-release.md §3 |
| AC-066 | REQ-044 | 运行中服务 | 查看日志、健康检查与管理页 | 日志含 requestId/jobId/stage/attemptId 且无密码/密钥/原始 PDF 内容/签名 URL 查询串；`/health/live` 与 `/health/ready` 分离且不泄露配置；管理页只显示 provider 是否配置；错误响应含 requestId 且无堆栈/SQL/密钥 | [必选] `cargo test -p everything-manual --test auth_api`（错误结构）+ 人工日志检查 |

## 5. 非功能与数据约束

本节给出 RD 可直接实现的具体数值与语义；与 contracts.md 冲突时以合同为准并回 PM 记录修订。所有数值都是**默认值**，除架构已固定项外可在后续决策中调整并注明。

### 5.1 认证、会话、权限

- 单管理员；`init` 交互输入密码（或受限文件），Argon2 哈希，无默认密码；生成类操作与所有 `/api/v1` 受会话保护（例外：`GET /health/live`、`GET /health/ready`、`POST /auth/login`）。
- 会话 cookie：HttpOnly + SameSite=Strict，HTTPS 时加 Secure；CSRF token 与 Origin 检查覆盖**所有**修改请求（含 multipart）；登录失败限速（默认 5 次/分钟 → 429，可配置）；会话默认绝对有效期 7 天（假设 A-05）；`POST /auth/logout` 撤销会话并清 cookie。
- 权限边界：资产访问先校验 asset 归属（404 处理越权，不泄露存在性）；不提供匿名访问、临时后门、网页磁盘浏览、永久删除 API。
- 默认监听 `127.0.0.1:8080`。MVP 非 loopback 的放行路径只有受信反向代理：必须显式配置 `trusted_proxy_cidrs` 且 TLS 由代理终止，否则拒绝启动；**MVP 不要求内置 TLS 监听**，配置 `tls.*` 时拒绝启动（fail-closed、不静默降级明文，A-15）；反向代理终止 TLS 的部署必须显式设置会话 cookie `Secure`（ADR-013）；不默认相信任意 `X-Forwarded-*`。

### 5.2 费用模型与告知

- **价格快照（2026-09-11）**：Tripo H 系列 `v3.1-20260211`，texture=true、pbr=true、texture_quality=standard、geometry_quality=standard、face_limit=100000、quad=false、generate_parts=false，标准带纹理多视图生成 **30 credits**；1 credit = USD 0.01，即约 **USD 0.30**；网页直接使用 GLB，**不默认加 USDZ 转换费用**。最终以供应商返回与配置价格核算。
- **说明书 AI**：以 USD 计（`usdMicros`，1/1,000,000 USD），模型名与单价由服务端配置（`providers.manual_ai` + price catalog）；报价必须给出保守上界，无法算出可靠上界的型号不进入首版支持清单。
- **金额单位**：整数运算，Tripo `creditMinor`（1/100 credit）、USD `usdMicros`；供应商小数字面量用精确 decimal 解析再转换；禁止浮点累加。
- **报价**：绑定输入哈希、模型参数、页数、最大输出 token、价格版本、预计与保守上界、`expiresAt`（默认 10 分钟）；报价只计算计划，不调用生成服务。
- **确认与冻结**：用户显式同意发送内容后才能提交；`POST jobs` 在同一事务内冻结快照 + 预留费用 + 创建 job；预留、结算、释放均幂等；明确未计费的失败释放预留；**结果未知（unknown）保留预留**，不得填 0。
- **预算语义**：允许上限必须覆盖服务器计算的计划上界，否则 422；预算保证的是"本应用不主动发起超出授权估算的请求"，**不是供应商账户级硬封顶**；不得自动降质量、换模型、增加处理阶段；重生成总是新快照 + 新预算确认。
- **展示要求**：Tripo credits 与 Manual AI USD 分列，不相加成无单位数字；缺价格配置时阻止正式生成并说明缺项。
- 自动重试不产生新的付费提交；真实收费只经 `cargo xtask test-live` 显式授权入口。

### 5.3 输入限制与数据对象约束

| 项目 | 默认限制／规则 |
| --- | --- |
| 原 PDF | ≤50 MiB、≤100 页；**加密 PDF 首版拒绝** |
| 照片 | 仅 JPEG／PNG，单文件 ≤20 MiB；HEIC/WebP 需用户先自行转换（假设 A-03）；webp/其他类型 415 |
| 页图 | 长边 ≤2000 px、白底 JPEG；每次只渲染一页；原点为旋转后 viewport 左上角；bbox 归一化 0..1、左上原点、无可靠框时为 null |
| GLB | ≤150 MiB；三角面 ≤100000；贴图单边 ≤4096；自包含 BIN 与 PNG/JPEG 贴图；拒绝外链 buffer/image URI 与未支持的 required extension |
| 物品累计 | ≤500 MiB |
| 物品字段 | name / brand / model / variant / revision / archived_at；**无物品级 `sourceUrl`**（出处由 `document.source_url` 承载；该字段当作未知字段处理，422，A-16） |
| 磁盘不足 | 服务端磁盘预留不足（预检/复检判定）统一 `413` + `details.reason=insufficientStorage`（含所需/可用字节）且不半提交；不新增 507 或独立错误码（A-14） |
| JSON 请求 | ≤1 MiB（页文字另设更高但有限的上限） |
| 页码 | 1-based（`pageNumber`），服务端与前端统一；旧文档 0-based 不适用 |
| ID / 时间 | 实体 ID 为服务器生成 UUIDv7 字符串；供应商 ID 为 opaque string 不校验不截断；时间为 UTC RFC3339 |
| 列表 | `{data, nextCursor}`，默认 20、最多 100；不返回裸数组 |
| 视图枚举 | front / left / back / right / detail；同一快照每视图最多一张；detail 不进入 Tripo |
| 多视图要求 | 至少 front + left/back/right 之一；缺照片要求用户补齐，不用 AI 造图替代实物照片 |
| 知识聚合 | Part / Step / Evidence / Hotspot / CameraPose / Review 的最小规则见 contracts.md §2；Hotspot 数值必须有限，禁止 NaN/Infinity；anchor 非空时含 modelRevisionId + modelSha256 + positionLocal |
| 乐观锁 | 可编辑聚合根带整数 revision，GET 返回 `ETag: "r7"`；PATCH/确认/发布/重试/取消要求 `If-Match`；缺 428、冲突 412；发布后不改 draft |

### 5.4 可靠性与崩溃语义（摘要，细则见 contracts.md §5）

- SQLite 是任务事实来源；Tokio channel 只唤醒不当队列；一个 data-dir 只允许一个进程持排他锁；WAL + `busy_timeout=5s` + `synchronous=FULL`；连接池上限 4；事务短小，不跨 HTTP 请求持有事务；WAL 要求本地文件系统（不支持 NFS／共享盘多实例）。
- 任务阶段带 `leaseOwner/leaseEpoch/leaseUntil/nextRunAt`；默认租约 120 秒、20 秒续约；状态推进必须校验当前租约 epoch，过期 worker 不得覆盖新结果。
- 退避与等待：安全重试最多 5 次、2/4/8/16/32 秒 + jitter、尊重 Retry-After；远端轮询 3 秒起逐步到 15 秒；总等待默认 30 分钟 → needs_input 并保留 task_id。
- 并发上限：全局远端生成 2、说明书批次 2（可降低，不可未经确认提高）。
- 客户端提交未知（submission_unknown）：先持久化 attempt 与预留，再标记 submitting 后发 POST；收到 ID 立即持久化（过期租约也允许把空 remote_task_id 补成返回值；已有不同 ID 记录冲突并停机告警）；Manual AI 同步链路不提供 attachRemoteTask。
- 崩溃断点（T21 必须逐个验证）：付费 POST 发出前；供应商接受但响应未到；task ID 到达但状态未推进；blob rename 后 DB 事务前；draft 已提交但 HTTP 响应未返回。
- 优雅退出：停止领取任务，完成必要短写入/checkpoint，释放锁；用 hard kill 验证恢复，不只测正常 shutdown。

### 5.5 性能与可访问性基线

- 桌面完整编辑，≥1280px 三栏阅读；768px 以下转抽屉/单栏并保留读说明书与步骤；移动端不支持校准时必须明确禁用并提示。
- 键盘可达所有非 3D 核心操作，有可见 focus、标签与错误关联；部件列表提供 3D 热点的文字替代；支持 `prefers-reduced-motion`。
- 默认 100k 三角面、≤4K 贴图预算下，目标桌面连续旋转 p95 帧耗时 ≤33ms（具名测试机）；模型加载耗时单独记录，不承诺任何 150 MiB 模型瞬开。
- 本地无供应商等待的普通元数据 API，具名测试机 p95 ≤200ms；PDF 准备逐页、有进度可取消，100 页不同时铺满 100 张 canvas。
- 3D/PDF 不在首屏库列表强制加载；连续切换 10 次模型后检查资源数与内存趋势。
- PM 可在编码前按目标设备调整目标，但必须记录原因；不达标时不得由 RD 私自降低阈值（回 PM 修订）。

### 5.6 浏览器基线与必选发布平台（PM 在 T00 明确）

**浏览器**：必须支持 Chrome、Edge、Firefox 的当前支持版本，发布报告写精确版本；Safari 需单独实机测试后才能列入支持；Headless WebGL 结果不替代目标设备人工复核。

**必选发布平台（本轮必须实机验收）**：

| 平台 | triple | 必选理由 |
| --- | --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` | 当前开发环境即 macOS（Darwin 25.x；验证记录 date 见 implementation），是管理员日常使用与内容制作的平台 |
| Linux x86_64（静态） | `x86_64-unknown-linux-musl` | 自托管服务器部署的主要形态；musl 静态链接最符合"单二进制、运行时不装额外依赖"的产品承诺 |

**扩展平台（本轮非必选，各自实机通过后才可声明）**：`x86_64-pc-windows-msvc`、`x86_64-apple-darwin`（Intel macOS）。规则：不得以交叉编译退出码 0 替代运行证据；某平台未跑不得贴支持标签；macOS 签名/公证需要用户账号授权，未做必须在发布报告中声明。

**部署边界（修订 2 裁决 D-4，T22 发布验收须覆盖）**：MVP 只支持两种部署形态——本机 loopback（默认）或位于显式受信反向代理之后的非 loopback（代理负责 TLS 终止与 HTTPS 暴露，应用侧配置 `trusted_proxy_cidrs` 并显式设置会话 cookie `Secure`，见 ADR-013）。**MVP 不包含内置 TLS 监听**：配置 `tls.*` 时服务拒绝启动（fail-closed、不降级明文，A-15），该拒绝即为期望语义。T22 的发布报告与交付说明必须按此声明部署边界，不得宣称"内置证书/开箱即用 HTTPS"；若后续要求内置 TLS 监听，属新增范围（需新卡或 PM 新修订）。

**单二进制内嵌清单**（启动时不得从 CDN 下载）：web HTML/JS/CSS、字体图标、PDF.js worker/CMaps/standard fonts/WASM（同版本）、允许的 3D 解码器、数据库迁移。构建期依赖 Node/Rust/C 编译器与平台 SDK；运行期不要求 Node/Python/PDFium/Tesseract/Chromium/外部数据库/容器。

**离线语义**：外部 AI 不可用时已有资料仍可读（3D/部件/步骤/原文/PDF）；服务停止则网站不可用，这属于部署边界，不包装成离线 PWA 能力。

### 5.7 隐私与日志

- 密钥只在服务端（受限配置文件或环境注入）；浏览器不接触 Tripo / 说明书 AI 密钥；API 密钥不写入日志、报告、导出包。
- 云端发送前展示"将发送什么给谁"；资料视为待分析数据而非指令，模型无工具执行权限，不能按 PDF 内文字改变预算、访问任意 URL 或运行命令。
- 日志不记录密码、密钥、原始 PDF 内容、签名 URL 查询串、整段私人资料；只记 requestId/jobId/stage/attemptId、耗时与错误码等必要摘要；日志脱敏在 T02 有测试覆盖。
- 供应商下载：HTTPS、允许域配置、DNS/每跳重定向与实际连接 IP 校验，拒绝私网/回环/链路本地；不把 bearer token 转发到模型 CDN。
- 备份与导出按部署者数据保护要求管理；不上传到未授权第三方。

## 6. UI 交互设计（由 UI 补充到本 PRD）

本章由 UI 角色在同一修订号（或按需增加修订）内补全。PM 只规定必须存在的信息结构、状态与边界，不定稿页面路由、组件树与视觉方案。UI 交付后本 PRD 冻结为 ui_ready 修订，RD 依此编码。

### 6.0 UI 必须完成的清单（PM 提出的输入）

1. **页面与路由**：给出登录、资料库、新建向导、任务中心、校准工作区、阅读器、设置/状态页的路由表与布局；桌面/窄屏两套布局；说明三栏何时退化为抽屉/单栏。
2. **交互合同表**：为每条交互建立 `UI-xxx` ID，并映射到 §3 的 REQ 与 §4 的 AC（至少覆盖 REQ-002、007、010–039），每行写清操作与状态变化、空/加载/失败/成功/禁用状态、可访问性与测试观察点。
3. **费用与告知界面**：报价页必须分列 Tripo credits 与 Manual AI USD、显示价格版本/快照日期/有效期/保守上界；告知页列出将发送给各供应商的资料类别与范围、模型名；文案不得把本地预算写成供应商账户级硬上限。
4. **错误与恢复界面**：`submission_unknown` 的对账入口（无"重试"按钮）、`needs_input` 的缺项与下一步、`stale` 热点提示、412 并发冲突的刷新提示、限流/断网提示；每类错误有可行动文案与安全返回路径。
5. **准备进度界面**：逐页进度、可取消、关闭标签页会导致准备中断的明确提示；加密/超 100 页 PDF 的拒绝说明。
6. **校准与确认界面**：部件列表↔热点双向联动、步骤导航、原文跳页（1-based）、modelReview 加载/确认两个动作的区分（事实确认 vs 几何校准）、未绑定部件的"仅文本条目"标识。
7. **发布与版本界面**：发布前不满足项的可读列表（来自 422 details）、发布后不可变提示、版本切换（旧发布版继续可读）。
8. **可访问性与降级**：键盘路径、focus、表单错误关联、减少动效、部件列表作为热点的文字替代、移动端禁用校准的提示。

### 6.1 页面与导航

#### 6.1.1 全局框架

- **顶栏（所有页面共用）**：产品名、当前物品名·型号（无当前物品时隐藏）、任务中心入口（带进行中任务计数徽标）、设置入口、登出。顶栏不承载业务提交按钮。
- **断点**：`desktop ≥1280px` 三栏；`mid 768–1279px` 主栏 + 单个可折叠侧栏（侧栏内用标签页切换「部件 / 步骤 / 原文」，不并排显示第三栏）；`narrow <768px` 单栏 + 抽屉。断点用视口宽度媒体查询判定，不用 UA 判断；同一 URL 在三种断点下是同一个页面、同一份数据，只有布局与可用操作不同。
- **深链接与刷新**：除登录页外所有路由可直接打开或刷新；列表状态（分页游标）与向导步骤由 URL 承载，不用内存状态代替。
- **错误呈现**：全局通知条只做摘要（`role="status"` 成功 / `role="alert"` 失败），字段级错误必须落到字段旁并与字段语义关联；每个路由挂错误边界，渲染异常显示可读文案 + `requestId`（响应含有时）+「返回资料库」，不显示堆栈。
- **登录返回路径**：`/login?next=<站内相对路径>`；`next` 只接受同源相对路径，非相对或跨站一律回落 `/`。

#### 6.1.2 路由表

| 路径 | 页面与目的 | 进入条件 | 桌面（≥1280px） | 窄屏（<768px） | UI IDs |
| --- | --- | --- | --- | --- | --- |
| `/login` | 登录（密码 + CSRF 会话建立） | 未登录会话 | 单卡片居中，无顶栏业务入口 | 同左，单列 | UI-001、UI-002 |
| `/` | 资料库：物品列表、空态、搜索、归档切换 | 已登录 | 单栏列表 + 右侧「最近使用/进行中任务」摘要 | 单栏列表，摘要折叠进抽屉 | UI-005、UI-007 |
| `/items/new` | 向导第 1 步：基本信息（新建物品） | 已登录 | 表单居中，步骤条第 1 步高亮 | 单列表单 | UI-006、UI-019 |
| `/items/:itemId` | 物品概览：资料清单、准备状态、草稿与发布列表、下一步入口 | 已登录且物品存在 | 左主栏（资料/草稿/发布）+ 右栏（下一步与状态） | 单栏 + 抽屉 | UI-007、UI-009、UI-010、UI-019、UI-021 |
| `/items/:itemId/edit` | 物品编辑（与新建同一表单组件） | 同上 | 同 `/items/new`，带 ETag/revision 提示 | 单列表单 | UI-006、UI-008、UI-021 |
| `/items/:itemId/import/document` | 向导第 2 步：绑定说明书原件与准备入口 | item 已存在 | 步骤条 + 主栏 | 单列 | UI-009、UI-011、UI-016、UI-019 |
| `/items/:itemId/import/views` | 向导第 3 步：照片上传与视图排列 | 同上 | 主栏为视图网格（front/left/back/right/detail 槽位） | 视图槽位单列堆叠 | UI-009、UI-010、UI-012、UI-013、UI-019 |
| `/items/:itemId/import/prepare` | 向导第 4 步：PDF 逐页准备进度与续传 | document 已绑定 | 步骤条 + 进度区 + 原文缩略预览 | 单列，避免同时显示多张 canvas | UI-014、UI-015、UI-017、UI-018、UI-019 |
| `/items/:itemId/import/confirm` | 向导第 5 步：报价、云端告知、确认与生成 | preparation ready 且视图满足 | 左栏报价与预算、右栏「将发送的资料」告知与确认 | 单列，确认区在报价之后 | UI-004、UI-019、UI-020、UI-022–UI-026 |
| `/jobs` | 任务中心：job 列表与状态筛选 | 已登录 | 单栏列表 + 状态筛选侧栏 | 单列列表 | UI-029、UI-030、UI-033 |
| `/jobs/:jobId` | 任务详情：阶段、费用、错误摘要与全部恢复入口 | 同上 | 左栏阶段列表、右栏费用与操作（取消/重试/对账） | 单列，操作区折叠 | UI-027、UI-028、UI-031–UI-039 |
| `/items/:itemId/drafts/:draftId/review` | 校准工作区：部件↔热点、步骤视角、知识确认、发布 | draft 存在且需复核 | 三栏（见 6.1.3） | 单栏 + 抽屉，校准禁用（见 6.1.4） | UI-042、UI-046–UI-056 |
| `/items/:itemId/releases` | 版本列表：历史发布版与当前草稿状态 | 已登录 | 单栏列表，标注发布版/草稿 | 单列 | UI-054、UI-055 |
| `/items/:itemId/releases/:releaseId` | 阅读器：3D + 部件 + 步骤 + 原文联动、导出 | release 存在 | 三栏（见 6.1.3） | 单栏 + 抽屉，保留读 | UI-043–UI-045、UI-057–UI-061 |
| `/settings` | 设置与状态：provider 是否配置、limits、capabilities、健康状态 | 已登录 | 单栏分区 | 单列 | UI-003、UI-004 |

向导步骤条固定为 5 步：`基本信息 → 说明书 → 视图排列 → 准备 → 预算/隐私确认`；每步一个 URL（上表），上一步/下一步只改 URL，不丢服务端已保存的资料（REQ-016）。

#### 6.1.3 桌面三栏线框（校准工作区与阅读器）

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│ 顶栏  万物说明书 | 相机 A · X100V | [任务 (2)] [设置] [登出]                │
├──────────────────┬──────────────────────────────────┬───────────────────────┤
│ 左栏 280–320px   │ 中栏 flex（3D 视口，懒加载）     │ 右栏 360–420px        │
│ 部件列表         │ 工具栏：[复位] [全部视角] [比例] │ 步骤导航 / 原文页     │
│  · 部件名 状态   │ ┌──────────────────────────────┐ │  · 步骤 1..N 列表      │
│    confirmed/    │ │ R3F canvas                    │ │  · 当前步骤：动作、    │
│    needs_review/ │ │  · 热点标记（可点选）         │ │    警示、引用页码      │
│    stale/未绑定  │ │  · 拾取高亮（校准模式）       │ │  · 原文页图 + 页文字   │
│  · 点击 = 高亮   │ └──────────────────────────────┘ │  · 确认与修订面板      │
│    对应热点      │ 状态行：加载/错误/重建提示        │  · modelReview 两动作  │
├──────────────────┴──────────────────────────────────┴───────────────────────┤
│ 操作区（仅在 /review）：保存视角 | 确认知识 | 发布（发布前检查结果在此展开） │
└─────────────────────────────────────────────────────────────────────────────┘
```

左栏是 3D 热点的**文字替代路径**（REQ-036）：任何只能通过 3D 点选完成的操作，在左栏都有等价的文字入口（选择部件、查看状态、跳转步骤/原文）；发布校验结果也从这里逐条定位。

窄屏对应关系：三个分栏不是消失，而是变为「主视图 + 两个抽屉」。默认进入主视图（阅读器 = 3D；校准 = 只读提示 + 部件列表），抽屉触发按钮固定在顶栏下方。

```text
<768px：单栏 + 抽屉
┌────────────────────────────┐   抽屉 A（部件）   抽屉 B（步骤/原文/确认）
│ 顶栏 物品名  [部件][步骤]☰ │   ┌───────────┐   ┌───────────┐
├────────────────────────────┤   │ 部件列表  │   │ 步骤导航  │
│ 主栏：3D 视口（阅读）      │   │ 状态标签  │   │ 原文页图  │
│       或校准只读提示       │   └───────────┘   └───────────┘
├────────────────────────────┤   抽屉为覆盖层，同一时刻只开一个；
│ 底部：当前选中部件/步骤摘要│   关闭后焦点回到触发按钮（键盘可达）
└────────────────────────────┘
```

#### 6.1.4 窄屏退化与必须明确禁用的能力

| 能力 | ≥768px | <768px | 提示文案要求 |
| --- | --- | --- | --- |
| 阅读器旋转/缩放/复位、点选已确认热点 | 可用 | 可用 | 无 |
| 部件列表选择、步骤导航、原文跳页、导出 | 可用 | 可用 | 无 |
| 文字知识确认（confirmed/needs_review，事实确认） | 可用 | 可用（抽屉内） | 无 |
| 热点绑定/解绑/重新绑定、点选建点（几何校准） | 可用 | **禁用** | 明确写「热点校准需要在 ≥768px 的桌面窗口完成；此处只能查看」并保留只读热点与部件列表；绑定以鼠标点选为主，不提供键盘等价路径（U-11/§8.5） |
| 步骤视角保存/清除 | 可用 | **禁用** | 同上，注明「视角保存属于校准」（U-08/§8.5） |
| 发布 | 可用 | 可用（缺 confirmed 热点等未满足服务端不变量时保持禁用，U-08/§8.5） | 禁用原因需写明「缺少已确认热点：需先在 ≥768px 窗口完成热点绑定」并可跳转校准页；发布本身受服务端不变量约束，不因窄屏放宽 |
| 3D 视口 | 可用 | 可用（只读） | — |

禁用不是隐藏：入口保留为可见的禁用控件 + 原因文案（REQ-039/AC-060 要求「不能坏掉无解释」）。禁用判定以视口宽度为准，窗口拉宽后无需刷新即可恢复可用。

窄屏能力边界按 §8.5 的 U-08/U-11 裁决固定：**窄屏可完成文字知识确认与发布**（发布不是旁路——仍受 REQ-035/AC-055/AC-056 的服务端不变量约束，缺 confirmed 热点时保持禁用并说明原因），**只禁用几何校准（热点绑定/解绑/重新绑定、点选建点）与步骤视角保存**。热点绑定以鼠标点选为主、不承诺键盘等价路径（属 3D 内部拾取，不在「非 3D 核心操作键盘可达」的承诺范围内）；部件列表是热点的文字替代路径（读取与导航），校准页须保留 UI-047 的「热点绑定以鼠标点选为主」注明。若未来要求窄屏校准或键盘绑定热点，属新增需求（PM 新修订）。

#### 6.1.5 键盘、焦点与减少动效（全局规则）

- 所有非 3D 核心操作键盘可达：Tab 顺序按「顶栏 → 主栏 → 侧栏/抽屉 → 底部操作区」；抽屉打开时焦点移入抽屉并做焦点陷阱，Esc 关闭并归还焦点。
- 可见 focus 环，不使用 `outline: none` 无替代；3D canvas 提供可聚焦的等价控件（复位、视角选择）与文字说明。
- 表单错误：错误摘要（锚点到字段）+ 字段级 `aria-describedby` 关联；提交失败后焦点移到第一个错误字段。
- `prefers-reduced-motion: reduce` 下：关闭相机自动动画/缓动、抽屉与面板过渡、加载骨架动画改为静态，模型加载体现在文字进度上。
- 页签/页图/3D 均不作为唯一信息载体：状态用文本标签表达（不只依赖颜色或图标）。

### 6.2 交互合同

| UI ID | REQ / AC | 操作与状态变化 | 空／加载／失败／成功／禁用状态 | 可访问性与测试观察 |
| --- | --- | --- | --- | --- |
| UI-001 | REQ-002 / AC-003, AC-004 | 登录页提交密码 → `POST /auth/login`；成功后持有 HttpOnly + SameSite=Strict 会话 cookie 与 CSRF token，跳转 `next` 或 `/`；登出 `POST /auth/logout`（204）后清空本地查询缓存回 `/login` | 空：仅密码一个字段，无默认值/无占位密码；加载：按钮「登录中…」并禁用；失败：401「密码不正确」、429「尝试过于频繁，请稍后再试」、403 CSRF/Origin 失败提示「请刷新页面后重试」；成功：进入资料库且顶栏出现登出；禁用：密码为空不提交 | 密码框 `type=password` + `autocomplete=current-password`，错误 `role="alert"` 且聚焦错误摘要；测试观察 AC-003 的 401 错误结构、cookie 属性、登出后旧 cookie 再请求 401 |
| UI-002 | REQ-002 / AC-003 | 刷新或直达受保护路由时 `GET /auth/session` 恢复会话；任意 API 401 → 丢弃本地状态跳 `/login?next=<站内相对路径>`；`next` 仅接受站内相对路径 | 加载：全屏骨架，不先闪登录页；失败：「登录已过期，请重新登录」并保留 next；成功：原地恢复页面，不重放已失败请求；禁用：恢复中不触发二次跳转循环 | 焦点在恢复后回到页面主体；测试观察 `GET /auth/session` 带 `Cache-Control: no-store`，登录后回到原路径 |
| UI-003 | REQ-007 / AC-012, AC-013 | 打开 `/settings` → `GET /settings/status`；分区显示 Tripo 与 Manual AI 的 providersConfigured、limits（体积/页数/并发）、capabilities、`/health/live` 与 `/health/ready` | 加载：分区骨架；失败：「无法读取服务状态」+ requestId + 重试；成功：未配置项标「未配置」并附「生成与报价不可用，已有资料仍可读」；禁用：不提供网页填写密钥的输入框，页面不含任何密钥字符；不提供 TLS/证书或监听配置入口（MVP 无内置 TLS 监听，部署边界＝loopback 或受信反向代理，见 §5.6/D-4） | 只读状态不作颜色唯一载体；测试观察 AC-012/AC-013：响应与页面均无密钥，ready 不因云端不可达而失败；页面不出现部署凭据或证书配置项 |
| UI-004 | REQ-007 / AC-012 | 未配置 Provider 或价格目录时，向导第 5 步与物品页的生成入口进入禁用并显示原因 | 空：无报价可显示；失败：estimate/jobs 返回 409 `PROVIDER_NOT_CONFIGURED` / `PRICE_CATALOG_MISSING` → 横幅指明缺项与配置键并链接 `/settings`；成功：不适用；禁用：生成按钮禁用并附带原因（不做点击后静默失败） | 禁用按钮 `aria-disabled` 且可被焦点读到原因；测试观察：不出现 0 费用假成功、不创建 job |
| UI-005 | REQ-010 / AC-016 | 资料库列表：`GET /items`（默认 20/页，游标继续加载）；按名称/型号搜索（A-06 首版无全文检索）；行内入口「继续准备 / 查看任务 / 打开说明书」 | 空：「还没有物品」+ 主按钮「新建物品」；加载：行骨架；失败：「加载失败，重试」且保留已加载行；成功：行显示名称、型号、状态、最近使用时间；禁用：归档物品默认隐藏，可由「显示已归档」开关切出 | 语义列表 + 行内链接，搜索与打开行均可键盘完成；测试观察 AC-016：`{data,nextCursor}` 分页、归档可见性 |
| UI-006 | REQ-010 / AC-016, AC-017 | 新建/编辑物品表单：名称、品牌、准确型号、变体/配置——**无物品级来源链接字段**（A-16/D-1；出处链接只在 UI-011 绑定说明书时写入 `document.source_url`）；`POST /items`（201）或 `PATCH /items/{id}`（If-Match） | 空：新建时字段为空，名称/型号标必填；加载：保存中禁用表单；失败：空白/超长 422 → 字段级错误（`details.fields`）并聚焦；缺 If-Match 428 由客户端修正重试；412 → 见 UI-008；成功：回物品概览并提示已保存；禁用：必填为空时禁用保存；请求体只含合同允许字段，若发送物品级 `sourceUrl` 会被 422（未知字段，不静默忽略） | 字段错误 `aria-describedby` 关联；Tab 顺序等于字段顺序；测试观察 AC-016 空白/超长 422、AC-017 同型号不同配置可并存；表单与请求体均无来源链接字段 |
| UI-007 | REQ-010 / AC-016 | 归档物品：`PATCH /items/{id}` 写 archived；归档后默认列表隐藏，已发布说明书仍可打开 | 成功：行状态变「已归档」并出现「恢复」；失败：412/网络错误保持原状态并提示；禁用：界面不提供「永久删除」入口 | 恢复入口键盘可达；测试观察 AC-016：归档后 release 仍可读、删除路由 404/405 |
| UI-008 | REQ-010 / AC-017 | 通用 412 并发冲突恢复：编辑、确认、发布、重试、取消任一 `If-Match` 操作收到 412 时显示「该内容已被其他操作更新（当前 rN）」+「刷新后重试」 | 失败：展示 `details.currentRevision`，不自动覆盖、不丢弃表单中已有输入；成功：刷新后重新载入并允许再次提交；禁用：刷新前禁用原提交按钮 | 提示 `role="alert"`；测试观察 AC-017 / AC-056：412 与 currentRevision 可见 |
| UI-009 | REQ-011 / AC-018, AC-019 | 共用上传控件：选择文件 → `POST /items/{id}/assets`（multipart file + purpose），按字节显示进度 | 空：显示允许类型（PDF/JPEG/PNG）与上限（原 PDF 50 MiB、照片 20 MiB；HEIC/WebP 需先转换，A-03）；加载：进度条 + 取消；失败：415 类型不支持；413 分两类——输入超限（含物品累计上限，`details.reason=itemTotalLimit`）与**磁盘预留不足**（`details.reason=insufficientStorage`：显示所需/可用字节 + 「磁盘空间不足，请清理后重试」的可行动提示，不得落成通用错误文案，A-14/D-2）；422 解码失败/像素炸弹；一切失败保留此前成功资产；成功：出现资产卡片；禁用：上传中禁止重复提交 | 原生 `input[type=file]` 可键盘触发；测试观察 AC-018 内容去重、AC-019 各错误码且无半提交资产；磁盘满按 `details.reason=insufficientStorage` 零歧义识别并给可行动提示 |
| UI-010 | REQ-011 / AC-019 | 上传失败后的重试与移除：重试沿用同一文件重新上传，不产生孤儿元数据 | 失败：错误卡片带「重试」「移除」；成功：重试成功替换错误卡片；禁用：同一文件的并发重复上传按钮禁用 | 焦点停留在错误卡片并可朗读；测试观察 AC-019：tmp 残留被隔离、共享 blob 不被删除 |
| UI-011 | REQ-012 / AC-020 | 绑定说明书原件：选已上传 PDF 资产 → `POST /items/{id}/documents`（sourceAssetId、title、可选 sourceUrl） | 空：无 PDF 资产时引导先上传；失败：非 PDF 或跨物品引用 → 422 明细；成功：出现 document 卡片与「开始准备」；固定提示：来源链接仅作出处记录，服务器不会访问该地址 | 说明文本与字段关联（`aria-describedby`）；测试观察 AC-020：对 source_url 的外呼次数为 0 |
| UI-012 | REQ-013 / AC-021 | 视图排列：为照片分配 front/left/back/right/detail 槽位，前四者各最多一张；修改 `PATCH /items/{id}/photos/{photoId}`（If-Match） | 空：五个槽位空态，front 标「必需」；失败：同视图第二张 → 「该视图已有照片，请先移除或改选」；成功：槽位显示缩略图与文件名；禁用：detail 槽位注明「特写仅用于理解与核对，不发送给 Tripo」 | 槽位用可键盘操作的按钮组，不自设拖拽唯一路径；测试观察 AC-021：detail 不进入多视图请求体 |
| UI-013 | REQ-013, REQ-016 / AC-021, AC-026 | 缺项提示：报价/生成入口旁列出缺失视图与准备状态缺项，并给下一步链接 | 空：显示「缺 front」「缺侧面视图（left/back/right 至少一张）」；失败：estimate 返回 422 时逐条呈现 `details` 缺项；成功：满足后缺项区消失；禁用：不满足时生成按钮禁用但原因常驻可见 | 缺项与按钮 `aria-describedby` 关联；测试观察 AC-026：缺 front 时按钮不可用并说明缺项 |
| UI-014 | REQ-014 / AC-022 | 准备页逐页渲染与上传：每次只渲染一页（长边 ≤2000px、白底 JPEG），`PUT /preparations/{id}/pages/{n}`（1-based）；进度显示为「第 n / N 页」与已完成页数 | 加载：显示当前页号、已完成页数、已用时；不使用与真实页数无关的百分比；失败：单页失败 → 该页标失败并可「重试本页」，其余页继续；成功：全部页完成后「封存资料」可用；禁用：ready 后写入控件禁用 | 进度用 `role="progressbar"` 且 `aria-valuenow` 为已完成页数；测试观察 AC-022 页资产与 1-based 页码、AC-062 不同时铺满 100 张 canvas |
| UI-015 | REQ-014 / AC-023 | 取消准备与离开提示：进行中「取消」销毁当前 render task 与 canvas；刷新/关闭标签页触发浏览器离开确认 | 加载：取消后回到未开始状态并保留已完成页；失败：已 ready 时提示不可取消；成功：取消后停止上传并保留进度记录；禁用：取消失败期间禁用重复点击；常驻文案「准备需要保持本标签页打开；关闭标签页会中断准备，重新进入只补齐未完成的页」 | 离开提示用 `beforeunload`，测试断言出现确认对话框；测试观察 AC-023：中断后只补缺页 |
| UI-016 | REQ-014 / AC-024 | 加密 PDF 与 >100 页 PDF 的拒绝说明 | 失败：加密 → 「该 PDF 已加密，首版不支持，请先解除加密后再上传」；超页数 → 「PDF 共 N 页，超过 100 页上限」；不创建页记录、不进入 job、不产生收费请求；成功：不适用；禁用：被拒文件不提供「开始准备」 | 错误与文件项关联并可读；测试观察 AC-024：无页记录、无 job、无收费请求 |
| UI-017 | REQ-014 / AC-023 | 断线续传：重新进入同一 document 时 `GET /preparations/{id}` 取名页状态，只渲染并上传缺失页；离线时 PDF.js worker/CMaps/standard fonts/WASM 全部本地加载 | 加载：「已完成 n / N 页，继续补齐」；失败：离线 → 「当前网络不可用，已完成的页仍保留」，恢复后继续；成功：页集合连续 1..N；禁用：无 | 键盘可触发「继续准备」；测试观察 AC-023：无 CDN 请求、只补缺页 |
| UI-018 | REQ-015 / AC-025 | 封存资料：`POST /preparations/{id}/complete`（If-Match + 声明 pageCount）→ ready | 加载：封存中按钮禁用；失败：缺页/哈希不符 422 → 逐条列出缺失页号与资产并提供「继续补齐」；成功：状态显示「准备完成（ready）」；禁用：ready 后页写入与再次封存禁用；complete 不自动发起任何收费任务 | 常驻说明「页图由本机浏览器生成（clientDerived）；哈希只证明字节一致，不证明其确实来自原 PDF，原件保留可复核」；测试观察 AC-025：连续页校验、ready 后写入被拒、complete 不创建 job |
| UI-019 | REQ-016 / AC-026 | 向导五步导航：步骤条与上一步/下一步，URL 即步骤；刷新或返回后从服务端恢复已上传资料 | 空：未上传时每步给引导文案；加载：按需加载当前步数据；失败：该步加载失败可重试且不丢其它步骤资料；成功：当前步高亮、已完成步可回跳；禁用：前置未满足时下一步禁用并说明缺什么 | 步骤条为可键盘导航链接列表；测试观察 AC-026：刷新后资料不丢 |
| UI-020 | REQ-016, REQ-022 / AC-026, AC-031 | 生成按钮与防重复提交：点击后进入「已提交，正在创建任务」并禁用；每轮报价生成一个幂等键，重复点击/断线重试复用同一键 | 空：无报价时不可用；加载：禁用 + 「正在创建任务…」；失败：被拒见 UI-026，网络错误见 UI-030；成功：跳转 `/jobs/{jobId}` 并显示已受理（202）；禁用：报价过期或确认未勾选时禁用 | 按钮 `aria-busy`；测试观察 AC-026/AC-031：重复点击与重放只产生一个 job |
| UI-021 | REQ-017 / AC-027 | 编辑进行中任务所依赖的资料时显示冻结快照提示 | 常驻提示：「当前任务使用提交时的资料快照（快照时间/输入摘要）；你的修改用于下次生成，需要重新报价确认」；失败：不适用；成功：保存编辑后已存在 job 的快照引用不变；禁用：不提供修改使用中快照的入口 | 提示为常驻区块（非一次性 toast）；测试观察 AC-027：输入哈希不变、旧发布版内容不变 |
| UI-022 | REQ-020 / AC-028 | 报价展示（向导第 5 步与物品页）：`POST /items/{id}/estimates` 结果显示 Tripo credits 与 Manual AI USD 分列、价格版本、快照日期、保守上界、`expiresAt` 倒计时 | 加载：报价中显示骨架并禁用生成；失败：见 UI-004/UI-013；成功：两个金额分别带单位（credits / USD）且不相加；禁用：倒计时到期后生成按钮禁用并提示「报价已过期，请重新获取」 | 金额同时以文本朗读单位，不靠位置区分；测试观察 AC-028：整数单位（creditMinor/usdMicros）、无生成调用、无费用记录 |
| UI-023 | REQ-020 / AC-029 | 报价前置缺项的呈现：未 ready、缺视图、缺价格配置分别给不同处理路径 | 失败：缺价格 → 409 `PRICE_CATALOG_MISSING`，文案指向 `/settings` 并阻止生成；准备未完成/缺视图 → 422 逐条列出具体缺项并给修复链接；任何情况不显示伪造精确金额；成功：不适用；禁用：缺项未解决前生成禁用 | 缺项列表 `role="alert"`；测试观察 AC-029：三种情况各自的响应与页面文案 |
| UI-024 | REQ-021 / AC-030 | 云端发送告知与确认：列出送往 Tripo 的图片视图、送往说明书 AI 的页范围/页图/型号文本、模型名与参数、价格版本与预算上界；勾选框默认不勾选 | 空：资料未齐时确认区不可用并说明；加载：勾选后提交按钮启用；失败：未确认提交被拒 → 提示「需要先确认发送内容」；成功：确认动作写入 audit_events，界面显示「已确认发送范围（时间）」；禁用：不提供默认勾选，「全选」按钮也不预先勾选 | 勾选框原生控件，说明文本与勾选框关联；测试观察 AC-030：未勾选不创建 job、默认不预勾选 |
| UI-025 | REQ-022 / AC-031 | 提交生成：`POST /items/{id}/jobs`（Idempotency-Key + quoteId + 输入 IDs + limits）；202 只表示已入队 | 加载：按钮进入提交中并禁用；失败：见 UI-020/UI-026；成功：跳转任务详情，显示「任务已创建，后台继续执行；关闭浏览器不影响已提交任务（需服务进程运行）」；禁用：提交中与已提交后不重复提交 | 文案常驻在任务详情；测试观察 AC-031：同键重放返回同一 job、同键不同 body 返回 409 |
| UI-026 | REQ-022 / AC-032, AC-031 | 提交被拒的处理：报价过期、输入已变、同键不同 body 分别给出不同恢复路径 | 失败：报价过期 → 「请重新获取报价」；输入已变 → 「资料已更新，请重新确认发送内容」并回向导；409 同键不同 body → 「该操作已存在一个任务」并链接已有 job，不新建；成功：不适用；禁用：被拒后保持禁用直到重新报价或确认 | 错误 `role="alert"` 且给出可点击的恢复入口；测试观察 AC-032：被拒不创建 job、无远端请求 |
| UI-027 | REQ-023 / AC-033 | 费用区块（任务详情与物品页共用）：Tripo credits 与 Manual AI USD 分列显示已消耗、预留中、unknown 仍保留的预留、预估偏差 | 加载：费用的骨架；失败：读取失败显示重试且不显示 0；成功：两个币种各自带单位、不相加成无单位数字；unknown 的预留显示为「未决预留（等待对账）」而不是 0；常驻说明「预算上限是本应用的发起上限，不是供应商账户级硬封顶」 | 需朗读的说明与费用区块同组；测试观察 AC-033：分列、unknown 保留预留、语义说明可见 |
| UI-028 | REQ-023 / AC-034 | 超出授权上界被拒与「不自动降质量」的呈现 | 失败：应用拒绝发起 → 「该处理超出本次授权上界，请调整预算或资料后重新确认」；界面不提供降质量/换模型/加阶段的快捷开关；成功：不适用；禁用：重生成入口必须先有新报价确认，未确认时禁用 | 可选地把被拒原因与「重新报价」按钮关联；测试观察 AC-034：无自动降质量、unknown 实际费用不被填 0 |
| UI-029 | REQ-031, REQ-024 / AC-049, AC-035 | 任务中心列表：`GET /jobs`（游标分页），可见页每约 2 秒轮询、页面不可见时降为约 15 秒、终态停止；显示名称/型号、整体状态、阶段摘要、已消耗/预留 | 空：「还没有任务」+「去新建物品」；加载：首屏骨架，轮询不闪屏（增量更新行）；失败：网络错误保留上次数据并标注「数据可能已过期」；成功：终态行显示结果入口（草稿/发布/失败详情）；禁用：终态行不显示无意义操作 | 轮询状态用文本标注最后更新时间；测试观察 AC-049 的 2 秒/15 秒/终态停止与刷新/重启后状态恢复 |
| UI-030 | REQ-031, REQ-024 / AC-049 | 状态区分与「下一步」：`queued / running / waiting_provider / retry_wait / needs_input / submission_unknown / succeeded / failed / cancelled` 各自独立标签与建议操作 | 加载：状态标签出现前不显示猜测值；失败：未知状态原值照实显示 + 「查看详情」；成功：succeeded 明确写「可复核草稿已产出，尚未发布」；禁用：需要人工的状态不显示「重试」按钮（见 UI-034/UI-037）；全页不使用线性总进度百分比 | 状态用文本标签（不只图标/颜色）；测试观察 AC-049：不显示虚假总进度 100% |
| UI-031 | REQ-024 / AC-036 | needs_input 的缺项与下一步：列出可行动缺项（缺视图、资料/schema 不足、超面数等），保留 `task_id`，恢复只查询不重新购买 | 空：不适用；加载：不适用；失败：缺项列表每条给「去补齐」链接；超过 30 分钟总等待转 needs_input 时说明「已停止自动等待，远端任务 ID 已保留，补齐后可继续查询」；成功：补齐后由用户触发继续（不自动重复购买）；禁用：不提供「重新生成一个新任务」的默认入口 | 缺项列表为语义列表，可键盘逐条跳转；测试观察 AC-036：needs_input 列缺项、保留 task_id |
| UI-032 | REQ-024 / AC-036 | 退避重试中的展示：`retry_wait` 显示「第 n/5 次重试，将在 X 秒后重试」并尊重 `Retry-After` | 加载：倒计时实时更新；失败：超 5 次转 failed 并给出错误摘要；成功：恢复 running 后倒计时消失；禁用：重试倒计时期间不提供「立即重试」按钮（避免打断退避） | 倒计时为文本而非动画唯一表达；测试观察 AC-036：退避序列与次数上限 |
| UI-033 | REQ-031 / AC-049 | 断网不误报业务失败：请求失败且为网络层错误时，显示「网络连接异常，正在自动重试（本地状态未变）」 | 加载：自动重试中保留旧数据；失败：区分「网络问题」与「业务失败」（仅服务端明确业务失败才显示失败）；成功：恢复后状态回到服务端实际状态；禁用：网络中断时不显示「任务失败」标签 | 提示 `role="status"` 不打断朗读；测试观察 AC-049：断网显示网络问题而非业务失败 |
| UI-034 | REQ-025 / AC-037 | `submission_unknown` 展示与对账入口：父 job 与对应分支显示「付费提交结果未知，已暂停该分支后续购买」，主操作为「对账」，**无「重试」按钮**；预留保留显示为未决 | 加载：不适用；失败：unknown 分支显示 attempt 时间、已保留预留、最后错误摘要；成功：对账完成后状态按服务端更新；禁用：重试按钮不存在（不是禁用而是不渲染），可用操作只有「对账」与「取消」 | 需向屏幕阅读器可辨（不依赖视觉差异）；测试观察 AC-037：unknown 无重试入口、预留不自动释放 |
| UI-035 | REQ-025 / AC-037, AC-038 | 对账面板：三种动作分别确认——`attachRemoteTask`（仅 Tripo：填写账户中查到的 task ID，验证类型与账号可访问性后二次确认）、`recordNoTask`（必须填写核查证据）、`authorizeReplacement`（再次预算确认 + 明确重复收费风险） | 空：无可选动作时说明原因；加载：动作提交中禁用；失败：验证不通过（类型不符/不可访问）→ 明确失败原因，保留旧 attempt 与未决账务；成功：显示动作结果与审计记录链接；禁用：Manual AI 分支不显示 attachRemoteTask 选项 | 危险动作（authorizeReplacement）需输入确认或二次确认对话框，焦点陷阱 + Esc 取消；测试观察 AC-037/AC-038：三动作合同与「不伪造不存在证明」 |
| UI-036 | REQ-026 / AC-039 | 取消任务：`POST /jobs/{id}/cancel`（If-Match），确认对话明确「不会取消已提交给供应商的付费操作」 | 加载：取消中禁用按钮；失败：412 → UI-008，已终态 → 提示无需取消；成功：未提交阶段显示「已取消」，已提交阶段显示「已停止新推进，保留查询与账务收尾」；禁用：终态任务不显示取消 | 确认对话为可键盘操作的模态；测试观察 AC-039：文案不声称已取消远端付费操作、产生 audit_event |
| UI-037 | REQ-026 / AC-040 | 按分支重试：`POST /jobs/{id}/retry`（If-Match + Idempotency-Key），只列出可重试阶段 | 空：无可重试阶段时说明原因（unknown 状态不提供入口）；加载：重试中禁用；失败：412/幂等冲突按 UI-008/UI-026 处理；成功：只重跑指定阶段并保留已完成成果（界面标注「已完成部分不会被覆盖」）；禁用：重试不改变模型/质量预设，界面不提供修改参数的捷径 | 阶段选择为可键盘操作的单选组；测试观察 AC-040：unknown 无 retry、重放不产生第二个 attempt |
| UI-038 | REQ-024, REQ-027, REQ-031 / AC-041, AC-049 | 生成阶段明细：逐个展示 `tripo_upload / tripo_submit / tripo_poll / model_download / model_validate / manual_extract(batch i/N) / manual_merge / assemble_draft` 与各自状态、用时、尝试次数 | 加载：进行中阶段显示「进行中」与已用时；失败：阶段失败给错误摘要 + requestId；成功：阶段完成后显示产物摘要（模型 revision、部件数、步骤数）；供应商未知状态**保留原值**显示并标注「状态待确认，将自动重新查询」；禁用：不显示编造百分比与预计剩余时间 | 阶段列表可键盘展开/收起；测试观察 AC-041：未知状态原值保留、429 退避 |
| UI-039 | REQ-028 / AC-043, AC-044 | 模型校验失败与 needs_input 提示：超面数、外链资源、不支持扩展、截断 GLB 等，保留原始模型与错误 | 失败：文案指明具体原因（超 100000 面/含外部资源/不支持扩展/文件截断）与下一步（更换资料重生成或按 needs_input 补齐），并注明「原始模型已保留」；成功：模型通过校验后可见于校准工作区；链接过期重取时显示「正在重新获取模型链接（查询已存在的远端任务，不会重新购买）」；禁用：不提供「降低面数/自动修复」按钮 | 原因列表可读；测试观察 AC-043/AC-044：无静默改坏模型、不自动降预算 |
| UI-040 | REQ-029 / AC-045, AC-046 | 知识出处与不确定项展示：每个部件/步骤/规格显示页码出处（1-based）可跳转原文；未被引用、拒答、截断的批次显示「未产出正式知识」 | 空：无知识时说明「本次提取未产出可用知识，可重试知识分支」；加载：按批次渐进显示；失败：拒答/截断/畸形 → 明确标记为未产出，不进入正式知识；成功：出处可点击跳到对应页；不使用「置信度 xx%」这类概率措辞（`confidence` 不是已验真概率） | 出处链接可键盘激活并回到原位；测试观察 AC-045/AC-046：引用页存在、扫描页走页图、合并保留出处 |
| UI-041 | REQ-029 / AC-046 | 批次覆盖与失败批次：显示页覆盖率（已处理页/总页）与每个批次的身份与状态，失败批次可单独重试 | 加载：覆盖率随批次推进更新；失败：失败批次列出页范围与错误摘要并提供「重试该批次」；成功：全部成功且覆盖完整后才出现「合并结果（merge）」区块；禁用：覆盖不完整时 merge 区块显示为不可用并说明原因 | 覆盖率用「已处理页数/总页数」文本；测试观察 AC-046：全部批次成功且覆盖完整才解锁 merge |
| UI-042 | REQ-030 / AC-047, AC-048 | 草稿提示：job `succeeded` 后进入草稿页，横幅明确「已生成可复核草稿（needs_review）——生成完成不等于已发布；需要人工确认知识并完成热点校准后才能发布」 | 空：无草稿时说明「尚未生成」；加载：草稿加载骨架；失败：草稿加载失败可重试；成功：显示「草稿 rN」与待办计数（未确认知识数、未绑定部件数、stale 热点数）；禁用：界面不存在任何「自动发布/一键发布到线上」入口 | 横幅为常驻区块；测试观察 AC-048：releases 为空、status=needs_review、页面明确提示需人工确认 |
| UI-043 | REQ-032 / AC-050, AC-051 | 阅读器 3D：R3F 模块懒加载，进入页面后加载 validated model revision 的 GLB；OrbitControls 支持旋转/缩放/复位；提供加载与错误状态 | 空：无模型时显示文字说明（仍可读部件/步骤/原文）；加载：canvas 区域显示进度与「加载模型…」；失败：错误状态显示原因与「重试加载」，不使整页不可用；成功：模型可操作，顶部提示「模型为外观资产，不表示机械结构」；禁用：模型未通过校验时不进入 3D 视图；卸载或换模型时释放 geometry/material/texture | canvas 外提供可聚焦的等价控件（复位、预设视角——即 U-11/§8.5 约定的键盘等价操作；旋转提示文案）；测试观察 AC-050 的懒加载/清理、AC-051 换模型不串资源 |
| UI-044 | REQ-032 / AC-050 | WebGL context lost/restored 处理：监听 `webglcontextlost` / `webglcontextrestored`，在视口内显示「3D 显示已中断，正在重建…」并提供「立即重建」 | 加载：重建中禁用交互；失败：连续失败显示「浏览器 3D 上下文不可用」+ 文字阅读入口；成功：`restored` 后重建渲染并保留当前相机与选中部件（能重建时）；禁用：不把「刷新页面」作为唯一手段 | 提示 `role="status"`；测试观察 AC-050：forceContextLoss 后可重建 |
| UI-045 | REQ-032, REQ-038 / AC-050, AC-059 | 3D 失败或供应商不可用时的文字/PDF 路径：部件列表、步骤、原文页图与页文字不依赖 WebGL 成功 | 空：不适用；失败：3D 区域显示错误，但部件列表、步骤导航与原文页仍可完整使用；成功：3D 与文字同时可用；禁用：无（文字路径始终可用） | 键盘可从部件列表跳到步骤与原文；测试观察 AC-050/AC-059：3D 失败仍可读文字与 PDF |
| UI-046 | REQ-033 / AC-052 | 校准工作区（桌面三栏）部件↔热点双向联动：点部件列表项 → 3D 高亮对应热点并居中；点选 3D 热点 → 左栏对应部件选中并展开 | 空：无部件时说明「草稿未产出部件」；加载：3D 加载中左栏仍可用（可先读部件）；失败：热点读取失败给重试；成功：选中状态在两侧一致可见；禁用：无部件可绑定时禁用绑定按钮并说明 | 选中项用 `aria-current`/文本标记；测试观察 AC-052：双向联动与选中一致 |
| UI-047 | REQ-033 / AC-052 | 点选绑定热点：进入「拾取模式」后点击模型表面 → 以 asset-root 局部坐标保存 `positionLocal`；人工直接拾取得到 confirmed；未绑定部件 anchor 显示为 null（不写 [0,0,0]） | 空：未绑定部件显示「未绑定」并给绑定入口，且可标记为「仅文本条目」（不进入 3D，发布时保留并明显标识，不自动隐藏）；加载：保存中禁用重复点选；失败：保存失败保留待保存状态并提示，不静默丢弃；成功：热点出现在模型上且左栏状态变「已确认」；禁用：非拾取模式下点击只做选中/旋转，不创建热点 | 热点绑定以鼠标点选为主（U-11/§8.5：绑定属 3D 内部拾取，不在「非 3D 核心操作键盘可达」承诺内，不提供键盘等价路径）；校准页显著注明「热点绑定以鼠标点选为主」，并指明部件列表是热点的文字替代路径（读取与导航）；测试观察 AC-052：数值有限、无 NaN/Infinity |
| UI-048 | REQ-033 / AC-052 | 拖动旋转不误触建点：判定点击与拖动（位移/时长阈值），拖动只改变相机；raycast 只包含模型 mesh 并排除热点自身 | 加载：不适用；失败：不适用；成功：拖动后热点数量不变；禁用：拖动过程中不显示拾取高亮 | 提供「旋转模式/拾取模式」互斥切换按钮（可键盘切换）；测试观察 AC-052：拖动旋转后无新建热点 |
| UI-049 | REQ-033 / AC-053 | stale 热点提示与重新绑定：模型 revision 变化后旧绑定进入 `stale`，不当作有效热点显示；提供重新绑定入口；旧 anchor 可保留作解释 | 空：无 stale 时该区块隐藏；加载：不适用；失败：用旧 sha 提交 confirmed 被 API 拒绝 → 提示「该绑定属于旧模型版本，需要在新模型上重新绑定」；成功：重新绑定后状态回到 confirmed 且 sha 匹配；禁用：stale 热点不能用于发布，界面不为 stale 提供「强制确认」 | 提示与部件条目关联；测试观察 AC-053：旧发布版仍指向旧模型、API 拒绝不符 sha |
| UI-050 | REQ-033 / AC-052 | 步骤视角保存：为步骤保存/清除 CameraPose（positionLocal/targetLocal/upLocal/fov），按 asset-root 计算；「保存当前视角」与「回到该视角」分开 | 空：未保存时显示「未设置视角」；加载：保存中禁用；失败：保存失败提示保留当前视角不丢失；成功：步骤行显示视角已保存标记与「重置」；禁用：窄屏禁用（见 6.1.4 与 §8.5 的 U-08）；FOV 超出合理范围时给字段错误 | 按钮可键盘触发；文案注明「视角是观察位置，不是机械动作」；窄屏禁用属 U-08/§8.5（只禁几何校准与视角保存）；测试观察 AC-052：视角随草稿保存并可复现 |
| UI-051 | REQ-034 / AC-054 | 知识确认与人工修订：实体级 `confirmed / needs_review` 切换；人工修改标 `userEdited` 并保留原始出处；供应商事实快照只读 | 空：无知识时说明未产出；加载：保存中禁用该条目；失败：引用校验失败（页码不存在/部件引用缺失）→ 字段级错误 + 定位；成功：条目显示「已确认（人工，时间）」或「已修订」+ 原文本对照；禁用：供应商快照字段不可编辑（无输入控件，仅「复制为本地修订」） | 状态切换为可键盘操作控件；测试观察 AC-054：userEdited 与出处保留、不能修改快照 |
| UI-052 | REQ-034 / AC-054 | modelReview 两个动作的区分（事实确认 ≠ 几何校准）：动作一「已在浏览器成功打开此模型」（用户声明 loaded），动作二「我已核对模型与资料一致」（用户声明 userConfirmed）；`checkedAt` 由服务器赋值 | 空：未打开模型时两个动作都不可选并说明「请先在 3D 中打开模型」；加载：保存中禁用；失败：保存失败可重试；成功：显示两个动作各自的完成时间与操作者声明，并注明「这是你的复核声明，不是服务端 GPU 测试结论」；禁用：换模型后 modelReview 清空，两个动作回到未完成并提示需重新确认 | 两个动作各有独立标签与说明，不用同一句「已确认」；测试观察 AC-054：loaded/userConfirmed 分离、换模型清空 |
| UI-053 | REQ-034 / AC-054 | 引用校验错误与 `bbox=null` 降级：页码必须 1-based 且存在于本次输入；无法可靠提供 bbox 时显示为空但保留跳页 | 失败：页码不存在 → 条目级错误「引用的第 N 页不在本次提取范围内」；成功：无 bbox 时只显示页图与文字并可跳页，不绘制猜测框；禁用：不提供「手动画框」作为必需步骤 | 错误与条目关联；测试观察 AC-054：bbox 为 null 仍可跳页、不捏造框 |
| UI-054 | REQ-035 / AC-056 | 发布前不满足项列表：`POST publish` 返回 422 时把 `details` 逐条呈现，每条带定位入口 | 空：不适用；加载：发布中禁用按钮；失败：逐条列出（必需知识未确认/缺 confirmed 热点/stale 热点/modelReview 未完成/引用页缺失）并给「去处理」链接；成功：全部满足时按钮可用并显示待发布摘要（模型 revision、部件数、步骤数）；禁用：不满足时按钮禁用且原因列表常驻 | 列表为可键盘导航的语义列表；测试观察 AC-056：422 details 与具体不满足项 |
| UI-055 | REQ-035 / AC-055, AC-053 | 发布成功、不可变提示与版本切换：显示 release 版本、发布时间、模型 revision、manifest 摘要与「已发布版本不可再修改」；版本列表可打开历史发布版（各自指向发布时的模型 revision，旧版继续可读） | 成功：提示「已发布 vN（不可变）：之后对草稿的修改不会改变该版本」并给「打开阅读器」「导出版本」入口；失败：不适用；禁用：已发布版本的编辑入口不存在（不提供"编辑已发布内容"） | 提示为常驻区块；测试观察 AC-055/AC-053：重读 release 字节与哈希不变、旧发布版仍指向旧模型且可读 |
| UI-056 | REQ-035 / AC-056 | 发布/编辑并发 412：提示「草稿已被更新（当前 rN），请刷新后重试」，不自动合并、不覆盖 | 失败：显示 `details.currentRevision` 与刷新按钮；成功：刷新后按最新 revision 重新校验并再次发布；禁用：刷新前禁用发布按钮 | 与 UI-008 共用组件；测试观察 AC-056：并发返回 412 与可恢复路径 |
| UI-057 | REQ-036 / AC-057 | 阅读端四方联动：部件列表 ↔ 3D 热点 ↔ 步骤导航 ↔ 原文页双向联动；引用跳转到正确 1-based 页码并与页图/页文字一致 | 空：无部件/无步骤时分别说明；加载：原文页按需加载，不阻塞 3D；失败：原文页加载失败给重试且不影响步骤浏览；成功：任一侧选择都在其它侧可见同步；禁用：无 | 键盘可从部件 → 步骤 → 原文并返回；测试观察 AC-057：双向联动与页码一致 |
| UI-058 | REQ-036 / AC-057 | 步骤导航：前进/后退/跳步，每步显示动作、警示、引用部件与页码；不累积错误状态 | 加载：切换步骤即时更新高亮与原文；失败：单步引用页缺失时提示该引用不可用但不阻断其它步骤；成功：任意顺序跳转后状态一致（不出现上一步残留）；禁用：首步「上一步」与末步「下一步」禁用 | 步骤列表为可键盘操作列表；测试观察 AC-057：跳步不累积错误状态 |
| UI-059 | REQ-036, REQ-039 / AC-057, AC-060 | 键盘路径与焦点：所有非 3D 核心操作均可键盘完成且有可见 focus；部件列表是 3D 热点的文字替代路径 | 空：不适用；加载：焦点不被骨架夺走；失败：错误摘要获得焦点；成功：Tab 顺序稳定（顶栏 → 主栏 → 侧栏/抽屉 → 操作区）；禁用：无 | 可见 focus 环、`aria-current` 标记选中；3D 只作增强，不做唯一入口（热点绑定等 3D 内部操作不在键盘承诺范围内，U-11/§8.5）；测试观察 AC-057/AC-060：仅用键盘完成部件选择、步骤导航、原文跳页 |
| UI-060 | REQ-037 / AC-058 | 导出入口（阅读器与版本列表）：`GET /releases/{releaseId}/export` 触发下载，显示进行中与失败重试 | 空：无 release 时不显示入口；加载：显示「正在打包…」，可离开页面稍后重试；失败：失败提示原因（网络/权限）并可重试；成功：浏览器开始下载并提示文件名与「包含原件、模型、manifest 与哈希」；常驻说明「导出包是数据便携与灾备，不承诺双击运行网站」；禁用：无 | 下载按钮可键盘触发；测试观察 AC-058：导出自包含包、无绝对路径/密钥 |
| UI-061 | REQ-038 / AC-059 | 供应商不可用或断网时的可读性：阅读端显示横幅「云端服务不可用，已有说明书内容仍可完整阅读」，3D/部件/步骤/原文/PDF 继续可用 | 加载：本地内容不因云端请求失败而阻塞；失败：云端相关操作（生成/报价）显示不可用并说明；成功：不适用；禁用：不出现「离线可用（PWA）」类声明，也不把云端不可用升级为全站错误页 | 横幅 `role="status"`；测试观察 AC-059：断外网仍可读、ready 不失败 |
| UI-062 | REQ-039 / AC-060 | 窄屏布局：<768px 单栏 + 抽屉（部件/步骤/原文），≥1280px 三栏，768–1279px 主栏 + 单个可折叠侧栏（标签页切换） | 空/加载/失败：与桌面同一组件与同一状态，只是布局不同；成功：切换宽度后无需刷新即可切换布局；禁用：同时只开一个抽屉，关闭后焦点归还触发按钮 | 抽屉焦点陷阱 + Esc 关闭；测试观察 AC-060：<768px 转抽屉/单栏且保留读说明书与步骤 |
| UI-063 | REQ-039 / AC-060 | 移动端禁用校准：<768px 时热点绑定/解绑/重新绑定与步骤视角保存入口保留但禁用，并显示解释性提示（U-08/§8.5：窄屏仍可做文字知识确认与发布，只禁几何校准与视角保存） | 禁用：控件可见但不可用，提示「热点校准需要在 ≥768px 的桌面窗口完成；此处只能查看」；成功：窗口拉宽后无需刷新即可用；失败：不出现无解释的空白或报错页 | 禁用原因与控件关联可朗读；测试观察 AC-060：明确禁用并显示解释性提示；窄屏文字确认与发布入口可用（发布受服务端不变量约束） |
| UI-064 | REQ-039 / AC-060 | 减少动效：`prefers-reduced-motion: reduce` 下关闭相机自动动画/缓动、抽屉与面板过渡、骨架动画，模型加载只显示文字进度 | 加载：静态文字进度；失败：不适用；成功：功能不减少，只减少动画；禁用：不使用自动播放式旋转作为信息表达 | 尊重系统设置，无需页面开关；测试观察 AC-060：减少动效偏好生效 |
| UI-065 | REQ-039 / AC-060 | 表单错误与字段关联（全局）：错误摘要锚点 + 字段级 `aria-describedby`，提交失败后焦点移到第一个错误字段 | 空：不适用；加载：错误仅在提交后出现，不在输入过程中闪烁；失败：字段级错误 + 摘要可读；成功：错误清除；禁用：不适用 | 与 UI-006/UI-009/UI-052 等共享实现；测试观察 AC-060：表单错误与字段关联、可读 |

**覆盖核对（UI 自检）**：REQ-002、REQ-007、REQ-010–REQ-039 全部有 UI ID；每个 UI ID 至少映射一条 §4 的 AC（映射见各行第 2 列）。REQ-001/003–006/008/040–044 属后端、CLI、构建与 SRE 范围，出现界面时由 UI-003（设置/状态）、UI-029–UI-033（任务与错误呈现）承载，不另设交互 ID。


### 6.3 设计理由与边界

#### 6.3.1 交互意图与权衡

1. **状态即事实**：界面只显示服务端返回的状态、金额、页码与校验结果；前端不推断"应该已经成功"。因此生成后进入任务详情等待状态（UI-025），不出现"生成成功"的客户端预测文案；这是 REQ-016「前端不得自行判断远端成功」与 REQ-022 幂等语义的界面体现。
2. **每个失败态都有安全返回路径**：401 回登录并保留 `next`（UI-002）、412 给刷新入口而不覆盖（UI-008/UI-056）、needs_input 给缺项与补齐链接（UI-031）、unknown 给对账而非重试（UI-034）、网络错误与业务失败分开（UI-033）。理由：单管理员自托管场景下，一次误操作可能直接造成重复付费或覆盖已确认事实，恢复成本远高于多一次点击。
3. **危险动作明示后果**：取消（UI-036）、对账中的 `authorizeReplacement`（UI-035）、发布（UI-054）都在动作前写清后果（不保证供应商撤单、可能重复收费、发布后不可修改）。这不是免责声明，而是让操作者的判断与他掌握的账务事实一致。
4. **3D 是增强，文字是主路径**：部件列表、步骤、原文在任何 3D 失败、WebGL 不可用或窄屏下都可用（UI-045/UI-057/UI-059）。理由：说明书的首要价值是可读、可核对，而不是必须渲染成功。
5. **可核对性优先于视觉装饰**：信息密度按"技术说明书"排布——白底或中性浅底、细边框、状态用文本标签 + 色点（不靠颜色唯一表达）、哈希/ID/页码用等宽字体、3D 视口用中性背景与常规光照，不使用会改变材质观感的强光、后处理或滤镜，便于用户核验材质与外观是否与实物一致。
6. **不为窄屏维护第二套逻辑**：同一路由、同一组件、同一状态，只在断点切换布局与可用操作（6.1.1），避免"移动端另有实现"导致行为漂移与验收缺口。

**被舍弃的方案及原因**：线性总进度百分比（掩盖"等待人工校准"这一真实阶段，违反 REQ-031）；unknown 上的一键重试（违反 ADR-006 与 REQ-025）；发布按钮旁的一键自动修复（会替用户确认事实，违反 ADR-005）；把 3D 热点做成唯一入口（违反 REQ-036 的键盘与文字替代要求）；在网页内填 API 密钥（违反 REQ-007 与 §5.7）；对错误操作提供"忽略并继续"（会静默跳过发布不变量）。

#### 6.3.2 与 PRD / ADR 边界的一致性

| 边界 | 在交互中的落地 |
| --- | --- |
| ADR-005 人工复核后发布，**无自动发布路径** | 界面不存在任何自动/后台/定时的发布入口；job `succeeded` 一律写作"可复核草稿已产出"（UI-042）；发布是一个显式按钮 + 服务端不变量校验（UI-054/UI-055） |
| 生成完成 ≠ 已核验 | 草稿页横幅与校准页待办计数常驻（UI-042）；AI 产出的部件/步骤默认 `needs_review`，不显示"已确认"（UI-051） |
| 费用告知：本地预算 ≠ 供应商账户级硬上限 | 费用区块与确认页固定说明"预算是本应用的发起上限，不是供应商账户级封顶"（UI-027）；缺价格配置时阻止正式生成并说明缺项（UI-004/UI-023） |
| 不显示虚假线性总进度 | PDF 准备用"第 n / N 页"（UI-014），任务用阶段列表与状态标签（UI-029/UI-030/UI-038），不出现与真实状态无关的百分比与预计剩余时间 |
| 部件列表是 3D 热点的文字替代 | 左栏部件列表承载选择、状态、跳转（UI-046/UI-059）；3D 点选不是创建或读取热点的唯一路径；发布不满足项从列表逐条定位（UI-054） |
| 哈希只证明字节一致，不证明页图来自原 PDF | 准备完成页常驻 `clientDerived` 说明（UI-018），不写"已证明来自原 PDF" |
| `confidence` 不是已验真概率 | 知识条目只显示"已确认/待复核"与出处，不显示概率化措辞（UI-040） |
| 不做离线 PWA 声明 | 云端不可用时横幅只说明已有内容仍可读，不承诺离线使用（UI-061） |
| 磁盘满 = `413` + `details.reason=insufficientStorage`（A-14/D-2，无 507） | UI-009 按 `details.reason` 识别并显示所需/可用字节与清理提示，不落成通用错误；不设计 507 或独立错误码的界面分支 |
| 部署边界：MVP 无内置 TLS 监听（§5.6/D-4/A-15） | 设置页只读展示 provider 配置状态与 limits/capabilities，不含证书/TLS/监听配置入口；界面不出现「内置证书/开箱即用 HTTPS」表述（部署声明属 T22 发布报告，不在界面承诺） |
| 「事实确认」与「几何校准」必须区分 | 文字确认（UI-051/UI-052 的 loaded/userConfirmed）与热点绑定/视角保存（UI-047/UI-050）在文案、位置与操作名称上分开，不使用同一个"确认"字样不加限定 |

**禁用措辞清单（RD/QA 可直接据此检查文案）**：不得出现"已自动校准"、"自动发布"、"总进度 100%"、"已证明页图来自原 PDF"、"供应商账户硬封顶"、"零费用"（未配置时）、"重试不会重复收费"、"离线可用"（除部署边界说明）、把 `confidence` 写成准确率。

#### 6.3.3 UI 层默认假设

| ID | 假设 | 影响范围 | 状态 |
| --- | --- | --- | --- |
| U-01 | 资料库用行式列表（名称/型号/状态/最近使用），不用卡片网格 | UI-005 | 已由 PM 接受（见 §8.5 / ADR-017） |
| U-02 | 时间按浏览器本地时区显示，格式 `YYYY-MM-DD HH:mm`；API 仍为 UTC RFC3339 | 全局 | 已由 PM 接受（见 §8.5 / ADR-017） |
| U-03 | 默认排序：资料库按最近使用倒序、任务中心按创建时间倒序、版本列表按发布时间倒序 | UI-005/UI-029/UI-055 | 已由 PM 接受（见 §8.5 / ADR-017）：三处排序均显示服务端已排序结果（游标绑定排序键），前端不得自行重排 |
| U-04 | 金额显示：credits 两位小数（源自 `creditMinor`），USD 显示 `$` + 精确到 1/1,000,000 的等值可读形式（不因显示丢失精度，禁止浮点累加） | UI-022/UI-027 | 已由 PM 接受（见 §8.5 / ADR-017）：金额来自整数单位（creditMinor/usdMicros），不得浮点累加 |
| U-05 | 报价到期后不自动重新报价，由用户点击"重新获取报价" | UI-022/UI-026 | 已由 PM 接受（见 §8.5 / ADR-017；与 A-02 一致） |
| U-06 | 中段宽度（768–1279px）为"主栏 + 单个可折叠侧栏"，不并排第三栏 | 6.1.1/UI-062 | 已由 PM 接受（见 §8.5 / ADR-017）：与 §5.5 的 ≥1280 三栏、<768 单栏不冲突，不新增 AC |
| U-07 | 成功提示用顶部条，失败信息同时保留在对应区块（toast 不承载唯一错误信息） | 全局 | 已由 PM 接受（见 §8.5 / ADR-017） |
| U-08 | 窄屏（<768px）允许"文字知识确认"与"发布"，只禁用几何校准与视角保存 | 6.1.4/UI-063 | 已由 PM 接受（见 §8.5 / ADR-017）：与 REQ-036/REQ-039 无冲突；窄屏发布不构成旁路，仍受服务端不变量约束，缺 confirmed 热点时保持禁用并说明 |
| U-09 | MVP 界面语言仅中文，不引入 i18n 框架 | 全局 | 已由 PM 接受（见 §8.5 / ADR-017）：文案仍受 §6.3.2 禁用措辞清单约束 |
| U-10 | 登录失败不区分字段原因（单管理员），只提示密码不正确并遵守限速 | UI-001 | 已由 PM 接受（见 §8.5 / ADR-017；与 A-05 一致） |
| U-11 | 3D 视口提供"复位/预设视角"按钮作为键盘等价操作；校准页需注明热点绑定以鼠标点选为主 | UI-043/UI-047 | 已由 PM 接受（见 §8.5 / ADR-017）：与 REQ-036 无冲突——键盘承诺仅覆盖非 3D 核心操作，热点绑定属 3D 内部拾取；校准页保留「热点绑定以鼠标点选为主」注明；若要求键盘绑定热点，属新增需求（PM 新修订） |
| U-12 | 不提供深浅色主题切换，跟随系统仅影响基础亮度 | 全局 | 已由 PM 接受（见 §8.5 / ADR-017）：跟随系统只影响基础亮度，不得降低对比度/可访问性；`prefers-reduced-motion` 仍生效（AC-060） |

以上 U 项均不改变 §3/§4 的需求与验收语义；裁决、理由与补充约束见 §8.5（ADR-017），状态列已全部更新为「已由 PM 接受」。若其中某项后续要升格为 REQ/AC，必须由 PM 新修订处理，UI 再更新受影响 UI ID。


## 7. 实施切片与依赖（REQ ↔ T01–T23 映射）

切片按 implementation-plan.md 的 M0–M4 里程碑划分。规则：当前切片所有依赖已验证才派 RD；只有卡级 PASS 不代表产品 PASS；全部切片完成后进行当前 PRD 修订的独立全量验收。

### 7.1 REQ → 任务卡映射

| REQ | 主要任务卡 | 说明 |
| --- | --- | --- |
| REQ-001 | T01、T22 | T01 最小二进制与内嵌资源；T22 正式包与冷启动 |
| REQ-002 | T04 | 认证、会话、CSRF、限速 |
| REQ-003 | T02 | CLI/配置/日志/data-dir 锁 |
| REQ-004 | T03 | 迁移与 repository |
| REQ-005 | T02（占位）、T20 | backup/restore 未实现期非零返回 |
| REQ-006 | T01、T21 | 合同生成与无漂移检查 |
| REQ-007 | T02、T04、T12、T14、T22 | 未配置不假成功、不回落 mock |
| REQ-008 | T05 | fixture 设施与隔离 |
| REQ-010 | T07 | 物品、乐观锁、归档 |
| REQ-011 | T06 | 上传、资产服务、Range |
| REQ-012 | T07 | 文档绑定 |
| REQ-013 | T07 | 照片与视图 |
| REQ-014 | T09 | 浏览器 PDF 准备与续传 |
| REQ-015 | T09 | preparation complete |
| REQ-016 | T08（框架）、T16 | 库与向导 |
| REQ-017 | T07、T11、T15 | 快照隔离与下游失效 |
| REQ-020 | T11 | 报价与价格快照 |
| REQ-021 | T11、T16 | 云端告知与确认 |
| REQ-022 | T11 | 冻结、预留、幂等 |
| REQ-023 | T11、T17 | 费用分列与预算语义 |
| REQ-024 | T10 | 持久执行器、租约、恢复 |
| REQ-025 | T10、T15 | unknown 与对账 |
| REQ-026 | T10、T15、T17 | 取消与分支重试 |
| REQ-027 | T12 | Tripo v3 适配 |
| REQ-028 | T13 | 下载、校验、不可变版本 |
| REQ-029 | T14 | 说明书 AI 与证据校验 |
| REQ-030 | T15 | 组装草稿 |
| REQ-031 | T17 | 任务中心 |
| REQ-032 | T18 | GLB 阅读器与资源恢复 |
| REQ-033 | T19 | 热点校准 |
| REQ-034 | T19 | 知识确认与修订 |
| REQ-035 | T19 | 发布 |
| REQ-036 | T18、T19 | 联动阅读 |
| REQ-037 | T20 | 导出 |
| REQ-038 | T21、T22 | 离线与降级 |
| REQ-039 | T16、T18、T19 | 窄屏与降级 |
| REQ-040 | T18、T19、T21 | 性能基线 |
| REQ-041 | T21、T22 | 浏览器矩阵 |
| REQ-042 | T22 | 必选平台发布 |
| REQ-043 | T21 | 安全回归 |
| REQ-044 | T02、T04、T17 | 可观察性 |

### 7.2 切片顺序与覆盖

| 切片 | 任务卡 | 覆盖 REQ（该切片内首次交付） | 交付门禁 |
| --- | --- | --- | --- |
| S1（M0 基础） | T01–T05 | REQ-001、002、003、004、006、007（部分）、008 | 最小二进制 + 认证 + 存储 + fixture 通过 |
| S2（M1 数据入口） | T06–T11 | REQ-010–017、020、021、022、023（部分）、024（部分） | 资料可落盘、可恢复准备、可创建幂等 job |
| S3（M2 生成闭环） | T12–T15 | REQ-027、028、029、030、024、025 | fixture 下端到端草稿，崩溃不重复购买 |
| S4（M3 用户闭环） | T16–T20 | REQ-016、031、032、033、034、035、036、037、023、026、039 | 向导→任务→校准→发布→导出全链路 |
| S5（M4 交付） | T21–T23 | REQ-005（最终）、038、040、041、042、043、044、001/002（最终） | 全量 fixture E2E、必选平台冷启动、授权真实链路 |

依赖与停止条件沿用 implementation-plan.md §1：需求不清回 PM、交互缺失回 UI、权限/预算/凭据缺失记 BLOCKED，不用 mock 冒充解决。

## 8. 假设、问题与决策

### 8.1 默认假设（PM 已采用，不阻塞开工；如与用户真实意图冲突需修订 PRD）

| ID | 假设 | 理由／来源 | 影响 |
| --- | --- | --- | --- |
| A-01 | 单管理员、默认 `127.0.0.1:8080`、非 loopback 的 MVP 放行路径为受信反向代理（见 A-15） | ADR-007 | REQ-002/003；未来多用户走 E04 |
| A-02 | 报价有效期 10 分钟；价格快照日期 2026-09-11 | contracts.md §4 + 架构价格快照 | REQ-020/022 |
| A-03 | 照片仅接受 JPEG/PNG，HEIC/WebP 由用户先转换 | 架构 §5.3 与 Tripo 上传格式差异 | REQ-013 的 415 行为 |
| A-04 | 视图枚举 front/left/back/right/detail，方向以物品自身为参照 | 旧文档 §4 与架构 §5.3 结合 | REQ-013/027 |
| A-05 | 会话绝对有效期 7 天；登录失败限速 5 次/分钟（均可配置） | PM 采用的安全默认值 | REQ-002/AC-003 |
| A-06 | 首版不做全文检索，只按名称/型号 | contracts.md §2 | 库列表交互 |
| A-07 | 必选发布平台为 aarch64-apple-darwin + x86_64-unknown-linux-musl | 见 §5.6 | REQ-042/AC-064 |
| A-08 | 页图 2000px/白底 JPEG、租约 120s/20s、并发 2/2、退避 2/4/8/16/32s、轮询 3→15s、30 分钟转 needs_input | 架构 §5.1/§6 已定值，本文引用 | REQ-014/024 |
| A-09 | data-dir 单进程排他，不支持 NFS/共享盘多实例 | 架构 §6 | REQ-003/004 |
| A-10 | Manual AI 参考适配器为 OpenAI Responses，模型名由服务端配置，不跟随"最新模型" | 架构 §5.2、ADR-004 | REQ-029 |
| A-11 | 费用分列的口径为 Tripo credits（creditMinor）+ Manual AI USD（usdMicros） | contracts.md §1 | REQ-023 |
| A-12 | 生成任务在不满足输入/预算/配置时以 4xx 拒绝并且不创建 job | contracts.md §4 | REQ-020/022 |
| A-13 | 未配置 Provider/价格时的错误码取 409 `PROVIDER_NOT_CONFIGURED` / `PRICE_CATALOG_MISSING`（业务前置条件缺失，非服务不可用） | PM 决定；503 保留给 DB/迁移/数据目录不就绪 | REQ-007/AC-012；RD 若认为需改码，走 NEEDS_PRD_CHANGE |
| A-14 | 磁盘预留空间不足统一返回 `413` + `details.reason=insufficientStorage`（含所需/可用字节），**不新增 507 或独立错误码** | 裁决 D-2（2026-09-12）：T06 已实现并经 QA 真实磁盘镜像实测（`disk-full-real.log`）；contracts §1 稳定码集合无 507，AC-019 只要求"明确错误"；`details.reason` 可零歧义识别 | REQ-011/AC-019；T06 无需返工。**合同留痕请求**：请协调者在 contracts §1/§7 补记该形态（PM 不改合同）。写流中途 ENOSPC 现为 500 属 T06 已记录的非阻断项，不属本裁决范围 |
| A-15 | MVP 部署边界：默认 loopback；非 loopback 仅支持显式受信反向代理（TLS 由代理终止）；**不要求内置 TLS 监听**；配置 `tls.*` 时 fail-closed 拒绝启动、不降级明文 | 裁决 D-4（2026-09-12）：architecture §7 是"内置 rustls **或**受信反向代理"的或关系，代理路径已满足"必须认证和 TLS"；T02 实测即 fail-closed（exit 6）；T01–T23 无"内置 TLS 监听"实现卡，MVP 承诺必须在范围内可兑现 | REQ-003/AC-006；T02 无需返工（行为未变，见 §9.1 影响面）；T22 发布报告须声明部署边界、不得宣称内置 TLS；后续要求内置 TLS 属新增范围 |
| A-16 | **取消物品级来源链接**：REQ-010 输入不含"来源链接"；出处链接仅由 `document.source_url` 承载（REQ-012/UI-011）；物品接口收到物品级 `sourceUrl` 按未知字段 422（不静默） | 裁决 D-1（2026-09-12）：contracts §2/§3 与迁移均无物品级列，无任何 AC 覆盖该字段；QA 回合 7 建议 B（改动最小、与已验收实现一致）；保留需加列 + 迁移 + DTO/OpenAPI 重生成 + 重验，且无功能收益 | REQ-010/REQ-012；T07 无需返工；§6.2 UI-006 的表单字段由 UI 在 ui_revision 2 移除 |

### 8.2 开放问题与待验证项（不阻塞 T01–T22 编码）

**开放问题（需用户或外部条件，缺失时对应任务记 BLOCKED，不伪造通过）**

| ID | 问题 | 影响 | 当前处理 |
| --- | --- | --- | --- |
| Q-01 | Tripo 账户区域与凭据、Manual AI 凭据、T23 真实资料的授权与一次性预算 | T23 真实链路验收、最终发布 gate 的"真实 API 已验证"项 | T01–T22 用 fixture 推进；T23 派发时向用户索取 |
| Q-02 | 说明书 AI 的实际模型名与单价（用于报价精度的真实数值） | 报价数值准确性 | PRD 只要求"配置化 + 保守上界"；具体数值由 T23 记录 |
| Q-03 | Linux 目标发行版与 CA 信任根方案（rustls 内置根 vs 系统根） | T22 冷环境真实 HTTPS 验证 | 默认按 rustls 方案实现，T22 记录实测 |
| Q-04 | macOS 最低系统版本、签名/公证安排 | 发布声明与分发方式 | 未决定前不声明签名支持；T22 报告写明 |
| Q-05 | 试点真实物品样本（≥1 件，含扫描页）与参考部件/步骤清单 | 内容质量验收（人工对照基线） | 编码期不影响；T23/QA 内容验收前提供 |

**待验证项（不得写成现有能力）**

| ID | 待验证 | 计划验证点 |
| --- | --- | --- |
| V-01 | Tripo 真实请求/响应、模型质量、credits 实际值 | T23（AC-042） |
| V-02 | Manual AI 真实模型效果、refusal/incomplete 行为、实际费用 | T23 |
| V-03 | 必选平台冷启动与系统动态依赖 | T22（AC-064） |
| V-04 | 100k 三角面模型在目标桌面的帧率实测 | T21（AC-061） |
| V-05 | 自动提取/热点候选准确率指标 | 本期**不承诺**自动热点候选（E01）；抽取质量只作记录 |

### 8.3 必须显式写入的范围假设（ADR-005）

**ADR-005 范围假设（本期生效，不得由 RD 默认扩大）**：系统自动产出模型与知识草稿；**发布必须经人工确认事实并完成表面热点校准**；自动热点候选属后续阶段 E01。由此派生：

- job `succeeded` 只表示"可复核草稿已产出"，不等于 published（REQ-030）。
- 不能承诺"全自动准确拆解"；模型质量指标只在样本上测量并如实报告。
- 热点与 modelReview 必须绑定精确 modelRevisionId + modelSha256，旧发布版不可变。
- 如果用户要求"无人复核直接发布"，必须由 PM 重新处理风险与验收（新修订 + 新 AC），RD 不得默认实现。

### 8.4 依赖与实施约束（提醒 RD/QA 的交叉规则）

- 冲突解决顺序：用户最新要求 > 当前 PRD > contracts.md > architecture.md > decisions.md；实现与需求不符必须修复或走需求变更，不能"以实现为准"。
- 默认测试禁止真实收费 API；fixture 与真实 Provider 显式隔离；未配置服务返回未配置状态，不能假成功。
- 本 PRD 修订 2 的第 6 章需 UI 按 §9.2 清单同步（UI-006 去掉物品级来源链接、§6.3.3 更新 U 裁决）并冻结为 ui_revision 2；在此之前不得派发依赖这些 UI 条目的界面工作（T08/T16 的相关部分）。
- 修订 2 裁决的落地约束（不得越界扩大）：items 不得新增物品级 `source_url` 列/字段；磁盘不足一律 `413` + `details.reason=insufficientStorage`（不新增 507）；MVP 不实现内置 TLS 监听（配置 `tls.*` 即 fail-closed 拒绝启动）。上述变更须由 PM 新修订才能调整（A-14/A-15/A-16）。

### 8.5 UI 层假设裁决（U-01–U-12，修订 2，PM）

来源：§6.3.3（ui_revision 1）。裁决日期 2026-09-12。**本裁决不新增 REQ/AC、不改变任何验收阈值**；UI 在 ui_revision 2 中按 §9.2 清单同步 §6 文本。

| U | 裁决 | 理由／补充约束 |
| --- | --- | --- |
| U-01 | 接受为产品决定 | 单管理员小规模物品库，行式列表更利于展示状态与下一步入口；卡片网格无功能增益 |
| U-02 | 接受 | 仅影响显示（浏览器本地时区，`YYYY-MM-DD HH:mm`）；API／导出／日志仍为 UTC RFC3339（contracts §1 不变） |
| U-03 | 接受（含补充） | 三处默认排序均显示服务端已排序结果；顺序以服务端返回为准（游标绑定排序键），前端不得自行重排 |
| U-04 | 接受 | credits 两位小数、USD 精确可读表示；金额来自整数单位（creditMinor/usdMicros），不得引入浮点累加（AC-028/AC-033 语义不变） |
| U-05 | 接受 | 与 A-02 一致；报价到期后由用户显式"重新获取报价"，不自动重复创建付费任务 |
| U-06 | 接受 | 768–1279px 为主栏 + 单个可折叠侧栏（标签页切换）；与 §5.5 的 ≥1280 三栏、<768 单栏不冲突，不新增 AC |
| U-07 | 接受 | 成功提示用顶部条，失败信息同时保留在对应区块；toast 不承载唯一错误信息 |
| U-08 | 接受为产品决定（与 REQ-036/REQ-039 **无冲突**） | REQ-039/AC-060 只要求窄屏保留"读说明书与步骤"、对不支持的校准"明确禁用并给解释性提示"，未要求窄屏可校准；窄屏发布不构成旁路——发布仍受 REQ-035/AC-055/AC-056 服务端不变量约束，缺 confirmed 热点时保持禁用并说明（UI-054/UI-063）；文字知识确认在抽屉内可用（AC-054 语义不变） |
| U-09 | 接受 | MVP 界面仅中文、不引入 i18n 框架；文案仍受 §6.3.2 禁用措辞清单约束 |
| U-10 | 接受 | 与 A-05 一致：单管理员、不区分登录失败原因、遵守登录限速 |
| U-11 | 接受（与 REQ-036 **无冲突**） | REQ-036/AC-057 的键盘承诺范围是"所有**非 3D** 核心操作"；热点绑定/点选建点属 3D 内部拾取操作，不在该承诺内，且部件列表已提供热点的文字替代路径（读取与导航）；校准页必须保留 UI-047 的"热点绑定以鼠标点选为主"注明。若未来要求热点绑定完全键盘可达，属新增需求（非 MVP，PM 新修订） |
| U-12 | 接受（含补充） | 不提供主题切换；跟随系统只影响基础亮度，不得降低对比度/可访问性；`prefers-reduced-motion` 仍生效（AC-060） |

本表覆盖 U-01–U-12 全部条目（含修订 1 中标注"UI 默认"的 U-05/U-07/U-10）。UI 在 §6.3.3 按上表更新状态列并引用本节；任何 U 项若后续要求升格为 REQ/AC，必须由 PM 新修订处理。

## 9. 变更记录与交接

### 9.1 修订记录

| 修订 | 日期 | 作者 | 变更 | 影响 |
| --- | --- | --- | --- | --- |
| 0 | 2026-09-11 | 协调者 | request.md 记录初始需求 | 无 |
| 1 | 2026-09-11 | PM | 首版明细 PRD：41 条 REQ、66 条 AC、非功能与数据约束、必选发布平台、T01–T23 映射、假设与开放问题 | 全量新建；第 6 章留给 UI |
| 1（UI 补充） | 2026-09-11 | UI | 同一修订内补全第 6 章：§6.1 路由与三栏/抽屉布局、§6.2 UI-001–UI-065 交互合同（含费用告知、错误与恢复、准备进度、校准与确认、发布与版本、可访问性与降级）、§6.3 设计理由、边界一致性与默认假设 U-01–U-12 | 未新增/修改 REQ、AC 与非功能数值；前端切片（T08、T09、T16–T19）按 §6 实现 |
| 2 | 2026-09-12 | PM | 裁决四项挂起决策：D-1 取消物品级来源链接（出处由 `document.source_url` 承载，A-16）；D-2 磁盘满沿用 `413` + `details.reason=insufficientStorage`（A-14）；D-3 U-01–U-12 全部接受并记录 U-08/U-11 与 REQ-036/REQ-039 的无冲突判定（§8.5）；D-4 MVP 不要求内置 TLS 监听、部署边界＝loopback 或受信反向代理、fail-closed 为期望语义（A-15）。改动章节：§1.3、§2.2-5、§3 REQ-003/010/011、§4 AC-006/AC-019、§5.1、§5.3、§5.6、§8.1（A-01 修订，新增 A-14/A-15/A-16）、§8.4、§8.5（新增）、§9.2 | 影响面（AC 变化 → 需重验切片）：未新增 REQ/AC、未降低任何阈值。**AC-006（T02）与 AC-019（T06）为文本澄清**——命名已实现且已在 QA 回合 2/6 现场记录的行为（回合 2 `manual-cli-run1.log`：配置 `tls.*` 即 exit 6 fail-closed；回合 6 `disk-full-real.log`：真实磁盘镜像 413 + `insufficientStorage` 且无半提交），无阈值变化 → **T01–T07 无需重验（无 needs_retest）**。REQ-010 移除的物品级字段无任何 AC 覆盖，T07 行为（未知字段 422）不变 → 无返工。§6 受影响条目（UI-006、§6.3.3）由 UI 输出 ui_revision 2。T22 发布报告须声明部署边界（§5.6/A-15）。合同留痕请求：contracts §1/§7 补记磁盘满形态（A-14），由协调者处理 |
| 2（UI 同步） | 2026-09-12 | UI | 按 §9.2 清单与 ADR-017 将 §6 同步到修订 2，冻结为 **ui_revision 2**：UI-006 移除物品级「来源链接」字段（D-1/A-16，出处链接仅由 UI-011 的 `document.source_url` 承载）；§6.3.3 将 U-01–U-12 状态列全部改为「已由 PM 接受（见 §8.5 / ADR-017）」并改指 §8.5（含 U-03/U-04/U-09/U-12 补充约束、U-08/U-11 无冲突判定）；UI-009 明确磁盘满按 `413` + `details.reason=insufficientStorage` 识别并给可行动提示（D-2/A-14）；§6.1.4 与 UI-047/UI-050/UI-059/UI-063 引用 U-08/U-11（窄屏可文字确认与发布、只禁几何校准与视角保存；热点绑定以鼠标点选为主、部件列表为文字替代路径）；UI-003 与 §6.3.2 明确设置页不含 TLS/证书配置入口（D-4/A-15）。改动章节：§6.1.4、§6.2（UI-003/UI-006/UI-009/UI-043/UI-047/UI-050/UI-059/UI-063）、§6.3.2、§6.3.3、§9.1 | 未新增/修改 REQ、AC 与任何验收阈值；未新增 UI ID（UI-001–UI-065 编号保持不变）；§6 冻结为 ui_revision 2，T08/T16 相关界面工作可据此派发 |

### 9.2 交接说明

**给 UI（修订 2 增补）**：修订 1 的补全要求维持不变。修订 2 需在 ui_revision 2 内完成的清单：(1) §6.2 UI-006 移除物品级"来源链接"字段（D-1/A-16；出处链接在 UI-011 的 document 绑定处填写）；(2) §6.3.3 状态列按 §8.5 更新 U-01–U-12 的 PM 裁决并引用 §8.5，同时更新该表末尾的说明句（不再写"若 PM 认为其中某项应成为需求…"，改为指向 §8.5）；(3) 建议（非必选）：UI-009 的磁盘满文案可注明按 `details.reason=insufficientStorage` 识别（A-14），UI-047/UI-063 可引用 §8.5 的 U-11/U-08 裁决。不得改动 §1–§5、§7、§8 的语义，如需调整交互导致需求变化，返回 PM 增加修订号。UI 完成后本修订冻结为 ui_revision 2。

**给 RD（修订 2 增补）**：除既有要求外：不得新增物品级 `source_url` 列/字段（A-16，合同变更须经 PM）；不得新增 507 或改动磁盘满的 `413` + `details.reason=insufficientStorage` 形态（A-14）；不实现内置 TLS 监听，`tls.*` 配置保持 fail-closed 拒绝启动（A-15）。

**给 QA（修订 2 增补）**：除既有要求外：AC-006/AC-019 的修订 2 文本与既有回合 2/6 证据一致，不要求重跑 T02/T06（除非协调者另行派发）；T22 验收须核对发布报告按 §5.6/A-15 声明部署边界（不得宣称"内置证书/开箱即用 HTTPS"）；T08/T16 验收需核对物品表单与接口无来源链接字段、窄屏 U-08/U-11 行为符合 §8.5。

**llmdoc 更新**：修订 1 新增 `llmdoc/requirements/web-mvp/prd.md` 与第 6 章；修订 2 更新本文件（§1.3/§2.2/§3/§4/§5.1/§5.3/§5.6/§8.1/§8.4/§8.5/§9.1/§9.2）并在 `llmdoc/decisions.md` 追加 ADR-017（D-1–D-4 裁决的跨需求留痕）。A-13–A-16 属 PRD 级产品决定；如后续确认影响跨需求架构，再由 PM 提炼为 ADR。

