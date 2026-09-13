# 决策与非代码知识

更新：2026-09-11。`accepted` 表示本次方案采用，不代表已实现；`verified` 必须附测试／运行证据；旧决定用 `superseded` 指向替代条目。所有角色开工先查与自己相关的条目，不能只凭上轮聊天。

## ADR-001 — 网站替代 macOS 原生路线

- 状态：accepted，未实现；日期：2026-09-11。
- 结论：React+TypeScript前端、Rust后端，分离开发、合一发布。
- 原因／依据：用户明确要求前后端分离网站、React／Rust、单二进制；旧macOS文档已标为历史。
- 影响：不继续SwiftUI/SceneKit/USDZ，不要求先恢复旧Swift源码；GLB是网页运行资产。见 [架构](architecture.md)。

## ADR-002 — 单文件运行不等于无数据目录或跨平台通用

- 状态：accepted，构建／冷启动待T01/T22验证；日期：2026-09-11。
- 结论：每OS/CPU一个binary，内嵌SPA/运行静态资源/SQLite/迁移，数据独立落data-dir。
- 原因：可执行资源适合内嵌，用户新增大模型和数据库需要持久可写空间；保留前后端HTTP边界。
- 影响：运行不依赖Node/Python/Redis/PDF程序；构建依赖不等于运行依赖；备份和升级必须管数据。详见 [发布](validation-release.md)。

## ADR-003 — 浏览器完成PDF页准备

- 状态：accepted，兼容性／内存待T09验证；日期：2026-09-11。
- 结论：PDF.js提页文字和页图，完整上传后Rust负责后续AI任务。
- 原因：首版避免原生PDF渲染器与OCR进程破坏单binary目标；不是宣称Rust无法渲染PDF。
- 影响：准备时标签页需保持打开，中断可续传；提交后后台可继续；哈希不证明派生页图真实性。首版单管理员信任边界见 [合同](contracts.md)。

## ADR-004 — Tripo几何与说明书知识分开处理

- 状态：accepted；外部格式已查官方文档，实际账号与产出未验；日期：2026-09-11。
- 结论：Tripo v3生成GLB，独立ManualAiProvider提取有出处的部件／步骤；参考OpenAI Responses HTTP适配器，模型可配置。
- 原因：生成形状与读懂操作手册是两类问题；Tripo分件不能证明机械语义。
- 影响：两套密钥／费用／错误／快照，输入隐私明确告知；不额外默认USDZ收费。证据链接在 [架构](architecture.md)。

## ADR-005 — 自动草稿与人工复核分阶段

- 状态：accepted 的MVP范围假设，T00须进入PRD；未实现；日期：2026-09-11。
- 结论：自动产出模型与知识草稿；首版人工确认事实／表面热点后发布，自动候选作为下一阶段。
- 原因：单张外观网格没有可靠部件语义和真实机械结构；错误绑定会误导说明书使用者。
- 影响：job成功不代表已发布；不能承诺全自动准确拆解；modelReview与热点必须绑定精确hash，旧发布版不可变。如果用户要求无人复核发布，PM必须重新处理风险和验收，不由RD默认扩大。

## ADR-006 — 持久化阶段任务，拒绝未知结果盲重购

- 状态：accepted，崩溃测试待T10/T15；日期：2026-09-11。
- 结论：SQLite保存job、批次、attempt、租约、预算与receipt；付费提交未知进入submission_unknown，不能自动重购。
- 原因：本地事务无法原子包含远端付费POST；检查过的Tripo文档不足以提供应用可依赖的exactly-once创建保证。
- 影响：过期worker仍可保存不可变receipt但不能推进状态；同步ManualAI逐批保存完整结果和usage，不能照抄Tripo轮询恢复；错误对账需要管理员。见 [合同第5节](contracts.md#5-任务阶段与崩溃语义)。

## ADR-007 — 单管理员自托管与安全默认值

- 状态：accepted 的首版默认，T00确认部署范围；日期：2026-09-11。
- 结论：默认loopback+认证，公共访问必须TLS／可信代理；无任意URL抓取、无数据目录静态暴露、无生产mock回退。
- 原因：用户未要求多租户；原始说明书、照片与供应商密钥都需保护；容易部署不能以匿名危险接口换取。
- 影响：未来SaaS需新PRD；供应商下载需域名／每跳DNS及实际连接约束；已有资料不依赖云端在线。

## ADR-008 — 主会话编排四角色，llmdoc持久交接

- 状态：配置已落盘，静态检查；真实Claude模型串联尚未运行；日期：2026-09-11。
- 结论：PM→UI同PRD→RD→QA→RD修复→QA复验；主会话维护state，不依赖角色间嵌套调度。
- 原因／依据：用户明确角色和回路；每个角色必须提炼代码不能表达的知识到llmdoc。主会话最终检查Claude Code版本为2.1.236；使用官方项目agents与commands机制，不依赖实验性Agent Teams。
- 影响：所有交接带PRD修订／AC／文件／证据；恢复按phase派发；切片PASS不可冒充完整PASS；详细约定见 [协作](collaboration.md)。

## ADR-009 — 代码合同单一来源，文档保留意图

- 状态：accepted，生成检查待T01；日期：2026-09-11。
- 结论：Rust DTO生成OpenAPI再生成TS类型，SQL迁移是持久schema的机器来源；llmdoc解释语义、设计取舍与证据。
- 原因：让不同能力Agent共同编码，最容易出错的是手抄两份DTO、页码基准、费用单位和状态语义漂移。
- 影响：统一camelCase/UUIDv7/UTC/1-based页码/整数计费/If-Match；代码偏离PRD仍必须修复或经过需求变更，不能“以实现为准”消解需求。

## 跨需求交互原则（非 ADR，UI 角色）

- 状态：accepted（设计在 web-mvp PRD §6；2026-09-12 ui_revision 2 已按 ADR-017 同步修订 2；前端未实现）；日期：2026-09-11。
- 背景与证据：web-mvp PRD §6（UI-001–UI-065）与 §6.3；ADR-005/006/007 在界面层的后果。
- 结论（可跨需求复用的交互原则，理由见 PRD §6.3.1）：
  1. **状态即事实**：界面只呈现服务端返回的状态、金额与校验结果，前端不预测远端成功（生成后进入任务详情等待，不显示客户端"成功"）。
  2. **失败必有安全返回路径，恢复优先于重试**：401→登录并保留 `next`；412→刷新后重试；needs_input→补齐缺项；submission_unknown→对账（不渲染重试按钮）。
  3. **危险动作前明示后果**：取消不保证供应商撤单、`authorizeReplacement` 可能重复收费、发布后不可修改。
  4. **3D 只作增强**：部件列表是热点的文字替代路径，步骤与原文在 WebGL 失败与窄屏下等价可用；这不是可选优化。
  5. **区分「事实确认」与「几何校准」**：不用无定语的"确认"指代两者（modelReview 的 loaded / userConfirmed 亦分开表述）。
  6. **禁用必须给原因**：窄屏禁用热点校准与视角保存时保留可见禁用控件与解释文案，不隐藏、不静默失败。
  7. **不显示虚假进度**：只显示真实可数进度（第 n / N 页、批次覆盖）与阶段列表，不用线性总百分比或预计剩余时间。
  8. **单套路由与组件适配三档断点**（≥1280 三栏 / 768–1279 主栏 + 单侧栏 / <768 单栏 + 抽屉），不为窄屏维护第二套页面。
- 影响：T08、T09、T16–T19 前端切片与后续 E01–E04 需求应复用；若新需求要提供自动发布、自动降质量/换模型入口或概率化措辞，必须先回 PM 修订。
- 未验证边界与下一步：以上均未经 RD 实现与 QA 验收。修订 2（2026-09-12，ADR-017）已裁决 §6.3.3 的 U-01–U-12 **全部接受**为产品决定，UI 已在 ui_revision 2 同步（PRD §8.5、§9.1）。两项对后续需求（E01–E04）有约束力：**U-08** 窄屏可做文字确认与发布、只禁几何校准与视角保存（发布仍受服务端不变量约束）；**U-11** 热点绑定以鼠标点选为主、部件列表为热点的文字替代路径，键盘承诺范围限「非 3D 核心操作」。若 E01–E04 要求窄屏校准或键盘绑定热点，属新增需求，须 PM 新修订（PRD §8.5）；窄屏中段（768–1279）布局与金额/时间格式继续按已接受的 U-02/U-03/U-04/U-06 执行，均未经 QA 验收。
- 相关链接：`requirements/web-mvp/prd.md` §6、§8.5、§9.1/§9.2；ADR-017；后续实现与证据由 RD/QA 绑定同一 PRD 修订。

## ADR-010 — T01 构建链：工具链固定、内嵌资源与生成链

- 状态：verified（2026-09-12，T01 实测：1.98.1 工具链、`cargo xtask contracts/check/dist/smoke-bootstrap` 全部通过；证据见 [implementation](requirements/web-mvp/implementation.md) §3/§5）。
- 日期／作者角色：2026-09-12 · RD（T01）。
- 背景与证据：architecture.md §3 要求 SQLx 0.9 系列（Rust ≥1.94），本机预装 stable 1.93.0 不满足，`rustup check` 显示 1.98.1 可用；本机网络 static.rust-lang.org 实测约 40 KB/s、github.com 不可达、index.crates.io 单请求约 15–22 s、static.crates.io 约 25 KB/s，而 mirrors.ustc.edu.cn 实测约 10 MB/s；本机 `~/.cargo/config` 把 crates-io 指向 git 索引，导致 cargo 卡在 `git fetch`。
- 结论与原因：
  1. `rust-toolchain.toml` 固定 `channel = "1.98.1"`（profile minimal + rustfmt/clippy），workspace `rust-version = "1.94"` 只作 MSRV 下限。固定具体版本而非 `stable`，避免 RD 与 QA 使用不同编译器；升级本文件必须重跑 `cargo xtask check` 与 `dist`+`smoke-bootstrap`。
  2. `embedded-ui` 由 feature 门控：仅该 feature 要求 `apps/web/dist`；`build.rs` 缺 dist 时 panic 并给出可执行指引（只声明 rerun-if-changed，不跑 npm、不联网）；启用 rust-embed `debug-embed`（debug 构建也从二进制读资源，避免“开发机可用、拷走即坏”）与 `deterministic-timestamps`（默认实现会把内嵌文件 mtime 编入二进制，导致同内容不同哈希；实测两次 dist 曾漂移，启用后连续两次哈希一致）。
  3. 生成链：Rust DTO → utoipa 5 →（经 `serde_json::Value` 排序）`contracts/openapi.json` → openapi-typescript（devDependency 锁定 7.13.0）→ `apps/web/src/api/generated.ts`；`contracts --check` 在临时目录生成后逐字节比较，不修改工作树；生成物提交版本控制。
  4. 前端类型系统固定 TypeScript 5.9.x：typescript-eslint 8.70 的 peer 为 `>=4.8.4 <6.1.0`（TS 7 不兼容），openapi-typescript peer 要求 `^5.x`。
  5. 项目 `.cargo/config.toml` 将 crates-io 覆盖为官方 sparse 索引（cargo ≥1.70 默认协议），不修改用户全局配置；受限网络下首次拉包可用单条命令的环境变量 `CARGO_SOURCE_CRATES_IO_REPLACE_WITH=ustc` + `CARGO_SOURCE_USTC_REGISTRY=sparse+https://mirrors.ustc.edu.cn/crates.io-index/`，锁文件仍记录官方 crates.io source（已核对 Cargo.lock 中 registry 源为 `github.com/rust-lang/crates.io-index`）。
- 影响：T02+ 必须沿用同一工具链、生成链与 dist 流程；修改 Rust DTO 后要运行 `cargo xtask contracts` 并提交生成物，否则 `cargo xtask check` 失败；新增内嵌资源时不要移除 `deterministic-timestamps`。
- 未验证边界与下一步：Linux musl 目标、正式包 `smoke`（恢复 data-dir）、签名/公证与平台动态依赖检查属 T22；`licenses.json` 目前只是 SPDX 标识汇总。
- 相关链接：`requirements/web-mvp/implementation.md` §3/§5/§6；`crates/server/build.rs`；`xtask/src/contracts.rs`、`xtask/src/dist.rs`、`xtask/src/smoke.rs`。

## QA 验收知识（非 ADR，QA 角色）

- 状态：verified；日期：2026-09-12（web-mvp T01 回合 1，证据与结论见 [qa-report](requirements/web-mvp/qa-report.md) 与 `artifacts/web-mvp/`）。
- 结论（跨卡复用陷阱）：
  1. 交付物未 git 提交时，“未见工作树改动”不能用 `git status` 证明；用源／生成文件的 sha256 清单前后比对（T01 实测 63 文件，含 `contracts/openapi.json`、`apps/web/src/api/generated.ts`）。
  2. `cargo xtask check` 不编译 `embedded-ui` feature（`embedded.rs` 与 4 条内嵌用例被 cfg 掉）；该发布路径只有 `dist` 与显式 `cargo test -p everything-manual --features embedded-ui --test bootstrap`（需 dist 在场）覆盖。改动 `embedded.rs` 后必须显式跑该 feature。
  3. `cargo xtask dist` 的 `licenses.json` 由不带 feature 的 `cargo metadata --locked` 生成，缺 embedded-ui 专属依赖（rust-embed 及 walkdir 等 15 个，实测 139 vs 锁文件 154）；完整 LICENSE 清单属 T22，T22 应以发布同一 feature 集生成。
  4. SPA 导航判定为“末段含 `.`”启发式：`/assets/<无扩展名>` 返回 200 HTML；T21 安全回归（validation-release §7.3）需按此复核，带扩展名的缺失资源已正确 404。
- 相关代码：`crates/server/build.rs`、`crates/server/src/http/embedded.rs`、`xtask/src/{check,dist,smoke}.rs`。

### T02 验收知识（追加，2026-09-12）

- 状态：verified；日期：2026-09-12（web-mvp T02 回合 2 PASS，证据见 [qa-report](requirements/web-mvp/qa-report.md) 与 `artifacts/web-mvp/t02-qa/`）。
- 结论（跨卡复用）：
  1. **零外呼有编译期证据**：T02 服务端依赖树无任何 HTTP 客户端/TLS crate（无 reqwest/hyper client/rustls），`check`/`serve` 启动阶段不可能外呼。T12/T14 引入 Provider HTTP 客户端后该证明失效，AC-006/AC-012 必须改由 fixture 计数与域名约束测试证明。
  2. **内置 TLS 监听无卡属主（未决）**：配置 `tls.*` 即 exit 6 拒绝（fail-closed，不降级明文），与 PRD §5.1/architecture §7 的"rustls 证书**或**可信反向代理"不冲突，本切片不阻断；但 `implementation-plan.md` 无任何卡包含"实现内置 TLS 监听"（T22 卡文只有 TLS 依赖检查），ADR-011 把实现点写成 T22 属口头指派。发布前必须由 PM/协调者决定：实现该监听，或修订 PRD/architecture 明确 MVP 仅支持反向代理模式。
  3. **交互式密码验收需 pty**：`stdin` 非终端时 `init` 返回 2；用 python pty 才能复核"不回显 + 两次确认 + 不一致不创建目录"。后续涉及交互输入（T04 管理员初始化等）沿用该手法。
  4. **`check` 有副作用**：向 `<data-dir>/logs/everything-manual.log` 追加一行 + `tmp/` 建删写探针（实测除既有日志外无新增文件）。T20 备份/升级流程若需要不触碰 data-dir 的检查，应增加显式只读标志。
  5. **锁语义**：`flock` 使 `kill -9` 后立即可重启（实测），无陈旧锁；`check` 在锁被占用时只警告且 exit 0（服务运行中是合法状态），T20 backup 必须沿用"先停服再备份"。
- 相关代码／测试：`crates/server/src/config/{commands,datadir,password}.rs`、`crates/server/tests/config_cli.rs`。

### T03 验收知识（追加，2026-09-12）

- 状态：verified；日期：2026-09-12（web-mvp T03 回合 3 PASS，证据见 [qa-report](requirements/web-mvp/qa-report.md#回合-3--t03) 与 `artifacts/web-mvp/t03-qa/`）。
- 结论（跨卡复用）：
  1. **迁移重编译证据必须在 `CARGO_INCREMENTAL=0` 下采集**：debug 默认增量构建同一源码两次 sha256 不同（本回合基线 `771a4b40…` = 恢复后；SQL 变更后 `26c7ccc3…`）。"无变更不重编译"也应一起记录。
  2. **构造"旧 schema（v1）库"的可靠手法**：用真实 `init` 建 v2 → `DROP` 0002 的 4 个触发器与 1 个部分唯一索引、`DELETE _sqlx_migrations` v2 行（0001 的 checksum 保持真实）→ 等价 v1，可被 `serve` 正常升级。直接用 sqlite3 伪造 `_sqlx_migrations` 行（如 checksum `X'00'`）只能做"未来 schema 拒绝"负例；用于升级路径会被 SQLx checksum 校验拒绝（`migration N was previously applied but has been modified`）。
  3. **`check` 不校验已应用迁移的 checksum**：修改迁移文件并重建后，`serve` 拒绝启动（exit 4、库字节不变），`check` 却仍报"已就绪" exit 0。`check` 作为停服前预检存在"预检通过但不能启动"的失真（本回合非阻断建议 3）；把 `check` 当升级/备份前置检查的卡（T20）需注意。
  4. **busy_timeout 行为学证据法**：python3 持 `BEGIN IMMEDIATE` 写锁时跑 `serve`（需写入的旧 schema 库），实测等待 5.43s 后以 `database is locked` 失败（exit 4），锁释放后自动升级成功——比只读 PRAGMA 更能证明连接设置生效。
  5. **`check` 副作用清单（更新回合 2 第 4 条）**：除日志追加与 `tmp/` 写探针外，**lock 文件诊断内容（`pid=`/`started_at_unix=`）会被改写**（锁机制本身是 flock，内容仅诊断）；本次未观察到空 `-wal`/`-shm` 残留。T20 若需"完全不触碰 data-dir"的检查应加显式只读标志。
  6. **冷目录迁移验证法**：把 release 二进制单独复制到临时目录（无源码、无 `migrations/`）执行 `init`+`check`，即可证明迁移随二进制内嵌。
  7. **smoke 计数口径**：`cargo xtask smoke-bootstrap` 实际输出 7 项 `[检查]`（另加 1 项 `[准备] init`）；报告计数按日志实数，勿沿用"8 项"。
  8. **sqlx 慢语句 WARN 会打印 SQL 全文**（含迁移 SQL）；后续卡引入带参数查询时需确认日志不携带用户数据。
- 相关代码／测试：`migrations/*.sql`、`crates/server/src/storage/*`、`crates/server/tests/storage.rs`、`crates/server/src/config/commands.rs`、`crates/server/build.rs`。

### T04 验收知识（追加，2026-09-12）

- 状态：verified；日期：2026-09-12（web-mvp T04 回合 4 PASS，证据见 [qa-report](requirements/web-mvp/qa-report.md#回合-4--t04) 与 `artifacts/web-mvp/t04-qa/`）。
- 结论（跨卡复用）：
  1. **会话 token"只存哈希"的独立验证手法**：`curl -c jar.txt` 的 cookie jar 是 **tab 分隔**，取 token 要用 `awk -F'\t' '$6=="em_session"{print $7}'`（`grep 'em_session='` 取不到——T04 QA 首版脚本踩坑）；再 `shasum -a 256` 与 `sessions.session_token_hash` 比对，并用 Python 在库文件原始字节里 `blob.count(token)` 确认 0 命中（强于只读测试断言）。CSRF 同理（`sha256(csrf)=csrf_hash`）。
  2. **405（方法不匹配）语义**：返回空 body 的 405 + `Allow` + `x-request-id`，**不套**统一错误信封；受保护路由上未带 CSRF 的修改请求先返回 403（守卫按方法判定，先于路由与 body 解析）。T06/T07/T12 新增路由时 `Allow` 值会变化；T07 补删除语义时需明确"未带 CSRF 的 DELETE 先 403 还是先 405"。（本项列 T04 报告非阻断建议 1/2。）
  3. **cookie `Secure` 伪造头负例**：带 `x-forwarded-proto: https`、`x-forwarded-ssl: on`、`forwarded: proto=https` 登录，Set-Cookie 仍不得含 `Secure`（T04 起判定只用 `public_origin` scheme / 内置 TLS 配置；反向代理终止 TLS 的部署必须显式 `EM_SESSION__COOKIE_SECURE=always`）。
  4. **CSRF 覆盖 multipart 的验证法**：向任意受保护路径发 `Content-Type: multipart/form-data` 且无 `X-CSRF-Token` 的 POST 应得 403 `CSRF_REJECTED`；T06/T09 加 multipart 路由后沿用该负例，并记住它们需要独立于 1 MiB JSON 的 body 上限（ADR-013 第 6 条）。
  5. **ready 负例法**：运行中把 `<data-dir>/manual.sqlite3` 移走 → 503 `not_ready`（`data_directory=fail`；`database` 因连接池持句柄仍 `ok`），恢复文件后立即 200。比 `chmod` 只读更贴近"数据目录损坏"且不动运行中连接。
  6. **items 三端点当前状态（T07/T08 派发用）**：T04 的 If-Match 载体已实测——无创建（POST → 405）、无物理删除（DELETE → 405）、`PATCH archived` 生效但**列表不过滤归档**、超长字段**未校验**（300 字符 name 被接受）、`{"brand":null}` 静默保留原值且 revision 递增、无字段级 422 明细。T08 不得按当前 `/items` 做冻结实现；T07 派发包应引用 T04 报告"特别裁定 2"的清单。
- 相关代码／测试：`crates/server/src/http/{router,auth,precondition,items,settings,health}.rs`、`crates/server/tests/{auth_api,common/mod}.rs`、`artifacts/web-mvp/t04-qa/qa-smoke.sh`。

### T05 验收知识（追加，2026-09-12）

- 状态：verified；日期：2026-09-12（web-mvp T05 回合 5 PASS，证据见 [qa-report](requirements/web-mvp/qa-report.md#回合-5--t05) 与 `artifacts/web-mvp/t05-qa/`）。
- 结论（跨卡复用）：
  1. **fixture 断言 API 与"缺脚本"语义（T12/T14 直接用）**：缺路由/步骤耗尽 = **501 + `script_problems()`**（不是异常、不是 2xx）；适配器测试应连同 `assert_no_script_problems()` 断言；`repeatLast` 只对已声明路由生效；`header_value("authorization")` 返回 `Bearer [REDACTED]`（保 scheme）；计数用 `assert_called_once/call_count/request_total`，请求体用 `json_body()`。T12/T14 的"付费 POST 恰 1 次"按此断言。
  2. **场景即数据**：`tests/fixtures/scenarios/*.json`（`match: prefix` 用于 `/v3/tasks/`）；传输层已实测支持 **chunked 请求体与 `Expect: 100-continue`**，可直接承接 reqwest 流式 multipart。QA 探针（仓库外 crate、path 依赖 test-support）`artifacts/web-mvp/t05-qa/probe-src/` 可作为"设施可用性"回归的模板（44 项检查）。
  3. **样例资产外部验真手法（不复用仓库校验器）**：PDF 文字层用系统 `swiftc` + PDFKit（`PDFDocument.page.string`；text PDF 320 字符 / scan PDF 0 字符）；GLB 用 python `struct` 独立解析（12 三角面、BIN 自洽、内嵌 PNG CRC 全对）；`sips` 解码/渲染（595×842）。T09/T13/T18 复用这些平台工具做交叉验证。
  4. **本机外呼采样法**：`while kill -0 <pid>; do lsof -p <pid> -a -i -P -n; sleep 0.05; done`——fixture_harness 20 次运行 171 次采样仅回环（490 行回环 / 0 非回环）。T12 引入 reqwest 后，"无外网"证据必须换成"全部 fixture URL 为 127.0.0.1:随机端口 + 计数断言"（T02 知识 1 的失效条件已触发）。
  5. **release 无密钥复核法**：空 data-dir + 受限密码文件 → `init` → `serve --listen 127.0.0.1:0`；`/health/ready` 200（process/data_directory/database/migrations）；日志 `provider_not_configured` ×2 明示"不回退 mock"；`lsof` 0 外部连接；`base_url` 指向不可达地址（`https://203.0.113.9/`）ready 仍 200；"配置已加载"可用"未知键 → exit 3 且指名文件"快速证明。
  6. **T04 limiter 单测时序 flake（未复现、机制成立）**：`window_restarts_after_expiry_and_zero_limits_are_clamped` 有两个真实时钟子句——(a) `new(0, 1ms)` 后 `record_failure`→`check` 间隔 ≥1ms 即失败（RD 报告失败的 `limiter.rs:154` 是**这一条**）；(b) `new(1, 40ms)` + sleep 60ms 子句间隔 ≥40ms 即失败。QA 22 轮 `cargo test --workspace`（含 12 轮 16-burner 加压）+ 110 次 lib 套件 + 240 次定向单测 + `--test-threads` 1/16 + release 均未复现（0 失败；本机背景负载 mds_stores ≈300%、load 37–44）。修复方向：可注入时钟，或窗口/睡眠放大 ≥100 倍。**RD 的失败原文未存 artifacts**（后续复现必须原样保留）。qa-report 回合 5 记为 BUG-001（P2、OPEN、不阻断 T05）。**→ 已由 [ADR-015](#adr-015--t06-上传与资产服务落地取舍两层体积防线先文件后元数据隔离不删range-语义磁盘满-413) 结论 9 修复（可注入时钟，2026-09-12 T06 回合），本条保留作历史根因记录。**
- 相关代码／测试：`crates/test-support/src/*`、`crates/server/tests/fixture_harness.rs`、`tests/fixtures/*`、`crates/server/src/http/auth/limiter.rs`（BUG-001）、`artifacts/web-mvp/t05-qa/`。

### T06 验收知识（追加，2026-09-12）

- 状态：verified；日期：2026-09-12（web-mvp T06 回合 6 PASS + BUG-001 CLOSED，证据见 [qa-report](requirements/web-mvp/qa-report.md#回合-6--t06含-bug-001-复验) 与 `artifacts/web-mvp/t06-qa/`）。
- 结论（跨卡复用）：
  1. **磁盘满的真实构造法**：`hdiutil create -size 32m -fs APFS` + `hdiutil attach -mountpoint` 得到真实小文件系统，`init` 时**密码文件必须 0600**（否则 exit 3——QA 首轮即踩到）；40 MiB 上传 → 真实 statvfs 下 413 + `details.reason=insufficientStorage`、无半提交；同环境小文件仍 201。收尾必须 `detach` + 删镜像。
  2. **写流中途 ENOSPC 的构造与实测结论**：`curl --limit-rate 5m` 上传 + 并发 `dd` 填充磁盘，可让 ENOSPC 落在预检/复检之后。实测：**不半提交**（0 行、0 tmp、0 文件；tmp 由 Drop 守卫清理），但响应是 `500 INTERNAL` 通用文案（细节只在服务端日志）。属非阻断改进点（映射为 413+insufficientStorage 更好）。
  3. **原子落盘顺序的可观察法**：限速上传 + 200ms 采样 `tmp` 文件数/字节、`blobs/` 文件数、`assets`/`blobs` 行数 → 可直接看到"tmp 单调增长期间元数据恒为 0，完成后一次性翻转"；配合"每条 asset 的 blob 文件存在且 sha256 相符"作为"元数据可见 ⇒ 文件在"的可观察结论。
  4. **流式处理的内存判据**：`ps -o rss= -p <server>` 50ms 采样（40 MiB 限速上传 112 次采样）——峰值 = 基线与文件大小无关即证明未整文件读入内存；比只读代码有力。上传内容的外部验真沿用 T05 知识 3（PDFKit/`sips`），避免"自建样例非法却被 201"的假阳性。
  5. **上传/资产边界语义（T09/T13 复用）**：加密 PDF（`/Encrypt`）与对象流页树（`/ObjStm` 不可定位）上传 201 + WARN，权威拒绝在 T09；`startxref` 必须落在文件尾 8192 B（否则 422，与真实阅读器一致）；空文件可上传、任何 Range → 416 + `bytes */0`；`quarantined` 状态不会由 T06 流程产生（需管理员/T20），上传同内容会 422 且不删文件；`check --data-dir` 不做资产扫描（存在 tmp 残留时不动文件，只读语义保持）。
  6. **BUG-001 关闭的机制判据**："消除时钟依赖"要同时看四点——(a) 被测代码所有时间读取都经同一时间源；(b) 生产构造仍用系统时钟、默认值/收口未变（HTTP 层实测"5 次错密码 → 第 6 次 429 + `retry-after: 60`" 比单测更有力）；(c) 测试文件已无真实 `sleep`/毫秒窗口断言；(d) 守护用例在"不推进时钟"下高频重复判定（1ms 窗口 ×1000 次）必过。回归 16 轮 0 失败。
- 相关代码／测试：`crates/server/src/assets/{blob_store,validate,pdf,upload,maintenance,range,error}.rs`、`crates/server/src/http/assets.rs`、`crates/server/src/storage/repo/{assets,blobs}.rs`、`crates/server/tests/assets.rs`、`crates/server/src/http/auth/limiter.rs`、`crates/server/src/config/commands.rs`（serve 接线）、`artifacts/web-mvp/t06-qa/`。

### T07 验收知识（追加，2026-09-12）

- 状态：verified；日期：2026-09-12（web-mvp T07 回合 7 PASS，证据见 [qa-report](requirements/web-mvp/qa-report.md#回合-7--t07) 与 `artifacts/web-mvp/t07-qa/`）。
- 结论（跨卡复用）：
  1. **发布二进制可复现的强判据**：QA 侧两次 + RD 侧两次 `cargo xtask dist` 得同一 sha256（`287c05c3…`，8 919 680 B）——比"两次一致"更强的四方独立复现，后续卡可沿用"QA 重建比对哈希"作为"产物与源码一致"的入口之一。
  2. **DB 级不变量需"绕过服务层"验证**：停服后用 `sqlite3 <data-dir>/manual.sqlite3` 直接 INSERT 与既有行同 `(item_id, view)` 的记录 → `UNIQUE constraint failed`，证明 `photos_item_view_unique` 是服务层 422 `details.reason=viewOccupied` 之外独立生效的最终防线；T09/T11/T19 的页号唯一、job_stage 唯一、release 不可变建议用同一手法加证。
  3. **0 外呼判据的现状与失效条件（T12/T14）**：本卡以"python 计数监听器 + `cargo tree -p everything-manual --edges normal` 生产依赖树无 HTTP client（reqwest 仅 xtask）"证明 source_url 不抓取；T12/T14 引入真客户端后必须换成 fixture 计数 + 域名约束（T05 知识 1 的失效条件已触发）。
  4. **"响应无磁盘路径"的覆盖面检查法**：以 QA 临时目录绝对路径为模式 grep 全部响应落盘文件（`grep -l "$WORK" *.json`），比逐条断言覆盖更大；T09/T13 的下载/准备响应可复用。
  5. **自写 curl 冒烟脚本的三个陷阱（QA 侧两次踩坑）**：`-w '%{http_code}'` 不带换行 → 重定向文件拼接粘连（加 `\n`）；包装函数参数错位时 curl 收到额外"URL"会把 body 打到 stdout（症状 `000000{json}200`，先验证单 URL 再写真言）；测"字段级 422"必须带齐必填字段，否则命中提取器级通用 422（`details=null`，英文 serde 文案）。
  6. **T07 语义边界（T08/T11/T12 复用）**：`GET /items` 归档过滤参数名是 **`archived`**（T04 报告设想的 `includeArchived` 未采用，以 OpenAPI 为准）；`GET /items/{id}/photos` 拒绝**任何**查询参数（含 `limit`）且 `nextCursor` 恒 null（集合上界 5 条）；`detail` 在 GET 列表可见但不在 `list_multiview_for_item`；照片无删除 API，"移除"由 PATCH 改选/换资产覆盖（真删除需合同修订）；缺 `assetId`/`sourceAssetId` 的 422 是提取器级（`details=null`），与 ADR-016"字段级 422 统一形态"不一致，T08 渲染需保留通用兜底。
  7. **T11 前置约束**：photos 行按 id 可变（PATCH 换资产后同 id 内容变化），快照必须按 contracts §2 保存 `photo_ids+hashes`，不能只存 id；本卡实测"改名/归档不改变 document/photo 引用与 source_sha256"只是冻结快照的结构性前提。
  8. **挂起 PM 决策的现状（勿在实现中先斩后奏）**：REQ-010 物品级"来源链接"在 PRD 与冻结合同之间冲突（实现按合同：`sourceUrl` 进 `POST /items` → 422），建议 PM 选"修订 PRD 去掉物品级字段、由 document 承载"（改动最小）或"修订合同加列 + 迁移"；`sourceUrl` 在 items 上出现即 422 是"不静默"的正确行为，不是缺陷。
- 相关代码／测试：`crates/server/src/http/{items,documents,photos,pagination,body}.rs`、`crates/core/src/validation.rs`、`crates/server/src/storage/repo/{items,documents,photos}.rs`、`migrations/0003_photos_view_unique.sql`、`crates/server/tests/items.rs`、`artifacts/web-mvp/t07-qa/`。

### T08 验收知识（追加，2026-09-12）

- 状态：verified；日期：2026-09-12（web-mvp T08 回合 8 PASS，证据见 [qa-report](requirements/web-mvp/qa-report.md#回合-8--t08) 与 `artifacts/web-mvp/t08-qa/`）。
- 结论（跨卡复用）：
  1. **CDP 头观测陷阱（最重要）**：Chrome 152 的 `Network.requestWillBeSent.request.headers` **不上报 `x-csrf-token`**（同一 POST 的 `if-match`、`origin`、`content-type` 都在），必须用 `Fetch.enable{patterns:[{requestStage:"Request"}]}` + `Fetch.requestPaused` 读实际头 + `Fetch.continueRequest` 放行，否则 T16/T17/T19 的 CSRF／Idempotency-Key 断言会出现"应用没发头"的假阴性（本回合踩坑并纠正）。可配"服务端强制对照"（缺头 → 403、带头 → 201）做反证。
  2. **CSRF token 每次登录都换**：浏览器走查中的头断言必须在**最近一次登录**的响应体里重新取 token，不能沿用早期登录的 token。
  3. **"点击被别的元素吞掉"的证明手法**：`document.elementFromPoint(控件中心)` 返回的元素 + 命中判定，可直接证明覆盖层拦截（本回合据此定位 BUG-001：`.notices{position:fixed;top:0;…;z-index:30}` 覆盖顶栏）；T17/T19 的模态、抽屉、toast 可沿用。
  4. **通知条/覆盖层必须同时验指针与键盘**：本回合同一时刻"指针点击失败、Tab+Enter 成功"，是判定"非阻断交互瑕疵"而非"功能不可用"的关键证据；只验其一都会误判严重度。
  5. **登录限速的稳妥验法**：先用 Node 侧第二客户端打满限速窗口取 429 + `Retry-After`，再让浏览器登录一次观测页面文案；连续在浏览器里试错容易触发下一条的 CDP 卡顿。
  6. **Chrome/CDP `Runtime.evaluate` 偶发 30 s 无响应**：本回合两次遇到（服务端日志正常、应用无异常、无 console.error），`Target.closeTarget` + `createTarget` + 重新 attach + 回到已知 URL 可恢复；后续卡的浏览器回归应内置该恢复，避免误判为"应用卡死"。`probe-reload.mjs` 证明单纯 `Page.reload` 不会复现，疑与长时间连续驱动有关。
  7. **`GET /auth/session` 的 `Cache-Control: no-store` 只在 200**（登录/会话成功响应走 `json_no_store`；401 不带）。AC-003 的断言口径应绑定"带凭据的会话响应"或 `fetch cache:"no-store"` 的行为，不要写成"任意状态码都带"。
  8. **前端错误边界的可复用手法**：`Fetch.fulfillRequest` 注入合同违约载荷（如 `sourceSha256=null`）并自带 `x-request-id`，可验证"边界显示 requestId 且不显示堆栈"；该注入会让 React 输出 1 条 `console.error`（被捕获异常的常规上报），断言"全程 0 console.error"时必须排除这一窗口。
  9. **浏览器走查建议直接用发布二进制内嵌 UI**（同源 + SPA fallback + cookie/Origin 与生产一致），比 Vite dev + 代理少一层变量；`cargo xtask dist` 已保证内嵌资源在场，T22 之前的卡都可这样验。
  10. **加载态观测窗可以用 `Network.emulateNetworkConditions{latency}` 放大**（本回合 1.2 s 稳定验证"会话恢复中不闪登录页"），比反复快速截图可靠。
  11. **未实现路由的"不冒称完成"查验法**：逐条打开占位路由 + 用 `Network` 事件断言"除 `/auth/session`、物品上下文与 health 外无业务请求、无写请求"，比只看页面文案更硬。
- 相关代码／测试：`apps/web/src/{App.tsx,styles.css}`、`apps/web/src/api/{client,endpoints,generated}.ts`、`apps/web/src/features/{auth,library,settings,shell}/**`、`apps/web/src/components/**`、`apps/web/src/features/shell/shell.test.tsx`、`artifacts/web-mvp/t08-qa/`（QA 自写 `walkthrough-qa.mjs` + `run-walkthrough-qa.sh`）。

### T09 验收知识（追加，2026-09-12；下 7 条为 RD 侧交付记录）

- 状态：**verified**（RD 实测 + **QA 回合 9 独立复验 PASS**，2026-09-12；QA 自写 spec 6 用例、`sqlite3` 直查零副作用、系统 PDFKit 验负例 fixture、release 内嵌 vendor 自写脚本复核；证据 `artifacts/web-mvp/t09-rd/` 与 `artifacts/web-mvp/t09-qa/`，结论见 [qa-report 回合 9](requirements/web-mvp/qa-report.md#回合-9--t09含-bug-002原-bug-001-r8复验)）。
- 结论（跨卡复用）：
  1. **e2e 首个 spec 的运行时约定**：`test:e2e` 由 `globalSetup` 自建后端（临时 data-dir + `init --password-file`）
     与 `webServer` 自起前端（`EM_WEB_PORT`/`EM_API_PROXY_TARGET`），端口 15173→18080 避开开发默认；
     `@playwright/test@1.60.0` 与本机缓存 chromium 1223 对齐（`npx playwright install --dry-run` 可核对，不必下载）。
  2. **`globalSetup` 早于测试文件收集**：spec 顶层若 `readRuntime()` 会 ENOENT，运行时信息必须**惰性读取**。
  3. **PDF.js 的 CMap 在浏览器里才可验证**：Node（legacy 构建）用 XHR 读 `cMapUrl`，Node 无 XHR →
     即使给出本机 HTTP 地址也不会发请求、文字层为空；不要用 Node 结果判断"CMaps 不能离线"，
     权威证据是真实 Chrome 的 e2e（请求清单里出现 `/vendor/pdfjs/cmaps/*.bcmap`）。
  4. **"断外网"用例的实现**：`page.route("**/*")` 放行 `127.0.0.1/localhost`、其余 `abort()` 并记录；
     断言"记录为空"+"本地 worker/CMaps 请求确实发生"，两条缺一不可（只断言前者可能什么都没加载）。
  5. **worker 路径断言要兼容两种形态**：dev 是 `/node_modules/pdfjs-dist/build/pdf.worker.min.mjs`，
     build 是 `/assets/pdf.worker.min-<hash>.mjs`；正则写 `pdf\.worker\.min(\.mjs|-[^/]*\.mjs)$`。
  6. **覆盖层几何修复的验法**：`elementFromPoint(链接中心)` + 真实点击 + 至少两个断点（本卡 1280/390），
     断言 `closest("a")` 的文本；顶栏高度随路由内容变化，修复要么实测高度、要么给足余量（本卡用实测 + 100ms 复核）。
  7. **macOS 上直接执行 `dist/<target>/everything-manual` 可能被 SIGKILL（实测 Killed: 9）**：
     与二进制签名/路径保护有关；按 `xtask smoke` 的做法**先拷到新临时目录再运行**（本卡 `check-embedded-vendor.sh` 即如此）。
- 相关代码／测试：`apps/web/playwright.config.ts`、`apps/web/tests/e2e/**`、`apps/web/vite.pdfjs-vendor.ts`、
  `apps/web/src/features/import/**`、`artifacts/web-mvp/t09-rd/`（含可复跑脚本 `check-embedded-vendor.sh`、`check-fixtures.mjs`）。

#### T09 验收知识 · QA 复验补充（回合 9，2026-09-12）

- 状态：verified。证据：`artifacts/web-mvp/t09-qa/`（QA 自写 spec `apps/web/tests/e2e/qa-t09-independent.spec.ts`、`check-embedded-vendor-qa.sh`、`fixture-probe/`）。
- 结论（跨卡复用）：
  1. **"只补缺页"的最强复现**：用 `page.close()` + `context.newPage()`（真实关标签页，新 tab 天然无 sessionStorage 指针），断言续传 PUT 序列恰为缺失页**且**已完成页的 `imageAssetId`/`textAssetId` 未被替换（防"重传但被幂等掩盖"的假阴性）。`page.reload()` 会保留指针，弱于本手法。
  2. **SQLite 直查做零副作用断言**：`sqlite3 <data-dir>/manual.sqlite3 "SELECT COUNT(*) FROM preparations|pages|jobs|cost_ledger|provider_attempts;"` 前后差值 = 0，是"拒绝路径不建记录、complete 不建 job/不收费"最硬的证据（WAL 运行中读取安全）；T11/T12/T14 沿用。
  3. **负例 fixture 用系统 PDFKit 验真**：`PDFDocument.isEncrypted/isLocked/unlock(withPassword:)`、`pageCount` 独立确认"加密样例真加密（`fixture-secret` 可解锁）""101 页样例真 101 页"，避免负例空转。注意 PDFKit 读不出本例 CJK 页文字（ToUnicode 形态非最标准），文字层正确性仍以真实 Chrome + PDF.js 为准（RD 知识 3：Node 无 XHR 验不了 CMap）。
  4. **断外网的完整判据**：外部请求清单为空 **且** 关键资源确实从本机加载（worker/CMap 请求 200）；补充做法是在页面内直接 `fetch` CMap/字体/WASM/ICC 五类 URL 断言 200 非空（样例不触发 WASM 时的替代证据）。
  5. **通知条定位的残留窗口（BUG-002 复验实测）**：390px + 顶栏因物品上下文换行为 138px 时，通知出现后第一帧 `--notices-top` 仍是媒体查询兜底 104px，链接中心（y=106）被通知条覆盖，≤100ms（100ms 轮询周期）后校正为 138px。RULE：定位/覆盖类修复若依赖轮询，报告里必须写"变化后 ≤周期"窗口；彻底解法为 `ResizeObserver` 同步跟随或让通知进入文档流。宽屏 1280 无此窗口（首帧即实测值）。
  6. **e2e 证据目录需隔离**：`playwright.config.ts` 的 `outputDir` 与后端日志固定写 `artifacts/web-mvp/t09-rd/`，第二个跑同一 spec 的角色会覆盖前者的 `e2e-server.log`；跑前先备份（本回合 `t09-qa/rd-evidence-preserved/`），T21/T22 建议改按运行/角色隔离。
  7. **脚本陷阱**：bash 中 `$VAR` 紧跟多字节字符（中文全角括号等）在非 UTF-8 locale 下被并入变量名（`unbound variable`）；写中文输出的脚本统一 `${VAR}`。
- 相关代码／测试：同上 + `apps/web/tests/e2e/qa-t09-independent.spec.ts`、`artifacts/web-mvp/t09-qa/`。

### T10 验收知识（追加，2026-09-12；回合 10 结果 FAIL，1 个 OPEN 缺陷 BUG-003）

- 状态：回合 10 已执行；BUG-003（P2，AC-036 第 4 子句）当时 **OPEN**；**RD 回合 11 已修复（见下方"T10 修复知识"），QA 回合 11 已复验关闭（见"T10 复验知识"）**。证据：`artifacts/web-mvp/t10-qa/` + `llmdoc/requirements/web-mvp/qa-report.md` 回合 10 节。
- 结论/手法（跨卡复用）：
  1. **崩溃注入的最强形态**：进程级 `libc::kill(SIGKILL)`（不 shell 出 `kill`）且**先断言崩溃现场、再断言恢复结果**；子进程用环境变量驱动（无变量时空操作），父进程用 fixture 计数（"请求确实离开进程"）作为注入点锚。用例：`crates/server/tests/qa_t10_independent.rs`（7 常规 + 1 ignored 复现）。
  2. **"不跨 HTTP 持事务"的可观察判据**：请求在途时从**另一连接**做一次写事务并计时（本次 2.3ms）。T11/T12/T14 的付费/长请求路径可复用。
  3. **迁移链验证手法**：合成旧版本库 = 顺序执行历史迁移 SQL + 手写 `_sqlx_migrations` 行（`checksum` = 迁移文件 **SHA-384 大写 hex** 的 BLOB；算法先用全新 `init` 的库反证），data-dir 结构需自带 `tmp/ logs/ blobs/ lock`（少一个 `check` 退出 4）；可验证任意旧 schema → 当前版本的升级路径与"`check` 只读"。
  4. **failpoint 门控核对必须带正对照**：只对发布二进制 `strings` 全 0 会空转；须同时看到测试二进制非 0（本次断点名各 3、`EM_TEST_FAILPOINT` 2、`nm` failpoint 符号 1251 行）。
  5. **BUG-003 根因（务必避免重犯）**：预算/等待类逻辑必须用**适配器真实的数据形态**验证——给被测阶段手工造 attempt 会让"锚点不可达"类缺陷看起来通过。`job_stages.poll` 阶段的"总等待 30 分钟"起点应取 job 级 accepted 提交事实（轮询处理器取 task ID 用的同一份），而非该阶段自身的 attempt；`model_download` 等未来可能返回 `WaitingProvider` 的阶段同样受影响（修 `plan_advance` 时一并覆盖）。
  6. **`#[ignore]` 承载"合同期望 vs 当前实现不符"的复现用例**：标准 `cargo xtask check`/`cargo test --workspace` 保持绿，复现证据可 `-- --ignored` 单跑；报告必须显式列出（本次 1 条 QA + 1 条既有 T07 doctest），避免"看起来没有 unrun 项"。
  7. 环境噪音：`~/.cargo/config` deprecated 警告与结论无关；统计 `cargo test --workspace` 时以全量日志求和（RD §T10-9 明细曾漏 1 条：jobs_recovery 21 vs 实际 22）。
- 相关代码／测试：`crates/server/tests/qa_t10_independent.rs`、`artifacts/web-mvp/t10-qa/qa-{t10-process-check,t10-migration-chain}.sh`。

#### T10 修复知识 · RD 回合 11（BUG-003 等待起点锚点，2026-09-12）

- 状态：**已修复，待 QA 复验**（round 10 判定的 P2 / AC-036 第 4 子句；未改合同语义、未降低阈值 1800s）。
- 结论与原因（可跨卡复用的规则）：
  1. **等待起点 = 该远端等待链的 `accepted` 提交事实**（`tripo_submit` 的 accepted attempt），
     而不是"该阶段自己的最近 attempt"。原因：`tripo_poll`（及 `model_download`/`model_validate`）
     在真实链路上不建 attempt，task ID 来自 job 级提交事实；用本阶段 attempt 会让预算恒不可达。
     实现：`manual_core::jobs::{remote_wait_anchor_kind, remote_wait_anchor, waited_seconds}` +
     执行器 `JobExecutor::wait_anchor`（仅 `WaitingProvider` 时查询）。
  2. **只接受 accepted 且带远端 task ID 的事实**：intent/submitting/unknown、以及仅有 `response_id`
     的同步 receipt 都不能起跑计时（避免"尚未证明被接受"的时间被算进远端等待）。
  3. **归属范围 = 同一 job**（`latest_accepted_for_job`），与处理器取 task ID 用同一份事实 →
     "正在轮询的任务"与"计时起点"必然同源，多 job 不串用；`manual_extract` 多批次不参与远端等待。
  4. 计时时间取 `started_at`（intent 创建即提交前一刻），与既有用例/QA 假设一致；`updated_at`（accepted 落库）
     与其差值在真实链路是亚秒级，不影响 30 分钟语义。
  5. **同类锚点核查**：安全重试计数（`attempt_count`）与轮询节奏（`poll_count`）用阶段列，语义即"该阶段自己的计数"，
     无锚点问题；恢复矩阵与提交窗口只关心"本阶段是否有未决事实"，无需 job 级回退。
     全仓库 attempt 时间戳消费者已逐一核对（见 implementation §T10-13-4）。
- 影响：T12（适配器契约应写明"等待起点 = accepted 提交事实"，并把 30 分钟预算纳入契约用例，QA 回合 10 非阻断建议 3）、
  T13（`model_download` 已由本修复覆盖锚点）、T15/T17（`needs_input` 缺项与 task_id 展示语义不变）。
- 证据：`artifacts/web-mvp/t10-rd-bug003/`（复现日志、真实形态探针 `120 次轮询/1800s → NeedsInput`、全量回归与 dist/smoke）；
  `llmdoc/requirements/web-mvp/implementation.md` §T10-13。
- 相关代码／测试：`crates/core/src/jobs.rs`、`crates/server/src/jobs/executor.rs`、
  `crates/server/tests/jobs_recovery.rs`（3 条新用例）。

#### T10 复验知识 · QA 回合 11（BUG-003 复验通过，2026-09-12）

- 状态：**BUG-003 CLOSED，T10 切片 PASS（回合 11）**。证据：`artifacts/web-mvp/t10-qa-r11/` + `llmdoc/requirements/web-mvp/qa-report.md` 回合 11 节。
- 结论/手法（跨卡复用）：
  1. **修复类复验的"判别力"判据**：不能只看"新用例转绿"；要构造**只能由修复后行为满足**的断言。本回合最强判据是可复算中间量——越界 note 中的等待秒数必须等于"accepted 提交事实起点 + 轮询节奏累加"（1811 = 1790 + 3+6+12）；再用"回拨 job/阶段行时间戳 40 分钟不影响判定"反证锚点不是行时间戳/阶段年龄。
  2. **文件清单（manifest）必须在最后一次写入之后生成**：回合 10 的 `source-manifest-after.txt`（09:18:02）早于其自身末次编辑（09:20:45），造成复验时哈希对不上，需要额外解释。后续：写文件后立刻哈希；报告中注明"清单时间 vs 文件 mtime"并说明交叉验证手段；无基线副本时如实记录"无法逐字节 diff"。
  3. **执行器短路路径**：被领取阶段若自身存在未决 attempt（`Submitting`/`Unknown`），执行器直接落 `submission_unknown`、不调处理器（`crates/server/src/jobs/executor.rs:433` 附近）。设计锚点/等待类边缘用例要绕开该形态；"阶段级非 accepted 事实"的反例应改用 accepted-但-无远端 ID 的 receipt。
  4. 复验环境事实（2026-09-12）：dist 二进制哈希由 QA 独立重建与 RD 完全一致（`cf754cf0…`），可作为"生产源码自 RD 构建后未变"的旁证；failpoint 正对照计数复现为 3/3×6 + `EM_TEST_FAILPOINT` 2 + `nm` 1251 行（与回合 10 相同）。
- 相关代码／测试：`crates/server/tests/qa_t10_r11_verify.rs`（QA 新写常驻回归 4 用例，无 `#[ignore]`）、`crates/server/tests/qa_t10_independent.rs`（回合 10 复现用例保留 `#[ignore]`，按派发约束未修改）。

## ADR-011 — T02 CLI／配置／锁／日志的落地取舍

- 状态：verified（2026-09-12，T02 实测：`cargo test -p everything-manual --test config_cli` 12 通过、`cargo xtask check` 全绿、`cargo xtask dist` 两次哈希一致 + `smoke-bootstrap` 通过；证据见 [implementation](requirements/web-mvp/implementation.md) §T02-6）。
- 日期／作者角色：2026-09-12 · RD（T02）。
- 背景与证据：PRD REQ-003/005/007/044、validation-release §5、architecture §6/§7；T02 卡要求 backup/restore 未实现期非零返回、未知配置键报错、缺密钥不回落 mock、非 loopback 无 TLS/代理拒绝启动。
- 结论与原因：
  1. **配置文件定位规则**：`--config` > `EM_CONFIG` > `<data-dir>/config.toml`（data-dir 来自 CLI/环境变量）> `./config.toml` > 内置默认（配置文件可缺席）。相对路径一律相对**进程工作目录**解析（不做“相对配置文件”解析），避免同一份配置因启动方式不同解析出不同路径；运维示例使用绝对路径。
  2. **环境变量是显式白名单**（`EM_` + 键路径以 `__` 分隔，共 23 个键 + `EM_CONFIG`），不做通用前缀扫描：防止 CI/shell 中无关的 `EM_*` 变量改变服务行为。密钥不由这些变量直接承载，而是由配置键 `api_key_env` 指向变量名（“密钥不进配置文件”）。
  3. **未知配置键 = 配置错误**（每层 `deny_unknown_fields`，退出码 3）：把“不静默忽略”做实，同时让拼写错误立刻暴露，而不是悄悄用默认值继续跑。
  4. **退出码固定 0/1/2/3/4/5/6/7**（含义见 `crates/server/src/config/error.rs`）：脚本与 QA 按此断言；`backup/restore` 用 7 与“运行时失败（1）”区分，未实现不会与真失败混淆。**T20 更新（2026-09-13）**：`backup/restore` 已实现，**7 的含义改为“备份/恢复完整性校验失败”**（4 扩展为含“备份输出已存在/恢复目标非空/备份 schema 比程序新”，5 含“backup 要求先停服”）；数值合同与 2/3/6 的行为不变。见 ADR-031 第 7 条与 [implementation §T20-5](requirements/web-mvp/implementation.md)。
  5. **内置 TLS 未实现时拒绝而非降级**：只要配置了 `tls.cert_file/key_file`，serve/check 明确报错（退出码 6）——不把“要求 TLS 的部署”静默服务成明文；非 loopback 的唯一放行路径是 `trusted_proxy_cidrs`（显式信任边界），T02 不消费任何 `X-Forwarded-*`，T04 起按 `Cidr::contains` 校验来源后才可信任转发头。
  6. **排他锁用 `flock` 而不是 pid 文件**：进程崩溃/`kill -9` 后由内核自动释放，不留需要人工判断“是否陈旧”的锁文件；锁文件里的 `pid/started_at_unix` 只作诊断展示，不是判断依据。不支持 NFS/共享盘多实例（A-09）。
  7. **日志同时写 stdout 与 `<data-dir>/logs/everything-manual.log`（JSON Lines，0600，追加）**：前台运行时终端即日志流（`smoke-bootstrap` 也据此解析启动行），排障时 data-dir 有历史；文件写失败不影响服务运行。请求日志只记 path（**不记查询串**——签名 URL 凭据在查询串）与 `requestId/status/errorCode/durationMs`；统一错误码经 axum response extension 传给中间件，响应体形状不变。
  8. **`init` 不持久化密码**：Argon2 + SQLite 落库属 T04（PRD §7.1 将 REQ-002 映射到 T04）。T02 只实现并测试密码输入路径（交互不回显两次确认 / `--password-file` 0600 受限文件；**不存在命令行参数形式**）与校验，输出明确说明凭据存储尚未启用，不伪称管理员已创建。
  9. **T02 不创建 `manual.sqlite3`**：数据库由 T03 迁移创建，避免留下无 schema 的空库被误当“已初始化”；`blobs/` 目录按架构 §6 布局预留。
- 影响的 REQ／任务／模块：REQ-003/005/007/044；T03（data-dir 与锁直接复用）、T04（`Settings.provider_status`/`SecretString`/`Cidr`/可信代理语义/requestId 中间件）、T20（backup/restore 退出码 7 占位）、T22（内置 TLS 监听与日志轮转的实现点）。
- 未验证边界与下一步：内置 TLS 监听未实现（配置即拒绝）；日志无轮转；`RUST_LOG` 只支持单一级别名（无 env-filter 指令语法）；`check` 的 schema 检查在 T03 接入；非 Unix 平台不支持锁与不回显输入（Windows 属扩展平台）。
- 相关链接：`crates/server/src/config/{mod,cli,commands,file,datadir,error,logging,password,secret}.rs`、`crates/server/src/http/logging.rs`、`crates/server/tests/config_cli.rs`、`config.example.toml`。

## ADR-012 — T03 持久层落地取舍（时间/枚举表示、触发器不变量、门禁与 check 语义）

- 状态：verified（2026-09-12，T03 实测：`cargo test -p everything-manual --test storage` 15 通过、`cargo test --workspace` 77 通过、`cargo xtask check` 7/7、`cargo xtask dist` 两次哈希一致 `1aaeb2df…` + `smoke-bootstrap` 通过；证据见 [implementation](requirements/web-mvp/implementation.md) §T03-4 与 `artifacts/web-mvp/t03-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T03）。
- 背景与证据：PRD REQ-004/AC-007/AC-008、contracts §1/§2/§4/§5、architecture §3/§6/§7；SQLx 0.9.0（bundled SQLite，MSRV 1.94）实测源码见 `~/.cargo/registry/.../sqlx-{core,sqlite,macros-core}-0.9.0`。
- 结论与原因：
  1. **时间列统一 INTEGER Unix 毫秒（UTC）**；API/日志经 `manual_core::timestamps::Timestamp` 转 RFC3339。原因：租约、会话过期、`next_run_at` 需要可靠比较；文本时间一旦格式不统一（有无小数秒）会静默破坏排序与 `<` 判断。契约的 RFC3339 是**线上格式**，不是存储格式。
  2. **枚举：SQL 值 snake_case、JSON 线上值 camelCase**（`waiting_provider` ↔ `waitingProvider`、`page_image` ↔ `pageImage`）。`AssetPurpose` 的迁移字面量开发期从 `pageImage` 修正为 `page_image` 以统一约定（当时该迁移尚未交付任何真实 data-dir）；枚举与迁移字面量的漂移由 `tests/storage.rs::enum_sql_values_match_schema_check_constraints` 守护。
  3. **迁移拆两条并只追加**：`0001_core_schema.sql`（表/唯一键/外键/CHECK/索引）+ `0002_invariants.sql`（触发器 + 部分唯一索引）。拆分让"旧程序 schema（v1）→ 新程序（v2）"的自动升级路径有真实测试对象，也把"表结构"与"写入不变量"分层。
  4. **用触发器钉住"必须保证"**：`generation_snapshots` 拒绝 UPDATE（输入冻结）、`manual_releases` 拒绝 UPDATE/DELETE（发布不可变）、`provider_attempts.remote_task_id` 只允许 null→值或同值（不覆盖、不清空）、`provider_attempts` 未对账状态（intent/submitting/unknown）每阶段至多一行（部分唯一索引）。原因：这些约束靠调用顺序无法保证，钉在 schema 上可像唯一键一样被测试直接验证（`invariant_triggers_...` 用例）。后续要新的写入模式必须加迁移，不能绕过。
  5. **schema 兼容门禁先于迁移执行**：读取 `_sqlx_migrations` 的最大成功版本，`> 程序版本` 即拒绝打开（退出码 4、可读错误），并用"主库文件字节前后 sha 相同"证明不修改数据（`future_schema_...` 用例）。`success=0` 的残留记录（未完成迁移）单独报错，不当作正常状态继续。
  6. **`init` 创建并迁移数据库、`serve` 启动自动迁移、`check` 只读三态**（Missing/Ready/Pending）：承接 ADR-011 注 9（"数据库由 T03 迁移创建"）；`check` 不创建库、不应用迁移，只报告状态（旧 schema → "待迁移"），因此 T20 的"停服检查"语义不被 `check` 破坏。库比程序新时 `check`/`serve`/`init` 均退出码 4。
  7. **`StorageError` 约束分类含 SQLite 实测特例**：外键 RESTRICT 拒绝父行删除经 SQLite 上报为扩展码 **1811（SQLITE_CONSTRAINT_TRIGGER）**，与触发器 `RAISE(ABORT, …)` 同码；按 SQLite 的稳定消息"FOREIGN KEY constraint failed"区分两者（前者 `ForeignKeyViolation`，后者 `ConstraintViolation`）。这是实测结论，不是文档推断。
  8. **仓储接口取 `&mut SqliteConnection` 而非 `&SqlitePool`**：调用方（T11 起）需要把多个原语与账本写入放进同一短事务（`pool.begin()` + `&mut *tx`），pool 形参无法组合事务；连接形参也让 `repo` 不持有池、不跨请求持有事务。
  9. **SQLx 0.9 的 `SqlSafeStr` 强制静态 SQL**：`sqlx::query("...")` 只接受 `&'static str`（或显式 `AssertSqlSafe`），动态字符串在编译期报错。这与"静态 SQL + bind、不拼用户字符串"的合同一致；唯一例外是测试里按常量表名拼 `PRAGMA foreign_key_list`，用 `AssertSqlSafe` 并注释来源。另：`Migration.sql` 在 0.9 是 `SqlStr`（取字符串用 `.as_str()`），`Migration`/`Migrator` 字段虽 `pub` 但标 `doc(hidden)`、semver-exempt，本卡只在测试里用 `Migrator::new(path)` 与内嵌 `MIGRATOR`。
  10. **数据库文件与 `-wal/-shm` 收紧到 0600**（data-dir 0700 是保护边界，这里不依赖 umask）；失败被忽略（与 T02 锁文件处理一致）。
  11. **debug 构建默认增量编译不可复现**（相同源码两次构建 sha256 不同，实测）：迁移重编译证据必须在 `CARGO_INCREMENTAL=0` 下采集（基线 `771a4b40…` = 恢复后；改 SQL 后 `99122411…`）。发布路径的可复现性仍由 `cargo xtask dist` 两次一致证明。
- 影响的 REQ／任务／模块：REQ-004（主）、REQ-001/003；T04（`Database` 接入应用状态、`/health/ready` 的 DB 检查、`admins`/`sessions` 写入、422/412 映射）、T06（blobs/assets 写入与 `storage_state`）、T07（items 业务校验与分页）、T09（preparations/pages 的 ready 冻结）、T10–T15（job_stages/attempts/ledger 的租约与幂等）、T19（drafts/releases 与发布不变量）、T20（WAL 一致备份、恢复校验）。
- 未验证边界与下一步：`/health/ready` 尚未检查 DB（属 T04 接线）；`check` 仍有日志/写探针副作用（T20 可加显式只读标志）；迁移原子性是"单条迁移"粒度（多条中第 N 条失败 → 停在一致的前一版本，下次启动重试）；非 Unix 平台无权限收紧（T02 已拒绝启动）；`storage/repo` 目前只有 items，其余表由后续卡追加。
- 相关链接：`migrations/0001_core_schema.sql`、`migrations/0002_invariants.sql`、`crates/server/src/storage/{db,migrations,error,repo/items}.rs`、`crates/core/src/{domain,ids,timestamps}.rs`、`crates/server/tests/storage.rs`、`crates/server/build.rs`、[SQLx 0.9 changelog](https://github.com/launchbadge/sqlx/blob/main/CHANGELOG.md)。

## ADR-013 — T04 认证/API 基础的落地取舍（CSRF 派生、Secure 判定、限速形状、requestId 同源、If-Match 严格性）

- 状态：verified（2026-09-12，T04 实测：`cargo test -p everything-manual --test auth_api` 13 通过、
  `cargo test --workspace` 115 通过、`cargo xtask check` 7/7、`cargo xtask dist` 两次哈希一致
  `b8aa9172…` + `smoke-bootstrap` 通过；手工 curl 冒烟见 [implementation](requirements/web-mvp/implementation.md) §T04-4）。
- 日期／作者角色：2026-09-12 · RD（T04）。
- 背景与证据：PRD REQ-002/REQ-007、§5.1/§5.7；AC-003/AC-004/AC-012（settings 侧）/AC-066（错误结构侧）；
  architecture §7；contracts §1/§2/§3；T02 遗留"init 不持久化密码"、T03 遗留"ready 未接数据层"；
  SQLx `sessions` 表在 0001 已建（`session_token_hash UNIQUE`、`csrf_hash`、`expires_at > created_at` CHECK）。
- 结论与原因：
  1. **CSRF token 由会话 token 派生**（`sha256(token || ":csrf-v1")`，库中只存 `sha256(csrf)`）。
     原因：表结构只有 `csrf_hash`，"库里只存哈希"与"`GET /auth/session` 能把同一个 CSRF token 交还前端"
     只能通过派生兼容——服务端每次请求都能从 HttpOnly cookie 得到会话 token 明文，跨站攻击者拿不到。
     若未来改为独立随机 CSRF token，需要新列（迁移追加）而不是改这一行为。
  2. **cookie `Secure` 只用服务端已知事实判定**（`public_origin` scheme / 内置 TLS 配置；显式 `always|never` 覆盖），
     **不读取任何 `X-Forwarded-*`**。原因：T02 未实现内置 TLS 监听，唯一现实路径是反向代理；
     未验证的转发头可以让攻击者也来"声明"HTTPS。代价：代理终止 TLS 的部署必须显式 `session.cookie_secure="always"`，
     已在配置示例与 implementation §T04-6 注明。
  3. **登录限速是内存固定窗口、按来源 IP、只计失败**；`X-Forwarded-For` 只在**对端位于 `trusted_proxy_cidrs`**时
     参与（右往左取第一个不受信地址，解析失败/全受信退回对端，fail-closed）。原因：限速是可用性防护而不是
     强安全边界；把未受信转发头纳入会让攻击者用一个头绕过或嫁祸他人。进程内状态在重启后清零（单管理员自托管）。
  4. **requestId 全链路同源**：最外层日志中间件生成 UUIDv7 放入请求扩展，错误体/响应头/`http_request` 日志
     使用同一个值（`ApiError::render(&RequestId)`；fallback 与 `JsonBody` 拒绝也取扩展）。原因：T01 时错误体与
     日志各生成一个 UUID，"给我这个 id 我查一下"在真实排障里会断链；这是对 T01 的**行为修正**，无合同变化。
  5. **`If-Match` 严格解析**：接受 `"r7"` 与宽容的 `r7`；`*` 与弱标签 `W/`、列表、非正整数一律 422。
     原因：`*` 会让"必须读后才改"退化成无条件更新；弱比较不适用于写前条件。缺头 428、过期 412
     （`details.currentRevision`）由仓储 CAS 错误映射保证，不靠 handler 自查。
  6. **`JsonBody` 提取器把 axum 默认纯文本拒绝转成合同错误**（413/415/422），JSON 体积上限落
     `DefaultBodyLimit`（默认 1 MiB）。原因：默认拒绝会绕过统一错误结构；1 MiB 是 PRD §5.3 的 JSON 上限，
     但**不能**套用到 T06/T09 的 multipart（它们需要独立上限）。
  7. **`init` 再次执行 = 重置密码**：更新 `admins.password_hash` 并撤销该管理员全部会话（留痕 `revoked_at`），
     没有独立命令。原因：单管理员自托管没有"忘记密码"的救援通道；能读 data-dir 的人本就能改库，
     拒绝重跑只会制造运维死角。输出明确写"已更新"。
  8. **Argon2id 固定 `Argon2::default()` 参数**（v19、m=19456 KiB、t=2、p=1、16 字节随机盐、PHC 编码），
     哈希/校验走 `spawn_blocking`。原因：参数是安全属性的一部分，写进代码与文档（`password.rs`）比隐式默认好审计；
     19 MiB 内存操作绝不能占用异步运行时线程。
  9. **`/health/ready` 503 使用 `{data}` 结构**（process/data_directory/database/migrations），不改成错误信封。
     原因：运维需要看到哪一项失败；T01 的 DTO 与 bootstrap 断言已按此修订，属于对既有实现的扩展而非新合同。
- 影响的 REQ／任务／模块：REQ-002/REQ-007（主）、REQ-043/REQ-044；T05（fixture 关闭 AC-013 的 release 侧）、
  T06（multipart 的 CSRF 覆盖已生效，但需自己的体积上限与资产授权）、T07（items 业务规则补齐，含列表过滤与
  字段级 422）、T08（前端按 `sessionCookie` + `X-CSRF-Token` + `If-Match` 消费生成类型）、T16+（401/412 恢复路径）。
- 未验证边界与下一步：内置 TLS 监听未实现（cookie `Secure` auto 在纯代理部署下不会自动为真）；
  限速状态不跨进程/不持久；会话清理只在登录时；`If-Match` 尚未在 T07 之外的聚合根（draft/job）落地；
  `items` 的 PATCH 不能显式清空可选字段；AC-013 的真实 release 语义待 T05 fixture 复核。
- 相关链接：`crates/server/src/http/auth/{mod,password,tokens,limiter}.rs`、`crates/server/src/http/{precondition,body,error,state}.rs`、
  `crates/server/src/config/{mod,file,commands}.rs`、`crates/server/tests/{auth_api.rs,common/mod.rs}`、
  `artifacts/web-mvp/t04-rd/`。

## ADR-014 — T05 fixture 测试设施落地取舍（std 自建服务器、场景即数据、仅回环客户端、内建样例资产）

- 状态：verified（2026-09-12，T05 实测：`cargo test -p everything-manual --test fixture_harness` **18 通过**
  （debug 与 `--release` 各一遍）、`cargo test --workspace` 133 通过、`cargo xtask check` 7/7、
  `cargo xtask dist` 哈希与 T04 **逐字节相同** `b8aa9172…` + `smoke-bootstrap` 通过、
  `cargo tree --edges normal` 0 命中 test-support；证据见
  [implementation](requirements/web-mvp/implementation.md) §T05 与 `artifacts/web-mvp/t05-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T05）。
- 背景与证据：PRD REQ-008（主）/REQ-007、AC-014/AC-013、§5.6/§5.7；architecture §3（"fixture 默认；
  真实收费验证另设授权入口"）与 §4（`crates/test-support` 仅 dev-dependency）；contracts §6（"真实 HTTP 适配器
  必须先用本地 fixture 验证字节协议，不能只测永远成功的 fake trait"）；validation-release §2（fixture URL 准入
  经仅测试构建开关与显式配置，不允许 release 因缺 `TRIPO_API_KEY` 自动走 fake）。
- 结论与原因：
  1. **fixture 服务器用 std 自建（`TcpListener` + 线程），不引入框架/异步运行时**。原因：需要发出
     字节级"坏"行为（RST、只发一半再 FIN、只读不答、原样畸形 JSON），这些用 axum/hyper 反而要绕回
     原始字节；同时让 T12/T14 的异步 reqwest 适配器与 fixture 天然解耦（fixture 只服务协议）。代价：
     自己实现了 HTTP/1.1 请求解析（含 chunked 与 `Expect: 100-continue`），有 16 MiB 请求体与 64 KiB 头部上限。
  2. **场景是数据（`tests/fixtures/scenarios/*.json`），路由按顺序消费步骤**；步骤耗尽且未声明
     `repeatLast`、或没有匹配路由时返回 **501** 并记入 `script_problems()`。原因：卡文要求"缺脚本必须失败，
     不能返回通用成功"；把失败做成可断言的显式状态（而不是静默 200），后续卡就不会"以为 fixture 在测别的东西"。
  3. **请求记录先于行为执行**：断连/超时/RST 场景也保留"调用了几次、发了什么"。**敏感头脱敏后保留 scheme**
     （`Bearer [REDACTED]`）：既守住"不落密钥"，又保留"是否携带 bearer"这一 T13 需要的断言点。
  4. **仅回环测试客户端 `LocalHttpClient` 作为"无真实外网调用"的机制**：目标必须是 IP 字面量（不做 DNS）
     且 `is_loopback()`，`https://` 直接拒绝。这比"约定不要外呼"更接近可验证的构造性保证；负例断言在
     `guarded_client_refuses_non_loopback_targets`。T12/T14 用 reqwest，但该保证靠"所有 fixture 地址只可能是
     127.0.0.1 的随机端口 + 测试断言"延续（T21 安全回归再复核）。
  5. **样例资产由仓库代码生成、sha256 固定、测试内双向比对**（生成器 → 与仓库字节、与固定哈希各比一次）。
     原因：卡文要求"原创 + 记录来源与许可 + 可复核"；把生成逻辑放进库里（而不是只在 bin 里）是为了让
     "重新生成的字节 == 提交的字节"成为自动化断言，而不是人工声明。不引入任何图像/PDF 编码 crate
     （PNG 用 zlib stored 块、JPEG 用自建最小 Huffman 表、PDF 手写 xref），代价是样例内容刻意极简。
  6. **扫描型 PDF 的全部字节控制在 ASCII 且不含文字算子字母**（像素取值集合刻意避开 `BT`/`endstream` 等
     序列）。原因：结构校验按字节偏移解析（startxref → xref），任何编码转换都会让偏移错位；同时让
     "无文字层"的启发式判定可靠。
  7. **生产隔离用四重证据**（测试内清单断言 + `--edges normal` 的 cargo tree + release 二进制 strings +
     默认 base_url 断言）。原因：T02 QA 已记录"零外呼有编译期证据"，本卡把它升级为"fixture 代码不在生产
     产物里"的可复核链条；dist 哈希与 T04 相同本身就是"未碰生产路径"的最强证据。
  8. **AC-013 的默认入口侧边界**：`/health/ready` 不依赖云端 + 缺密钥时 `providersConfigured=false` +
     fixture 0 次调用；正式包的冷启动与"不连接本机 Provider fixture"仍属 T22，真实链路属 T23。
- 影响的 REQ／任务／模块：REQ-008/REQ-007（主）；T06/T09（上传与准备测试可复用"外呼计数"）、
  T10（任务执行器故障注入可复用场景脚本）、T12/T14（适配器契约测试必须用本 fixture 与记录断言）、
  T15/T21（端到端与故障矩阵）、T22（正式包不连接 fixture）、T23（test-live 的授权入口与预算校验）。
- 未验证边界与下一步：真实适配器（reqwest）尚未接入，字节协议断言要到 T12/T14 才落地；场景响应是
  构造样例、需按官方文档在 T12/T14 修订；`reset` 在非 Unix 平台退化为普通关闭；不模拟 TLS 握手中断；
  fixture 无鉴权、不是安全边界测试对象。
- 相关链接：`crates/test-support/src/{server,scenario,client,record,assets,generate,presets}.rs`、
  `crates/test-support/src/bin/generate_fixtures.rs`、`crates/server/tests/fixture_harness.rs`、
  `tests/fixtures/{README.md,assets,responses,scenarios}`、`artifacts/web-mvp/t05-rd/`。

## ADR-015 — T06 上传与资产服务落地取舍（两层体积防线、先文件后元数据、隔离不删、Range 语义、磁盘满 413）

- 状态：verified（2026-09-12，T06 实测：`cargo test -p everything-manual --test assets` **12 通过**、
  `cargo test --workspace` **172 通过 ×6 轮**、`--test-threads=16` 4 轮 + 高负载 1 轮全过、
  `cargo xtask check` 7/7、`cargo xtask dist` sha256 `0c457a2a…` + `smoke-bootstrap` 通过、
  真实 data-dir curl 冒烟（206/416/304/HEAD/去重/崩溃隔离）全符合预期；证据见
  [implementation](requirements/web-mvp/implementation.md) §T06 与 `artifacts/web-mvp/t06-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T06）。
- 背景与证据：PRD REQ-011/AC-018/AC-019、§5.3；contracts.md §2/§3/§7；architecture.md §6/§7；
  T06 卡；ADR-012（blobs/assets 列、枚举 SQL/线上取值）、ADR-013 第 6 条（multipart 不受 1 MiB JSON 上限约束）。
- 结论与原因：
  1. **multipart 用 axum 官方 feature（multer），两层体积防线**：路由级 `DefaultBodyLimit` 取
     "最大用途上限 + 1 MiB 开销"（解析前拒绝），`StagedWriter` 读流时按用途上限再次计数。
     **实测结论（供 T09 复用）**：axum `DefaultBodyLimit` 是"最后写入请求扩展的值生效"——路由层在中间件
     之后执行，因此**路由级上限可以覆盖外层 api_v1 的 1 MiB JSON 上限**；反过来把大上限放在外层则无法被
     内层缩小。`purpose` 字段可能排在 `file` 之后，因此读流先用最大上限，读完再按用途上限兜底（413）。
  2. **先文件、后元数据；请求路径永不删除 blob 文件**：`tmp` 流写 + fsync → 原子 rename 到
     `blobs/<sha 前 2 位>/<sha>` → fsync 目录 → 短事务（blob 幂等插入 + asset 行）。DB 失败时最多留下
     "孤儿文件"，而**不**尝试删除——同 sha256 可能已被其他 asset 引用（AC-019 的共享 blob 场景），
     删除会让现有资产变成"元数据在、内容不在"。孤儿由启动扫描按引用处理。
  3. **崩溃残留隔离而不是清理**：`serve` 启动（持锁、迁移后、监听前）扫描 `tmp/*` 与 `blobs/**`，
     只移动不删除；被任何 blob 行引用的文件（含 `quarantined`/`missing` 状态与多资产共享）绝不移动。
     同一遍扫描收敛 `missing ↔ stored`。选择"移动"是因为崩溃现场对排障有价值、删除不可逆；
     `quarantine/` 不进 T02 的必需目录（`check` 保持只读、旧 data-dir 不被判为结构不完整）。
  4. **内容服务语义**：ETag 用内容 sha256 的强校验器（资产不可变、去重后同内容共享 ETag）；
     单区间 206、不可满足 416 + `Content-Range: bytes */N`、多区间与非法语法回落完整 200、
     `If-None-Match` 弱比较 304、`If-Range` 只认强校验器（日期形式视为不匹配 → 200）。
     GET/HEAD 共用一个响应构造器且 HEAD 显式注册（不依赖 hyper 自动去 body），避免"测试有 body、
     真实环境无 body"的差异。不挂压缩中间件，206 也不带 `Content-Encoding`。
  5. **磁盘满映射为 413 + `details.reason=insufficientStorage`**（而不是 507）：contracts §1 的稳定错误码
     集合没有 507/独立码，本卡不允许修改 `crates/core::ApiErrorCode`；PRD AC-019 只要求"明确的磁盘不足错误"，
     用 `details.reason` 给前端零歧义识别。"是否新增独立错误码"作为合同变更提案留给 PM。
  6. **`pageText`(2 MiB)、图片单边(20 000 px)、解码像素(80 MP) 是 T06 常量**：PRD §5.3 只写"页文字另设
     更高但有限的上限"，未给数值；常量比新增配置键更小、可回退（见 implementation §T06-8 的提案）。
  7. **PDF 页数上限是"上传第一道防线，T09 权威"**：探针只做有界读取的结构判定；页树被对象流（`/ObjStm`）
     压缩时返回 `Unparsed` **放行**并记 WARN，加密 PDF（`/Encrypt`）接受上传——两者按 REQ-012/REQ-014 由准备
     阶段拒绝。原因：把"真实厂商 PDF 一律判 422"挡在门外，而权威拒绝位置本来就在 T09。
  8. **`serve` 接线是本卡对允许文件清单的唯一越界改动**：崩溃隔离必须由真实启动路径调用，否则只是库函数；
     只加约 25 行调用与日志，失败不阻塞启动（此时尚无资产流量）。
  9. **BUG-001 用可注入时钟修复**（`TimeSource::System | Manual(Arc<ManualClock>)`）：生产构造 `new()` 的
     行为与默认值完全不变；测试不再 `thread::sleep`，窗口推进由测试驱动，原有断言一条未删（另补 3 条更强断言
     与 1 个 1000 次迭代的确定性守护用例）。理由：真实时钟依赖是结构性问题，放大窗口只是降低概率。
  10. **依赖增量最小化**：只加 axum `multipart` feature（multer）+ tokio `fs`/`io-util` + `tokio-util`
      （`ReaderStream`），未升级任何既有 crate；响应体流式读取避免把大资产读进内存。
- 影响的 REQ／任务／模块：REQ-011（主）；T07（`repo::assets::find_for_item` 作为归属校验入口、物品累计口径）、
  T09（页资产复用同一上传路由与限制；加密与超页数的权威拒绝）、T13（模型资产走同一 blob 存储，用途/上限另定）、
  T20（隔离区与 blob 引用扫描进入备份/运维范围）、T21（上传/路径/越权故障矩阵）。
- 未验证边界与下一步：磁盘满的 413 vs 507 待 PM 决策；页文字/像素上限是否配置化待 T09 实测；
  PDF 对象流与加密 PDF 的真实厂商样本行为待 T09/T23；隔离区暂无清理/管理界面；启动扫描为全量遍历
  （大数据量下的时延未测）；非 Unix 平台无 `statvfs`（跳过预留检查）未在目标平台验证。
- 相关链接：`crates/server/src/assets/{blob_store,validate,pdf,upload,maintenance,range,error}.rs`、
  `crates/server/src/http/assets.rs`、`crates/server/src/storage/repo/{blobs,assets}.rs`、
  `crates/server/src/http/auth/limiter.rs`、`crates/server/tests/assets.rs`、
  [implementation §T06](requirements/web-mvp/implementation.md)、`artifacts/web-mvp/t06-rd/`。

## ADR-016 — T07 物品/资料/版本落地取舍（PATCH 清空语义、视图唯一、游标绑定、字段级 422）

- 状态：verified（2026-09-12，T07 实测：`cargo test -p everything-manual --test items` **11 通过**、
  `cargo test --workspace` **195 通过**、`cargo xtask check` 7/7、
  `cargo xtask dist` 两次哈希一致 `287c05c3…` + `smoke-bootstrap` 通过、
  真实 data-dir curl 手工冒烟（创建→上传→绑定→照片→归档→引用核对→405）符合预期；
  证据见 [implementation](requirements/web-mvp/implementation.md) §T07 与 `artifacts/web-mvp/t07-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T07）。
- 背景与证据：PRD REQ-010/012/013/017、AC-016/017/020/021、§5.3；contracts §1/§2/§3/§7；
  T04 QA 回合 4"特别裁定 2"的 T07 待补清单（`{"brand":null}` 静默保留原值、无创建端点、
  无字段级 422、列表不过滤归档、游标在过滤条件下的语义）；ADR-012（时间/枚举）、ADR-013（If-Match/硬约束）、
  ADR-015（`details.reason` 惯例、迁移只追加）。
- 结论与原因：
  1. **PATCH 可选字段显式 `null`/空白 = 清空**（必填字段 `null`/空白 = 422）：用 serde 双层 Option
     （`http::body::double_option`）区分"字段缺失"与"显式 null"，配合 `core::validation` 规范化。
     原因：T04 的实现无法区分两者，`{"brand":null}` 会"保留原值却递增 revision"——对客户端是静默假成功。
     状态字段 `archived` 的显式 `null` 同样 422（状态没有"清空"语义，缺失才是保持）。
     空 PATCH 直接 422（不静默无操作、不空递增 revision）；校验失败一律不写库。
  2. **同视图第二张照片 = 拒绝**（不覆盖）：服务层在同一短事务内先查占用者，返回
     422 `details={reason:"viewOccupied", view, existingPhotoId}`；迁移 0003 的
     `photos_item_view_unique(item_id, view)` 是并发窗口的最终防线（索引冲突映射为同一 reason）。
     原因：合同写"每视图最多一张"，静默覆盖会让 UI 槽位与已绑定的快照不一致；错误里给出
     `existingPhotoId` 才能让 UI-012 提示"请先移除或改选"并给出可操作路径（改选 = PATCH 已有照片）。
     **不提供照片 DELETE**：合同路由表未定义，擅自新增会扩大删除面；"移除"暂由"改选"覆盖（记录在案）。
  3. **列表游标绑定查询条件**（`v1:<scope>:<sortMillis>:<id>`）：scope 为
     `items:active`/`items:archived`/`documents:<itemId>`，格式错误或跨过滤条件复用 → 422。
     原因：T04 的游标只是"位置"，换过滤条件复用会静默跳页/重复；把它变成显式拒绝比"文档里写一句
     注意事项"更符合本项目"不静默"的原则。游标对客户端始终不透明（不得自行拼接）。
  4. **字段级 422 统一 `details.fields=[{field,message}]`**（`field="body"` 表示请求体整体问题）：
     一次返回全部字段问题，前端可一次聚焦全部错误；`message` 保留首个问题的通用摘要。
     `view` 等枚举字段在请求 DTO 里用**字符串**而不是 serde 枚举，就是为了拿到字段级明细
     （serde 枚举反序列化失败只会在提取器层得到通用 422）；DTO 里用 `value_type` 保持 OpenAPI 的
     `string` 类型与示例。
  5. **长度上限由 RD 取值并集中定义**（PRD 只写"超长 422"）：name/model ≤200、brand ≤100、variant ≤200、
     title ≤200、sourceUrl ≤2000 字符（按 `chars().count()`，先 trim）。常量在 `manual_core::validation`；
     OpenAPI 的 `maxLength` 字面量用**编译期断言** + 单测守护，避免两处漂移（utoipa 不接受常量表达式）。
  6. **`sourceUrl` 只存不取**：只校验绝对 http(s)、无用户凭据、无空白、长度上限，然后原样保存；
     服务端**没有任何**抓取路径（依赖树里也没有 HTTP 客户端）。测试用本机计数监听器证明 0 连接。
     原因：REQ-012 与 ADR-007 都明确"来源链接仅作出处"，任意的服务端抓取是 SSRF 面。
  7. **跨物品/未知资产一律 404 且响应逐字相同**：`repo::assets::find_for_item`（T06 交付）是唯一归属入口，
     document/photo 绑定都必须经过它；"存在但不属于你"与"不存在"不可区分（测试断言 message 相等）。
  8. **归档语义**：`archived_at` 记录首次归档时间（重复归档不刷新）；默认列表 `archived_at IS NULL`、
     `archived=true` 为 `IS NOT NULL`（对称过滤）；归档不删除任何资料，document/photo/资产内容仍可读；
     没有 `DELETE` 路由（405），未知删除型路径 404。编辑物品不改变既有 document/photo 引用
     （REQ-017 的结构性前提；快照冻结属 T11）。
  9. **迁移 0003 只追加一个唯一索引**（不改 0001/0002）：表结构在 T03 已建齐，"每视图最多一张"此前
     只是服务层约定；把它钉到 schema 上与 0002 的"不变量钉在 SQL 上"同一思路。
  10. **读取侧路由的申报**：新增 `GET /items/{id}/documents`、`GET /items/{id}/photos`、
      `GET /items/{id}/photos/{photoId}`（contracts §3 列核心路由，未禁止读取集合）。
      原因：UI-011/UI-012 的文档卡片与五槽位、AC-016"归档后资料仍可读"的可验证性都依赖集合读取；
      没有它就只能靠直读数据库证明。**若协调者认为路由清单封闭，请补记而不是回滚**（回滚会同时失去可验证性）。
  11. **REQ-010 的"可选来源链接"没有物品级落点**（记录为限制，不改合同）：contracts §2/§3 的 items
      数据模型与输入不含 source_url；本卡按冻结合同实现，出处链接由 document 承载。
      若 PM 要求物品也保存来源链接，需先修订合同（新增列 + 迁移）。
- 影响的 REQ／任务／模块：REQ-010/012/013/017（主）、REQ-016/020/021（下游读取同一集合语义）；
  T08（用生成的 DTO；`nextCursor` 是不透明字符串、PATCH 清空语义、`details.fields`/`details.reason` 渲染）、
  T09（document/`sourceSha256` 与页资产归属继续走 `find_for_item`）、T11（多视图集合 `list_multiview_for_item`、
  快照冻结真身）、T12（detail 过滤在请求体侧）、T19（发布与归档交互）、T21（删除语义与归属回归）。
- 未验证边界与下一步：照片无删除 API（需合同变更）；归档后是否应禁止编辑（需 PM 裁定）；
  `sourceUrl` 的可达性/内容不验证（设计如此，与"不抓取"一致）；文档/照片集合的读取侧路由需协调者补记；
  多视图集合只到数据层，Tripo 请求体断言在 T12；REQ-017 的快照冻结在 T11。
- 相关链接：`crates/core/src/validation.rs`、`crates/server/src/http/{items,documents,photos,pagination,body,error}.rs`、
  `crates/server/src/http/dto/{items,documents,photos}.rs`、`crates/server/src/storage/repo/{items,documents,photos}.rs`、
  `migrations/0003_photos_view_unique.sql`、`crates/server/tests/items.rs`、
  [implementation §T07](requirements/web-mvp/implementation.md)、`artifacts/web-mvp/t07-rd/`。

## ADR-017 — PRD 修订 2 四项挂起决策（物品级来源链接、磁盘满错误形态、UI 假设 U-01–U-12、内置 TLS 边界）

- 状态：accepted（2026-09-12，PM 裁决，对应 prd_revision=2；已验收切片 T01–T07 判定**无 needs_retest**，依据见 prd.md §9.1 影响面）。
- 日期／作者角色：2026-09-12 · PM（web-mvp，PRD 修订 2）。
- 背景与证据：QA 回合 2 非阻断建议 1（内置 TLS 监听无实现卡）、回合 6 特别评估项 1（磁盘满 `413+details` vs 507）、回合 7 特别评估项（REQ-010 物品级来源链接无落点，建议 B）与裁定核对 1（PATCH 清空语义留痕建议）；UI §6.3.3 的 U-01–U-12 待 PM 确认（U-08/U-11 涉及 REQ-036/REQ-039 可访问性承诺）；`state.yaml.next_action` 要求裁决 D-1–D-4 后才能派发 T08。
- 结论与原因：
  1. **D-1 取消物品级来源链接**（QA 建议 B）：REQ-010 输入移除该字段；出处链接仅由 `document.source_url` 承载（REQ-012/UI-011，AC-020 已验证）。理由：contracts §2/§3 与迁移均无物品级列，无任何 AC 覆盖；保留需加列 + 迁移 + DTO/OpenAPI 重生成 + 重验，且无功能收益；物品接口对未知字段 422（不静默）的既有行为不变。
  2. **D-2 磁盘满沿用 `413` + `details.reason=insufficientStorage`**：不新增 507/独立错误码。理由：contracts §1 稳定码集合无 507，AC-019 只要求"明确错误"；T06 已实现并经 QA 真实磁盘镜像实测，前端可零歧义识别；改码需合同变更 + 代码 + 重验而无用户可见收益。**合同留痕请求**：请协调者在 contracts §1/§7 补记该形态（PM 不改合同）。T06 QA 非阻断建议 1（写流中途 ENOSPC 现为 500 通用错误）不属本裁决范围，仍待后续卡。
  3. **D-3 U-01–U-12 全部接受为产品决定/UI 默认**（含 U-03/U-04/U-12 的补充约束；无 U 项升格为 REQ/AC）；**U-08 与 REQ-039/AC-060 无冲突**（该 AC 只要求窄屏保留读能力并对不支持的能力明确禁用，窄屏发布仍受 REQ-035 服务端不变量约束）；**U-11 与 REQ-036/AC-057 无冲突**（键盘承诺范围是"非 3D 核心操作"，热点绑定属 3D 内部拾取，部件列表已提供文字替代路径）。详见 prd.md §8.5。
  4. **D-4 MVP 不要求内置 TLS 监听**：部署边界＝loopback 或显式受信反向代理（非 loopback 仅代理放行，代理终止 TLS）；配置 `tls.*` 时的 fail-closed 拒绝启动为期望语义（不降级明文）。理由：architecture §7 是"内置 rustls **或**受信反向代理"的或关系，代理路径已满足"必须认证和 TLS"的承诺；T02 实测即 fail-closed（exit 6）且 QA 已把该行为当合同值核对；T01–T23 无实现卡，MVP 内的对外承诺必须可兑现。**对 ADR-011 第 5 条/T22 预期的修订**：内置 TLS 监听从 MVP 范围移除；若后续要求，属新增范围（新卡 + PM 新修订）。T22 发布报告须声明部署边界、不得宣称"内置证书/开箱即用 HTTPS"。
- 影响的 REQ／任务／模块：REQ-003/010/011/012；AC-006（T02）与 AC-019（T06）为**文本澄清**（命名已在 QA 回合 2/6 现场记录的行为，无阈值变化，不回退 accepted）；T07（物品级字段不存在，未知字段 422 语义不变）；T08/T16（UI-006 表单字段由 UI 在 ui_revision 2 移除；T08 派发前需 UI 冻结）；T22（发布声明部署边界）；contracts §1/§7 与 §3 读取侧补记（留痕请求，协调者处理）；implementation-plan 无需改动。
- 未验证边界与下一步：写流中途 ENOSPC 的 413 映射（T06 非阻断建议 1）；照片无删除 API、归档后是否禁止编辑（ADR-016 遗留，仍待 PM 另行裁定）；U-08/U-11 声明无冲突，若未来要求窄屏校准或键盘绑定热点，需 PM 新修订。
- 相关链接：[prd.md §8.1/§8.5/§9.1/§9.2](requirements/web-mvp/prd.md)、[qa-report.md 回合 2/6/7](requirements/web-mvp/qa-report.md)、ADR-011（T02 fail-closed 与 T22 预期）、ADR-015（磁盘满 413 取舍）、ADR-016（T07 来源链接与清空语义）、AC-006/AC-019/AC-020 的 QA 证据（`artifacts/web-mvp/t02-qa/manual-cli-run1.log`、`artifacts/web-mvp/t06-qa/disk-full-real.log`）。

## ADR-018 — T08 前端框架与基础交互落地取舍（路由版本、Query 默认值、CSRF 内存态、断点双实现、游标由 URL 承载）

- 状态：verified（2026-09-12，T08 实测：`shell.test.tsx` **19 通过**、前端 `test -- --run` **30 通过**、
  `cargo xtask check` 7/7、`cargo test --workspace` 195 通过、`cargo xtask contracts --check` 一致、
  `cargo xtask dist` 两次哈希一致 `668e9958…` + `smoke-bootstrap` 通过、
  真实 Chrome 152 浏览器联调 **28/28 通过**；证据见 [implementation](requirements/web-mvp/implementation.md) §T08 与 `artifacts/web-mvp/t08-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T08）。
- 背景与证据：PRD 修订 2（ui_revision 2）§6.1/§6.2/§6.3.2/§8.5；architecture §3（React 19 + React Router 声明式路由 +
  TanStack Query + 原生 fetch 封装，不用 Redux）；contracts §1/§3；ADR-013（CSRF 派生、If-Match 严格解析、
  session `no-store`）、ADR-016（字段级 422 `details.fields`、游标绑定过滤条件、`details.reason`）。
- 结论与原因：
  1. **React Router 取 v7（7.18.3）而非当时的 latest v8**：v8 的 peer 要求 React ≥19.2.7 且属新大版本，
     本卡无可参考的迁移验证；v7 是仓库既有技术基线（声明式 `<BrowserRouter>/<Routes>`）的稳定实现，
     且 npm `version-7` dist-tag 仍在维护。升级到 v8 需要新的验证证据，不在 T08 范围。
  2. **Query 默认"不自动重放"**：`retry: false`（query 与 mutation）+ `refetchOnWindowFocus: false`，
     与 UI-002「不重放已失败请求」一致；网络类失败由页面显式「重试」按钮触发，避免 401 重试风暴与
     "断网后自动重放"造成的费用/幂等风险（T16+ 的提交类操作尤其依赖这一点）。
  3. **CSRF token 只存模块作用域内存**（`api/client.ts::setCsrfToken`），不落 localStorage/sessionStorage/DOM/日志；
     登出与任意 401 时清空。测试用"DOM 不含 token 值"作为可断言的负例。
  4. **三档断点由 JS 与 CSS 双实现、数值同源**：`useBreakpoint` 优先 `matchMedia("(min-width: 1280px|768px)")`
     （与 CSS 媒体查询同一判定源），无 `matchMedia` 的 jsdom 退化为 `window.innerWidth`；不使用 UA 判断。
     代价：两处数值需同步（1250/768），已在 `useBreakpoint.ts` 与 `styles.css` 注释互指。
  5. **列表游标由 URL 承载的语义**：`?cursor` 表示"本视图的起始位置"（服务器游标是"位置之后"），
     「加载更多」成功后才把新页起始游标写入 URL；刷新从该位置继续，另给「回到列表开头」。
     切换归档范围会同时清空 cursor（ADR-016 第 3 条：游标绑定过滤条件，跨条件复用 422）。
  6. **错误边界的 requestId 取"最近一次响应头 `x-request-id`"**（`lastRequestId()`）：渲染异常通常不带 API 错误对象，
     服务端已保证该头与日志/错误体同源（ADR-013 第 4 条）；非合同错误只显示通用文案，不暴露异常消息与堆栈。
  7. **未实现路由用显式占位页**（含计划卡片编号、`aria-current="step"` 的五步步骤条），页面不发起任何业务请求；
     顶栏"任务计数徽标"与右栏"最近使用/进行中任务"摘要因缺服务端数据而**不显示假数字**（§6.1.2 的实际入口在 T15/T17 落地）。
  8. **搜索按"已加载行内筛选"**：`GET /items` 无检索参数，页面在提示中写明筛选范围；服务端检索需合同新增参数。
- 影响的 REQ／任务／模块：REQ-002/REQ-010/REQ-039 的前端消费侧；T09/T16（复用 API 客户端、表单错误渲染、向导步骤条骨架）、
  T17（任务中心与网络/业务失败区分）、T18/T19（复用 `PageLayout` 三栏与抽屉、错误边界）、T21/T22（回归与发布 smoke）。
- 未验证边界与下一步：中段（768–1279）侧栏标签页仅资料库单面板被实测，三栏真实页面属 T18/T19；
  React Router v8 未评估；`useBreakpoint` 与 CSS 断点数值需人工保持同步（无自动守护用例）；
  浏览器联调用 Vite dev，生产单二进制下的同一页面由 T22 smoke 覆盖；触屏/移动浏览器未实测（AC-060 只承诺视口宽度行为）。
- 相关链接：`apps/web/src/api/{client,endpoints}.ts`、`apps/web/src/features/shell/{useBreakpoint,PageLayout,Drawer,AppShell,RequireSession,SessionExpiryWatcher}.tsx`、
  `apps/web/src/features/shell/shell.test.tsx`、`apps/web/src/{App.tsx,styles.css}`、
  [implementation §T08](requirements/web-mvp/implementation.md)、`artifacts/web-mvp/t08-rd/`（含浏览器联调脚本与截图）。

## ADR-019 — T09 PDF 准备落地取舍（页数上限两层分工、业务冲突 422+reason、viewport 落库、幂等按内容哈希、vendor 同版本内嵌、通知不遮挡主入口）

- 状态：verified（2026-09-12，T09 实测：`--test preparations` **9 通过**、`test:e2e -- pdf-preparation.spec.ts`
  **10 通过**（真实 Chrome + 真实后端）、前端 `test -- --run` 46 通过、`cargo xtask check` 全绿、
  `cargo test --workspace` 全通过、`contracts --check` 一致、`xtask dist` `a9e41f14…` + `smoke-bootstrap` 通过、
  release 二进制内嵌 vendor 资源 200 且 CMap sha256 与 node_modules 一致；
  证据见 [implementation](requirements/web-mvp/implementation.md) §T09 与 `artifacts/web-mvp/t09-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T09）。
- 背景与证据：PRD 修订 2 §3 REQ-014/015、§4 AC-022–025、§5.3、§6.2 UI-014–018、§6.3.2 禁用措辞；
  architecture §3「PDF」/§5.1/§7；contracts §1/§2/§3；ADR-003（浏览器完成 PDF 页准备）、ADR-009（合同单一来源）、
  ADR-013（If-Match 严格解析）、ADR-016（业务冲突用 `details.reason`）；T06 的 `assets/pdf.rs` 上传探针注释
  （"页数上限是第一道防线，权威判定在 T09"）。
- 结论与原因：
  1. **页数上限的两层分工**：T06 上传探针只做廉价文本读取（不解析间接引用/对象流），能判定时先拒绝
     （422 `details.pageCount`）；T09 在准备阶段用 PDF.js 的 `numPages` 做权威拒绝，服务端**不信任客户端**，
     在 `PUT` 页号与 `complete pageCount` 再做 `≤100` 校验（422 `details.reason=pageLimitExceeded`）。
     样例 `sample-manual-many-pages.pdf`（101 页、`/Count` 间接引用）专门走 T09 路径，e2e 断言其文案与实际页数。
  2. **业务规则冲突沿用 422 + `details.reason`**（`preparationReady`/`incompletePages`/`assetMismatch`/`sourceChanged`/
     `pageLimitExceeded`/`invalidPageNumber`），不新增错误码：contracts §1 的稳定码集合没有冲突专用码，
     T07 已按 ADR-016 形成惯例；避免为此触发 PRD/合同变更（QA 可零歧义按 reason 断言）。
  3. **viewport 落库**（`pages.viewport_json = {width,height,rotation}`，迁移 0004 追加列）：页图坐标原点定义为
     **旋转后 viewport 左上角**，后续 `bbox` 归一化需要该尺寸/旋转（contracts §2）；不写 `[0,0]` 占位，
     未渲染/旧记录为 NULL。`clientDerived` 同样落列（`preparations.client_derived`，complete 事务内置 1）。
  4. **页幂等按内容哈希**：比较 text/image 资产的 **blob sha256** 与 viewport，而不是 asset id ——
     同一内容重复上传会命中同一 blob（T06 去重），换一次上传得到的新 asset id 内容相同仍应幂等。
     判定与写入在同一事务内完成，避免"先查内容再写"的 TOCTOU；内容变化必须 `If-Match`（428/412）。
  5. **ready 门禁在仓储层**：`write_page`/`complete` 在自己的事务内先读 `state`，非 `preparing` 一律
     `StorageError::NotWritable`（HTTP 映射 422 + reason），不依赖调用方"先查再写"。
  6. **vendor 资源由 Vite 插件同版本内嵌**：不从 CDN、不手工拼 worker 路径（`?url` import 交给 Vite），
     CMaps/standard fonts/WASM/ICC 由插件从 `node_modules/pdfjs-dist` 复制进 `dist/vendor/pdfjs/`，
     缺目录即构建失败；dev 由中间件直接服务同一目录，版本漂移不可能发生。
  7. **Playwright 自管后端与端口**：`test:e2e` 用 `globalSetup` 构建并启动真实后端（临时 data-dir + `init`），
     `webServer` 用 `EM_WEB_PORT`/`EM_API_PROXY_TARGET` 起独立前端（15173→18080），不依赖外部已运行服务；
     失败保留 trace/截图与后端日志。浏览器固定 `@playwright/test@1.60.0`（与构建机缓存 chromium 1223 对齐）。
  8. **通知不得遮挡主入口指针操作（BUG-001-r8 的规则）**：通知条定位到顶栏之下（实测顶栏高度驱动
     `--notices-top`），容器 `pointer-events:none` + 通知体 `auto`；规则对未来所有通知/浮层复用。
- 影响的 REQ／任务／模块：REQ-014/REQ-015（T09 交付）；T11（`preparation ready` 作为报价前置）、T14（页文字/页图作为
  提取输入与出处）、T16（向导第 4 步复用本页）、T17/T19（状态与出处展示）、T21/T22（性能、离线与内嵌 vendor 复核）、
  E03（无浏览器准备改走原生渲染时需重评本 ADR）。
- 未验证边界与下一步：WASM 代码路径未被样例触发（仅内嵌与可服务证据）；100 页规模的耗时/内存未实测（T21 度量）；
  beforeunload 仅用合成事件断言（真实弹窗留 T21 人工走查）；e2e 用 Vite dev，生产内嵌 UI 的同 spec 由 T21/T22 补；
  `--notices-top` 依赖 100ms 轮询（仅通知可见期间）。
- 相关链接：`crates/server/src/http/preparations.rs`、`crates/server/src/storage/repo/preparations.rs`、
  `migrations/0004_preparation_pages.sql`、`apps/web/src/features/import/**`、`apps/web/vite.pdfjs-vendor.ts`、
  `apps/web/tests/e2e/pdf-preparation.spec.ts`、`crates/server/tests/preparations.rs`、`tests/fixtures/README.md`、
  [implementation §T09](requirements/web-mvp/implementation.md)、`artifacts/web-mvp/t09-rd/`。

## ADR-020 — T10 持久执行器落地取舍（依赖边落库、租约 guard 三分、付费错误语义、恢复先接管）

- 状态：verified（2026-09-12，T10 实测：`cargo test -p everything-manual --test jobs_recovery` **22 通过**（连续 4 次）、
  `cargo test --workspace` **236 通过**、`cargo xtask check` 7/7、`cargo xtask contracts --check` 无漂移、
  `cargo xtask dist` `c7832470…` + `smoke-bootstrap` 通过；另含真实子进程 `kill -9` 两例与手工进程级演示；
  证据见 [implementation](requirements/web-mvp/implementation.md) §T10 与 `artifacts/web-mvp/t10-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T10）。
- 背景与证据：PRD 修订 2 REQ-024（主）/REQ-025/REQ-026、AC-035/AC-036/AC-037（执行器侧）/AC-038（执行器侧）；
  contracts §2/§5；architecture §6；validation-release §2/§3；ADR-006（拒绝未知结果盲重购）、ADR-012（触发器不变量）；
  既有实现：`migrations/0001–0004`、`repo::*`、`config::Concurrency`、T02 的 data-dir 排他锁。
- 结论与原因：
  1. **阶段依赖边落库**（迁移 0005 新增 `job_stage_deps`）：合同的解锁条件是"依赖阶段全部 succeeded"，
     领取必须与状态更新在同一语句快照内判定；把边持久化后，领取 SQL 用 `NOT EXISTS(... dep.status <> 'succeeded')`
     即可，**不靠内存循环**，崩溃重启后判定依旧成立。DAG 规则本身放在 `manual_core::jobs::stage_dependency`
     （`Fixed([...])` / `AllBatchesOf(manual_extract)`），仓储插入阶段时物化边。
     代价：`manual_merge` 必须在批次之后插入（T11/T15 建单顺序约束，§T10-10）。
  2. **领取用 `BEGIN IMMEDIATE` + 条件更新**：容量谓词（远端生成 2 / 说明书批次 2）内联在候选 SELECT 中，
     与 `lease_epoch + 1` 的 UPDATE 处在同一写锁事务里；两个 worker 同时领取同一行时只有一个 0 行失败重试。
     不用"先查再更"的宽松模式，也不做跨请求长事务（HTTPS 期间不持有事务）。
  3. **"事实"与"决定"分开**：attempt 的 intent/submitting/receipt 与阶段 `result_asset_id/usage_json`
     是**不可变事实**，写入不带租约 guard（合同：过期 worker 可保存事实）；`status` 是**决定**，
     必须带 `status='running' AND lease_owner AND lease_epoch AND lease_until > now` 的 guard。
     过期 worker 的推进被拒后**不重试推进**，由恢复矩阵收敛（否则会与接管者互相覆盖）。
  4. **付费 POST 的错误语义按"能否证明未被接受"分类**：429（含 `Retry-After`）→ attempt `failed` + 可重试；
     5xx／网络中断／超时／200 但缺 task_id → attempt `unknown` + `submission_unknown`（**不自动重购**）。
     理由：合同明文"网络中断、含糊 5xx、超时均不能证明（未被接受）"。AC-036 的"429/5xx → retry_wait"
     由**非付费阶段**承担（本卡用 `tripo_poll` fixture 阶段验证退避序列与 `Retry-After`），不混用两种语义。
  5. **`Retry-After` 尊重 + 截断**：≤300s 原样采用（不加 jitter），>300s 截断到 300s 并把"已截断"写进
     阶段 `last_error`（避免看起来忽略了头部）。退避基数 2/4/8/16/32s 加性 jitter（≤25%），
     第 6 次失败转 `failed`。
  6. **父 job 聚合优先级**：终态保持 → `submission_unknown` → `needs_input` → `failed` → 全 succeeded →
     `waiting_provider` → `running` → `queued`；"已有阶段完成、后续排队"也算 `running`（任务已在推进），
     只有从未开始的 job 是 `queued`。`failed` 不阻止独立分支继续跑完（只影响展示状态）。
  7. **恢复先接管再推进**：恢复扫描对"`running` 且 `lease_until <= now`"的阶段执行 `lease_epoch + 1`
     的接管；接管失败的实例说明别的 worker 已收敛，直接跳过。判定表见 `jobs::recover::plan`：
     intent → 重领；submitting 无远端事实 → unknown；有 task ID → 补推进（下游按 ID 查询）；
     结果已持久化 → 校验后补推进（不重新付费）；同步链路的 `response_id` 不假定可轮询。
  8. **轮询计数持久化**（`job_stages.poll_count`）：3→6→12→15s 的节奏重启后要接着走，
     不能每次重启又从 3 秒开始；同一列也让 UI 能显示"查询第 n 次"。
  9. **failpoint 只存在于测试构建**：Cargo feature `job-failpoints` 仅由 `[dev-dependencies]` 自引用开启，
     宏在未启用时展开为空语句（参数不参与展开，断点名不进二进制，strings/nm 证据见 §T10-7）；
     注册表按 **worker owner** 分区，使并行测试互不干扰（`clear()` 会误伤同进程其他执行器，故测试用
     `clear_owner`）。生产启动路径**没有** failpoint 分支，也没有"开发用 crash 开关"。
  10. **`serve` 用空注册表启动执行器**：本卡没有真实适配器，注册空表并记录 `job_executor_no_handlers`；
      已入队阶段被**延后**（`Defer`：`queued` + `next_run_at`，不消耗重试额度、不写 attempt），
      不假成功、不把"未接入"记成阶段失败。退出时先停 HTTP → 停执行器（停止领取 + 宽限 5s）→ 关库 → 释放锁。
  11. **`[jobs]` 配置**：`lease_seconds`/`renew_seconds` 默认 120/20，硬约束 `1 <= renew < lease`
      （否则租约在续约前过期），`lease <= 600s`（过大让崩溃恢复长时间不生效）；并发上限沿用
      `[concurrency]`（只可降低）。
  12. **取消的阶段级规则**（仓储原语，HTTP 端点在 T15）：`queued`/`retry_wait`/`needs_input` 直接取消；
      `running` 仅当没有 accepted attempt 时取消；`waiting_provider` 与 `submission_unknown` 保留
      （供应商侧可能已计费，收尾/对账属 T15）；取消后执行器不再领取该 job 的任何阶段
      （用例 `cancel_marks_only_unsubmitted_stages_and_keeps_submitted_or_unknown`）。
- 影响的 REQ／任务／模块：REQ-024（主）、REQ-025/REQ-026 的执行器侧；T11（建单时按 DAG 顺序插入阶段、
  预留与 attempt 同事务、unknown 保留预留）、T12（真实 Tripo 适配器复用 `SubmissionWindow` 与
  `latest_accepted_for_job`）、T13/T14（下载/同步批次复用结果事实与恢复规则）、T15（reconcile/retry/cancel
  端点、草稿组装、已提交阶段收尾）、T17（任务中心展示阶段明细与 needs_input 缺项）、T21（包含本卡断点的
  五个崩溃断点矩阵）。
- 未验证边界与下一步：真实 Provider 的字节协议与错误码映射（T12/T14）；费用预留/结算与 unknown 保留
  （T11）；`cost_ledger` 与 attempt 的联动；多实例并发（不在 MVP）；`manual_merge` 边的追加时机（T15）；
  `If-Match`/revision 在任务推进期间的 412 语义（T15 定义）。
- 相关链接：`crates/core/src/jobs.rs`、`crates/server/src/jobs/**`、`crates/server/src/storage/repo/{jobs,job_stages,attempts,audit}.rs`、
  `migrations/0005_job_execution.sql`、`crates/server/tests/jobs_recovery.rs`、
  [contracts §5](contracts.md#5-任务阶段与崩溃语义)、[implementation §T10](requirements/web-mvp/implementation.md)、
  `artifacts/web-mvp/t10-rd/`。

## ADR-021 — T11 报价／确认／冻结／预留／幂等建单落地取舍

- 状态：verified（2026-09-12，T11 实测：`cargo test -p everything-manual --test generation_requests` **23 通过**、
  `cargo test --workspace` **291 通过**、`cargo xtask check` 7/7、`cargo xtask contracts --check` 无漂移、
  `cargo xtask dist` 两次哈希一致 `3b02e24a…` + `smoke-bootstrap` 通过、真实二进制手工冒烟全链路通过
  （`artifacts/web-mvp/t11-rd/smoke-manual.log`）；证据见
  [implementation §T11](requirements/web-mvp/implementation.md)）。
- 日期／作者角色：2026-09-12 · RD（T11）。
- 背景与证据：PRD 修订 2 §3 REQ-020–023、§4 AC-028–034、§5.2/§5.3；contracts.md §1/§2/§3/§4；
  architecture §5 第 3 步、§6；ADR-006（拒绝未知结果盲重购）、ADR-012（时间/枚举表示、触发器不变量、仓储取
  `&mut SqliteConnection`）、ADR-020（T10 failpoint 按 owner 分区）；T07 QA 前置约束（快照必须存 `photo_ids+hashes`）；
  T10 遗留"费用预留/结算与 unknown 保留（T11）"。
- 结论与原因：
  1. **报价落库为 `quotes` 表（迁移 0006），载荷与响应同源**：`POST /jobs` 的输入只有 `quoteId`，
     服务端必须能"不接受前端费用数值"地回读报价（金额、发送范围、到期时间、输入指纹）。
     `quote_json` 是 `QuoteDto` 的序列化结果，响应与落库**逐字节同一份**（用例断言）；报价内容
     由触发器冻结，id 在插入前生成（否则无法把 id 写进载荷）。
  2. **确认是一个显式端点**（`POST /items/{id}/estimates/{quoteId}/confirm`）而不是 `jobs` 的一个布尔字段：
     REQ-021 要求"确认动作写入 `audit_events`"且"未确认的提交被拒"，一个可审计的显式动作比"提交时顺带确认"
     更接近合同语义（确认范围与审计同事务）；`confirmed_at`/`confirmation_json` 只允许 NULL→值
     （重复确认幂等返回首次时间）。**不存在任何隐式确认路径**（不默认勾选由 API 语义保证，前端属 T16）。
  3. **一份报价只能建一份任务**（`quotes.consumed_at` 条件更新）：与"重生成总是新快照 + 新预算确认"一致，
     防止"同一报价配不同幂等键"绕过幂等产生第二份生成单。已消费再提交返回 422 `quoteAlreadyUsed` +
     `details.jobId`（UI 可链接已有任务）。
  4. **输入指纹 = 物品(revision/name/model) + preparation(原件 sha/页数) + 照片(photoId+view+sha256，槽位顺序)
     + 预设/供应商参数/prompt 版本/价格版本**的规范化 JSON 的 sha256，报价与建单各自重算。物品 revision 入指纹
     是**有意从严**：报价确认页展示物品身份，任何编辑都要求重新报价确认（代价是改名也会失效，10 分钟 TTL 下可接受）。
  5. **幂等键作用域 `admin+POST+路由模板+key`，body_hash 由校验后的规范结构计算**；重复点击/断连/重启由
     `idempotency_records` 唯一键兜底。**"报价已被同键请求消费"的竞争窗口回退为重放**（否则并发同键会
     错误地返回 `quoteAlreadyUsed`）——这是本卡修掉的实现期缺陷（用例 `concurrent_submissions_...`）。
  6. **价格目录是文件（`price_catalog_path`），启动即解析、非法即失败（退出码 3）**；`version` 随报价/快照/账本
     冻结，运营者改价必须同时改 `version`（否则旧报价继续按旧价提交）；提交时版本不一致 → 422 `priceVersionChanged`。
     目录价格与实际请求模型必须一致（`providers.tripo.model` == 预设 model、manual_ai model 必须有单价），
     否则 409 `PRICE_CATALOG_MISSING`——不让"目录价"与"实际请求"各说各话。
  7. **预留金额 = 服务端计算的保守上界**（不是用户授权的上限）：用户 `limits` 只做门禁（必须 ≥ 上界，否则 422
     `budgetBelowPlannedUpperBound`），账本与快照只记服务端数值（用例：宽松上限下 `reserved` 仍 = 3000 creditMinor）。
     请求体**没有**任何费用字段（未知字段 422）——"不接受前端传入的费用数值"是结构性的，不靠校验。
  8. **账本路径分离**：`release_definitely_not_billed` 只放行 `reserved`（重复释放幂等）；
     `unknown` 的释放必须走 `release_after_reconciliation`（T15 的 `recordNoTask` 等）——把"unknown 保留预留、
     不自动释放、不填 0"钉在服务层函数边界上（用例断言自动路径对 unknown 返回 `Rejected`）。
     账本状态机（`next_ledger_state`）是 core 纯函数；0006 的 `cost_ledger_active_reservation`
     部分唯一索引保证"同一快照+供应商同时只有一笔 reserved"。
  9. **说明书 AI 上界是本项目声明的保守假设**（每批 ≤5 页、批次开销 1200 token、文字 1 token/字节、
     无文字页页图 3000 token/张、输出 = 批次数 × 4096），预计口径另算；上界**只高不低**（分项之和 = 上界）。
     真实 token 计量属 T23；T14 改发送策略必须同步 core 的常量与用例（否则报价上界不再可信）。
  10. **建单事务一次性写入**：快照 + 2 笔预留 + job + 阶段 DAG（含依赖边，`manual_merge` 在批次之后插入）+
     幂等记录 + 审计，提交前有测试断点（owner = 幂等键，沿用 T10 的 owner 分区以避免并行测试互相命中；
     生产二进制无该断点，`strings`/`nm` 证据见 implementation §T11-6 第 8 条）。
  11. **新增 409 错误码**（core `ApiErrorCode`）：`PROVIDER_NOT_CONFIGURED`、`PRICE_CATALOG_MISSING`、
     `IDEMPOTENCY_CONFLICT`（PRD §8.1 A-13 与 contracts §4 的 409 语义）；其余业务冲突沿 T07/T09 惯例用
     422 + `details.reason`（`quoteExpired`/`quoteAlreadyUsed`/`confirmationRequired`/`inputChanged`/
     `priceVersionChanged`/`budgetBelowPlannedUpperBound`/`modelPresetUnsupported`/`preconditionsFailed`/`pageSetUnusable`）。
- 影响的 REQ／任务／模块：REQ-020/021/022/023（主）、REQ-017（快照结构性前提）；T12/T14（必须按快照的
  `provider_config` 与预算上界发请求；付费 attempt 与账本联动：`attach_attempt` + 结算/unknown）、
  T15（`reconcile`/`retry` 使用 `release_after_reconciliation`；`cost_ledger_active_reservation` 约束）、
  T16（确认页数据来自 `sendScope` + `budgetNotice`；提交按钮复用幂等键）、T17（任务详情展示 `reservations`）、
  T22/T23（发布包不含断点；真实费用与上界偏差记录）。
- 未验证边界与下一步：真实 Provider 的计费字段与上界偏差（T12/T14/T23）；`GET /jobs` 与费用页（T15/T17）；
  报价过期清理策略未做（保留供审计）；价格目录热更新未实现（重启生效）；两条新增路由待协调者在
  contracts §3 补记（`GET estimates/{quoteId}`、`POST estimates/{quoteId}/confirm`）。
- 相关链接：`migrations/0006_generation_requests.sql`、`crates/core/src/{cost,generation}.rs`、
  `crates/server/src/generation/**`、`crates/server/src/http/{estimates,jobs}.rs`、`crates/server/src/http/dto/generation.rs`、
  `crates/server/src/storage/repo/{quotes,snapshots,ledger,idempotency}.rs`、`crates/server/tests/generation_requests.rs`、
  `price-catalog.example.toml`、[contracts §4](contracts.md#4-输入快照与费用合同)、
  [implementation §T11](requirements/web-mvp/implementation.md)、`artifacts/web-mvp/t11-rd/`。

### T11 验收知识（追加，2026-09-12；QA 回合 12 结果 FAIL，1 个 OPEN 缺陷 BUG-004）

- 状态：回合 12 已执行；AC-028–AC-034 的 API 语义与卡内项独立通过。**BUG-004（P2，回读路由状态陈旧）已由 RD 修复
  （2026-09-12，`http/estimates.rs::get_estimate` 合并三列），等待 QA 复验（回合 13）**（是否 CLOSED 由 QA 判定）。
  证据：`artifacts/web-mvp/t11-qa/` + `llmdoc/requirements/web-mvp/qa-report.md` 回合 12 节。
- 结论/手法（跨卡复用）：
  1. **报价上界必须能按"价格目录 + 页输入"独立复算**：说明书 AI 上界口径 = 批次开销 1200 token/批 + 页文字 1 token/字节 +
     无文字层页 3000 token/张（页图），输出 = 批次数 × 4096，全部 Ceil 后按单价换算（QA 两次手算：20367 = 2175+8192+10000；
     真实二进制 19742 = 1550+8192+10000，与响应逐项一致）。T14 改发送策略必须同步 `crates/core/src/generation.rs` 常量并重验；
     评审报价用例时**先自己算一遍**，只对照实现常量会漏掉口径错误。
  2. **回读/回显类合同必须"先变更状态再读"**：只断言创建时的 null 会让"永远返回冻结载荷"的缺陷（BUG-004）看起来通过。
     `GET estimates/{quoteId}` 的合同（OpenAPI description）明说含确认/消费状态，但 handler 只解析 `quote_json`（创建时冻结），
     `confirmed_at/consumed_at/consumed_job_id` 三列从不合并——修复方向：读回后用列覆盖 DTO 字段（`quote_json` 仍冻结）。
     受影响前端语义：UI-024「已确认发送范围（时间）」、UI-026「链接已有任务」。
  3. **同资源数组顺序可能随代码路径变化**：预留数组首建按写入顺序 `[tripo, manual_ai]`、重放按 provider 排序
     `[manual_ai, tripo]`（内容/金额相同）。跨路径比较用集合/排序后比较，避免把顺序差异误判为重复写入；UI 渲染建议统一顺序。
  4. **测试设计陷阱**：请求体被 422 拒绝**不会**消费报价，随后"换新键重提同一报价"会合法 202；写"重生成必须新报价"用例
     必须先有一次成功建单再重提（QA 本轮自触后修正）。
  5. **failpoint 门控核对（沿用正对照法）**：dist `strings`/`nm` 对新断点 `generation_after_reserve_before_commit` 全 0，
     测试二进制命中 2/`EM_TEST_FAILPOINT` 1/`nm` 133，证明检查非空转。
  6. **迁移链合成手法复用（v5 → v6）**：全新 `init` 库 6 行 `_sqlx_migrations` 的 checksum = 对应迁移文件 SHA-384 大写 hex（6/6 反证）；
     合成 v5 库后 `check` 只读报"待迁移（库 v5 → 程序 v6）"（前后行数不变），`serve` 升级出 quotes 表 + 2 索引 + 3 触发器 +
     `cost_ledger_active_reservation` 部分唯一索引，`integrity_check`/`foreign_key_check` 干净。脚本骨架可复用于 T13/T15/T20。
  7. **配置类启动失败口径**：非法价格目录 `check`/`serve` 均 **exit 3** 且指出具体键（`unknown field credits…`）；
     正常 `check` 打印 `已就绪（v6；WAL/synchronous/foreign_keys/busy_timeout…）`。
  8. **"不产生远端请求"是结构性的**：生产依赖树无 reqwest/ureq/curl/rustls/native-tls，`hyper-util` 仅 server 特性；
     真实二进制冒烟再用 `lsof` 断言无非 loopback 连接（比只看 `provider_attempts=0` 更强）。
  9. **含中文字符的 shell 脚本必须写 `${VAR}`**：`$VAR（` 会被解析成变量名的一部分，`set -u` 下直接 unbound variable 中断
     （QA 本轮踩过两次）；清理 trap 中 `wait` 也要写 `${SERVER_PID:-}`。
- RD 修复侧补充（2026-09-12，BUG-004，详见 `implementation.md` §T11-10）：**状态字段的唯一事实源是持久层当前列，
  冻结载荷只承载不可变输入**——`quote_json` 是"创建时"快照，回读任何路由时状态类字段（确认/消费）必须由
  `quotes` 表的当前列覆盖后再出网，且不得回写 `quote_json`；"是否仍可用"不在回读里表达，仍由 confirm/提交时的
  服务端校验判定（过期报价可读不可用）。BUG-004 已在 `http/estimates.rs::get_estimate` 合并三列修复，
  QA 复现用例单跑已通过；`quote_payload` 保持"纯解析冻结载荷"语义并加注释指向合并点。
- 相关代码／测试：`crates/server/tests/qa_t11_independent.rs`（13 用例，含 1 条 `#[ignore]` 复现 BUG-004）、
  `artifacts/web-mvp/t11-qa/qa-smoke-t11.sh`、`artifacts/web-mvp/t11-qa/qa-t11-migration-chain.sh`；
  BUG-004 修复落点：`crates/server/src/http/estimates.rs::get_estimate`；回归用例：
  `crates/server/tests/generation_requests.rs::{get_estimate_reflects_confirmation_and_consumption_from_database,
  get_expired_quote_reports_status_facts_but_stays_unusable}`；手工证据 `artifacts/web-mvp/t11-rd-bug004/`。

### T11 验收知识（QA 回合 13 补充；BUG-004 复验 **CLOSED**，T11 切片 PASS）

- 状态：2026-09-12 回合 13 已执行。四状态（未确认未消费/已确认/已消费/已过期）× 真实二进制 + `sqlite3` 交叉核对全部通过；
  回归 305 passed、`xtask check` 7/7、`contracts --check` 两份 `[一致]`、QA 独立重建 dist 哈希 = RD 值
  （`324b2fbe…`）。证据：`artifacts/web-mvp/t11-qa-r13/`（脚本 `qa-bug004-four-states.sh`）、`qa-report.md` 回合 13 节。
- 结论/手法（跨卡复用）：
  1. **`time` crate RFC3339 小数位规则**：`Timestamp::to_rfc3339()` 在小数部分为 0 时**整体省略小数**，否则打印
     **去掉尾随 0** 的最短位数（`…:50.520Z` → `…:50.52Z`；`…:50.000Z` → `…:50Z`）。凡"DB 毫秒列 → 字符串"复算与响应
     逐字比对（回读类断言）必须实现同一规则，否则尾数为 0 时误报缺陷；实现见
     `artifacts/web-mvp/t11-qa-r13/qa-bug004-four-states.sh` 的 `rfc3339()`。
  2. **DB 合成"过期报价"夹具必须同步 `quote_json.expiresAt` 与 `quotes.expires_at`**：真实路径由服务层同源写入，
     只改列不改载荷会让"回读取载荷值"看起来像缺陷（本轮夹具自伤一次后修正）。HTTP 层无法产生过期报价
     （服务端时钟 + 0006 触发器冻结 `expires_at`），所以该合成手法是真实二进制上验证过期语义的标准做法。
  3. **回读路由的验收序列**：变更状态（确认/消费）→ 回读 → 与 `sqlite3` 列交叉核对（不是只看响应）；同时断言
     冻结输入（金额/上界/价格版本/expiresAt/sendScope）与创建响应逐值相等、`quote_json` sha256 不变。
  4. **修复复验的最小充分集**：缺陷复现用例转绿 → 读修复点代码 → 四状态 × DB 交叉核对（真实二进制）→ 全量回归 +
     合同漂移检查 + 独立重建 dist 比对哈希；不采信 RD 日志本身。
  5. **`$VAR（` 陷阱复现**：含中文字符的 shell 脚本必须写 `${VAR}`（本轮再次踩到，与回合 12 知识 9 相同）。
- 未关闭的观察（非缺陷）：GET 的 `expiresAt` 来自冻结载荷 `quote_json` 而非列（正常路径同源写入 + 触发器禁止改列，
  当前无发散路径）；若后续（如 T15 重试/替换）出现"只更新列/只更新载荷"的路径，需复核该不变量。
- 相关代码／测试：`crates/server/src/http/estimates.rs::get_estimate`（合并三列）、
  `artifacts/web-mvp/t11-qa-r13/qa-bug004-four-states.sh`、`crates/server/tests/qa_t11_independent.rs`（未改动，
  sha256 `87653b7d…`）。

### T12 验收知识（追加，2026-09-12；QA 回合 14 结果 PASS，无新增缺陷）

- 状态：2026-09-12 回合 14 已执行。AC-041 全部子句经 QA 自建 Python fixture（与 RD fixture 相互独立）+
  真实 dist 二进制端到端复现（10 场景全部通过）；`tripo_contract` 18 passed；workspace 346 passed / 0 failed /
  3 ignored；`xtask check` 7/7；`contracts --check` 两份 `[一致]`；QA 独立重建 dist 哈希 = RD 值（`2ec8a45b…`）。
  证据：`artifacts/web-mvp/t12-qa/`（`qa-t12-run-all.sh` + `qa-tripo-fixture.py` + `qa-t12-verify.py` + 各场景日志）、
  `crates/server/tests/qa_t12_independent.rs`、`qa-report.md` 回合 14 节。
- 结论/手法（跨卡复用）：
  1. **付费 POST"不自动重发"的完整守卫链（HTTP 栈层）**：应用层无重试中间件（`providers/tripo/client.rs` 只配
     `connect_timeout`/`timeout`/`Policy::none()`）；**reqwest 0.13 默认带一个 tower 重试层**
     （`retry::Builder::default()` = `Classifier::ProtocolNacks`，`max_retries_per_request=2`、Unscoped、无预算），
     但其唯一判定 `is_retryable_error()` 的真值分支全在 `http2`/`http3` feature 内——本项目
     `default-features=false`（仅 json/multipart/stream/rustls）下**该层恒不重试**（旁证：`h2` 不在 Cargo.lock、
     `hyper` 只启 `http1`、dist 二进制无 h2/`REFUSED_STREAM` 符号）。hyper-util 的
     `retry_canceled_requests`（默认 true）只在"请求未写入连接"（`try_send_request` 返回 message，
     仅 reused 连接）时重发一次，不会复制一条服务器已收到的请求。**若将来任何人给 reqwest 打开 `http2`
     feature，该默认策略会重试 h2 `GOAWAY(NO_ERROR)`/`REFUSED_STREAM`**——建议显式
     `ClientBuilder::retry(reqwest::retry::never())` 并把"付费 client 不得启用 http2/http3 默认重试"写进代码注释。
  2. **`take_message()` 语义是"未序列化到连接"的判据**（hyper 1.x 文档与实现）：`TrySendError` 带 message
     ⇔ 请求字节从未写出。用它区分"可安全重发的未发出请求"与"可能已被接受的未知结果"。
  3. **QA 自建 fixture 的场景清单可直接复用**（`qa-tripo-fixture.py`）：`happy / disconnect / unknown /
     business_error(200+code!=0) / no_model / token_unknown(非候选字段) / submit_429 / poll_503 / no_billing /
     token_verbatim(含空白 token 逐字透传)`；每个场景用真实二进制 + curl 建单 + `sqlite3` 交叉核对阶段/attempt/账本。
     T13/T23 前可直接复用（T23 只需把 base_url 换成真实地址并加预算文件）。
  4. **业务错误 `HTTP 200 + code!=0` 与"明确拒绝"同族**：进程内 `TripoError::Business`（含 4xx 与 200/code!=0）
     都归 `is_definitively_refused()=true` → attempt `failed` + `release_definitely_not_billed`；传输/超时/断连/
     含糊 5xx/缺 task_id → `unknown` + 保留预留（`actual` 不填 0）。验证手法：fixture 计数（付费 POST 恰好 1 次）+
     `provider_attempts.submit_state` + `cost_ledger.state/actual` 三处交叉，而不是只看阶段状态。
  5. **"success 缺模型"与"success 无计费字段"要分开断言**：前者 → `retry_wait`（保留 task_id、不重购），
     后者 → 阶段照常 `succeeded` 但**不结算**（`cost_ledger` 保持 `reserved`、`actual IS NULL`）——两种
     "容错"方向相反，必须分别用 fixture 场景锁定（本轮 `no_model` / `no_billing`）。
  6. **`sqlite3` 直改 `job_stages.status='queued'` 是复验"付费不重发"的好手法**（模拟恢复/重试入口的最终防线）：
     改库后 + `kill -9` 重启同一 data-dir，付费 POST 计数仍为 1（attempt 未决事实拦住处理器）。
  7. **lsof 采样要写 `-a`**（`lsof -p PID -a -i -P -n`），否则 `-p` 与 `-i` 是 OR 关系、抓不到"该进程的网络连接"；
     采样期间只为该进程追加输出即可断言"非 loopback = 0"。
- 未关闭的观察（非缺陷，供 T23/后续卡）：
  1. 三处候选字段名（上传 token `file_token`/`image_token`、计费 `credits_consumed`/`credits`）与多视图 `inputs`
     扁平形态均以项目内 2026-09-11 冻结核对记录为准（官方站 2026-09-12 本机不可达）；T23 需逐项核对并在报告中
     记录实际字段名。容错方向经本轮验证是"安全方向"：认不出候选字段 → 可见失败（上传 `retry_wait`、不产生付费
     POST）；无计费字段 → 不结算（不会把真实金额当 0，也不会伪造成功）。
  2. 相同内容用于两个视图（front/left 同 sha256）时，上传按内容哈希缓存导致提交体出现**同一 token 两次**；
     当前不可达（T11 禁止重复 photoId，但允许两个 photo 行指向同一 asset）。低风险：供应商会以业务错误拒绝
     （可见失败、单次付费），T23 若遇到再决定是否需要 `needs_input`。
  3. `qa_t10_independent.rs` / `qa_t11_independent.rs` 中 BUG-003/BUG-004 的复现用例 `#[ignore]` 标记已过时
     （两缺陷已 CLOSED）：本轮用 `--ignored` 单跑，**两条均通过**（exit 0）。建议后续批次去掉 ignore 转为常驻回归
     守卫；本轮为保持既有哈希证据链未改动文件。
- 相关代码／测试：`crates/server/src/providers/tripo/**`、`crates/server/tests/tripo_contract.rs`、
  `crates/server/tests/qa_t12_independent.rs`、`artifacts/web-mvp/t12-qa/**`。

### T13 验收知识（追加，2026-09-12；QA 回合 15 结果 PASS，无新增缺陷，3 条 P3 观察）

- 状态：2026-09-12 回合 15 已执行。AC-043/AC-044 全部子句经 **QA 自写 15 条集成用例**
  （`crates/server/tests/qa_t13_independent.rs`：GLB 负例字节手写、原始 TCP fixture 自建）+
  **QA 自写发布门禁脚本**（`artifacts/web-mvp/t13-qa/qa-t13-release-gate.py`，25/25 项）复现；
  `model_assets` 22 passed；`tripo_contract` 18 + `qa_t12_independent` 3 保持；`xtask check` 7/7；
  workspace 391 passed / 0 failed / 3 ignored；`contracts --check` 两份 `[一致]`；dist 哈希 = RD 值（`24264baf…`）。
  证据：`artifacts/web-mvp/t13-qa/**`、`qa-report.md` 回合 15 节。
- 结论/手法（跨卡复用）：
  1. **"连接 pin 到已校验 IP + 保留 hostname/SNI"的可观测证明**：给允许域注入"只在解析器里存在"的映射
     （解析到回环），URL 必须用**域名**而不是 IP 字面量。若实现未 pin（连接前再次系统解析），`.test`
     保留域不可能解析成功 → 连接失败；"成功 + fixture 收到 `Host: <域名>:<port>`"同时证明 pin 生效与
     hostname 保留。T23 复核真实域名时沿用（真实 DNS 下断言 Host/SNI）。
  2. **"第 N 跳 0 连接"要用独立连接计数**：请求计数区分不了"连上但没发请求"与"根本没连"；QA fixture
     把 `connections` 与 `requests` 分开计数。重定向上限的判别语柄是"恰好 6 个请求"（max_redirects=5）。
  3. **两道大小门的判别语柄**：`DownloadError::TooLarge.declared`（`Some`=声明长度门、`None`=流中计数门）；
     第二道门要用 `transfer-encoding: chunked`（无 Content-Length）才触发；两者都必须断言 0 blob / 0 `.part`。
  4. **发布门禁必须用发布产物 + 故意误配**：只跑测试构建的单测证明不了"发布构建不放行本机 fixture"；
     用 dist 二进制 + `allow_local_fixture=true` + 真任务走链，三段证据（fixture 0 次模型请求 / 0 资产与 0
     revision / 付费仍 1 次 + 日志 `download_local_fixture_ignored`）。
  5. **全库 canary 扫描法（本轮新增，跨卡复用价值最高）**：遍历 `sqlite_master` 的表与列，执行
     `SELECT COUNT(*) … WHERE instr(CAST({col} AS TEXT), ?) > 0`，能发现"某个没被想到的表/列里躺了临时 URL
     或密钥"。正是它把"临时签名 URL 只允许出现在 `tripo_poll` 观察事实"这一事实钉死（T13 自有位置 0 命中）。
  6. **sqlx 0.9 的动态 SQL**：`query(&format!(…))` 被拒绝；列清单用 `SELECT name FROM pragma_table_info(?)`
     （带 bind 的静态 SQL），行扫描用 `sqlx::AssertSqlSafe(String)`。
  7. **schema 观测入口是 `_sqlx_migrations`**（不是 `schema_migrations`）；T13 未新增迁移，
     `model_revisions` 来自 0001，迁移集 0001–0006 全部 success。
  8. **GLB 能力清单的一处未文档化边界（P3-2）**：`HEADER_WINDOW = 64 KiB` 决定"JPEG 的 SOF 必须在首
     64 KiB 内"；SOF 偏移 66541 的合法 JPEG 被判 `gltf_image_invalid`（fail-closed 但属误拒）。同类
     "只读头部判尺寸"的实现要注意该窗口与真实产物的 EXIF/ICC 段大小。
- 未关闭的观察（非缺陷，供 T15/T17/T20/T23 与 PM）：
  1. **P3-1**：临时签名 URL 长期保存在 `tripo_poll.usage_json`（T12 设计，供下载阶段取用）；T13 自有位置 0 命中。
     出网脱敏属 T15/T17、导出不含临时 URL 属 T20；是否缩短保留窗口需 PM 决定。
  2. **P3-2**：见结论 8；若真实产物带大段元数据需扩窗口或流式扫 marker。
  3. **P3-3**：`model_revisions` 无 `(item_id, sha256)` UNIQUE（应用层幂等，跨 job 并发理论上可双行；
     本回合未构造复现）。当前 UNIQUE 只有 `cost_ledger_active_reservation`、`photos_item_view_unique`、
     `provider_attempts_unresolved_stage`。
  4. 回合 14 遗留：`qa_t10_independent.rs` / `qa_t11_independent.rs` 的过时 `#[ignore]` 仍在（本轮 3 ignored
     计数不变），建议后续批次转正。
- 相关代码／测试：`crates/server/src/assets/glb/{mod.rs,download.rs}`、`crates/server/src/providers/tripo/handlers.rs`、
  `crates/server/tests/qa_t13_independent.rs`、`artifacts/web-mvp/t13-qa/**`。

## ADR-022 — T12 Tripo v3 适配器落地取舍（无重试层的错误分类、计费容错、上传内容哈希缓存）

- 状态：verified（2026-09-12，T12 实测：`cargo test -p everything-manual --test tripo_contract` **18 通过**、
  `cargo test --workspace` **343 通过 / 0 失败 / 3 ignored**、`cargo xtask check` 7/7、`contracts --check` 无漂移、
  `cargo xtask dist` 两次哈希一致 `2ec8a45b…`（20 880 880 B）+ `smoke-bootstrap` 通过、真实 dist 二进制 +
  本机 python Tripo fixture 的手工端到端冒烟（上传 2 次 → 付费提交 1 次 → 查询 running→success →
  状态/计费落库、Tripo 预留 `settled/actual=3000`、lsof 采样 0 非 loopback）；证据见
  [implementation §T12](requirements/web-mvp/implementation.md) 与 `artifacts/web-mvp/t12-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T12）。
- 背景与证据：PRD 修订 2 REQ-027（主）/AC-041、AC-042（T23）；contracts.md §6（Tripo 合同）、§5（提交窗口与
  "未返回 ID 禁止通用自动重试"）、§1（creditMinor）；architecture §3（reqwest 0.13 + rustls）、§5.3（v3 基线、
  banned/expired、产物过期 ≠ 链接过期）、§7（下载不复用带 Authorization 的 client）；validation-release §2/§3；
  ADR-004/006/020/021；T05 fixture、T10 提交窗口/恢复、T11 快照/报价/预留。
  **官方文档可达性**：`developers.tripo3d.ai` 于 2026-09-12 在本机不可达（curl 20s 超时、WebFetch 被拒），
  字段形态依据 2026-09-11 的项目内冻结核对记录（contracts §6 / architecture §5.3 / 旧方案 §6.2 与 T05 样例）。
- 结论与原因：
  1. **错误分类决定"能否自动再提交"，分类函数是纯数据（`TripoError::is_definitively_refused`）**：
     429 / 非 429 的 4xx / 业务 `code!=0` / 3xx = 供应商明确拒绝（无 task ID 产生）→ attempt `failed` +
     `Failed`（或 429 的 `Retryable`）；传输失败/超时/断连/含糊 5xx / `code==0` 但缺 `task_id` =
     **不能证明未被接受** → attempt `unknown` + `SubmissionUnknown`。客户端**不挂任何重试层**，
     付费 POST 一次调用只发一次（T05 fixture 的计数断言 + 恢复路径复测三重证据）。
  2. **429 保留预留、明确拒绝才释放**：429 会触发自动退避重试，释放会让重试失去预算背书且
     `cost_ledger_active_reservation` 部分唯一索引不允许无谓的第二笔；明确拒绝（确定性失败、分支终止）
     才走 `release_definitely_not_billed`。失败/取消/封禁/过期**不动账本**（供应商是否计费无法由本方证明）。
  3. **计费容错但绝不猜金额**：`credits_consumed` → `credits` 候选字段名 + 数字/字符串字面量两种形态，
     统一走精确 decimal（scale=2, Ceil）；原始字面量、来源字段名、currency 一起落库；
     解析失败只记录 `billingProblem` 且不结算。理由：文档字段名未能在本机复核，但"读到哪个字段"必须可审计，
     "金额怎么算"必须确定（不用浮点、不向下取整）。
  4. **上传 token 的两个候选字段名（`file_token` → `image_token`）与来源字段落库**：两份项目内文档记录
     不一致，容错读取不改变语义，T23 实测后收敛为一个。
  5. **上传按内容哈希缓存，载体是阶段 `usage_json`（不新增表）**：`{view, sha256, token, tokenField}` 每张图
     上传完立即作为**事实**落库（`set_result_fact` 无租约 guard）；重新执行阶段时按 sha256 跳过已上传内容。
     理由：本卡不允许新增迁移；上传免费且幂等，缓存只是为了少发请求，事实列已经具备"崩溃后仍可复用"的性质。
  6. **发送范围取"用户已确认"的报价 `sendScope.tripo.views`，并与快照 `photo_ids/photo_hashes` 交叉核对**
     （顺序 + 内容一一对应）：REQ-021 要求用户确认"发什么给谁"，快照只冻结 id/hash 不冻结方向；
     任何不一致 → `needs_input`（不猜测发送内容）。生成参数同理只取快照 `provider_config.tripo`
     （camelCase 键），不读当前配置、不用默认值。
  7. **不跟随重定向**（`redirect::Policy::none()`）：避免把 `Authorization` 带到别的地址；3xx 视为配置错误。
     响应体读取有上限（8 MiB）并截断标记；URL 形式的诊断文本经 `redact_url_query`。
  8. **`modelUrl`（含签名查询串）作为阶段事实落库供 T13 下载，但出网必须脱敏**：本卡所有日志用它之前
     先 `redact_url_query`；T15/T17 暴露 `usage_json` 时受同一约束。
  9. **单图 `image-to-model` 分支不做**：首版产品路径是多视图（PRD REQ-027），单图涉及"用单张实物照片／
     AI 造图"的产品与告知问题，应由 PM 决定后另开切片。
- 影响的 REQ／任务／模块：REQ-027（主）、REQ-023/033 的"unknown 保留预留"在 Tripo 侧的落地；
  T13（用 `modelUrl` 下载、独立无 bearer 的 client、链接过期重新查询）、T14（复用"同步链路未知"的语义与
  计费落库形态）、T15（reconcile/retry/`authorizeReplacement`；`Failed` 后重新预留的时机）、
  T17（阶段明细展示 `rawStatus`/`normalizedStatus`/`billingProblem` 且必须脱敏 `modelUrl`）、
  T21/T23（真实字段名、真实 credits 与上界偏差）。
- 未验证边界与下一步：官方响应原文未逐字核对（三处候选字段名待 T23 收敛）；真实 TLS/cert 链、DNS/私网
  拦截属 T13 的下载路径；真实 429/5xx 行为与 `Retry-After` 的实际形态属 T23；`usage_json` 的对外 DTO
  脱敏规则由 T15/T17 落实。
- 相关链接：`crates/server/src/providers/**`、`crates/server/tests/tripo_contract.rs`、
  `tests/fixtures/responses/tripo/`、`artifacts/web-mvp/t12-rd/{smoke-t12-e2e.sh,tripo-fixture.py,smoke-t12-e2e.log}`、
  [contracts §6](contracts.md#6-外部-provider-合同)、[implementation §T12](requirements/web-mvp/implementation.md)。

## ADR-023 — T13 模型下载与 GLB 校验落地取舍（独立无凭据 client、pin IP、两道测试门、rejected 版本）

- 状态：verified（2026-09-12，T13 实测：`cargo test -p everything-manual --test model_assets` **22 通过**、
  `cargo test --workspace` **376 通过 / 0 失败 / 3 ignored**、`cargo xtask check` 全通过、`contracts --check`
  无漂移、`cargo xtask dist` + `smoke-bootstrap` 通过（sha256 `24264baf…`）、**测试构建二进制 + 本机
  fixture 的手工端到端**（链接 403 → 重查同一 task → 新链接 → 下载 → 校验 → `validated` revision；
  付费提交恰 1 次、CDN 请求无 Authorization、签名 URL 未落地为永久地址）与**发布构建门禁核对**
  （误配 `allow_local_fixture` 不生效 → `needs_input` + CDN 0 请求）；证据见
  [implementation §T13](requirements/web-mvp/implementation.md) 与 `artifacts/web-mvp/t13-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T13）。
- 背景与证据：PRD 修订 2 REQ-028（主）/AC-043、AC-044；contracts.md §7（GLB 检查清单、预算、超预算保留
  原始模型、CPU 校验≠GPU 可绘制）、§2（`model_revisions` 不可变、validated 才可进阅读器）、§5（链接过期 ≠
  产物过期）；architecture.md §7（HTTPS/允许域/每跳与 IP 校验/不转发 bearer/关闭自动重定向/pin IP 保留
  SNI/测试本机 fixture 仅测试配置显式允许）；validation-release §2（"fixture URL 准入通过仅测试构建开关 +
  显式配置"）、§3（下载行矩阵）；ADR-007/022；QA 回合 14 的三条 P3 建议（其中"显式 retry::never"本卡落实）。
- 结论与原因：
  1. **下载 client 与 API client 完全分离，且不持有任何凭据**：模型 CDN 只取字节；`Authorization` 只在
     T12 的 `TripoClient` 上逐请求添加。理由是 architecture §7 的明文要求，也避免"签名 URL 被当成受信
     通道"的错误假设（CDN 与 API 不是同一信任域）。
  2. **允许域默认空 = 拒绝一切下载，不内置供应商 CDN 域名**：官方文档在本机不可达（ADR-022 背景），
     凭空写死域名既不可验证也不安全；缺配置时给可行动 `needs_input`（`download_host_not_allowed`）。
     代价：真实链路在 T23 复核域名并写入部署配置前不可用——这是**有意为之的失败可见**，不是静默降级。
  3. **"逐跳校验 + pin 到已验证 IP"用 `ClientBuilder::resolve_to_addrs(host, [ip])` 实现**：连接只可能
     到已验证地址，而 URL 的 hostname 不变（Host 头 / TLS SNI / 证书校验仍按域名），因此不存在
     "预检后再次解析"的重绑定窗口；比自建 connect 层简单且不碰 rustls 配置。解析器抽象
     （`HostResolver`）让 DNS 重绑定在测试里可注入复现。**DNS 返回的全部地址都要通过校验**（混合
     公网/私网答案 → 整次拒绝），避免"取第一个"被投毒列表绕过。
  4. **测试放行本机 fixture = 两道门**：`download.allow_local_fixture = true`（显式测试配置）**且**测试
     构建（复用既有 `job-failpoints` feature 作为"本 crate 唯一的测试构建开关"）。发布构建忽略该键并
     打 `download_local_fixture_ignored` 告警。**只放行回环**：私网/链路本地等即使开了这个键也一律拒绝
     （redirect 到私网的用例因此仍能被拦住）。理由：validation-release §2 要求"仅测试构建开关 + 显式配置"，
     单靠配置键会在生产留下 SSRF 后门。
  5. **超预算与结构问题都进 `needs_input` 并写 `rejected` revision**：`rejected` 语义 = "已下载并保留原始
     模型、校验未通过"（`bounds` 为空、不允许进入草稿/发布）；`(item_id, sha256)` 幂等避免重试产生第二行。
     这样"原始模型与错误都保留"是**可查询的持久事实**，而不是只存在于日志里；同时不违背"校验通过才可进入
     阅读器"（`validated` 才是可用版本）。
  6. **链接过期与产物过期分流**：`401/403/404/410` = 链接过期 → `GET /tasks/{task_id}` **重新查询同一任务**
     取新链接（绝不新建付费任务）；重新查询显示 `expired/banned/failed/cancelled` 且无本地副本 → `failed` +
     "不可自动恢复、需确认新预算后重新生成（不会自动重新购买）"。**本地副本优先**：阶段重跑先看
     `result_asset_id` 指向的 blob 是否仍在，在则不发起任何请求。
  7. **中断重试 = 整文件重下，不用 Range 续传**：模型 ≤150 MiB 且一次性交付；续传需要额外的偏移/哈希拼接
     状态，一旦出错会得到"看起来成功、内容错位"的资产。宁可多下一次，也不引入静默损坏路径（成本由 T23 实测
     后再决定是否优化）。
  8. **`modelUrl` 的存储边界不变**：签名 URL 只作为 T12 的阶段**观察事实**（供本卡取用），
     `model_revisions`/`assets`/下载与校验的 `usage_json` 都不保存它（只保存 host/sha256 等摘要）；
     出网 DTO 脱敏仍按 ADR-022 第 8 条由 T15/T17 落实。
  9. **`retry(reqwest::retry::never())` 同时落在两个 client 上**（下载 + `TripoClient`）：本卡落实 QA 回合 14
     的 P3（原建议落点即 T13/T15）。当前 feature 组合下默认重试层是死代码，但"启用 http2 即改变安全边界"，
     显式关闭可防止未来切换协议栈时改变付费/下载语义。
- 影响的 REQ／任务／模块：REQ-028（主）、REQ-033/035（`validated` 才可被草稿/发布选中；`rejected` 不可用）；
  T15（组装草稿时应取"该 job 的 validated revision"，并处理 `needs_input`/`rejected` 的展示）、
  T17（`needs_input` 缺项代码与"原始模型已保留"文案）、T18（浏览器二次验证：`modelReview.loaded`）、
  T20（导出/备份需包含被 revision 引用的模型 blob）、T23（真实 CDN 域名、真实产物尺寸/贴图特征、
  下载耗时与超时取值）。
- 未验证边界与下一步：真实 `output.model_url` 域名与签名形态、真实 CDN 是否要求特定请求头（如 Referer）、
  真实 150 MiB 级模型的校验耗时、TLS 证书链在部署环境的行为——均属 T23。真实 GPU 可绘制性属 T18。
- 相关代码／测试：`crates/server/src/assets/glb/{mod.rs,download.rs}`、`crates/server/src/providers/tripo/handlers.rs`、
  `crates/server/src/storage/repo/model_revisions.rs`、`crates/server/tests/model_assets.rs`、
  `artifacts/web-mvp/t13-rd/{smoke-t13-e2e.sh,verify-release-gate.sh,model-fixture.py}`；
  [contracts §7](contracts.md#7-资产和发布不变量)、[implementation §T13](requirements/web-mvp/implementation.md)。

## ADR-024 — T14 说明书 AI 适配与证据校验落地取舍（batch 结果资产 purpose、needs_input 语义、BEGIN IMMEDIATE 加固、服务端回填出处）

- 状态：verified（2026-09-12，T14 实测：`cargo test -p everything-manual --test manual_ai_contract`
  **19 通过**、`cargo test --workspace` **431 通过 / 0 失败 / 3 ignored**、`cargo xtask check` 7/7、
  `contracts --check` 无漂移、`cargo xtask dist` 两次哈希一致
  `22a4fc4f5d84036883e2f0db6923257bc5d79088b8e4932916849603f02a7b72` + `smoke-bootstrap` 通过、
  **真实 dist 二进制 + 本机 python fixture 的手工端到端**（7 页 → 2 批提取 → 合并；覆盖率/出处/冲突
  落库；注入样例只作数据；lsof 非 loopback = 0）；证据见
  [implementation §T14](requirements/web-mvp/implementation.md) 与 `artifacts/web-mvp/t14-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T14）。
- 背景与证据：PRD 修订 2 REQ-029（主）/AC-045、AC-046；contracts.md §6（Manual AI 合同）、§5
  （同步 Responses 的"先结果资产、后 usage/receipt/checkpoint"与 `submission_unknown`）、§2
  （Part/Step/Evidence/Review 最小信息）、§1（1-based 页码）；architecture.md §5.2（≤5 页/批、覆盖率、
  聚合去重保留出处、扫描页发页图、不用正则抢救、`text.format` 而非 `response_format`、资料是数据
  不是指令、无工具权限、快照冻结版本）；validation-release.md §3（说明书 AI 行矩阵 + 批次中断场景）；
  ADR-004/005/006/009/020/021；T05 fixture、T09 页资产、T10 执行器与 `manual_extract` batch_index、
  T11 快照/报价/预算/阶段建单。官方文档可达性：本卡按 architecture/contracts 的 2026-09-11 冻结核对
  记录实现（Responses 形态），真实字段与效果留待 T23。
- 结论与原因：
  1. **"不产生正式知识"= `needs_input`（不是 `succeeded`）**：拒答 / `incomplete`（截断）/
     `output_text` 非法 JSON / schema 违规 / 无输出 / 信封不可解析 / 供应商 `status` 非 completed /
     4xx 明确拒绝 —— 这些批次**仍写结果资产与诊断**（含原始响应 blob），但阶段结论是
     `needs_input` + 稳定错误码。理由：① AC-046"全部批次成功且页覆盖完整才解锁 merge"要求
     未产出知识的批次不算成功（否则带空洞的草稿看起来像完整产物）；② 这些批次**可能已计费**，
     自动重试会重复付费（`needs_input` 不自动重试）；③ 人工在 T15 的入口上"只对该批重新授权
     重算"（合同 §5 明文）。
  2. **批次结果资产复用 `page_text` purpose（mime `application/json`）**：合同的"每批独立结果资产"
     需要 `job_stages.result_asset_id` 指向 `assets` 行（恢复矩阵也据此判定"结果已持久化→补推进"），
     而冻结的 `assets.purpose` CHECK 只有 5 个取值；新增取值需要重建 `assets` 表（迁移），
     会连带影响不在本卡写入范围的 v6 schema 断言。因此复用"由页派生的文本内容"这一既有取值
     （T10 测试设施 `seed_result_asset` 同约定），并在代码模块文档注明语义妥协；是否新增专用
     purpose 由 PM/后续卡决定（一次性迁移 + 既有测试同步）。
  3. **模型只提供页号与引文；`documentId`/`preparationId`/`derived`/`bbox` 由服务端决定**：
     要求模型复述 UUID 不可靠且给"编造出处"留口子；服务端从冻结输入回填文档/准备 id、
     按"本页是否以页图发送"打 `derived`、`bbox` 恒为 null（不为了满足 schema 捏造框）。
     schema 里 `quote` 用 `type:["string","null"]` 表达可选值（不省略键，strict 模式要求全 required）。
  4. **两步解析区分 `invalid_format` 与 `schema_violation`**：先 JSON 语法、再 schema 形状；
     两者都不做任何"修复/正则抢救"，但分开报码便于诊断（HTTP 层另有 `envelopeInvalid`）。
  5. **`merge` 用内容派生 id + 保留出处 + 冲突双方保留**：`<kind>-<sha256(内容)[..12]>` 让同内容
     跨批自然去重、同名不同事实自然分成不同 id；冲突写入 `conflicts[]`（`needs_review`）而不是
     自动裁决；合并是纯本地确定性计算（同输入同字节），不额外调用 AI、不无限循环。
  6. **同步恢复语义按合同实施，披露一处账本边界**：崩溃恢复路径由 T10 执行器收敛（只改 attempt、
     不动账本），预留保持 `reserved`（同样占用预算、等待对账）；处理器**当场**判定结果未知时
     才把预留标 `unknown`。两者都不自动释放、不填 0。
  7. **`SubmissionWindow::begin_intent` 改 `BEGIN IMMEDIATE`（跨卡加固）**：该事务"先读（查未决
     attempt）后写（插 intent）"，WAL 下 deferred 事务的读→写升级遇到活跃写者会**立即**
     SQLITE_BUSY（`database is locked`，busy_timeout 不等待）——T14 冒烟实测 1 次把付费批次
     误判为可重试失败（安全但浪费额度）。先取写锁由 busy_timeout 正常等待（与
     `job_stages::claim_next` 同一模式）；T12 的付费提交路径同受益。回归守卫：
     `manual_ai_contract::begin_intent_survives_active_writer_contention`（修复前 5/5 失败、
     修复后通过）。
  8. **显式重试前清理陈旧结果引用（`reset_result_fact`）**：`needs_input` 批次被重新授权后，
     新请求发出前清掉本阶段的 `result_asset_id`/`usage_json`（旧资产仍保留），否则"新请求未
     持久化完整响应"时恢复矩阵会把上一次的拒答诊断当成已完成结果补推进（阶段会以
     `succeeded` 状态携带"未产出知识"的内容）。
  9. **提示注入防护是"提示词 + 结构"双层的**：提示词显式声明资料是数据不是指令；请求体只由
     冻结输入构造（页内容无法影响预算/模型/页集合/工具）；服务端只接受"本批输入页 + 同批部件"
     引用；资料文本不进入任何 URL/命令路径。用例同时断言"网络行为不变"（fixture 计数）。
 10. **不结算说明书 AI 预留**：批次 `usage`（token 计量）只保存为事实；把 token 换算成 USD
     需要供应商计费模型的实测证据（含页图是否重复计入 input token），本卡不把估算当"实际金额"、
     也不填 0。结算/释放路径属 T15/T23（与 ADR-021 的 unknown/释放分离一致）。
- 影响的 REQ／任务／模块：REQ-029（主）、REQ-021/023（预算不被资料改变、预留不被静默释放）、
  REQ-024/025 的同步分支语义（T14 不提供 `attachRemoteTask`）；T15（合并结果资产是组装草稿的输入；
  重试/对账入口需先处理 `needs_input` 批次与陈旧结果引用）、T17（阶段 `usage_json`/诊断码的展示与
  脱敏）、T20（导出不含诊断 blob；purpose 语义若变更需同步）、T23（真实模型拒答/截断率、真实
  `store` 支持、真实 token 计费与结算）。
- 未验证边界与下一步：真实模型的稳定性与拒答率、真实 `usage` 字段形态与计费口径、`store=false`
  在各模型上的支持情况、`max_output_tokens` 与真实截断行为、100 页（20 批）规模下的耗时与费用——
  均属 T23。批次结果 purpose 的收口（新增专用取值）需 PM 决定后走一次性迁移。
- 相关代码／测试：`crates/core/src/knowledge.rs`、`crates/server/src/providers/manual_ai/**`
  （`client.rs`/`dto.rs`/`prompt.rs`/`store.rs`/`handlers.rs`）、`crates/server/src/jobs/submission.rs`、
  `crates/server/src/storage/repo/job_stages.rs`、`crates/server/tests/manual_ai_contract.rs`、
  `tests/fixtures/responses/manual_ai/**`、`artifacts/web-mvp/t14-rd/**`；
  [contracts §6](contracts.md#6-外部-provider-合同)、[implementation §T14](requirements/web-mvp/implementation.md)。

### T14 验收知识（追加，2026-09-12；QA 回合 16 结果 PASS，无新增缺陷，3 条 P3 观察）

- 状态：2026-09-12 回合 16 已执行。AC-045/AC-046 全部子句经 **QA 自写 14 条集成用例**
  （`crates/server/tests/qa_t14_independent.rs`：自建原始 TCP fixture、响应体与页文字全部现场构造）
  复现；`manual_ai_contract` 19 passed（RD 套件基线复跑，非通过依据）；workspace **445 passed / 0 failed
  / 3 ignored**；`xtask check` **7/7**；`contracts --check` 两份 `[一致]`；dist 哈希 = RD 值
  （`22a4fc4f…`）+ `smoke-bootstrap` 通过；lsof 观察非回环 **0**（ESTABLISHED 12 作正对照）。
  证据：`artifacts/web-mvp/t14-qa/**`、`qa-report.md` 回合 16 节。
- 结论/手法（跨卡复用）：
  1. **macOS/BSD 的 `accept()` 会继承监听 socket 的 `O_NONBLOCK`（本轮最大的测试陷阱）**：
     `TcpListener::set_nonblocking(true)` 之后 accept 出来的 socket 仍是非阻塞，`read()` 在客户端
     字节到达前返回 `EAGAIN`，表现为**随机**的"传输失败：请求发送失败/连接提前结束"，极易误判为
     产品缺陷（首轮 12 用例中 2–3 个随机失败）。本机 fixture 必须在 accept 后
     `stream.set_nonblocking(false)` 再 `set_read_timeout`。症状与"产品真的发不出请求"无法从错误
     文本区分，只能靠 fixture 端的读错误码（os error 35）定位。
  2. **多批用例的响应路由必须按键而不是按到达顺序**：`claim_next` 排序键为
     `COALESCE(next_run_at,0), created_at, id`，UUIDv7 在同一毫秒内的低位是随机的 →
     批次领取顺序不保证等于 `batch_index`。用"批内页内容标记"（如 `QA-PAGE-N-CONTENT`）选响应，
     断言才不会随调度漂移；`[第 1 页]` 与 `[第 11 页]` 这类页标记有子串歧义，不能当键。
  3. **lsof 观测必须有正对照**：毫秒级短连接在 200ms 采样下几乎看不见（首轮 76 行里 ESTABLISHED=0），
     "非回环 0"可能只是"什么都没采到"。做法：加一条 1.5s 慢响应用例把连接窗口拉长，先确认采样能
     看见 ESTABLISHED，再宣称 0。另：汇总行本身含 `TCP ` 字样，统计前要给采样行打前缀。
  4. **恢复矩阵对"拒答诊断结果资产"同样生效（P3-1 根因）**：`recover::plan` 以
     `stage.result_asset_id` 非空判定"结果已持久化 → 补推进"，而 T14 的失败路径也会写结果资产
     （拒答/incomplete/格式错都会落"零知识"批次结果）。因此"结果已落库、checkpoint 未推进"的崩溃
     路径会把拒答批次补推进为 `succeeded`；知识侧安全（`merge_batches` 按 `producedKnowledge` 拒收
     → merge `needs_input manual_batch_without_knowledge`），**T15/T17 不得只看阶段状态判定"该批
     产出了知识"**，应读批次结果的 `producedKnowledge`/`outcome`。
  5. **执行器错误日志的脱敏边界**：`job_stage_handler_error.detail` 打印 `JobError` 的 Display。
     当前无泄密面（Handler 消息由处理器构造、Storage 消息来自 SQLite 的表/列/约束名与我们自己的
     触发器文案、manual_ai 客户端错误先经 `redact_url_query` + 截断），但它是"未来处理器错误消息"
     的转发通道——新增处理器不要把页原文、签名 URL 或密钥放进错误串（T23 记得复核）。
  6. **提交窗口的 `BEGIN IMMEDIATE` 加固经 QA 独立复现确认**：活跃写者持锁期间 `begin_intent`
     等待后成功（不再立即 `database is locked`），且"未决 attempt 不得叠加"（submitting → 新建 intent
     报错且不落行）语义未变；残余形态是"等满 busy_timeout 后失败"（intent 未创建、可安全重领、
     不重复付费），本回合未构造。
- 未关闭的观察（非缺陷，供 T15/T17/T20/T23 与 PM）：
  1. **P3-1**：拒答批次在崩溃恢复路径被补推进为 `succeeded`（展示口径，见结论 4）。
  2. **P3-2**：**成功**批次的完整原始供应商响应也长期留存（blob + `assets` 行，无保留窗口/清理策略），
     内容含模型读出的页文本；契约只要求"短摘要 + 受限诊断路径"。保留策略需 PM/T15/T23 决定。
  3. **P3-3**：批次结果资产复用 `page_text` purpose（ADR-024 第 2 条同结论）；QA 复核确认无按 purpose
     的既有查询受影响，风险在 T20 导出/备份按 purpose 分类时误判。
  4. 回合 14/15 遗留：`qa_t10_independent.rs` / `qa_t11_independent.rs` 的过时 `#[ignore]` 仍在
     （3 ignored 计数不变），建议后续批次转正或更新注释。
- 相关代码／测试：`crates/core/src/knowledge.rs`、`crates/server/src/providers/manual_ai/**`、
  `crates/server/src/jobs/{submission.rs,recover.rs,executor.rs}`、`crates/server/tests/qa_t14_independent.rs`、
  `artifacts/web-mvp/t14-qa/**`、[qa-report 回合 16](requirements/web-mvp/qa-report.md)。

## ADR-025 — T15 组装草稿与任务控制端点落地取舍（assemble 领取规则、幂等 upsert、retry/cancel/reconcile 语义、P3-1 判据）

- 状态：verified（2026-09-12，T15 实测：`cargo test -p everything-manual --test pipeline` **11 通过**、
  `cargo test --workspace` **460 通过 / 0 失败 / 3 ignored**、`cargo xtask check` 7/7、
  `contracts --check` 两份 `[一致]`、`cargo xtask dist` 两次哈希一致
  `f6bc9e2a1380a8c018aaeee2ca9f5abf97cd8f9198940a54d39e1bfee09ec85c` + `smoke-bootstrap` 通过、
  **测试构建二进制 + 本机三段 fixture 的手工端到端**（A 全链路草稿 / B 知识分支失败后按分支重试 /
  C 取消耗费语义；零真实外网/付费）；证据见
  [implementation §T15](requirements/web-mvp/implementation.md) 与 `artifacts/web-mvp/t15-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T15）。
- 背景与证据：PRD 修订 2 REQ-030（主）/REQ-025/REQ-026 与 AC-047/048、AC-037/039/040 的端点侧；
  contracts.md §5（DAG、事件表、取消行、reconcile 细则、retry 限制）、§2（manual_drafts/audit 与
  「草稿≠已发布」「聚合更新需 If-Match」）、§3（`GET/PATCH drafts`、`GET /jobs[/{id}]`、cancel/retry/
  reconcile 路由）、§4；ADR-005/006/020/021/024；T10 已验收的取消与分支独立语义（qa-report 回合 10/11）、
  T14 P3-1（回合 16）。
- 结论与原因：
  1. **`assemble_draft` 的领取规则放宽到"依赖已定性"**：普通阶段仍要求依赖全部 `succeeded`；
     组装阶段接受依赖处于 `succeeded/failed/needs_input/submission_unknown/cancelled`。
     理由：REQ-030 要"部分成功可展示"（分支头被阻塞时草稿仍产出并标缺项），而 T10 已验收
     "上游批次失败 → merge 保持 `queued` → assemble 保持 `queued`"（QA 回合 11 的用例断言）不能破坏。
     代价与边界：**上游阻塞不产出草稿**（分支未定性），部分成功在任务详情的阶段状态里展示；
     上游被人工重试后由 `requeue_succeeded_dependents` 把组装拉回队列。
  2. **父 job `succeeded` 的含义收紧为"完整可复核草稿已产出"**：部分组装时父 job 显示阻塞状态
     （needs_input/failed/unknown）。不把部分草稿冒充成完整产物。
  3. **草稿外壳 `manual_draft_v1`**（`completeness`/`model`/`knowledge`/`missing[]`）：知识存
     `MergedKnowledge` 原文（保留其自身 schemaVersion），模型只存**不可变 revision 引用**；
     外壳允许追加字段（T19 扩展不破坏旧草稿可读）。
  4. **幂等 upsert 用"内容比较"而非"总是覆盖"**：`snapshot_id UNIQUE` + `ON CONFLICT … WHERE
     内容不同` + `RETURNING`。内容相同 → 不写库、不递增 revision（重放/重跑/重启同版本）；
     内容变化 → `revision+1` 且 `status` 拉回 `needs_review`（旧"已复核"声明不再适用）。
     `assemble_draft` **不写独立结果资产**（草稿行即产物），避免"陈旧结果被恢复补推进"的窗口。
  5. **`retry` 的拒绝清单**：阶段必须 `failed`/`needs_input`；同分支有未对账提交 → 拒；
     分支预留已释放/缺失 → 拒（`budgetNotHolding`：重试会重新请求，必须有预算背书）；
     任务已取消 → 拒。`Idempotency-Key` 必填（重放优先于 If-Match 返回原结果——重复点击/
     断线不产生第二个 attempt）。只重置指定阶段并**清下游闭包内 succeeded 阶段的结果事实引用**
     （否则恢复矩阵会用陈旧结果补推进）；旧资产不删除。
  6. **`cancel` 的"保留查询与账务"解释**：未提交阶段转 `cancelled`；已提交（`waiting_provider`）
     与 `submission_unknown` 保持原状态，attempt/远端 task ID/预留保留可查；**本地不再自动轮询**
     （沿用 T10 已验收的"取消后不领取该 job 任何阶段"），远端是否仍在计费由管理员在账户侧核对；
     响应固定 notice 明示"不撤销远端付费操作"。若 T23 需要"取消后继续轮询到远端终态"，
     属新语义需 PM 修订。
  7. **`reconcile` 三动作的落地边界**：`attachRemoteTask` 仅异步链路（同步 Manual AI 明拒），
     需二次确认 + **用当前凭据查询验证可访问性与任务形态**（查询失败无副作用，且**不**据此宣称
      "任务不存在"）；`recordNoTask` 要求证据、审计里标 `providerProof:false`（管理员声明不是
     供应商证明）；`authorizeReplacement` 需再次预算确认（覆盖冻结上界）+ 重复收费风险确认，
     创建新 attempt 而**不新建预留**（旧条目保留未决）。unknown 预留一律不自动释放。
     已取消任务的 `authorizeReplacement` 被拒（取消后不新增付费步骤），
     `attach`/`recordNoTask` 仍可用于账务收尾（只记录不推进）。
  8. **P3-1 判据 = 事实里的 `producedKnowledge`，且只认显式 `false`**：`recover::plan_succeed`
     用它决定"补推进为 succeeded 还是 needs_input"；字段缺失（旧事实/夹具）保持 T10 语义，
     避免无谓破坏已验收行为。任务详情的 `stages[].knowledgeProduced` 用同一字段（展示与恢复同判据）。
  9. **草稿 PATCH 本卡只接受 `status`**：字段级校验、Part/Step/Evidence/Hotspot 引用与 `modelReview`
     属 T19；`deny_unknown_fields` 明确拒绝，不提供绕过 T19 校验的入口。
  10. **父 job 状态的人工逃逸**（`recompute_status_after_retry`）：人工重试/替代提交后允许把
      `failed` 的父任务按阶段重新聚合（仅此一处允许脱离终态）；另一分支仍失败时不制造"已恢复"假象。
- 影响的 REQ／任务／模块：REQ-025/026/030/031（主）、REQ-023 的预算语义（重试必须有预算背书）；
  T17（任务中心/草稿页的 API 语义：阶段明细、`needsInput`、`knowledgeProduced`、对账入口、
  cancel/retry 端点）、T19（草稿外壳扩展、modelReview、发布事务）、T20（批次结果/诊断资产的
  purpose 与导出处置）、T21（崩溃断点矩阵扩展）、T23（真实账户的 attach 验证、真实轮询与结算）。
- 未验证边界与下一步：真实供应商 `attachRemoteTask`（需真实账户 ID）、取消后远端最终状态的人工
  核对流程、`GET /jobs` 的 N+1 查询在更大规模下的表现、多进程并发（不在 MVP）。
- 相关代码／测试：`crates/server/src/drafts/**`、`crates/server/src/jobs/{pipeline,control,recover}.rs`、
  `crates/server/src/storage/repo/{drafts,job_stages,jobs,attempts}.rs`、`crates/server/src/http/{jobs,drafts}.rs`、
  `crates/server/tests/pipeline.rs`、`artifacts/web-mvp/t15-rd/**`；
  [contracts §5](contracts.md#5-任务阶段与崩溃语义)、[implementation §T15](requirements/web-mvp/implementation.md)。

### T15 验收知识（追加，2026-09-12；QA 回合 17 结果 PASS，无新增缺陷，3 条 P3 观察）

- 状态：2026-09-12 回合 17 已执行。AC-047/AC-048 全部子句与 AC-037/AC-039/AC-040 的**端点侧**
  经 **QA 自写 11 条集成用例**（`crates/server/tests/qa_t15_independent.rs`：自建「方法 + 路径
  精确/前缀 + 请求体标记」三元组路由的原始 TCP fixture，Tripo 上传/提交/查询/CDN 与说明书 AI
  响应体全部现场构造，全链路/取消/重试/对账/崩溃恢复由 QA 自己驱动）复现；workspace
  **471 passed / 0 failed / 3 ignored**；`xtask check` **7/7**；`contracts --check` 两份
  `[一致]`；dist 哈希 = RD 值（`f6bc9e2a…`，两次重建一致）+ `smoke-bootstrap` 通过；
  lsof 正对照观察非回环 **0**（常规 ESTABLISHED 2 / 正对照 762，全回环）。
  证据：`artifacts/web-mvp/t15-qa/**`、[qa-report 回合 17](requirements/web-mvp/qa-report.md)。
- 结论/手法（跨卡复用）：
  1. **fixture 路由要三元组**：`(方法, 路径精确/前缀, 请求体标记)` 命中即消费脚本，能同时驱动同一
     host 上的多供应商多路径；**未命中必须显式失败**（501 + unexpected 列表），否则路由错误会被
     伪装成产品失败（例如"该批拿到拒答"其实是脚本没命中）。
  2. **lsof 正对照用环境变量开关的 fixture 延迟**：把响应延迟做成 `QA_T15_SLOW_MS`（默认 0，
     正常跑不受影响），同一条命令跑两遍——常规 ESTABLISHED 只有 2 行且全回环，正对照 762 行全回环，
     "非回环 0"因此有灵敏度背书。比"另写一条慢用例"更省事且不污染断言。
  3. **`tripo_poll` 成功即结算（`settled` 不占预算）**：于是**任何 Model 分支阶段**的手动 retry 都
     会被 `budgetNotHolding` 拒绝；模型分支头（`model_validate`）进入 `needs_input` 后，公开 API
     没有重试入口（只能新建任务/新购买），而草稿 `missing[]` 文案仍写"可对该阶段重试"。写测试或做
     UI 时不要把"模型侧可重试"当既有事实（qa-report 回合 17 P3-1）。
  4. **草稿 revision 的递增基准是"当前 revision"**：状态 PATCH 也会 +1（`needs_review→ready` 即
     `r2`），内容变化在当前值上再 +1；断言不要以"首次组装的 revision + 1"为基准。
  5. **assemble 领取放宽的实测边界**：放宽只作用于两条**分支头**（`manual_merge` / `model_validate`）；
     上游批次阻塞时分支头仍 `queued` → 组装不可领取（QA 三组现场实测 `drafts=0`），不会提前产出
     误导性部分草稿；部分组装时父 job 保持阻塞状态（不置 `succeeded`，ADR-025 第 2 条的收紧）。
     `requeue_succeeded_dependents`（已成功组装因上游重试回队列）在本卡可达的失败形态下**较难触达**：
     模型头 `needs_input` 时模型侧重试被预算门槛拒绝，知识头 `needs_input` 需要批次已全部
     `succeeded`（此时批次不可重试）——T17/T21 若要"部分草稿补齐"的 UI 入口，需先明确可达路径。
- 未关闭的观察（非缺陷，供 T17/T21/T22 与 PM）：
  1. **P3-1**：模型分支头 `needs_input` 时重试被 `budgetNotHolding` 拒绝且文案仍称可重试（见结论 3）。
  2. **P3-2**：`implementation.md` §T15-10 记录的 dist 体积（22 434 320 B）与实际产物
     （22 494 896 B，哈希 `f6bc9e2a…`）不符；发布报告应以实际 `stat`/`SHA256SUMS` 为准。
  3. **P3-3**：AC-048/AC-039 的 UI 文案侧与 `import-flow.spec.ts` / `job-recovery.spec.ts` 属
     T16/T17；本卡只验端点与 DTO（`notices`/`notice`/`stageSummary`/`needsInput`/`knowledgeProduced`）。
  4. 说明书 AI 预留长期停在 `reserved`（`actual=null`），本卡未构造结算场景；账务收尾语义待 T23。
- 相关代码／测试：`crates/server/src/drafts/**`、`crates/server/src/jobs/{pipeline,control,recover}.rs`、
  `crates/server/src/storage/repo/{job_stages,jobs,drafts,attempts}.rs`、`crates/server/src/http/{jobs,drafts}.rs`、
  `crates/server/tests/qa_t15_independent.rs`、`artifacts/web-mvp/t15-qa/**`；
  [contracts §5](contracts.md#5-任务阶段与崩溃语义)、[ADR-025](#adr-025--t15-组装草稿与任务控制端点落地取舍assemble-领取规则幂等-upsertretrycancelreconcile-语义p3-1-判据)。

## ADR-026 — T16 资料库与新建向导落地取舍（会话指针、预设常量、XHR 上传单一实现、乐观确认、URL 即步骤、禁用入口）

- 状态：verified（2026-09-12，T16 实测：`npm --prefix apps/web run test:e2e -- import-flow.spec.ts` **6 通过**、
  全量 `npm --prefix apps/web run test:e2e` **22 通过**（T16 6 + T09 10 + QA 回合 6）、
  `npm --prefix apps/web run test -- --run` **62 通过**（T16 新增 16）、`cargo xtask check` **7/7**、
  `cargo test --workspace` **471 passed / 0 failed / 3 ignored**、`cargo xtask contracts --check` 两份 `[一致]`、
  `cargo xtask dist` 两次哈希一致 `5c33ce93adaea07d00560a353eeda97aecd3226abfed323868f2412dc08f48cf`
  （22 544 432 B）+ `smoke-bootstrap` 1+7 全过；真实浏览器逐步截图 16 张。
  证据见 [implementation §T16](requirements/web-mvp/implementation.md) 与 `artifacts/web-mvp/t16-rd/`）。
- 日期／作者角色：2026-09-12 · RD（T16）。
- 背景与证据：PRD 修订 2（ui_revision 2）REQ-016（主）/REQ-021/REQ-030、AC-026/AC-030（UI 侧）/AC-048（UI 侧）、
  §6.1 路由与断点、§6.2 UI-005/006/009–014/019–026、§6.3 禁用措辞与边界、§8.5（U-01–U-12）；
  contracts §1/§3/§4；T11 的报价/确认/幂等键语义、T15 的 `GET /jobs` 与建单契约、T09 的准备页与 Playwright 设施。
- 结论与原因：
  1. **向导第 5 步用"会话指针"找 preparation**：服务端没有"按 document 列出 preparation"的读取端点，
     而 `POST /items/{id}/estimates` 需要 `preparationId`。取舍：第 4 步把 preparationId 写入
     `sessionStorage['em.prepare.<itemId>']`（T09 已有同形态指针），第 5 步读取后仍以
     `GET /preparations/{id}` 为权威状态；指针缺失/失效一律按"准备未完成"处理，不伪造状态、不新开准备记录。
     **这是已知限制**（换浏览器/清存储需回第 4 步），建议后续卡加 `GET /items/{id}/preparations`
     或把 preparationId 落进 document DTO；在此之前不假装"任意浏览器都能直接进第 5 步"。
  2. **模型预设是前端常量 + 服务端 422 兜底**：`/settings/status` 不返回预设清单、`POST /estimates` 又要求显式
     `modelPreset`。取舍：按 T11 唯一受支持预设 `tripo-h-v3.1-standard`（= `price-catalog.example.toml`）提交；
     服务端目录不同时按 422 `modelPresetUnsupported` 的 `details.supportedPresets` 照实显示，**不猜测、不回落 mock**。
     建议后续在 settings 暴露 `supportedPresets`（属合同新增，本卡不改）。
  3. **multipart 上传收敛为单一实现（XHR）**：fetch 没有上传进度事件，而 UI-009 要求"按字节显示进度"且可取消。
     `features/import/upload.ts` 成为唯一实现（`AssetUploadError` 带 status/code/details/requestId），
     `api.ts#uploadPageAsset` 委托给它——避免"两份 multipart 各自演化"（CSRF 注入与 403 刷新重试只有一处）。
  4. **确认勾选框做乐观更新**：勾选后立即显示为已勾选并禁用，失败回退 + 原因（不做"点了没反应"）。
     语义不变：未确认的提交仍被服务端 422 拒绝（`confirmationRequired`），确认动作仍由 `/confirm` 写 audit_events。
  5. **URL 即步骤、导航只改 URL**：步骤条与上一步/下一步都是 `<Link>`，不携带或保存状态；
     每步的数据都从服务端查询（documents/photos/preparation），因此"返回上一步/刷新"天然不丢资料
     （e2e 以"返回+前进后 POST assets/photos 计数为 0"为证）。前置未满足时"下一步"保留为禁用控件 + 原因
     （`aria-describedby`），不做死链接。
  6. **资料库行内入口按能力如实禁用**：「继续准备」可用（第 2–4 步已交付）；
     「查看任务」「打开说明书」分别等 T17 与 T18/T19，渲染为 disabled + 常驻原因（不链到占位页假装可用）。
  7. **UI-021 冻结快照提示用只读 `GET /jobs?itemId=`**：物品页/编辑页/上传照片与说明书的步骤页显示
     常驻提示（"进行中任务使用冻结快照；修改用于下次生成，需重新报价确认"）；查询失败静默不显示，
     不把只读提示升级为页面错误，也不实现任务中心交互（属 T17）。
  8. **e2e 的 Provider 配置指向 `127.0.0.1:1`（保留端口，连接被拒）**：T16 只验证到"建单受理"，
     executor 对不可达端的重试是**预期现象**；密钥用专用假环境变量（`EM_E2E_TRIPO_KEY` 等），
     浏览器侧阻断非本机请求并断言 `external == []` ——"零真实外网/零付费"有可复核证据。
     不用 fake 成功响应冒充供应商（fixture 语义留给 T10/T12/T14/T15 的 Rust 测试）。
- 未关闭的观察（非缺陷，供 T17/T21 与 PM）：
  1. **报价是"进入第 5 步即自动获取一次"**（同一输入签名只请求一次）：不收费、不写账本；
     若 PM 希望"仅在用户点击后获取"，属交互调整（U-05 仍要求过期后手动重报）。
  2. **`GET /jobs` 在本卡只用于 UI-021 只读提示**；`/jobs` 页面、轮询（2s/15s）与恢复入口属 T17。
  3. **AC-030 的 audit_events 断言仍在服务端侧**（T11 用例）；本卡只交付告知与确认的前端行为。
- 相关代码／测试：`apps/web/src/features/import/{upload,upload-messages,money,views,WizardSteps,preparation-pointer,
  AssetUploadCard,DocumentStepPage,ViewsStepPage,ConfirmStepPage,MissingItemsList}.ts(x)`、
  `apps/web/src/features/library/{JobSnapshotNotice,LibraryPage,ItemOverviewPage,ItemFormPage}.tsx`、
  `apps/web/src/api/{client,endpoints}.ts`、`apps/web/tests/e2e/{import-flow.spec.ts,global-setup.ts,helpers.ts}`、
  `artifacts/web-mvp/t16-rd/**`；[PRD §6.1/§6.2/§6.3](requirements/web-mvp/prd.md)、
  [contracts §3/§4](contracts.md)、[ADR-019](#adr-019--t09-pdf-准备落地取舍页数上限两层分工业务冲突-422reasonviewport-落库幂等按内容哈希vendor-同版本内嵌通知不遮挡主入口)、
  [ADR-021](#adr-021--t11-报价确认冻结预留幂等建单落地取舍)。

### T16 验收知识（追加，2026-09-12；QA 回合 18 结果 **FAIL**，1 条新增缺陷 BUG-005（P2））

- 状态：2026-09-12 回合 18 已执行。AC-026 全部子句、AC-030（Playwright 侧）全部子句、
  AC-048 的"无自动发布路径"子句与 UI-006/009–026 经 **QA 自写 12 条 Playwright 用例**
  （`apps/web/tests/e2e/qa-t16-independent.spec.ts`，真实后端 + 真实 Chrome，DOM/网络头/服务端事实/
  几何测量四路证据）复现；全量 e2e **33 passed / 0 failed / 1 skipped**（skipped = BUG-005 复现）；
  `npm run typecheck/lint/test(62)/build` 全绿；`cargo xtask check` **7/7**；
  `cargo test --workspace` **471 passed / 0 failed / 3 ignored**；`contracts --check` 两份 `[一致]`；
  dist 两次哈希一致 `5c33ce93…`（22 544 432 B）+ `smoke-bootstrap` 1+7 全过 + 内嵌文案 8 条命中；
  lsof 12 轮采样非回环 **0**。证据：`artifacts/web-mvp/t16-qa/**`、
  [qa-report 回合 18](requirements/web-mvp/qa-report.md)。**唯一阻断项 = BUG-005**（UI-005 行布局，
  见下第 4 条）。
- 结论/手法（跨卡复用）：
  1. **Playwright `route.request().headers()` 能看到 `x-csrf-token`（与裸 CDP 不同）**：
     本回合实测 `POST /items/{id}/jobs` 的头名含 `x-csrf-token`、`idempotency-key`、`origin`、`cookie`
     （`t16-qa/playwright-request-headers.json`）。这与 T09 回合 9 的结论"Chrome 152 的
     `Network.requestWillBeSent` 不上报 `x-csrf-token`"**不矛盾**：那是裸 CDP 事件，Playwright 1.60
     的 `Request.headers()` 走的是另一条路径。T17/T19 断言 CSRF/幂等头**优先用 Playwright 的
     `request.headers()`**，不必再搭 `Fetch.enable` 回调。
  2. **fixture 页码不要用朴素正则数**：`tests/fixtures/assets/sample-manual-text.pdf` 是 **2 页**
     （PDF.js 实际解析）；正则数 `/Type /Page` 会数出 3（含 `/Type /Pages` 等噪声）。准备进度断言写
     `/第 \d+ \/ \d+ 页/` 或读 `numPages`（本回合第一版即因此误判超时一次）。
  3. **行式列表新增"整句说明文字"会撑爆 grid 行（BUG-005 的通用形态）**：`grid-template-columns` 含
     `auto` 轨道时，轨道按该单元格 max-content 定宽；说明文字（702px）把 `2fr/1fr/1fr` 压成
     0/31.6/45.6px（1280px 视口），身份列 0px → 名称逐字换行、单行高 358px。**新增长文本前先看容器是否
     有显式布局**，并实测 `getComputedStyle(row).gridTemplateColumns` + `boundingBox()`
     （Playwright 两行代码即得证据）。T17 任务中心列表会加阶段/费用列，同类风险高。
  4. **对二进制做多字节 grep 必须用字节语义**：`zh_CN.UTF-8` 下 `grep -a "中文串" <binary>` 会因无效
     多字节序列**漏报**（本回合复核 dist 内嵌文案时踩到）；`LC_ALL=C grep -a` 或 Python `bytes.count`
     才可靠（`t16-qa/qa-check-embedded-strings.sh`）。
  5. **e2e 证据目录与 RD 共享仍会互相覆盖**（T09 回合 9 的 P3-2 仍开放）：`playwright.config.ts` 固定写
     `artifacts/web-mvp/t09-rd/`，本回合 4 次运行再次覆盖其中的 `e2e-server.log` 与 `playwright-output/`；
     QA 已另存 `t16-qa/e2e-server.qa-t16.log`。T21/T22 前应改为按运行/角色隔离。
- 未关闭的观察（非缺陷，供 T17/T21/T22 与 PM）：
  1. **AC-048 的"生成成功后界面提示需人工确认才能发布"本轮不可达**：e2e 的 Provider 指向
     `127.0.0.1:1`，job 停在 queued/retry，草稿页横幅属 T18/T19。**不得**据回合 18 报告宣称该子句通过。
  2. **资料库行内「查看任务」「打开说明书」为真 disabled + 原因**（T17/T18/T19 未交付）；顶栏「任务中心」
     为可用链接指向写明"尚未实现"的占位页——属可接受形态，T17 交付时替换。
  3. **真实磁盘满（`insufficientStorage`）未在浏览器链路端到端触发**：本回合用"服务端形状的注入 413"
     验证前端渲染（字段/文案逐字取自 `crates/server/src/assets/{error,blob_store}.rs`），服务端侧由 T06
     `assets.rs` 的 `SpaceProbe` 用例覆盖；真实 statvfs + 写流中途 ENOSPC 仍是开放边界。
  4. **第 5 步会话指针（ADR-026 第 1 条）**：QA 独立复现"无指针 → 如实报缺项、不请求报价、不伪造状态"。
- 相关代码／测试：`apps/web/tests/e2e/qa-t16-independent.spec.ts`（QA 新增，12 用例，QA-10 因 BUG-005
  `test.fixme`）、`artifacts/web-mvp/t16-qa/**`（几何 JSON、请求头采样、lsof 脚本与日志、24 张截图）；
  [PRD §6.2/§6.3](requirements/web-mvp/prd.md)、[ADR-026](#adr-026--t16-资料库与新建向导落地取舍会话指针预设常量xhr-上传单一实现乐观确认url-即步骤禁用入口)。

### T16 BUG-005 修复知识（追加，2026-09-12；RD 修复回合，QA 回合 19 待复验）

- 状态：BUG-005（P2）已修复（`styles.css` + `LibraryPage.tsx`），几何与全量回归见
  `implementation.md` §T16-FIX；证据 `artifacts/web-mvp/t16-rd-fix/**`。
- 结论/手法（跨卡复用）：
  1. **媒体查询不增加特异性——覆盖规则必须放在基础规则之后**。T16 的
     `@media (max-width:767px) { .item-row { grid-template-columns: minmax(0,1fr) } }` 写在基础
     `.item-row` 之前，被后出现的同特异性规则静默覆盖（"死规则"），**窄屏单列堆叠从未生效**
     （探针实测 767/375px 仍四列、身份列 0px）。凡写"窄屏/宽屏覆盖"先确认源顺序，
     并用 `getComputedStyle().gridTemplateColumns` 实测——CSS 里看得见的规则不等于生效。
  2. **grid 行内新增"常驻整句说明"的正确形态**：说明文字独占整行（`grid-column: 1 / -1`，且必须是
     行的直接子元素，不要塞进四列中的某一列）；四列用显式 `minmax(0, …)` 轨道 + 子项 `min-width: 0`；
     操作区 `display:flex; flex-wrap:wrap`；名称 `display:block`（几何可测、点击区/焦点框覆盖整列）。
     **反例（BUG-005）**：说明文字内联在 `.item-row__actions`（块级、无 CSS）里 → 末端 `auto` 轨道按
     702px max-content 吃掉全部宽度，三个 `fr` 列合计被压到 77px、身份列 0px、名称逐字换行、行高 358px。
     阈值参考：名称/身份列 ≥120px、行高 ≤140px（1024–1920px）。T17 任务中心列表（阶段/费用列）同类风险高。
  3. **行几何断言的测量法**：视口切换会触发断点重挂载（wide/mid/narrow 三套 DOM）——分次
     `locator.boundingBox()` 会拿到 stale 句柄（返回 null）或混入上一档数值（QA-10 在 1280px 曾出现
     name=228 与 grid=346 不一致的瞬态，不影响其 ≥120px 断言）。可靠做法：`setViewportSize` 后
     `waitForTimeout(100)`，再用**一次 `row.evaluate()` 原子取整行 `getBoundingClientRect`**。
  4. **后端偶发 500（OPEN，非本次范围）**：并发建照片时 `POST /items/{id}/photos` 可返回 500
     `database is locked`（SQLITE_BUSY code 5）——`crates/server/src/http/photos.rs:90` 的 deferred 事务
     （`view_occupant` SELECT → `photos::create` INSERT）在 WAL 下读→写升级遇活跃写者**立即**失败、
     `busy_timeout` 不等待，与 T14 已修的 `begin_intent`（本文件 1091 行）同类；e2e 全量首跑
     QA-5 因此失败、复跑通过（服务端日志 `ERROR 存储错误…database is locked`，同 requestId 500）。
     建议后续卡改 `BEGIN IMMEDIATE`；前端行为正确（如实报缺项、不伪造成功），故只是稳定性问题。
- 相关代码／测试：[`apps/web/src/styles.css`](../apps/web/src/styles.css)、
  [`apps/web/src/features/library/LibraryPage.tsx`](../apps/web/src/features/library/LibraryPage.tsx)、
  [`apps/web/tests/e2e/library-row-layout.spec.ts`](../apps/web/tests/e2e/library-row-layout.spec.ts)、
  `crates/server/src/http/photos.rs`；
  [implementation.md §T16-FIX](requirements/web-mvp/implementation.md)、
  [qa-report 回合 18 BUG-005](requirements/web-mvp/qa-report.md)。

### T16 BUG-005 复验与并发 500 复现知识（追加，2026-09-12；QA 回合 19，T16 切片 **PASS**）

- 状态：BUG-005（P2）**CLOSED**（QA 解除 `qa-t16-independent.spec.ts` QA-10 的 `test.fixme` 后 1 passed；两次全量 e2e 均 36/0/0）。新增 **BUG-006（P2 / OPEN）**：`POST|PATCH /items/{id}/photos` 在活跃写者下 deferred 事务读→写升级立即 `database is locked`（code 5/517）→ 500。证据 `artifacts/web-mvp/t16-qa-r19/**`、[qa-report 回合 19](requirements/web-mvp/qa-report.md)。
- 结论/手法（跨卡复用）：
  1. **确定性复现"先读后写 deferred 事务"锁失败（两段式，跨卡可复用）**：① 外部 `python3 sqlite3` 连接对同一 DB 执行 `BEGIN IMMEDIATE` 并持锁数秒；② 期间用 curl 打应用端点（该端点事务是 `BEGIN`(deferred) → SELECT → INSERT）。实测 `POST /photos` **3.59ms 即 500**（未等 `busy_timeout=5s`），锁释放后同一请求 201。纯 SQLite 对照实验（同库同 `busy_timeout=5000`）：deferred `SELECT→INSERT` **0.001s 失败**、deferred `INSERT` 先写**等待 2.14s 后成功**、`BEGIN IMMEDIATE` 正常——**"读快照后升级写字锁"不能靠 busy_timeout 重试**，这是 T14 `begin_intent`（`jobs/submission.rs`）、T14 `claim_next`（`job_stages.rs`）都用 `BEGIN IMMEDIATE` 的原因；凡"先查后插/先查后改"的事务都要按此甄别。
  2. **执行器是常驻写者**：`jobs/executor.rs` 的 `idle_poll = 250ms`，每次 tick 的 `claim_next` 都 `BEGIN IMMEDIATE`（空闲也取写锁）。因此"用户单发一个写请求"与"后台 tick"天然竞争——这就是 e2e 全量偶发 `database is locked` 的来源（RD 首跑 1 次命中；QA 顺序单发 20 次 0 命中，属低概率自然命中）。复现脚本：`artifacts/web-mvp/t16-qa-r19/repro-conc-photos-500*.sh`（A 确定性 / B 自然并发 / C 聚焦量化；B 400 请求命中 500×237）。
  3. **跑测前先备份会被覆盖的证据**：`playwright.config.ts` 固定写 `artifacts/web-mvp/t09-rd/`，RD 守卫 spec 固定写 `artifacts/web-mvp/t16-rd-fix/`——QA 跑一次全量就会覆盖前一角色的 `e2e-server.log`、`library-row-geometry*.json` 与截图。**先 `cp -p` 到自己的证据目录再跑**（本回合已照此把 RD 的几何 JSON 存为 `r18-rd-fix-*.json`）。
  4. **写 Playwright 结构探针的正确形态**：`row.evaluate()` 内一次取 `getBoundingClientRect` + `getComputedStyle`（含 `note.parentElement === node`、`gridColumnStart/End`、`devicePixelRatio` 无关的 CSS px），改视口后 `waitForTimeout(100)` 等断点重挂载；探针 spec 运行后删除（原 stdout tee 到 artifacts），避免留下未派发的测试文件。
  5. **bash 陷阱两则（写复现脚本时踩到，值得跨卡记住）**：① `$VAR` 后紧跟全角字符（如 `$WORK（`）会被 zsh/bash 当成变量名的一部分 → `unbound variable`，必须写 `${VAR}`；② `{ ... } | tee log` 会让整块在子 shell 执行，块内启动的后台服务 PID 在主 shell 的 `EXIT` trap 里**不可见**，且无参 `wait` 会连服务一起等（脚本挂住）——收 `pids+=($!)` 逐个 `wait`，并按端口 `pkill -f "listen 127.0.0.1:<port>"` 兜底清理。
- 未关闭观察：BUG-006 未修（不阻断 T16；MVP 全量验收前必修，修法 + 守卫建议见 qa-report 回合 19 缺陷段）；`t16-rd-fix/` 与 `t09-rd/` 证据目录共享仍开放。
- 相关代码／测试：[`apps/web/tests/e2e/qa-t16-independent.spec.ts`](../apps/web/tests/e2e/qa-t16-independent.spec.ts)（QA-10 已解除 fixme）、[`crates/server/src/http/photos.rs`](../crates/server/src/http/photos.rs)、[`crates/server/src/jobs/executor.rs`](../crates/server/src/jobs/executor.rs)；`artifacts/web-mvp/t16-qa-r19/**`；[implementation.md §T16-FIX](requirements/web-mvp/implementation.md)、[qa-report 回合 19](requirements/web-mvp/qa-report.md)。

### BUG-006 复验知识（追加，2026-09-12；QA 回合 20，BUG-006 **CLOSED**，无新增产品缺陷）

- 状态：BUG-006（P2）**CLOSED**（QA 回合 20）。判据（回合 19 立案时写死）与结果：三条 QA 脚本原样重跑
  **500×0**（A 锁内 `POST /photos` → 201，等待 3.3947s；锁释放后同视图 → 422 `viewOccupied`；B 400 请求
  201×280 / 422×120 / 500×0；C code5=code517=status:500=0）；QA 自写 **337 请求**独立压测（同物品多视图 /
  同视图 6 并发 / 跨物品 / 常驻外部写者 / prepare+complete 与照片写并发 / 60 并发突发）**0×500**，
  422 全部 `viewOccupied`；`xtask check` 7/7、workspace 476/0/3、dist sha256 `dc05f695…`（QA 独立复现 RD 值）、
  smoke 1+7、全量 e2e **36/36 ×2** 且服务端 0 条 `database is locked`。证据 `artifacts/web-mvp/bug006-qa/**`、
  [qa-report 回合 20](requirements/web-mvp/qa-report.md)。
- 跨卡复用手法：
  1. **"修复后复验"的判据要在立案时就写死**（回合 19 已写明"同脚本重跑 + 0×500 + 合同状态码"），复验只执行、
     不重新定义口径；复验时**原样重跑旧脚本**比新写脚本更能证明前后对比。
  2. **并发类缺陷的"掩盖核对清单"**：`busy_timeout` 设置点唯一且值未变（5s）、连接池上限（4）、
     生成/批次并发上限（2/2）、执行器 tick（250ms）未调低、无新增 Mutex/RwLock/Semaphore、无 BUSY 捕获/重试层、
     `BEGIN` 字面量仍只有一处；再加"对照路径延迟 vs 修复前"证明快速路径无劣化（1.096ms → 1.170ms）。
  3. **全局复核计数法**：`grep -rn "\.begin()\|begin_with\|begin_write" crates/server/src` +
     `grep '"BEGIN IMMEDIATE"'` 交叉核对，一次证明"无残留 deferred 写事务 + 无第二处事务字面量"；
     抽查"先读后写"分类要真的读事务内**首条语句**（sqlx 事务里首条 SELECT 即高危）。
- **断点瞬态判定（产品缺陷 vs 测试瞬态）——要在页内逐帧采样**：用 rAF 循环在页内记录
  row/identity/grid/layoutClass，而不是多次 CDP 往返；多次 `boundingBox()`/`evaluate()` 会横跨 React
  断点重挂载，产生"row 是 mid 态 + 子元素是 wide 态"这种**单次布局不可能出现的组合**与 `name: null`。
  实测（1024→1280，5 周期）：切换后**第一帧即稳定**（row 840 / identity 228）、1.2s 内 0 个 identity<120 或 null 的帧、
  边界 1276–1282 往返无抖动；且**同一批 e2e 中 `getComputedStyle` 仍可能在重挂载窗口读出空字符串**
  （证明竞态真实存在但与产品布局无关）。结论：QA-10 的 1280px 失败是**测量法竞态**（低概率假 FAIL），
  加固 = 单次原子 `evaluate` + `setViewportSize` 后等待断点落定；**不得据此判产品或切片回退**。
- **脚本陷阱（本回合再次踩到并已清理）**：`{ … } | tee` 包住的脚本里 `SERVER_PID` 只在子 shell 可见，
  父 shell 的 EXIT trap 读不到 → **服务端进程残留**（临时目录已删、端口仍被监听）。写复现脚本必须
  在 cleanup 内按端口 `pkill -f "listen 127.0.0.1:<port>"` 兜底，脚本跑完核对 `lsof -nP -iTCP:<port>` 与
  `pgrep -fl everything-manual`（本回合发现并清理了回合 19/20 各遗留的一个 18091 进程）。
- 未关闭观察：QA-10 测量法加固（P3，测试侧，待协调者派卡/授权）；`t09-rd/`、`t16-rd-fix/` 证据目录共享
  （T09 起未关闭）；写事务持锁 >5s 仍为既有 500「数据库暂不可用」（合同未定义专用码）。

## ADR-027 — 写事务统一 `BEGIN IMMEDIATE`（BUG-006 系统性修复；`storage::tx`）

- 状态：verified（2026-09-12 RD 修复回合：QA 三条复现脚本重跑 500×0；`cargo test --workspace` 476/0/3；
  `cargo xtask check` 7/7；`dist` sha256 `dc05f695bb3f1ac3d27c53b2a41f68a16695cf5c43c4c5a89fe4d3039ac420da` + smoke 1+7；
  全量 e2e 36/36 且服务端 0 条 `database is locked`。证据 `artifacts/web-mvp/bug006-rd/`、
  [implementation §BUG-006](requirements/web-mvp/implementation.md)、[qa-report 回合 19 BUG-006](requirements/web-mvp/qa-report.md)）。
- 日期／作者角色：2026-09-12 · RD（BUG-006 修复回合）。
- 背景与证据：BUG-006（P2）——`POST|PATCH /items/{id}/photos` 在活跃写者下 3ms 级返回 500
  `database is locked`（code 5 / 517）；QA 400 并发 500×236、同视图 4 并发 28/40=500。
  机制（QA 纯 SQLite 对照可复现）：**WAL 下 deferred 事务"先读建快照、后写升级"不能等待写锁**，
  SQLite 立即返回 `SQLITE_BUSY`（`busy_timeout` 不参与）；deferred 且首条即写（INSERT）则正常等待 2.14s 后成功。
- 结论与原因：
  1. **判据 = "事务内第一条 SQL 语句"**：第一条是 SELECT、后面有写 → 必然有读→写升级风险；第一条即写 →
     当前安全但**安全性依赖语句顺序**，后续维护挪一条 SELECT 到首位即静默退化。故统一策略：
     **每个可能写库的事务都用 `BEGIN IMMEDIATE` 先取写锁**（封 `storage::tx::{begin_write, begin_write_pool}`），
     只读事务才用 `begin()`。本仓库 24 个事务点全部为写事务，全部改为统一封装（8 处先读后写为必修，
     14 处写先为防御性统一，2 处既有 IMMEDIATE 收敛到同一封装）。
  2. **不采用的做法及理由**：① 加大 `busy_timeout` 无效——升级失败**不经过** busy handler，改的是等待语义外的路径；
     ② 全局互斥/串行化会破坏既有并发上限与响应时间目标，且 SQLite 本身已串行写者，`BEGIN IMMEDIATE` 不降低写吞吐；
     ③ 通用"捕获 SQLITE_BUSY 重试"需要判定事务是否可安全重放（本仓库部分事务含条件更新/审计），
     语义与合同未定义，首版不做。
  3. **语义保持**：并发冲突回到合同承诺——同视图第二张 422 `viewOccupied`（现在是**在事务内看到已提交占用者**），
     跨视图并发 = 排队后 201；生成 2 / 批次 2 并发上限、连接池 4、`busy_timeout=5s`、`synchronous=FULL` 均未变。
  4. **守卫**：并发用例必须"与常驻写者同时写"才有判别力（执行器 `idle_poll` 每 tick `BEGIN IMMEDIATE`，
     空闲也是写者）；把 `photos.rs` 改回 deferred 时 `tests/photos_concurrency.rs` 3/5 失败（500），
     修复后 5/5 通过。**未采用 `#[ignore]`**（默认运行、只用合同状态码断言，不卡紧时序）。
- 影响的 REQ／任务／模块：REQ-013 / AC-021（并发语义）、contracts §1 状态码、§2 photos 唯一性；
  `crates/server/src/**` 全部写事务；后续任何新增写事务都必须使用 `storage::begin_write*`（评审检查点）。
- 未验证边界与下一步：真锁超时（>5s）仍走既有 500「数据库暂不可用」（合同未定义专用码，本卡不加）；
  未做 BUSY 重试/退避层；e2e QA-10 的 1280px 跨档瞬态仍开放（前端测量法问题，与后端无关）。
- 相关代码／测试：[`crates/server/src/storage/tx.rs`](../crates/server/src/storage/tx.rs)、
  [`crates/server/src/http/photos.rs`](../crates/server/src/http/photos.rs)、
  [`crates/server/tests/photos_concurrency.rs`](../crates/server/tests/photos_concurrency.rs)；
  官方依据 <https://www.sqlite.org/lang_transaction.html>、<https://www.sqlite.org/wal.html>。

## ADR-028 — T17 任务中心与可行动错误落地取舍（retry 准入单一判据、追加式 DTO、P3① 改文案而非放宽门槛、可见性驱动轮询、e2e 自管后端）

- 状态：verified（2026-09-12，T17 实测：`npm --prefix apps/web run test:e2e -- job-recovery.spec.ts` **8 通过**、
  `npm --prefix apps/web run test -- --run` **79 通过**（T17 新增 17）、`cargo test -p everything-manual --test pipeline` **12 通过**、
  `cargo test --workspace` **477 passed / 0 failed / 3 ignored**、`cargo xtask check` **7/7**、
  `cargo xtask contracts --check` 两份 `[一致]`、`cargo xtask dist` `3262691d25f4ae93cbf2d6d11620728d374a885366a2651ae35529a55a937219`
  （22 611 216 B）+ `smoke-bootstrap` 全过；真实浏览器截图见 `artifacts/web-mvp/t17-rd/screenshots/`）。
- 日期／作者角色：2026-09-12 · RD（T17）。
- 背景与证据：PRD 修订 2（ui_revision 2）REQ-023/025/026/031、AC-049、AC-033/034 的 UI 侧、
  §6.2 UI-027–UI-039、§6.3.2；contracts §3/§5；T15 的端点语义（`retryable_stage_status`、
  `branch_provider` 预算背书）与 QA 回合 17 的 **T15 P3①**（模型分支头 `needs_input` 时 retry 被
  `budgetNotHolding` 拒绝，而草稿缺项文案仍写"可对该阶段重试"）。
- 结论与原因：
  1. **重试准入抽成单一判据**（`crates/server/src/jobs/control.rs::retry_gate`）：端点
     `POST /jobs/{id}/retry` 与任务详情 `stages[].retry` 使用**同一个纯函数**（取消 → 状态 →
     同分支未对账 → 预算背书，reason/message/details 逐字一致）。理由：界面要么渲染一个必然被拒的
     按钮，要么自己复刻"账本状态 + 分支归属"的判定；两者都会漂移。代价是端点逻辑被重构一次
     （行为不变：既有 `pipeline.rs`/`qa_t15_independent.rs` 的断言全部保持通过）。
  2. **DTO 只做追加**：`JobStageDto.retry`（`{allowed, reason, message}`）与
     `JobStageDto.submissionStyle`（`asyncRemoteTask`/`syncResponse`）。不新增路由、不改请求体、
     不改状态枚举与错误码；`submissionStyle` 让"哪些阶段可用 attachRemoteTask"来自服务端事实
     （core 的 `submission_style`），前端不复刻 provider 知识。经 `cargo xtask contracts` 生成，
     `--check` 一致。
  3. **T15 P3① 选择"文案/入口如实"而不是放宽预算门槛**：允许"无外呼的本地阶段"免预算重试会改变
     REQ-023/AC-040 的已验收语义（"重试会再次发起请求 → 预留必须仍占预算"），属 PM 层面的语义变更；
     RD 不做。改为：①详情按 `retry.allowed` 决定是否渲染入口、被拒时显示真实原因与"重新报价/新建任务"；
     ②`drafts/service.rs` 的缺项文案不再承诺可重试（改为"请在任务中心查看该阶段可用的恢复动作
     （重试需要该分支仍有预算背书）"）。**遗留候选增强**：若 PM 认为本地校验/下载类阶段应免预算重试，
     需新裁定并重开 AC-040 的验收。
  4. **轮询：可见性驱动而非 react-query 的焦点语义**：`refetchIntervalInBackground: true` +
     `useDocumentVisible()`（2s/15s）；若按默认（false），窗口失焦时 interval 会**完全暂停**，
     那样"后台 15s"会退化成"后台不轮询"，与 UI-029 的降频语义不符。终态（succeeded/failed/cancelled）
     返回 `false` 停止；列表的停止条件是"已加载行全部终态"（单个终态行不会让整页停轮询，符合预期）。
  5. **断网模拟用路由 abort 而不是 `context.setOffline`**：`setOffline` 会同时掐断 Vite 的模块/HMR
     连接（本套 e2e 的页面来自 Vite dev server），观察点会被污染；用 `route.abort('internetdisconnected')`
     精确模拟"业务请求网络失败"（fetch 抛 TypeError → UI-033 的网络提示）。
  6. **e2e 自管后端（`job-recovery-harness.ts`）**：共享 `globalSetup` 后端无法被用例重启，
     且需要把 provider 指向本机 fixture 才能产生 needs_input/submission_unknown 与"付费计数"。
     取舍：新增一个 harness 文件（fixture + `init`/`serve` 进程 + 造数），浏览器侧用 `page.route`
     把 `/api/v1/**` 改写指到该后端。两个必须知道的约束：
     (a) 该后端必须声明 `public_origin = http://127.0.0.1:<E2E_WEB_PORT>`——Origin 校验按
     `public_origin` 或请求 Host 同源判定，路由改写后 Host 与浏览器 Origin 不同源；
     (b) 后端用**测试构建**（`--features job-failpoints`）才能放行"明文 http + 回环"的模型下载
     （T13 的两道门），发布构建会按策略拒绝（`InsecureScheme`/`ForbiddenAddress` → needs_input）。
     另外 CSRF token 与**会话 cookie 绑定**：造数函数每次重新登录后必须重新取 token（否则 403）。
- 影响的 REQ／任务／模块：REQ-023/025/026/031（UI 呈现）、AC-049、AC-033/034（UI 侧）、AC-037/039/040（UI 侧）；
  `crates/server/src/jobs/control.rs`、`crates/server/src/http/{jobs.rs,dto/jobs.rs}`、
  `crates/server/src/drafts/service.rs`、`crates/core/src/jobs.rs`、`apps/web/src/features/jobs/**`、
  `apps/web/tests/e2e/job-recovery*.ts`。
- 未验证边界与下一步：真实供应商账户下的对账三动作（T23）；"本地阶段免预算重试"若采纳需 PM 新裁定；
  顶栏"进行中任务计数徽标"未做（全局轮询取舍，待 PM 决定间隔与预算）；列表状态筛选目前是客户端筛选
  （服务端 `GET /jobs` 只支持 `itemId`，服务端过滤需合同扩展）。
- 相关PRD／代码／测试／官方来源链接：
  [PRD §6.2 UI-027–UI-039](requirements/web-mvp/prd.md)、
  [implementation §T17](requirements/web-mvp/implementation.md)、
  [`crates/server/src/jobs/control.rs`](../../crates/server/src/jobs/control.rs)、
  [`apps/web/src/features/jobs/`](../../apps/web/src/features/jobs/)、
  [react-query refetchIntervalInBackground](https://tanstack.com/query/latest/docs/framework/react/reference/useQuery)。

### T17 验收知识（追加，2026-09-12；QA 回合 21 结果 PASS，无新增缺陷，4 条非阻断观察）

- 状态：2026-09-12 回合 21 已执行。AC-049 全部子句、AC-033/AC-034 的 **UI 侧**、§6.2 UI-027–UI-039 与
  **T15 P3① 复验**经 **QA 自写 Playwright 独立验收** `apps/web/tests/e2e/qa-t17-independent.spec.ts`
  （8 用例：金额逐字段与 API 对账、轮询按请求时间戳测间隔、只掐 GET 制造陈旧 ETag、断网期间在服务端改状态再看
  恢复、unknown 无重试 + 对账、失败与 unknown 的区分）+ QA 重跑 RD 套件 **8 passed** +
  全量 e2e **52 passed / 0 failed**（qa-t16 的 12 条含本回合事实更新）+ vitest **79** +
  workspace **477 passed / 0 failed / 3 ignored** + `xtask check` **7/7** + contracts 两份 `[一致]` +
  QA 独立 dist `f08444b9…`（22 627 728 B）+ `smoke-bootstrap` 全过；lsof 采样非回环 ESTABLISHED **0**；
  证据：`artifacts/web-mvp/t17-qa/**`、[qa-report 回合 21](requirements/web-mvp/qa-report.md)。
- 结论/手法（跨卡复用）：
  1. **headless Chromium 里 `page.bringToFront()` 不会产生 `visibilityState=hidden`**（QA 探针实测
     两个标签页都是 `visible`，`artifacts/web-mvp/t17-qa/r21-visibility-probe.log`）。验证"后台降频"
     只能用 `Object.defineProperty(document,'visibilityState',…)` + `visibilitychange`（与应用读取同一
     API 面）；**真实浏览器后台节流仍属未覆盖边界**，不要把它写成已验证。
  2. **任务中心轮询的可靠测法是"按请求时间戳算间隔"，终态停止要在"只含终态任务的列表/后端"上测**：
     共享后端里其他用例残留的非终态行会让列表继续轮询（RD §T17-5 已记录）。后台判别用
     "转后台后前 10 秒 0 次请求"（2 秒轮询会给约 5 次）比"17 秒内 ≤3 次"更锐。
  3. **状态瞬态陷阱**：拒答等零延迟 fixture 下 `needs_input → queued → running → needs_input` 的
     循环**快于 250ms 轮询**，不能把"离开 needs_input"当稳定中间态断言；要验证重试生效，先切换
     fixture 脚本（成功）再点按钮，然后等 **succeeded**。
  4. **付费提交基线必须在建单之后采集**：`seedJob` 自身会产生 1 次 Tripo 提交；"重复操作不多收费"
     的断言基线若在建单前采集会把建单计入差值（QA 本回合首跑因此假 FAIL）。
  5. **同一页面上下文里已登录时不要再走登录表单**（`getByLabel("密码")` 会等到测试超时）；跨路由
     复用会话直接 `page.goto`。
  6. **unknown 的"不填 0/不释放"有 schema + 业务两层保证**：`cost_ledger` 的
     `CHECK (state <> 'unknown' OR actual IS NULL)` 与 `recordNoTask` 的 `reservationReleased:false`。
     独立留证方式：运行期用 `sqlite3 -readonly <data-dir>/manual.sqlite3 "SELECT provider,currency,reserved,actual,state FROM cost_ledger"`
     （QA `r21-cost-ledger-dump.txt` 实测 `tripo|3000|NULL|unknown`）。
  7. **AC-034 的"不自动降质量/换模型/加阶段"要用控件角色断言**：文案里合法地存在"应用不会自动降质量、
     换模型或增加处理阶段"，因此 `getByRole("button"|"link"|"radio", {name:/降质量|换模型|增加阶段|自动修复|一键重试|立即重试/})`
     计数 0 才可靠，纯文本断言会被说明文字误伤。
  8. **e2e 设施两条硬约束**（`job-recovery-harness.ts`，复核 RD ADR-028 第 6 条）：自管后端必须声明
     `public_origin = http://127.0.0.1:<E2E_WEB_PORT>`（路由改写后 Host 与浏览器 Origin 不同源，否则 403）；
     且必须用 `--features job-failpoints` 的**测试构建**才放行"明文 http + 回环"的模型下载。
     另：`global-setup` 的 `waitForHealth` 只轮询就绪探针，**18080 有上次残留后端时会静默对着旧
     data-dir 跑整套用例**（RD §T17-10 第 8 条）——QA 每次全量前先核对 `lsof -nP -iTCP -sTCP:LISTEN`。
  9. **产物与工作树的一致性要现场核对**：RD 记录的 T17 dist 哈希（`3262691d…`，18:13Z）早于 18:22–18:23
     的 5 个前端源文件收尾修改；QA 用最终工作树重建 dist 并用 `grep -a <中文文案> <binary>` 确认内嵌了
     最终文案。后续卡收尾改动后必须重跑 dist/smoke 再记录哈希。
- 未关闭的观察（非缺陷，供 PM／后续卡）：
  1. **P3-1**：UI-032 的字面「第 n/5 次重试，将在 X 秒后重试 / 实时倒计时」未实现——界面为状态标签
     「退避重试中」+「尝试 N 次」+「下次运行约 <绝对时间>」（发布包 bundle 无 `秒后重试`/`次重试`）。
     AC-036 的必选可观察项（状态转换/退避序列/上限/Retry-After）在 `cargo test` 侧已通过；是否要求
     字面倒计时需 PM 裁定（不阻断 T17）。
  2. **P3-2**：PRD §6.1.1 的顶栏「进行中任务计数徽标」未实现（RD §T17-10 第 1 条已披露，无独立 UI ID）。
  3. **P3-3**：`retry_wait` 与 Manual AI `submission_unknown` 的浏览器呈现本回合**未做端到端**
     （fixture 无对应脚本），只在字段/组件层覆盖。
  4. **P3-4**：证据目录共享互相覆盖（沿用回合 9 的 P3-2）：本轮多次运行覆盖了
     `artifacts/web-mvp/t17-rd/screenshots/**` 与 `t09-rd/playwright-output/**`。
- 相关代码／测试：`apps/web/src/features/jobs/**`、`apps/web/tests/e2e/{qa-t17-independent.spec.ts,job-recovery*.ts}`、
  `crates/server/src/{jobs/control.rs,http/jobs.rs,http/dto/jobs.rs,drafts/service.rs}`、`migrations/0001_core_schema.sql`；
  [PRD §6.2 UI-027–UI-039](requirements/web-mvp/prd.md)、[contracts §5](contracts.md#5-任务阶段与崩溃语义)、
  [ADR-028](#adr-028--t17-任务中心与可行动错误落地取舍retry-准入单一判据追加式-dtop3①-改文案而非放宽门槛可见性驱动轮询e2e-自管后端)、
  [qa-report 回合 21](requirements/web-mvp/qa-report.md)。

## ADR-029 — T18 GLB 阅读器与资源恢复落地取舍（版本兼容、asset-root 坐标语义、上下文恢复状态机、显式资源账本、e2e 数据边界）

- 状态：verified（2026-09-12，T18 实测：`npm --prefix apps/web run test -- --run src/features/viewer/coordinates.test.ts`
  **15 通过**、`npm --prefix apps/web run test:e2e -- viewer.spec.ts` **8 通过**、
  全量 `npm --prefix apps/web run test:e2e` **60 通过**、`npm --prefix apps/web run test -- --run` **111 通过**、
  `cargo test --workspace` **477 passed / 0 failed**、`cargo xtask check` **7/7**、
  `cargo xtask contracts --check` 两份 `[一致]`、`cargo xtask dist` `6665f5a56c41ab6ea3cbb82489a0a268c5377f87489684a729bebeb6a81e796f`
  + `smoke-bootstrap` 全过；真实 Chrome 100k 面模型实测 p95 帧耗时 **18.6 ms**（目标 ≤33 ms）；
  证据见 `artifacts/web-mvp/t18-rd/`。）
- 日期／作者角色：2026-09-12 · RD（T18）。
- 背景与证据：PRD 修订 2（ui_revision 2）REQ-032（主）/REQ-036/038/039/040、AC-050/AC-051/AC-062、
  §6.2 UI-043/UI-044/UI-045/UI-059、§6.3（"3D 是增强，文字是主路径"）；architecture §3（固定版本、
  3D 模块懒加载）、§5.4（资产版本不可变、asset-root 局部坐标、显示变换放外层、法线非均匀缩放、
  raycast 排除热点自身）；contracts §2（Hotspot/CameraPose 数值有限、身份用 revision+sha）、§7
  （GLB 自包含、贴图预算）；validation-release §3/§4（3D 行矩阵、性能基线、headless 不作达标判据）；
  ADR-023（T13：`validated` 才可进阅读器、GLB 检查口径）。
- 结论与原因：
  1. **版本组合：`three@0.186.0` + `@react-three/fiber@9.4.2`（React 19.3.0 下唯一合规的 9.x）**。
     R3F 9.5+ 的 peerDependencies 是 `react >=19 <19.3`，本项目 React 是 19.3.0；用
     `--legacy-peer-deps` 强装会把版本约束变成谎话，且无法保证运行时兼容。代价：R3F 9.4.2 内部
     仍用 three 的 `Clock`（0.186 起标记 deprecated，控制台一条 warn，无功能影响）。
     `@types/three@0.186.0` 只作 devDependency（three 自身不发布类型）。
  2. **asset-root 局部坐标 = `gltf.scene`（GLB 场景根）的局部坐标**；fit 的居中/等比例缩放在
     **外层 display group**，从不写进 asset-root 自身变换，也不烘焙进顶点。读写同源：写用
     `assetRoot.worldToLocal`、读用 `localToWorld`（§5.4 的原始要求）。锚点身份只来自
     `modelRevisionId + modelSha256`，**不用 `mesh.uuid`／节点名**（重新加载/生成后必变）。
  3. **坐标层拆成"纯数学 + three 桥"两个模块**：`coordinates.ts` 不 import three（可在 Node/Vitest
     直接验证，且 T19 保存视角/校验锚点时可复用同一套语义），`asset-root.ts` 才接触 `Object3D`。
     理由：three 属于懒加载 chunk，坐标语义不该被它绑死；测试也无需 WebGL。
  4. **法线用逆转置 `R·S⁻¹`**（`transformNormal`），方向用 `R·S`，世界→局部方向用 `S⁻¹·Rᵀ`
     （相机 `up` 属于世界量）。测试固化"把法线当普通方向量"在非均匀缩放下**不垂直**的反例，
     防止实现退化。
  5. **上下文丢失走真实事件，不做"刷新按钮"**：监听 `webglcontextlost`/`webglcontextrestored`
     且必须 `preventDefault()`（不调用则浏览器不再派发 restored）；丢失期间禁用 OrbitControls、
     状态行 `role="status"` 提示；**「立即重建」= 重新挂载 Canvas（新上下文 + 重新解析模型）**；
     恢复后 three 的 `initGLContext()` 重建 GL 侧缓存，相机对象不变 → 位姿自然保留（e2e 断言）。
     丢失 8s 无 restored 才显示"浏览器 3D 上下文不可用"+ 文字路径（UI-044 的连续失败分支）。
     帧计数只在未丢失时累加，使"恢复后继续渲染"成为可断言事实。
     **订正与补充（2026-09-12，BUG-007 修复回合；T19 复用状态机时按此）**：
     - 面板状态**不能只由 canvas 事件驱动**：新挂载的 `StageScene` 必须在挂载时上报一次 `ok`
       （首次挂载是无操作；「立即重建」换新 Canvas 时这一步把面板从"正在重建…"收敛回可用态）。
       否则 `contextState` 永久停在 `lost`、`interactive` 永久 false、控件永久 `disabled`——
       这正是 BUG-007 的第一个症状。
     - `unavailableTimer`（8s 兜底判定）必须在收到 `ok` 时**清除**、在进入 `lost` 与「立即重建」
       时**重启**：清除避免把已恢复的显示误报为"上下文不可用"；重启保证重建失败时仍给出
       "不可用 + 文字路径"而不是静默卡住。手动重建期间面板进入 `restoring`（文案与禁用语义
       同 `lost`，且不显示"立即重建"入口，避免重复重建）。
     - **手动重建的位姿保留是显式实现**（不是自动分支的"相机对象不变"）：面板在点击重建时
       捕获 `CameraPose`（asset-root 局部坐标），新舞台在模型就绪的 fit **之后**套用
       （`restorePoseRef` + `applyPose`，读取/写入同一对变换）；`reset` 仍回到默认初始取景。
       断言教训：重建类路径的 e2e 必须查**状态文案 + 控件可用性 + 8s 后的稳定性**，只查
       帧数/模型存活会漏掉 BUG-007（RD 原用例 3 即因此绿灯）。
  6. **用显式资源账本而不是 `renderer.info`**：账本的收集口径与释放集合同源（遍历场景图收集
     geometry/material/材质属性上的 texture），`created/disposed/alive` 严格对应、`dispose()` 幂等。
     `renderer.info.memory` 只反映"当前 GPU 上传"，无法证明"我加载的那份被释放"。另外 R3F 明确
     不释放 `<primitive>` 内的对象（其源码注释 "Never dispose of primitives"），所以释放责任在
     本卡代码：加载 effect 的 cleanup 对每一份加载结果 `dispose()`（覆盖换模型/卸载/重建/StrictMode 双调）。
  7. **只读可观测桥 `window.__EM_VIEWER__`**（帧数/位姿/视口/锚点投影/往返误差/资源账本/上下文状态）：
     QA 需要的证据（资源是否释放、锚点是否漂移、恢复后是否继续渲染）无法从 DOM 稳定读出；
     桥只暴露读操作，没有修改相机/写入锚点/强制丢失上下文。e2e 的"丢失上下文"用浏览器
     `WEBGL_lose_context`（与 three 的 `forceContextLoss()` 同一条 API）走真实路径，不经过桥。
  8. **浏览器侧纵深防御**：解析前做自包含检查（拒绝带 `uri` 的 buffer/image，含 `data:`，
     拒绝非空 `extensionsRequired`）。服务端 T13 已校验一遍，但浏览器若照单加载外链资源，
     就违反"模型只作资料资产、不执行脚本/不请求外部地址"的边界。
  9. **e2e 数据来源边界（订正 2026-09-12，理由：QA 回合 22 实测推翻原表述；见下）**：原文写
     "服务端在 T18 尚无'模型版本 → 资产'端点，因此 `viewer.spec.ts` 只在浏览器侧提供两处合同形态
     响应"。**实测事实**：`GET /assets/{id}/content`（T06）与 `GET /items/{id}/drafts/{draftId}`
     （T15）**均已可用且无测试构建门控**，浏览器可全程消费真实草稿 DTO 与真实资产字节；唯一需要
     "测试构建（`--features job-failpoints`）+ 显式 fixture 配置"的环节是**造出**带 validated 模型
     的草稿（本机 fixture 供应商走真实流水线）。因此：真实链路用例（`viewer.spec.ts` 用例 9 的
     `installRealBackendRouting`、QA 的 `qa-t18-independent.spec.ts`）为**首选**，断言 `route.fulfill`
     计数为 0；浏览器侧拦截（`installViewerRoutes`）**仅保留给真实链路无法构造的注入场景**
     （T19 前的 stale 热点、500 故障注入）。会话/物品/document/preparation/页资产/**原 PDF 字节**
     在所有用例中都是真实后端。AC-050/AC-051 的断言点建立在真实浏览器 + 真实后端数据之上。
  10. **e2e 内的"换模型"用 SPA 内路由跳转（`pushState` + `popstate`）而不是 `page.goto`**：
      `goto` 会重建 JS 上下文、账本归零，从而无法证明"旧模型被释放"（只能证明"新页面没有旧资源"）。
  11. **性能只在真实设备上判**：headless Chromium（SwiftShader 软件光栅）不作达标依据
      （validation-release §4）。测量脚本用 `channel=chrome` + headed + 真实 GPU + `embedded-ui`
      单二进制（生产路径），记录设备/浏览器/GPU/模型哈希；e2e 只断言"能渲染/能恢复/资源不增长/降级可用"。
- 影响的 REQ／任务／模块：REQ-032/036/038/039/040（前端读取侧）、AC-050/AC-051/AC-060 前端侧/AC-062；
  `apps/web/src/features/viewer/**`、`apps/web/src/api/{client,endpoints}.ts`、`apps/web/src/App.tsx`、
  `apps/web/tests/e2e/{viewer.spec.ts,viewer-harness.ts,fixtures/**}`、
  `artifacts/web-mvp/t18-rd/**`；**T19**（复用坐标层与桥：热点绑定/视角保存/stale 判定、release 阅读器
  换真实端点、热点标记视觉与 raycast 排除规则）、**T21/T22**（窄屏与浏览器矩阵、单二进制内 three chunk 的
  资源路径）。
- 未验证边界与下一步：真实 Tripo GLB 的材质/贴图复杂度下的帧耗时（T23 用真实产物复核）；Safari 需实机
  （PRD §5.6）；真实 GPU 驱动复位后浏览器自动 `restored` 的时延；T19 一旦确定热点落库位置，
  `draft-view.ts` 的字段路径需同步（读取器已按 contracts §2 字段名宽容读取）。
- 相关代码／测试／官方来源链接：
  [`coordinates.ts`](../../apps/web/src/features/viewer/coordinates.ts)、
  [`asset-root.ts`](../../apps/web/src/features/viewer/asset-root.ts)、
  [`ViewerStage.tsx`](../../apps/web/src/features/viewer/ViewerStage.tsx)、
  [`resources.ts`](../../apps/web/src/features/viewer/resources.ts)、
  [`bridge.ts`](../../apps/web/src/features/viewer/bridge.ts)、
  [`viewer.spec.ts`](../../apps/web/tests/e2e/viewer.spec.ts)、
  [`measure-viewer-perf.mjs`](../../artifacts/web-mvp/t18-rd/measure-viewer-perf.mjs)、
  [implementation §T18](requirements/web-mvp/implementation.md)、
  [PRD §6.2 UI-043–UI-045](requirements/web-mvp/prd.md#62-交互合同)、
  [architecture §5.4](architecture.md#54-3d-和热点)、
  [three WebGLRenderer 上下文恢复](https://github.com/mrdoob/three.js/blob/dev/src/renderers/WebGLRenderer.js)、
  [WEBGL_lose_context 扩展](https://registry.khronos.org/webgl/extensions/WEBGL_lose_context/)、
  [React Three Fiber](https://r3f.docs.pmnd.rs/)。

### T18 验收知识（追加，2026-09-12；QA 回合 22 结果 **FAIL**，1 条新增缺陷 BUG-007（P2）+ 1 条非阻断观察 N1）

- 状态：本小节是 **QA 验收手法与限制**（非代码知识），供 T19/T20/T21 复用；ADR-029 的实现结论未变，仅其第 9 条的"无真实端点"表述被本回合实测推翻（**已于同日 RD 修复回合订正**：见第 9 条与 `implementation.md` §T18-8；T18 的真实链路用例见 `viewer.spec.ts` 用例 9）。
- 复用价值最高的 5 条：
  1. **真实链路 e2e 手法（T19/T20 直接用）**：`cargo build -p everything-manual --features job-failpoints`（测试构建）+ 显式测试配置（provider `base_url` 指本机 fixture、`[download] allowed_hosts=["127.0.0.1"] allow_local_fixture=true`、`public_origin=http://127.0.0.1:<页面端口>`）+ 浏览器把 `${WEB_PAGE}/api/v1/**` 改写源站到测试后端 ⇒ 浏览器可消费**真实草稿 DTO 与真实 `GET /assets/{id}/content` 字节**（模型分支真实下载 fixture GLB → validated → 落库）。可用性实测：`GET /assets/{id}/content`（T06）与 `GET /items/{id}/drafts/{draftId}`（T15）**均无测试构建门控**，正常构建即可读；只有"造草稿"需要 fixture 供应商。样例：`apps/web/tests/e2e/qa-t18-independent.spec.ts`；踩坑：后端必须声明 `public_origin`，否则改写后的修改请求被 403。
  2. **独立 WebGL 资源计数必须包在原型上**（`WebGL2RenderingContext.prototype`），不能包在上下文实例上：SPA 换路由/「立即重建」会换新上下文，实例包装在第一次切换后**冻结**，会给"常驻资源恒定"的**假通过**。已知噪声：three 每个渲染器实例自建 4 张 empty texture（`WebGLState`：TEXTURE_2D/CUBE_MAP/2D_ARRAY/3D），因此累计 `createTexture` 随上下文数增长**不是**泄漏判据；应改用「每轮真实 `deleteBuffer/deleteTexture` > 0 + 每次卸载 `webglcontextlost` + 账本 alive 恒定 + 堆趋势」。
  3. **堆趋势用 CDP `Performance.getMetrics` 的 `JSHeapUsedSize`**：`performance.memory.usedJSHeapSize` 可能连续 10 次给出同一数值（RD 记录 42.1 MB 恒定，量化/GC 时机所致），不适合做趋势观测（本回合 QA 采样 24.5–33.1 MB、首末比 1.05）。
  4. **`coordinates.composeTransforms` 只允许等比例的外层 display 缩放**（非均匀父级产生剪切：逐分量组合 ≠ 真实矩阵链）。写测试时若用非均匀 display 会得到假失败；生产 `ViewerStage` 用 `scale.setScalar`，符合契约。
  5. **重建类降级路径的断言教训（BUG-007 根因的泛化）**：凡是"重挂载/重建"的恢复路径，UI 断言不能只查帧数/模型存活，必须查**状态文案与控件可用性**。BUG-007 根因摘要：`ViewerPanel.contextState` 仅由 stage 的 `webglcontextlost/restored` 回调更新，`rebuild()` 重挂载 Canvas 后新 `StageScene` **不上报 `ok`** ⇒ `contextState` 永久 `lost`、`interactive` 永久 false；丢失时启动的 8 s `unavailableTimer` 在重建时也未清除 ⇒ 8 s 后升级为「浏览器 3D 上下文不可用」。RD 的 `viewer.spec.ts` 用例 3 因只查帧数/存活而绿灯——这是"绿灯不等于通过"的现场例子。
- 其它已核实事实：真实草稿的 `knowledge` 外壳为 `{schemaVersion, sourceJobId, completeness, model, knowledge, missing}`，**没有 `hotspots`**（T19 才落库）；`model.bounds` 由服务端给出 `{min,max,triangles,vertices}`（阅读器的 `localBounds()` 应与之一致）；`knowledge.hotspots[]` 相关断言在 T19 前只能靠合成 DTO。
- 影响的 REQ／任务／模块：T19（release 阅读端点 + 热点落库后的真实链路用例）、T20/T21（真实链路手法与观测口径）、`apps/web/src/features/viewer/**`。
- 相关代码／测试／证据链接：[`qa-t18-independent.spec.ts`](../../apps/web/tests/e2e/qa-t18-independent.spec.ts)、[`qa-t18-independent.test.ts`](../../apps/web/src/features/viewer/qa-t18-independent.test.ts)、[`r22-resource-trend.json`](../../artifacts/web-mvp/t18-qa/r22-resource-trend.json)、[`r22-rebuild-state.json`](../../artifacts/web-mvp/t18-qa/r22-rebuild-state.json)、[QA 回合 22 报告](requirements/web-mvp/qa-report.md)。

### T18 验收知识（追加，2026-09-12；QA 回合 23：BUG-007 复验 PASS → CLOSED）

- 状态：本小节是 **QA 验收手法与限制**（非代码知识），供 T19/T20/T21 复用。BUG-007 的独立复验结论、N1 处理核对与 1280px 调查见 [QA 回合 23 报告](requirements/web-mvp/qa-report.md)。
- 复用价值最高（回合 23 新增）：
  1. **手动重建的完整语义清单（T19 复用状态机时按此断言）**：① 状态行回「模型已加载」；②「复位视角/适配模型」enabled；③ **等待 ≥8.6 秒后仍不得出现"上下文不可用"**（8 s 兜底计时的清理是 BUG-007 的第二个症状，只看短窗口抓不到）；④「立即重建」入口从 DOM 消失；⑤ 重建后拖动仍可旋转且**独立 draw 计数继续增长**；⑥ 位姿策略：重建后 ≈ 重建前（实测差 ~1e-15；1e-3 断言余量充足）、「复位视角」回默认取景。
  2. **Playwright `fullPage` 截图会把模拟视口瞬时改成 1×1**（实测：`innerWidth/clientWidth/visualViewport` 全为 1，`(min-width:1279/1269px)` 同步翻转，恢复后再变回）。后果：`PageLayout` 的 wide/mid/narrow 是三棵不同的树 → 截图期间**整页（含 3D Canvas 与原文面板）卸载并重挂载，模型重新加载、相机复位**。因此：**不要在同一用例的测量窗口前做 fullPage 截图**；截图后如需继续测量，先等面板回到可用态再取数。这是回合 22 报告中"1280px 断点边界整页重挂载"的**真实机制**——与 1280 断点无关（1440×900 同样复现，RD 原机理描述已在 §T18-13/§T18-8 订正）；`scrollbar-gutter` 一类方案不能解决该问题。
  3. **NaN 会骗过否定式断言**：`Math.abs(NaN)` / `not.toBeCloseTo` 在"位姿暂不可读"时会假通过。取位姿/距离必须先等有限值（`Number.isFinite` 轮询，见 `viewer.spec.ts` 的 `finiteDistance()`），本回合 QA 新用例同样采用该手法。
  4. **真实链路"零伪造"证据的强度分层**：`route.fulfill` 计数只有在**真的可能被调用**时才是证据（本仓库 `installRealBackendRouting` 内没有任何 `route.fulfill`，其 `fulfilled` 计数恒 0 = 守卫而非证明）。更强的证据是：`assetRequests` 命中 `/assets/{id}/content` 真实端点 + 模型 `sha256` == 仓库 fixture GLB 的真实哈希 + 草稿/部件/步骤文案来自真实流水线 + `external == []`。写 T19 真实链路用例时按此分层取证。
- 未覆盖边界（回合 23 记录）：占宽（经典）滚动条环境无法在本机 Playwright 复现（本环境滚动条为 overlay，`::-webkit-scrollbar { width }` 不占宽）——"真实桌面滚动条出现/消失是否影响断点"未实测，交 T21 浏览器矩阵。
- 相关代码／测试／证据链接：[`qa-t18-r23-manual-rebuild.spec.ts`](../../apps/web/tests/e2e/qa-t18-r23-manual-rebuild.spec.ts)、[`qa-t18-r23-layout-boundary.spec.ts`](../../apps/web/tests/e2e/qa-t18-r23-layout-boundary.spec.ts)、[`r23-manual-rebuild-state.json`](../../artifacts/web-mvp/t18-qa-r23/r23-manual-rebuild-state.json)、[`r23-layout-1440.json`](../../artifacts/web-mvp/t18-qa-r23/r23-layout-1440.json)、[`r23-layout-control.json`](../../artifacts/web-mvp/t18-qa-r23/r23-layout-control.json)、[QA 回合 23 报告](requirements/web-mvp/qa-report.md)。

## 新增记录模板

```text
ADR-xxx — 简短结论
状态：proposed / accepted（未实现）/ verified / superseded-by-xxx
日期／作者角色：
背景与证据：
结论与原因：
影响的REQ／任务／模块：
未验证边界与下一步：
相关PRD／代码／测试／官方来源链接：
```

## ADR-030 — T19 校准、复核与发布落地取舍（聚合写入形状、覆盖层与快照分离、发布事务与不变量、stale 继承、迁移重建表）

- 状态：accepted（T19 已实现并通过 `cargo test --test publishing`、`manual-review.spec.ts`、全量回归；QA 复验中）；日期：2026-09-12。
- 依据：PRD 修订 2 / ui_revision 2 的 REQ-033/034/035/036 与 AC-052–AC-057；[contracts.md](contracts.md) §2/§3/§7；ADR-005（无自动发布路径）；ADR-009（迁移只追加）；任务卡 T19。

**结论（10 条落地取舍）：**

1. **受限字段 PATCH 的形状**：`PATCH /items/{id}/drafts/{draftId}` 接受
   `status/hotspots/stepPoses/clearStepPoses/entities/modelReview`（`deny_unknown_fields`）。
   热点与步骤视角写进 `knowledge_json` 外壳（T19 追加字段，`serde(default)` 读旧草稿）；
   实体复核与 modelReview 写进 `review_json` 覆盖层。为什么这样分：热点/视角是"发布内容"
   （要进 manifest 并被 3D 消费），复核是"人的声明"（要能整体清空而不动内容）。
2. **供应商事实快照不可改**：PATCH 没有直改 `knowledge.knowledge` 的入口（未知字段 422）；
   人工修订进覆盖层 `entities[].userEdited{...}` 并保留 `editedAt/editedBy`，原文本与出处
   留在快照里供对照（UI-051）。服务端在**内容实际变化时**才盖时间戳——重复提交同一内容
   保持幂等（不递增 revision），维持 T15 以来的语义。
3. **`knowledge_json` 外壳版本仍写 `manual_draft_v1`**：T19 的字段是向后兼容的追加
   （旧读者忽略未知字段、新读者 `serde(default)` 读旧数据），递增字符串不会增加保护，
   却会让 T15 已验收的 QA 断言失效。读取器对两个版本字符串都接受。
4. **stale 是"继承 + 规整"，不是"静默复用"**：组装新草稿时继承上一份（同快照优先，否则
   同物品最近一份）的热点，但只继承部件仍存在的；`normalize_stale_hotspots` 把 anchor 与
   当前模型 revision+sha 不一致的 confirmed/candidate 一律降级 `stale`（anchor 保留作解释）。
   部件消失的旧绑定丢弃并在 `missing[]` 报告；步骤视角只在模型身份完全一致时继承
   （视角是几何量，换模型必须重新保存）。API 拒绝任何旧 sha 的 confirmed/candidate 提交。
5. **发布是唯一写 `manual_releases` 的路径**（`crate::releases`）：显式路由 + If-Match +
   Idempotency-Key。发布**给草稿递增一次 revision**：发布是聚合根上的显式操作（contracts §1
   把发布与 PATCH 并列），递增后并发发布/发布前编辑竞态的第二个请求必然 412（AC-056）。
   响应用 `draftRevisionAfterPublish` 给出新 revision（release 不可变，不返回 ETag）。
6. **发布不变量的判据是纯函数**（`releases/invariants.rs`）：输入冻结草稿聚合 + 覆盖层 +
   冻结输入事实（preparation 页数/文档），输出稳定 `code` 的逐条问题（422 `details.issues`）。
   前端 `review-state.publishChecklist` 是同一判据的**镜像**（按钮禁用 + 原因常驻），
   服务端始终是唯一权威；界面把服务端 issues 原样渲染（不做二次改写）。
7. **manifest 是不可变内容寻址资产**（`schemaVersion=manual_release_v1`，用途
   `release_manifest`）：冻结草稿聚合（knowledge/hotspots/stepPoses/missing）+ 复核覆盖层 +
   资产清单（模型/原件 sha256 与来源）+ 文档引用 + 计数。release 只引用它和
   `model_revision_id`，不引用会变的 draft 行——"发布后改 draft 不改 release"由此成立。
8. **`assets.purpose` 的枚举扩展必须重建表**：SQLite 不能原地改 CHECK，迁移
   `0007_release_manifest.sql` 用 `-- no-transaction`（sqlx 支持）在事务外
   `PRAGMA foreign_keys=OFF` + 建新表/拷行/删旧表/重命名/重建索引 + 恢复 PRAGMA
   （SQLite 官方推荐步骤）。这是本卡**唯一**的 schema 变更；附带把既有的
   `schema v6`/迁移计数断言更新为 7（纯事实）。
9. **modelReview 是用户声明、服务器盖章**：`loaded`/`userConfirmed` 由用户声明，
   `checkedAt/loadedAt/userConfirmedAt` 与模型身份由服务器赋值；`userConfirmed` 不能脱离
   `loaded`；组装内容变化时整条记录清空（换模型必须重新复核）。它**不是**服务端可证明的
   GPU 测试（contracts §2 原文）。
10. **e2e 数据来源与三个稳健化教训**：
    - release 用例必须走**真实链路**（QA 回合 23 门禁）：测试构建 + 本机 fixture 供应商 +
      源站改写（`route.fulfill` 恒为 0）；有状态造数/并发干扰用**独立 API 会话**完成；
    - **CSRF 与会话绑定**：`seedJob` 内部会再次登录（换 cookie），此后必须重新取 token
      （取"当前 cookie jar 那次会话"的），否则 403 CSRF_REJECTED；
    - **视图身份依赖**：3D 面板按 `assetId/revisionId/sha256`（而不是对象引用）重取模型
      字节，否则每次校准写入都会让 Canvas 整体重挂载（丢拾取状态）；
    - **fullPage 截图后整页可能重挂载**（QA N6）：点击目标可能在重挂载窗口消失，交互后用
      客户端路由前进，而不是依赖点击；stale 的"已失效"文案与 `steps-list` 等 T18 已验收的
      文字替代路径 testid 必须保留（否则破坏既有证据链）。

- 影响：T20 导出按 `manual_release_v1` 与 `release_manifest` 用途收集资产；T21 回归沿用
  上述 e2e 稳健化经验；若未来要求"窄屏校准"或"键盘绑定热点"，属新增需求（PM 新修订）。
- 非目标（本卡明确不做）：自动发布路径、自动热点候选（E01）、分件/机械运动（E02）、
  导出/备份（T20）。

### T19 验收知识（追加，2026-09-12/13；QA 回合 24：AC-052–AC-057 全绿 → 切片 PASS）

- 状态：本小节是 **QA 验收手法与限制**（非代码知识），供 T20–T23 复用；ADR-030 的实现结论未变。T19 的 AC 矩阵、命令与原始结果、P3 观察见 [QA 回合 24 报告](requirements/web-mvp/qa-report.md)。
- 复用价值最高（回合 24 新增）：
  1. **“重新生成模型 → stale”的真实链路复现需要一个可切换字节的模型 CDN**：T17 `LocalFixture` 的 `/cdn/model.glb` 每次读同一个 fixture 文件，无法在一次运行里给出两个模型版本。QA 自建 `QaFixture`（`qa-t19-independent.spec.ts`：Tripo v3 + 说明书 AI + `modelBytes` 可变字段，约 120 行；翻转 `sample-model.glb` BIN 起始一个字节即得结构合法、sha 不同的“新模型”），在同一物品上跑两轮真实流水线即可复现 stale/旧发布版不可变。
  2. **画布（3D canvas）交互前必须把它滚进视口**：`expect().toBeVisible()` / `click()` 会把页面滚到断言目标（例如左栏底部的 stale 区块），此后旧的 `boundingBox()` 使画布落在视口外，`page.mouse.click` 会点到别处（现象是“UI 重新绑定没生效”）。统一用“scrollIntoViewIfNeeded + 视口内校验 + 必要时 `window.scrollTo`”换算画布相对坐标。
  3. **窄屏抽屉是覆盖层**：`drawer-overlay` 会拦截主栏（发布面板/刷新草稿/发布按钮）的点击——窄屏用例先 `Esc` 关闭抽屉再做主栏交互；同一时刻只开一个抽屉的约束对同页所有断点都适用。
  4. **`:focus-visible` 只对键盘意图显示焦点环**：`locator.focus()` 之后模态仍是鼠标，`getComputedStyle().outlineStyle` 可能读到 `none`（假失败）。断言可见 focus 环前先用 `Shift+Tab`+`Tab` 制造键盘模态并 `toBeFocused()` 确认。
  5. **raycast 排除热点标记的可判定做法**：放大到热点标记（局部半径 0.035）的屏幕半径 >45px，然后在**标记覆盖内、18px 点选半径外**点击，断言拾取点仍在模型包围盒内（±0.002）且至少一个轴贴面——若射线命中标记球体，命中点会外向越界 ~0.03 局部单位。
  6. **modelReview 是整体状态提交**：`{loaded, userConfirmed}` 两字段都必填（`deny_unknown_fields`）；“只声明 loaded”要发 `{loaded:true,userConfirmed:false}`；`{loaded:false,userConfirmed:true}` 在**没有既有 loaded 声明**时 422（不能脱离 loaded）；客户端自带 `checkedAt` → 422（服务器赋值）。
  7. **stale 热点不会产生 `hotspotNotMatchingModel`**：已有 stale 绑定的部件是通过 `hotspotMissing` 阻断发布（“不得当有效热点用”）；`hotspotNotMatchingModel` 只针对**自称 confirmed/candidate 但 anchor 与当前模型不符**的热点，API 会拒绝这类写入，故只能在测试里篡改注入（RD `publishing.rs` 用例 6）。
- 未关闭观察（P3，非阻断；供 RD/PM/T20/T21）：① `KnowledgeReviewPanel` 把 `**` 原样渲染（DOM 实测）；② 发布 412 后按钮未按 UI-056「刷新前禁用」且冲突面板在「刷新草稿」后不消失（可恢复路径正常，窄屏发布 201 成功）；③ 校准页步骤面板「引用部件」显示内部 id（阅读器显示名称）；④ 修订表单无字段级错误关联（源码复核）。
- 相关代码／测试／证据：[`qa-t19-independent.spec.ts`](../../apps/web/tests/e2e/qa-t19-independent.spec.ts)、`artifacts/web-mvp/t19-qa/`、[QA 回合 24 报告](requirements/web-mvp/qa-report.md)、[ADR-030](#adr-030--t19-校准复核与发布落地取舍聚合写入形状覆盖层与快照分离发布事务与不变量stale-继承迁移重建表)。

### T20 验收知识（追加，2026-09-13；QA 回合 25：AC-009/AC-010/AC-058 与全部卡内项通过 → 切片 PASS；1 个 P3 待 PM 解释）

- 状态：本小节是 **QA 验收手法与限制**（非代码知识），供 T21–T23 复用；ADR-031 的实现结论未变。T20 的 AC 矩阵、退出码实测表、UI-060 裁定与 P3 清单见 [QA 回合 25 报告](requirements/web-mvp/qa-report.md)。
- 复用价值最高（回合 25 新增）：
  1. **“恢复目录字节 == 备份快照字节”的比较必须在恢复后首次启动服务之前做**：备份快照把 journal 模式归一为 `DELETE`（ADR-031 注 2），恢复后首次 `serve` 按合同切回 WAL/FULL → 主文件头被改写，sha256 必然不同（数据不变）。QA 首轮先起过服务再比对，读到 `6c4629…` ≠ manifest `7d61a8…`，属**预期行为**而非恢复错误。验收脚本排序要求：restore → 字节/哈希比对 → 才起服务读 release/PDF/GLB。
  2. **验收“备份不得只复制运行中 WAL 主文件”的最小可执行判据（QA 实跑）**：① 服务运行中通过真实 HTTP 写入一条数据；② `kill -9` 停机（SIGTERM 会 checkpoint 并删除 `-wal`，`kill -9` 才留下已提交事务）；③ **负对照**：把 `manual.sqlite3` 复制到别处并查询（缺这条数据）；④ `backup` 后查快照（含这条数据）；⑤ 源 data-dir 主文件与全部 blob 的前后 sha256 相同。另：**带 `-wal` 的快照在 `restore` 侧被显式拒绝**（`backup_database_sidecar`，exit 7）。
  3. **重启后仍有 `-wal` 的库不能用只读 URI 打开**（`sqlite3 file:…?mode=ro` → `unable to open database file (14)`：只读连接不能创建 `-shm`）。做“主文件副本”对照时要可写打开，或先让 `-shm` 存在。
  4. **手工构造 v1（旧 schema）data-dir，用于任何“升级前备份/自动迁移”类验收**：`sqlite3` 执行 `migrations/0001_core_schema.sql` → 自建 `_sqlx_migrations` 表 → 写 1 行（`checksum = sha384(迁移文件字节)`、`description` 取文件名去 `.sql`、`success=1`、`installed_on` 任意）→ 程序把它识别为 v1；随后 `check` 报“待迁移”+ 升级提示，`backup` **不迁移源库**（manifest 记 `schemaVersion: 1`），`restore` + `serve` 自动迁移并保留数据。比改现有库或依赖 RD 的测试更可控。
  5. **退出码 5“恢复目标被占用”在实现里不可达**（文档订正项）：恢复目标只要有 `lock` 文件就不是“空目录”，`restore` 会先以 4（非空）拒绝；用 python 持 `flock` 的空目录实测仍得 4。fail-safe 无风险，但 §T20-5/`error.rs` 表如写成“可发生场景”需订正。
  6. **AC-010 的“备份内容不含……临时云端 URL”与 DB 保留供应商响应的张力（BUG-008）**：`job_stages.usage_json`（`tripo_poll` 观察事实）在 production 保存供应商签名 URL，备份是 DB 的忠实副本 → 快照内必然出现该字串；快照脱敏会破坏在途任务的恢复/对账依据。QA 另证：**导出包**全项干净（密钥/会话/口令哈希/绝对路径/临时 URL/其它资产字节 0 命中）。该点需 PM 解释（是否并入 T13 既有“签名 URL 留存”P3 的 T23 收敛项）。
  7. **macOS bash 3.2 的变量拼接陷阱（复核 RD §T20-10 第 9 条）**：`"$var（"` 会把全角括号并入变量名（`unbound variable`）；`set -u` 下会让脚本中途退出，容易造成“命令没跑却以为通过”。变量后紧跟中文一律 `${var}`。
- 未关闭观察（非阻断；供 RD/PM/T21/T22）：**BUG-008（P3，OPEN，需 PM 解释，见上）**；OB-1 退出码 5 文档订正；OB-2 `备份中的 blob的 sha256` 文案拼接；OB-3 备份/恢复无空间预检（RD 已记录）；OB-4 GLB 经内容端点仍为 `application/octet-stream`（T06 既定行为）；OB-6 环境中一个 00:18 起的孤儿测试 `serve` 进程（PPID 1、临时 data-dir 已删；提示测试 Harness 中断场景的进程清理门禁）。
- T20 验收资产可复用：`artifacts/web-mvp/t20-qa/`（`qa-phase1/2/3/3b/4/5/6/7/8/9*.log` 与同名脚本、`qa-export-sample.zip`、`work-notes/`）；RD 的样例备份 `artifacts/web-mvp/t20-rd/sample-backup/`（含测试管理员，口令 `test-password-t20-backup`）可作 T22 `smoke` 的合法输入；QA 已用它完成一次完整 restore→serve→读 release/PDF/GLB→export。
- 相关代码／测试／证据：`crates/server/src/backup/**`、`crates/server/tests/backup_restore.rs`、`artifacts/web-mvp/t20-qa/`、[QA 回合 25 报告](requirements/web-mvp/qa-report.md)、ADR-031、ADR-011（退出码/锁）。

## ADR-031 — T20 导出、备份与恢复落地取舍（VACUUM INTO 快照、快照清会话、目录式备份、先校验后写入、自写 STORE ZIP、7 = 完整性校验失败）

- 状态：verified（2026-09-13，T20 实测：`cargo test -p everything-manual --test backup_restore` 7 通过、
  `cargo test --workspace` 511 通过、`cargo xtask check` 7/7、`cargo xtask dist` 两次同哈希
  `b8776b11…` + `smoke-bootstrap` 通过、真实二进制手工演练（备份→恢复→读 release/PDF/GLB→导出）全通过；
  证据见 [implementation](requirements/web-mvp/implementation.md) §T20 与 `artifacts/web-mvp/t20-rd/`；
  **QA 回合 25 独立复核通过**（dist 二进制自建两次同哈希、负例自造、退出码 0–7 实测；见 [QA 回合 25 报告](requirements/web-mvp/qa-report.md) 与本节后的「T20 验收知识」）。
- 日期／作者角色：2026-09-13 · RD（T20）。
- 背景与证据：PRD 修订 2 REQ-005/REQ-037（AC-009/AC-010/AC-058）；contracts §3 与 **§7 最后一条**
  （导出 manifest 元素、不含绝对路径/密钥/会话/临时云端 URL、"当前不顺手开放 ZIP 导入"）；
  architecture §6/§7（停服 + 独占锁快照、不得只复制运行中 WAL 数据库的主文件、恢复到新空目录
  校验后再用、升级前备份、程序回滚≠数据库回滚）；validation-release §2/§5。
- 结论与原因：
  1. **备份快照用 `VACUUM INTO`（只读源连接），不复制 `manual.sqlite3`**：WAL 模式下已提交事务
     可能仍在 `-wal`，复制主文件会丢数据（架构明令禁止）；`VACUUM INTO` 由 SQLite 产生事务一致的
     完整副本，且**目标文件已存在即报错**，天然满足"不覆盖"。只读连接保证备份不 checkpoint、不改
     journal 模式、不改源库一个字节（测试对源库与全部源 blob 做前后 sha256 比对；ADR-012 注 3 的
     "check 只读三态"同源）。**备份不解释 schema**：库比程序新时也允许备份（灾备窗口），
     但**不允许 restore 到比程序新的 schema**（拒绝，退出码 4）。
  2. **快照里清空会话、保留管理员口令哈希**：AC-010/REQ-005 要求备份不含会话；恢复旧备份不应复活
     旧会话。口令哈希必须保留，否则灾备恢复后无法登录、"恢复后同一 release 可读"的验收无法成立。
     实现：`VACUUM INTO` 后在快照连接上 `DELETE FROM sessions` 并把日志模式归一到 `DELETE`
     （快照是单文件、无 `-wal/-shm`；`restore` 对带边车的备份直接拒绝，防"只复制主文件"的假快照）。
  3. **备份是目录而不是单文件**（`manifest.json` + `SHA256SUMS` + `database/manual.sqlite3` +
     `blobs/…`）：`SHA256SUMS` 是 manifest 之外的独立校验清单（覆盖快照、全部 blob 与 manifest
     自身），让标准工具（`shasum -a 256 -c` / `sha256sum -c`）能一键校验整个备份——
     不需要引入压缩/归档依赖、人工可检查、`--out` 的"不存在即通过"语义简单可判定；
     单文件打包（zip/tar）留给将来的"导出便携"增强。输出路径与 data-dir **互相嵌套一律拒绝**
     （避免备份与数据一起丢失）。
  4. **restore 先全量校验、后写入**：manifest 格式（`deny_unknown_fields` + 版本不认识即拒绝）、
     每个相对路径的安全校验（绝对路径/`..`/反斜杠/UNC 一律拒绝——与将来 ZIP 导入的 zip-slip
     验收同口径）、快照与全部 blob 的 sha256/大小、`PRAGMA foreign_key_check`、每个 `blobs` 行
     都必须在 manifest 中登记。任何失败都在**创建目标之前**返回（退出码 7），因此"损坏 blob /
     hash 不符 → 失败并保留现场"的现场就是"目标目录不存在、备份目录未被修改"；I/O 类失败才可能
     留下部分目标，消息明确提示清理后重试（不静默删除现场）。
  5. **导出 ZIP 自己写（STORE 无压缩，不新增依赖）**：条目先扫描一遍得到 CRC32/大小再写 local
     header（不用 data descriptor，最保守的解压工具也能读）、固定 DOS 时间戳（1980-01-01，结构确定）。
     条目标识只由服务端生成（角色 + sha256）；导出前先做路径安全校验。自动化测试用同一模块的独立
     解析器（含 CRC 校验），手工演练再用系统 `unzip -t` 与 `python3 -m zipfile -t` 交叉验证。
     将来若开放导入，必须另立 zip-slip/解压炸弹验收（contracts §7），本卡不提供导入接口。
  6. **导出白名单来自冻结 manifest**：只导出 `assets[]` 中角色为 `model`/`document` 的资产 +
     冻结 `release/manifest.json`（字节原样，sha256 与发布详情一致）+ 导出清单；未知角色直接报
     完整性错误（不静默丢资产）。根清单含 `schemaVersion`/`item`/`release`/`knowledge`/`review`/
     `files[{path,role,assetId,sha256,size,mime,source}]`/`notes`，全部为相对路径与标识；
     不含数据库、会话、密钥、绝对路径、供应商临时 URL。用户填写的 `sourceUrl` 属"来源"保留
     （contracts §7 要求"sha256 和来源"），不是临时云端 URL。包生成到 `<data-dir>/tmp/` 后
     **流式响应**（大 GLB 不进内存），响应结束/断开时删除临时文件。
  7. **退出码 7 的含义从"功能未实现"更新为"备份/恢复完整性校验失败"**（数值不变）：
     `ExitCode::NotImplemented` → `ExitCode::Integrity`。运维/QA 脚本按"非零 = 失败、7 = 数据损坏"
     区分处置；4（路径/前置条件，含备份输出已存在、恢复目标非空、备份 schema 比程序新）与
     5（backup 要求先停服）的范围相应扩展，2/3/6 不变（"未知/错误参数行为不变"由既有用例守护）。
  8. **升级提示落地在 `check`/`serve`/`init` 与备份 manifest notes**：`serve`/`init` 通过新增的
     `Database::open_and_migrate_reporting` 得到 `MigrationReport{from,to,upgraded}`（`from ≥ 1`
     才算升级，避免全新库误报），真的迁移时打印"升级前应先备份；程序回滚不等于数据库回滚（旧程序
     不能打开更新的 schema，只能用迁移前备份恢复到新目录）"；`check` 在"待迁移"时给同样的提示
     （不修改数据）。这是 T20 卡"补升级前备份的文档与提示"的具体交付。
- 影响的 REQ／任务／模块：REQ-005/REQ-037（主）、REQ-003/REQ-004（退出码与 schema 门禁）；
  T21（可把样例备份 `artifacts/web-mvp/t20-rd/sample-backup/` 当回归输入）、T22（`smoke` 的
  "从合法备份恢复"输入与升级/回滚说明）、T23（真实链路不涉及备份）。
- 未验证边界与下一步：ZIP 导入（含 zip-slip/解压炸弹验收）未开放；网页端导出按钮（UI-060 的
  交互部分）未实现（本卡允许文件不含 web 功能代码，端点与生成类型已就绪）；备份/恢复无空间预检
  （磁盘满走 I/O 错误）；`model/gltf-binary` 经内容端点仍以 `application/octet-stream` 提供
  （T06 白名单未含该值，属既有行为，未在本卡改动）；备份不含日志与 `config.toml`（只有数据库与 blob，
  配置属部署侧）。
- 相关链接：`crates/server/src/backup/{mod,error,manifest,files,create,restore,export,zip}.rs`、
  `crates/server/tests/backup_restore.rs`、`crates/server/src/http/releases.rs`、
  `crates/server/src/config/{commands,error,cli}.rs`、`crates/server/src/storage/{db,migrations}.rs`、
  `artifacts/web-mvp/t20-rd/`（日志、演练脚本、导出样例包、样例备份）、
  [implementation §T20](requirements/web-mvp/implementation.md)、ADR-002、ADR-011（退出码/锁）、ADR-012（迁移门禁）。

## ADR-032 — 供应商临时/签名 URL 的脱敏规则与"按 task_id 重查"的恢复语义（BUG-008）

- 状态：implemented（2026-09-13，T20 修复；RD 自验：`--test backup_restore` 9 通过、
  `--test model_assets` 23 通过、`cargo test --workspace` 522 通过 / 0 失败、
  `cargo xtask check` 7/7、dist `688a57b9…`（25 063 520 B）+ `smoke-bootstrap` 通过、dist 级脚本
  `artifacts/web-mvp/t20-rd-fix/bug008-verify.sh` 29/29；**待 QA 回合复验**）。
- 日期／作者角色：2026-09-13 · RD（BUG-008 修复回合）。
- 背景与证据：QA 回合 25 缺陷 **BUG-008（P3，AC-010）**——备份快照忠实复制了 `job_stages.usage_json`
  里 `tripo_poll` 观察到的供应商临时 URL（`output.model_url`；证据 `artifacts/web-mvp/t20-qa/
  qa-phase8-urls.log`、`work-notes/backup-temp-url-row.txt`，同一行也存在于归档样例备份
  `artifacts/web-mvp/t20-rd/sample-backup/database/manual.sqlite3`）；与 T13 起登记的 P3 同源。
  协调者裁定**不放宽 AC-010**（AC-010 字面："导出与备份内容不含密钥、绝对路径、会话、临时云端 URL"），
  按更严格一侧处理=**脱敏**。相关合同/架构：contracts §1（不向用户输出完整供应商签名 URL）、
  §5（链接过期 → 重新查询已知任务取新链接，不重新购买；恢复判据是 task_id）、§6（`output.model_url`）、
  §7 末条（导出不含临时云端 URL）；architecture §5.3（"不把临时供应商 URL 当永久模型地址"）、§8（日志脱敏）。
- 结论与原因：
  1. **临时 URL 只保留"不可逆摘要 + host"，永不落库/出网/进备份**：URL 字符串替换为
     `{"redacted":true,"host":…,"sha256":<sha256(url) 前 16 位>}`（唯一实现点 `crate::redaction`）。
     保留 host 与摘要用于回答"当时看到的是哪个 CDN / 哪一次链接"（任务卡第 1 条明示允许），
     但裸主机名不构成可用链接；判据是 `://`、签名查询串与 URL path。**不放宽也不过度删除**：
     不把 host 一并抹掉，否则诊断信息归零、修复变成"删数据"。
  2. **链接是易失能力，不进入持久化事实**：`tripo_poll` 观察到 URL 时写入进程内
     `EphemeralLinks`（有界 ≤64、按 (jobId,taskId) 键、`Debug` 不打印 URL、不落盘）；
     `model_download` 缓存未命中（重启/崩溃恢复/淘汰）时 `GET /tasks/{id}` **重新查询**取新链接，
     链接过期时同样续签并覆盖缓存——两条路径都**没有**付费 POST（保持 contracts §5 的"不重新购买"）。
     这是"URL 不落库"与"既有恢复语义"兼得的唯一已知做法：不新增表/列、不改 DAG、不改 attempt 语义；
     代价是重启后多一次免费 GET（T23 真实 Provider 时复核 Tripo 的查询频次上限）。
  3. **历史数据不强制迁移**（读写分离）：源 data-dir 不被改写；展示路径读取时脱敏（`http` DTO），
     备份路径在快照连接上过滤（`job_stages`/`provider_attempts` 的**全部文本列**，含非 JSON 文本兜底），
     并把处数写进日志与 CLI 输出（`tempUrlsRedacted`）与 manifest `notes`。理由：备份是灾备，
     历史行里有旧 URL 不该让用户无法备份；作用域只覆盖"供应商事实表"（按合同只存系统事实），
     用户内容（`sourceUrl`、知识 JSON）不在清洗范围内（contracts §7"来源"必须保留，T20-10 第 8 条）。
  4. **导出路径取 fail-closed 校验**（`export_manifest_url_forbidden`）：导出包由白名单字段构成，
     出现 URL 只能说明代码有缺陷 → 拒绝导出而不是发出链接；刻意排除用户内容（`knowledge`/`review`/
     `item`），避免把用户填写的出处链接误判成泄露、也保证"用户内容含 URL 仍能导出"。
     冻结的 `release/manifest.json` 字节不得改动（AC-058 断言其 sha256 与发布时一致），故不做过滤。
  5. **任务详情 DTO 的脱敏实现收敛为一处**：`http::jobs::redact_urls` 改为调用 `crate::redaction`，
     历史行与备份走同一规则（避免"展示一套、备份另一套"再次分叉）。
- 影响的 REQ／任务／模块：AC-010（本项）/AC-058（对照）/REQ-005/REQ-028；T12（poll 事实形状）、
  T13（下载与恢复路径）、T15/T17（任务详情 DTO）、T20（备份/导出）、T23（真实 Provider 复核查询频次）。
  生成物：`contracts/openapi.json`/`generated.ts`（usage 描述更新；同时补齐工作树里已存在但生成物落后的
  `/api/v1/releases/{releaseId}/export` 路由）。
- 未验证边界与下一步：真实 Provider 下 `GET /tasks/{id}` 的频次/配额（T23）；崩溃后"poll 已 success、
  下载恰好未跑"的现场在真实链路的恢复演练（fixture 已覆盖同形场景）；若 PM 未来要求"连 host 也不留"，
  属新修订（需改本 ADR）。
- 相关链接：`crates/server/src/redaction.rs`、`crates/server/src/providers/tripo/{handlers,links}.rs`、
  `crates/server/src/http/jobs.rs`、`crates/server/src/backup/{create,export}.rs`、
  `crates/server/tests/{model_assets,backup_restore,tripo_contract,pipeline}.rs`、
  `artifacts/web-mvp/t20-rd-fix/`、[implementation §T20-13](requirements/web-mvp/implementation.md)、
  [QA 回合 25 BUG-008](requirements/web-mvp/qa-report.md)、ADR-006（不自动重购）、ADR-031（备份语义）。

### BUG-008 复验验收知识（追加，2026-09-13；QA 回合 26：BUG-008 **CLOSED**，T20 切片 PASS 不受影响；复验中新增 BUG-009（P2）+ BUG-010（P4））

- 状态：QA 验收手法、缺陷根因与残余风险（非代码知识），供 T21–T23 复用。缺陷与命令证据见
  [QA 回合 26 报告](requirements/web-mvp/qa-report.md)；ADR-032 的实现口径未变。
- 复用价值最高（回合 26 新增）：
  1. **"签名 URL 是否泄露"必须覆盖失败路径，不能只扫成功路径**。写侧 `tripo_poll.task_usage` 已
     摘要化，但 `model_download` 的**失败消息**是另一条通路：`DownloadError::Transport{detail}`
     直接拼接 `reqwest::Error`，而 reqwest（0.13.5 `src/error.rs:299-302`）在请求错误后追加
     ` for url (<完整 URL，含查询串>)`。该消息经 `download_failure_outcome` →
     `StageOutcome::Retryable/Failed.reason` → `job_stages.last_error` → 任务详情 DTO `lastError`
     （**未脱敏**；只有 `usage` 走 `redact_urls`）与 `model_download_failed` 日志
     （`detail = %error.log_summary()`）→ **BUG-009（P2）**。
  2. **传输失败的三种最小构造，证据等级不同（QA 实做，`crates/server/tests/qa_t20_bug008_independent.rs`）**：
     ① TCP **半关闭截断**（回 200 写一半后 shutdown）→ reqwest 解码错误，消息**不含 URL**；
     ② **连接被拒**（`bind` 后立即释放端口，连一个无监听端口）→ 消息**含**完整签名 URL；
     ③ **收下请求不响应**（本机 TCP 黑洞 + 1s 超时）→ 消息**含**完整签名 URL。
     只测 ① 会得出"无泄露"的错误结论。① 的对照用例在套件里常绿，②③ 为本回合新增的缺陷复现
     （`#[ignore]` 待 RD 修好后转正；项目惯例同 BUG-003/BUG-004 复现用例）。
  3. **`redaction::redact_urls_in_text` 右向扫描会吞掉紧邻 URL 的非 URL 字符**（BUG-010，P4）：
     停字符集只含空白/引号/括号/中文标点，**不含汉字与 ASCII 字母** →
     `…?sign=x已过期，请重试` 里的"已过期"被并入 URL 段后一起替换（QA 实测快照丢字）。
     复现脚本 `artifacts/web-mvp/bug008-qa/qa-r26-text-swallow.sh`。修法：右边界按 RFC 3986
     允许字符集（`A-Za-z0-9-._~:/?#[]@!$&'()*+,;=%`）收敛。
  4. **备份"过滤不误伤"与"仍生效"的可执行判据**：注入 ① `job_stages.usage_json`（JSON，含
     `modelUrl`/`renderedImageUrl`）、② `job_stages.last_error`、③ `provider_attempts.last_error`
     （非 JSON 文本）→ 备份后**供应商事实 0 命中**（`tempUrlsRedacted=4` 入日志/CLI/manifest notes），
     同时 ④ `documents.source_url` 的**用户出处链接必须原样保留**（证明不过度清洗）。
  5. **导出 fail-closed 与"用户内容不误伤"的双向手法（绕过 `manual_releases` 不可变触发器）**：
     读冻结 manifest 的 blob 文件 → 改 JSON → 重算 sha → 新插 `blobs` 行（**要带 `created_at`**，
     NOT NULL 无默认）→ `UPDATE assets SET blob_id`（release 行不动）→ ① URL 写进生成字段
     （`assets[].source`）→ 导出必须 5xx + 日志 `export_manifest_url_forbidden` + **无 ZIP**（判据用
     `PK` magic，别拿响应体长度当"包大小"——错误 JSON 也有 100+ 字节）；② 用户链接写进
     `knowledge` 子树 → 导出必须仍 200 且链接随包保留。
  6. **恢复库的"按 task_id 重查"要自己造现场**：把 `model_download` 置 `queued` **并清
     `result_asset_id`/`last_error`/`needs_input_json`**（只改 status 会被"本地副本优先"跳过下载），
     重启 `serve` + 本机 fixture → 观察到 `GET /v3/tasks/`（免费重查）且
     `POST /v3/generation/multiview-to-model` 计数仍为 0。
  7. **验收脚本的外网门禁**：把 `[providers.manual_ai]` 只配 `api_key_env` 而不配 `base_url` 时，
     `manual_extract` 会真实外发到默认公共端点（QA 脚本首版踩到一次 `manual_extract_sent`）。
     脚本必须把**两个 provider 的 base_url 都指向本机**，并断言"serve 日志里非回环主机数 = 0"。
  8. **服务运行中直接复制 `.sqlite3` 会丢 WAL 里的已提交数据**：留证用 `.dump`/查询输出，
     或 `kill -9` 后连 `-wal` 一起复制（本轮 `work-*/manual.sqlite3` 副本因此为空表）。
- 未关闭缺陷／观察：**BUG-009（P2，OPEN）**、**BUG-010（P4，OPEN）**；OB-9（`TripoError::redacted()`
  与 `redact_url_query` 保留 `scheme://host/path?[redacted]`：无签名但按"严格 `://`"口径会命中，
  需 PM/RD 在 T21 定标）；T18-4 判定为**测试口径脆弱**（非产品泄漏），QA 已把判据改为
  "强制 GC 后保留堆前 3/后 3 中位数比"（阈值不变 1.5；隔离 3 次 + 全量 1 次实测 1.09–1.10）。
- 相关代码／测试／证据：`crates/server/src/{redaction.rs,assets/glb/download.rs,providers/tripo/handlers.rs,http/jobs.rs}`、
  `crates/server/tests/qa_t20_bug008_independent.rs`、`apps/web/tests/e2e/qa-t18-independent.spec.ts`、
  `artifacts/web-mvp/bug008-qa/`、[QA 回合 26 报告](requirements/web-mvp/qa-report.md)。

## ADR-033 — 失败路径的签名 URL 脱敏与"URL 形态"口径（BUG-009/BUG-010 修复；OB-9 定标）

- 状态：implemented（2026-09-13，RD 修复回合；RD 自验：`--test qa_t20_bug008_independent -- --ignored`
  修复前 RED（2 failed，原文 `artifacts/web-mvp/r28-rd-fix/repro-before-qa-t20-ignored.log`）→ 修复后 2 passed；
  新增 `--test redaction_persistence` 4 通过、`--lib` 172 通过、`cargo test --workspace`、`cargo xtask check`、
  dist + smoke、e2e 结果见 `llmdoc/requirements/web-mvp/implementation.md` §R28；**待 QA 回合 27 复验**）。
- 日期／作者角色：2026-09-13 · RD（QA 回合 26 缺陷修复：BUG-009 P2 / BUG-010 P4 / OB-9 定标）。
- 背景与证据：QA 回合 26 报告（`requirements/web-mvp/qa-report.md`）
  ——BUG-009：`DownloadError::Transport{detail}` 直接拼接 `reqwest::Error`，reqwest 0.13.5 的
  `Display` 追加 ` for url (<完整签名 URL>)`（`reqwest-0.13.5/src/error.rs:299-302`），消息经
  `download_failure_outcome` → `job_stages.last_error`（仓储 `advance`）→ 任务详情 DTO `lastError`
  与 `model_download_failed` 日志（`detail = %error.log_summary()`）原样出网；BUG-010：
  `redact_urls_in_text` 右向停字符集不含汉字/ASCII 字母，`…?sign=x已过期` 丢"已过期"；OB-9：
  `TripoError::redacted()`/`redact_url_query` 保留 `scheme://host/path?[redacted]` 与"严格 `://`"口径冲突。
- 结论与原因：
  1. **OB-9 口径（定标）**：**需要脱敏的 URL 文本** = 文本中的 `scheme://…` 片段（scheme 为 RFC 3986
     scheme 字符集；出现 `://` 即命中），**无论是否带查询串**；处理 = 整段替换为统一摘要标签
     `（临时供应商地址已脱敏：host=…；sha256=…）`。**裸 host**（`cdn.example.invalid`、`127.0.0.1`）
     不是 URL 文本、**允许保留**（最小诊断）；task_id、计数、状态、摘要对象/标签原样保留。
     **唯一例外** = 部署者自有配置回显（provider `baseUrl` 的日志/管理页），走 `redact_url_query`
     保留 `scheme://host/path`（构造时已校验不含查询串/片段，不是供应商临时地址；QA 判据表同款例外）。
     选严格侧而非"允许 `?[redacted]`"：QA 的全库/全日志扫描判据是 `://`，保留任何 `scheme://` 形态
     都会让未来回归"各说各话"；且该形态唯一来源是供应商文本，换成摘要不损失可行动信息。
  2. **统一入口（文本）** = `crate::redaction::redact_text_urls`（`redact_urls_in_text` 的收敛形态）；
     调用点四处收口，结构上不再依赖"记得补某个展示点"：
     ① **错误构造**：`assets::glb::download::classify_transport`、`providers::tripo::client`
     `classify_transport_error` + `business_summary`、`providers::manual_ai::client`
     `classify_transport_error`；② **持久化写入**：`storage::repo::job_stages`（`advance` 全部变体 +
     `reset_for_retry`/`requeue_succeeded_dependents`/`apply_reconcile_resolution`）与
     `storage::repo::attempts`（`mark_unknown`/`mark_failed`/`set_last_error`）；③ **对外 DTO 读取**
     （`http::jobs` 的 `lastError`、attempt `lastError`、`needs_input` 消息；覆盖历史行，不改写源库）；
     ④ 备份兜底（`backup::create` 沿用同一函数）。`needs_input_json` 是序列化 JSON 文本，直接做文本级
     替换是安全的：摘要标签只含 ASCII/中文/全角标点，不含 `"`/`\`/控制字符（单测断言脱敏后仍可解析）。
  3. **reqwest 文本策略 = `without_url()` + 兜底脱敏**（不是截断原始文本）：先 `Error::without_url()`
     让 Display 不再追加 URL，再对拼好的 detail 走统一入口（reqwest 未来若在消息正文内嵌 URL 也被兜住）。
     丢弃的只有 URL 与底层 source 链（OS 细节，修复前也从未出现在 Display 里），**保留**错误类别
     （超时/连接失败/响应体读取中断）、稳定错误码（`download_transport`）、目标 host（日志字段+摘要标签）
     与"下载可安全重试；链接过期按 task_id 重查（不重新购买）"的结论 → 恢复/诊断能力不退化。
  4. **BUG-010 修法**：右向扫描从"停字符集"改为"**只消费 RFC 3986 允许字符**"
     （`A-Za-z0-9-._~:/?#[]@!$&'()*+,;=%`），其余字符一律结束 URL 段 → `…?sign=x已过期，请重试`
     只替换 URL 本体；有意偏离 RFC：`'` 不消费（自由文本里更像引号）。**已知限制**（记录不掩盖）：
     URL 后紧邻的 ASCII 字母无法与 URL 本体区分（按 URL 语法本属 URL），中文/全角/空白等场景已覆盖。
  5. **既有断言的变化方向 = 收紧**：`providers/{tripo,manual_ai}::client` 两个单测把
     `text.contains("?[redacted]")` 改为 `!text.contains("://")` + host 标签 + "URL 之后文本保留"；
     没有放宽/删除任何 AC 或断言。
- 影响的 REQ／任务／模块：AC-010 / AC-066 / REQ-043 / REQ-044（§5.7 隐私与日志）；T13（下载错误）、
  T12/T14（Provider 错误文本）、T15/T17（任务详情 DTO 与日志）、T20（备份过滤同函数）；T21 安全矩阵可把
  "失败路径的签名 URL"并入 AC-065/066 用例集。
- 未验证边界与下一步：真实 Provider/CDN 下的连接失败与超时形态（T23，本回合全部本机 fixture）；
  错误文本的具体措辞（`error sending request` 为 reqwest 原文）在真实链路下的可读性复核；
  若未来要求"连 host 也不留"属新修订（需改本 ADR 与 ADR-032 第 1 条）。
- 相关代码／测试／证据：`crates/server/src/redaction.rs`、`crates/server/src/assets/glb/download.rs`、
  `crates/server/src/providers/{tripo,manual_ai}/client.rs`、`crates/server/src/storage/repo/{job_stages,attempts}.rs`、
  `crates/server/src/http/jobs.rs`、`crates/server/tests/{redaction_persistence,model_assets}.rs`、
  `crates/server/tests/qa_t20_bug008_independent.rs`（QA 复现用例，修复后 `-- --ignored` 全绿）、
  `artifacts/web-mvp/r28-rd-fix/`、[implementation §R28](requirements/web-mvp/implementation.md)、
  [QA 回合 26](requirements/web-mvp/qa-report.md)、ADR-032（脱敏规则与"按 task_id 重查"）。

### BUG-009/BUG-010 复验验收知识（追加，2026-09-13；QA 回合 27：两只缺陷 **CLOSED**，新增 BUG-011/BUG-012（均 P3））

- 状态：QA 验收手法、残余缺口与复用判据（非代码知识），供 T21–T23 与后续缺陷修复复用。缺陷与命令证据见
  [QA 回合 27 报告](requirements/web-mvp/qa-report.md)；ADR-033 的口径 **QA 接受，未变**。
- 复用价值最高（回合 27 新增）：
  1. **"脱敏是否收口"要看三层 × 三类值，不能只看一层**。写侧（`job_stages`/`provider_attempts` 的
     `last_error`/`needs_input_json` 已收口）、读取侧（DTO `lastError`/`needsInput`/`usage` 已兜底）、
     备份侧（文本列已兜底）之外，**`job_stages.usage_json` 不在任何写侧脱敏范围**：
     `manual_ai` 的 `failure_of` → `not_produced(error_summary)` → `errorSummary` 把**提供方原文**
     （refusal / incomplete reason / 信封解析错误）写进结果事实 → **BUG-012**（凭据 URL 落库；
     API/备份/导出仍兜住，暴露面 = data-dir 库文件）。同类入口还有 `envelope_error`。
  2. **备份的 JSON 列兜底与文本列兜底语义不同 → 过度替换（BUG-011）**：`redact_urls_in_json` 的判据是
     "整串含 `://` → 整串替换为 `{"redacted":true,…}`"，对 `needs_input_json` 的**句子型 message**
     会把整条句子换成对象 → 恢复后 `Vec<JobMissingItemDto>` 反序列化失败被
     `http/jobs.rs` 的 `.ok()` + `unwrap_or_default()` **静默吞成空列表**（stage.status 仍是
     `needs_input`，但 UI 看不到任何缺项；同阶段无 URL 的条目也一起消失）。
     最小复现：注入 `download_insecure_scheme` 的消息（文案含 `http://host`）→ backup → restore → 查 DTO。
  3. **"产品可达的 `://` 文案"清单**（判 BUG-011/012 是否触发时直接用）：`DownloadError::InsecureScheme`
     的 message（`模型下载必须使用 HTTPS（实际 {scheme}://{host}）…`）是目前唯一会把 `://`
     写进 `needs_input_json` 的产品文案；提供方自由文本（refusal/incomplete/信封）是 `usage_json` 的入口。
  4. **`without_url()` 之后 host 只存在于日志字段**：传输失败消息正文不会有 host（没有 URL 片段可做摘要），
     `model_download_failed` 的 `host` 字段是唯一保留点 → 评估"诊断能力"时别只在消息里找 host。
  5. **判"RD 是否改过 QA 测试文件"的可用手法**（文件未入 git 时）：文件 mtime 早于 RD 本轮首个产物 +
     修复前后日志里的 panic 行号与当前断言行一致 + 用例名/canary/文案逐字比对。
  6. **dist 级注入法可完整验证"读取侧 + 备份 + 日志"三层**（产生侧仍只能在测试构建做，release 不允许
     本机 fixture）：把修复前形态的 URL 注入 `job_stages.last_error`/`needs_input_json`/`provider_attempts.last_error`
     → `backup`（扫快照）→ `serve`（扫 DTO/日志）→ 同时断言"源库 dump sha256 前后一致"与
     "`SHA256SUMS`/manifest sha256 一致"；脚本 `artifacts/web-mvp/bug009-qa/qa-r27-redaction-dist.sh`。
  7. **e2e 的 30s 代价**：把"黑障超时"复现用例转正会让默认 `cargo test` 多花 ~30s（黑障线程 join）；
     接受该代价换取"只测半关闭会误判无泄露"的回归保护。
- 未关闭缺陷／观察：**BUG-011（P3）**、**BUG-012（P3）**；OB-11（提供方文本同类入口建议一并收口）、
  OB-12（`is_url_like` 的"整串含 `://`"语义与文本路径的"片段替换"语义分叉，易被后续字段误用）。
- 相关代码／测试／证据：`crates/server/src/redaction.rs`、`crates/server/src/backup/create.rs`、
  `crates/server/src/providers/manual_ai/handlers.rs`、`crates/server/src/storage/repo/job_stages.rs`、
  `crates/server/src/http/jobs.rs`、`crates/server/tests/qa_t13_independent.rs`（回合 27 的 `qa_bug009_*`/`qa_bug012_*`）、
  `crates/server/tests/qa_t20_bug008_independent.rs`（回合 27 转正）、`artifacts/web-mvp/bug009-qa/`、
  [QA 回合 27 报告](requirements/web-mvp/qa-report.md)。

## ADR-034 — JSON 列的结构感知脱敏与写侧收口（BUG-011/BUG-012；OB-11/OB-12 处置）

- 状态：implemented（2026-09-13，修复回合；RD 自验结果与命令见 `implementation.md` §R29；
  **待 QA 回合 28 复验**）。
- 日期／作者角色：2026-09-13 · RD（QA 回合 27 新增缺陷修复 + 穷举式收口）。
- 背景与证据：QA 回合 27（`requirements/web-mvp/qa-report.md`「回合 27」缺陷节）——
  **BUG-011**：备份快照对含 `://` 的 JSON 字符串**整串替换为摘要对象**，`needs_input_json`
  的句子型 `message` 整条丢失 → 恢复后整列解析失败被 `stage_dto` 的 `.ok() +
  unwrap_or_default()` 静默吞成空列表（复现：`artifacts/web-mvp/r29-rd-fix/repro-before-bug011.log`）；
  **BUG-012**：`job_stages.usage_json` 写侧无脱敏，`manual_ai` 的 `errorSummary`
  （refusal/incomplete/信封解析错误原文）把签名 URL 落库（复现：`repro-before-bug012-red.log`，
  全库扫描命中 `[("job_stages","usage_json",1)]`）；QA 观察 OB-11（同类入口）、
  OB-12（`is_url_like` 的"整串"语义与文本路径"片段"语义分叉易被误用）。
- 结论与原因：
  1. **JSON 列的规则 = 逐字符串值脱敏（ADR-033 文本规则在值内的投影）**，两种值形态：
     ① 字符串**整体就是一个 URL**（去首尾空白后只有一个 URL 段）→ 摘要对象
     （`{"redacted":true,…}`，ADR-032 未变）；② 字符串是**句子**（URL 只是片段）→
     **只替换 URL 片段**为摘要标签，句子、键与结构原样保留（BUG-011 的修法；OB-12 的分叉
     在此**显式定标**：JSON 路径 = 值形态自适应，不再有"整串含 `://` 就替换"的第三种语义）。
     实现唯一入口：`redaction::redact_urls_in_json_with(value, mode)` 与序列化文本入口
     `redact_json_text_urls(text, mode)`；模式 `SummaryObject`（任意事实 JSON；ADR-032 形态）
     / `KeepString`（**契约规定值必须是字符串**的列，如 `needs_input_json.message`——
     值变对象会让按类型反序列化的读取侧整列失败，BUG-011 的教训）。
  2. **写侧收口点**（纵深：产生侧 + 仓储侧二层）：
     - 产生侧：`manual_ai::handlers` 的 `failure_of` 统一包一层 `redact_text_urls`
       （覆盖 refusal / incompleteReason / 信封错误 / 空输出各分支）与 schema 校验错误的
       `error.detail_text()`——`usage_json.errorSummary`、批次结果资产 blob、
       `needs_input_json` 的 message 三个产物同源同清（QA 的 OB-11 处置）；
     - 仓储侧：`job_stages::advance(Succeeded)` 与 `set_result_fact` 的 `usage_json`
       走 `redact_json_text_urls(SummaryObject)`；`advance(NeedsInput)` 的
       `needs_input_json` 改为 `KeepString`（与备份侧同规则）。
  3. **备份侧**（BUG-011）：`backup::create` 的 JSON 列按列契约选形态
     （`needs_input_json` → `KeepString`，其余 → `SummaryObject`），解析失败仍文本兜底；
     "整串对象替换"只保留给纯 URL 值。既有断言（快照 `usage.modelUrl` 是摘要对象）不变。
  4. **可诊断性不退化**：`needsInput` 的缺项 code 与句子、`usage` 的 task_id/摘要/计费
     全部保留；摘要标签给出 host + 不可逆 sha256 前缀。`stage_dto` 对
     `needs_input_json` 的解析失败**如实写日志**（`job_detail_needs_input_parse_failed`），
     不再静默吞（BUG-011 隐蔽性的根因）；源库不被改写，历史行由读取/备份两侧兜底。
  5. **结构性保障（回答"明天新增一个供应商错误文本字段会不会漏"）**：
     - **唯一 SQL 写入层**：供应商事实表的写语句只允许出现在 `src/storage/repo/`；
     - **写入函数脱敏义务**：带写语句的仓储函数必须调用统一入口或在豁免清单登记理由
       （身份标识符 `remote_task_id`/`response_id`、纯状态/NULL、常量输入——脱敏 id 会
       破坏"按 task_id 重查"的恢复语义）；
     - 以上两条固化为**审查测试** `crates/server/tests/redaction_surface.rs`（新增仓储文件/
       新列会被自动扫描；负向验证：临时投放违规文件必失败，见 implementation §R29 证据）；
     - **备份按"供应商事实表 × 全部 TEXT 列"扫描**（规则在表级，不按字段名）→ 新增列自动
       被兜住；读取侧 DTO 对历史行兜底；
     - **行为扫描**：新建数据全链路 canary → 全库逐表逐列 0 命中（RD
       `redaction_persistence.rs`、QA `qa_t13_independent.rs::qa_bug012_*`）+ dist 级
       `read→backup→restore→DTO→导出→日志` 脚本（QA `qa-r27-*`、RD
       `artifacts/web-mvp/r29-rd-fix/verify-redaction-dist.sh`）。
  6. **表外边界（如实登记，不冒充覆盖）**：`manual_ai` 的**原始响应诊断 blob** 逐字节保留
     提供方原文（既定设计：受限诊断路径、无 HTTP 路由、`manual_ai_contract.rs` 有逐字节
     断言）——若提供方原文含签名 URL，它以"提供方自己的原文"形态存在于 data-dir/备份的
     blob 文件中，不在 QA 现有判据口径（DB 列/DTO/备份库/日志）内；若未来要求连它一起清洗，
     属新修订（需同步改该逐字节断言与 ADR-024 诊断语义）。用户内容（`sourceUrl`、知识 JSON）
     与标识符按 ADR-032 原样保留。
- 影响的 REQ／任务／模块：AC-009/AC-010、AC-036、AC-066、REQ-024/REQ-043/REQ-044；
  T13（下载错误）、T14/T15（提供方文本、任务详情）、T20（备份/导出）、T23（真实 Provider 复核）。
- 未验证边界与下一步：真实 Provider/真实 CDN 形态（T23）；dist（release）无法产生
  传输失败/提供方文本（fixture 仅测试构建），dist 侧验读取/备份/日志三层。
- 相关代码／测试／证据：`crates/server/src/redaction.rs`、`crates/server/src/backup/create.rs`、
  `crates/server/src/providers/manual_ai/handlers.rs`、`crates/server/src/storage/repo/job_stages.rs`、
  `crates/server/src/http/jobs.rs`、`crates/server/tests/{redaction_surface,redaction_persistence,
  backup_restore,manual_ai_contract}.rs`、`artifacts/web-mvp/r29-rd-fix/`、
  [implementation §R29](requirements/web-mvp/implementation.md)、[QA 回合 27](requirements/web-mvp/qa-report.md)、
  ADR-032（脱敏规则）、ADR-033（文本口径与统一入口）。

### BUG-011/BUG-012 复验验收知识（追加，2026-09-13；QA 回合 28：两只缺陷 **CLOSED**，"穷举式收口"结构性保障四项实测成立）

- 状态：QA 验收手法、实测结论与两条 P4 残余（非代码知识），供 T21–T23 与后续字段扩展复用。缺陷原文与命令证据见
  [QA 回合 28 报告](requirements/web-mvp/qa-report.md)；ADR-034 的口径 QA 已接受，未变。
- 复用价值最高（回合 28 新增）：
  1. **结构守卫的负向验证手法**（不改生产代码、不重编译）：在 `crates/server/src/`（或 `storage/repo/`）**新建**一个含违规语句的临时 `.rs` 文件（不被 `mod` 引用即不参与编译）→ `cargo test --test redaction_surface` 应失败并打印文件/行号 → 删除文件恢复。两个守卫的判别力均经此实测（`artifacts/web-mvp/bug011-qa/qa-r28-guard-negative.log`）。
  2. **"新列免改代码"的实测手法**（dist 级、零代码改动）：restore 样例后对 **源 data-dir** `ALTER TABLE job_stages ADD COLUMN …` 造 3 类新列（非 JSON 裸 URL / JSON 句子 / JSON 整串 URL）→ `backup` → 快照中三列全部脱敏。备份列清单来自 `PRAGMA table_info`（`backup/create.rs`），所以**新列自动兜底**成立；但**新表**要加进固定表清单、**新字符串契约列**要在 `json_string_mode_for_column` 登记（否则整串 URL 值变对象）——两处即 OB-13/OB-14（P4）。
  3. **守卫颗粒度**（如实记录）：①按字面 SQL 模式扫 `src/**`（跨行写/动态表名不拦；`AssertSqlSafe` 点是人工审计点）；②函数级（既有脱敏函数内新增字段不触发）。缓解 = 行为 canary（`redaction_persistence`/`qa_t13`/dist 脚本）+ 评审清单。
  4. **表外边界（诊断 blob）可独立复核**：往 data-dir 注入内容寻址 blob（sha 与内容一致；backup 会校验一致性）→ `backup` **原样复制**（blobs/ 不过滤内容）→ ADR-034 §6 的边界声明属实（`qa-r28-blob-boundary.sh`）。
  5. **可复现构建**：同一工作树两次 `cargo xtask dist` 产出**逐位相同**的 sha256（本轮 QA 重建与 RD 构建一致）——比对二进制 hash 是判断"QA 是否复用了 RD 的构建"最省事的证据。
  6. **本轮 QA 命令集**（可直接复跑）：`qa-r28-bug011-sentence.sh`（句子保真/多条目/损坏行告警/新列兜底，182 项）、`qa-r27-bug011-evidence.sh`（回合 27 现场）、`qa-r28-rd-verify-redaction-dist.sh`（RD 脚本字节相同副本）、`qa-r27-redaction-dist.sh`（回合 27 三层回归）、`qa-r28-blob-boundary.sh`；全量回归见 qa-report 回合 28 表。
- 未关闭观察：OB-13（新列/新表需显式接线）、OB-14（守卫颗粒度）——均 P4 非阻断；OB-11/OB-12 已随 R29 处置。
- 相关代码／测试／证据：`crates/server/tests/redaction_surface.rs`（守卫）、`crates/server/tests/qa_t13_independent.rs`（`qa_bug012_*` 转正、`qa_r28_usage_json_*` 新增）、`crates/server/src/backup/create.rs`、`crates/server/src/redaction.rs`、`artifacts/web-mvp/bug011-qa/`、[implementation §R29](requirements/web-mvp/implementation.md)、ADR-032/ADR-033/ADR-034。

### T21 验收知识（追加，2026-09-13；QA 回合 29：§3 矩阵全行覆盖、五个崩溃断点 + 批次中断 + 性能记录，切片 **PASS**，无新增缺陷）

- 状态：QA 验收手法与非代码结论，供 T22/T23 与后续故障注入复用。完整矩阵与命令证据见
  [QA 回合 29 报告](requirements/web-mvp/qa-report.md)；本轮未改任何命令合同语义。
- **可跨卡复用的验收手法**：
  1. **否定型用例必须有正对照**：上传"断流"用例先证明 `tmp/*.part` **出现**（请求确实进了流式 handler），再断言断开后归零、`assets/blobs=0`；否则"零副作用"的通过可能只是请求被 401/403 提前拒绝的空转。
  2. **断点四（blob rename 后 DB 事务前）的两级证据**：① 进程内——直接往 `blobs/<前2位>/<sha>` 写一段**未被引用**的合法内容（rename 已完成、元数据未提交）+ 一个 `tmp/*.part`，然后**重开数据库 + 新 AppState/Router** 并调用 `assets::maintenance::scan_and_quarantine`（与 `config/commands.rs` serve 启动例程**同一函数**）；② dist 级——`serve` → 上传 → **kill -9** → 注入现场 → 同 data-dir 重启（`qa-r29-blob-breakpoint-dist.sh`，10/10 [OK]）。核对五件事：jobs／provider_attempts／cost_ledger／model_revisions 行数不变、被引用资产逐字节可读、孤儿与 tmp 进 `quarantine/`（只移动不删除）、同内容重传 201 且 blob 行不重复。**注意**：注入内容必须是合法可上传类型（PDF），否则重传会 415，把脚本自身缺陷误判成产品问题。
  3. **断点五（draft 已提交但客户端拿不到结果）**：直接调 `drafts::assemble_draft`（与 `assemble_draft` 阶段处理器同一入口）落草稿行 → 把该阶段改回 `running` + 过期租约 → `recover_expired_leases` + 注册真实 `PipelineHandlers` 重跑 → 断言草稿恒 1 份、同 id、revision 不增、attempts/ledger/release 均为 0。**注意**：`assemble_draft` 对"分支 succeeded 但缺结果事实"是**硬完整性错误**（`draft_merge_result_missing` / `draft_model_revision_missing`），构造用例时应让分支停在 `needs_input`（→ 部分草稿），不要伪造成成功。
  4. **批次级"不重跑/不重发"必须按 per-stage attempt 计数归因**（`provider_attempts WHERE stage_id = ?`）：fixture 的全局调用计数无法区分批次；批次身份独立性的证据 = 各自 stage id + `page_set` + attempt 行。
  5. **手册批次中断的实测形态**（11 页 → 3 批）：批 0 成功 → 批 1 命中 `manual_after_request_before_response` → 恢复扫描后批 1=`submission_unknown`；**恢复后批 3 在已确认预算内继续执行**（validation-release §3 允许"继续或暂停"，contracts §5 表格措辞更严 → OB-16 已交 PM 留痕）。
  6. **Playwright 阻断 PDF worker 的正确姿势**：**不可**用宽 glob（如 `**/*pdf.worker*`）——dev 下 `vendor.ts` 会静态 import `pdf.worker.min.mjs?url`，连带 abort 会让 PreparePage 模块图整体失败（页面渲染不出来，表现为"测试超时"而非产品缺陷）；应只阻断真正的 worker 脚本请求（predicate 过滤掉 `?url`）。另：`prepare-status` 只在拿到总页数后渲染，拒绝/失败态不存在 → 读可选文案必须给**有界 timeout**，否则 `textContent()` 会等到测试超时。
  7. **bash 脚本与全角标点**：`$VAR（` 会被 bash 解析为变量名的一部分（`ORPHAN_SHA（: unbound variable`）；中文文案里引用变量统一写 `${VAR}`（本轮两次踩坑）。
  8. **元数据 API 性能测量**：`init --password-file` + `serve`（dist）→ 登录 → 造数 30 物品 → `curl -w '%{time_total}'` 采样 300 次取 p95（`qa-r29-metadata-api-p95.sh`）。本机 Apple M1：items 列表 **p95 0.6 ms**（目标 ≤200 ms）。阅读器旋转 QA 复跑 RD 脚本：**p95 17.7 ms**（目标 ≤33 ms，1 203 样本，Chrome 152 真实 GPU）。
- **未关闭观察（P4）**：OB-15（worker 缺失被归类为"文件可能损坏"，文案指向错误排查方向）；OB-16（contracts §5 与 validation-release §3 的措辞张力）；OB-17（缺陷 CLOSED 后其 `#[ignore]` 复现用例可能长期静默跳过——本轮已转正 BUG-003/004 两例，建议写进卡面完成定义）。
- **对后续卡的明确事项**：T22 需交付 CI（当前仓库无 `.github/workflows`；合同未把 CI 列为 AC 门禁，但 T22 卡面含"CI release矩阵"）、正式包 smoke（AC-002 全项）、Firefox/Edge 浏览器矩阵（本机未安装，Playwright 仅 chromium）、平台动态依赖清单；性能长期项（≥5 分钟连续旋转、模型加载耗时、PM 具名设备）。
- 相关测试／证据：`crates/server/tests/qa_t21_independent.rs`、`apps/web/tests/e2e/qa-t21-independent.spec.ts`、`artifacts/web-mvp/t21-qa/**`（含 `qa-r29-blob-breakpoint-dist.sh`、`qa-r29-metadata-api-p95.sh`、`r29-e2e-full.log` 88 passed/0 skipped、`r29-source-manifest.txt`）。

## ADR-035 — T22 发布链落地取舍（产物形态、路径归一化、licenses 同源、两级 smoke、平台 BLOCKED 的恢复路径）

- 状态：accepted，macOS aarch64 已实测（2026-09-13）；Linux x86_64-musl **BLOCKED（环境）**。
- 日期／作者角色：2026-09-13 · RD（T22）。
- 背景与证据：
  1. `cargo xtask dist` 升级为正式发布链后，首轮隔离扫描实测：二进制中**不含**仓库根 / `apps/web/dist`
     / `node_modules` 路径，但**含 651 处 `$HOME` 路径**，全部形如
     `<home>/.cargo/registry/src/index.crates.io-…/<crate>/src/…`（依赖 crate 的 panic 位置字符串）。
  2. `smoke`（§7 的 7 步）在 macOS 全过：`restore` T20 样例备份 → 生产 `embedded-ui` → 登录/资源/嵌套路由
     → Range/HEAD/ETag + sha256 比对 → `sandbox-exec` 断网读取 → 重启持久化 → `backup`→新目录 `restore`
     再读，三个 sha256 与首轮一致。产品代码无需任何修复。
  3. 二进制可复现：`--check-reproducible` 用 `cargo clean -p everything-manual --release` 后再建，
     与首轮、与上一次完整 `dist` 共 3 次构建 sha256 一致（`ab693cc3…`）。
  4. `licenses.json` 旧实现用不带 feature 的 `cargo metadata`（139 包，缺 embedded-ui 专属依赖，
     见 QA 回合 1 知识）；新实现按 **cargo tree normal 边 + 目标平台过滤** 得 203 包。
- 结论与原因：
  1. **发布产物固定 5 件**：`everything-manual` / `SHA256SUMS`（仅二进制） / `licenses.json`（v2） /
     `build-info.json`（v2） / `dynamic-dependencies.txt`；`assert_clean_output_dir` 拒绝其它任何文件
     （防 node_modules、源码与旧构建残留混入）。
  2. **用 `--remap-path-prefix` 从根上消除构建机路径，而不是放宽检查**：构建 release 时注入
     `$HOME=/build/home`、`<repo>=/build/repo`；隔离扫描仍**硬失败**于任何构建机绝对路径命中。
     代价：RUSTFLAGS 变化会使依赖树整体重编译一次（~1–2 min），且路径含空格会失败（不是静默出错）。
  3. **动态依赖必须在原生构建时采集**：`dist` 在 `target == host` 时写 `otool -L` / `file`+`ldd`+`readelf -d`
     结论；跨构建只写"未采集"说明（§6：交叉编译退出码 0 不是运行证据）。
  4. **`--check-reproducible` 是"独立重链接"而非空转比对**：先 `cargo clean -p everything-manual --release`，
     再构建并比对；仅比对两次连续构建会被"没有改动所以不重编"掩盖真实不确定性。
  5. **两个 smoke 分工固定**：`smoke-bootstrap` = T01 最小（init + 页面/资源/health）；`smoke` = T22 正式包
     （restore 合法备份 + 认证 + 资产 Range/HEAD + 断网 + 重启 + 备份恢复链）。`smoke` **不得**连 fixture：
     以"子进程环境清空 + 工作目录不在仓库 + 断言 `providersConfigured=false` + 报价 409
     `PROVIDER_NOT_CONFIGURED`"四重保障落实。
  6. **离线证据按平台取证**：macOS 用 `sandbox-exec`（`deny network-outbound` + 仅放行 localhost，
     已用对照实验证明双向行为）；Linux 用 `docker run --network none` 跑整条 smoke。二者都**不是**
     修改 PATH 或"没配 Provider"这类弱证据。
  7. **Linux 平台 BLOCKED 的恢复路径固化为脚本**：`scripts/linux-musl.sh`（宿主侧复制工作树 → 容器内
     dist + file/ldd + smoke → 产物回收到 `artifacts/`；`--offline-only` 走 `--network none`）。
     脚本在宿主复制而非挂载，避免容器内 `npm ci` 把宿主 `node_modules` 换成 Linux 版。
- 影响：
  1. 后续任何 `dist` 必须保持 `--remap-path-prefix`（否则隔离扫描会失败，这是**期望的**保护）。
  2. `licenses.json` schemaVersion=2：字段路径变化（`rust.packages` / `web.packages`）；引用方按 v2 读取。
  3. `SHA256SUMS` 只覆盖二进制，`build-info.json` 因含构建时间**不追求**跨次字节一致（二进制才要求一致）。
  4. 新增平台只需 `rustup target add` + 在目标平台执行 `dist`（原生）——不得用交叉构建产出替代运行证据。
- 未验证边界与下一步（BLOCKED 解除条件）：
  1. Docker Desktop 引擎在宿主 ENOSPC 事件后无法启动（VM 引导后约 1 s 被宿主 `engines` 发
     `POST /shutdown`；`services.ErrServiceFailed` / `io: read/write on closed pipe`）。非破坏性动作
     （重启应用/后端、`docker desktop stop|start`、`POST /engine/start`、`diagnose`）均已尝试无效；
     Clean/Purge 或删除 64 GB `Docker.raw` 属破坏性操作，需所有者授权。
  2. 引擎恢复后：`scripts/linux-musl.sh --arch amd64`（构建 + smoke）与 `--offline-only`（断网读取）
     采集 Linux 证据；在两平台产物齐备前，AC-064/AC-002/AC-059 只能按 macOS 半边记录。
  3. 浏览器矩阵（Firefox/Edge）、真实 Provider（T23）、签名/公证仍为未覆盖项。
- 相关链接：`xtask/src/{dist,smoke,util}.rs`、`.github/workflows/ci.yml`、`docs/operations.md`、
  `scripts/{linux-musl,container-linux-musl}.sh`；证据：`artifacts/web-mvp/t22-rd/**`、
  `dist/aarch64-apple-darwin/*`；实现记录：`requirements/web-mvp/implementation.md` §T22。

## ADR-036 — T22 Linux x86_64-musl：容器原生构建的路径/缓存/镜像取舍，与 ADR-035 的差异

- 状态：accepted（Linux 半边**已实测**：原生 `dist` + 原生 `smoke` 7 步 + `--network none` 整条 smoke +
  静态链接 0 动态依赖 + 独立重链接同哈希；2026-09-13）。**尚未经独立 QA 验收**。
- 日期／作者角色：2026-09-13 · RD（T22 续做，QA 回合 30 待验）。
- 背景与证据（原始日志 `artifacts/web-mvp/t22-rd/linux/**`，实现记录 §T22-13）：
  1. Docker 引擎恢复后，仍走 ADR-035 固化的脚本路径（宿主复制工作树 → amd64 容器内原生构建 + smoke），
     没有另建旁路流程。宿主 Apple Silicon 上 `--platform linux/amd64` 由 **Rosetta** 执行
     （容器 `/proc/cpuinfo` = `VirtualApple @ 2.50GHz`），冷启动全流程约 9 分钟、热构建约 100 秒——
     比"数小时"的保守预期快得多，故今后同环境可放心重跑。
  2. 产物 `77b38c91…`（28 236 728 B）：`file` = `static-pie linked`、`ldd` = `statically linked`、
     `readelf -d` 的 **DT_NEEDED = 0**；`--check-reproducible` 独立重链接后同哈希。
  3. `--network none` 下整条 smoke 7 步全过；容器内自证断网（无默认路由、DNS 失败、`curl` http_code=000）。
- 结论与原因：
  1. **容器内工作树挂载点必须是"不与隔离扫描子串语义冲突"的深路径**，最终
     `/src/everything-manual`（`EM_LINUX_WORK_MOUNT` 可覆盖）。两次真实误报：
     ① `/build` 与 remap 目标 `/build/home`、`/build/repo` 自撞；② `/work` 与依赖 panic 路径里极常见的
     `.../worker.rs` 自撞（sqlx/tokio 共 5 处）。**没有**放宽扫描的子串语义——这是构建环境问题。
  2. **`CARGO_HOME` 与 `RUSTUP_HOME` 必须成对，且 `CARGO_HOME` 要在 `$HOME` 之下**：
     rustup 要求自身装在 `$CARGO_HOME/bin`（否则报 "rustup is not installed at …"）；而 `dist` 的隔离扫描
     以 `$HOME` 为归一化锚点，`CARGO_HOME` 在 `$HOME` 之外时 registry/src 的构建机路径既不被 remap
     归一化、也不被扫描拒绝（运行包带脏路径）。脚本固定 `HOME=/root` + `CARGO_HOME=/root/.cargo`
     （从镜像 `/usr/local/cargo` 播种）+ `RUSTUP_HOME=/usr/local/rustup`，两个缓存目录宿主持久化，
     供离线轮次无网络复用（离线容器还要能通过 rust-toolchain.toml 的 rustfmt/clippy 校验，
     否则 rustup 会在每次调用时尝试联网补齐而失败）。
  3. **传输镜像只换来源，不换内容**：本环境 `deb.debian.org`(http) 不可用、`static.rust-lang.org` 握手失败、
     `static.crates.io` 极慢且常失败、`registry.npmjs.org` 不可达。默认走 apt `https://mirrors.ustc.edu.cn`
     （`debian` 与 `debian-security` 是两条不同路径，不能统一替换）、rustup `https://rsproxy.cn`、
     crates sparse `https://rsproxy.cn/index/`（`CARGO_SOURCE_CRATES_IO_REPLACE_WITH`，即 ADR-010 机制）、
     npm `https://registry.npmmirror.com`、Node 二进制 npmmirror。内容校验依旧：rustup sha256 清单、
     npm `package-lock` integrity、cargo `Cargo.lock` checksum；`Cargo.lock` 的 source 仍是官方 crates.io。
     容器内 Node 由 v22.20.0 提至 **v22.22.2**（`jsdom@30` 声明 `engines.node ^22.22.2`）。
  4. **样例备份的 DB 不在版本控制内**：`.gitignore` 的 `*.sqlite3` 使
     `artifacts/web-mvp/t20-rd/sample-backup/database/manual.sqlite3` 在全新 checkout 里缺失
     （manifest/SHA256SUMS 在、DB 不在）→ §7 第 2 步必然失败。新增 `scripts/linux-musl.sh
     --prepare-sample-backup`：容器内用 T20 造数用例（`prepare_rehearsal_datadir`）+ 正式二进制 `backup`
     重新生成自洽备份；宿主脚本在仓库备份不完整时**拒绝覆盖**构建目录里的完整备份（否则 manifest 与 DB 对不上）。
     CI 的 `dist-musl` job 同步补了该步骤。**这是"环境缺文件"，不是产品缺陷**。
  5. **挂载点变化必须清 `target/`**：build script 二进制把编译期 `CARGO_MANIFEST_DIR` 烧进
     `cargo:rerun-if-changed=<旧路径>/migrations`，cargo 指纹只看源码 mtime → 会复用旧 build script，
     `build.rs` 按旧路径找 `apps/web/dist` 并 panic。宿主脚本用缓存目录里的 `work-mount.txt` 检测并清理。
  6. **修订 ADR-035 第 3 条（动态依赖采集判据）**：原判据 `target == host` 会把
     `x86_64-unknown-linux-gnu` 主机上的 `x86_64-unknown-linux-musl` 产物标成"跨构建→未采集"，
     而 §6 与 T22 交接要求每平台依赖清单。改为**同架构同 OS 即采集**（gnu→musl 只差 libc/ABI，
     且产物就在同一环境被 `smoke` 真实运行）；跨架构/跨 OS 仍是"未采集"。`build-info.json` 保留真值
     `crossCompiled: true`、`sameArchOs` 语义字段 `samePlatformAsBuild: false`，不制造"原生 triple"假象；
     `dynamic-dependencies.txt` 改为不截断输出 `readelf -d` 并给出 `DT_NEEDED` 计数。
  7. **macOS 宿主脚本的 bash 3.2 会把全角标点当变量名字符**：`"$VAR）"` 解析为变量 `VAR）` →
     `unbound variable`（真实踩到）。两脚本中"变量紧跟全角标点"处统一加 `${}`。
- 影响：
  1. 后续调 `scripts/linux-musl.sh` 时不要改挂载点短名、不要把 `CARGO_HOME` 挪出 `HOME`；
     改了就重跑并复核 `build-info.json.isolation.hits` 与 `remappedHomePathHits`。
  2. Linux 侧新增三个可用入口：`--offline-only`（离线证据）、`--prepare-sample-backup`（备份重建）、
     `--check`（Linux 侧回归）、`--check-reproducible`（可复现）。
  3. `dist` 的"同架构同 OS"判据对 macOS 无影响（target==host 仍成立）。
- 未验证边界与下一步：
  1. **新发现（未修复）**：`crates/server/tests/backup_restore.rs::legacy_schema_backup_restores_and_migrates_automatically`
     在 Linux 容器内必现失败（3/3），原因是 `run_serve` 打印 `listening on …` 之后才把
     `shutdown_signal()` 交给 `axum::serve(...).with_graceful_shutdown(...)`，而 tokio 的 SIGTERM 处理器
     **在被 poll 时才注册** → 该窗口内 SIGTERM 走默认动作杀进程（退出码 None，测试期望 0）。
     同容器内 `config_cli.rs` 的同类 SIGTERM 用例通过，说明稳态停服正常。属产品代码范围，
     本卡未改（沿用"产品代码零改动"口径，且改产品代码会使已交付的 macOS 证据需要重跑）；
     建议由 PM/协调者决定是否作为 T20/T21 缺陷派发。
  2. 容器是 Rosetta 模拟执行 x86_64，不是真实 x86_64 硬件：架构相关的 `file`/`ldd` 结论可采信，
     性能类结论不得引用；若需真实硬件证据，应在 x86_64 Linux 机器上重跑同一脚本（脚本已参数化）。
  3. 浏览器矩阵（Firefox/Edge）、真实 Provider（T23）、签名/公证仍为未覆盖项。
- 相关链接：`scripts/{linux-musl,container-linux-musl}.sh`、`xtask/src/dist.rs`、`.github/workflows/ci.yml`、
  `docs/operations.md`；证据：`artifacts/web-mvp/t22-rd/linux/**`；实现记录：`requirements/web-mvp/implementation.md` §T22-13。

### T22 Linux 半边验收知识（追加，2026-09-13；QA 回合 30：AC-064 Linux / AC-002 / AC-059 Linux 三条必选独立复现通过，切片 **PASS**；新增未关闭 **BUG-013**（P3，跨卡））

- 状态：QA 独立复现与定性结论，供 T23、真机复跑与测试硬化复用。完整证据见
  [QA 回合 30 报告](requirements/web-mvp/qa-report.md)；本轮未改任何命令合同语义。
- **"原生运行"要分三层说（本轮核心判据）**：① 目标 OS 内核；② 目标 ABI 用户态；③ 物理 CPU。
  `--platform linux/amd64` 在 Apple Silicon 上只满足 ①②（x86_64 用户态由 Rosetta 翻译），
  ③ 未满足。因此：`file`/`ldd`/`readelf`/sha256 这类**产物自身属性**可采信；
  **任何性能结论不得引用**（本轮 RD/QA 都没有引用，已逐处核对）；需要 ③ 时用同一参数化脚本在真机重跑。
  另：`build-info.json.crossCompiled=true` 只表达 **triple 差**（gnu 构建机 → musl 目标），
  **不表达跨 OS**——不要把它读成"在别的 OS 上交叉编译"。
- **启动协议行先于信号处理器注册（跨卡陷阱）**：`run_serve` 先打印 `listening on …`，
  之后 `shutdown_signal()` 才被首次 poll → 期间 SIGTERM 走默认动作。实测窗口 **≈50–100 ms**
  （≤50 ms → 退出码 143；≥100 ms → 0）。`crates/server/tests/storage.rs:1817` 已知此坑并用
  300 ms settle 规避；**`backup_restore.rs` / `config_cli.rs` 未 settle**，故 Linux 容器内
  `legacy_schema_backup_restores_and_migrates_automatically` **必现失败（3/3）**（→ BUG-013）。
  **后续任何"读到协议行即发信号"的测试都必须先 settle**，否则在慢环境（容器/Rosetta）从偶发变必现。
- **重建型 fixture 的验收三步问法**（本轮样例备份场景，可复用于任何 T20 式 fixture）：
  ① 生成器是否**独立于**被测路径（此处＝仓库自己的 T20 造数用例 + 正式 `backup`，不是 smoke 的产物）；
  ② 差异是否只落在**非语义字段**（本轮 11 个 blob 中 8 个逐字节相同，3 个仅 `assetId`/`documentId`/
  `preparationId` 时间戳不同）；③ 断言是否仍**跨三方**（备份清单 ↔ 恢复结果 ↔ 实际下载字节）。
  三条都满足即等价；但"与版本控制里那份逐字节相同"这条身份断言会丢，**必须显式登记**。
  附带事实：`artifacts/web-mvp/t20-rd/sample-backup/database/manual.sqlite3` 因 `.gitignore` 的
  `*.sqlite3` **不在任何 checkout 里**，macOS 侧复跑同样需要先重建（`docs/operations.md` §1）。
- **复跑取证的纪律**：`scripts/linux-musl.sh` 的 `ARTIFACT_DIR` **硬编码**为 `artifacts/web-mvp/t22-rd/linux`，
  QA/他人复跑会覆盖 RD 同名证据 → 复跑前**快照**、跑后**按快照还原**并 `diff -r` 校验（本轮已做，
  逐字节还原通过）。建议后续支持 `EM_LINUX_ARTIFACT_DIR` 覆盖。
- **隔离扫描的挂载点命名**：扫描把"仓库根"当**子串**搜，短名挂载点会自撞（`/build` 撞 remap 目标
  `/build/home`；`/work` 撞依赖 panic 路径 `.../worker.rs`）。正解是**深路径**（`/src/everything-manual`），
  **不是**放宽扫描子串语义；`CARGO_HOME` 必须在 `$HOME` 之下，否则路径既不归一化也不被拒绝。
- **脚本静默失败面**：`cmd | grep -c "X" || true` 在工具本身失败时会打印 `0` 且返回成功
  （本轮 `readelf -d … | grep -c "(NEEDED)"`）；`step()` 必须用 `${PIPESTATUS[0]}` 取真实退出码，
  失败路径要"先回收日志再非零退出"。以上属本轮 P4（OB-20/OB-21）。
- **取证手法（可复用）**：① 判据权威性——不采信产物自述，QA 在容器内**自算**禁用模式命中数与
  `DT_NEEDED`；② provenance——QA 独立重跑 `dist --check-reproducible`，比对交付 sha256，
  并要求重建的 `build-info.json` 与交付件**除 `builtAt` 外逐字段一致**；③ §7 第 7 步"无 Node/Python/源码"
  的**直接**证据——裸 `alpine` 容器只挂一个二进制做 init/serve/资源/404/停服
  （`artifacts/web-mvp/t22-qa/qa-r30-coldstart-bare-container.sh`），不再靠"环境清理 + 静态链接"推断。
- **未覆盖边界（本轮记录，不冒充通过）**：物理 x86_64 硬件；真实 HTTPS（静态 musl 不含系统 CA，
  真实 TLS 信任链属真实链路与运维声明）；Firefox/Edge（AC-063）；签名/公证；`--check-reproducible`
  只做"重链接"不做依赖重编（与 macOS 同口径）。
- 相关代码／证据：`crates/server/src/config/commands.rs`、`crates/server/tests/{backup_restore,storage,config_cli}.rs`、
  `xtask/src/{dist,smoke}.rs`、`scripts/{linux-musl,container-linux-musl}.sh`、`artifacts/web-mvp/t22-qa/**`、
  `artifacts/web-mvp/t22-rd/linux/**`；实现记录：`requirements/web-mvp/implementation.md` §T22-13。
