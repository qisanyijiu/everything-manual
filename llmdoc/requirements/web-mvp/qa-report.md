# web-mvp QA 验收报告

**结果：PASS（仅 T01 切片范围，不代表产品全量）** · 回合：1 · PRD 修订：1（ui_revision 1）· 范围：切片（task_ids=[T01]，AC-001、AC-011）

- QA 执行时间：2026-09-12 01:05–01:16（本地 UTC+8）；执行者：QA 子 agent，独立复现，未采信 RD 报告结论。
- 派发核对：PRD 修订 1 与 `prd.md` §9.1 修订记录一致（含 UI 同修订补充）；`implementation.md` 状态 RD_READY；无历史缺陷。
- 结论口径：本报告只覆盖 T01 派发范围；T02–T23 未实现，不作为本回合缺陷。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb44ee2a6088f440eade3f1424546fbb62d`（"init"，仅 1 次提交）；T01 交付物**尚未提交**（`git status` 中 `.cargo/`、`Cargo.toml`、`Cargo.lock`、`crates/`、`xtask/`、`apps/`、`contracts/`、`rust-toolchain.toml` 均为未跟踪；`build-info.json` 如实记录 `git.dirty=true`） |
| 平台 | macOS Darwin 25.6.0 · aarch64-apple-darwin（本机即 PRD §5.6 必选平台之一） |
| 工具链 | rustc/cargo 1.98.1（`rust-toolchain.toml` 固定值，rustup 自动切换生效）；node v26.0.0；npm 11.12.1 |
| 交付二进制 | `dist/aarch64-apple-darwin/everything-manual`，2040688 bytes，sha256 `69576fb9d0a415420776631d363e5bbc34a23e2becc9c5b2dff9b6c5b42c6393`（QA 独立重建产物与 RD 产物逐字节一致） |
| 数据／fixture | 无（T01 无 data-dir、无 Provider）；全程零付费 API 调用；构建期仅 npm registry 与 crates 索引网络访问（合同允许） |
| 未验证（非本回合范围） | Linux musl 目标与 AC-064（T22）、正式 `xtask smoke`／AC-002（T22）、Playwright／AC-063（T21）、真实 Provider（T23）、T02+ 功能 |

## AC 验收矩阵

| AC ID | 期望 | 实际 | 结果 | 命令／步骤／证据 |
| --- | --- | --- | --- | --- |
| AC-001 | `cargo xtask dist --target <本机target>` 产出单可执行文件 + SHA256 + licenses + build-info；随后在只含该二进制的新临时目录跑 `cargo xtask smoke-bootstrap` 退出 0，无源码无 Node 可运行 | dist 退出 0，产出 4 件套；QA 独立重建哈希与既有产物一致；smoke-bootstrap 退出 0（7 项 HTTP 检查全过）；QA 另以 `env -i PATH=/usr/bin:/bin` 在仅含二进制的目录手工启动成功 | **PASS** | 01:14 `cargo xtask dist --target aarch64-apple-darwin` → sha256 69576fb9…；01:14 `cargo xtask smoke-bootstrap --binary $PWD/dist/aarch64-apple-darwin/everything-manual` → exit 0；01:15 手工冷目录探测；证据 `artifacts/web-mvp/dist-run.log`、`smoke-bootstrap.log`、`dist-hash-before.txt` |
| AC-011 | 干净工作树 `cargo xtask contracts --check` 退出 0 且不改工作树；改动生成文件后非零退出；`npm --prefix apps/web run typecheck` 通过；前端类型来自生成文件 | check 退出 0，生成在临时目录逐字节比较，63 文件 sha256 清单前后完全一致；对 `generated.ts` / `openapi.json` 分别注入漂移均退出 1 并指名漂移文件，恢复后回到 0；typecheck 退出 0；`client.ts` 仅从 `./generated` 取类型 | **PASS** | 01:13–01:14；证据 `contracts-check-1.log`、`contracts-check-neg1.log`、`contracts-check-neg2.log`、`contracts-check-restored.log`、`manifest-before.txt`　`manifest-after-check1.txt`、`typecheck.log` |

### 卡内检查项（T01 卡 + validation-release.md §2）

| 检查项 | 实际结果 | 判定 | 证据 |
| --- | --- | --- | --- |
| `cargo test -p everything-manual --test bootstrap` 真实执行且测试数 > 0 | exit 0，`5 passed; 0 failed; 0 ignored`；另跑 `--features embedded-ui` 得 `8 passed` | PASS | `bootstrap-test.log`、`bootstrap-test-embedded.log` |
| `cargo xtask check` 各步骤真实通过、无 `|| true` 短路 | 7/7 步骤 `[通过]`，exit 0；全仓库 grep 无 `|| true`／吞错模式；注入合同漂移后 exit 1，仍先执行并报告其余 6 步 `[通过]`，随后 `[失败] cargo xtask contracts --check` | PASS | `xtask-check.log`、`xtask-check-drift.log` |
| 未知 `/api/*` 返回 JSON 404（非 index.html）；SPA fallback 只服务 HTML 导航 | 实测 `/api`、`/api/unknown`、`POST /api/unknown` → 404 `application/json`（`error.code=NOT_FOUND`、requestId 为 UUID）；`/library/some-item` → 200 HTML；`/assets/definitely-missing.js`、`/favicon.ico` → 404 非 HTML。无扩展名 `/assets/foo` → 200 HTML，见非阻断建议 2 | PASS | 01:15 冷目录 curl 探测；`smoke-bootstrap.log`；单元用例 5 条 |
| `embedded-ui` 缺 dist 必须编译报错（负例，验证后恢复） | 移走 `apps/web/dist` → `cargo build -p everything-manual --release --locked --features embedded-ui --target aarch64-apple-darwin` exit 101，build.rs panic：「embedded-ui 需要前端构建产物……不允许在缺少前端产物时产出空壳二进制」；同一时刻普通 `--test bootstrap` 仍 exit 0（证明单测不要求 dist）；恢复 dist 后重建成功且哈希不变（69576fb9…） | PASS | `embedded-missing-dist.log`、`bootstrap-test-nodist.log`；目录已复位（无 `dist.qa-bak` 残留） |
| `build.rs` 不跑 npm、不联网 | 源码审查：仅读文件系统 + 打印 `rerun-if-changed`；`crates/server` 无 `[build-dependencies]`，不具备联网依赖；dist 日志中 npm 仅由 xtask 在构建前段调用 | PASS | `crates/server/build.rs`；`dist-run.log` |
| 生成物确定性（独立再生成比对） | ① `contracts --check` 于临时目录重生成后与工作树逐字节一致；② QA 独立 `cargo xtask dist`（含 npm ci + vite build + release 编译）产出与既有产物哈希一致；③ 恢复 dist 后的 release 重建哈希第三次一致 | PASS | `dist-run.log`、`contracts-check-1.log` |
| 前端 lint/test 脚本真实存在且非 0 测试 | lint exit 0（`eslint . --max-warnings=0`）；vitest `2 files / 5 tests passed / 0 skipped` | PASS | `lint.log`、`vitest.log` |
| 附加：单二进制冷启动与依赖 | `file`=Mach-O arm64；`otool -L` 仅 `/usr/lib/libiconv.2.dylib`、`/usr/lib/libSystem.B.dylib`（无 Homebrew 路径、无 Node）；`env -i PATH=/usr/bin:/bin` 下正常服务 | PASS | 01:15 冷目录运行记录 |
| 附加：版本／合同一致性 | Cargo.toml、Cargo.lock、package.json、openapi `info.version`、build-info 均为 0.1.0；openapi 3.1.0，health 路径与错误 schema 符合 contracts.md §1（camelCase、`{data}` 单项包装、`error.code/message/details/requestId`）；ADR-010 与 architecture.md §3/§4 一致（SQLx 0.9 → MSRV 1.94；rust-embed 8 仅发布路径要求 dist；build.rs 不跑 npm／联网） | PASS | `contracts/openapi.json`、`dist/aarch64-apple-darwin/build-info.json`、`Cargo.toml` |

## 缺陷

无。本回合 0 个 OPEN 缺陷（AC-001、AC-011 及全部卡内必测项通过）。

### 非阻断建议（不计入阻断项，不要求 T01 返工）

1. **licenses.json 覆盖面（建议 T22 处理）**：dist 产物 `licenses.json` 实际含 **139** 个 package，而 Cargo.lock 有 **154** 个；差额包括 rust-embed 8.12.0 及 walkdir/same-file/unicase/mime_guess/winapi-util、sha2 0.11 系等 15 个——其中 rust-embed 树是被编译进发布二进制的组件，未出现在许可证汇总中。原因：`cargo xtask dist` 用不带 `--features embedded-ui` 的 `cargo metadata --locked`。计划已把「LICENSE 清单／完整许可证文本与合规检查」划归 T22（implementation-plan T22 允许范围；`licenses.json` 自带 note；ADR-010 未验证边界），T01/AC-001 只要求“产出 licenses”，故不阻断。建议 T22 用与发布相同的 feature 集生成清单。另：RD `implementation.md` §5 表格中“154 个 package”与产物实际 139 项不符，建议 RD 顺手修正该表述。
2. **SPA 导航边界（建议 T21 回归前明确）**：导航判定按“路径末段是否含 `.`”，因此 `/assets/foo`（无扩展名）返回 200 HTML。带扩展名的缺失资源（`/assets/*.js` 等）已正确 404 且不返回 HTML，嵌入查找为内存 map 查询、无路径穿越风险。影响：validation-release §7.3「未知非导航静态资源不得返回 HTML200」在 `/assets/<无扩展名>` 这一形态上不成立。建议在 T04/T08 固化路由表时收紧（如 `/assets/` 前缀一律不作导航），或由 PM/UI 明确该启发式为可接受语义。
3. **embedded-ui 代码路径不在 `cargo xtask check` 覆盖内**：check 的 clippy/测试均未启用该 feature（与 T01 卡要求一致），`embedded.rs` 与 4 条内嵌用例只在显式 `--features embedded-ui`（需 dist 在场）时编译执行；QA 手工运行该命令得 `8 passed`。建议 T21/T22 将其纳入 check／发布流水线，避免后续改 `embedded.rs` 时本地 check 全绿而发布路径编译失败（dist 会挡住，但反馈更晚）。

## 非代码知识与限制

- **“干净工作树”的解释（重要）**：T01 交付物尚未 git 提交，`git status` 无法作为“未修改工作树”的证据。QA 以 63 个源／生成文件（含 `contracts/openapi.json`、`apps/web/src/api/generated.ts`）的 sha256 清单在 `contracts --check` 前后逐字节比对：完全一致，且临时目录已清理。提交时点由协调者决定；建议在提交后（尤其 T22 发布验收前）复跑一次 `contracts --check` 以消除“未提交态”歧义（成本约 1 秒）。
- **负例现场制作并已全部恢复**：`generated.ts` 与 `openapi.json` 分别追加内容 → exit 1 并指名漂移文件；恢复后哈希回到 `e1561d31…` / `713df537…`，最终 63 文件清单与初始清单完全一致；`apps/web/dist` 移走验证后已复位；无残留临时目录（`everything-manual-smoke-*`、`-contracts-check-*` 均已清理）、无遗留进程（`pgrep everything-manual` 为空）。
- **环境差异**：本机 `~/.cargo/config` 为 git 索引（受限网络不可达），项目 `.cargo/config.toml` 覆盖为官方 sparse 索引；QA 运行未依赖外部镜像（依赖已缓存）。cargo 对 `~/.cargo/config` 的 deprecated 警告为环境噪音，不影响结论。
- **可接受限制（本回合范围外，已记录不冒充通过）**：AC-002 正式 smoke、Linux musl、Playwright/e2e、真实 Provider 与费用、T02+ CLI/DB/认证均未验收；T01 通过不代表产品通过。
- **跨卡复用测试陷阱**：已同步 `llmdoc/decisions.md`「QA 验收知识」条目（未提交态的清单比对法、check 不覆盖 embedded-ui feature、licenses 缺 feature 依赖、SPA 无扩展名边界）。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 2026-09-12 | slice | T01（AC-001、AC-011） | 1（ui_revision 1） | PASS | 本文件 |

- 交接给 RD：无缺陷需修复。
- 交接给协调者：建议将 `{task_id: T01, prd_revision: 1, qa_round: 1, report: llmdoc/requirements/web-mvp/qa-report.md, accepted_ac_ids: [AC-001, AC-011], status: accepted}` 记入 `accepted_tasks`，并派发 T02；非阻断建议 1/3 转 T22，建议 2 转 T21（或按 PM 决定）。
- 证据目录：`artifacts/web-mvp/`（21 个文件：命令日志、哈希清单、篡改前备份、冷目录探测记录）。

---

# 回合 2 · T02

**结果：PASS（仅 T02 切片范围，不代表产品全量）** · 回合：2 · PRD 修订：1（ui_revision 1）· 范围：切片（task_ids=[T02]；AC-005、AC-006、AC-012 配置侧）

- QA 执行时间：2026-09-12 02:17–02:21（本地 UTC+8；日志内时间戳为 UTC）；执行者：QA 子 agent，独立复现，未采信 RD 报告结论。
- 派发核对：PRD 修订 1 与 `prd.md` §9.1 一致（含 UI 同修订补充）；`implementation.md` 状态 RD_READY；`state.yaml` qa_round=2、scope=slice、current_tasks=[T02]；进入时 open_defects 为空，与本回合结论一致。
- 结论口径：只覆盖 T02 派发范围。T03–T23 未实现内容（数据库 schema 检查、`/settings/status`、真实 backup/restore、内置 TLS 监听等）不计为本切片缺陷，列在"未覆盖边界"。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb44ee2a6088f440eade3f1424546fbb62d`（"init"）；T01/T02 交付物**仍未 git 提交**（`git status` 基线：RD 改动 `.gitignore`、`llmdoc/decisions.md`，协调者改动 `state.yaml`，其余为未跟踪目录） |
| 平台 | macOS Darwin 25.6.0 · aarch64-apple-darwin |
| 工具链 | rustc/cargo 1.98.1（`rust-toolchain.toml` 固定）；node v26.0.0；npm 11.12.1 |
| 被测二进制（手工负例/边界） | QA 现场重编译 `target/debug/everything-manual`，sha256 `c919e595…094ff`（与源码同步：`cargo build` 无重编译动作） |
| 被测二进制（发布路径） | `dist/aarch64-apple-darwin/everything-manual`，3 467 184 bytes，sha256 `f5886c5b542dd968c686cb46a7b04ac339d51297e34135c9757d1ed5b28bc297`；QA 连续两次独立 `cargo xtask dist` 复现同一哈希 |
| 数据／凭据 | 全部为 QA 自建临时 data-dir 与 canary 假凭据（`canary-*`）；`config-cli` 用例自建自清临时目录；零真实 Provider 调用、零费用 |
| 工作树完整性 | 79 个源／测试／生成文件的 sha256 清单在验收前后**逐字节一致**；`git status` 与基线一致；无残留进程、无残留锁文件、无残留 smoke 临时目录 |

## AC 验收矩阵

| AC ID | 期望 | 实际 | 结果 | 命令／步骤／证据 |
| --- | --- | --- | --- | --- |
| AC-005 | `cargo test -p everything-manual --test config_cli` 覆盖 init/serve/check 分派、未知配置键报错、缺密钥不启 mock、第二个进程持同一 data-dir 失败退出、日志不出现敏感字段、backup/restore 未实现非零 | QA 独立执行：exit 0，**12 passed / 0 failed / 0 ignored**（0.80s），12 个用例与卡内要求一一对应（含 `--password` 不存在、宽权限密码文件被拒、锁冲突、canary 日志脱敏） | **PASS** | 02:17 `cargo test -p everything-manual --test config_cli`；证据 `artifacts/web-mvp/t02-qa/ac005-config-cli.log` |
| AC-005（手工负例复核） | 上述行为在真实二进制上可复现（不只看测试计数） | QA 用现场二进制逐项复现：`--help/--version`=0；`frobnicate`/无参数/`--password`=2（可读报错、无 panic）；缺 data_dir=3；未 init 目录=4；宽权限密码文件=3；`backup`=7 且**不创建输出**；`restore`=7 且**不创建目标**；TOP 与嵌套未知键=3 并指认键名（含行列） | **PASS** | 02:17 手工脚本；证据 `manual-cli-run1.log`（含每步 exit code 与原始输出） |
| AC-005（锁） | 第二个进程持同一 data-dir 必须失败退出 | 第一个 `serve` 运行中：第二个 `serve` **exit 5**，报"正被另一个进程使用"+持有者 pid；`init` 并发同样 exit 5；`check` 运行中给"排他锁：警告"但 exit 0（运行中属合法状态）；`kill -9` 后锁由内核自动释放，`check` 回到"排他锁：通过"，服务可立即重启 | **PASS** | 02:17–02:18 `manual-cli-run1.log`；`config_cli.rs::second_process_on_same_data_dir_fails_fast` |
| AC-005（日志脱敏） | 日志不出现密码/密钥/查询串 | canary 复核：`init` 密码 canary 不出现在 stdout/stderr/日志；API key canary（经 `api_key_env` 注入）与两次带查询串请求（`?sig=canary-…`）的 canary **在日志与 stdout 中均为 0 次命中**；请求日志含 `requestId/path/status/errorCode/durationMs` 且 path 已去查询串 | **PASS** | 02:19 `manual-log-redaction-run3.log`；`config_cli.rs::logs_never_contain_secrets_or_query_strings` |
| AC-005（init 密码输入） | 交互不回显或 0600 受限文件；不进 argv／shell history | QA 用 pty 独立复现：交互提示两次且**输入未回显**（输出中无密码字面量），一致=0、不一致=2 且"未做任何初始化"（未创建目录）；`--password` 参数被 clap 直接拒绝（tip 指向 `--password-file`），代码中不存在其它密码参数形态；0600 文件=0，0644=3（提示 chmod 600），缺失=3，短于 8 字符=2 | **PASS** | 02:18 `manual-pty-init.log`；02:17 `manual-cli-run1.log`；`crates/server/src/config/cli.rs` |
| AC-006 | 无 TLS 且无可信代理时非 loopback `serve` 拒绝启动、退出码非零 | `serve --listen 0.0.0.0:18099` **exit 6**、stderr 给出"必须 TLS 或 trusted_proxy_cidrs"、不打印 listening；`check --listen 0.0.0.0:…` 同规则 exit 6；显式 `trusted_proxy_cidrs` 后非 loopback 可启动（loopback 访问 health 200）；非法 CIDR=3；TLS 未成对=3 | **PASS** | 02:17–02:18 `manual-cli-run1.log`；`config_cli.rs::non_loopback_listen_requires_tls_or_trusted_proxy` |
| AC-006 | `check --data-dir` 全程不发起任何外部 HTTP（记录为 0） | QA 自建计数监听器（TcpListener）并把两个 Provider 的 `base_url` 指向它、密钥注入使状态为"已配置"：`check` exit 0、输出"本命令不发起任何网络请求"、**监听器收到 0 个连接**；`serve` 启动阶段同样 0 连接。静态旁证：`crates/server` 依赖树**不含任何 HTTP 客户端或 TLS crate**（无 reqwest/hyper client/rustls），T02 服务端在编译期就不具备外呼能力 | **PASS** | 02:18 `manual-zero-http.log`；`cargo tree -p everything-manual`；`config_cli.rs::check_and_serve_startup_make_no_external_http_requests` |
| AC-012（配置侧，仅配置层语义） | 缺 Provider 密钥时不存在 mock 回退、不产生假成功 | 无密钥：`check` 如实输出 `providers.tripo/manual_ai = 未配置（缺少…）` 且全输出无"已配置"、无 mock 字样；`serve` 正常启动（health 200）并记录 `provider_not_configured`，**不存在 `provider_configured` 事件**；`api_key_env` 命名但未导出 → 仍未配置（无隐式默认、无 fake）；代码库 grep 无任何 mock/fake Provider 实现；业务端点（`/api/v1/items`、`/estimates`、`/jobs`、`/settings/status`）当前一律 JSON 404，**不存在假成功路径**；密钥注入后 `check` 输出"已配置（密钥来源：环境变量 X）"，密钥字面量不出现在输出与日志 | **PASS（仅配置侧）** | 02:17–02:20 `manual-cli-run1.log`、`manual-log-redaction-run3.log`；`config_cli.rs::missing_provider_keys_start_without_mock_fallback`；AC-012 的 `/settings/status` 与 estimate/jobs 部分**未验**（T04/T11） |

### 卡内检查项（T02 卡 + validation-release.md §5）

| 检查项 | 实际结果 | 判定 | 证据 |
| --- | --- | --- | --- |
| 子命令分派 `init/serve/check/backup/restore` | 5 个子命令均存在且行为符合卡文；`backup/restore` 固定 exit 7、说明含"未实现/T20/不会创建或覆盖任何文件"，实测**未创建** `--out` 与恢复目标目录 | PASS | `manual-cli-run1.log`、`config_cli.rs` |
| 未知配置键报错（任意层级） | 顶层 `totally_unknown_key` 与 `[providers.tripo] bogus_field` 均 exit 3，stderr 带 TOML 行列定位与键名；合法配置照常通过 | PASS | `manual-cli-run1.log` |
| 配置优先级 CLI 非密钥项 > 环境变量 > TOML > 默认 | 四层可观察：默认 `127.0.0.1:8080` → TOML `:1111` → `EM_LISTEN :2222` → `--listen :3333`；`data_dir` 方向另有 CLI > TOML、环境变量 > TOML 用例；`--config`/`EM_CONFIG` 定位生效，缺失配置路径=3 | PASS | `manual-config-precedence-run2.log`、`config_cli.rs::config_precedence_cli_env_toml_default` |
| 未知子命令/错误路径可读报错非 panic | 全部负例 stderr 为可读单行中文（`错误：…`），无 `panicked at`、无堆栈、无 SQL；`--password` 走 clap 用法错误 exit 2 | PASS | 各手工日志；测试 `run()` 内置 panic 断言 |
| 退出码约定 0/1/2/3/4/5/6/7 | 逐码实测：0（成功/help/version）、1（端口被占用：Address already in use）、2（无参数/未知子命令/`--password`/非交互无密码源/密码不一致/短密码）、3（未知键/非法值/配置与密码文件问题/缺 data_dir/TLS 未成对/price_catalog 缺失/api_key_env 与 api_key_file 同时设置/密钥文件权限过宽）、4（目录不存在/结构不完整/不可写写探针）、5（锁冲突）、6（非 loopback 无安全配置/TLS 已配置但未实现）、7（backup/restore） | PASS | `manual-cli-run1.log`、`manual-config-precedence-run2.log`、`manual-log-redaction-run3.log` |
| data-dir 结构与锁 | `init` 幂等创建 `tmp/ logs/ blobs/ lock`（目录 0700、锁文件与日志 0600，实测 `drwx------`/`-rw-------`），**不创建 `manual.sqlite3`**，不清空已有内容；重复 init 成功；`serve` 对空目录/缺结构目录 exit 4 并提示先 init（不隐式创建） | PASS | 02:17 `manual-cli-run1.log`（含 `ls -la`/`stat` 输出） |
| 缺密钥不回落 mock（REQ-007 配置侧） | 见 AC-012 行；另：`api_key_file` 0600 可用、0644 拒绝、与 `api_key_env` 同设报错；密钥内容（canary）不出现在 `check` 输出与日志 | PASS | 02:19–02:20 手工复核；证据 `provider-config-edge-cases.log` |

### T01 回归（RD 改动了 xtask/smoke.rs 与 CLI，需确认 AC-001/AC-011 证据链未破坏）

| 检查项 | 实际结果 | 判定 | 证据 |
| --- | --- | --- | --- |
| `cargo test -p everything-manual --test bootstrap` | exit 0，**5 passed**（health JSON 精确相等、ready 自检、`/api/unknown` JSON 404、非 API 未知路径非 HTML） | PASS | 02:18 终端输出 |
| `cargo test -p everything-manual --features embedded-ui --test bootstrap`（dist 在场） | exit 0，**8 passed**（含 4 条内嵌用例） | PASS | `bootstrap-embedded-ui-qa.log` |
| `cargo xtask contracts --check` | exit 0，`[一致]` 两项，未修改工作树（前后 79 文件哈希清单一致） | PASS | 02:18 终端输出；`manifest-before.txt`/`manifest-after.txt` |
| `cargo xtask check` | exit 0，**7/7 步 `[通过]`**（fmt/clippy/test/前端 lint/typecheck/test/合同），workspace 测试 53 条全绿（server lib 34 + bootstrap 5 + config_cli 12 + core 2） | PASS | `artifacts/web-mvp/t02-qa/xtask-check-qa.log` |
| `cargo xtask dist --target aarch64-apple-darwin`（两次） | 均 exit 0，两次独立重建 sha256 完全一致 `f5886c5b…`，与 RD 交付产物一致（3 467 184 bytes），`deterministic-timestamps` 机制未破坏 | PASS | `dist-run-1.log`、`dist-run-2.log` |
| `cargo xtask smoke-bootstrap --binary <绝对路径>` | exit 0；冒烟目录仅含二进制 + 其 data-dir + 临时密码文件；`init` → `serve`（`--listen 127.0.0.1:0`）后 8 项 HTTP 检查全过（/ HTML、hashed JS、health/live、health/ready、`/api/unknown` JSON 404、SPA 深链接、缺失资源 404 非 HTML）；结束时自身进程与临时目录已清理（QA 复查无残留 `everything-manual-smoke-*`） | PASS | `smoke-bootstrap-qa.log` |

## 缺陷

无。本回合 0 个 OPEN 缺陷（AC-005、AC-006、AC-012 配置侧及全部卡内必测项、T01 回归均通过）。

## 未覆盖边界／已知限制（本回合记录，不冒充通过）

1. **内置 TLS 监听未实现（RD 自报限制，QA 判定：不阻断本切片）**
   - 实测行为：只要配置 `tls.cert_file/key_file`（无论监听地址），`serve` 与 `check` 均 **exit 6**，报"内置 TLS 监听尚未实现；请移除 tls 配置或改用 trusted_proxy_cidrs"；不降级明文、不打印 listening。TLS 只配一半 → exit 3（必须成对）。
   - 是否触碰本切片必选 AC：**否**。AC-006 只要求"无 TLS 证书且无可信代理配置时非 loopback 拒绝启动且退出码非零"，该路径实测通过；AC-005/AC-012 不涉及 TLS 监听。T02 卡交接项只要求"说明内置 TLS 证书输入与反向代理边界"，已实现（配置键被接受并明确拒绝，示例配置写明）。
   - 是否与 `architecture.md` §7 冲突：**不冲突**。§7 的承诺是"外网／局域网部署必须认证和 TLS，可用内置 rustls 证书文件**或**明确受信任的反向代理"；当前"拒绝启动 + 反向代理为唯一放行路径"属于 fail-closed，没有违反任一承诺（未认证/未加密不提供服务）。PRD §5.1 同样为"或"关系（A-01 同）。
   - 是否可带限制进入后续卡：**可以，但必须显式指派属主**。ADR-011 将实现点写为 T22，但 `llmdoc/implementation-plan.md` 的 T22 卡文只列"TLS 依赖检查"，**没有任何任务卡明确包含"实现内置 TLS 监听"**；若不在发布前处理，交付时 PRD §5.1/architecture §7 中"内置 rustls 证书"这一可选路径会变成对外承诺未兑现项。
     → 非阻断建议 1（建议处理卡位：由协调者/PM 在 T04 之后、T22 之前显式指派，或修订 PRD/architecture 明确 MVP 仅支持反向代理模式）。
2. **反向代理模式的来源校验与认证属 T04**：T02 允许在配置 `trusted_proxy_cidrs` 后绑定非 loopback，但当前不消费 `X-Forwarded-*`、也尚无认证（T04 才有）。当前影响极小（除 health 外全部 JSON 404），但**在 T04 完成前不建议把服务暴露到非 loopback**；这是调度风险提示，不是 T02 缺陷（T02 卡文明确要求该放行行为）。
3. `check` 的副作用（已由 RD 在 ADR-011/T02-7 声明并实测一致）：会向 `<data-dir>/logs/everything-manual.log` 追加一行日志并在 `tmp/` 建删写探针；实测除既有日志文件外**未新增文件**。若后续运维要求只读检查，可考虑 `--dry-run`（非阻断建议 2）。
4. 未验收内容（属其它卡，不能解读为通过）：数据库 schema 检查与建库（T03）、`/settings/status` 与认证（T04）、estimate/jobs 的未配置错误（T11/T12）、真实 backup/restore（T20）、日志轮转（T22）、内置 TLS 监听（无卡）、非 Unix 平台（Windows 属扩展平台，T02 明确拒绝启动）、真实 Provider 与费用（T23，需授权）。
5. 本回合未使用浏览器/Playwright——T02 范围内无浏览器行为要求（AC-005/AC-006 均为 CLI/集成层），不构成 BLOCKED。

### 非阻断建议（不计入阻断项，不要求 T02 返工）

1. **内置 TLS 监听的属主（建议 T22 前，或由 PM 修订 PRD）**：见"未覆盖边界"第 1 条。建议协调者/PM 二选一：(a) 在 T04 与 T22 之间显式指派实现内置 rustls 监听（含 CA 信任根方案，与 Q-03/AC-064 的平台冷环境 HTTPS 验证衔接）；(b) 修订 PRD §5.1/architecture §7，把"内置 rustls 证书"从 MVP 承诺中移除、明确 MVP 只支持受信反向代理模式。不要在发布时让文档承诺处于未实现状态。
2. **`check` 的只读模式（可选，建议 T03/T20 时考虑）**：当前 `check` 有日志与写探针副作用；备份/升级流程（validation-release §5）中若需要"不触碰 data-dir 的检查"，可增加显式只读标志。
3. **`licenses.json` 覆盖面**（T01 回合 1 建议 1 的延续，仍建议 T22 处理）：本次 dist 仍以不带 `embedded-ui` 的 `cargo metadata` 生成许可证清单，T22 应以发布同一 feature 集生成。
4. **stdout 混合结构化日志与人读输出**：`init`/`serve` 的 stdout 同时含 JSON 日志行与结果行（`listening on …` 为 smoke 协议行）。运维脚本应以退出码为准而非解析首行；已在 ADR-011 说明，无需改动（提示性记录）。

## 非代码知识与限制（跨卡复用）

- **T02 的 fail-closed 语义**：TLS 已配置但监听未实现 → 拒绝启动（6），比"静默明文"安全；后续卡实现 TLS 时不得改动这一拒绝为静默降级。QA 已把该行为当作合同值核对。
- **零外呼可静态证明**：T02 服务端依赖树无任何 HTTP 客户端/TLS crate，这是 AC-006"check 不外呼"的编译期证据；后续卡（T12/T14 接入 Provider）引入 HTTP 客户端后，该证明失效，届时必须依赖 fixture 计数与域名约束测试重新证明。
- **锁的正确性依赖 flock**：`kill -9` 后立即可重新启动（实测），无陈旧锁问题；`check` 在锁被占用时不失败（运行中合法），后续卡（T20 备份）必须复用同一语义"先停服再备份"。
- **交互式密码测试需 pty**：脱离终端（`stdin=null`）时 `init` 返回 2 并提示 `--password-file`；QA 用 python pty 独立复现了回显关闭与两次确认。该手法可复用于后续涉及交互输入的验收。
- 以上第 1/2 条（TLS 属主、零外呼静态证明）已同步 `llmdoc/decisions.md`「QA 验收知识」。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 2026-09-12 | slice | T01（AC-001、AC-011） | 1（ui_revision 1） | PASS | 本文件 |
| 2 | 2026-09-12 | slice | T02（AC-005、AC-006、AC-012 配置侧） | 1（ui_revision 1） | PASS | 本文件 |

- 交接给 RD：本回合无缺陷需修复。T02 交付质量良好，无 REOPEN 项。
- 交接给协调者：（a）建议 `accepted_tasks` 追加 `{task_id: T02, prd_revision: 1, qa_round: 2, report: llmdoc/requirements/web-mvp/qa-report.md, accepted_ac_ids: [AC-005, AC-006], status: accepted}`；**AC-012 不建议标记 accepted**——本回合只验证了配置层语义，完整 AC-012 仍需 T04（`/settings/status`）与 T11/T12（estimate/jobs 未配置错误）落地后验收。（b）`qa_history` 追加本回合记录。（c）非阻断建议 1（内置 TLS 监听属主）需要协调者/PM 决策，建议在派发 T03 时一并明确其落点。
- 证据目录：`artifacts/web-mvp/t02-qa/`（16 个文件：config_cli 与 embedded-ui bootstrap 测试日志、xtask check/dist×2/smoke 日志、手工 CLI/交互 pty/零外呼/配置优先级/日志脱敏/Provider 配置边界 记录、前后哈希清单与 git status 基线）。

---

# 回合 3 · T03

**结果：PASS（仅 T03 切片范围，不代表产品全量）** · 回合：3 · PRD 修订：1（ui_revision 1）· 范围：切片（task_ids=[T03]；AC-007、AC-008）

- QA 执行时间：2026-09-12 02:52–02:58（本地 UTC+8；日志内时间戳为 UTC）；执行者：QA 子 agent，独立复现，未采信 RD 报告结论。
- 派发核对：PRD 修订 1 与 `prd.md` §9.1 一致（含 UI 同修订补充）；`implementation.md` §T03 状态 RD_READY；`state.yaml` qa_round=3、scope=slice、current_tasks=[T03]；进入时 open_defects 为空。
- 结论口径：只覆盖 T03 派发范围。T04–T23 未实现内容（`/health/ready` 的 DB 检查、认证、资产、任务执行器等）不计为本切片缺陷，列在"未覆盖边界"。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb44ee2a6088f440eade3f1424546fbb62d`（"init"，仅 1 次提交）；T01–T03 交付物仍未 git 提交（`git status` 与回合 2 基线一致；"工作树未改动"用源文件 sha256 清单证明） |
| 平台 | macOS Darwin 25.6.0 · aarch64-apple-darwin（PRD §5.6 必选平台之一） |
| 工具链 | rustc/cargo 1.98.1（`rust-toolchain.toml` 固定）；SQLx 0.9.0；bundled SQLite 3.51.3（libsqlite3-sys 0.37.0）；系统 `sqlite3` CLI 3.51.0（独立旁证读取） |
| 被测二进制（debug，手工负例） | `target/debug/everything-manual`；`CARGO_INCREMENTAL=0` 基线 sha256 `771a4b404494aa8cd08cbc02d14c3a9dfb22cb0954dd8adb50a44db54bb6b06a`（QA 独立复现，与 RD 报告一致；debug 默认增量构建哈希不可复现，见 ADR-012） |
| 被测二进制（发布路径） | `dist/aarch64-apple-darwin/everything-manual`，6 843 376 bytes，sha256 `1aaeb2dff5ce8e286333d0233a487db9a02b1359f32947d96396e0732de5b889`；QA 独立 `cargo xtask dist` 复现同一哈希 |
| 数据／凭据 | 全部 QA 自建临时 data-dir（`/tmp/em-t03-qa*`，已清理）与假密码 canary；零真实 Provider 调用、零费用 |
| 工作树完整性 | 83 个源／测试／生成文件 sha256 清单验收前后逐字节一致（`manifest-before.txt` vs `manifest-after.txt`）；迁移文件 sha256 前后一致（0001=`d2244c1e…`、0002=`372d6ff5…`）；无残留进程、无残留锁、无残留 smoke 临时目录 |

## AC 验收矩阵

| AC ID | 期望 | 实际 | 结果 | 命令／步骤／证据 |
| --- | --- | --- | --- | --- |
| AC-007 | `cargo test -p everything-manual --test storage`：核心表/唯一键/外键/索引按合同建立；重复迁移幂等；外键拒绝非法引用；revision CAS 生效；回滚不留半状态；关闭并重开同一 data-dir 后数据保留 | QA 独立执行 exit 0，**15 passed / 0 failed / 0 ignored**（1.18s），15 个用例名与卡内要求一一对应。另以真实二进制 + 系统 sqlite3 独立复核（不依赖测试代码）：`init` 建库 schema v2；20 张表（19 张合同表 + `_sqlx_migrations`）、25 个命名索引、4 个触发器、14 组外键（RESTRICT）+ `sessions→admins` CASCADE 全部实测存在；重复 `init`/`check` 幂等（迁移记录保持 2 条）；唯一键与 CHECK 负例（重复 blob sha、page_number=0、空白 name、expires<=created、孤儿外键、RESTRICT 删除父行）全部被拒；CAS 并发"恰好一胜一冲突"由用例实测；事务回滚无残留；v1 库数据经升级/重开后保留 | **PASS** | 02:52 `cargo test -p everything-manual --test storage`；02:53 `cli-01-init-idempotency.log`、`cli-02-schema-vs-contracts.log`、`cli-03-unique-checks.log`、`cli-10-fk-on-probes.log`；证据目录 `artifacts/web-mvp/t03-qa/` |
| AC-008（旧 schema 升级） | 旧 schema 自动迁移成功且数据保留 | QA 用真机二进制独立构造：`init` 建 v2 → DROP 0002 的 4 触发器 + 1 索引、DELETE `_sqlx_migrations` v2 行（0001 checksum 保持真实，等价 v1 库）、插入 items(升级前数据, revision=3) → `check` exit 0 输出「待迁移（库 v1 → 程序 v2）」且库字节 sha 不变（`14d59f25…`）、触发器仍为 0（未迁移）→ `serve` 自动迁移至 v2（`database_ready schemaVersion=2`）、SIGTERM 后 exit 0 → 升级后 migrations=1,2、触发器=4、部分唯一索引在、items 行仍在（revision=3） | **PASS** | 02:53 `cli-05-old-schema-upgrade.log` |
| AC-008（新 schema 拒绝） | 比程序更新的 schema 被拒绝打开、输出可读错误、库文件未被修改 | 真机负数例：v9999 成功迁移记录 + 用户数据（items revision=4）→ 库 sha `c1f41cc7…`；`serve` **exit 4**、stderr 可读中文（含 `v9999`/`v2`/`拒绝打开`）、无 `listening` 行；`check` 同样 **exit 4**；两次 sha 前后一致（`c1f41cc7…`）；副本读取证明用户数据与 9999 记录仍在 | **PASS** | 02:53 `cli-04-future-schema-negative.log`、`cli-04b-future-schema-data-read.log` |
| AC-008（重编译） | 修改迁移 SQL 后二进制触发重编译（rerun-if-changed） | `CARGO_INCREMENTAL=0` 基线构建 sha `771a4b40…` → 向 0002 追加注释 → 重建出现 `Compiling everything-manual`、sha 变为 `26c7ccc3…`（证明 SQL 内容确被编入二进制）→ 逐字节恢复（迁移文件 sha 回到 `372d6ff5…`）→ 重建 sha 回到 `771a4b40…`；无变更时构建无 `Compiling`。附加安全负例：修改已应用迁移后 `serve` 拒绝启动（`migration 2 was previously applied but has been modified`、exit 4、库字节不变） | **PASS** | 02:55 `reg-rerun-if-changed.log` |

### 卡内检查项（T03 卡 + PRD §5.4 + architecture §6）

| 检查项 | 实际结果 | 判定 | 证据 |
| --- | --- | --- | --- |
| WAL / `foreign_keys=ON` / `busy_timeout=5s` / `synchronous=FULL` 真实施加（PRAGMA 实测） | ① `check` 输出读自应用真实连接：`WAL=wal synchronous=2 foreign_keys=ON busy_timeout=5000ms`；② 系统 sqlite3 读主文件 `journal_mode=wal`（持久值，非会话默认）；③ 行为学证据：python3 持 `BEGIN IMMEDIATE` 写锁时 `serve` 迁移**等待 5.43s 后**以 `(code: 5) database is locked` 失败（exit 4）、失败后 migrations=1/触发器=0 无半状态、锁释放后重启即成功升至 v2——证明 busy_timeout 实际生效而非仅上报数值 | PASS | 02:53–02:54 `cli-01`、`cli-07-busy-timeout-behavior.log` |
| 连接池上限 4 | 全仓仅一处创建池（`storage/db.rs::connect`，`max_connections(POOL_MAX_CONNECTIONS=4)`）；用例断言 `pool.options()`=4 | PASS（配置层；无可外部观测的运行时并发证据，见未覆盖边界） | `crates/server/src/storage/db.rs`、storage 用例 |
| 迁移随二进制内嵌（冷目录无源码仍可迁移） | 将 release 二进制单独复制到 `/tmp/em-t03-qa-cold/bin`（工作目录内无仓库、无源码、无 migrations/）→ `init` 创建并迁移 schema v2、`check` 就绪、4 触发器在；用例 `embedded_migrations_match_repository_files` 证明内嵌内容与 `migrations/*.sql` 逐字节一致 | PASS | 02:56 `reg-smoke-and-cold-dir.log` |
| SQL 写法（静态 SQL + bind；无字符串拼接注入面） | `crates/server/src/{storage,config}` 内无 `format!`/拼接 SQL（grep 0 命中）；`AssertSqlSafe` 仅测试文件（表名来自测试常量）；SQLx 0.9 `SqlSafeStr` 仅对 `&'static str` 实现（QA 在 `~/.cargo/registry/.../sqlx-core-0.9.0/src/sql_str.rs` 源码核对），动态字符串编译期被拒 | PASS | 02:54 `cli-09-sidecar-lockfile.log`（含 grep 结果）、sqlx 源码 |
| `check` 已接入 schema 校验且不发起外部 HTTP（AC-006 不回退） | 有库三态实测：Missing→"尚未建立" exit 0 且**不创建**库文件；Pending→"待迁移" exit 0 且库 sha 不变、不应用迁移；Ready→"已就绪" exit 0。零外呼：两 Provider `base_url` 指向 QA 计数监听器并注入密钥（状态"已配置"）→ `check` exit 0、**监听器 0 连接**（正对照 1 次为 QA 自测连接）。静态旁证：服务端依赖树无 HTTP 客户端/TLS crate（hyper 仅作 axum 服务端传输，`crates/server/src` 无任何引用） | PASS | 02:54 `cli-06-check-missing-db-and-zero-http.log`、`cli-08` |
| 迁移只追加、单条迁移事务（ADR-009/REQ-004） | `migrations/` 仅 0001/0002 两个文件；无 `-- no-transaction` 标记（每条迁移在事务内执行）；`failed_migration_leaves_no_partial_state` 用例实测坏迁移不留半表、版本停在前一版本、修复后可继续升级 | PASS | `crates/server/src/storage/migrations.rs`、storage 用例 |

### 合同对照结论（contracts.md §2 ↔ migrations/*.sql，逐项核对）

- **表**：19/19 全建（+ `_sqlx_migrations`）；缺表 **0**。核心字段逐行对照均存在（含 `preparations.source_sha256`、`generation_snapshots.photo_hashes`、`job_stages.lease_*`、`provider_attempts.response_id`、`cost_ledger.price_version` 等）。
- **唯一键/主键**：`blobs.sha256` PK（唯一内容存储）、`sessions.session_token_hash` UNIQUE、`pages(preparation_id,page_number)` PK、`job_stages(job_id,stage_kind,batch_index)` UNIQUE、`idempotency_records(admin_id,method,route,key)` UNIQUE、`manual_drafts.snapshot_id` UNIQUE、`provider_attempts` 部分唯一索引（intent/submitting/unknown 每阶段 ≤1）全部实测存在；缺键 **0**。
- **外键**：14 组全部存在（RESTRICT），`sessions→admins` CASCADE；`audit_events` 为多态引用（entity_type/entity_id）无外键属预期；缺外键 **0**。
- **类型/默认值**：时间列 INTEGER Unix 毫秒、枚举 SQL snake_case（线上 camelCase）、金额 INTEGER 最小单位、UUIDv7 TEXT id——均为 ADR-012 已记录的设计取舍，与 contracts §1（RFC3339 为线上格式）不冲突；无未记录偏差。`items.revision`/`jobs.revision`/`manual_drafts.revision` 默认 1、`blobs.storage_state` 默认 stored 等与合同语义一致。
- **"必须保证"列**：DB 可强制项（sha256 唯一存储、页号唯一、非批处理 `batch_index=0`、远端 ID 只允许 null→值或同值、输入/发布不可变、unknown 不留 actual、会话明文不落库）已由主键/唯一键/CHECK/触发器/索引落地并有测试；服务层承担项（名称型号 422/412、页号连续 1..N 与 ready 冻结、租约推进、幂等 409、发布不变量）按卡映射 T07/T09/T10/T11/T19——记录为未覆盖边界，非本卡缺陷。
- **ADR-012 一致性**：逐条与架构/合同及本回合实测可对齐（INTEGER 毫秒、枚举 snake_case、触发器钉不变量、门禁先于迁移、init 建库/check 只读三态、1811 双义分类、`&mut SqliteConnection`、`SqlSafeStr`、DB 0600、CARGO_INCREMENTAL 证据要求）；QA 复核通过，无冲突。

## 缺陷

无。本回合 0 个 OPEN 缺陷（AC-007、AC-008 及全部卡内必测项、T01/T02 回归均通过）。

## 特别裁定（本回合必须结论）

1. **`/health/ready` 未检查 DB/迁移/data-dir：裁定不属于 T03 必选范围，不阻断本切片。**
   - 现状实测：`GET /api/v1/health/ready` 返回 `{"data":{"status":"ready","checks":[{"name":"process","status":"ok"}]}}`（仅进程自检）。
   - 依据：合同要求见 `contracts.md` §3「ready 检查 DB/迁移/数据目录」与 PRD REQ-007/AC-013；但 **(a)** T03 派发卡允许文件不含 `crates/server/src/http/`，AC-007/AC-008 只覆盖存储层；**(b)** AC-013 的正式测试载体是 `--test fixture_harness`（T05 才建立），当前无法执行；**(c)** 接线需在应用状态注入 `Database`，PRD §7.1 将 REQ-007 映射到 T02/T04/T12/T14/T22，ADR-012 亦记为"属 T04 接线"。当前 `serve` 启动已强制通过迁移门禁，运行中进程的 DB 在启动时有效，现阶段实际风险小。
   - 建议落点：**T04**（Database 入应用状态，ready 检查 DB/迁移/数据目录）；**AC-013 由 T05 的 fixture_harness 正式关闭**。附带文本问题：`crates/server/src/http/router.rs` 与 `contracts/openapi.json` 的 ready 描述仍写"T03 起扩展到数据库/迁移/数据目录"，与 RD 决策不符，建议 T04 接线时同步修正描述；协调者应把该接线点写入 T04 派发包，避免 AC-013 到 T05 才发现未接线。
2. **`check` 的日志/写探针副作用：裁定不违反 REQ-003/AC-006 的硬约束，不阻断本切片。**
   - 实测副作用清单（QA 逐文件 sha256 对比）：向 `<data-dir>/logs/everything-manual.log` 追加一行；**改写 lock 文件诊断内容**（`pid=… started_at_unix=…`；锁本身是 flock，文件内容仅诊断）；`tmp/` 写探针建删（无残留）；**主库字节不变、无新增残留文件**；外部 HTTP 0 次、费用 0（计数监听器实测）。本次未观察到 RD 声明的"空 `-wal`/`-shm` 残留"。
   - 判定：PRD REQ-003 的约束句是"`check` 不调用任何外部 API、不产生费用"（"只验证配置／目录／schema"描述验证范围）；AC-006 的实际约束是"全程不发起任何外部 HTTP 请求"，实测满足。写探针是 T02 起的刻意行为（为 serve 验证目录可写）且有测试覆盖；日志追加是 ADR-011 的日志设计。属"非严格只读"而非合同违反。
   - 建议：保留回合 2 的非阻断建议（T20 前增加显式只读/`--dry-run` 标志），新增数据点：lock 文件内容会被改写（见非阻断建议 2）。

## 未覆盖边界／已知限制（本回合记录，不冒充通过）

1. `/health/ready` 的 DB/迁移/数据目录检查（T04 接线；AC-013 由 T05 fixture_harness 验收）——见裁定 1。
2. 服务层业务不变量：页号连续 1..N 与 ready 后不可修改（T09/AC-025）、租约/epoch 推进（T10）、幂等 409（T11）、发布不变量（T19）、上传 purpose 白名单与资产归属校验（T06）——DB 层已就绪，端到端行为未验。
3. 连接池上限 4 仅配置断言 + 代码核对，无可外部观测的运行时证据；WAL 下多连接并发写行为属 T10/T11 范围。
4. T04–T23 未实现内容（认证、资产、Provider、backup/restore、发布链）与本轮无关；真实 Provider 与费用（T23，需授权）未验。
5. 非 Unix 平台（Windows 属扩展平台，T02 起明确拒绝启动）；系统 sqlite3 3.51.0 仅作旁证（产品内为 bundled 3.51.3）。
6. 本回合未使用浏览器/Playwright——T03 无浏览器行为要求（纯存储/CLI 层），不构成 BLOCKED。

### 非阻断建议（不计入阻断项，不要求 T03 返工）

1. **`/health/ready` 接线与描述文本修正（落点 T04）**：见裁定 1。
2. **`check` 只读模式（沿用回合 2 建议 2，落点 T20 或按 PM 决定）**：新增数据点——`check` 会改写 lock 文件诊断内容（`pid`/`started_at_unix`），且只读打开数据库可能新建空 `-wal`/`-shm`；若 T20 备份/升级流程需要"完全不触碰 data-dir"的检查，应增加显式只读标志。
3. **`check` 与 `serve` 对"已应用迁移被修改"判定不一致（建议后续卡处理）**：修改迁移文件后重建，`serve` 拒绝启动（exit 4、可读错误、库不变），`check` 仍报"已就绪" exit 0。`check` 作为停服前预检工具存在"预检通过但不能启动"的失真；建议把 checksum 校验纳入 `check`（可复用 sqlx 迁移校验）。非阻断：AC-005/007/008 未要求该项，且 `serve` 的安全拒绝已实测。
4. **`assets.purpose` 的 SQL 值含 `model`（T06 接口提醒）**：`model` 为服务内部用途（`model_revisions.asset_id`）；T06 实现上传接口时须按 contracts §3 只接受 document/photo/pageImage/pageText，避免把 `model` 暴露为上传目的。
5. **smoke 检查计数口径（文档性）**：`xtask smoke-bootstrap` 实际输出 7 项 `[检查]`（+1 项 `[准备] init`）；回合 1/2 报告与 RD 记录写"8 项 HTTP 检查"，建议后续按日志实数描述（不影响结论）。
6. **sqlx 慢语句日志**：本次写锁竞争触发 sqlx slow-statement WARN，日志含 SQL 全文（为迁移 SQL，无用户数据/密钥）。后续卡引入带参数查询时需确认日志不会带出用户数据。

## 非代码知识与限制（跨卡复用）

- **迁移重编译证据必须 `CARGO_INCREMENTAL=0`**（debug 增量构建哈希不可复现；基线 `771a4b40…`）；无变更时"不重编译"也应作为证据之一。
- **构造"旧 schema 库"的可靠手法**：用真实 `init` 建 v2 → DROP 0002 的 4 触发器 + 1 索引并删除 `_sqlx_migrations` v2 行（0001 checksum 保持真实，可被 `serve` 正常升级）；直接用 sqlite3 伪造迁移行（checksum X'00'）只能用于"未来 schema 拒绝"负例——用于升级路径会被 checksum 校验拒绝。
- **busy_timeout 行为学证据法**：python3 持 `BEGIN IMMEDIATE` 写锁时测 `serve` 迁移等待时长（本次 5.43s），比只读 PRAGMA 更强。
- **冷目录迁移验证**：release 二进制单独放入临时目录 `init`+`check` 即可证明迁移内嵌（无需源码目录）。
- **`check` 副作用清单（更新）**：日志追加 + lock 文件内容改写 + `tmp/` 写探针（无残留）；本次未观察到空边车文件残留。
- **负例后恢复核对**：83 文件 sha256 清单 + 迁移文件 sha256 + `git status` 三者前后一致（未提交仓库下清单比对法延续回合 1）。
- 以上第 1/2/3/5 条已同步 `llmdoc/decisions.md`「QA 验收知识」。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 2026-09-12 | slice | T01（AC-001、AC-011） | 1（ui_revision 1） | PASS | 本文件 |
| 2 | 2026-09-12 | slice | T02（AC-005、AC-006、AC-012 配置侧） | 1（ui_revision 1） | PASS | 本文件 |
| 3 | 2026-09-12 | slice | T03（AC-007、AC-008） | 1（ui_revision 1） | PASS | 本文件 |

- 交接给 RD：本回合无缺陷需修复。非阻断建议 1/3/4/6 建议由相关卡承接（T04/T06/后续），建议 2 落点 T20，建议 5 属文档计数口径。
- 交接给协调者：（a）建议 `accepted_tasks` 追加 `{task_id: T03, prd_revision: 1, qa_round: 3, report: llmdoc/requirements/web-mvp/qa-report.md, accepted_ac_ids: [AC-007, AC-008], status: accepted}`；（b）`qa_history` 追加本回合记录；（c）派发 T04 时把 `/health/ready` 接线（含 router/openapi 描述文本修正）明确写入派发包（裁定 1）；（d）回合 2 遗留的"内置 TLS 监听属主"仍未决，继续挂起待 PM/协调者决定。
- 证据目录：`artifacts/web-mvp/t03-qa/`（22 个文件：storage 测试日志、手工 CLI 负例/PRAGMA/FK/唯一键/busy_timeout/零外呼/副作用记录、workspace/xtask check/contracts/rerun-if-changed/dist/smoke 与冷目录日志、前后 sha256 清单）。

# 回合 4 · T04

**结果：PASS（仅 T04 切片范围，不代表产品全量）** · 回合：4 · PRD 修订：1（ui_revision 1）· 范围：切片（task_ids=[T04]；AC-003、AC-004、AC-012 的 `/settings/status` 侧、AC-066 错误结构侧）

- QA 执行时间：2026-09-12 03:20–03:26（本地 UTC+8；以下日志内时间戳为 UTC）；执行者：QA 子 agent，独立复现，未采信 RD 报告结论（未运行 `artifacts/web-mvp/t04-rd/smoke-manual.sh`，另写 QA 脚本 `artifacts/web-mvp/t04-qa/qa-smoke.sh`）。
- 派发核对：`prd.md` 修订 1（ui_revision 1）与派发包一致；`implementation.md` §T04 状态 RD_READY；`state.yaml` qa_round=4、scope=slice、current_tasks=[T04]、open_defects 空。
- 结论口径：只覆盖 T04 派发范围。estimate/jobs 未配置错误（T11/T12）、fixture（T05）、上传（T06）、物品业务（T07）不计为本切片缺陷，列在"未覆盖边界"。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb44ee2a6088f440eade3f1424546fbb62d`（仍未新增提交）；`git status` 与回合 3 基线一致（M `.gitignore`/`decisions.md`/`state.yaml` + 未跟踪目录） |
| 平台 | macOS Darwin 25.6.0 · aarch64-apple-darwin |
| 工具链 | rustc/cargo 1.98.1；Node v26.0.0 / npm 11.12.1；系统 sqlite3 3.51.0（仅旁证读取）；curl 8.7.1 |
| 被测二进制（发布路径，手工冒烟） | `dist/aarch64-apple-darwin/everything-manual`，8 062 784 bytes，sha256 `b8aa9172f452593a4a676e364ac99dbd794eb79712c47e25c42abefcb7a38cb4`；QA 独立 `cargo xtask dist` 连跑两次复现同一哈希 |
| 数据／凭据 | 全部 QA 自建临时 data-dir（`/tmp/em-t04-qa*`，脚本结束即删除，实测无残留）与假密码 canary；零真实 Provider 调用、零费用、零外网 |
| 工作树完整性 | 72 个源／测试／生成文件 sha256 清单验收前后逐字节一致（`manifest-before.txt` vs `manifest-after.txt`）；QA 自建临时目录（`/tmp/em-t04-qa*`）已全部删除、无残留 serve 进程；未改动 prd.md / state.yaml / 规范文档。**另发现 RD 冒烟遗留 5 个临时目录**（`/tmp/em-t04-manual.*` ×4 + `/tmp/em-t04-check`，时间戳 03:12–03:19，属 RD 回合产物，含假密码 pw.txt；QA 未删除他人现场，见非阻断建议 6） |

## AC 验收矩阵

| AC ID | 期望 | 实际 | 结果 | 命令／步骤／证据 |
| --- | --- | --- | --- | --- |
| AC-003 | 未登录 `GET /api/v1/items` → 401 且 `error.code/message/details/requestId`；登录返回 HttpOnly+SameSite=Strict cookie 与 CSRF token；注销后旧 cookie → 401；`GET /auth/session` 带 `Cache-Control: no-store` | 401 响应体四键齐全，`requestId=01a091ea-eefd-77bd-8c8a-787d6a6ba39d` 与响应头 `x-request-id` 同值；登录 200 返回 `set-cookie: em_session=…; Path=/; HttpOnly; SameSite=Strict; Max-Age=604800`（loopback http 无 `Secure`，无 `Domain`）、`cache-control: no-store`、64 hex `csrfToken`，响应体不含 token 明文；`GET /auth/session` 200 + no-store + 同一 CSRF；登出（带 CSRF）204 + `em_session=; …Max-Age=0`，旧 cookie 再请求 401 且库中该会话 `revoked_at` 已置位；伪造/过期/已撤销 cookie 均 401 | **PASS** | `cargo test -p everything-manual --test auth_api` → exit 0，**13 passed / 0 failed / 0 ignored**（`ac003-004-auth-api.log`）；手工链路 `qa-smoke.log` §5–7、§21–22、`qa-followup.log` §C |
| AC-003（Secure 判定） | HTTPS 判定下加 Secure，且判定不依赖未受信转发头 | `public_origin=https://…` 用例 `https_declared_origin_marks_cookie_secure` 通过；loopback http 手工实测无 Secure；带 `x-forwarded-proto: https`＋`x-forwarded-ssl: on`＋`forwarded: proto=https` 登录，Set-Cookie **仍无** Secure | **PASS** | `qa-smoke.log` §6、§8 |
| AC-004 | 无 CSRF/跨站 Origin 的修改 → 403；连续错误密码 → 429（限速）；缺 `If-Match` → 428；过期 revision → 412；错误响应不泄露堆栈或 SQL | 缺 CSRF PATCH → 403 `CSRF_REJECTED`；带 CSRF + `Origin: https://attacker.example` → 403 `ORIGIN_REJECTED`；5 次错密码全 401，第 6 次（正确密码）→ 429 + `retry-after: 60`；缺 If-Match → 428 `PRECONDITION_REQUIRED`；`If-Match: *` → 422；`"r1"` → 200 + `etag: "r2"`，再用 `"r1"` → 412 `REVISION_CONFLICT` + `details.currentRevision=2`；全部错误体无 SQL/堆栈（扫描 0 命中），message 为通用文案、不回显密码 | **PASS** | 同上测试命令（`mutating_requests_require_csrf_and_same_site_origin`、`wrong_password_is_401_and_repeated_failures_are_429`、`rate_limit_window_expires_and_success_resets_counter`、`if_match_missing_is_428_and_stale_revision_is_412_with_current_revision`）；手工 `qa-smoke.log` §9–15、§23、`qa-followup.log` §D |
| AC-012（`/settings/status` 侧） | `providersConfigured`/`limits`/`capabilities`，无密钥或完整配置；未配置如实 false、不假成功 | 200 `{"providersConfigured":{"tripo":false,"manualAi":false},"limits":{"maxJsonRequestBytes":1048576,"maxPdfBytes":52428800,"maxPdfPages":100,"maxPhotoBytes":20971520,"maxGlbBytes":157286400,"maxItemTotalBytes":524288000},"capabilities":{"generation":false}}`；对 `api_key/apiKey/base_url/baseUrl/data_dir/dataDir/listen/openapi.tripo3d/api.openai/secret` 逐字面量扫描均 0 命中；注入假密钥 canary 后 `tripo=true`、`generation=false`，响应不含 canary 与密钥来源（用例断言） | **PASS（仅 settings 侧）** | 用例 `settings_status_reports_configuration_without_secrets`；手工 `qa-smoke.log` §18。estimate/jobs 的未配置错误属 T11/T12，本回合未验 |
| AC-066（错误结构侧） | 错误响应含 requestId 且无堆栈/SQL/密钥；日志可关联 | 每个错误体固定四键（用例断言 `keys.len()==4`），`requestId` 为 UUID 且与 `x-request-id` 响应头一致、与 `http_request` 日志行同值（手工 grep 命中 1 行）；日志无密码（0 命中）、无查询串（`?secret=QA_QUERY_CANARY_1` 0 命中）、无 `"body"` 字段 | **PASS** | `qa-smoke.log` §24；用例 `assert_contract_error` 全量断言 |

### 卡内检查项（T04 卡 + PRD §5.1/§5.7 + contracts §1/§3）

| 检查项 | 实际结果 | 判定 | 证据 |
| --- | --- | --- | --- |
| cookie 属性（HttpOnly/SameSite=Strict/Secure 判定不读未受信 `X-Forwarded-*`） | 实测 Set-Cookie 全文见 AC-003；无 `Domain`；伪造三种转发头不产生 Secure | PASS | `qa-smoke.log` §6/§8 |
| 会话 token 落库为哈希而非明文 | `sha256(cookie token)=4518e69f…` **等于**库中 `sessions.session_token_hash`；`sha256(csrf)=b45837ca…` 等于 `csrf_hash`；库文件原始字节与日志中 token/csrf 明文各 **0 命中** | PASS | `qa-followup.log` §A |
| CSRF/Origin 覆盖含 multipart 的修改请求 | 守卫按方法（POST/PUT/PATCH/DELETE）判定、先于路由与 body 解析：`Content-Type: multipart/form-data` 无 CSRF 的 POST → 403 `CSRF_REJECTED`；跨会话 CSRF（A 会话 cookie + B 会话 token）→ 403 | PASS（机制层；T06 真实 multipart 路由落地后需复验） | `qa-items-boundary.log` §1、`qa-smoke.log` §11 |
| 限速窗口与 Retry-After | 默认 5 次/60s：第 6 次 429 + `retry-after: 60`（剩余秒向上取整、最小 1）；1s 短窗口用例实测"窗口过期自动重置 + 成功登录清零 + 不同 IP 互不影响"；不可信 peer 下 5 次带不同 `X-Forwarded-For` 仍累计为同一限速键 | PASS | `qa-followup.log` §D；用例 `rate_limit_window_expires_and_success_resets_counter` + `limiter` 单元测试 |
| 错误结构四键 + requestId 与日志/响应头一致 | 见 AC-066 行；fallback 404 与 `JsonBody` 拒绝也取同一 requestId（T01 遗留"body/header 不同 UUID"已修复，实测同值） | PASS | `qa-smoke.log` §5/§17/§24 |
| `/api/unknown` JSON 404 | `/api/unknown`、`/api/v1/does-not-exist`、`/api` 均 `application/json` 404（四键 + requestId）；embedded-ui 构建下同样不落 SPA（`bootstrap` 8 用例含该断言） | PASS | `qa-smoke.log` §17、`qa-404-405.log`、`reg-bootstrap-embedded.log` |
| 统一 404/405 语义 | 未知路径 → JSON 404（未知路径上的 POST 也是 404，不因方法不同而 401/405）；已知路由方法不匹配 → 405 + `Allow` + `x-request-id`，**body 为空**（不套统一错误信封）；受保护路由上未带 CSRF 的 DELETE 先得 403（守卫先于路由） | 语义正确、无泄露；405 body 为空列非阻断建议 1 | `qa-followup.log` §B、`qa-404-405.log` |
| OpenAPI/TS 生成无漂移 | `cargo xtask contracts --check` → 两份生成物`[一致]`、不改工作树；OpenAPI 3.1 已含 8 条路径、`sessionCookie`（apiKey/in=cookie/em_session）安全方案、items PATCH 响应 200/401/403/404/412/422/428；`generated.ts` 含 `ReadinessCheckName: "process"|"data_directory"|"database"|"migrations"` 等新类型 | PASS | `reg-contracts-check.log`；`contracts/openapi.json`、`apps/web/src/api/generated.ts` |

### 上游闭合核对

| 遗留项 | 实际结果 | 判定 | 证据 |
| --- | --- | --- | --- |
| T02 遗留：`init` 不持久化密码 | `init` 输出"管理员凭据：已创建（admins 表；Argon2id v19、m=19456 KiB、t=2、p=1、16 字节盐；无默认密码）"；库中 `admins.password_hash` 为 97 字符 `$argon2id$v=19$m=19456,t=2,p=1$…`；明文密码在**整个 data-dir**（含 WAL 边车）与日志 0 命中；重跑 `init` → "已更新" + 撤销全部 4 个既有会话（`revoked_at` 全非空），旧密码 401、新密码 200 | PASS | `qa-smoke.log` §1、§1a–1c、§25–26 |
| T03 遗留：`/health/ready` 接数据层 | 200 `{"status":"ready","checks":[process,data_directory,database,migrations]}`；运行中把 `manual.sqlite3` 移走 → 503 `not_ready` + `data_directory=fail`（恢复文件后 200）；不检查云端、不输出路径/版本；`router.rs`/OpenAPI 文案已改为"检查进程、数据目录、数据库与迁移版本"（回合 3 建议 1 闭合） | PASS | `qa-smoke.log` §19–20；`reg-contracts-check.log`；`crates/server/src/http/health.rs` |

### 回归（AC-001/005/006/007/008/011 证据链）

| 项 | 实际结果 | 证据 |
| --- | --- | --- |
| `cargo test -p everything-manual --test bootstrap` / `config_cli` / `storage` | 5 / 12 / 15 passed，0 failed、0 ignored | `reg-integration-suites.log` |
| `cargo test --workspace`（含于 xtask check） | 62+13+5+12+15+8 = **115 passed / 0 failed**；仅 1 条 doctest `ignore`（`storage/repo/items.rs` 文档示例，刻意标记，非被跳过用例） | `reg-xtask-check.log` |
| `cargo xtask check` | exit 0，7 步全部`[通过]`（fmt/clippy/workspace 测试/前端 lint/typecheck/vitest 5/合同检查） | `reg-xtask-check.log` |
| `cargo xtask contracts --check` | exit 0，`[一致]`×2，工作树未被修改 | `reg-contracts-check.log`、manifest 前后比对 |
| `cargo xtask dist`（QA 独立两次）+ `smoke-bootstrap` | 两次哈希一致 `b8aa9172…`（8 062 784 bytes）；smoke exit 0，7 项 `[检查]` + 1 项 `[准备]` 全过 | `reg-dist-and-smoke.log` |
| `cargo test -p everything-manual --features embedded-ui --test bootstrap` | 8 passed（含内嵌页面/哈希资源/缺失资源 404 非 HTML） | `reg-bootstrap-embedded.log` |

## 缺陷

无。本回合 0 个 OPEN 缺陷（AC-003、AC-004、AC-012 settings 侧、AC-066 错误结构侧及全部卡内必测项、T01–T03 回归均通过）。

## 特别裁定（本回合必须结论）：items 三端点作为 If-Match 载体

1. **结论：不属于 T07 越界缺陷；判为"提前实现、语义与 REQ-010 不冲突、边界已显式标注"，不阻断本切片。**
   - 依据：T04 卡的实现项含"If-Match 工具"，需要有可测端点才能证明 428/412（AC-004 的必选内容）；协调者派发包已明确该载体；`crates/server/src/http/items.rs`、`dto/items.rs` 顶部、`implementation.md` §T04-6、ADR-013 均显式标注 T04 边界。
   - QA 实测语义与 REQ-010 的对照（无冲突项）：**没有**创建端点（`POST /items` → 405，不伪造 201）；**没有**物理删除 API（`DELETE /items`、`DELETE /items/{id}` → 405）；归档走 `PATCH {"archived":true|false}`（与 REQ-010"归档用 PATCH 字段"一致），归档后单条可读；空白 name/model → 422（数据库 CHECK 兜底）；同品牌型号不强制唯一（与合同一致）；If-Match 缺 428、过期 412 与合同一致。
   - 风险与缓解：载体**不能**被当作 T07 完成——已发现的未实现项（下表）全部记录在案；建议协调者在 T07 与 T08 派发包中显式引用本裁定，避免 T08 前端把 `/items` 当作冻结接口消费。

2. **T07 必须覆盖的剩余项（QA 实测确认当前未实现）**：
   | 项 | 当前实测行为 | 依据 |
   | --- | --- | --- |
   | 创建物品（201 + UUIDv7 + revision） | `POST /api/v1/items` → 405 | AC-016 |
   | 超长字段校验 | 300 字符 name 被接受（200） | AC-016"空白/超长 422" |
   | 字段级 422 明细 | 仅数据库约束兜底，`details` 为 null | AC-016/合同 §1 |
   | 归档物品的列表过滤 / `includeArchived` | 归档后仍出现在默认列表 | AC-016"归档物品不出现在默认列表" |
   | PATCH 清空可选字段（brand/variant 置 null） | `{"brand":null}` 返回 200 但静默保留原值，且 revision 仍递增 | ADR-013 未验证边界 |
   | 资产/资料/照片关联（T06/T07） | 路由不存在 | T06/T07 卡 |
   | 分页游标在过滤条件下的语义 | 仅有全量 (createdAt DESC, id DESC) 游标 | ADR-013/T07 |
   | 归档不破坏已发布资料 | 无 release 可用，未验 | AC-016 |

3. **附带观察（非缺陷）**：T04 卡"允许范围"文本未列 `http/{items,settings,health,precondition,body,state}.rs` 与 `storage/repo/{admins,sessions}.rs`；这些文件由卡内目标（settings 状态、404 API 分离、If-Match 工具）派生，且协调者派发包已确认 items 载体。建议协调者在该项上记录一次（或更新卡文），避免后续回合把它当成未申报改动。

## 非阻断建议（不计入阻断项，不要求 T04 返工）

1. **405 响应为空 body（无统一错误信封）**：`DELETE/PUT/PATCH` 到已存在路由 → `405 Method Not Allowed` + `Allow` + `x-request-id`，`content-length: 0`。不违反 T04 必选 AC（无泄露、含 requestId 头），但 `contracts.md` §1"错误统一 `error.code/message/details/requestId`"未排除 405。建议二选一：(a) PM/协调者在合同层明确"框架级 405 不要求信封"并记录；(b) 后续卡为 MethodRouter 增加 JSON 405 fallback（与 `/api/*` JSON 404 对齐）。若按严格解读 (a) 不成立，则由协调者转 PM。
2. **未带 CSRF 的方法不匹配请求先返回 403 而非 405**（守卫先于路由）：`DELETE /api/v1/items`（无 CSRF）→ 403 `CSRF_REJECTED`。与建议 1 同源；T07 补删除语义时应一并明确期望顺序。
3. **`/health/ready` 复检粒度**：把库文件移走时 `data_directory=fail` 而 `database=ok`（连接池仍持已删除文件的句柄）；总状态正确（503/not_ready）。若运维需要"哪一项失败"更精确，可在后续卡增加目录内容/文件存在性校验（非 T04 要求）。
4. **`OPTIONS`/`HEAD` 到受保护路由返回 401**（无 CORS 预检支持）：同源 SPA 部署下无影响；若未来出现跨源开发代理场景需另行设计（当前 Vite 代理为同源，T08 无影响）。
5. **会话清理只在登录时执行 `purge_expired`**（RD 已记录）；T20 备份前若要求会话表整洁需另设清理点。
6. **RD 冒烟脚本不清理临时目录（卫生项）**：`artifacts/web-mvp/t04-rd/smoke-manual.sh` 末行只 `echo "清理：$WORK"`，未 `rm -rf`；实测遗留 `/tmp/em-t04-manual.{IEUIjT,mU90MF,qIVFb6,S0zvAD}`（03:12–03:18）与 `/tmp/em-t04-check`（03:19），内含明文假密码文件（0600）。建议 RD 在脚本末尾真正删除，或由协调者清理；T22 smoke 的临时目录清理应一并要求（非阻断）。

## 未覆盖边界／已知限制（本回合记录，不冒充通过）

1. AC-012 的 estimate/jobs 未配置错误与"不创建 job/不写费用/不发起远端请求"（T11/T12）；AC-013 的 release 级 fixture 语义（T05）。
2. T06 上传：multipart 的真实路由、独立体积上限、资产授权与 Range/ETag（本回合只验证了守卫对 multipart 方法的覆盖）。
3. T07 物品业务规则（见裁定 2 的清单）。
4. T10+ 任务执行、租约、幂等、发布不变量未验（无实现）。
5. 未做浏览器/Playwright：本切片无浏览器行为要求（AC-003/004/012 均非浏览器 AC），不构成 BLOCKED。
6. 未验证真实 Provider 与费用（T23 需授权）；未验证 Linux musl 平台（T22）；未做反向代理端到端（cookie `Secure=always` 仅有配置与单元证据）。
7. 限速器为进程内状态：多实例/重启清零语义按 ADR-013 设计，未做跨进程实测。

## 非代码知识与限制（跨卡复用）

1. **会话 token"只存哈希"的独立验证手法**：`curl -c jar.txt` 的 cookie jar 是 **tab 分隔**，要用 `awk -F'\t' '$6=="em_session"{print $7}'` 取 token（用 `grep 'em_session='` 会取不到——本回合首版脚本即踩此坑）；再 `shasum -a 256` 与 `sessions.session_token_hash` 比对，并用 Python 在原库字节中 `blob.count(token)` 确认 0 命中。
2. **405 语义（T06/T07/T12 复用）**：方法不匹配返回空 body 405 + `Allow` + `x-request-id`；受保护路由上未带 CSRF 的修改请求先得 403（守卫按方法判定，先于路由与 body 解析）。新增路由的方法集合变化会改变 `Allow` 值。
3. **cookie `Secure` 伪造头负例**：带 `x-forwarded-proto: https`、`x-forwarded-ssl: on`、`forwarded: proto=https` 登录，Set-Cookie 仍不得含 `Secure`（T04 起判定只看 `public_origin`/TLS 配置；反向代理部署必须显式 `session.cookie_secure="always"`）。
4. **CSRF 覆盖 multipart 的验证法**：向任意受保护路径发 `Content-Type: multipart/form-data` 且无 `X-CSRF-Token` 的 POST，应得 403 `CSRF_REJECTED`；T06/T09 加 multipart 路由后沿用该负例（并注意它们需要独立于 1 MiB JSON 的 body 上限）。
5. **ready 负例法**：服务运行中把 `<data-dir>/manual.sqlite3` 移走 → 503 `not_ready`（`data_directory=fail`，`database` 因池内句柄仍 `ok`），恢复文件后立即 200——比只读 `chmod` 更贴近"数据目录损坏"场景且不影响运行中的连接。
6. 以上 1/2/4/5 已同步 `llmdoc/decisions.md`「QA 验收知识 · T04 验收知识」。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 2026-09-12 | slice | T01（AC-001、AC-011） | 1（ui_revision 1） | PASS | 本文件 |
| 2 | 2026-09-12 | slice | T02（AC-005、AC-006、AC-012 配置侧） | 1（ui_revision 1） | PASS | 本文件 |
| 3 | 2026-09-12 | slice | T03（AC-007、AC-008） | 1（ui_revision 1） | PASS | 本文件 |
| 4 | 2026-09-12 | slice | T04（AC-003、AC-004、AC-012 settings 侧、AC-066 错误结构侧） | 1（ui_revision 1） | PASS | 本文件 |

- 交接给 RD：本回合无缺陷需修复。非阻断建议 1/2 建议在 T07（物品/删除语义）与合同层一并处理，建议 3/5 归 T20/T22 范围，建议 4 记录备查。
- 交接给协调者：（a）`accepted_tasks` 追加 `{task_id: T04, prd_revision: 1, qa_round: 4, report: llmdoc/requirements/web-mvp/qa-report.md, accepted_ac_ids: [AC-003, AC-004, AC-012（settings 侧）, AC-066（错误结构侧）], status: accepted}`——若 state 只接受 AC 粒度，建议记 `[AC-003, AC-004]` 并在备注写明 AC-012/AC-066 为部分验收（estimate/jobs 侧属 T11/T12、日志人工侧已验）；（b）`qa_history` 追加本回合；（c）派发 T05 或 T07 时把本报告"特别裁定 2"的 T07 剩余项清单显式写入 T07 派发包，并提示 T08 不要按当前 `/items` 接口做冻结实现；（d）本轮无新增阻塞；"内置 TLS 监听属主"继续挂起。
- 证据目录：`artifacts/web-mvp/t04-qa/`（16 个文件：auth_api/regression/xtask/contracts/dist+smoke/embedded 日志、QA 手工冒烟脚本与日志、token 哈希核对、405 与 items 边界、前后 sha256 清单、服务端原始日志）。

---

# 回合 5 · T05

**结果：PASS（仅 T05 切片范围，不代表产品全量）** · 回合：5 · PRD 修订：1（ui_revision 1）· 范围：切片（task_ids=[T05]；AC-014、AC-013、AC-015 的默认测试入口侧）

- QA 执行时间：2026-09-12 03:42–04:00（本地 UTC+8；日志内时间戳为 UTC）；执行者：QA 子 agent，独立复现，未采信 RD 报告结论（未复用 RD 的 t05-rd 日志作为通过证据；AC-014 另建仓库外探针 `artifacts/web-mvp/t05-qa/probe-src/`，以 path 依赖直接驱动 `test-support` 公共 API，44 项检查独立通过）。
- 派发核对：`prd.md` §9.1 修订 1（ui_revision 1，与派发包一致）；`implementation.md` §T05 状态 RD_READY；`state.yaml` qa_round=5、scope=slice、current_tasks=[T05]、open_defects 空。
- 结论口径：只覆盖 T05 派发范围。T06+ 业务、T12/T14 真实适配器、T23 `test-live`、T08/T09 的 e2e 入口均不在本切片，列为未覆盖边界。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb44ee2a6088f440eade3f1424546fbb62d`（未新增提交）；`git status` 与 T04 基线一致（M `.gitignore`/`decisions.md`/`state.yaml` + 未跟踪目录） |
| 平台 | macOS Darwin 25.6.0 · aarch64-apple-darwin（本机即 PRD §5.6 必选平台之一） |
| 工具链 | rustc/cargo 1.98.1（rust-toolchain.toml 固定）；Node v26.0.0 / npm 11.12.1；swiftc（PDFKit 独立复核用，系统自带） |
| 被测二进制（发布路径） | `dist/aarch64-apple-darwin/everything-manual`，8 062 784 bytes，sha256 `b8aa9172f452593a4a676e364ac99dbd794eb79712c47e25c42abefcb7a38cb4`——**QA 独立 `cargo xtask dist` 重建后逐字节相同**（与 T04 验收产物同哈希，证明 T05 未触碰生产路径） |
| 数据／凭据 | 全部 QA 自建临时 data-dir（脚本结束即删，实测无残留）、假密码/假密钥 canary；零真实 Provider 调用、零费用 |
| 工作树完整性 | 120 个源／生成文件 sha256 清单（`manifest-before-src.txt` vs `manifest-after-src.txt`）验收前后逐字节一致；最终比对（`manifest-final.txt`）全量差异仅三类：本报告与 `decisions.md` 的 QA 追加（本回合允许写入）、`apps/web/node_modules/.vite/vitest/**/results.json`（运行前端测试再生成的缓存，非源码）；未改 prd.md / state.yaml / 生产代码；QA 临时目录（`/tmp/em-t05-qa*`）已删除、无残留进程 |
| 环境噪声（记录） | 本机长期高负载（`mds_stores` 约 300% CPU；加压复现期间 load average 37–44），对 flake 结论有影响，见"特别裁定" |

## AC 验收矩阵

| AC ID | 期望 | 实际 | 结果 | 命令／步骤／证据 |
| --- | --- | --- | --- | --- |
| AC-014 | fixture 覆盖延迟/断连/429/5xx/畸形 JSON/成功；记录调用次数与请求体；缺脚本时必须失败而非通用成功；断言测试进程无真实外网调用；样例资产 hash 与许可记录可见 | `cargo test -p everything-manual --test fixture_harness` → exit 0，**18 passed / 0 failed / 0 ignored**（debug 0.40s；`--release` 同样 18 passed）。QA 独立探针 44 项检查全过：成功+查询串记录、缺路由→**501**+`ScriptProblem::NoRoute`（且请求仍被记录）、方法不匹配→501、步骤耗尽→501（`repeatLast` 例外）、延迟实测 260ms、断连/`ConnectionReset`/截断（expected=20 actual=8）/429+`Retry-After: 2`/503/畸形 JSON 原样返回/超时 402ms、守卫拒绝 `example.com`/公网 IPv4/IPv6/`0.0.0.0`/`https`（建连前）、`Authorization: Bearer [REDACTED]` 脱敏且 Debug 无 canary、付费 POST 恰 1 次+请求体逐字段断言、chunked 与 `Expect: 100-continue` 实测可用、3 个场景文件可解析、5 个资产固定 sha256 双向一致 | **PASS** | `fixture-harness-qa.log`、`fixture-harness-release-qa.log`、`probe-run.log`（44 checks / 0 failures，探针源 `probe-src/`） |
| AC-014（无外网，独立采样） | 测试进程不得产生真实外网调用 | ① `LocalHttpClient` 仅接受 IP 字面量+回环（代码审查+负例）；② 20 次 fixture_harness 运行期间按 50ms 采样 `lsof -p <test-pid> -a -i -P -n`：**171 次采样、490 行回环连接、0 行非回环**；③ `cargo tree -p everything-manual --edges normal` 中 `test-support` 0 命中、`reqwest`/`rustls` 0 命中（当前测试图无任何 HTTP 客户端/TLS crate） | **PASS** | `net-sampling-summary.txt`、`net-sampling-raw.txt`、`cargo-tree-qa.log` |
| AC-014（资产与许可） | 样例资产 hash 与许可记录可见 | `tests/fixtures/README.md` 记录来源（仓库代码按 glTF 2.0/PDF 1.4/PNG/JPEG 规范现场构造、无第三方素材）与许可（同仓库源码条款）+5 个 sha256；QA 用平台工具独立复核：`shasum` 5 项与 pinned 一致；`sips` 解码 JPEG 32×32、PNG 64×64；两个 PDF 渲染 595×842（21 525 B / 19 588 B）；**PDFKit（Swift）提取文字：text PDF 320 字符、scan PDF 0 字符**（文字层判定独立成立）；python 独立解析 GLB（v2、12 三角面、BIN 长度自洽、内嵌 PNG 三个 chunk CRC 全对） | **PASS** | `assets-sha256-qa.txt`、`sips-decode-qa.log`、`pdfkit-text-extract.log`、`glb-independent-parse.log` |
| AC-013 | release 构建缺 API key：不自动回退 mock/fixture、不产生假模型；ready 依据 DB/迁移/data-dir；云端不可达不使 ready 失败 | 用 QA 独立重建的 release 二进制（`b8aa9172…`）在空 data-dir、无密钥：`init` exit 0；`/health/live` 200；`/health/ready` 200 `{process,data_directory,database,migrations}`；启动日志 `provider_not_configured` ×2（tripo/manual_ai）且消息明示"不回退 mock"；`/settings/status` `providersConfigured=false`×2、`capabilities.generation=false`；`lsof` 全采样 0 条外部连接（仅回环 LISTEN）；**另配 `providers.tripo.base_url=https://203.0.113.9/v3`（TEST-NET 不可达）重启后 ready 仍 200、0 外部连接**；配置确实被加载（同文件加未知键 → exit 3 指名路径）。fixture_harness 内 `app_without_provider_keys_never_contacts_the_fixture` 同时断言"无密钥时 fixture 0 次调用" | **PASS** | `release-smoke.log`、`release-smoke-serve-p1.log`、`release-smoke-serve-p2.log`；`fixture-harness-qa.log` |
| AC-015（默认测试入口侧） | `cargo xtask check`（与前端默认测试入口）不触发任何真实收费 API | `cargo xtask check` → exit 0，**7/7 `[通过]`**（fmt/clippy/workspace 测试/前端 lint/typecheck/vitest/合同检查），无 `|| true` 短路；`cargo test --workspace` 中全部测试指向本机 fixture 或进程内路由；前端默认入口 `npm run test`（vitest）对 `fetch` 用 `vi.stubGlobal` 替身（审查 `client.test.ts`/`App.test.tsx`），无真实网络；服务端生产依赖树无 HTTP 客户端（见上）。`test-live` 属 T23、`test:e2e` 属 T08/T09，均未实现、不计本切片 | **PASS（仅默认入口侧）** | `xtask-check-qa.log`；`apps/web/src/api/client.test.ts`、`App.test.tsx` |
| 卡内·生产隔离 | 测试开关不进入生产默认 | 三重独立复核：① `crates/server/Cargo.toml` 仅 `[dev-dependencies]` 含 test-support，生产源码 0 处引用（QA 自行 grep `crates/server/src` 0 命中，且无 "fixture" 字样）；② `cargo tree --edges normal` 0 命中、`--edges normal -i test-support` = "nothing to print"、`--edges dev -i` 显示 `[dev-dependencies]` 关系；③ release 二进制 strings：`fixture script missing`/`fixture-accept`/`test.support`/`fixture_harness`/`tests/fixtures` 均 0 命中，正例 `openapi.tripo3d.ai` 1 命中（扫描有效） | **PASS** | `cargo-tree-qa.log`、`release-binary-strings-qa.log` |
| 卡内·场景机制与记录 API | 本机绑定、场景脚本、调用/请求体记录与断言 | 见 AC-014 行；服务器只 `bind(127.0.0.1:0)` 并断言 `is_loopback`（探针实测 `127.0.0.1:54300`）；场景 JSON `{method,path,match,repeatLast,steps[]}`、响应体可引用 `responses/*.json`；断言 API `assert_called_once/assert_called_times/call_count/request_total/requests_matching/json_body/header_value/script_problems` 全部实测 | **PASS** | `probe-run.log`；`crates/test-support/src/{server,scenario,record}.rs`（代码审查） |
| 回归（T01–T04） | bootstrap/config_cli/storage/auth_api/contracts/check 证据链不破坏 | 4 套件全过：auth_api 13、bootstrap 5、config_cli 12、storage 15（0 failed/0 ignored）；`cargo xtask contracts --check` exit 0，`openapi.json` 与 `generated.ts` 均 `[一致]`；`cargo xtask check` 7/7；`cargo test --workspace` 22 轮全过（每轮 133 = 62+13+5+12+18+15+8，另有 1 条刻意 `ignore` 的 doctest） | **PASS** | `regression-targets-qa.log`、`contracts-check-qa.log`、`xtask-check-qa.log`、`flake-workspace-rounds.log`、`flake-hunt*.log` |
| 回归（发布链） | dist + smoke-bootstrap 与 T04 一致 | QA 独立 `cargo xtask dist --target aarch64-apple-darwin` exit 0，SHA256 `b8aa9172…`（与 T04 逐字节相同）；`smoke-bootstrap` exit 0，1 项 `[准备]` + 7 项 `[检查]`（页面/静态资源/health/JSON404/SPA/缺失资源 404）全过 | **PASS** | `dist-qa.log`、`dist-hash-qa.txt`、`smoke-bootstrap-qa.log` |

### 与 RD 记录的差异（QA 核对）

| 项 | RD 记录 | QA 实测 | 判定 |
| --- | --- | --- | --- |
| AC-013 "回退 mock" 措辞 | implementation §T05-6 第 8 条写日志含 `provider_not_configured … 回退 mock` 字样 | 实测日志为 `provider_not_configured` + "Provider 未配置：生成与报价能力将返回"未配置"，不回退 mock；已有资料仍可读"（即日志在否定语境下提到 mock，与 RD 摘录一致，非歧义） | 一致 |
| T04 flake 行号归属 | §T05-7 第 9 条把根因归到 "40ms 窗口 + 60ms sleep" 子句 | RD 报告所引 `limiter.rs:154` 实为 **1ms 窗口子句**（`new(0, 1ms)` 后立即断言）；40ms 子句在第 156–163 行、有自定义消息。两个子句都有真实时间依赖，RD 的修复建议应覆盖两者 | 记录，见 BUG-001 |

## 缺陷

### BUG-001 · `window_restarts_after_expiry_and_zero_limits_are_clamped` 单测存在真实时间依赖（时序 flake，T04 既有代码）

- 严重度／状态：**P2 / OPEN**（测试稳定性／ CI 可靠性；无生产行为错误，但会造成随机红灯与误归因）
- 对应 REQ / AC：无直接 AC；属 T04 已验收代码的单元测试（`crates/server/src/http/auth/limiter.rs`）；不属 T05 的 AC 覆盖范围
- 环境与输入：macOS Darwin 25.6.0 aarch64、rustc 1.98.1、debug/release 均涉及；`cargo test --workspace` / `--lib`
- 复现步骤（RD 报告的原始现象）：`cargo test --workspace` 复跑，偶发（RD 称 4 次中 1 次）失败 `assertion failed: limiter.check(ip()).is_err()`（limiter.rs:154）；单跑 lib 10/10 通过
- 期望与实际：期望该用例确定性通过；实际存在两个真实时钟依赖子句——(a) `new(0, 1ms)` 后 `record_failure` → `check` 之间若被调度延迟 ≥1ms，窗口已过期，断言失败（RD 报告失败即此子句）；(b) `new(1, 40ms)` + sleep 60ms 后 `record_failure` → `check` 之间若延迟 ≥40ms，同样失败
- QA 独立复现结论：**未复现**。已执行 22 轮 `cargo test --workspace`（8 轮无加压 + 2 轮 12 burners + 12 轮 16 burners）、110 次 lib 套件整跑（含 25×4 并发高压）、240 次单测定向复跑（debug 180 + release 60）、`--test-threads=1`×3 与 `=16`×3、`--release` 观察——**0 次失败**；期间机器本身处于高负载（mds_stores ≈300%、load 37–44）。按"未复现不得据此判为不存在"原则：机制成立（真实时钟窗口，无注入时钟），RD 报告的现象不能排除；定级 P2 不变
- 证据路径：`artifacts/web-mvp/t05-qa/flake-workspace-rounds.log`、`flake-ws-round-{1..8}.log`、`flake-hunt.log`、`flake-hunt2.log`、`run-flake-*.sh`；RD 侧原始失败输出**未保留**（RD 的 `t05-rd/test-workspace-reruns.log` 只含 3 次全过记录，失败原文仅存于 implementation 叙述）
- 建议修复方向（交 RD，一行级改动 + 语义保持）：把窗口断言改为**可注入时钟**（`LoginRateLimiter` 构造时接受 `now: fn() -> Instant` 或把 `check/record_failure` 改为可传 `now`），或退一步把测试窗口/睡眠放大 ≥100 倍（1ms→≥200ms、40ms→≥500ms+700ms sleep）以消除调度抖动窗口；**不要**删除断言或用重试掩盖
- 对 T05 切片 PASS 的影响：**不阻断**。理由：缺陷位于 T04 已验收切片交付的测试文件（T05 未触碰 `http/auth/**` 任何文件，dist 哈希与 T04 相同证明生产路径零变化）；T05 的必选 AC（AC-014/AC-013/AC-015 默认入口侧）与该用例无关；本回合 22 轮全量复跑未出现失败，不构成"当前切片必选 AC 未通过"。缺陷仍需保留在案，由协调者决定修复落点（建议并入下一张触碰 auth/limiter 的卡，或由协调者指派独立微型修复）
- 回归范围：修复后复验 `cargo test -p everything-manual --lib`（limiter 3 用例）+ `cargo test --workspace` 数轮；建议同时在 `--test-threads=16` 下跑

## 未覆盖边界／已知限制（本回合记录，不冒充通过）

1. `cargo xtask test-live` 的参数/预算校验（AC-015 后半、T23）；真实 Tripo/Manual AI 链路与费用（T23，需授权）。
2. `npm --prefix apps/web run test:e2e`（Playwright）尚不存在（属 T08/T09）；本回合仅验现有默认入口（vitest + xtask check）。
3. T12/T14 真实 reqwest 适配器的字节协议断言未验（本卡只交设施；场景响应是构造样例，非官方原文）。
4. Linux musl 平台的 fixture 行为未验（`reset` 在非 Unix 退化为 FIN）；T22 负责。
5. fixture 服务器无鉴权、不模拟 TLS 握手中断（T05-7 已声明）；不构成安全边界测试对象。
6. 外呼断言是"设计守卫 + 负例 + 采样"组合证据（无 root 级网络追踪）；测试进程理论上仍可用 std TcpStream 直连外网，由"无 http 客户端依赖 + 守卫客户端唯一入口"约束，后续 T12 引入 reqwest 后该证明需改由 fixture 计数与域名约束延续（T02 QA 知识 1 的复现条件）。

## 非代码知识与限制（跨卡复用）

1. **flake 未复现的统计口径**：见 BUG-001；"未复现"不等于"不存在"，且本机高负载环境反而说明触发概率低。后续任何复现必须原样保留失败输出到 artifacts。
2. **fixture 断言 API 与场景语义（T12/T14 直接用）**：缺脚本/步骤耗尽是 **501 + `script_problems()`**（不是异常），适配器测试应连同 `assert_no_script_problems()` 一起断言；`repeatLast` 只对已声明路由生效；`header_value("authorization")` 返回 `Bearer [REDACTED]`（保留 scheme）；chunked 与 `Expect: 100-continue` 已可承接 reqwest 流式 multipart。
3. **样例资产独立验真手法**：PDF 文字层用系统 `swiftc` + PDFKit（`PDFDocument.page.string`；320 vs 0 字符）；GLB 用 python `struct` 独立解析（不复用仓库校验器）；`sips` 解码/渲染。均可脱离仓库代码验证"资产真实可用"。
4. **本机外呼采样法**：`while kill -0 <pid>; do lsof -p <pid> -a -i -P -n; sleep 0.05; done`——fixture_harness 20 次运行 171 采样只出现回环连接，可作为后续 HTTP 类卡的补充证据（T12 起配合 fixture 计数使用）。
5. **release 无密钥行为复核法**：空 data-dir + `--password-file` init → serve → `/health/ready` 200 + 日志 `provider_not_configured`；`lsof` 0 外部连接；不可达 `base_url`（`https://203.0.113.9/`）不影响 ready；"配置已加载"可用未知键 exit 3 快速证明。
6. **RD 遗留临时文件（卫生，非阻断）**：本回合实测 `/tmp/em-t05-*.log`、`/tmp/em-t05-render/`、`/tmp/em-lib-run-1..10.log`、`/tmp/em-ws-run-1..3.log` 共 **18 个** RD T05 会话残留（时间戳 03:34–03:40）；QA 未删除他人现场，建议协调者提示 RD 清理（T04 的建议 6 同类问题再现）。QA 自身临时目录（`/tmp/em-t05-qa*`）已全部删除、进程无残留。
7. 以上 2/3/4/5 已同步 `llmdoc/decisions.md`「QA 验收知识 · T05 验收知识」。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 2026-09-12 | slice | T01（AC-001、AC-011） | 1（ui_revision 1） | PASS | 本文件 |
| 2 | 2026-09-12 | slice | T02（AC-005、AC-006、AC-012 配置侧） | 1（ui_revision 1） | PASS | 本文件 |
| 3 | 2026-09-12 | slice | T03（AC-007、AC-008） | 1（ui_revision 1） | PASS | 本文件 |
| 4 | 2026-09-12 | slice | T04（AC-003、AC-004、AC-012 settings 侧、AC-066 错误结构侧） | 1（ui_revision 1） | PASS | 本文件 |
| 5 | 2026-09-12 | slice | T05（AC-014、AC-013、AC-015 默认入口侧） | 1（ui_revision 1） | PASS | 本文件 |

- 交接给 RD：本回合无阻断缺陷需返工；**BUG-001（P2）** 为 T04 既有单测时序 flake，建议并入下一张触碰 `http/auth/limiter.rs` 的卡或由协调者指派微型修复（修复方向见缺陷条目；QA 不代改）。
- 交接给协调者：（a）`accepted_tasks` 追加 `{task_id: T05, prd_revision: 1, qa_round: 5, report: llmdoc/requirements/web-mvp/qa-report.md, accepted_ac_ids: [AC-014, AC-013, AC-015（默认测试入口侧）], status: accepted}`；（b）`qa_history` 追加本回合；（c）建议 `open_defects` 记录 `BUG-001`（P2、OPEN、非阻断，目标修复落点建议 T06+ 任一触碰 auth 的卡或独立微型修复；不记录也可，但需由本报告持续跟踪）；（d）T04 报告的 T07 剩余项清单、T02/T03 的"内置 TLS 监听属主"两个挂起项继续有效。
- 证据目录：`artifacts/web-mvp/t05-qa/`（44 项：fixture_harness debug/release、回归、xtask check、contracts、cargo tree、dist+smoke、release 冒烟脚本与双阶段服务日志、二进制字符串扫描、独立探针源与日志、PDFKit/GLB/资产哈希外部复核、外呼采样、flake 四组复现脚本与 22 轮日志、前后 sha256 清单）。

---

# 回合 6 · T06（含 BUG-001 复验）

**结果：PASS（仅 T06 切片范围 + BUG-001 关闭，不代表产品全量）** · 回合：6 · PRD 修订：1（ui_revision 1）· 范围：切片（task_ids=[T06]；AC-018、AC-019；BUG-001 复验）

- QA 执行时间：2026-09-12 04:20–04:36（本地 UTC+8；服务端日志内时间戳为 UTC）；执行者：QA 子 agent，独立复现，未采信 RD 报告结论（未复用 `artifacts/web-mvp/t06-rd/` 的任何日志作为通过证据；自建样例生成器、自建 HTTP 客户端、自建磁盘镜像与冒烟脚本）。
- 派发核对：`prd.md` §9.1 修订 1（ui_revision 1，与派发包一致）；`implementation.md` §T06 状态 RD_READY；`state.yaml` qa_round=6、qa_scope=slice、current_tasks=[T06]、open_defects=[BUG-001 fixed_pending_reverify]。
- 结论口径：只覆盖 T06 派发范围与 BUG-001。T07 物品业务规则、T09 准备阶段（加密/超页数权威拒绝）、T13 模型下载不在本切片；AC-018/AC-019 的 UI 侧（UI-009/UI-010）属 T08/T21，未验。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb44ee2a6088f440eade3f1424546fbb62d`（未新增提交）；`git status` 与本回合进入时基线一致（M `.gitignore`/`decisions.md`/`state.yaml` + 未跟踪目录） |
| 平台／工具链 | macOS Darwin 25.6.0（26.6.2）· aarch64-apple-darwin；rustc/cargo 1.98.1（rust-toolchain.toml 固定）；Node v26.0.0；`sqlite3`、`python3`、`sips`、`hdiutil`、`swiftc`+PDFKit（均为系统自带，用于独立复核） |
| 被测二进制 | `dist/aarch64-apple-darwin/everything-manual`，8 626 640 B，sha256 `0c457a2a5b7204c37a64d854bf5e09986894f2c1d0cbde5173781ae450bb95fa`——**QA 独立 `cargo xtask dist` 重建后逐字节相同**（`dist-hash-before.txt` = `dist-hash-after.txt`），冒烟与全部手工负例都跑在该哈希上 |
| 数据／凭据 | 全部 QA 自建临时 data-dir 与 canary 假密码（`qa-*-password-*`）；自建原创样例（`qa-assets.py`：512×512 PNG、`sips` 转换的 JPEG、手写 xref 的 3 页 PDF/40 MiB PDF/像素炸弹/截断/CRC 损坏样本）；零真实 Provider 调用、零费用、零外网 |
| 独立验真 | 自建 PDF 用 **PDFKit（Swift）** 独立复核：`manual.pdf` 与 `big.pdf` 均 `pages=3 encrypted=false`；自建 PNG/JPEG 用 `sips` 复核 512×512。因此"上传 201"不是把垃圾当合法内容的假阳性 |
| 工作树完整性 | 101 个源／测试／生成文件的 sha256 清单（`manifest-before.txt` vs `manifest-after.txt`）**完全一致**；前端源码 mtime 未变（仅 `apps/web/dist` 构建产物与 vitest 缓存被测试/重建刷新）；无残留进程、无残留磁盘镜像挂载、QA 临时目录（`/tmp/em-t06-qa-*`）全部删除 |

## AC 验收矩阵

| AC ID | 期望 | 实际 | 结果 | 命令／步骤／证据 |
| --- | --- | --- | --- | --- |
| AC-018 | `POST /items/{id}/assets` 上传 PDF/JPEG/PNG；重复上传同文件；`GET`/`HEAD`/Range；返回 asset 且按 sha256 去重；ETag + `If-None-Match` 304；HEAD 无 body；206 的 Content-Range/Length 正确；416；响应不含磁盘路径 | ① `cargo test -p everything-manual --test assets` → exit 0，**12 passed / 0 failed / 0 ignored**（2.04s）；② 真实 release 二进制 + 真实 data-dir 手工冒烟 **136/136 项通过**：PNG/JPEG/PDF 均 201（含 `filename=../../../qa-pwn.png`、`../../../../pwned.pdf` 只保留 basename）；重复上传同内容 → 同 sha256、**不同 asset id**、`blobs` 行 1 行/文件 1 个；`GET` 200 字节与源文件 `cmp` 一致、`content-length/etag/accept-ranges/nosniff` 正确、无 `content-encoding`；**HEAD bodyLen=0**（自建 HTTP 客户端量，不用 curl 的 HEAD 包装）且头与 GET 相同；206 三种写法（`0-9`/`10-`/`-5`）的 `Content-Range`、`Content-Length`、字节切片逐项一致；416（`99999999-`、`-0`）带 `Content-Range: bytes */377121`；多区间回落完整 200；304（bodyLen=0、带 ETag）与不命中 200；`If-Range` 强校验器匹配 206、stale/弱/日期形式均完整 200；**全部响应体与响应头扫描 0 处出现 data-dir 路径或 `blobs/`** | **PASS** | 04:20 `cargo test -p everything-manual --test assets`（`ac-assets-run1.log`）；04:27 `bash artifacts/web-mvp/t06-qa/manual-smoke.sh`（`manual-smoke-run2.log`、`manual-smoke2-run2.log`、`raw/db-state.txt`、`raw/blobs-layout.txt`、脚本内嵌独立样例生成器 `qa-assets.py` 与 `qa-http.py`） |
| AC-019 | 伪造类型 415；超限 413；像素炸弹/解码失败 422；路径穿越不越界；越权 404 或明确磁盘不足错误；不半提交；DB 回滚不删共享 blob；tmp 隔离 | ① 集成 12 用例全过（含「fsync 后元数据事务外键失败 → 共享 blob 文件仍在、原资产 200 且字节一致」「崩溃孤儿隔离 + 引用保留 + 状态收敛」「注入空间探测的两段检查」）；② 手工负例：文本/PNG 冒充 PDF、文本冒充 JPEG → **415**；`purpose=model`/`page_image`/缺字段/未知字段/非 multipart → 415/422 按合同；缺 CSRF → 403；无会话 → 401；未知资产/幽灵物品 → 404；21 MiB 照片（含 purpose 后到）与 52 MiB 请求体 → **413**；60000×60000 像素炸弹 → **422 + `details.reason=imagePixels`**；截断/CRC 损坏 PNG、缺 EOI 的 JPEG、结构不完整 PDF、非 UTF-8 pageText → **422**；路径穿越三形态（`../../..`、绝对路径、反斜杠）→ data-dir 内外均无越界文件，内容只在 `blobs/<前2位>/<sha256>`；全部失败路径后 `assets/blobs` 行数、`blobs/` 文件数、`tmp` 文件数**与失败前完全一致**；③ **真实磁盘满**（32 MiB APFS 磁盘镜像 + 真实 statvfs，无注入）：40 MiB 上传 → **413 + `details.reason=insufficientStorage`**、消息含所需/可用字节、`tmp`/`blobs`/行数全 0，同一环境小文件仍 201；④ **写流中途 ENOSPC**（限速上传中途用填充文件占满磁盘）：**无半提交**（0 行、0 tmp、0 blob 文件、tmp 守卫已清理），响应见非阻断建议 1；⑤ 隔离：`tmp` 残留与无引用孤儿被移入 `quarantine/`（内容逐字节保留、被引用 blob 不动、`missing↔stored` 收敛、二次扫描幂等、`blobs` 目录内容与操作前逐字节一致）；⑥ 并发 4 份同内容上传 → 4×201、blob 1 行 1 文件、4 条资产、tmp 0、文件名与内容 sha256 全部一致 | **PASS** | `ac-assets-run1.log`；`manual-smoke-run2.log`；`disk-full-real.log`（真实镜像 413 + 无半提交，10/10）；`disk-full-midwrite.log`；`raw/quarantine-listing.txt`、`raw/serve{1..4}.log`、`raw/concurrent-serve.log` |

### 卡内检查项（T06 卡 + validation-release §2/§3）

| 检查项 | 实际结果 | 判定 | 证据 |
| --- | --- | --- | --- |
| **流式处理（不得整文件读入内存）** | 可观察证据：40 MiB PDF 以 `--limit-rate 5m` 上传（约 8s，112 次 50ms 采样服务端 RSS）：**峰值与上传前完全相同（29 792 KB，Δ=0），上传后 30 432 KB（+0.6 MiB，与文件大小无关）**；同一次上传的落盘时间线（200ms 采样）显示 `tmp/<uuid>.part` 单文件从 0 单调增长到 41 680 638 B，期间 `blobs/` 文件数、`blobs` 行、`assets` 行恒为 0；完成后一次性变为 tmp=0/blobs 文件 1/assets 行 1（HTTP 201）。代码路径复核：`http/assets.rs` 用 `field.chunk()` 逐块读、`StagedWriter::write` 只处理当前 chunk、响应体用 `ReaderStream`（无整文件 `Vec`） | PASS | `raw/rss-samples.txt`、`ordering-timeline.log`、`crates/server/src/assets/blob_store.rs`、`crates/server/src/http/assets.rs` |
| 读流二次计数（两道体积防线） | 真实 HTTP：21 MiB 照片（Content-Length 已知、低于路由上限 51 MiB）→ 413，说明是**读流计数**而非解析前上限拦的；`purpose` 排在 `file` 之后时同样 413（读完按用途上限兜底）；52 MiB 请求体 → 解析前 413。配置覆写复核：`max_photo_bytes=4096` 的真实配置下 377 KB PNG → 413 | PASS | `manual-smoke-run2.log` §6；`raw/limits-serve.log` |
| Content-Type/后缀不可信（magic 为准） | 双向实测：真 PNG 谎报 `application/pdf` + `.pdf` 后缀 + `purpose=photo` → **201 且 `mime=image/png`**；非图片谎报 `image/png` → 415；文本谎报 `application/pdf` → 415 | PASS | `manual-smoke-run2.log` §2b |
| `purpose` 取值校验 | `model`/`page_image`/空/未知 → 422；线上 camelCase（`pageImage`/`pageText`）→ 201；SQL 侧 `page_image` 仅由仓储解析（`repo/assets.rs::parse_purpose`），单测断言"SQL 风格值不是线上值" | PASS | `manual-smoke-run2.log` §5；`crates/server/src/assets/upload.rs` 单测；`raw/db-state.txt` |
| 原子落盘顺序（tmp→fsync→rename→元数据） | 三重观察：① 落盘时间线（上表）显示元数据只在 tmp 完成并 rename 后出现；② 提交后一致性：14 条 asset 全部有对应 `blobs/<前2位>/<sha256>` 文件且逐字节 sha256 相符（"元数据可见 ⇒ 文件在"）；③ 注入失败：fsync 后元数据事务外键失败 → 文件保留、原资产仍可读（集成用例）；真实 ENOSPC 写入失败 → tmp 被守卫清理、0 行 0 文件。源码顺序：`flush→sync_all→rename→sync_dir→事务` | PASS | `ordering-timeline.log`；`manual-smoke2-run2.log` §15；`ac-assets-run1.log`；`crates/server/src/assets/{blob_store,upload}.rs` |
| 磁盘预留空间检查 | 真实 statvfs（32 MiB 镜像）：不足 → 413 + `insufficientStorage` + 不半提交；落盘前复检分支由集成的脚本化探测用例覆盖（预检通过、复检失败 → 413 + tmp 清理 + 0 行） | PASS | `disk-full-real.log`；`ac-assets-run1.log` |
| `blobs/<sha256 前缀>/<sha256>` 布局 | 真实 data-dir 文件清单：`blobs/24/24d6…`、`41/4161…`、`82/824c…`、`8f/8f05…`、`c0/c05e…`、`dc/dc30…`、`e1/e14f…`、`e3/e3b0…`（空文件）——目录名=sha 前 2 位、文件名=完整 sha，且每个文件 `shasum` 与文件名一致 | PASS | `raw/blobs-layout.txt`、`raw/db-state.txt` |
| 文件名不作路径 | 三个穿越形态只保留 basename 且 `originalName` 正确；data-dir 内外无被穿越文件；内容寻址路径完全由 sha256 拼出（`blob_path` 单测） | PASS | `manual-smoke-run2.log` §2c；`crates/server/src/assets/upload.rs` 单测 |
| 物品累计口径（去重后求和） | 真实 SQL 复核：物品累计 = 该物品 `DISTINCT blob_id` 的 size 和（同内容重复引用不重复计数）；配置 `max_item_total_bytes=1000` 的真实服务上：600 B + 600 B → 第 2 次 **413 + `details.reason=itemTotalLimit`**（含 current/incoming/limit），无半提交 | PASS | `manual-smoke-run2.log` §3；`raw/item-total-serve.log`、`raw/limits-serve.log` |
| 配置覆写路径（不写死默认值） | `[limits]` 的 `max_photo_bytes` / `max_pdf_pages` / `max_item_total_bytes` 均实测可覆写并在真实服务上生效（413 / 422 `pdfPageLimit`(pageCount=3,maxPages=2) / 413 `itemTotalLimit`） | PASS | `raw/limits-serve.log`、`raw/item-total-serve.log` |
| 隔离区状态机、`serve` 启动扫描接线 | 3 次真实重启：第一次 `asset_scan` = `tmpQuarantined=1, blobsQuarantined=1, blobsKept=3, blobsMarkedMissing=1`（我手工移走 1 个被引用文件）；`check --data-dir` 在存在 tmp 残留时 **exit 0 且不移动任何文件**（只读语义保持）；`init`/`check` 无扫描调用点 | PASS | `manual-smoke-run2.log` §12；`crates/server/src/config/commands.rs`、`crates/server/src/assets/maintenance.rs` |
| `quarantined` 状态的防御分支 | 通过 DB 注入 `storage_state='quarantined'` 后上传同内容 → **422 + 明确文案**（不静默解隔离、不删除文件、无新行）；T06 自身流程不会产生该状态（留给 T20/管理员） | PASS | `raw/quarantined-state-response.json`、`raw/quarantined-state-serve.log` |

## BUG-001 复验（P2 时序 flake → **CLOSED**）

- 修复位置与形态：`crates/server/src/http/auth/limiter.rs` 引入 `TimeSource::System | Manual(Arc<ManualClock>)`；`check`/`record_failure` 的时间全部取自 `self.time_source.now()`；`LoginRateLimiter::new(max, window)` 仍是生产构造（系统单调时钟），`with_time_source` 只是注入口。
- **机制层面核对（不是只看跑绿）**：
  1. 全文件时间读取只有两处（`check`、`record_failure`），都走时间源；`Instant::now()` 只出现在 `TimeSource::System` 分支与 `ManualClock::new` 的基准值。`grep -rn "TimeSource\|ManualClock\|with_time_source" crates/` 在 `limiter.rs` 之外 **0 命中**；`AppState::with_asset_store` 仍调用 `LoginRateLimiter::new(settings.session.login_rate_limit_per_minute, window)`（`http/state.rs`）。因此不存在"测试用时钟泄漏到生产"或"仍读真实时钟"的路径。
  2. 测试文件内 `thread::sleep`/`tokio::time::sleep` **0 处**（`grep` 只命中注释）；三个既有用例的等待全部改为 `clock.advance(...)`，断言未删并更强（窗口内 59ms 仍 429、过期后新窗口起点=第二次失败、`retry >= 1`）；新增守护用例 `millisecond_window_is_deterministic_with_injected_clock`（1ms 窗口、不推进时钟、**连续 1000 次**判定必须 429，再推进 1ms 放行）——旧实现下该循环必然因调度延迟失败。
  3. **对外语义与默认值未变**（HTTP 层实测，非仅单测）：真实二进制默认配置下 5 次错误密码全 401、**第 6 次（正确密码）→ 429 + `retry-after: 60` + `code=RATE_LIMITED`**（与 T04 验收记录一致）；`serve` 启动日志 `loginRateLimitPerMinute=5`；`window.max(1ms)`/`max_failures.max(1)` 收口保留。
- 回归复跑（QA 自跑，原始日志 `artifacts/web-mvp/t06-qa/bug001/`）：`--lib` 3 轮 ×**89 passed/0 failed**（含 4 个 limiter 用例）；`cargo test --workspace` **6 轮 ×172 passed/0 failed**；`--test-threads=16` **4 轮 ×172**；定向 `limiter --test-threads=16` 3 轮 ×4；全部日志无 `FAILED`/`panicked`/`failures:`。共 16 轮、0 失败。
- 结论：**已修复并复验关闭（CLOSED）**。缺陷根因（真实时钟窗口 + 毫秒级窗口断言）已由结构性注入时钟消除，而非靠放大窗口/重试掩盖；T05 §T05-7 第 9 条"偶发失败复跑归因"的说法自本回合起失效。证据：`artifacts/web-mvp/t06-qa/bug001/`（16 个日志）、`manual-smoke-run2.log` §9、`raw/serve1.log`（serve_start 行）。

## 特别评估项（本回合派发要求）

1. **磁盘满用 `413 + details.reason=insufficientStorage`（合同未定义 507）——非阻断，建议 PM 决策后写回合同**
   - `contracts.md` §1 的稳定集合为 401/403/404/409/413/415/422/429/503，**没有 507/`INSUFFICIENT_STORAGE`**；§7 只要求"磁盘预留空间不足返回明确错误，不半提交"；PRD AC-019 只要求"明确的磁盘不足错误"。因此该形态**不构成合同冲突**，也不违反任何必选 AC。实测（真实磁盘镜像）该响应可被前端零歧义识别（`details.reason` + 所需/可用字节 + 可读文案），且失败路径无半提交。
   - 保留意见：413 在合同里的语义是"超限"（客户端输入超限），而磁盘满是**服务端条件**，语义上是 `507 Insufficient Storage`（RFC 9110/4918）。当前实现把服务端资源不足塞进 413 属于"合同未覆盖→用相邻码 + details 消歧"，可接受但应在合同层留痕。
   - 建议（交协调者/PM）：在 `contracts.md` §1/§7 明确"磁盘不足 = 413 + `details.reason=insufficientStorage`"（现状即合同），或新增 507/独立错误码（需 PM 批准 + 合同变更，改动面仅 `assets/error.rs` + 文档 + 测试）。**不影响本切片 PASS**。
2. **RD 唯一越界改动 `config/commands.rs`（约 25 行 serve 启动扫描接线）——必要接线，未见未授权行为**
   - 位置与顺序：`serve` 在 `datadir::verify` → 排他锁 → `Database::open_and_migrate` 之后、`TcpListener::bind` 之前调用 `maintenance::scan_and_quarantine`，失败只记 WARN 不阻塞启动。该位置保证扫描时没有在途上传（tmp 里任何文件必为崩溃残留）、迁移已完成（blobs 表存在）。
   - 越界范围核对：以 mtime + 内容审查列出本卡触碰的全部文件（26 个代码/生成物），asset 相关改动仅此一处接线（`grep` 只有 1 个调用点，`init`/`check`/`backup`/`restore` 均不扫描）；实测 `check --data-dir` 在存在 tmp 残留与 `blobs/` 散文件时 exit 0 且**不移动任何文件**，只读语义保持。否则 `scan_and_quarantine` 会成为无人调用的库函数，"崩溃残留隔离"在真实启动路径不成立——属必要接线。
   - 建议：请协调者把该文件补记入 T06 卡的允许范围（过程性补记，不要求返工）；启动全量遍历 `blobs/` 的时延风险由 RD 记入 T06-8 第 6 条，交 T20/T22。
3. **上传接受加密 PDF 与对象流压缩 PDF（权威拒绝在 T09）——与 REQ-012/REQ-014 边界一致，记录为非阻断限制**
   - 边界依据：REQ-012 明文"加密 PDF 的拒绝发生在准备阶段（见 REQ-014）"；REQ-014/AC-024 把"加密、>100 页"的拒绝放在 preparation。PRD §5.3 的"加密 PDF 首版拒绝"因此由 T09 满足。
   - 实测：手工构造 trailer 带 `/Encrypt` 的 PDF → **201 + WARN 日志**；构造 `/Root` 对象在纯对象区不可定位（模拟 `/ObjStm`）→ **201 + WARN**（`reason=未定位到目录对象 1 0 R（可能被对象流压缩）`）。两者都避免了把真实厂商 PDF 一律误判 422。
   - 影响与交接：**T06 的 100 页 422 只是第一道防线**（页树可解析时生效）；加密与对象流 PDF 的页数在 T09 才权威判定。T09 必须覆盖：加密拒绝、>100 页拒绝（含对象流页树）、以及大量 `Unparsed` 出现时把对象流解析纳入 T09 范围。此前向 PM 记录：T06 的此行为是"按 REQ-012/REQ-014 的正确分工"，不是缺陷。
   - 附带观察（供参考，不构成缺陷）：PDF 探针要求 `startxref` 落在文件尾 8192 B 内。QA 第一版 40 MiB 样例把填充放在 `startxref` 与 `%%EOF` 之间 → 422"缺少 startxref"；经 PDFKit 复核该文件对真实阅读器同样不可用（PDF 规范要求 `%%EOF` 为最后一行、`startxref` 紧邻文件尾），故该拒绝是正确的，不是误杀。

## 缺陷

**无阻断缺陷（0 个 OPEN/BUG）**。本回合未发现违反 AC-018/AC-019 或卡内必测项的问题；BUG-001 复验关闭。

### 非阻断建议（不计入阻断项，不要求 T06 返工）

1. **写流中途磁盘耗尽 → 500 通用错误（P3，建议后续卡处理）**：预检与复检之间的竞态窗口里空间被占满时，响应是 `500 INTERNAL`（客户端只见通用文案，具体 `No space left on device (os error 28)` 只在服务端日志）。**不半提交**这一硬要求满足（0 行、0 tmp、0 文件）。建议后续把写流阶段的 `ErrorKind::StorageFull`/ENOSPC 也映射为 413 + `details.reason=insufficientStorage`（一行级改动，与建议 1 的合同决策一起做）。证据 `disk-full-midwrite.log`。
2. **`serve` 启动扫描是全量遍历 `blobs/`**（RD 已记 T06-8 第 6 条）：大数据量下的启动时延未测，T20/T22 若要求可加增量策略。
3. **T06 常量未配置化**（`PAGE_TEXT_MAX_BYTES=2 MiB`、图片单边 20 000 px、解码像素 80 MP）：PRD 未给数值，RD 以常量落地并留提案；T09 实测页文字更大时需新增配置键。
4. **PDF 探针边界**：对象流页树 → `Unparsed` 放行（见特别评估项 3），页数上限需 T09 兜底；若 T09 实测 `Unparsed` 比例高，应把对象流解析纳入 T09 而不是在 T06 加启发式。

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实 ≥500 MiB 物品累计**未在真实磁盘构造（用 `[limits]` 覆写为 1000 B 的真实服务复现同一代码路径：413 + `itemTotalLimit`）；超大数据量下 `item_total_bytes` 的查询性能未测。
2. **多进程／跨进程并发上传**未测（单管理员单进程模型；同内容 4 路并发在单进程内已验证去重与原子性）。
3. **非 Unix 平台**（无 `statvfs` → 跳过预留检查）未验；必选发布平台 Linux musl 属 T22。
4. **Range 在超大文件（>4 GiB 偏移）**与多区间拼接语义未测（合同明确首版回落完整 200，已验）。
5. UI 侧（UI-009 上传控件、UI-010 重试/移除）与 Playwright 属 T08/T21，未验；`HEAD` 未进 OpenAPI（RD 已记录，OpenAPI 3.1 无 HEAD 操作对象）。
6. AC-019 的"越权资产"在本产品是"单管理员 + 物品归属"模型：跨物品归属用仓储原语 `find_for_item` 验证（T07/T09 必须经过它），HTTP 层目前没有"他人资产"的第二主体可构造。
7. 写流中途 ENOSPC 的错误码形态（非阻断建议 1）；加密/对象流 PDF 的权威拒绝（T09）。

## 非代码知识与限制（跨卡复用）

1. **BUG-001 修复的机制判据（T06+ 任何"可注入时钟"改动可复用）**：确认"消除时钟依赖"要同时看四点——(a) 被测代码的所有时间读取是否都经同一时间源；(b) 生产构造路径是否仍用系统时钟且默认值/收口未变（HTTP 层实测 5 次→第 6 次 429 + `retry-after: 60` 比单测更有说服力）；(c) 测试文件里是否还有真实 `sleep`/毫秒窗口断言；(d) 守护用例是否在"不推进时钟"下高频重复判定（1 ms 窗口 ×1000 次）。
2. **真实磁盘满的构造法（本次新增，可复用）**：`hdiutil create -size 32m -fs APFS` + `attach -mountpoint` 得到一个真实小文件系统，`init` 需注意**密码文件必须 0600**（否则 exit 3，本次 QA 首轮就踩到）；随后用 `[limits]`/真实 statvfs 观察 413/无半提交。写流中途 ENOSPC 用 `curl --limit-rate` + 并发填充文件实现。收尾必须 `detach` + 删除镜像（本次已确认无残留挂载）。
3. **原子落盘顺序的可观察法**：限速上传 + 200 ms 采样 `tmp` 文件数与字节数、`blobs/` 文件数、`assets`/`blobs` 行数，可直接看到"tmp 单调增长期间元数据恒为 0，完成后一次性翻转"。
4. **流式处理的内存判据**：`ps -o rss=` 采样服务进程（40 MiB 限速上传，112 次采样）——峰值 = 基线即证明未整文件读入内存；比只看代码更有力。
5. **上传内容的独立验真**：自建 PDF 用系统 `swiftc`+PDFKit（`pageCount`/`isEncrypted`）复核，避免"自建样例本身非法却被 201 接受"的假阳性；PNG/JPEG 用 `sips`。
6. **本卡实测的边界语义（T09 直接复用）**：加密 PDF 与对象流 PDF 在上传阶段 201 + WARN；`startxref` 必须落在文件尾 8192 B（否则 422，与真实阅读器一致）；空文件（0 B pageText）可上传，任何 Range → 416 + `bytes */0`；`quarantined` 状态行不会由 T06 流程产生。
7. 以上 1–6 已追加到 `llmdoc/decisions.md`「QA 验收知识 · T06 验收知识」。
8. **`manual-smoke-run1.log` 中的 8 个 FAIL 是 QA 脚本/样例自身的缺陷，不是产品缺陷**（保留原始日志以留痕）：① 40 MiB 样例的填充放在 `startxref` 与 `%%EOF` 之间 → 产品按合同拒绝 422，仿真器改把填充放到 xref 之前后 201（该样例对 PDFKit 也不可用）；② 304 响应 curl 不创建 `-o` 文件导致"body 为空"断言取到空串（改用 `[ -f ]` 判断）；③ "文本冒充 PDF"误用了带 `%PDF-` 头的坏 PDF，产品正确给 422，改用真正的纯文本后得 415；④ 去重引用计数/文件数期望值写错（未把"PNG 谎报 PDF"用例计入）。修正后 `manual-smoke-run2.log` 为 136/136。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 6 | 2026-09-12 | slice | T06（AC-018、AC-019）+ BUG-001 复验 | 1（ui_revision 1） | PASS | 本文件 |

- 交接给 RD：本回合无缺陷需返工（BUG-001 已由 RD 修复，QA 复验 **CLOSED**）；建议 1/2/3/4 为非阻断项，按协调者排期（建议 1/3 与 PM 的合同决策一起做）。
- 交接给协调者：（a）`accepted_tasks` 追加 `{task_id: T06, prd_revision: 1, qa_round: 6, report: llmdoc/requirements/web-mvp/qa-report.md, accepted_ac_ids: [AC-018, AC-019], status: accepted}`；（b）`qa_history` 追加本回合（result: PASS）；（c）`open_defects` 移除 `BUG-001`（已修复并复验关闭，依据：机制核对 + 16 轮 0 失败 + HTTP 层默认语义不变）；（d）**PM 决策项**：磁盘满错误码形态（413+details 现状 vs 507/独立码，见特别评估项 1）；(e) 请把 `config/commands.rs` 补记入 T06 允许范围；(f) T09 必须覆盖加密/对象流 PDF 的权威拒绝与页数上限（含 `Unparsed` 比例复核）；(g) T04 报告的 T07 剩余项、T02/T03 的"内置 TLS 监听属主"挂起项继续有效。
- 证据目录：`artifacts/web-mvp/t06-qa/`（25 个文件 + `bug001/` 16 个回归日志 + `raw/` 13 项）：`ac-assets-run1.log`、`bug001/`（16 个 16 轮复跑日志）、`xtask-check.log`、`dist-qa.log`+`dist-hash-before/after.txt`、`smoke-bootstrap-qa.log`、`manual-smoke-run{1,2}.log`+`manual-smoke.sh`、`manual-smoke2-run{1,2}.log`+`manual-smoke2.sh`、`ordering-timeline.{sh,log}`、`disk-full-real.{sh,log}`、`disk-full-midwrite.{sh,log}`、`qa-assets.py`、`qa-http.py`、`manifest-before/after.txt`、`raw/`（serve1–4、rss 采样、db 状态、blobs 布局、quarantine 清单、limits/item-total/concurrent/quarantined 服务日志与响应）。

---

# 回合 7 · T07

**结果：PASS（仅 T07 切片范围，不代表产品全量）** · 回合：7 · PRD 修订：1（ui_revision 1）· 范围：切片（task_ids=[T07]；AC-016、AC-017、AC-020、AC-021 + 卡片内裁定项与 T04 遗留闭合）

- QA 执行时间：2026-09-12 05:05–05:15（本地 UTC+8；日志内时间戳为 UTC 2026-09-11T21:05–21:11Z）；执行者：QA 子 agent。
- 独立性声明：`implementation.md` §T07 与 `artifacts/web-mvp/t07-rd/` 只作线索，未作为通过证据；本回合全部结论来自 QA 现场执行——自写 `artifacts/web-mvp/t07-qa/smoke-manual-qa.sh`（111 项检查，独立于 RD 脚本）、QA 自跑测试与构建、现场读源码与数据库。RD 声称的用例数、哈希、0 外呼均被 QA 独立复现（见下）。
- 派发核对：`prd.md` §9.1 修订 1（ui_revision 1，与派发包一致）；`state.yaml` qa_round=7、scope=slice、current_tasks=[T07]、open_defects 空；实现文件与 §T07-2 清单一致（新增 11 文件、修改 13 文件、未触碰 `assets*`/`0001|0002`）。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| binary | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`287c05c3a8faa02222cfc4926804035e50725439256f17504efecea338601fba`**，8 919 680 B |
| 构建复现 | QA 独立 `cargo xtask dist` **两次**（`dist-qa.log`/`dist-qa-second.log`）+ RD 两次，**四次构建同一哈希**；`smoke-bootstrap` 1+7 项全过 |
| 工具链／平台 | rustc 1.98.1（`rust-toolchain.toml` 固定）；macOS 26.6.2（aarch64）；SQLite 经 sqlx（WAL） |
| 数据与 fixture | 临时 data-dir（QA 每次自建）+ `tests/fixtures/assets/sample-manual-{text,scan}.pdf`、`sample-photo-{front.jpg,left.png}`（PDF sha256 `e18cf61a…`）；假凭据；**未配置真实 Provider** |
| 未验证 | 真实 Tripo/ManualAI（T23 授权后）；浏览器/Playwright（本切片无浏览器行为要求）；release 级"归档后已发布资料可读"（依赖 T19） |

## 命令与原始结果（QA 现场执行）

| # | 命令 | 结果（日志在 `artifacts/web-mvp/t07-qa/`） |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test items` | exit 0，**11 passed / 0 failed**（`items-qa.log`）；用例语义逐条与 §T07-8 一致，非空断言（含 UUIDv7 version==7、并发 `tokio::join!`、sqlite 行值游标翻页 25 条不重不漏） |
| 2 | `cargo test --workspace` | exit 0，**195 passed / 0 failed**（lib 94 / assets 12 / auth_api 13 / bootstrap 5 / config_cli 12 / fixture_harness 18 / **items 11** / storage 15 / core 15；另 1 条 doctest 刻意 `ignore`，`repo/items.rs` 文档示例）（`workspace-test-qa.log`） |
| 3 | `cargo xtask check` | exit 0，**7 步全 `[通过]`**（fmt/clippy/workspace 测试/前端 lint/typecheck/vitest 5/合同检查）→ `全部检查通过。`（`xtask-check-qa.log`） |
| 4 | `cargo xtask dist` ×2 + `smoke-bootstrap` | 两次哈希一致（上表）；smoke 1 项 `[准备]` + 7 项 `[检查]` 全过，启动日志 `schemaVersion: 3`（`dist-qa*.log`、`smoke-bootstrap-qa.log`） |
| 5 | **QA 独立冒烟** `bash artifacts/web-mvp/t07-qa/smoke-manual-qa.sh <binary>`（发布二进制 + 真实 data-dir + curl） | exit 0，**111/111 通过**（`smoke-manual-qa.log`）；脚本 `trap` 清理临时目录、只结束自己启动的进程 |

## AC 验收矩阵

| AC | 期望 | 实际（QA 独立复现） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| AC-016 创建 | 201 + UUIDv7 id + 整数 revision | 201；`uuid.UUID(id).version==7`；`revision=1`；`ETag: "r1"`；name/brand 去空白落库 | PASS | 冒烟 S1；`items-qa.log` |
| AC-016 校验 | 空白或超长 422 | `{}`→422（name+model 同报）；空白 name + 101 字符 brand + 201 字符 model →422 且 `details.fields`=三字段；200 字符边界通过；未知字段 422；失败创建不落行（GET 仍 1 条） | PASS | 冒烟 S2/S3；同左 |
| AC-016 列表 | `{data, nextCursor}`，默认 20／最多 100 | 顶层 `{data,nextCursor}`；21 条→首页 20 + 游标 `v1:items:active:…`、第二页 1 条 + `null`、两页 21 条不重不漏；`limit=101/0/abc`、重复参数、未知参数、坏游标、`archived=maybe` 全 422 | PASS | 冒烟 S4/S4b/S22 |
| AC-016 归档 | 归档物品不在默认列表；资料仍可读；无永久删除 | 归档后默认列表不含、`archived=true` 只含它；单条、documents、photos、资产内容（字节与 sha256 一致）全部 200；`DELETE` 到 5 条既有路径→405（`Allow: GET,HEAD,PATCH`），未知删除路径→JSON 404；"已发布资料（release）"部分见未覆盖边界 | PASS（release 级待 T19） | 冒烟 S12/S21/S13；`items-qa.log` |
| AC-017 并存 | 同品牌型号不同配置互不覆盖 | 富士/X100V 标准版+增强版两条 201、id 不同；PATCH 一条后另一条 variant 与 revision 不变 | PASS | 冒烟 S25 |
| AC-017 并发 | 后到 PATCH 旧 revision → 412 + `details.currentRevision` | 两个并行 PATCH（同 `If-Match: "r3"`）恰好一 200 一 412；loser `details.currentRevision=4`；胜者写入可见、后到者未覆盖；缺 If-Match 428、非法 `*` 422 | PASS | 冒烟 S10/S11；`items-qa.log` |
| AC-020 绑定 | 201 + `source_sha256`；跨物品被拒（404/422） | 201，`sourceSha256`=本地上传前计算的 sha256；跨物品资产 404 与"不存在资产"404 **`error.message` 逐字相同**；照片资产当 PDF→422 `fields[sourceAssetId]` | PASS | 冒烟 S14/S15/S17；`items-qa.log` |
| AC-020 0 抓取 | source_url 不触发任何服务端抓取 | sourceUrl 指向 QA 自建本机计数监听器，创建成功后 1.2 s 内 **0 连接**；URL 原样保存；`cargo tree -p everything-manual --edges normal` 生产依赖树无 HTTP client（reqwest 仅 xtask） | PASS | 冒烟 S15/S16 |
| AC-021 视图 | 只接受五值；同视图第二张规则明确 | 非法 `top`/`FRONT`/缺失/null →422 `fields[view]`；第二张 front→**422 `details.reason=viewOccupied` + view + existingPhotoId**（指认第一张 id）；失败后该视图仍 1 张；PATCH 改视图后可重新占用 | PASS | 冒烟 S18；`items-qa.log` |
| AC-021 锁 | PATCH 缺 If-Match 428、冲突 412 | 照片 PATCH 缺 If-Match 428；`r1`→`r2` 成功 + 新 ETag；再用 `r1`→412 + `currentRevision=2`；空体 422（field=body） | PASS | 冒烟 S19 |
| AC-021 detail | detail 可查询、不属于多视图集合 | GET 列表含 detail 且槽位序 `front,detail`（插入序为 detail→front，证明按槽位排序）；`repo::photos::list_multiview_for_item` 只返回 front（代码 `LIST_MULTIVIEW_SQL` + RD 用例），T12 端到端断言属 T12 | PASS（数据层） | 冒烟 S18；`items-qa.log` |

## 卡内项与 T04 遗留清单闭合核对

| 项（T04 报告"特别裁定 2"） | QA 实测 | 结论 |
| --- | --- | --- |
| 创建物品 201 + UUIDv7 + revision | 见 AC-016 创建行 | **已闭合** |
| 超长字段校验 | 201 字符 model →422；200 字符通过 | **已闭合** |
| 字段级 422 明细 | items 全部枚举/必填/空体/未知字段路径均 `details.fields`；照片 `view` 同形态 | **已闭合**（缺 assetId 例外见建议 1） |
| 归档列表过滤 | 默认排除、`archived=true` 对称过滤、恢复后回默认列表 | **已闭合** |
| PATCH 清空可选字段 | `{"brand":null}`→200 且 brand=null、`{"variant":"   "}`清空；不再"静默保留 + 递增 revision" | **已闭合** |
| 资产/资料/照片关联 | asset→document→photo 全链路 + 归档后仍可读 + 跨物品 404 不泄露 | **已闭合** |
| 分页游标在过滤条件下语义 | 游标 `v1:<scope>:…` 绑定过滤；跨条件复用→422 字段级（明确要求从头分页） | **已闭合** |
| 归档不破坏已发布资料 | documents/photos/资产内容已验；**release 不存在（T19）**，该半句留 T19 复核 | 部分（记录为未覆盖边界） |

## 裁定核对 1：PATCH 清空语义 vs contracts §1/§2

- **结论：与合同不冲突，接受 ADR-016 的裁定。** 逐条依据：contracts §1 只规定"可编辑聚合根带整数 revision、PATCH 要求 If-Match、缺 428 冲突 412、创建 201"与错误码集合，**未定义**"字段缺失 vs 显式 null"的语义；§2 items 行只写"名称／型号必填；同品牌型号不强制唯一"，同样未定义清空。PRD §5.3 乐观锁行也只写锁协议。因此 RD 的裁定是对合同未覆盖点的一次显式补充（合同未写 ≠ 合同禁止），并且修复的是 T04 QA 记录的"静默保留原值却递增 revision"这一**对客户端假成功**的行为——符合本项目"不静默"的一致原则；OpenAPI（机器合同）已把 `null` 语义写入 ItemPatchRequest/PhotoPatchRequest schema，`contracts --check` 一致。
- **QA 实测三种输入**：字段**缺失**=`{"variant":"限量版"}`→brand 保持 null、只改 variant（保持）；**显式 null**=`{"brand":null}`→清空（200，brand=null）、`{"name":null}`/`{"archived":null}`→422 字段级且 revision 不变；**空体**=`{}`→422（field=body，不空递增）。另测空白字符串=清空、超长→422、未知字段→422、失败全程 revision 保持 3。全部与 §T07-3 第 1 条逐字相符。
- 保留意见（非阻断，供 PM 一并留痕）：合同层可以补一句"可选字段显式 null/空白 = 清空；必填字段显式 null = 422"把它从 ADR 升格为合同语义，避免 T08 只读 contracts 的前端实现者产生歧义。

## 裁定核对 2：同视图第二张双重防线（服务层 + 迁移 0003 唯一索引）

- **服务层**：实测第二张 front →422 `VALIDATION_FAILED` + `details={reason:"viewOccupied", view:"front", existingPhotoId:<第一张 id>}`；PATCH 把目标改到被占视图同样 422 且 revision 不变；改选后重新占用成功。错误形态沿用 T06 `details.reason` 惯例，contracts §1 稳定码集合无独立 409 码，**未擅自新增错误码**，符合合同。
- **唯一索引**：停服后用 `sqlite3 <data-dir>/manual.sqlite3` 直接 `INSERT` 一条与既有行同 `(item_id, view)` 的记录 → **`UNIQUE constraint failed`**（绕过服务层的第二道防线真实生效）；`sqlite_master` 存在 `photos_item_view_unique`；`_sqlx_migrations` 恰 3 条。
- 代码路径核对：`UniqueViolation` 分支把索引冲突映射为**同一** 422 `reason=viewOccupied`（并发窗口兜底）。说明：真正的并发竞态（两请求同时通过占用预检查）未在运行时复现——SQLite 单写者 + 短事务使窗口极窄；此点属"结构上成立"，同 RD 声明一致，未见夸大。
- 附带确认：0003 是**只追加**迁移（未改 0001/0002），`embedded_migrations_match_repository_files` 断言内嵌 SQL 与仓库文件逐字节一致；`check` 输出"数据库 schema：已就绪（v3；WAL=wal synchronous=2 foreign_keys=ON busy_timeout=5000ms）"；`/health/ready` 200 且不含版本字样；旧库升级/未来库拒绝/幂等重开由 storage 套件 15 用例覆盖（QA 复跑通过）。

## 特别评估项：REQ-010 的"可选来源链接"无物品级落点

- **结论：属"PRD 与冻结合同之间的显式冲突"，不是 T07 编码缺陷，但也不是纯记录项——必须由 PM 做一次裁决；当前不阻断 T07 PASS。**
- 事实链：`prd.md` §3 REQ-010 的输入含"可选**来源链接**"，§6 UI-006 明确表单有"来源链接"字段；但 `contracts.md` §2 items 核心字段（name/brand/model/variant/revision/archived_at）与 §3 `GET/POST /items` 输入（"名称品牌型号配置"）**都不含**物品级 URL；`migrations/0001` items 表无 `source_url` 列。实现按冻结合同落地：`POST /items` 带 `sourceUrl` → 422 `deny_unknown_fields`（不静默忽略，行为本身正确）；出处链接由 document 的 `sourceUrl` 承载（REQ-012/UI-011，AC-020 已验）。
- 判定：CLAUDE.md 的效力顺序是"PRD → architecture/contracts"——按此，REQ-010/UI-006 的物品级来源链接是**未被任何 AC 覆盖、也未在合同中定义落点**的 PRD 输入；RD 无权单方面改合同加列，RD 已显式申报（ADR-016 第 11 条），因此不构成"隐瞒的越界"。但若 PM 不做裁决，T08 实现 UI-006 时将出现"表单字段被 API 422"的确定性冲突。
- **建议（交协调者批量提交 PM）**：二选一——(A) 修订合同：items 新增 `source_url`（迁移 0004 + 校验 ≤2000 字符 + DTO/OpenAPI 重生成 + PATCH 清空语义与 brand/variant 对齐），需明确与 document.sourceUrl 的分工；(B) 修订 PRD（推荐，改动最小且与现有 AC/实现一致）：REQ-010 输入列表与 UI-006 去掉物品级"来源链接"，改为指向"绑定说明书时可填写出处链接"（UI-011）。裁决截止点建议不晚于 T08 派发。

## 路由范围裁定：T07 新增读取侧 GET documents/photos

- **结论：属对 contracts §3 的"读取侧扩展"，非破坏性、非未授权扩张；建议协调者补记（不要求回滚）。** 依据：contracts §3 是"核心路由清单"，其注记只禁止"永久删除或任意磁盘浏览 API"，本案新增的是**会话保护**（实测无 cookie →401）、按所属物品读取的集合路由，没有任何删除/写能力；不补记则 AC-016"归档后资料仍可读"与 UI-011/UI-012（T08）将失去可验证性与消费面。OpenAPI（机器合同）已包含并 `contracts --check` 一致（含新增 6 条路径）。**建议落点**：在 contracts §3 补两行（GET /items/{id}/documents、GET /items/{id}/photos[/{photoId}]）或加一句"路由清单非封闭，读取侧集合可随卡扩展并须在 OpenAPI 申报"；T08 必须以 `apps/web/src/api/generated.ts` 为准。

## 缺陷

**无。** 本回合 0 个 OPEN/BUG；AC-016/017/020/021 与全部卡内必测项通过，T01–T06 回归通过（195 用例 + 7 步门禁 + dist/smoke）。RD 报告中的用例数、哈希、0 外呼、405/404 语义、清空语义均被 QA 独立复现。

### 非阻断建议（不计入阻断项，不要求 T07 返工）

1. **缺 `assetId`/`sourceAssetId` 的 422 是提取器级（`details=null`）而非字段级**（P3，T08 渲染注意）：实测 `POST /items/{id}/photos {"view":"top"}`（缺 assetId）→422，message 为英文 serde 文案"请求体不符合接口结构：Failed to deserialize … missing field `assetId`"，`details=null`；而 items 创建/照片 view 是 `details.fields` 字段级形态（ADR-016 第 4 条）。不违反任何 AC（AC-020/021 未断言该形态），但前端"字段级错误并聚焦"的通用渲染（UI-006/UI-012）会漏掉这一分支。建议后续把 DTO 必填字段改为 `Option` + 校验层报错（与 items 同法），或由 T08 保留通用兜底文案。
2. **归档过滤参数名是 `archived`（非 T04 报告设想的 `includeArchived`）**：T08/UI-005 实现"显示已归档"开关时按生成类型（`archived=true`）走；该名已写入 OpenAPI，无兼容负担（T08 未开工）。
3. **`GET /items/{id}/photos` 拒绝任何查询参数（含 `limit`）且 `nextCursor` 恒 null**：与列表"默认 20／最多 100"的通用约定不同，但集合被视图唯一性上界为 5 条，属合理取舍且已文档化；T08 不得给该请求带分页参数（传了会 422，行为已在 OpenAPI 描述）。

## 未覆盖边界（本回合记录，不冒充通过）

1. **AC-016 的 release 级"已发布资料仍可读"**：release 尚不存在（T19），本回合以 documents/photos/资产内容归档后可读等价验证；T19 发布首个 release 后必须补"归档物品 → release 仍可打开/导出"的断言。
2. **detail 不进入 Tripo 多视图请求体**的端到端断言属 T12（本卡只验数据层集合语义，符合派发口径）。
3. UI 侧（UI-005/006/007/008/011/012）与 Playwright 属 T08/T21；本切片无浏览器行为要求，不构成 BLOCKED。
4. 真实 Provider／费用（T23 授权后）；Linux musl 与正式 smoke（T22）。
5. 多进程/多实例并发未测（单管理员单进程模型）；唯一索引的运行时竞态未复现（见裁定 2 说明）。
6. **T11 前置提示（结构性）**：photos 行按 id 可变（PATCH 换资产会使同 id 的照片内容变化），因此快照必须按合同保存 `photo_ids+hashes`（§2 generation_snapshots），不能只存 id；本卡实测"改名/归档不改变 document/photo 引用与 source_sha256"仅为冻结快照的结构性前提。
7. 归档物品仍可编辑（ADR-016 第 7 条）：若 PM 决定归档后只读，需要新裁定。

## 非代码知识与限制（跨卡复用）

1. **发布二进制可复现的强判据**：QA 侧两次 + RD 侧两次 `cargo xtask dist` 得同一 sha256（`287c05c3…`）——后续卡可沿用"QA 重建比对哈希"作为"产物与源码一致"的入口之一。
2. **DB 级不变量的"绕过服务层"验证法**：停服后 `sqlite3` 直插冲突行验证唯一索引兜底（本卡 `photos_item_view_unique`）；T09/T11/T19 的页号唯一、stage 唯一、release 不可变可用同一手法。
3. **0 外呼的独立构造**：python 计数监听器 + `cargo tree --edges normal` 生产依赖树核对（reqwest 仅存在于 xtask）；T12/T14 引入真客户端后该判据必须换成 fixture 计数（T05 知识 1 的失效条件已触发）。
4. **响应无磁盘路径的检查法**：以 QA 临时目录绝对路径为模式 grep 全部响应落盘文件，比逐条断言覆盖面大。
5. **自写 curl 冒烟的三个陷阱（QA 侧复现）**：`-w '%{http_code}'` 无换行→多文件拼接粘连（加 `\n`）；包装函数参数错位时 curl 会多 URL 输出（症状 `000000{json}200`，首版脚本踩坑）；想测"字段级 422"必须带齐必填字段，否则命中提取器级通用 422。
6. 以上 1–5 与裁定结论已追加到 `llmdoc/decisions.md`「QA 验收知识 · T07 验收知识」。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 7 | 2026-09-12 | slice | T07（AC-016、AC-017、AC-020、AC-021） | 1（ui_revision 1） | PASS | 本文件 |

- 交接给 RD：本回合无缺陷需返工；非阻断建议 1（缺必填字段的字段级形态）建议与 T08/T09 排期一起做。
- 交接给协调者：（a）`accepted_tasks` 追加 `{task_id: T07, prd_revision: 1, qa_round: 7, report: llmdoc/requirements/web-mvp/qa-report.md, accepted_ac_ids: [AC-016, AC-017, AC-020, AC-021], status: accepted}`（AC-016 的 release 级半句备注"待 T19 复核"）；（b）`qa_history` 追加本回合（result: PASS）；（c）**PM 决策项新增/合并**：REQ-010 物品级"来源链接"落点（见特别评估项，建议选项 B，截止 T08 派发前）；PATCH 清空语义是否升格写入 contracts §1（建议写）；（d）**补记项**：contracts §3 补记两条读取侧 GET（或明确清单非封闭）；implementation-plan T07 允许范围补记 `http/{documents,photos,pagination,body}.rs` 与 `storage/repo/{documents,photos}.rs`（§T07-2 已列，合同清单未列）；（e）T11 必须按快照 `photo_ids+hashes` 冻结（照片行可变，见未覆盖边界 6）；（f) T04 遗留清单除 release 半句外全部闭合。
- 证据目录：`artifacts/web-mvp/t07-qa/`（8 文件）：`smoke-manual-qa.sh`（QA 自写，111 项）、`smoke-manual-qa.log`、`items-qa.log`、`workspace-test-qa.log`、`xtask-check-qa.log`、`dist-qa.log` + `dist-qa-second.log`、`smoke-bootstrap-qa.log`。

---

# 回合 8 · T08

**结果：PASS（仅 T08 切片范围，不代表产品全量）** · 回合：8 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T08]；AC-003/AC-004 前端侧、AC-016/AC-017 的前端消费侧、AC-060 的前端侧 + §6.2 UI-001/002/003/005/006/008 + §6.1 全局框架 + §6.3.2 禁用措辞）

- QA 执行时间：2026-09-12 05:40–07:2x（本地 UTC+8；日志内时间戳为 UTC 2026-09-11T21:4x–23:0xZ）；执行者：QA 子 agent。
- 独立性声明：`implementation.md` §T08 与 `artifacts/web-mvp/t08-rd/`（含其脚本、截图、浏览器日志）只作线索，**未用作通过证据**。本回合全部结论来自 QA 现场执行：QA 自写 CDP 走查脚本 `artifacts/web-mvp/t08-qa/walkthrough-qa.mjs` + 运行脚本 `run-walkthrough-qa.sh`（与 RD 的 `browser-walkthrough.mjs` 无共享代码，未读其脚本内容）、QA 自跑全部命令与两次 `dist` 构建、QA 自写 Node 侧第二客户端对照实验。**未引用 RD 截图**；QA 截图 `01`–`09`。
- 派发核对：`prd.md` §9.1 修订 2（ui_revision 2）与派发包一致；`state.yaml` qa_round=8、scope=slice、current_tasks=[T08]、open_defects 空；`apps/web/src/` 34 个源文件与 §T08-2 清单逐条一致；`contracts/openapi.json`、`apps/web/src/api/generated.ts`（mtime 05:00，T07 时点）未被 T08 触碰，`cargo xtask contracts --check` 一致；`crates/**`、`migrations/**` 未改动。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD 仍为 `cb69afb`（"init"），T01–T08 交付物**尚未提交**；`git status` 与 T07 回合一致（仅 `.gitignore`、`llmdoc/contracts.md`、`llmdoc/decisions.md`、`state.yaml` 为已跟踪修改）；QA 用 sha256 清单前后比对确认本轮命令未改动源码／合同（唯一变化 `dist/…/build-info.json`，来自 QA 重建） |
| 平台 | macOS Darwin 25.6.0 · aarch64-apple-darwin |
| 工具链／浏览器 | rustc/cargo 1.98.1（`rust-toolchain.toml`）；node v26.0.0；npm 11.12.1；Google Chrome **152.0.7977.84**（headless=new，QA 自写 CDP 客户端驱动） |
| 前端依赖 | react/react-dom 19.3.0、react-router 7.18.3、@tanstack/react-query 5.102.8（package.json 与 package-lock 一致，`npm ci` 语义未改动锁文件） |
| 交付二进制 | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`668e9958d6fdad9006d1baf2d9607817dd30c832435af4f02fd050e07f680491`**，9 051 776 B（QA 独立重建两次 + RD 两次，**四次同哈希**） |
| 数据／fixture | 临时 data-dir（QA 每轮自建 + `init --password-file` 0600）；**未配置真实 Provider**（预期显示「未配置」）；无付费调用 |
| 浏览器走查方式 | **发布二进制自带内嵌 UI**（同源 `http://127.0.0.1:8099`，SPA fallback 生效）+ 真实 Chrome + 临时 data-dir（比 RD 的 Vite dev 少一层代理变量，且覆盖生产同源语义） |
| 未验证 | 真实 Tripo/ManualAI（T23 授权后）；3D/PDF/校准/任务中心/阅读器（T09/T17/T18/T19 切片）；触屏与移动浏览器；Safari；Vite dev 代理模式 |

## 命令与原始结果（QA 现场执行，2026-09-12；日志在 `artifacts/web-mvp/t08-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `npm --prefix apps/web run test -- --run src/features/shell/shell.test.tsx` | exit 0，**1 file / 19 passed / 0 failed**（`shell-test-qa.log`） |
| 2 | `npm --prefix apps/web run typecheck` | exit 0（`typecheck-qa.log`） |
| 3 | `npm --prefix apps/web run lint` | exit 0（`lint-qa.log`） |
| 4 | `npm --prefix apps/web run test -- --run` | exit 0，**3 files / 30 passed**（`web-test-qa.log`） |
| 5 | `npm --prefix apps/web run build` | exit 0；`dist/index.html` + `assets/index-QoC9xBFM.css`(7.55 kB) + `assets/index-B6p0J4kW.js`(343.00 kB)（`build-qa.log`） |
| 6 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`；工作树未被修改（`contracts-check.log`、`manifest-before/after.txt`） |
| 7 | `cargo test --workspace` | exit 0，**195 passed / 0 failed / 1 ignored**（`workspace-test-qa.log`） |
| 8 | `cargo xtask check` | exit 0，**7 步全 `[通过]`**（fmt/clippy/workspace/npm lint/typecheck/test/contracts）（`xtask-check-qa.log`） |
| 9 | `cargo xtask dist --target aarch64-apple-darwin` **×2** | 均 exit 0，两次哈希一致（上表）→ 与源码一致的可复现产物（`dist-qa.log`、`dist-qa-second.log`、`dist-hash-1/2.txt`） |
| 10 | `cargo xtask smoke-bootstrap --binary $PWD/dist/aarch64-apple-darwin/everything-manual` | exit 0；1 项 `[准备]`（init）+ **7 项 `[检查]`**全过（页面/JS/health live+ready/`/api/unknown` JSON404/SPA 深链接/缺失资源 404 非 HTML）（`smoke-bootstrap-qa.log`） |
| 11 | `bash artifacts/web-mvp/t08-qa/run-walkthrough-qa.sh`（QA 自写） | **exit 0，78 passed / 0 failed**；日志另含 1 条 `[DEFECT-P3] BUG-001`（该缺陷按设计不计入 PASS/FAIL 计数）（`walkthrough-final.log`，真实 Chrome 152 + 发布二进制内嵌 UI + 临时 data-dir；截图 `01`–`09`） |

## AC 验收矩阵（本切片派发范围）

| AC | 期望（前端消费侧） | 实际（QA 独立复现） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| AC-003 | 401 结构 / 登录 cookie 属性 / 登出旧 cookie 失效 / session no-store | 未登录 `GET /auth/session` 与业务请求均 401；正确密码登录 200 + `Set-Cookie: em_session`（HttpOnly、SameSite=Strict、JS `document.cookie` 不可读）；登出 204 → 用旧 cookie 的 Node 再请求 `GET /items` **401**；已登录 `GET /auth/session` 200 + `Cache-Control: no-store` 且 `fromDiskCache=false` | PASS | 走查步骤 1/4/14；`walkthrough-final.log` |
| AC-004 | 403（Origin/CSRF）/429/428/412 的界面与客户端处理 | 跨站 `Origin` 登录请求 403（服务端拒绝）；连续错误密码序列 `401,401,401,401,401,429`（`Retry-After=60`）且限速窗口内页面显示「尝试过于频繁，请稍后再试」（`role=alert`）；编辑冲突 **412** 进入 UI-008 恢复路径；428 由客户端先取 ETag 补齐（正常路径不产生，单测覆盖） | PASS | 走查步骤 3/8/15 |
| AC-016 | 创建 201、列表 `{data,nextCursor}`、归档可见性（前端消费） | 表单 `POST /items` 201 → 跳物品概览显示 `r1`；列表行显示名称/型号/状态/更新时间；归档后默认列表隐藏、开关切出可见、可恢复（服务端语义见回合 7） | PASS（前端消费侧） | 走查步骤 7 |
| AC-017 | 412 后 `details.currentRevision` 可见且不覆盖（前端消费） | 外部客户端把 revision 推到 r4 后，浏览器用旧 ETag 提交 → 412；页面显示「该内容已被其他操作更新（当前 r4）」、输入保留、提交禁用、「刷新后重试」→ 用 r4 ETag 重新提交 200（服务端语义见回合 7） | PASS（前端消费侧） | 走查步骤 8；`05-conflict-412.png` |
| AC-060（前端侧） | 断点布局 / 键盘可达 / 减少动效 | 1400/900/600px 三种布局（并排侧栏 / 可折叠单侧栏 / 单栏+抽屉）；抽屉 `aria-modal` 焦点陷阱 + Esc 归还焦点；Tab 顺序 顶栏→主栏；`prefers-reduced-motion: reduce` 骨架与过渡 ≤1ms（默认 1.4s） | PASS（前端侧；read/PDF/校准页属 T18/T19） | 走查步骤 5/11/12 |

矩阵外说明：AC-001/002/005–015、018–059、061–066 分别由已验收切片或后续切片（T09–T23）承载，本回合不作为缺陷。

## 卡内必验项（T08 卡 + UI 条目）

| 卡内要求 | 结果 | 证据 |
| --- | --- | --- |
| `npm --prefix apps/web run test -- --run src/features/shell/shell.test.tsx` | 19 passed / 0 failed（非 0 测试） | 命令 1 |
| `npm --prefix apps/web run typecheck` | exit 0 | 命令 2 |
| **键盘登录** | PASS：body 起 Tab → 焦点 `#field-password` → 输入 → Tab → 焦点「登录」按钮（空密码时 disabled 且 Tab 跳过）→ **Enter 提交成功** | 走查步骤 2 |
| **错误信息关联表单** | PASS：登录失败 `role=alert` 摘要获得焦点、密码框 `aria-describedby=field-password-error` + `aria-invalid=true`；物品表单超长型号 422 → 字段级错误 + 摘要 + **焦点移到 `#field-model`**（UI-065） | 走查步骤 3/7；`03-login-error-focus.png` |
| **412 可恢复** | PASS：`details.currentRevision` 可见、不自动覆盖、不丢输入、刷新后重试成功 | 走查步骤 8 |
| **不泄露 token** | PASS：64 字符 CSRF token 不在 DOM innerHTML / localStorage / sessionStorage / `document.cookie` / 控制台 / 浏览器日志；表单无残留值；会话 cookie HttpOnly | 走查步骤 4/16 |
| UI-001 登录表单 | PASS：`type=password` + `autocomplete=current-password` + 无默认值 + 空密码禁用；401/429 文案实测（403 文案见未覆盖边界 1） | 走查步骤 1/3/15 |
| UI-002 会话恢复与 next | PASS：恢复中全屏骨架不闪登录页；401 → `/login?next=<站内相对路径>`；`next` 只接受同源相对路径（`//evil.com`、`https://evil.com` 均回落 `/`，`/settings` 正常返回） | 走查步骤 1/2/14 |
| UI-003 设置页 | PASS：`/settings/status` + `/health/live` + `/health/ready` 只读展示；未配置项标「未配置」并提示「生成与报价不可用；已有资料仍可读」；limits 数值与接口逐项一致；**页面 0 个 input/select/textarea/form**、无证书/TLS/监听配置入口、无密钥样式字符串；接口响应字段仅 providersConfigured/limits/capabilities | 走查步骤 6；`02-settings-page.png` |
| UI-005/UI-006 资料库与物品表单 | PASS：库列表（行式、状态标签、更新时间）、空态、归档开关、行内入口；表单字段恰为 名称/准确型号/品牌/变体配置，**无物品级来源链接字段**且页面说明「物品不保存来源链接」（ADR-017 D-1） | 走查步骤 7 |
| UI-008 通用 412 | PASS（同 AC-017 行） | 走查步骤 8 |
| §6.1.1 三断点 / 深链接刷新 / 顶栏 / 错误边界 | PASS：三断点见 AC-060 行；`/items/{id}`、`/settings` 直达与 `Page.reload` 正常；顶栏含产品名·当前物品名·型号、任务中心、设置、登出；错误边界显示「页面出现异常」+ **注入响应的 `x-request-id`** + 返回资料库，无 `.tsx`/堆栈 | 走查步骤 4/10/11/13；`08-error-boundary.png` |
| §6.3.2 禁用措辞清单 | PASS：累计 15 份页面文本（含 9 条占位路由页）× 8 条禁用措辞 0 命中（设置页"部署边界"文案未使用「离线可用」等） | 走查步骤 16 |
| 占位路由不冒称完成 | PASS：9 条占位路由均显示「该页面尚未实现。」、无业务请求、无写请求、无假计数；向导占位页保留五步步骤条与 `aria-current="step"` | 走查步骤 9 |
| API 客户端用生成类型 | PASS（代码核对）：`endpoints.ts` 全部响应/请求体类型经 `operations[...]`/`components[...]` 派生；无手写 DTO（`ItemPageRequest` 仅是查询参数容器，不含实体字段）；`contracts --check` 一致 | 命令 6 + 源码核对 |
| CSRF / If-Match 真实生效 | PASS：Fetch 域 request 阶段实测浏览器 `POST /items`、两条 `PATCH` 的 `x-csrf-token`/`if-match`/`Origin` 头；Node 侧对照：缺 token 403、带 token 201 | 走查步骤 7/8 |

## 浏览器走查（QA 独立执行；`walkthrough-final.log` + 截图）

### next 安全性（构造输入）
- `/login?next=%2F%2Fevil.com` → 登录后落 `/`；`/login?next=https%3A%2F%2Fevil.example%2Fsteal` → 落 `/` 且页面无 `evil.example`；`/login?next=%2Fsettings` → 登录后确实回 `/settings`。
- 未登录深链 `/settings` → `/login?next=%2Fsettings`；用 `Network.emulateNetworkConditions{latency:1200}` 放大观测窗确认**恢复中只有全屏骨架、无密码框**（不闪登录页）。
- 源码核对：`safeNextPath` 拒绝非 `/` 开头、`//`、`/\`、含 `://`、含 `\`、控制字符与空值（单测 10 组）。

### CSRF / If-Match
- 见「卡内必验项」表末行；补充：GET 请求不携带 CSRF；`Origin` 为同源 `http://127.0.0.1:8099`。
- 注意（跨卡复用）：CDP `Network.requestWillBeSent` **不上报 `x-csrf-token`**，必须用 `Fetch.requestPaused` 才能拿到完整头，否则会产生"应用没发 CSRF"的假阴性（见非代码知识 1）。

### 占位路由
- 9 条路由（向导 2–5 步、`/jobs`、`/jobs/:id`、`review`、`releases`、`releases/:id`）逐条显示「该页面尚未实现。」；网络记录仅 `GET /auth/session`、物品上下文 `GET /items/{id}`、`/health/*`；无 `/jobs`、`/estimates`、`/preparations` 等请求，无任何 POST/PATCH/PUT；任务中心不显示编造计数；`06-wizard-placeholder.png`。

### 全局框架与可访问性
- 断点、Tab 顺序、抽屉焦点陷阱/Esc、错误边界 requestId、减少动效、深链接刷新：见「卡内必验项」表；`07-narrow-drawer.png`、`08-error-boundary.png`。
- 错误边界用 `Fetch.fulfillRequest` 注入合同违约载荷（`documents[0].sourceSha256=null`）+ 自定义 `x-request-id`，页面显示的 requestId 与注入头逐字一致。

### 凭据不泄露
- CSRF token（64 字符）在 DOM、localStorage、sessionStorage、`document.cookie`、控制台、浏览器日志中均无命中；登录后输入框无残留；`em_session` cookie `httpOnly=true`、`sameSite=Strict`（HTTP 下 `secure=false`）。

## 缺陷

### BUG-001 · 成功通知条（固定顶部条）盖住顶栏，指针点击被吞掉

- 严重度／状态：**P3 / OPEN**；**非阻断**（判定依据：未违反本切片任何必选 AC/UI 条目——PRD §6.3.3 U-07 明确"成功提示用顶部条"；覆盖层不夺取焦点，实测通知条存在时 **Tab+Enter 仍可完成顶栏导航**；影响窗口 ≤6 s 且提供「关闭」按钮；无数据损失、无费用/安全影响）。
- 对应 REQ / UI / AC：PRD §6.1.1（顶栏为所有页面共用的产品名/任务中心/设置/登出入口）、U-07（成功提示顶部条）；**无对应 AC**。
- 环境与输入：发布二进制内嵌 UI + Chrome 152；QA 临时 data-dir；登录后执行"新建物品"。
- 复现步骤：① 新建物品（触发 `notify("已创建物品「…」")`）；② 在其后 6 s 内用指针点击顶栏「资料库」。
- 期望与实际：期望点击顶栏入口完成导航；实际点击命中通知条（`elementFromPoint` 返回 `.notices` 内的 DIV），导航不发生；同一时刻键盘 Tab+Enter 导航成功。
- 证据：`artifacts/web-mvp/t08-qa/walkthrough-final.log` 的 `[DEFECT-P3]` 行（实测 `noticeHeight=61`、顶栏链接 `linkCenterY=41`、`hitInNotice=true`、`hitIsLink=false`）与 `指针点击顶栏「资料库」被通知条吞掉`、`[PASS] BUG-001 补充：…键盘（Tab+Enter）路径仍可完成导航` 两行；根因见 CSS `apps/web/src/styles.css` 的 `.notices{position:fixed;top:0;left:0;right:0;z-index:30}`。
- 建议修复方向（不改验收口径）：通知条容器不拦截其内容之外的指针事件（`.notices{pointer-events:none}` + `.notice{pointer-events:auto}`），或把通知条放到顶栏之下/撑开布局。
- 回归范围：任何触发通知条的操作（新建/保存/归档/恢复/登出失败）之后，顶栏各入口的指针可用性（宽屏 + 窄屏）。
- RD 修复摘要引用：待 RD 修复；修复后由 QA 复验 CLOSED。

## 非阻断建议（不计入阻断项，不要求 T08 返工）

1. **P3｜`GET /auth/session` 的 401 响应不带 `Cache-Control: no-store`**（只有登录/会话 200 走 `json_no_store`）。未违反 AC-003（该 AC 针对带凭据的会话响应，前端 fetch 亦显式 `cache:"no-store"`、实测 `fromDiskCache=false`）；但 T21 若把安全回归写成"任意状态码都带 no-store"会失败，建议按 200 断言或给 401 补该头。
2. **P3｜通知条与顶栏的覆盖关系属全局视觉约定**，T17 会大量使用通知；建议在修 BUG-001 时一并定义"通知不得遮挡主入口"的规则。
3. **P3｜物品表单不做客户端长度预校验**（RD §T08-6 第 8 条声明）：超长由服务端 422 + `details.fields` 报错并聚焦字段，行为可接受、无假成功；若需减少一次往返，T16 可加提示（不得手抄服务端常量）。
4. **P3｜搜索是"已加载行内筛选"**（RD §T08-6 第 1 条）：页面已如实标注范围（"在当前已加载的 N 条中筛选…"）；服务端检索需合同新增参数，建议 T16 决策。
5. **P3｜中段侧栏标签页无 ←/→ 键盘导航**（RD §T08-6 第 10 条）：T08 只用到单面板，未构成可访问性缺口；T18/T19 接入三栏时须按 ARIA Tabs 模式补齐或改回普通区块标题。

## 未覆盖边界（本回合记录，不冒充通过）

1. **403 页面文案**（CSRF/Origin 失败 →「请刷新页面后重试」）：服务端 403 已实测（跨站 Origin 登录），但真实浏览器中应用自身不会发出跨站 Origin 请求，无法在浏览器端复现；文案由单元用例 `429 与 403 使用限速/刷新文案` 覆盖。
2. **428（缺 If-Match）**：客户端提交前用 GET 的 ETag 补齐 If-Match，正常路径不产生 428；仅单元用例覆盖（`effectiveEtag===null` 时进入 UI-008 路径）。
3. **错误边界的真实违约触发**：本回合用响应注入构造；未观察到真实生产违约响应。
4. 中段（768–1279）三栏与窄屏校准禁用等**真实页面**属 T18/T19；`/review`、`/releases`、`/jobs` 本回合只有占位页。
5. 触屏/移动浏览器与 Safari 未实测（AC-060 只承诺视口宽度行为；REQ-041 浏览器矩阵属 T21）。
6. 顶栏任务计数徽标、右栏"最近使用/进行中任务"摘要按"不显示假数字"核对通过，实际数据属 T15/T17。
7. **走查稳定性**：本回合观察到 Chrome/CDP 偶发卡在 `Runtime.evaluate`（服务端日志正常、应用无异常；重建页面目标即恢复）。已写入 QA 脚本并记录到非代码知识 7；不作为产品缺陷依据，但后续卡的浏览器回归需预留该恢复逻辑，避免误判为应用卡死。
8. 真实 Provider、PDF/3D 链路、正式 `xtask smoke`（T22）不在本切片。

## 非代码知识与限制（跨卡复用）

1. **CDP 头观测陷阱（重要）**：Chrome 152 的 `Network.requestWillBeSent.request.headers` **不上报 `x-csrf-token`**（同一请求的 `if-match`、`origin`、`content-type` 都在），必须用 `Fetch.enable{requestStage:"Request"}` + `Fetch.requestPaused` 取实际头，否则 T16/T17/T19 的 CSRF/Idempotency-Key 断言会出现假阴性。
2. **CSRF token 每次登录都换**：断言须从最近一次登录响应体重新取值（本回合踩坑：沿用旧 token 导致假阴性）。
3. **"点击被别的元素吞掉"的证明手法**：`document.elementFromPoint(控件中心)` 返回的是什么元素，比"点了没反应"强得多（`pointer-events`/覆盖层/模态问题通用）。
4. **登录限速的稳妥验法**：先用 Node 侧第二客户端打满限速窗口取 429 + `Retry-After`，再做一次浏览器登录观测页面文案；连续在浏览器里试错易触发 CDP 卡顿。
5. **浏览器走查直接用发布二进制内嵌 UI**（同源 + SPA fallback + cookie/Origin 与生产一致），比 Vite dev + 代理少一层变量；`cargo xtask dist` 保证内嵌 UI 在场。
6. **加载态观测窗可用 `Network.emulateNetworkConditions{latency}` 放大**（本回合 1.2 s 稳定验证"恢复中不闪登录页"）。
7. **`Fetch.fulfillRequest` 注入合同违约载荷**是验证前端错误边界与 requestId 来源的可复用手法；**CDP `Runtime.evaluate` 偶发 30s 无响应时重建页面目标（`Target.closeTarget`+`createTarget`+重新 attach）可恢复**。
8. 以上 1–7 与 BUG-001 根因已追加到 `llmdoc/decisions.md`「QA 验收知识 · T08 验收知识」。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 8 | 2026-09-12 | slice | T08（AC-003/AC-004 前端侧、AC-016/AC-017 前端消费侧、AC-060 前端侧） | 2（ui_revision 2） | PASS | 本文件 |

- 交接给 RD：（a）**BUG-001（P3，OPEN，非阻断）**建议随 T09/T16 一并修复：`.notices` 改为不拦截内容以外指针事件或下移；修复后由 QA 复验 CLOSED；（b）非阻断建议 1（401 的 no-store）若修，请同步 T21 安全回归的断言口径；（c）T09 起浏览器断言头请用 `Fetch` 请求阶段拦截（非代码知识 1）。
- 交接给协调者：（a）`accepted_tasks` 追加 `{task_id: T08, prd_revision: 2, qa_round: 8, report: llmdoc/requirements/web-mvp/qa-report.md, accepted_ac_ids: [AC-003, AC-004, "AC-016（前端消费侧）", "AC-017（前端消费侧）", "AC-060（前端侧）"], status: accepted}`；（b）`qa_history` 追加本回合（result: PASS，scope: slice）；（c）`open_defects` 登记 **BUG-001（P3，OPEN，非阻断）**——如需严格按"未关闭缺陷不得 PASS"处理，请把它标为"非阻断/记录在案"，本回合判定依据见缺陷段；（d）T08 已建立的路由骨架/占位策略是 T09/T16/T17/T18/T19 的接入点（占位页须在对应卡删除并替换为真实实现）。
- 建议 state 更新：`phase` 按流程推进到下一切片（T09 派发）；`prd_revision: 2`、`ui_revision: 2` 保持；`qa_round: 9`；`accepted_tasks` 追加 T08 条目（如上）；`qa_history` 追加回合 8（PASS）；`open_defects: [BUG-001（P3，非阻断）]`；`next_action`：派发 T09（或先安排 BUG-001 的 P3 修复，由 QA 复验）。
- 证据目录：`artifacts/web-mvp/t08-qa/`：QA 自写 `run-walkthrough-qa.sh`、`walkthrough-qa.mjs`（**78 项检查**）、`walkthrough-final.log`（参考证据：78 passed / 0 failed / exit 0）与 `walkthrough-run1..9.log`（迭代留痕）、`probe-reload.mjs`（CDP 卡顿诊断）、截图 `01-login-next-settings.png`、`02-settings-page.png`、`03-login-error-focus.png`、`04-item-overview.png`、`05-conflict-412.png`、`06-wizard-placeholder.png`、`07-narrow-drawer.png`、`08-error-boundary.png`、`09-rate-limited.png`、日志 `shell-test-qa.log`、`typecheck-qa.log`、`lint-qa.log`、`web-test-qa.log`、`build-qa.log`、`contracts-check.log`、`workspace-test-qa.log`、`xtask-check-qa.log`、`dist-qa.log`、`dist-qa-second.log`、`dist-hash-1.txt`、`dist-hash-2.txt`、`smoke-bootstrap-qa.log`、`server-qa.log`、`manifest-before.txt`、`manifest-after.txt`。

# 回合 9 · T09（含 BUG-002（原 BUG-001-r8）复验）

**结果：PASS（仅 T09 切片范围，不代表产品全量）** · 回合：9 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T09]；AC-022/AC-023/AC-024/AC-025 + §6.2 UI-014–UI-018 + §6.3.2 禁用措辞 + BUG-002 复验 + 回归）

- QA 执行时间：2026-09-12 08:05–08:2x（本地 UTC+8；命令日志时间戳为 UTC 2026-09-12T00:0xZ）；执行者：QA 子 agent。
- 独立性声明：`implementation.md` §T09 与 `artifacts/web-mvp/t09-rd/`（脚本、截图、日志）**只作线索，未用作通过证据**。本回合结论来自 QA 现场执行：QA 自跑 `cargo test -p everything-manual --test preparations`、RD 的 AC 合同命令 `npm --prefix apps/web run test:e2e -- pdf-preparation.spec.ts`（只执行、不采信其断言语义），并**另写独立 spec** `apps/web/tests/e2e/qa-t09-independent.spec.ts`（6 用例，自己的断言、自己的造数与记录方式、不 import RD 的 `helpers.ts`/断言）复现"真实关闭标签页后续传只补缺页"与"断外网完成准备"；用 `sqlite3` 直查 SQLite 交叉验证后端事实；用系统 PDFKit（swiftc）独立验证负例 fixture；用 QA 自写脚本复核 release 二进制内嵌 vendor。**未引用 RD 截图**。
- 派发核对：`prd.md` §9.1 修订 2（ui_revision 2）与派发包一致（AC-022–025 为 [必选]，命令合同见 §4）；`state.yaml` qa_round=9、qa_scope=slice、current_tasks=[T09]、open_defects=[BUG-001-r8 fixed_pending_reverify]；`implementation.md` §T09 状态 RD_READY；RD §T09-2 文件清单**逐项在场**（Rust：迁移 0004、`core/domain.rs` 的 `client_derived`/`PageViewport`、`validation.rs` 上限与校验、`storage/error.rs` 的 `NotWritable`、`storage/repo/preparations.rs`、`http/preparations.rs`、DTO、`tests/preparations.rs`；web：`vite.pdfjs-vendor.ts`、`playwright.config.ts`、`tests/e2e/**`、`features/import/**`、`App.tsx` 懒加载路由、`notifications.tsx` + `styles.css`；fixture 4 个新 PDF 与 `fixture_harness.rs` 断言）。
- **缺陷编号重编说明**：派发包要求把回合 8 的 `BUG-001-r8`（通知条覆盖顶栏，P3）在本报告改称 **BUG-002**，原因是该编号与回合 5/6 已修复并关闭的限速单测 flake `BUG-001`（见回合 5/6/ADR-015 结论 9）冲突，旧的 `open_defects` 记录也因此有歧义。本报告此后只用 **BUG-002** 指代"通知条遮挡顶栏"缺陷；`llmdoc` 中 ADR-019 与 T09 实现记录里的 `BUG-001-r8` 字样保留为历史痕迹，语义不变。**无其它编号受影响**（回合 6 的 limiter BUG-001 已 CLOSED，不重开）。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD 仍为 `cb69afb`（"init"），T01–T09 交付物**尚未提交**；QA 用 177 文件 sha256 清单前后比对（`source-manifest-before/after.txt`）：唯一新增为本回合 QA 自写 spec，`crates/**`、`migrations/**`、`contracts/**`、`apps/web/src/**`、`apps/web/vite.pdfjs-vendor.ts` 等**零改动** |
| 平台／工具链 | macOS Darwin 25.6.0 · aarch64-apple-darwin；rustc/cargo 1.98.1；node v26.0.0；npm 11.12.1；`@playwright/test` 1.60.0（Chromium 1223 / Chrome for Testing **148.0.7778.96**，headless shell）；`pdfjs-dist 6.3.289`（package-lock 锁定） |
| 交付产物 | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`44ab49e509a16a2dc4942b32fe6ed8481f216ae096e0903aca876448c8a97d74`**，14 617 632 B（QA 独立重建，与 RD §T09-9 第 12 行**同哈希** → 确定性构建） |
| 数据／fixture | e2e 自建临时 data-dir（`/tmp/em-web-mvp-e2e`，`init --password-file` 0600；验后由 QA 清理）；样例为 T05 自建 fixture（QA 用系统 PDFKit 独立验证，见下）；**未配置真实 Provider、无付费调用** |
| 浏览器走查拓扑 | Playwright 自管：`globalSetup` 真实 Rust **debug** 后端（127.0.0.1:18080）+ Vite dev（15173，`/api` 代理）；发布二进制侧由 `xtask dist` + `smoke-bootstrap` + QA 自写内嵌 vendor 脚本覆盖（见"发布面"行） |
| 未验证 | 真实 Tripo/ManualAI（T23）；100 页规模的耗时/内存（T21，AC-062 度量）；真实 `beforeunload` 弹窗（需用户激活，T21 人工）；触屏/移动浏览器与 Safari（REQ-041→T21）；发布二进制内嵌 UI 上跑同一 e2e spec（T21/T22）；WASM/ICC 解码代码路径（样例不需要 JPX/JBIG2/ICC） |

## 命令与原始结果（QA 现场执行，2026-09-12；日志在 `artifacts/web-mvp/t09-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test preparations` | exit 0，**9 passed / 0 failed**（`cargo-test-preparations.log`） |
| 2 | `npm --prefix apps/web run test:e2e -- pdf-preparation.spec.ts` | exit 0，**10 passed / 0 failed**（`e2e-rd-spec-run.log` + `artifacts/web-mvp/t09-rd/playwright-output/.last-run.json`=`{"status":"passed","failedTests":[]}`）；自管后端/临时目录见 `e2e-runtime.json` |
| 3 | `npm --prefix apps/web run test:e2e -- qa-t09-independent.spec.ts`（**QA 自写**，6 用例） | 第 1 轮 5 passed / 1 failed（`e2e-qa-independent.log`，失败项是 QA 自设的"同帧几何"判据——见 BUG-002 段，非产品失败）；**最终轮 exit 0，6 passed / 0 failed**（`e2e-qa-independent-2.log`） |
| 4 | Playwright 失败证据保留（临时故意失败 spec，跑后删除） | 失败时保留 `trace.zip` + `error-context.md`（有页面时另有 `test-failed-1.png`，见第 3 行首轮失败输出）；副本 `playwright-failure-evidence/`、日志 `playwright-failure-evidence.log` |
| 5 | `npm --prefix apps/web run typecheck` / `lint` / `test -- --run` / `build` | 全部 exit 0；vitest **6 files / 46 passed**；build 输出 `pdf.worker.min-Dswkl-cV.mjs`（Vite 哈希化 worker）与 `[plugin em-pdfjs-vendor] vendor/pdfjs 已写入 dist/vendor/pdfjs（cmaps=169 standard_fonts=16 wasm=13 iccs=2）`；首屏 `index-*.js` 343.53 kB（PDF.js 在懒加载块 `PreparePage-*.js`，不进首屏）（`web-checks-qa.log`） |
| 6 | `cargo test --workspace` | exit 0，**206 passed / 0 failed / 1 ignored**（唯一 ignored 是 T07 既有的 `storage/repo/items.rs` doctest，与本切片无关）（`cargo-workspace-qa.log`） |
| 7 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`；工作树未被改动（清单比对）（`contracts-check-qa.log`） |
| 8 | `cargo xtask check` | exit 0（fmt/clippy/workspace 测试/web lint+typecheck+test/合同检查全部 `[通过]`）（`xtask-check-dist-smoke.log`） |
| 9 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0；sha256 `44ab49e5…`、14 617 632 B（与 RD 同哈希）；产出 binary/SHA256SUMS/licenses.json/build-info.json |
| 10 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` | exit 0；1 项 `[准备] init` + **7 项 `[检查]`** 全过（页面/JS/health live+ready/`/api/unknown` JSON404/SPA 深链接/缺失资源 404）；服务日志确认无 Provider 配置时不回退 mock |
| 11 | `bash artifacts/web-mvp/t09-qa/check-embedded-vendor-qa.sh <dist>`（**QA 自写**） | exit 0：8 个 vendor 资源（CMaps×2/标准字体×2/WASM×3/ICC）在 release 二进制上全部 `HTTP 200` 且非空；站点服务的 `UniGB-UCS2-H.bcmap` 与 `node_modules/pdfjs-dist/cmaps/` 源文件 **sha256 逐字节一致**（`9201569b…`）；内嵌 CSS 含 `.notices` 的 `pointer-events:none` 与 `--notices-top`；未知 API → 404（`embedded-vendor-check-qa.log`） |
| 12 | `swiftc` + PDFKit 样例探针（QA 自写，系统引擎，非仓库解析器） | 文字 PDF=2 页有文字层；扫描 PDF=2 页无文字层；旋转 PDF=2 页（第 2 页声明 /Rotate 90）；非拉丁 PDF=1 页（PDFKit 取不到该页文字，见非代码知识 5）；加密 PDF **isEncrypted=true / isLocked=true，`fixture-secret` 解锁成功**；101 页 PDF **pageCount=101**（`fixture-probe/probe-output.txt`） |

**发布面（第 9–11 行）与浏览器面（第 2–3 行）分开记录**：前者证明"新增 PDF vendor 资源确实随二进制内嵌、字节一致、静态路径可服务、CSS 修复在场"；后者证明真实浏览器行为。两者拓扑不同（dev 代理 vs 同源内嵌 UI），本回合不把二者混成一次运行证据。

退出码口径：第 1/2/5 行的首轮执行经过管道（`PIPESTATUS` 未落盘），QA 已**复跑并以显式 `$?` 捕获退出码**：`cargo-test-preparations-2.log`（`PREPARATIONS_EXIT=0`，9 passed）、`e2e-rd-spec-run-2.log`（`E2E_RD_SPEC_EXIT=0`，10 passed）、`web-checks-qa-2.log`（`TYPECHECK_EXIT=0`、`LINT_EXIT=0`、`VITEST_EXIT=0`、`BUILD_EXIT=0`）。第 3 行 `e2e-qa-independent-2.log` 内已含 `EXIT=0`。

## AC 验收矩阵（本切片派发范围；QA 独立复现）

| AC | 期望 | 实际（QA 独立复现） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| **AC-022** | 每页 1-based `pageNumber` 的页文字与页图；同内容重复 PUT 幂等；页图长边 ≤2000px、白底 JPEG；原点=旋转后 viewport 左上；`GET /preparations/{id}` 反映已完成页 | ① Rust 9 用例覆盖：页号 1-based（页 0 → 422 `invalidPageNumber`；>100 → 422 `pageLimitExceeded`）、同内容 PUT 幂等（两次/不同 asset id 同内容均不增 revision、不新增行）、覆盖需 If-Match（缺 428/过期 412+`currentRevision`）、viewport 校验（零尺寸/2001px/45°）、资产归属（跨物品 404）与 purpose/JPEG 校验、`GET` 返回页集与 ETag；② e2e 实测 `PUT .../pages/1,2`（1-based 升序）、页图魔数 `FF D8 FF`、MIME `image/jpeg`、viewport 长边 ≤2000、页文字资产内容含 "Loosen the four captive screws"；③ **QA 自写加强**：`GET /preparations/{id}` 的 `viewport.width/height` 与页图 **JPEG 实际像素尺寸逐页相等**（旋转页 = 800×565 / rotation=90、正立页 565×800）、四角像素 RGB≥250 且 alpha=255（白底）；`GET` 反映已完成页（页集、`missingPages`、revision/ETag） | **PASS** | 命令 1/2/3；`cargo-test-preparations.log`、`e2e-rd-spec-run.log`、`e2e-qa-independent-2.log`（用例 6） |
| **AC-023** | 中断后重进只补缺页、不重传；离线时 worker/CMaps/standard fonts/WASM 本地加载（无 CDN）；扫描页走页图；旋转页与非拉丁字体页文字正确 | ① **QA 独立复现"只补缺页"**：第 2 页 PUT 注入 500 → 第 1 页完成；**真实 `page.close()` 关闭标签页**后开**新标签页**（sessionStorage 指针随旧 tab 消失，页面先显示"开始准备"，证明没有本地缓存假设）→ 点击后 PUT 序列恰为 `[2]`，且服务端第 1 页的 `imageAssetId`/`textAssetId` 与续传前**完全相同**（未重传/未替换）；② 离线：`page.route` 阻断全部非 127.0.0.1/localhost 请求后仍完成整轮准备，`external == []`，且页面内直接 fetch 五类资源（CMap/标准字体 2 种/WASM×2/ICC）全部 200 非空；worker 本地加载（`/node_modules/.../pdf.worker.min.mjs`）；主包与 worker 版本字符串与 `pdfjs-dist/package.json` **同为 6.3.289**；③ 扫描页 `textAssetId=null` + 页图存在（e2e）；旋转页 `rotation=90` 且宽高互换 + 页文字含 `ROTATE-PAGE-TWO`（e2e+QA 尺寸核对）；非拉丁页文字提取出「部件一：松开四颗螺丝」且伴随本地 CMap 请求（e2e+QA 内容断言） | **PASS** | 命令 2/3；`e2e-qa-independent-2.log`（用例 1/2）、`e2e-rd-spec-run.log` |
| **AC-024** | 加密 PDF、>100 页 PDF 均明确拒绝并给原因；不创建页记录、不进入 jobs、不产生收费请求；文案可行动 | **QA 独立复现（负例）**：拒绝发生在打开阶段，页面上 **0 个 `/preparations` 请求、0 个页 PUT**（请求清单断言）；文案逐字命中「该 PDF 已加密，首版不支持，请先解除加密后再上传」与「PDF 共 101 页，超过 100 页上限」；封存按钮禁用；随后 `sqlite3` 直查：`preparations`/`pages`/`jobs`/`cost_ledger`/`provider_attempts` 五表**零新增**。样例真实性由系统 PDFKit 独立确认（加密样例 isEncrypted/isLocked、101 页样例 pageCount=101）。服务端侧：PUT 页 101 与 complete `pageCount=101` → 422 `pageLimitExceeded`（Rust） | **PASS** | 命令 1/3/12；`e2e-qa-independent-2.log`（用例 5）、`fixture-probe/probe-output.txt` |
| **AC-025** | complete 成功页号连续 1..N + `clientDerived`（保留原件）；缺页/哈希不符 422 列缺项；ready 后写入被拒；complete 不创建 job、不产生费用 | QA 实测：① 页 1、3 已上传、声明 3 页 → 422 `details.reason=incompletePages` + `missingPages=[2]`；blob 被隔离 → 422 `assetMismatch` + `pages[{pageNumber,problem}]` 且**仍是 preparing**（无半封存）；缺 `If-Match` → 428；② 补齐后封存 → `state=ready`、`pageCount=N`、`clientDerived=true`、`missingPages=[]`；ready 后 PUT → 422 `preparationReady`、重复封存同样被拒；③ **`sqlite3` 直查 jobs/job_stages/cost_ledger/provider_attempts 均 0**（Rust 测试的写死表名计数 + QA e2e 的前后差值双重证据）；④ e2e 侧封存后页面显示「准备完成（ready）」+ 常驻 `clientDerived` 说明（"哈希只证明字节一致，不证明其确实来自原 PDF"）、写入控件禁用 | **PASS** | 命令 1/3；`cargo-test-preparations.log`、`e2e-qa-independent-2.log`（用例 1） |

## 卡内必验项与风险点（T09 卡 + UI-014–UI-018 + §6.3.2）

| 卡内要求 | 结果 | 证据 |
| --- | --- | --- |
| 逐页顺序处理 | PASS：PUT 序列 `[1,2]`，续传 `[2]`；单轮只处理缺失页（升序） | 命令 3 用例 1/4 |
| **100 页不同时铺满 canvas** | PASS（可观察部分）：QA 用 `addInitScript` 包裹 `document.createElement` 计数——整轮准备**最多创建 1 张 canvas**（离屏复用），DOM 中 canvas 数恒为 0（准备前后断言）；101 页样例在渲染前即被拒绝。100 页规模的渲染耗时/内存未测（T21/AC-062） | 命令 3 用例 4 |
| render task/canvas 在取消与切换时销毁 | PASS（代码核对 + 可观察行为）：`prepare.ts` 的 `onAbort` 取消进行中的 `renderTask`，`finally` 重置 canvas 尺寸（0×0）+ `loadingTask.destroy()`，每页 `page.cleanup()`；`PreparePage` 卸载/取消时 `abort()`；文档选择器在准备中禁用（不允许中途切换原件）。可观察面：取消后不再产生新页 PUT；重启（新标签页）不复用旧缓冲（每次 `fetchAssetBytes` 重读原件）。**GPU 资源释放无法在本回合直接观测**（未做内存取证） | 源码核对 + 命令 3 用例 4 |
| 关闭标签页中断提示 | PASS（合成事件）：进行中派发 `beforeunload` 被 `preventDefault`（e2e + QA 各自断言）；真实弹窗需用户激活，留 T21 人工走查（RD §T09-11 第 3 条一致） | 命令 2/3 |
| 进度文案「第 n / N 页」，不得出现线性总百分比 | PASS：进行中文本匹配 `第 \d+ / 2 页`，`role=progressbar` 的 `aria-valuenow`=已完成页数、`aria-valuemax`=真实页数；全文不含「总进度」「预计剩余」；QA 另做源码扫描：§6.3.2 禁用措辞清单 9 条在 `apps/web/src/**` 的生产代码中 0 命中（仅 `shell.test.tsx` 的禁用词清单常量） | 命令 2/3 + 源码扫描 |
| PDF.js 主包与 worker 同版本；资源由构建内嵌（无 CDN）；worker 路径不手工拼接 | PASS：`package-lock` 锁定 6.3.289；主包与 worker 产物内版本字符串均为 6.3.289；worker 由 `?url` import（dev `/node_modules/...`、build `/assets/pdf.worker.min-<hash>.mjs`）；CMaps/字体/WASM/ICC 由插件复制进 `dist/vendor/pdfjs`（169/16/13/2），release 二进制上全部 200 且 CMap 字节一致；离线用例 `external == []` | 命令 3/5/11 |
| worker 的 ArrayBuffer 被转移后不假设可读 | PASS（代码核对）：`fetchAssetBytes` 每次运行重新读取原件字节（`prepare.ts`/`vendor.ts` 注释 + `PreparePage.start` 每次 `fetchAssetBytes`）；续传用例在新标签页重新解析成功佐证 | 源码核对 + 命令 3 用例 1 |
| `pageNumber` 全链路 1-based（含前端与出处） | PASS：contracts §1/§5.3 与 OpenAPI 描述 1-based；服务端拒绝 0（`invalidPageNumber`）；前端 PUT 使用 `pageNumber`（1-based）且页资产文件名 `page-0001.jpg`；`GET` 返回页号从 1 起；e2e/QA 的 PUT 序列均无 0 基 | 命令 1/2/3 |
| UI-014 准备页逐页渲染与上传 / UI-015 取消与离开提示 / UI-016 拒绝说明 / UI-017 断线续传 / UI-018 封存 | PASS（逐条见上面各行的可观察断言）：进度与已完成页数、单页失败「第 N 页失败」+「重试本页」、取消后回到未开始且已完成页保留、常驻「准备需要保持本标签页打开」文案、拒绝文案逐字、`已完成 n / N 页，继续补齐`、封存成功/422 缺项渲染（`messages.ts` 单测 + 端到端） | 命令 1/2/3 + `PreparePage.tsx` 核对 |
| Playwright 设施：自管后端与临时目录、失败保留证据、不依赖外部服务 | PASS：运行前后 `lsof` 确认 18080/15173 无外部服务；`globalSetup` 自建临时 data-dir + `init`；`reuseExistingServer:false`；teardown 结束进程（QA 验后 `ps` 0 残留）；失败用例保留 `trace.zip`/`error-context.md`（+截图为 PNG）；后端日志留档 `artifacts/web-mvp/t09-rd/e2e-server.log` | 命令 2/3/4 + 进程/端口核对 |
| 回归证据链 | PASS：见"命令与原始结果"第 1、5–11 行；新增 PDF vendor 随二进制内嵌由 QA 自写脚本独立复核 | 命令 1/5–11 |

## BUG-002 复验（原 BUG-001-r8；P3 → **CLOSED**）

- 缺陷原文（回合 8，编号重编为 BUG-002）：成功通知条（`.notices` 固定顶部整宽条）在 6 秒可见期内覆盖顶栏，指针点击顶栏「资料库」被吞（`elementFromPoint` 命中通知条 DIV），键盘路径不受影响。
- RD 修复（QA 核对实现后实测定论）：① CSS：`.notices` 定位到 `top: var(--notices-top, var(--top-bar-height))`（顶栏之下）、容器 `pointer-events:none` + `.notice { pointer-events:auto }`；② `notifications.tsx`：通知可见时实测 `.top-bar` 高度写入 `--notices-top`，每 100ms 复核 + `resize` 监听。
- **QA 独立复验（我的 spec 用例 3；宽屏 1280 + 窄屏 390，真实 Chrome，通知可见时操作）**：

| 检查 | 1280px | 390px |
| --- | --- | --- |
| `elementFromPoint` 顶栏「资料库」链接中心（稳定态） | 命中 `A`/「资料库」 | 命中 `A`/「资料库」 |
| 真实指针点击链接 | 导航到资料库 ✓ | 导航到资料库 ✓ |
| 键盘：聚焦链接 + Enter（通知可见） | 导航成功 ✓ | 导航成功 ✓ |
| 通知条「关闭」按钮中心命中 | `BUTTON` 且可点 ✓ | `BUTTON` 且可点 ✓ |
| 稳定态几何（通知条顶 ≥ 顶栏底） | 63 ≥ 63 ✓ | 138 ≥ 138 ✓ |
| `.notices` 计算样式 | `pointer-events: none` ✓ | `pointer-events: none` ✓ |

- **残留窗口（QA 实测，非阻断，见下节建议）**：390px（顶栏因物品上下文换行为 138px）时，通知条出现后的**第一帧** `--notices-top` 仍是窄屏媒体查询兜底值 104px，链接中心 y=106 落在通知条下（`elementFromPoint` 命中 DIV）；**≤100ms** 后轮询复核写入 138px，此后稳定命中链接。原始样本：`artifacts/web-mvp/t09-qa/bug002-hit-samples.json`（`{"width":390,"firstFrame":{"tag":"DIV","linkText":null,...},"settled":{"tag":"A","linkText":"资料库","noticeTop":138,...}}`）与逐 50ms 时间线 `bug002-geometry-timeline.json`。QA 首轮自设"同帧几何"判据因此失败（`e2e-qa-independent.log`），复核后改用验收相关判据（稳定态命中 + **显式**断言，而非只靠 poll），保留该测量为证据。
- 复验结论：**CLOSED**。依据：(a) 稳定态下通知条完全位于顶栏之下且容器不拦截指针——"吞掉顶栏点击"的原缺陷在 6 秒全窗口内不再出现（宽/窄屏、指针/键盘、真实点击均过）；(b) 键盘可达性未回退；(c) RD 已在 §T09-11 第 8 条如实记录 ≤100ms 定位轮询窗口。残留窗口不足以重开：它是轮询定位的固有瞬态（≤100ms、仅在顶栏高度变化后、需在通知出现的同一瞬间精确点击同一链接才可触发），无数据/费用/安全影响，且不违反任何必选 AC/UI 条目。
- 回归范围：任何触发通知的操作（新建/保存/归档/恢复/封存…）之后顶栏各入口的指针与键盘可用性（宽屏 + 窄屏）；规则"通知不得遮挡主入口指针操作"（ADR-019 第 8 条）。

## 非阻断建议（不计入阻断项，不要求 T09 返工）

1. **P3｜390px 的 ≤100ms 定位窗口**（BUG-002 残留，见上）：建议把 `--notices-top` 改为同步测量（`ResizeObserver` 观察 `.top-bar`，或 `useLayoutEffect` + 首帧校正），可完全消除该窗口；若不改，建议在 T16/T17 大量使用通知前把"每 100ms 轮询"降级为"仅在可见期间 + ResizeObserver 兜底"。现状不阻断（判定依据见 BUG-002 段）。
2. **P3｜e2e 证据目录与 RD 共享**：`playwright.config.ts` 把 `outputDir`/后端日志固定写 `artifacts/web-mvp/t09-rd/`，QA/T21/T22 再跑同一 spec 会**覆盖** RD 的 `e2e-server.log`（本回合 QA 已先备份到 `t09-qa/rd-evidence-preserved/` 再执行）。建议后续卡改为按运行隔离（如 `t09-rd/` 只读快照 + `PLAYWRIGHT_*` 环境变量或时间戳子目录）。
3. **P3｜WASM/ICC 代码路径未被样例触发**（RD §T09-11 第 2 条）：本回合只证明了资源内嵌 + 可服务 + 浏览器内可 fetch；若要让 `jbig2/openjpeg/qcms` 真正解码，需要 JPX/JBIG2/ICC 样例（建议 T21 补样例并把请求断言加进 `pdf-preparation.spec.ts`）。
4. **P3｜100 页规模未实测**：AC-062 的耗时/内存与"100 页不同时铺满 canvas"的规模面归 T21；本回合只证明了单 canvas 与逐页顺序。
5. **P3｜RD 的 `check-embedded-vendor.sh` 不清理临时目录**：遗留 `/tmp/em-t09-embed.*`（7 个，QA 未删除以保留 RD 证据）；QA 自写脚本用 `trap cleanup EXIT` 自清。建议后续脚本统一加清理。
6. **P3｜非拉丁样例的 ToUnicode 可再增强**：系统 PDFKit（`PDFDocument.page.string`）读不出该页文字（返回空），而 PDF.js + 本地 CMap 能正确提取「部件一：松开四颗螺丝」——说明该样例的 ToUnicode 流对"非 PDF.js 引擎"不够友好。不影响本 AC（验的是 PDF.js 路径），但若 T14 知识提取要做"跨引擎一致性"或人工复核，建议把该样例的 ToUnicode 做成标准形态。

## 未覆盖边界（本回合记录，不冒充通过）

1. **发布二进制内嵌 UI 上跑同一 e2e spec** 未做（本回合浏览器面走 Vite dev + 代理；发布面只做了静态资源/内嵌 CSS/smoke）。T21/T22 应补（RD §T09-11 第 1 条同）。
2. **真实 `beforeunload` 弹窗**未观测（需用户激活）；只验证了监听注册与 `preventDefault` 生效。
3. **100 页规模**的渲染耗时、内存、逐页进度在真实大文件上的表现（含"取消时在途页最多 1 页"的规模面）未测。
4. **WASM/ICC 解码**未触发（样例不需要）；**JPX/JBIG2** 类 PDF 的拒绝/渲染行为未测。
5. **render task 与 canvas 的 GPU 侧销毁**无法直接观测（只验证了 canvas 尺寸归零、DOM 无 canvas、取消后无后续上传）。
6. **CMap 缺失时的反证**未做（未移除构建资源验证"无 CMap 必失败"）；本回合只证明"提取正确 + 同轮出现本地 CMap 请求"。
7. 触屏/移动浏览器、Safari、真实 Provider、性能基线（AC-061/062/063）不在本切片。
8. **RD 证据的独立性**：本回合未复用 RD 脚本/截图作证据；`rd-evidence-preserved/` 仅为保护性备份。
9. **"worker 缺失"负例未覆盖**（`validation-release.md` §3 PDF 行失败/恢复列的矩阵项之一）：本回合只验证了 worker 在场并本地加载（离线用例）；"worker 资源被删除/损坏时页面如何失败并可诊断"未构造（资源由构建强制内嵌、缺目录即构建失败，正常部署不易出现；若 T21 要覆盖，可在 dev 中间件或构建产物层构造）。

## 非代码知识与限制（跨卡复用）

1. **"只补缺页"的最强复现手法**：不要用 `page.reload()`（会保留 sessionStorage 指针）；用 `page.close()` + `context.newPage()`（新标签页天然丢失 sessionStorage），再断言 PUT 序列恰为缺失页 **且**已完成页的 asset id 未被替换（后者能抓"重传但内容相同被幂等掩盖"的假阴性）。
2. **断外网的完整判据**：阻断非本机请求（记录清单为空）**加上**"关键资源确实从本机加载"（worker/CMaps 请求出现且 200），两条缺一不可；进一步可在页面内直接 fetch 各类 vendor 资源（CMap/字体/WASM/ICC）验证"需要时可用"——比只等 PDF.js 触发更全面（本卡样例不触发 WASM）。
3. **SQLite 直查可绕过应用层做零副作用断言**：`sqlite3 <data-dir>/manual.sqlite3 "SELECT COUNT(*) FROM <表>"` 前后差值，是"不创建记录/不收费/不建 job"最硬的证据（WAL 下运行中读取安全）。T11/T12/T14 的"不产生费用记录""不重复购买"可沿用（配合 `job_stages`/`provider_attempts`）。
4. **Playwright 自管后端的运行时信息要惰性读取**（`globalSetup` 晚于测试文件收集）；**e2e 的 `outputDir` 目前写死在 `t09-rd/`**，多角色/多轮运行会互相覆盖，跑前先备份。
5. **系统 PDFKit 是负例 fixture 的独立验真器**：`PDFDocument.isEncrypted/isLocked/unlock(withPassword:)` 与 `pageCount` 可直接验证"加密样例真的加密""101 页样例真的 101 页"，避免自建 fixture 让负例测试空转；但 PDFKit 读不出本例的 CJK 页（见非阻断建议 6），文字层正确性仍以真实 Chrome + PDF.js 为准（Node legacy 构建无 XHR，不能验 CMap——T09 知识 3）。
6. **通知/浮层的定位不能只靠"设计值兜底"**：顶栏高度会因物品上下文与断点换行变化；用轮询/延迟测量会留下"变化后 ≤周期"的窗口（本卡 100ms）。替代：`ResizeObserver` 同步跟随后再显示，或把通知条放进文档流（撑开布局）。
7. **shell 脚本中 `$VAR` 紧跟多字节字符（如中文全角括号）在非 UTF-8 locale 下会被当作变量名的一部分**（本回合 QA 自写脚本踩坑：`SOURCE_BIN）` → `unbound variable`）；写中文注释/输出的脚本统一用 `${VAR}` 花括号形式。
8. 以上 1–7 与 BUG-002 的窗口测量已追加到 `llmdoc/decisions.md`「T09 验收知识」（QA 复验段）。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 9 | 2026-09-12 | slice | T09（AC-022/023/024/025 + UI-014–018 + BUG-002 复验） | 2（ui_revision 2） | PASS | 本文件 |

- 交接给 RD：**无返工项**；BUG-002 复验 **CLOSED**（依据见该段）。非阻断建议 1（`--notices-top` 同步测量）、2（e2e 证据目录隔离）、5（脚本清理临时目录）、6（样例 ToUnicode）可按协调者排期并入 T16/T21/T22 或 T09 的后续触碰；建议 3/4 是 T21 范围。
- 交接给协调者：（a）`accepted_tasks` 追加 `{task_id: T09, prd_revision: 2, qa_round: 9, report: llmdoc/requirements/web-mvp/qa-report.md, accepted_ac_ids: [AC-022, AC-023, AC-024, AC-025, "UI-014–UI-018"], status: accepted}`；（b）`qa_history` 追加本回合（result: PASS，scope: slice）；（c）`open_defects` 移除 `BUG-001-r8`（已复验 CLOSED；编号在 qa-report 重编为 **BUG-002**，避免与已关闭的 limiter BUG-001 混淆——如需保留旧编号可在 state 里写 `BUG-002（原 BUG-001-r8，CLOSED）`）；本回合**无新增 OPEN 缺陷**，非阻断建议 1–6 不阻断；（d）T09 已交付的路由/占位替换点（`/items/:itemId/import/prepare` 占位页已替换为真实页面）是 T16（向导第 4 步）与 T21/T22 的接入点；（e）QA 在 `apps/web/tests/e2e/` 新增 `qa-t09-independent.spec.ts`（6 用例，属验收测试，不进 vitest；`npm run typecheck/lint` 已在其上通过），后续全量 `test:e2e` 会一并运行它。
- 建议 state 更新：`phase` 推进到下一切片（T16 派发或按协调者顺序）；`prd_revision: 2`、`ui_revision: 2` 保持；`qa_round: 10`；`accepted_tasks` 追加 T09 条目（如上）；`qa_history` 追加回合 9（PASS）；`open_defects: []`（BUG-002 CLOSED）；`next_action`：派发 T16（向导/前端整合）或 T11（报价冻结，依赖 preparation ready——本回合已具备 ready 语义）。
- 证据目录：`artifacts/web-mvp/t09-qa/`：QA 自写 `qa-t09-independent.spec.ts`（仓库内 `apps/web/tests/e2e/`）、`check-embedded-vendor-qa.sh`、`fixture-probe/probe.swift`+`probe-output.txt`；日志 `cargo-test-preparations.log`/`-2.log`、`e2e-rd-spec-run.log`/`-2.log`、`e2e-qa-independent.log`（首轮，含失败判据）与 `e2e-qa-independent-2.log`（最终 6 passed / exit 0）、`playwright-failure-evidence.log`+目录、`web-checks-qa.log`/`-2.log`、`cargo-workspace-qa.log`、`contracts-check-qa.log`、`xtask-check-dist-smoke.log`、`embedded-vendor-check-qa.log`；数据 `bug002-hit-samples.json`、`bug002-geometry-timeline.json`、`e2e-runtime.json`、`source-manifest-before/after.txt`；`rd-evidence-preserved/`（RD 日志的防覆盖备份）。

# 回合 10 · T10（持久任务执行器）

**结果：FAIL（仅 T10 切片范围；1 个 OPEN 缺陷 BUG-003）** · 回合：10 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T10]；AC-035、AC-036、AC-037/AC-038 执行器侧 + 卡内项 + 回归）

- QA 执行时间：2026-09-12 09:12–09:22（本地 UTC+8；命令日志时间戳为 UTC 2026-09-12T01:12–01:21Z）；执行者：QA 子 agent。
- 独立性声明：`implementation.md` §T10 与 `artifacts/web-mvp/t10-rd/`（脚本、日志、进程演示）**只作线索，未用作通过证据**。本回合结论全部来自 QA 现场执行：
  (a) QA **自写** `crates/server/tests/qa_t10_independent.rs`（8 用例，不 import RD 的 `jobs_recovery.rs` 或 `common`；自带子进程入口、fixture 场景、种子 SQL 与断言；落库事实用原始 SQL 直查），含 **2 个进程级 SIGKILL 场景**（`libc::kill(SIGKILL)`，先断言崩溃现场、再断言恢复结果）；
  (b) QA 自写进程级脚本 `qa-t10-process-check.sh`（真实 dist 二进制 serve 启停、延后、优雅停止、锁释放、重启恢复扫描）；
  (c) QA 自写迁移链脚本 `qa-t10-migration-chain.sh`（合成 v4 库 → `check` 只读报待迁移 → `serve` 升级 v5）；
  (d) 独立核对 failpoint 门控（release/dist 二进制 `strings`/`nm` 全 0，**并带测试二进制正对照**）、`cargo xtask dist` 独立重建同哈希、`cargo tree` 生产依赖树；
  (e) **未引用 RD 的 SIGKILL 日志/截图**（RD 同场景日志仅用于事后比对）。
- 派发核对：`prd.md` §9.1 修订 2（ui_revision 2）与派发包一致（AC-035/AC-036 与 AC-037/038 为 [必选]）；`state.yaml` qa_round=10、qa_scope=slice、current_tasks=[T10]、open_defects=[]；`implementation.md` §T10 状态 RD_READY；RD §T10-2 文件清单**逐项在场**（`crates/core/src/jobs.rs`、`crates/server/src/jobs/{mod,handler,submission,executor,recover,failpoints}.rs`、`crates/server/src/storage/repo/{jobs,job_stages,attempts,audit}.rs`、`migrations/0005_job_execution.sql`、`crates/server/tests/jobs_recovery.rs`、配置与既有测试同步改动）。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD 仍为 `cb69afb`（"init"），T01–T10 交付物**尚未提交**；QA 用 159 文件 sha256 清单前后比对（`source-manifest-before/after.txt`）：**唯一变化 = QA 新增 `crates/server/tests/qa_t10_independent.rs`**，RD 生产代码／既有测试**逐字节未动**（QA 的 `cargo fmt --all` 未改动任何既有文件） |
| 平台／工具链 | macOS Darwin 25.6.0 · aarch64-apple-darwin；rustc/cargo 1.98.1；node v26.0.0；npm 11.12.1；SQLx 0.9.0 |
| 交付产物 | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`c7832470b7b5c5d288b9c812c69477844a19b26ab59ea32b47a08656daf92f43`**，15 073 840 B；QA **独立重建同哈希**（确定性构建、与 RD 一致）；build-info `features=["embedded-ui"]`（无 `job-failpoints`） |
| 数据／fixture | 全部为本机 fixture（T05 `FixtureServer`）与临时 data-dir（QA 自建、验后清理）；**未配置真实 Provider、0 次付费调用**；`sqlite3` 直查仅针对 QA 临时库 |
| 未验证 | 真实 Tripo/Manual AI 协议与费用（T12/T14/T23）；`reconcile`/`retry`/`cancel` HTTP 端点与对账 UI（T15/T17）；费用预留/结算与 unknown 预留保留（T11）；多实例并发（不在 MVP）；崩溃断点中的 blob rename / draft 响应未返回（T13/T15/T21） |

## 命令与原始结果（QA 现场执行，2026-09-12；日志在 `artifacts/web-mvp/t10-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test jobs_recovery` | exit 0，**22 passed / 0 failed**（`qa-jobs-recovery-run1.log`；含 RD 的 2 个真实 `kill -9` 用例） |
| 2 | `cargo test -p everything-manual --test qa_t10_independent`（**QA 自写**，8 用例） | exit 0，**7 passed / 1 ignored**（ignored = BUG-003 复现用例，见缺陷段；`qa-independent-run2.log`） |
| 3 | `cargo test -p everything-manual --test qa_t10_independent -- --ignored --nocapture qa_total_wait_budget` | **exit 101（复现 BUG-003）**：对照形态 `needs_input`；真实形态 4 次轮询后仍 `waiting_provider`（`qa-bug003-repro.log`） |
| 4 | `cargo test -p everything-manual --test qa_t10_independent -- --nocapture qa_sigkill` | exit 0，**2 passed**；原始输出含 `[qa] 付费 POST 在途时的独立写事务耗时=2.3ms`、`[qa] kill -9 pid=… fixture 记录总数=1`（`qa-independent-sigkill-nocapture.log`） |
| 5 | `cargo test --workspace` | exit 0，**244 passed / 0 failed / 2 ignored**（1 条 T07 doctest + 1 条 QA 的 BUG-003 复现；`qa-workspace-tests-full.log`） |
| 6 | `cargo xtask check`（冻结工作树后复跑） | exit 0，**7/7 `[通过]`** + `全部检查通过。`（`qa-xtask-check-final2.log`）。注：QA 首轮（未加 QA 测试前）误报的两次失败是 **QA 自己新文件**的 fmt/clippy 问题，修好后全绿——也旁证 check 不会短路隐藏失败 |
| 7 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`（`qa-contracts-workspace.log`） |
| 8 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0；sha256 `c7832470…`（**与 RD 同哈希**，QA 独立重建）；binary/SHA256SUMS/licenses.json/build-info.json 在场（`qa-dist-smoke.log`） |
| 9 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` | exit 0；init + **7 项 `[检查]`** 全过；服务输出含 `job_executor_start`（lease=120s renew=20s 上限 2/2）与 `job_executor_no_handlers` |
| 10 | `strings -a`/`nm -C` on **dist** 与测试二进制（QA 现场） | dist：6 个断点名 + `EM_TEST_FAILPOINT` + `job-failpoints` 全 **0**，`nm` failpoint 符号 **0**；测试二进制（正对照）：断点名各 3、`EM_TEST_FAILPOINT` 2、符号 1251 行（`failpoint-gate-dist-qa-rebuild.txt`、`failpoint-gate-testbin.txt`） |
| 11 | `zsh artifacts/web-mvp/t10-qa/qa-t10-process-check.sh`（**QA 自写**，真实 dist 二进制） | exit 0（`qa-process-check.log`）：init/check v5 → 执行器启动/空注册表 → 已入队阶段**延后**（`status=queued`、`attempt_count=0`、`next_run_at` 有值、`provider_attempts=0`、`last_error=阶段处理器未注册…`）→ SIGTERM 退出码 0 + `job_executor_stopped` + `已停止：data-dir 排他锁已释放。` → 重启恢复 `scanned=1 recovered=1 unknown=1` 收敛为 `submission_unknown`（attempt `unknown`、无远端 ID、job/attempt 计数不变）→ 无残留进程 |
| 12 | `zsh artifacts/web-mvp/t10-qa/qa-t10-migration-chain.sh`（**QA 自写**） | exit 0（`qa-migration-chain.log`）：合成 v4 库（0001–0004 + 与 sqlx 一致的 sha384 校验和）→ `check` **只读**报「待迁移（库 v4 → 程序 v5）」（前后 `_sqlx_migrations` 行数均 4）→ `serve` 自动迁移（`database_ready schemaVersion=5`；`job_stage_deps` 出现、`job_stages` 3 新列；`PRAGMA integrity_check=ok`、`foreign_key_check` 空）→ 迁移后 `check` 报「已就绪（v5）」 |
| 13 | `EM_JOBS__*` 运行时配置校验（真实 dist 二进制） | 默认 `jobs = lease=120s renew=20s`；`renew==lease` 与 `lease=601` 均 exit 3 且给出明确原因；`lease=60 renew=10` exit 0 |

## AC 验收矩阵（本切片派发范围；QA 独立复现）

| AC | 期望 | 实际（QA 独立复现） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| **AC-035** | 阶段 DAG 按合同落库（`batch_index`/`stage_kind` 唯一）；双 worker 竞争只有一个领取；过期 worker 晚到 receipt 可保存但不能推进；SIGKILL 后按已知远端 ID 继续查询、不产生第二次付费提交；并发上限 2/2 | ① **DAG 落库**：QA 原始 SQL 直查 `job_stage_deps`（批次→freeze、merge→两个批次）；重复 `(job,kind,batch)` 插入被拒、非批处理 `batch_index=1` 被拒；依赖未全 succeeded 时 merge 不可领取，两批次成功后解锁；② **竞争**：8 路并发领取仅 1 个成功、`lease_epoch=1`（RD 用例，QA 复跑）；③ **过期 worker**：QA 用真实短租约（300ms）+ 真实等待复现——旧 guard 推进被拒（状态仍 `running`）、`set_result_fact` 允许（usage 事实落库）、新 worker `take_over_expired` 后 `epoch=2` 可推进，旧 worker 晚到推进仍被拒；④ **SIGKILL**：QA 自写 2 例，真实 `kill -9`——付费 POST 已发出、响应未到 → 崩溃现场 attempt `submitting`/无远端 ID/阶段 `running` → 重启后 `submission_unknown`、attempt `unknown`、下游仍 `queued`、fixture POST 计数保持 **1**、jobs=1、attempts=1；已知 task ID 的查询被 kill → 重启用**同一** `qa-task-known` 继续 GET、POST=0、最终 `succeeded`；⑤ **并发上限**：3 个 job 的远端阶段 + 4 个说明书批次 → 首轮恰领取 4 个（远端 running=2、批次 running=2），一个批次完成后才补位且仍在飞 2（RD 用例用在飞峰值计数器独立佐证 2/2） | **PASS** | 命令 1/2/4；`qa-independent-run2.log`、`qa-independent-sigkill-nocapture.log`、`qa-jobs-recovery-run1.log` |
| **AC-036** | 429/5xx/超时 → `retry_wait`，退避 2/4/8/16/32 秒 + jitter 且尊重 `Retry-After`，超 5 次 → `failed`；资料/schema/支持能力不足 → `needs_input` 列可行动缺项、不无休止重试；**总等待超 30 分钟 → `needs_input` 且保留 task_id（恢复只查询、不重新购买）** | ①②③ **PASS（QA 复核 RD 断言 + 源码）**：固定 jitter 下退避序列精确 2/4/8/16/32 秒（`attempt_count` 1..5，第 6 次 → `failed` 且父 job `failed`）；`Retry-After: 3` 原样采用（不加 jitter）、`999999` 截断为 300s 且原因记录；`needs_input` 落两条缺项 JSON（`code`+可行动 message）、之后不再被领取；子句"不无休止重试"在 `needs_input`/`failed` 后 tick 均 Idle。④ **FAIL**：30 分钟总等待预算在**真实轮询链路形态**下不可达（QA 复现，见 BUG-003）——轮询阶段自身没有 attempt（本卡 fixture `FixtureTripoPoll` 与 RD §T10-10 第 6 条都从 job 级 `tripo_submit` 的 accepted attempt 取 task ID），执行器 `plan_advance` 却用"该阶段自己的最近 attempt"当等待起点，`waited_seconds` 恒为 0；QA 对照实验：给轮询阶段**人为**造一个 31 分钟前的 attempt → `needs_input`（RD 用例的形态）；只让 submit 级 attempt 老化 → 时钟推过 30 分钟并轮询 4 次后仍 `waiting_provider` | **FAIL**（4 个子句中 3 项通过；第 4 项仅在 RD 用例的人工造数形态下成立） | 命令 1/2/3；`qa-bug003-repro.log`、`qa-jobs-recovery-run1.log`；`crates/server/src/jobs/executor.rs:860-898` |
| **AC-037（执行器侧）** | 付费 POST 已发出但响应未到 → attempt `submission_unknown`、该分支后续购买暂停、**不自动重发** | QA 独立复现：断点式（RD 用例，panic）与**真实进程 kill -9**（QA 自写）两种注入都收敛为 `submission_unknown`；attempt `unknown`、无 `remote_task_id`（不编造）；下游 `queued`；fixture POST 计数 1→1（重启后无第二次付费 POST）；jobs/attempts 行数不变；恢复后连续 tick 均 Idle。远端 task ID **冲突**路径：RD 用例断言"记录审计 + 不覆盖 + 停机告警 + 该分支 unknown"（QA 复核断言与触发器兜底） | **PASS（执行器侧；`reconcile` 端点属 T15 不在本回合）** | 命令 1/2/4；`qa-independent-sigkill-nocapture.log`、`qa-process-check.log` |
| **AC-038（执行器侧）** | 同步批次响应未持久化 → 该批 `submission_unknown`；不假定 `response_id` 可轮询/重取；已持久化结果的批次不重跑；result 已存 checkpoint 未推进 → 补推进不重付 | QA 复核 RD 用例断言（含 `response_id: None` 断言与"已完成批次不动"）；QA 进程级脚本在真实二进制上复现同构场景：手工造"running + 租约过期 + attempt=submitting"的 `manual_extract` 现场 → 重启恢复 `unknown=1`、`action=submission_unknown`、attempt `unknown`、无远端 ID、job 会计数不变；结果事实已存而 checkpoint 未推进的补推进用例（断点 6）由 RD 用例断言（结果资产保留、补推进、POST=1）；QA 的 epoch 用例另行证明"事实可存、推进必须带 epoch" | **PASS（执行器侧；真实同步协议属 T14）** | 命令 1/2/11；`qa-process-check.log`、`qa-independent-run2.log` |

### 卡内项（T10 卡 + 派发包附加项）

| 卡内要求 | 结果 | 证据 |
| --- | --- | --- |
| 租约与 epoch 语义：默认 120s/20s 续约、条件更新领取、epoch 校验推进 | **PASS**：`ExecutorConfig::default()` 120/20（QA 运行时配置校验 + 真实二进制启动日志 `leaseSeconds=120 renewSeconds=20`）；领取 SQL 为 `BEGIN IMMEDIATE` + 条件更新 + 原子 `lease_epoch+1`；8 路竞争仅 1 胜（epoch=1）；续约用例（RD）断言 4 次采样剩余租约 ≥250ms 且 epoch 不变；QA 的过期 worker 用例证明"推进=带 guard（owner+epoch+lease_until>now）" | 命令 1/2/11/13；`qa-process-check.log`、`qa-independent-run2.log` |
| 事务领取不跨 HTTP 持事务 | **PASS**：QA 在**付费 POST 在途时**（子进程挂在 fixture 上）从另一连接完成一次写事务并计时——**2.3ms** 成功（未被长期写锁阻塞）；代码核对：领取/事实/推进各自短事务，HTTP 在事务外（`submission.rs` 标记 `submitting` 提交后才发请求） | 命令 4；`qa-independent-sigkill-nocapture.log`、`crates/server/src/jobs/{executor,submission}.rs` |
| **failpoint 仅测试构建存在（release/dist 独立核对）** | **PASS**：QA 独立重建的 dist 二进制（`c7832470…`，构建特征仅 `["embedded-ui"]`）：6 个断点名、`EM_TEST_FAILPOINT`、`job-failpoints` 的 `strings` 命中全 **0**，`nm` failpoint 符号 **0**；`cargo tree` 生产依赖树 failpoint 命中 0；**正对照**：同一批断点名在测试二进制命中 3/3/…、`EM_TEST_FAILPOINT` 2、符号 1251 行（检查非空转）；`serve` 日志亦含"崩溃断点仅测试构建" | 命令 10；`failpoint-gate-dist-qa-rebuild.txt`、`failpoint-gate-testbin.txt` |
| 执行器在 serve 启停时的行为 | **PASS**：启动绑监听前启动执行器并记录 `job_executor_start` + `job_executor_no_handlers`；空注册表下已入队阶段**延后**（不假成功、`attempt_count=0`、无 attempt、`last_error` 记因）；SIGTERM → `job_executor_stopped` → `serve_stop` → `已停止：data-dir 排他锁已释放。`（退出码 0）；随即重启成功（证明锁已释放）并完成恢复扫描 | 命令 11/9；`qa-process-check.log`、`qa-dist-smoke.log` |
| `job_stage_deps` 依赖表示与解锁条件 | **PASS**：依赖边在插入时按 `manual_core::jobs::stage_dependency` 物化（QA 原始 SQL 直查 merge→两个批次、批次→freeze）；领取 SQL `NOT EXISTS(... dep.status <> 'succeeded')`；解锁只发生在依赖**全部** succeeded 之后（QA 用例：b0 完成、b1 在跑时 merge 不可领取；两者完成后 merge 被领取） | 命令 2；`qa-independent-run2.log` |
| 崩溃断点（"至少三个"）：付费 POST 发出前 / 供应商接受但响应未到 / task ID 到达但状态未推进 | **PASS**：三类都有"注入 → 崩溃现场 → 重启结果"的完整证据——① 发出前（intent/submitting 两个断点，RD panic 用例 + QA 真实 kill 前的 `submitting` 现场断言）；② 供应商接受但响应未到（QA **真实进程 kill -9**：POST=1、无远端 ID → `submission_unknown`、绝不重发）；③ task ID 到达但状态未推进（RD 断点 4 用例 + QA 进程级"查询被 kill 后按同一 ID 继续"）；另覆盖同步批次与"结果事实先落库"两个断点 | 命令 1/2/4/11/12；`qa-independent-sigkill-nocapture.log`、`qa-jobs-recovery-run1.log`、`qa-process-check.log` |
| 回归：workspace 测试／check／合同／dist + smoke／schema v5 迁移链 | **PASS**：见命令 5–9、12；schema 从 v4（0001–0004 合成库）**只追加**升级到 v5（5 行 `_sqlx_migrations`、`job_stage_deps` 与 3 个新列出现、`integrity_check` ok、`foreign_key_check` 空、`check` 前后行数不变=只读）；既有表不变量由 workspace 内 `storage.rs` 15 用例继续覆盖 | 命令 5–9/12；`qa-workspace-tests-full.log`、`qa-migration-chain.log` |

## 崩溃注入独立性说明（我做了什么、观察到什么）

1. **我自己的注入代码**：`crates/server/tests/qa_t10_independent.rs` 里 QA 自写子进程入口 `qa_t10_child_entry`（环境变量 `QA_T10_CHILD_*` 驱动，无变量时空操作）、自写处理器（`QaHangPaySubmit` 严格按"intent → submitting → 发 POST"顺序；`QaPollKnownTask` 先取 resume 提示、再回退到 job 级 accepted attempt）、自写 fixture 场景（挂起 120s 的 POST/GET 与健康的 GET）。父进程用 `libc::kill(pid, SIGKILL)` 直接发信号（不 shell 出 `kill`），并断言"进程不应正常退出"。
2. **观察序列**（付费 POST 场景，`qa-independent-sigkill-nocapture.log`）：fixture 首次记录到唯一 1 次 POST（说明请求已离开进程）→ 在途时另一连接写事务成功（2.3ms，证明未跨 HTTP 持事务）→ `kill -9`（pid 已回收、fixture 总数仍 1）→ **重开库检查崩溃现场**：attempt `submitting`、阶段 `running` → 等 1.5s（1s 租约过期）→ 重启子进程做恢复 → 重开库：`submission_unknown` / attempt `unknown` / 无远端 ID / 下游 `queued` / jobs=1 / attempts=1 / fixture POST 仍为 1。
3. **第二个场景（已知 task ID）**：查询被挂起时 kill -9（fixture 记 GET，POST=0）→ 换健康 fixture 重启 → 用**同一** `qa-task-known` 继续查询（新 fixture 计数器 ≥1）、POST=0、阶段 `succeeded`、attempts=1。
4. **与 RD 证据的关系**：RD 的 `sigkill-tests.log` 等仅作事后比对（同一结论），未作为本回合任何判定的依据；QA 崩溃注入的原始输出、pid 与计数均在本报告引用的 QA 日志中。

## BUG-003 · 30 分钟总等待预算在真实轮询链路上不可达（AC-036 第 4 子句）

- 严重度／状态：**P2 / OPEN**（修复后由 QA 复验）
- 对应 REQ / AC：REQ-024 / **AC-036 [必选]**（"总等待超 30 分钟转 needs_input 且保留 task_id（恢复只查询、不重新购买）"）；contracts §5（"总等待默认 30 分钟后进入 needs_input，保留 task_id"）
- 环境与输入：临时 data-dir（QA 自建）；`manual_extract`/`tripo_poll` 真实执行链；fixture 返回远端 `status=running`；轮询阶段**不自带 attempt**（与本卡 `FixtureTripoPoll`、RD §T10-10 第 6 条"`tripo_poll` 通过 `latest_accepted_for_job(job, tripo_submit)` 取远端 task ID（T12 可直接复用）"一致），仅 `tripo_submit` 有 31 分钟前的 accepted attempt（含 task ID）。
- 复现步骤：
  1. 造 job：`freeze_inputs`/`tripo_upload`/`tripo_submit` = succeeded，`tripo_poll` = queued；给 `tripo_submit` 造 `accepted` attempt，`started_at = now − 31 分钟`，task ID `qa-task-wait`；
  2. 注册与真实适配器同构的查询处理器（远端 `running` → `StageOutcome::WaitingProvider`），用 `ManualClock` 驱动执行器；
  3. 连续 `tick()` 并把时钟推过 30 分钟（按 3/6/12/15 秒节奏到点重新领取，共 4 轮）；
  4. 对照：另造一个 job，改为让 **`tripo_poll` 自身**带一个 31 分钟前的 accepted attempt（即 RD 用例的形态），同样 tick 一次。
- 命令：`cargo test -p everything-manual --test qa_t10_independent -- --ignored --nocapture qa_total_wait_budget_must_trigger_on_real_poll_shape`（用例地址 `crates/server/tests/qa_t10_independent.rs:1247` 起；`#[ignore]` 说明见下）
- 期望与实际：
  - 期望（对照）：`needs_input` + 缺项 `remote_wait_budget_exceeded` + task ID 保留 → **实际得到 `needs_input`**（对照通过）。
  - 期望（真实形态）：时钟推过 30 分钟并轮询 4 次后应为 `needs_input` → **实际恒为 `waiting_provider`**（`qa-bug003-repro.log` 四行 `观察 2 … status=waiting_provider`，随后断言失败 `left: "waiting_provider", right: "needs_input"`）。
- 根因（QA 定位，供 RD 参考）：`crates/server/src/jobs/executor.rs` 的 `plan_advance` 用 `attempt.started_at`（该**阶段自己**的最近 attempt，见 `execute()` 里 `repo::attempts::latest_for_stage(&conn, &stage.id)`）作等待起点；`StageOutcome::WaitingProvider` 分支在 `attempt = None` 时 `waited_seconds = 0`（`unwrap_or(0)`），于是 `remote_wait_exceeded` 永不成立。轮询阶段在真实链路里不建自己的 attempt（也不应该借 `begin_intent` 造一个"付费提交"），所以该保护在真实链路上是死代码；受影响行为还包括"到点后保留 task_id 并可继续查询"的展示语义（task ID 实际保存在 submit 级 attempt）。
- 影响：远端任务长时间未完成时，任务会**无限轮询**（15s 节奏）而不进入 `needs_input`，用户看到的是永不结束的"等待供应商"，与 REQ-024/AC-036 的"停止自动等待、保留 task_id、只查询不重购"承诺不符；无重复付费/数据损坏风险。
- 建议修复方向（不限定实现）：`WaitingProvider` 分支的等待起点改为"该 job 最近的 accepted 提交事实"（如 `repo::attempts::latest_accepted_for_job(job, TripoSubmit)`——轮询处理器已在用同一份事实取 task ID），或在阶段上显式持久化"等待起点"事实；修复后需同时保证 `needs_input` 的缺项文案与 task ID 展示仍可追溯到该 accepted attempt。
- 回归范围：`plan_advance` 的 `WaitingProvider` 分支（预算触发/未触发两侧）、轮询节奏与 `poll_count`、`needs_input` 缺项语义、恢复矩阵中"有 task ID 继续查询"路径；配套用例 `remote_wait_budget_turns_poll_into_needs_input_and_keeps_task_id` 与 QA 的复现用例（移除 `#[ignore]` 后应转绿）。
- RD 修复摘要引用：待填（RD 回合 11）。
- QA 复验结果与日期：待复验。

> 复现用例已放在 QA 自写测试文件内并标 `#[ignore = "BUG-003 复现…修复后移除 ignore"]`：标准 `cargo xtask check` / `cargo test --workspace` 保持全绿，同时该用例可随时用 `-- --ignored` 单跑取原始失败证据（本报告命令 3）。这是本回合**唯一**被 ignore 的 QA 用例，另一条 ignored 是既有的 T07 doctest。

## 非阻断建议（不计入阻断项，但建议排期）

1. **P3｜RD 文档计数不一致**：`implementation.md` §T10-9 第 4 行写 "`cargo test --workspace` → **236 passed**" 且明细里写 `jobs_recovery 21`，而同节第 1 行写 jobs_recovery **22**；QA 实测（加入 QA 用例前）应为 **237**（RD 明细漏计 1 条）。建议 RD 顺手修正文本（不影响功能）。
2. **P3｜failpoint 证据口径**：RD §T10-7 的测试二进制 `strings` 计数为 57/57/…/40，QA 实测同一断点在 `jobs_recovery` 测试二进制为 3/3/…/2（不同二进制/统计范围所致）。不是问题，但建议证据里写明"哪个二进制、统计的是什么"，避免后续角色复算对不上。
3. **P3｜等待起点应在 T12 卡内显式化**：即使按 BUG-003 修好锚点，也建议 T12 在适配器契约里写清"等待起点 = accepted 提交事实"，并把"30 分钟预算"纳入 T12 的契约用例（真实协议形态），而不是只依赖 T10 的 fixture 造数。
4. **P2｜（与 BUG-003 同源，不重复计数）**：`model_download` 等未来可能返回 `WaitingProvider` 的阶段同样受锚点问题影响，修 `plan_advance` 时一并覆盖。

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实 30 分钟墙钟**未等待：预算相关判定用 `ManualClock` 驱动（等价性已由"BUG-003 复现实验"旁证：时钟推进确实改变领取时点与轮询次数）；真实时间下的长跑未做（成本高、需外部供应商配合）。
2. **优雅停机的在途 checkpoint 完成**只观察了短任务（未注册处理器，秒级）；"宽限 5s 超时后中止在途阶段"的路径未构造（需长任务处理器 + 观察 `job_shutdown_abort_inflight`）。
3. `reconcile`/`retry`/`cancel` 的 HTTP 端点、任务中心 UI、费用预留/结算（T15/T17/T11）不在本回合；`unknown` 预留保留、`recordNoTask` 证据要求等属 T11/T15。
4. 真实 Tripo/Manual AI 协议的字节级行为、429/5xx 的真实形态（T12/T14/T23）未验证；本回合全部 HTTP 均发往本机 fixture。
5. 五个崩溃断点中的 **blob rename 后 DB 事务前**、**draft 已提交但 HTTP 响应未返回** 与本卡无关（T13/T15/T21）。
6. **多进程竞争同一行**只在单进程内 8 路并发（RD）与"旧/新 worker 顺序接管"（QA）下验证；两个**真实进程同时**领取同一行的竞争未构造（SQLite 写锁 + 条件更新为同一机制，但真机双进程未跑）。
7. data-dir 排他锁与执行器的组合只在"同一进程重启"下验证；多实例同时开同一 data-dir（应被锁拒绝）属 T02 既有结论，本回合未重复。

## 非代码知识与限制（跨卡复用）

1. **崩溃注入的最强形态**：进程级 `libc::kill(SIGKILL)` + **先断言崩溃现场、再断言恢复结果**（只看恢复后状态会漏掉"崩溃点其实没发生"的假阳性）；子进程用环境变量驱动、父进程用 fixture 计数当"请求确实离开进程"的锚点。T12/T13/T15/T21 的崩溃矩阵可复用该形态。
2. **"不跨 HTTP 持事务"的可观察判据**：请求在途时从**另一连接**做一次写事务并计时（本回合 2.3ms）。比读代码更硬，且能发现"连接池被单事务占满"类问题。
3. **迁移链验证手法（可复用）**：合成旧版本库 = 按顺序执行历史迁移 SQL + 手工写 `_sqlx_migrations` 行（`checksum` = 迁移文件 **SHA-384 大写 hex** 的 BLOB，先用全新 `init` 的库反证算法）；data-dir 结构需自带 `tmp/ logs/ blobs/ lock`（少一个即 `check` 退出 4）。可用来验证 schema ≤ 当前版本的任意升级路径。
4. **failpoint 门控核对必须带正对照**：只在发布二进制上 `strings` 全 0 可能因"断点名根本没进任何二进制"而空转；须同时在测试二进制上看到非 0 计数（本回合 3/3/…/2 + `nm` 1251 行）。
5. **预算/等待类逻辑要用"真实链路形态"验证**：给被测阶段手工造 attempt 会让"锚点不可达"类缺陷（BUG-003）看起来通过；必须按适配器真实的数据形态（哪个阶段持有什么事实）复算。
6. **`#[ignore]` 承载"合同期望 vs 当前实现不符"的复现用例**：标准 check 保持绿，复现证据可随时单跑；但报告必须显式声明（本回合 1 条，另有既有 T07 doctest 1 条），避免"看起来没有 unrun 项"。
7. 环境噪音：本机 `~/.cargo/config` 的 deprecated 警告与结论无关；`cargo test --workspace` 的 target 数统计以 QA 全量日志为准（RD 明细曾漏 1）。
8. 以上 1–7 已追加到 `llmdoc/decisions.md`「T10 验收知识（QA 回合 10）」。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–9 | 2026-09-12 | slice | T01–T09 | 1/2 | PASS | 本文件上半部分 |
| 10 | 2026-09-12 | slice | T10（AC-035、AC-036、AC-037/038 执行器侧 + 卡内项 + 回归） | 2（ui_revision 2） | **FAIL** | 本节 |

- 交接给 RD（**必须修复后由 QA 复验**）：**BUG-003（P2，OPEN）**——`plan_advance` 的等待起点用"阶段自己的最近 attempt"，真实轮询链路（`tripo_poll` 无自身 attempt）下 30 分钟预算不可达；修复点与建议方向见缺陷段，复现用例 `crates/server/tests/qa_t10_independent.rs::qa_total_wait_budget_must_trigger_on_real_poll_shape`（`#[ignore]`，修好后移除 ignore 并转绿）。修复不得通过改 PRD、删断言或放宽预算常量来"转绿"。
- 交接给协调者：(a) `qa_result: FAIL`、`open_defects` 追加 `BUG-003（P2，OPEN；对应 AC-036）`；(b) `qa_history` 追加回合 10（result: FAIL，scope: slice）；(c) T10 **不**进入 `accepted_tasks`；AC-035 与 AC-037/038 执行器侧、卡内项均已独立通过，可在缺陷修复后直接复验受影响的 AC-036 子句与回归；(d) QA 在 `crates/server/tests/` 新增 `qa_t10_independent.rs`（8 用例，含 2 个真实 SIGKILL 场景与 1 条 `#[ignore]` 复现；`cargo xtask check` 全绿已含它），后续全量测试会一并运行；(e) 修复建议顺带处理非阻断建议 1（文档计数）。
- 建议 state 更新：`phase: qa_failed`（或按协调者约定）；`qa_round: 10`；`qa_result: FAIL`；`open_defects: [{id: BUG-003, severity: P2, ac: AC-036, status: OPEN, owner: rd}]`；`current_tasks: [T10]` 保持；`next_action`：交 RD 修复 BUG-003（等待起点锚点）后 QA 复验（回合 11），复验范围 = AC-036 第 4 子句 + 相关回归。
- 证据目录：`artifacts/web-mvp/t10-qa/`——日志 `qa-jobs-recovery-run1.log`、`qa-independent-run1/-run2.log`、`qa-independent-sigkill-nocapture.log`、`qa-bug003-repro.log`、`qa-workspace-tests-full.log`、`qa-xtask-check-final2.log`、`qa-contracts-workspace.log`、`qa-dist-smoke.log`、`qa-process-check.log`、`qa-migration-chain.log`、`failpoint-gate-dist*.txt`、`failpoint-gate-testbin.txt`；脚本 `qa-t10-process-check.sh`、`qa-t10-migration-chain.sh`；数据 `source-manifest-before/after.txt`。

---

# 回合 11 · T10（BUG-003 修复复验）

**结果：PASS（仅 T10 切片范围；BUG-003 复验 CLOSED）** · 回合：11 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T10]；复验范围 = AC-036 第 4 子句 + BUG-003 回归 + 相关全量回归）

- QA 执行时间：2026-09-12 09:32–09:39（本地 UTC+8；命令日志时间戳为 UTC 2026-09-12T01:32–01:39Z）；执行者：QA 子 agent（回合 11）。
- 独立性声明：RD §T10-13 与 `artifacts/web-mvp/t10-rd-bug003/`（含 repro-before-fix、real-shape-evidence、jobs-recovery-after-fix、dist/smoke 日志）**只作线索，未用作通过证据**。本回合结论全部来自 QA 现场执行：
  (a) 复跑回合 10 的 `#[ignore]` 复现用例（**运行，未修改该文件**）——修复前失败、本回合通过；
  (b) **新写** `crates/server/tests/qa_t10_r11_verify.rs`（4 用例，不引用 `jobs_recovery.rs`/`common`/`qa_t10_independent.rs` 的任何代码或夹具；含真实链路 + 真实 HTTP fixture、锚点归属、非 accepted 事实、阈值边界与多 job 隔离）；
  (c) 独立重建 dist 二进制（与 RD 同哈希）、smoke-bootstrap、failpoint 门控（带测试二进制正对照）；
  (d) 全部计时用 `ManualClock` 可注入时钟，**0 真实 sleep**（含真实 HTTP 的 4 用例合计 0.10s）。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）与派发包一致，AC-036 第 4 子句仍为 [必选]；阈值常量 `REMOTE_WAIT_BUDGET_SECONDS = 1800`（`crates/core/src/jobs.rs:131`）**未被降低**；`remote_wait_exceeded` 仍为 `>=`（回合 10 语义，未改）。
- 交付版本：dist `dist/aarch64-apple-darwin/everything-manual` sha256 **`cf754cf0eedfbc0805733a819804e91607e1fe90ac9d2e1874d0adb13f84b9e2`**（15 094 640 B，QA 独立重建，**与 RD 同哈希**；`features=["embedded-ui"]`、`schemaVersion: 1`、commit `cb69afb` dirty）；dist 同哈希亦旁证生产源码自 RD 构建以来未被改动。

## 命令与原始结果（QA 现场执行，2026-09-12；日志在 `artifacts/web-mvp/t10-qa-r11/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test jobs_recovery` | exit 0，**25 passed / 0 failed**（22 条原用例名全在 + 3 条新 `wait_budget_*`；`qa-jobs-recovery-r11.log`） |
| 2 | `cargo test -p everything-manual --test qa_t10_independent`（**未修改该文件**） | exit 0，**7 passed / 1 ignored**（`qa-independent-standard.log`） |
| 3 | `cargo test -p everything-manual --test qa_t10_independent -- --ignored --nocapture qa_total_wait_budget` | **exit 0，1 passed**（修复前为 exit 101 + 断言失败）：`观察 1 … status=needs_input`；`观察 2（submit 级 attempt 才老化）status=needs_input` ×4（修复前同位置为 `waiting_provider` ×4，`qa-bug003-repro.log`）（`qa-bug003-reverify.log`） |
| 4 | `cargo test -p everything-manual --test qa_t10_r11_verify`（**QA 新写**，4 用例） | exit 0，**4 passed / 0 failed**，0.10s（`qa-r11-verify-final.log`；编译期版本日志 `qa-r11-verify-run1/run2.log`） |
| 5 | `cargo test -p everything-manual --test jobs_recovery -- --nocapture wait_budget_triggers_on_real_chain_shape_without_poll_attempt` | exit 0，1 passed；`[rd probe] 真实轮询链路形态：120 次轮询（时钟推进 1800s）后 poll status=NeedsInput，task_id=task-real-chain 保留`（0.09s，确定性时钟）（`qa-realshape-probe-r11.log`） |
| 6 | `cargo test --workspace` | exit 0，**254 passed / 0 failed / 2 ignored**（= RD 的 250 + QA 新增 4；2 ignored = QA 回合 10 复现用例 + 既有 T07 doctest）（`qa-workspace-tests-r11.log`） |
| 7 | `cargo xtask check` | exit 0，**7/7 `[通过]`** + `全部检查通过。`（`qa-xtask-check-r11b.log`；首轮 `qa-xtask-check-r11.log` 因 **QA 新文件**的 clippy `needless_borrow` 失败——check 未短路隐藏失败，QA 修正自己文件后全绿；生产代码未动） |
| 8 | `cargo xtask contracts --check`（在 #7 内） | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`（DTO 未变更） |
| 9 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0；sha256 **`cf754cf0…`**（与 RD 同哈希，QA 独立重建）；build-info `features=["embedded-ui"]`（`qa-dist-r11.log`） |
| 10 | `cargo xtask smoke-bootstrap --binary $PWD/dist/aarch64-apple-darwin/everything-manual` | exit 0；init + **7 项 `[检查]`** 全过；服务输出含 `job_executor_start`（lease=120s renew=20s 上限 2/2）与 `job_executor_no_handlers`（`qa-smoke-bootstrap-r11.log`） |
| 11 | `strings -a`/`nm -C` on **dist** 与测试二进制（QA 现场） | dist：6 个断点名 + `EM_TEST_FAILPOINT` 全 **0**，`nm` failpoint 符号 **0**；**正对照**：测试二进制断点名各 3、`EM_TEST_FAILPOINT` 2、`nm` 1251 行（`qa-failpoint-gate-dist-r11.txt`、`qa-failpoint-gate-testbin-r11.txt`） |

## 复验要点逐条（派发要求 1–7）

| # | 要求 | 实测（QA 独立） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| 1 | **真实形态**（轮询阶段无自身 attempt）：推过 30 分钟 → `needs_input`、task_id 保留、缺项可行动、恢复只查询不重购 | ① 回合 10 复现用例（真实形态）由 `waiting_provider`×4 变为 `needs_input`（命令 3）；② QA 新用例 T1：submit accepted（1790s 前）→ 3/6/12s 三拍后第 4 拍越界 → `needs_input`；缺项 `remote_wait_budget_exceeded`，message 含"远端任务 ID 已保留"+"不重新购买"；task ID `qa-r11-task` 保留在 submit 事实（state=`accepted`）；③ 恢复模拟（T15/T17 入口的服务端对应物，原始 SQL 置 `queued`）：再次领取只按**同一** task ID 发 1 次 GET（GET 4→5），**零 POST**、attempt 数不变、随即回到 `needs_input` | **PASS** | 命令 3/4；`qa-bug003-reverify.log`、`qa-r11-verify-final.log` |
| 2 | **反向核对**：轮询全程无 attempt、该 job attempt 总数=1、计时起点取自 job 级 accepted 提交事实 | ① "无 attempt"在 T1 **每一拍**断言（poll 阶段 attempt 计数恒 0，含恢复后）；② 全 job attempt 总数每拍断言=1（无第二次付费提交）；③ 计时起点可复算：越界 note = `总等待 1811s（≥ 30 分钟）→ needs_input（保留 task_id：qa-r11-task）`，1811 = 1790（提交事实起点）+ 3+6+12（节奏），仅当锚点取 `tripo_submit` accepted 的 `started_at` 才成立；④ 归属反证：把 job/阶段行 `created_at/updated_at` 回拨 40 分钟但提交事实为"刚刚"→ 仍 `waiting_provider` 且"远端已等待 0s"（排除行时间戳/阶段年龄做锚点） | **PASS** | 命令 4；`qa-r11-verify-final.log` |
| 3 | **跨 job／非 accepted 事实不串用** | ① T2：submit 阶段仅有 `intent`／`submitting`／`unknown`（31 分钟前）→ 均 `waiting_provider`（3s 节奏、无缺项）；仅有 `response_id` 的同步 receipt（job 级与阶段级）同样不起跑；执行器未为其建 attempt；② T4：同库三 job 交错 tick——X（恰好 1800s）→ `needs_input`，Y（1799s）→ `waiting_provider` 3s，Z（刚提交）→ `waiting_provider` 3s，Y/Z 无缺项、attempt 数不变（X 的老化事实不污染他人） | **PASS** | 命令 4；`qa-r11-verify-final.log` |
| 4 | **未降低阈值、未删除断言** | ① 常量仍 1800s（源码行 + 边界用例：恰好 1800 触发、1799 不触发）；② 22 条原用例名**全部在场且通过**（命令 1），抽查关键数值断言未弱化：退避 2/4/8/16/32 + 第 6 次 failed、`Retry-After: 3` 原样/`999999→300s` 记录、轮询节奏 3/6/12/15、`paid_post_without_response`（POST=1、unknown、不重购）、`manual_batch_without_persisted_response`（`response_id: None`）等；③ 新增仅 3 条（22→25），全部与本次缺陷相关 | **PASS**（附限：无法对 `jobs_recovery.rs` 逐字节 diff，见"未覆盖边界"6） | 命令 1；`qa-jobs-recovery-r11.log`、源码逐条复核 |
| 5 | **不是真实 sleep 绕过** | RD 真实形态用例 120 次轮询/1800s 推进实测 **0.09s**；QA 4 用例（含真实 HTTP fixture）**0.10s**；两者均用 `ManualClock`；jobs_recovery 整包 25 用例 1.92s（回合 10 的 22 用例为 1.99s，无墙钟回退） | **PASS** | 命令 1/4/5 |
| 6 | **QA 验收测试文件未被修改** | 见下节"QA 测试文件完整性核对"：mtime 09:20:45 早于 RD 修复窗口；内容与回合 10 报告/日志逐项一致（panic 行 1355:5、忽略理由、8 用例名）；`#[ignore]` 仍在（RD 未"顺手移除"）；本回合 QA 亦未改它 | **PASS**（附限：manifest-after 哈希为回合 10 QA 自身陈旧值，已解释并记录） | `qa-bug003-repro.log`、命令 2/3；`source-manifest-after.txt` |
| 7 | **回归**：workspace／check／合同／dist+smoke／failpoint 门控 | 命令 6–11 全部 exit 0：254 passed/2 ignored、check 7/7、合同 2 `[一致]`、dist 同哈希 + 冒烟 7 项、dist 断点符号全 0 且测试二进制正对照非 0 | **PASS** | 命令 6–11；`artifacts/web-mvp/t10-qa-r11/` |

## QA 测试文件完整性核对（要求 6，含一处必须解释的哈希差异）

- 现状：`crates/server/tests/qa_t10_independent.rs` sha256 **`9aeeeaf3df0d11f0390c97db9a3e05be902bed426ad331dced67764d2403ce60`**（47 274 B，1365 行，mtime **09:20:45**）。回合 10 的 `source-manifest-after.txt`（09:18:02 生成）记录的是 **`142e287d…`**——**两者不一致**。
- 判定：该差异**不是 RD 改动**，理由三条：(a) 文件 mtime 09:20:45 **早于 RD 修复窗口**（RD 首个产物 `t10-rd-bug003/repro-before-fix.log` 为 09:24；生产代码 mtime 09:25–09:27）；(b) 内容与回合 10 自身证据逐项吻合——回合 10 `qa-bug003-repro.log`（09:20:48）的记录为 panic 于 `qa_t10_independent.rs:1355:5`，当前文件第 1355 行正是该 `assert_eq!`；忽略理由文本与 RD 09:27 运行输出一致；(c) 回合 10 的清单在 09:18:02 生成，而其复现日志（含"观察 1 对照"两段输出）在 09:20:48，即**回合 10 QA 在清单之后又编辑了自己的文件**（报告 §BUG-003 复现步骤 4 描述了该对照），manifest-after 因此是陈旧快照。
- 复核补充：RD §T10-13 明确"未改该文件"，且其 09:27 运行仍显示 1 ignored（忽略标记未被移除）；`git status` 无异常（该文件为 untracked 路径下 QA 自建文件）。
- 局限（如实记录）：仓库中不存在回合 10 的末态字节副本，**无法做逐字节 diff**；上述结论基于 mtime + 内容一致性 + 交叉日志。按派发约束，本回合 QA **未修改**该文件；其承载的合同期望已由**常驻**回归 `qa_t10_r11_verify.rs`（无 `#[ignore]`，4 用例）承接。

## 缺陷状态

- **BUG-003（P2，AC-036 第 4 子句）：CLOSED（复验通过，2026-09-12 回合 11）。** 关闭依据：回合 10 的失败复现（真实形态 4×`waiting_provider`）现为 `needs_input`；QA 独立 4 用例覆盖真实形态、锚点归属、非 accepted 事实、边界与多 job 隔离；全量回归绿。修复未降低阈值、未删断言、未改 PRD/合同、未动 QA 文件（命令 1–11）。
- 无新增缺陷。本回合非阻断观察见"未覆盖边界"3/6（锚点缺失语义、无基线 diff），不构成 OPEN 缺陷。

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实 30 分钟墙钟**仍未等待：预算判定用 `ManualClock` 推进（120 次轮询/1800s 的确定性证据 + QA 的边界/复算断言）；真实时间下的长跑未做（成本高、需外部供应商配合）——与回合 10 相同。
2. 真实 Tripo/Manual AI 协议与费用（T12/T14/T23）未验证；本回合所有 HTTP 均发往本机 T05 fixture，0 次付费调用。
3. **锚点缺失形态**：若远端等待阶段返回 `WaitingProvider` 而同一 job **没有任何** accepted 远端事实，实现按 0s 起算（不触发预算）→ 理论上可无限等待。真实适配器必须先有 task ID 才能查询（QA 的 (d) 用例即证明无 task ID 时不下发请求），故该形态在真实链路上不可达；仍建议 T12 把"等待起点 = accepted 提交事实"写进适配器契约（回合 10 非阻断建议 3）。
4. `needs_input` 之后"用户补齐→继续"的服务端入口属 T15/T17；本回合用原始 SQL 模拟该入口，只验证"同一 task ID 只查询、零重购、预算已越界则立即回到 `needs_input`"。
5. `reconcile`/`retry`/`cancel` HTTP 端点、任务中心 UI、费用预留/结算（T15/T17/T11）不在本回合；多进程真机竞争、blob/draft 断点同回合 10 仍未构造。
6. **无法对 `jobs_recovery.rs` 做逐字节 diff**：该文件 untracked 且无回合 10 基线副本（回合 10 的 manifest-after 早于 RD 改动）。替代证据 = 22 条原用例名完整 + 关键数值断言逐条复核 + 全量通过；"断言被静默删除/弱化"的残余风险以此方式覆盖，不能声称逐字节等价。
7. 未对 `crates/core/src/jobs.rs` 做 diff（同因）；其边界语义（`>=`、1800s）与常量经源码直读 + 边界用例独立复核。

## 非代码知识与限制（跨卡复用）

1. **修复类复验的"判别力"检查**：仅重跑新用例不够，须判断"断言是否只能由修复后的行为满足"。本回合的最强判据是**可复算的中间量**——越界 note 中的 `1811s` 必须等于"提交事实起点 + 轮询节奏累加"，锚点取错（None/行时间戳/阶段 attempt）都无法得到该值；另用"回拨行时间戳 40 分钟不影响判定"做反证。
2. **清单（manifest）必须在最后一次写入之后生成**：回合 10 的 `source-manifest-after.txt` 早于其自身末次编辑（09:18 vs 09:20），导致哈希对不上，复验时要额外解释。后续 QA：生成清单用一个原子步骤（写文件后立刻哈希），并在报告中注明"清单时间 vs 文件 mtime"。
3. **执行器对"被领取阶段自身存在未决 attempt"有短路**（`executor.rs:433`：`Submitting`/`Unknown` resume → 直接落 `submission_unknown`，不调处理器）。设计锚点/等待类边缘用例时要绕开该形态，否则测到的是另一条（正确的）安全路径；阶段级"非 accepted 事实"的反例应改用 accepted-but-无远端 ID 的 receipt。
4. 崩溃注入、迁移链、failpoint 门控手法同回合 10（未变；本回合 failpoint 正对照复现相同的 3/3/…/2 + `nm` 1251 计数）。
5. 以上 1–3 已追加到 `llmdoc/decisions.md`「T10 验收知识（回合 11 复验）」。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–9 | 2026-09-12 | slice | T01–T09 | 1/2 | PASS | 本文件上半部分 |
| 10 | 2026-09-12 | slice | T10 | 2（ui_revision 2） | **FAIL** | 回合 10 节 |
| 11 | 2026-09-12 | slice | T10（BUG-003 修复复验） | 2（ui_revision 2） | **PASS** | 本节 |

- 缺陷交接：**BUG-003 → CLOSED**（无需再交 RD）。若后续（T12 真实协议）发现新的等待语义问题，按新缺陷流程记录，不重开本条。
- 交接给协调者：(a) `qa_result: PASS`、`open_defects` **清空**（BUG-003 CLOSED）；(b) `qa_history` 追加回合 11（result: PASS，scope: slice，task_ids: [T10]）；(c) T10 建议进入 `accepted_tasks`：`{task_id: T10, prd_revision: 2, qa_round: 11, accepted_ac_ids: [AC-035, AC-036, "AC-037/AC-038 执行器侧", "T10 卡内项 + 回归"], status: accepted}`；(d) QA 新增 `crates/server/tests/qa_t10_r11_verify.rs`（4 用例，无 ignore，已入 `cargo xtask check`/workspace 全量）；(e) `qa_t10_independent.rs` 的复现用例按派发约束**保留 `#[ignore]`**（其合同覆盖已由 (d) 承接）；若要移除 ignore 属新派发，本回合不做。
- 全项目状态提醒：本判定仅覆盖 T10 切片；T11–T23（费用、真实适配器、任务中心 UI、备份、单二进制正式 smoke 等）尚未实现/验收，不得由本回合 PASS 推断整体 MVP 完成。
- 证据目录：`artifacts/web-mvp/t10-qa-r11/`——`qa-jobs-recovery-r11.log`、`qa-independent-standard.log`、`qa-bug003-reverify.log`、`qa-r11-verify-final.log`（+ run1/run2）、`qa-workspace-tests-r11.log`、`qa-xtask-check-r11.log`（首轮失败留档）、`qa-xtask-check-r11b.log`、`qa-dist-r11.log`、`qa-smoke-bootstrap-r11.log`、`qa-realshape-probe-r11.log`、`qa-failpoint-gate-dist-r11.txt`、`qa-failpoint-gate-testbin-r11.txt`、`source-manifest-r11.txt`（200 文件 sha256 收尾快照；含全部 llmdoc，在本报告与 decisions 落笔后生成）。

---

# 回合 12 · T11（输入快照、报价、预算与幂等）

**结果：FAIL（仅 T11 切片范围）** · 回合：12 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T11]；scope=slice）

- QA 执行时间：2026-09-12 10:20–10:40（本地 UTC+8；日志内时间戳为 UTC 02:2x–02:3xZ）；执行者：QA 子 agent（回合 12）。
- 独立性声明：`implementation.md` §T11 与 `artifacts/web-mvp/t11-rd/**`（含 `smoke-manual.sh/.log`）**只作线索，未用作任何通过证据**。本回合结论全部来自 QA 现场执行：
  (a) **QA 自写** `crates/server/tests/qa_t11_independent.rs`（13 用例，不引用 RD 的 `generation_requests.rs` 的任何代码；金额按价格目录单价与自选页输入**手工复算**，不抄 RD 常量）；
  (b) **QA 自写**二进制脚本 `artifacts/web-mvp/t11-qa/qa-smoke-t11.sh`（另一端口 18331、另一套数据；含 HTTP 层 20 次重放、未知字段拒绝、**服务重启后重放**、**改价 + 重启后的 version 拒绝**、进程外连检查）与 `qa-t11-migration-chain.sh`（合成 v5 库 → check 只读 → serve 升 v6）；
  (c) QA 独立重建 dist（**与 RD 同哈希**，见环境表），failpoint 门控独立核对（dist `strings`/`nm` + 测试二进制正对照）。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）= 派发包；AC-028–AC-034 在修订 2 中均为 **[必选]**，未被降级；`state.yaml` qa_round=11、current_tasks=[T11]、open_defects=[]。
- **结论摘要**：AC-028–AC-034 的**全部 API 语义子句**与卡内项在真实环境独立复现**通过**（含 20 次重放、10 路并发同键、6 路并发同报价、断点事务中断、真实二进制重启后重放、价格版本变更）；**但发现 1 个未关闭缺陷 BUG-004（P2）**——本卡新增的回读路由 `GET /items/{id}/estimates/{quoteId}` 与本卡自己交付的机器合同（OpenAPI：返回"含…确认/消费状态"）不符：确认/消费后仍返回 `confirmedAt=null`、`consumedAt=null`、`consumedJobId=null`。按派发规则"当前切片必选 AC 未全部通过**或**有未关闭验收缺陷，不能 PASS"，本回合判定 **FAIL**，交 RD 修复后由 QA 复验（回合 13）。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD 仍 `cb69afb`（"init"），T01–T11 未提交；QA 现场 `find`（mtime 晚于 RD 交付窗口 10:22）显示：crates/apps/contracts/migrations/xtask/tests 下**唯一新增源码文件 = QA 的 `qa_t11_independent.rs`**，`apps/web/dist/**` 为 `cargo xtask dist` 重建产物（gitignore 覆盖） |
| 平台／工具链 | macOS Darwin 25.6.0 · aarch64-apple-darwin；rustc/cargo 1.98.1；node v26.0.0；npm 11.12.1；SQLx 0.9.0 |
| 交付产物 | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`3b02e24a3bc18c321d4859516157791dbc8c05c56c5f900533a4340773b52fdb`**（15 996 928 B）——**QA 独立重建，与 RD §T11-6 报告值逐字节一致**（旁证：确定性构建；且 QA 新增测试文件不影响 release 目标，等价于生产源码自 RD 构建以来未被改动）；build-info `features=["embedded-ui"]`、`schemaVersion: 1`、commit `cb69afb` dirty |
| 数据／fixture | 临时 data-dir（QA 自建，退出即删，无 `/tmp/em-t11-qa-*`、`/tmp/em-t11-smoke-*` 残留；`pgrep` 无残留服务进程）；全部为本机 fixture 与假凭据 canary（`canary-tripo-key-do-not-leak` / `canary-manual-ai-key-do-not-leak`）；**0 次真实付费调用** |
| 未验证 | 真实 Tripo/Manual AI 协议、真实 token 计量与账单（T12/T14/T23）；AC-030/AC-033/AC-034 的 **UI 侧**（Playwright `import-flow.spec.ts`、`job-recovery.spec.ts` 与 UI-022–UI-028 页面，属 T16/T17；当前 `/import/*` 仍是"尚未实现"占位）；`GET /jobs` 费用展示端点（T15/T17）；多管理员（单管理员模型） |

## 命令与原始结果（QA 现场执行，2026-09-12；日志在 `artifacts/web-mvp/t11-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test generation_requests` | exit 0，**23 passed / 0 failed / 0 ignored**（非空转：23 条用例名齐全；`generation-requests-qa.log`） |
| 2 | `cargo test -p everything-manual --test qa_t11_independent`（**QA 自写**，13 用例） | exit 0，**12 passed / 0 failed / 1 ignored**（ignored = BUG-004 复现用例；`qa-t11-independent-final.log`） |
| 3 | 同上 `-- --ignored --nocapture qa_defect_get_estimate_reflects_confirmation_and_consumption` | **exit 101（复现 BUG-004）**：`AFTER_CONFIRM: GET confirmedAt=null（确认响应="2026-09-12T02:32:42.186Z"）`；`AFTER_JOB: GET consumedAt=null consumedJobId=null（真实 job=01a09375-bdce-…）`（`bug004-repro.log`） |
| 4 | `cargo test --workspace`（冻结最终文件后复跑） | exit 0，**303 passed / 0 failed / 3 ignored**（RD 291 + QA 新增 12；3 ignored = QA 回合 10 复现 + QA 回合 12 复现 + 既有 T07 doctest；`workspace-tests-final.log`；更早一轮 `workspace-tests.log` 结果相同） |
| 5 | `cargo xtask check`（冻结最终文件后复跑） | exit 0，**7/7 `[通过]`** + `全部检查通过。`（`xtask-check-final2.log`）。注：更早一轮 `xtask-check-final.log` 失败项是 **QA 自己新文件**的 clippy `too_many_arguments`，修正后全绿——旁证 check 不短路隐藏失败，且生产代码未被 QA 触碰 |
| 6 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`（`contracts-check.log` + 命令 5 内） |
| 7 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0；sha256 **`3b02e24a…`（与 RD 同哈希）**、binary/SHA256SUMS/licenses.json/build-info.json 在场 |
| 8 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` ×2 | 两次均 exit 0；各 **7 项 `[检查]`** 全过（`smoke-bootstrap-1.log`、`smoke-bootstrap-2.log`） |
| 9 | failpoint 门控（QA 现场）：`strings`/`nm` on dist + 测试二进制正对照 | dist：`generation_after_reserve_before_commit` **0**、`EM_TEST_FAILPOINT` **0**、`job-failpoints` **0**、`nm` failpoint 符号 **0**；正对照（`generation_requests` 测试二进制）：断点名 **2**、`EM_TEST_FAILPOINT` **1**、`nm` **133**（`failpoint-gate-dist.txt`、`failpoint-gate-testbin.txt`）——检查非空转 |
| 10 | `bash artifacts/web-mvp/t11-qa/qa-smoke-t11.sh <dist>`（**QA 自写**，真实二进制 + curl + sqlite3） | **exit 0**（`qa-smoke.log`）：非法价格目录 `check`/`serve` 均 **exit 3**；正常 `check` exit 0 且报 `已就绪（v6…）`；estimate 201 且 QA 手算上界 `19742 = 1550+8192+10000` 与响应一致、DB 此时 `cost_ledger=0/jobs=0/attempts=0/quotes=1`；未确认提交 **422 `confirmationRequired`**；未知费用字段 **422**；**20 次同键重放全部 202 且同一 job id**（首次无重放头、后续 `x-idempotent-replay: true`）；同键不同 body **409 `IDEMPOTENCY_CONFLICT`**；库内 `jobs=1/snapshots=1/cost_ledger=2(reserved)/幂等记录=1/quotes 已消费=1/audit=[confirmed,job_created]/provider_attempts=0`；快照 `photo_hashes` 与 QA 独立计算的图片内容 sha256 一致、`provider_config` 无密钥；**服务重启后同键重放仍同一 job**；**改 version + 重启后旧报价提交 422 `priceVersionChanged` 且不建单**；服务进程**无非 loopback 连接** |
| 11 | `zsh artifacts/web-mvp/t11-qa/qa-t11-migration-chain.sh <dist>`（**QA 自写**） | exit 0（`qa-migration-chain.log`）：全新库 6 行迁移、每行校验和 = 对应迁移文件的 SHA-384（算法反证）；合成 v5 库 → `check` 报 **"待迁移（库 v5 → 程序 v6）"且只读**（前后均 5 行、无 quotes 表）；`serve` → `schemaVersion=6`，`quotes` 表 + `quotes_item/quotes_expires` 索引 + 0006 的 **3 个触发器** + `cost_ledger_active_reservation` 唯一索引出现，`PRAGMA integrity_check=ok`、`foreign_key_check` 空；升级后 `check` 报"已就绪（v6）" |
| 12 | `cargo tree -p everything-manual --edges normal`（＋源码 grep） | 生产依赖树**无 reqwest/ureq/curl/rustls/native-tls**，`hyper-util` 仅 `server` 特性；`crates/{server,core}/src` 无出站 HTTP 用法——"不产生远端请求"在 T11 是**结构性**成立，而非仅靠计数（`dep-tree-no-client.txt`） |

## AC 验收矩阵（本切片派发范围；QA 独立复现）

| AC | 期望 | 实际（QA 独立复现） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| **AC-028** | 分列金额（creditMinor/usdMicros）、价格版本与快照日期、保守上界、expiresAt 默认 10 分钟；不调用生成服务、不产生费用记录 | QA 自选页输入（1 页文字 3000B + 1 页扫描 + 1 页文字 1500B）**手算**：输入上界 1200+3000+3000+1500=8700 → 2175 micros、输出 4096 → 8192、页图 1 张 → 10000，合计 **20367**；响应 `manualAi.upperBoundMinor=20367` 与 `upperBoundLines` 三项分项和**逐项一致**；`tripo.currency="creditMinor"`、`upperBoundMinor=3000`、display `"30.00 credits"`；`priceVersion/priceSnapshotDate=2026-09-11`；`expiresAt − createdAt = 600_000 ms`（QA 自算）；响应与 `quotes.quote_json` **逐值同源**、GET 回读一致；估计后 `cost_ledger=0/jobs=0/provider_attempts=0/audit_events=0/quotes=1`；响应与落库文本均不含 canary 密钥 | **PASS** | 命令 2；`qa-t11-independent-final.log` |
| **AC-029** | 缺价格 → 409 `PRICE_CATALOG_MISSING`；未 ready/缺视图 → 422 具体缺项；不返回伪造精确金额 | ① 未配置目录 → 409 `PRICE_CATALOG_MISSING` + `details.missing`，响应无金额字段、`quotes=0`；② 缺 Provider（manual_ai 无密钥/模型）→ 409 `PROVIDER_NOT_CONFIGURED` 且 `missing` 指向 `manual_ai.*`；③ 目录价与 `providers.tripo.model` 不一致 → 409（`tripoModelMismatch`）；④ 未知预设 → 422 `modelPresetUnsupported` + `supportedPresets=[tripo-h-v3.1-standard]`；⑤ 准备未完成 + 缺侧视图 → 422 `preconditionsFailed` 一次列出 `preparationNotReady`+`missingSideView` 与 `presentViews`；仅 left → `missingFrontView`；detail 混入 → `detailViewNotAllowed`；以上均 `quotes=0` 且**任何失败响应都不含金额** | **PASS** | 命令 2 |
| **AC-030** | 未确认提交被拒（不建 job）；确认页数据齐全；确认写 `audit_events`；不默认勾选 | ① 未确认提交 → 422 `confirmationRequired`，`jobs=0/cost_ledger=0/audit=0/attempts=0`；② 报价创建时 `confirmedAt=null`（**不存在隐式确认路径**）；③ `confirm` 200 返回 `confirmedAt` + `sendScope`（Tripo 视图 2 条含 photoId+sha256、模型名与参数、Manual AI 页 1–3/文字页 [1,3]/页图页 [2]/物品名称+型号、价格版本、`plannedUpperBound`）；④ `audit_events` **恰 1 条** `generation_send_scope_confirmed`（actor=管理员，metadata 含 tripoViews/priceVersion/upperBound，无密钥），重复确认幂等（时间不变、审计仍 1 条）；⑤ 过期报价**不允许确认**（422 `quoteExpired`） | **PASS（API 语义层）**；UI/Playwright 部分属 T16 | 命令 2/10 |
| **AC-031** | 同键 20 次 → 1 job（首次 202、重放同 id）；同键不同 body → 409；job 与预留同事务；事务中断不留半笔预留 | ① **现场 20 次**（进程内 + 真实二进制两处独立跑）：全部 202、**同一 job id**、20 次后 `jobs=1/snapshots=1/cost_ledger=2/idempotency_records=1/provider_attempts=0/audit=2`（重放不重复审计），重放响应预留内容与首建一致；② 同键不同 body → **409 `IDEMPOTENCY_CONFLICT`** + `reason=idempotencyKeyReused` + `existingResourceId=原 job`，仍 1 job；③ **并发现场**：10 路同键 **全部 202 且同一 job**；6 路不同键抢同一报价 **恰 1 个 202**，其余 422 `quoteAlreadyUsed`，`ledger=4`（每 job 2 笔）；④ **事务中断**（断点 `generation_after_reserve_before_commit`，owner=本请求键）：请求 panic 后 `jobs/snapshots/cost_ledger/job_stages/idempotency_records=0`、报价未消费，清断点后同键重试成功（首建、无重放头）；⑤ 真实二进制**重启后**同键重放仍返回同一 job（`jobs=1`）；⑥ 幂等记录行 = `admin_id + POST + /api/v1/items/{id}/jobs + key`（路由模板，不按物品隔离——跨物品复用同键 → 409） | **PASS** | 命令 2/10 |
| **AC-032** | 过期报价/输入已变/预算不足 → 拒绝且不建 job、无远端请求；服务端不用前端费用数值 | ① 过期报价（服务层以 700 秒前时刻创建、HTTP 用当前时间提交；另在真实二进制上以"改价 + 重启"路径验证版本语义）→ 422 `quoteExpired`，`jobs=0/ledger=0/attempts=0`；② 报价后**替换 front 照片资产（photoId 不变、内容变化）** → 422 `inputChanged`（T07 QA 前置约束生效）；③ 价格版本变化 → 422 `priceVersionChanged`（服务层注入改版 Settings；真实二进制上"改文件+改 version+重启"同样 422 且不建单）；④ 上限低于上界 → 422 `budgetBelowPlannedUpperBound` 且 `details` 给出两侧数值（3000 / 20367）；⑤ 宽松上限（99999/9999999）→ 202，但账本 `reserved` 仍 = **服务端上界**（3000/20367），jobs 里 `reservations` 分列带 currency/state；⑥ 请求体注入 `tripoCreditMinor` / `textureQuality` 等 → **422**（`deny_unknown_fields`，服务端结构上无费用/质量字段可传） | **PASS** | 命令 2/10 |
| **AC-033（服务端侧）** | credits 与 USD 分列、不相加、无单位数字；显示已消耗/预留/unknown 仍保留的预留 | 报价与建单响应均分列给出 `currency`（`creditMinor`/`usdMicros`）+ 整数最小单位 + display；`budgetNotice` 含"不是供应商账户级硬封顶"；预留状态机现场验证：`reserved→unknown` 后 **reserved 金额不变、`actual` 保持 NULL**、`holds_budget=true`；直接 SQL 把 unknown 的 `actual` 填 0 **被 CHECK 拒绝**；`unknown→settled`（对账口径，2 950 micros）**同值幂等、改值拒绝**；`reserved→released` 幂等且 `actual` 仍 NULL（不是 0） | **PASS（服务端侧）**；页面文案属 T17 | 命令 2 |
| **AC-034（服务端侧）** | 预算用尽/需更高成本 → 拒绝并说明；不自动降质量/换模型/加阶段；重生成需新报价确认；unknown 不填 0 | ① 超上界 → 422 且文案含"不自动降质量/换模型"；② 请求体无质量/模型/阶段字段（结构性），带 `textureQuality` → 422 且不建 job；③ 报价消费后再提交（新键，等价"重生成"）→ 422 `quoteAlreadyUsed` + `details.jobId`；④ 自动释放路径对 `unknown` 返回 `Rejected`（必须走对账），unknown 的 `actual` 永不写 0（用例 + CHECK 双证） | **PASS（服务端侧）**；UI 呈现属 T17 | 命令 2/10 |

### 卡内项（T11 卡 + 派发包附加项）

| 卡内要求 | 结果 | 证据 |
| --- | --- | --- |
| 快照输入不可变且**不含 API key** | **PASS**：`UPDATE generation_snapshots …` 被触发器拒绝；`provider_config` 仅含模型/参数（`tripo.model=v3.1-20260211`、`faceLimit=100000` 等），canary 密钥与 `api_key/secret` 字样均 0 命中 | 命令 2/10 |
| 快照含 `photo_ids+hashes`（T07 前置约束） | **PASS**：槽位顺序 front→left；`photo_ids` 与 `photo_hashes` 一一对应；哈希 = QA **独立计算的图片内容 sha256**（进程内与真实二进制两处均核对） | 命令 2/10 |
| 报价一次性消费 | **PASS**：`consumed_at/consumed_job_id` 只允许 NULL→值（触发器；清除/改写被拒）；新键复用同报价 → 422 `quoteAlreadyUsed` + `jobId` | 命令 2/10 |
| 价格目录版本变更处理（`priceVersionChanged`） | **PASS**：服务层注入版本差异 → 422；真实二进制"改价 + 改 version + 重启"→ 422 且不建单（旧报价必须重报） | 命令 2/10 |
| 幂等键作用域（admin+POST+路由模板+key） | **PASS**：DB 行逐字段核对（`admin_id/method=POST/route=/api/v1/items/{id}/jobs/key/body_hash/resource_id/response_status=202`）；跨物品复用同键（body 不同）→ 409 `IDEMPOTENCY_CONFLICT`（命名空间不按物品切分）；缺失键 → 422 字段级 | 命令 2 |
| 回归：workspace／check／合同／dist+smoke／schema v6 迁移链 | **PASS**：命令 4–11 全部 exit 0；v5→v6 迁移链由 QA 合成库独立验证（只追加、check 只读、3 触发器 + 1 部分唯一索引 + quotes 表出现、integrity/外键检查干净） | 命令 4–11 |

## 缺陷

### BUG-004 · `GET /items/{id}/estimates/{quoteId}` 不反映确认/消费状态（与自身 OpenAPI 合同不符）

- 严重度／状态：**P2 / OPEN**（修复后由 QA 复验）
- 对应 REQ / UI / AC：REQ-020/REQ-021/REQ-022 的**回读语义**；contracts.md §3 该行（"读取既有报价（确认页回显用）"）；UI-024（确认后显示"已确认发送范围（时间）"）、UI-026（"该操作已存在一个任务"并链接已有 job）；本卡 API 合同 `contracts/openapi.json` 中该路由 description："返回报价载荷（含分列金额、保守上界、发送范围、expiresAt 与**确认/消费状态**）"、`QuoteDto.consumedJobId` 描述："消费它的任务 id（UI 用于'该操作已存在一个任务'的链接）"
- 环境与输入：临时 data-dir（QA 自建）；真实 SQLite；进程内 HTTP 与真实二进制（`3b02e24a…`）**两处**均可复现；canary 假凭据、0 付费调用
- 复现步骤：
  1. 建物品 → PDF 准备 ready → front+left 照片 → `POST /items/{id}/estimates`（201）；
  2. `POST /items/{id}/estimates/{quoteId}/confirm` → 200，响应 `data.confirmedAt="2026-…Z"`（同一时刻 DB `quotes.confirmed_at` 有值）；
  3. `GET /api/v1/items/{id}/estimates/{quoteId}` → **`confirmedAt` 仍为 null**（期望等于首次确认时间）；
  4. 继续 `POST /items/{id}/jobs`（202，建单且 `quotes.consumed_at` 有值）后再次 GET → **`consumedAt=null`、`consumedJobId=null`**（期望分别为消费时间与 job id）。
- 命令：`cargo test -p everything-manual --test qa_t11_independent -- --ignored --nocapture qa_defect_get_estimate_reflects_confirmation_and_consumption`（用例 `crates/server/tests/qa_t11_independent.rs`；`#[ignore]` 措辞见下）
- 期望与实际：期望 GET 如实反映 DB 事实（合同明说"含确认/消费状态"）；实际 GET 恒返回**创建时冻结**的 `quote_json`（`crates/server/src/http/estimates.rs::get_estimate` 直接 `quote_payload(&record)`），`confirmed_at/consumed_at/consumed_job_id` 三列**从不合并进响应**。原始输出：`AFTER_CONFIRM: GET confirmedAt=null（确认响应="2026-09-12T02:32:42.186Z"）`、`AFTER_JOB: GET consumedAt=null consumedJobId=null（真实 job=01a09375-bdce-…）`；断言失败 `left: Null, right: String("2026-09-12T02:32:42.186Z")`。
- 影响：确认页刷新后（该路由的存在理由）界面会显示"未确认/未消费"——UI-024 的"已确认发送范围（时间）"丢失、UI-026 期望的"链接已有任务"拿不到 `consumedJobId`（只能靠被拒提交的 `details.jobId` 兜底）；前端若按机器合同生成类型并信任该字段，会得到**与 DB 相反的状态**（"状态即事实"原则的反例）。**无重复付费/数据损坏风险**：服务端提交校验用的是列而非载荷（`confirmationRequired`/`quoteAlreadyUsed` 均正确拒绝），确认 POST 幂等返回首次时间。
- 建议修复方向（不限定实现）：`get_estimate`（或 `quote_payload`）在读回后以 `QuoteRecord.confirmed_at/confirmation_json/consumed_at/consumed_job_id` 覆盖 DTO 对应字段（`quote_json` 仍保持冻结、不被回写）；修复后 GET 对"未确认/未消费"仍为 null（不得引入隐式确认）。
- 回归范围：`GET estimates/{quoteId}` 的四个状态字段；`confirm` 幂等；`POST jobs` 的状态校验（不得因回读改动而改变拒绝语义）；`estimate` 响应与 `quotes.quote_json` 的同源断言（落库载荷仍必须冻结）。
- RD 修复摘要引用：待填（RD）。
- QA 复验结果与日期：待复验（回合 13）。

> 复现用例放在 QA 自写文件内并标 `#[ignore = "BUG-004 复现：GET 报价不反映确认/消费状态（RD 修复后由 QA 复验）"]`：标准 `cargo xtask check` / `cargo test --workspace` 保持全绿，同时可随时 `-- --ignored` 单跑取失败证据（命令 3）。本回合 workspace 全量中共 3 条 ignored：QA 回合 10 复现用例、本条、既有 T07 doctest。

## 非阻断建议（不计入阻断项，不要求 T11 返工）

1. **P3｜预留数组顺序在两条路径不一致**：首建响应按写入顺序 `[tripo, manual_ai]`，重放路径按 `provider` 排序 `[manual_ai, tripo]`（内容相同、金额相同）。不是合同承诺，但 UI 若按数组索引渲染会产生顺序跳变。建议两路径统一（如都按 provider 排序）。QA 用例已改为按集合比较。
2. **P3｜RD 文档计数不一致**：`implementation.md` §T11-2 写"T11 集成测试（**21 用例**）"，而 §T11-6 第 1 条写 **23 passed**，实际为 **23**。建议顺手修正文本。
3. **P3｜报价无上限累积**：每次 estimate 都写一行 `quotes`（无清理/归档，RD §T11-7 第 10 条已声明）。单管理员场景量级可接受，建议在 T15/T17 排期时确认是否需要归档策略。

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实 Provider 与真实计费未验证**（T12/T14/T23）：本卡不产生任何付费提交（`provider_attempts` 恒 0），所有断言基于配置价格与 fixture 数据；"上界与真实账单的偏差"属 T23。
2. **AC-030/AC-033/AC-034 的 UI/Playwright 侧未执行**：派发范围为本卡 API 语义；`import-flow.spec.ts` / `job-recovery.spec.ts` 与 UI-022–UI-028 属 T16/T17（当前 `/import/*` 仍是"尚未实现"占位）。AC-030 的 Playwright 子句因此 **NOT_RUN（超出本切片）**，不得据此推断前端已完成。
3. **10 分钟墙钟过期未等待**：过期语义用服务层注入过去时刻、真实二进制用"改价 + 重启"路径验证；真实时间流逝下的过期未做（成本高、无必要）。
4. **`cost_ledger_active_reservation` 部分唯一索引无 API 可达场景**：一份报价只能建一份任务，当前 API 无法对同一快照重复预留；该约束只验证了"索引在场"与 RD 单测，未构造真实并发触达（属 T15 重试/替换流程）。
5. **并发为同进程内多请求**（10 路同键 / 6 路同报价，共享 SQLite 池）：两个真实进程同时抢同一行未构造（SQLite 写串行 + 唯一键语义与 T10 QA 结论一致）。
6. **`GET /jobs`（已消耗/预留/unknown 展示）端点不存在**：`reservations` 通过建单响应与原始 SQL 验证，任务详情页展示属 T15/T17。
7. **报价过期清理/归档、价格目录热更新**按设计未实现（RD §T11-7 已声明），本回合按现状记录。
8. 断点用例只覆盖本卡新增的 `generation_after_reserve_before_commit`；T10 的五个崩溃断点不在本回合范围（回合 10/11 已验）。

## 非代码知识与限制（跨卡复用）

1. **报价上界必须能"按目录 + 页输入"复算**：说明书 AI 上界口径 = 批次开销 1200 token/批 + 页文字 1 token/字节 + 无文字层页 3000 token/张（页图），输出 = 批次数 × 4096，全部**向上取整**再按单价换算；QA 两次独立手算（20367 = 2175+8192+10000；真实二进制 19742 = 1550+8192+10000）与响应逐项一致。T14 若改发送策略必须同步常量并重验；**评审报价用例时先自己算一遍**，只对照实现常量会漏掉口径错误。
2. **回读类路由的验收必须"先变更状态再读"**：只断言创建时的 null 会让 BUG-004 这类"永远返回冻结载荷"的缺陷看起来通过（本卡 RD 用例与 QA 首版断言都只覆盖了创建态）。凡"回读/回显"合同，测试序列应为：变更（确认/消费/编辑）→ 回读 → 断言反映事实。
3. **测试设计陷阱（QA 自触）**：请求体被 422 拒绝时**不会**消费报价；随后"换新键重提同一报价"会合法地 202。写"重生成必须新报价"用例时，必须先有一次成功建单再重提，否则会误判为产品缺陷。
4. **同一资源的数组顺序可能随路径变化**（首建=写入顺序，重放=DB 排序）：跨路径比较用集合/排序后比较，避免把顺序差异当成重复写入。
5. **failpoint 门控核对沿用"dist 全 0 + 测试二进制正对照"**：本轮新断点名 `generation_after_reserve_before_commit` 在 dist 命中 0、测试二进制命中 2（`EM_TEST_FAILPOINT` 1、`nm` 133），证明检查非空转。
6. **迁移链合成手法可直接复用**：全新 `init` 库的 `_sqlx_migrations.checksum` = 对应迁移文件 **SHA-384 大写 hex**（本轮 6/6 行反证）；据此合成 v5 库 → `check` 只读报"待迁移"（前后行数不变）→ `serve` 升级到 v6。后续 schema 升级（T13/T15/T20）可用同一脚本骨架。
7. **配置类启动失败的口径**：非法价格目录 → `check`/`serve` 均 **exit 3** 且给出具体键名（`unknown field credits…`）；正常目录 `check` 打印 `已就绪（v6；WAL/synchronous/foreign_keys/busy_timeout…）`——运维排障可直接看这一行。
8. **写含中文字符的 shell 脚本要用 `${VAR}`**：`$VAR（` 会被 bash/zsh 当成变量名的一部分（`set -u` 下直接 `unbound variable` 中断，本轮 QA 脚本踩过两次）。证据脚本自带清理 trap 时，`wait "$SERVER_PID"` 也要写成 `${SERVER_PID:-}` 以免提前退出时二次报错。
9. 环境噪音：`~/.cargo/config` 的 deprecated 警告与结论无关；`cargo xtask check` 首轮失败仅因 QA 新文件，不能据此认为 RD 交付未过 check（以最终一轮为准）。
10. 以上 1–9 已追加到 `llmdoc/decisions.md`「T11 验收知识（QA 回合 12）」。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–9 | 2026-09-12 | slice | T01–T09 | 1/2 | PASS | 本文件上半部分 |
| 10 | 2026-09-12 | slice | T10 | 2（ui_revision 2） | **FAIL** | 回合 10 节 |
| 11 | 2026-09-12 | slice | T10（BUG-003 复验） | 2（ui_revision 2） | **PASS** | 回合 11 节 |
| 12 | 2026-09-12 | slice | T11 | 2（ui_revision 2） | **FAIL** | 本节 |

- 交接给 RD（**必须修复后由 QA 复验**）：**BUG-004（P2，OPEN）**——`GET /items/{id}/estimates/{quoteId}` 永远返回创建时冻结的载荷，`confirmedAt/consumedAt/consumedJobId` 三个字段恒为 null，与本卡交付的 OpenAPI 合同（"含…确认/消费状态"）和 DB 事实不符；修复点与建议方向见缺陷段，复现命令见缺陷段命令行（`crates/server/tests/qa_t11_independent.rs::qa_defect_get_estimate_reflects_confirmation_and_consumption`，`#[ignore]`）。修复不得通过改 PRD/合同描述措辞、删除 `QuoteDto` 字段或放宽断言来"转绿"；`quote_json` 的冻结语义与 `POST jobs` 的拒绝语义不得改变。
- 交接给协调者：(a) `qa_result: FAIL`、`open_defects` 追加 `BUG-004（P2，OPEN；对应 REQ-020/021/022 回读语义 + OpenAPI 合同）`；(b) `qa_history` 追加回合 12（result: FAIL，scope: slice，task_ids: [T11]）；(c) T11 **不**进入 `accepted_tasks`；(d) 其余全部结论（AC-028–AC-034 API 语义、卡内项、回归、迁移链、真实二进制冒烟）已独立通过，缺陷修复后只需复验 BUG-004 + 回归（回合 13），**不要求重验全部 AC**；(e) QA 新增 `crates/server/tests/qa_t11_independent.rs`（13 用例，含 1 条 `#[ignore]` 复现；已入 workspace/check 全量）与两个 QA 脚本（`artifacts/web-mvp/t11-qa/qa-smoke-t11.sh`、`qa-t11-migration-chain.sh`）；(f) 顺带建议处理非阻断建议 1/2。
- 全项目状态提醒：本判定仅覆盖 T11 切片；T12–T23（真实适配器、任务中心 UI、阅读器、备份、单二进制正式 smoke、真实 API 授权验收）尚未实现/验收，不得由本回合任何 PASS 结论推断整体 MVP 完成。
- 证据目录：`artifacts/web-mvp/t11-qa/`——`generation-requests-qa.log`、`qa-t11-independent-final.log`、`bug004-repro.log`、`workspace-tests-final.log`（+ 更早 `workspace-tests.log`）、`xtask-check-final.log`（首轮失败留档）、`xtask-check-final2.log`、`contracts-check.log`、`dist-1.log`、`dist-1-sha256.txt`、`smoke-bootstrap-1.log`、`smoke-bootstrap-2.log`、`failpoint-gate-dist.txt`、`failpoint-gate-testbin.txt`、`dep-tree-no-client.txt`、`qa-smoke.log`、`qa-migration-chain.log`、`manifest-qa-r12.txt`；脚本 `qa-smoke-t11.sh`、`qa-t11-migration-chain.sh`。

---

# 回合 13 · T11（BUG-004 修复复验）

**结果：PASS（仅 T11 切片范围；BUG-004 CLOSED）** · 回合：13 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T11]；scope=slice；派发=复验 BUG-004 + 回归）

- QA 执行时间：2026-09-12 10:46–10:52（本地 UTC+8；日志内时间戳为 UTC 02:46–02:51Z）；执行者：QA 子 agent（回合 13）。
- 独立性声明：`implementation.md` §T11-10 与 `artifacts/web-mvp/t11-rd-bug004/**` 只作线索，**未用作任何通过证据**。本回合结论全部来自 QA 现场执行：QA 自写二进制脚本（`artifacts/web-mvp/t11-qa-r13/qa-bug004-four-states.sh`；另一端口 18341、另一套数据、真实二进制 + sqlite3 交叉核对）、QA 自写用例复跑、全量回归、独立重建 dist。
- 派发范围：只复验 BUG-004 + 回归。T11 其余 AC 已在回合 12 独立通过，本回合**不重验、不冒充新证据**（回归命令顺带执行了它们的用例）。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）= 派发包；AC-028–AC-034 仍为 **[必选]**。`state.yaml` 为 qa_round=11 / qa_result=FAIL / open_defects=[BUG-004 fixed_pending_reverify]（QA 不改 state）。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD 仍 `cb69afb`（"init"），T01–T11 未提交；`git status --porcelain` 与回合 12 同形（20 项，无新增生产代码改动；QA 本轮改动仅 `artifacts/web-mvp/t11-qa-r13/**` 与 llmdoc 记录） |
| 修复波及面抽查（mtime > 10:30，排除 node_modules/dist/target） | 仅 3 个文件：`crates/server/src/http/estimates.rs`（10:41）、`crates/server/src/generation/estimate.rs`（10:41，RD 声明仅注释）、`crates/server/tests/generation_requests.rs`（10:38，+2 用例）。**DTO/OpenAPI/schema/迁移均未改动**（由 `contracts --check` 独立复核） |
| 平台／工具链 | macOS Darwin 25.6.0 · aarch64-apple-darwin；rustc/cargo 1.98.1；node v26.0.0；npm 11.12.1；SQLx 0.9.0 |
| 交付产物 | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`324b2fbed9e2f415991bf39555790e09af54c4040d805ef61f16c4c89903c181`**（15 987 440 B）——**QA 独立重建，与 RD §T11-10-5 报告值逐字节一致**（旁证：确定性构建；生产源码自 RD 构建以来未被改动） |
| 数据／fixture | 临时 data-dir（QA 自建，退出即删；`/tmp/em-r13-*` 无残留、无残留进程）；受控假凭据；**0 次真实付费调用**；标准四状态中的"已过期"为 **DB 合成夹具**（见下，明确标注） |
| 未验证 | 真实 Provider／真实计费（T12/T14/T23）；AC-030/AC-033/AC-034 的 UI/Playwright 侧（T16/T17）；`GET /jobs` 展示（T15/T17） |

## 复验要点逐条（派发要求 1–6）

1. **四状态 × DB 交叉核对（真实二进制 + `sqlite3`）**：**PASS**，逐项（命令 9 原始输出）：
   - **S1 未确认未消费**：GET 200，三字段 null；DB 三列（`confirmed_at/consumed_at/consumed_job_id`）NULL（一致）；冻结部分（amounts/expiresAt/priceVersion/sendScope/其余载荷）与创建响应**逐值相等**——无隐式确认。
   - **S2 已确认未消费**：confirm 200（`confirmedAt=2026-09-12T02:50:19.959Z`）→ GET 200，`confirmedAt` 与确认响应**逐字相等**、且等于 DB `confirmed_at=1789181419959` 的 RFC3339 推导值；消费两字段仍 null、DB 两列仍 NULL；冻结部分不变。
   - **S3 已消费**：建单 202（真实 job `01a09385-e1fa-76a3-8532-158ecdb7753d`）→ GET 200，`consumedAt` = DB `consumed_at=1789181420025` 推导值（`…02:50:20.025Z`）、`consumedJobId` = DB 列 = **真实 job id**；`confirmedAt` 保持首次确认值；冻结部分不变；**`quote_json` 未被回写**（sha256 S1=S3=`6980bca7…`；DB 载荷仍等于创建响应、其中三字段仍为 null）；已消费报价新键重提 422 `quoteAlreadyUsed`、jobs 仍 1。
   - **S4 已过期（可读不可用）**：DB 合成过期报价（HTTP 造不出：服务端时钟 + 0006 触发器冻结 `expires_at`；合成保持载荷 `expiresAt` 与列同源）→ GET 200 可读，`expiresAt` = DB 冻结值（`2026-09-12T02:48:20Z`），三字段 null；confirm 与提交均 **422 `quoteExpired`**、DB 三列仍 NULL、jobs 仍 1；响应无派生的"是否可用"字段。**S4b**（已确认+已过期，沿用真实确认列）→ GET 200 仍给出 `confirmedAt` = DB = 首次确认值（事实不因过期消失），消费字段 null。
2. **冻结部分不可变**：**PASS**——四个状态点的响应中金额/上界/priceVersion/priceSnapshotDate/expiresAt/sendScope 均与创建响应逐值相等；`quotes.quote_json` 全程 sha256 不变（未被回读或消费回写）。
3. **可用性与状态分离**：**PASS**——过期报价读得到（200）但 confirm/提交被拒（422 `quoteExpired`）且拒绝后 DB 状态列保持 NULL；S3 消费后回读如实给出消费事实，同时新键提交被 422 `quoteAlreadyUsed` 拒绝——消费状态未被当作"可用性"的替代。
4. **QA 测试文件未被 RD 修改**：**PASS**——`crates/server/tests/qa_t11_independent.rs` sha256 = `87653b7d709b08315f546e489e91fd0c8b224b22a379d1f99915b4dfebcc0f46`，与回合 12 清单 `artifacts/web-mvp/t11-qa/manifest-qa-r12.txt` **完全一致**；mtime 10:29（早于 RD 修复窗口）。ignored 用例由 QA 单跑（`--ignored`），**未改文件**（理由见"决定与理由"）。
5. **回归**：**PASS**——`cargo test --workspace` **305 passed / 0 failed / 3 ignored**（QA 对 22 个批次求和复核）；`cargo xtask check` 7/7 `[通过]` + `全部检查通过。`；`cargo xtask contracts --check` 两份 `[一致]`（**DTO/合同无漂移**，独立复核而非采信 RD）；`cargo xtask dist` 独立重建哈希 = RD 值；`smoke-bootstrap` ×2 各 1 `[准备]` + 7 `[检查]` 全过；failpoint 门控 dist 全 0 + 测试二进制正对照（非空转）。
6. **RD 新增用例的断言强度抽查**：**PASS**——两用例（`get_estimate_reflects_confirmation_and_consumption_from_database`、`get_expired_quote_reports_status_facts_but_stays_unusable`，`crates/server/tests/generation_requests.rs` 2720/2859 行起）均经 `SELECT confirmed_at, consumed_at, consumed_job_id FROM quotes` 读真实列、与 `Timestamp::from_millis(...).to_rfc3339()` 逐字比对；并 `SELECT quote_json` 断言冻结载荷未被回写；过期用例用服务层注入过去时刻构造，断言 `expires_at` = DB 值、拒绝后 DB 列仍 NULL、jobs=0。**不是只断言响应回显。**

## 命令与原始结果（QA 现场执行，2026-09-12；日志在 `artifacts/web-mvp/t11-qa-r13/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test qa_t11_independent` | exit 0，**12 passed / 0 failed / 1 ignored**（13 条用例名齐全；`qa-t11-independent-r13.log`） |
| 2 | 同上 `-- --ignored --nocapture qa_defect_get_estimate_reflects_confirmation_and_consumption` | **exit 0，1 passed**（修复前为 exit 101）：`AFTER_CONFIRM: GET confirmedAt="2026-09-12T02:47:06.065Z"`（= 确认响应）；`AFTER_JOB: GET consumedAt="…"、consumedJobId="01a09382-ec55-70c5-91bd-5892c35dac96"`（= 真实 job）（`bug004-repro-r13.log`） |
| 3 | `cargo test -p everything-manual --test generation_requests` | exit 0，**25 passed / 0 failed / 0 ignored**（含 RD 新增 2 条；`generation-requests-r13.log`） |
| 4 | `cargo test --workspace` | exit 0，**305 passed / 0 failed / 3 ignored**（`workspace-check-contracts-r13.log`；3 ignored = QA 回合 10 复现、QA 回合 12 复现、既有 T07 doctest） |
| 5 | `cargo xtask check` | exit 0，fmt/clippy/test/lint/typecheck/vitest/合同 7 项全 `[通过]` → `全部检查通过。` |
| 6 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`（无 DTO 漂移） |
| 7 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0；sha256 **`324b2fbe…`（= RD 报告值）**；binary/SHA256SUMS/licenses.json/build-info.json 在场（`dist-r13.log`、`dist-sha256-r13.txt`） |
| 8 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` ×2 | 两次 exit 0，各 7 项 `[检查]` 全过（`smoke-bootstrap-1.log`、`smoke-bootstrap-2.log`） |
| 9 | `bash artifacts/web-mvp/t11-qa-r13/qa-bug004-four-states.sh <dist 副本>`（**QA 自写**） | **exit 0**（`qa-bug004-four-states.log`）：S1/S2/S3/S4a/S4b 全部通过（细节见上节）；收口 jobs=1、quotes=3（1 源 + 1 已确认源 + 2 合成过期）、provider_attempts=0、服务进程**无非 loopback 连接** |
| 10 | failpoint 门控（`strings`/`nm` on dist + 测试二进制正对照） | dist：断点名 **0**、`EM_TEST_FAILPOINT` **0**、`nm` **0**；正对照（`generation_requests` 测试二进制）：断点名 **2**、`EM_TEST_FAILPOINT` **1**（`failpoint-gate-r13.txt`）——检查非空转 |
| 11 | 修复波及面抽查（`find -newermt "10:30"`，排除 node_modules/dist/target） | 仅 `http/estimates.rs`、`generation/estimate.rs`、`tests/generation_requests.rs` 三个文件——与 RD §T11-10 声明一致（外加 `contracts --check` 证明 DTO 未漂移） |
| 12 | 源码核对 | `estimates.rs::get_estimate` 合并 3 列（只读）；`quote_payload` 仍为纯解析；`QuoteDto` 无新增字段（无派生"是否可用"字段）；OpenAPI 该路由 description 仍为"含…确认/消费状态" |

## BUG-004 复验结论：**CLOSED**（P2）

- 复验日期：2026-09-12（回合 13）；复验人：QA 子 agent。
- 修复落点核对（读代码）：`crates/server/src/http/estimates.rs::get_estimate` 在返回前用 `QuoteRecord` 当前列覆盖 `payload.confirmed_at/consumed_at/consumed_job_id`（3 行赋值 + 注释），冻结输入仍只来自 `quote_json`；`generation/estimate.rs::quote_payload` 保持"纯解析冻结载荷"并加注释。与 RD §T11-10-3 描述一致。
- 复验证据：命令 1（QA 复现用例转绿）、命令 3（RD 两条 DB 断言用例）、命令 9（真实二进制四状态 + sqlite3 交叉核对，含过期"可读不可用"与 `quote_json` 未回写）、命令 7（哈希一致）。
- 影响面确认（无回归）：confirm 幂等（时间不变、审计仍 1 条）、`POST jobs` 拒绝语义（`confirmationRequired`/`quoteAlreadyUsed`/`quoteExpired`）、`quote_json` 冻结与 `estimate` 响应同源、报价金额与上界冻结——均由本轮命令 1/3/4/9 覆盖通过。
- 是否重开：**否**。后续若任一状态在回读中失真（例如新路由复用 `quote_payload` 而未合并列），按新缺陷记录。

## 决定与理由（QA 自决项）

1. **未解除 `#[ignore]`，文件保持字节不变**：派发允许"解除 ignore 后运行"；权衡后保留原状——(a) 派发同时要求核对文件未被 RD 修改，保持 sha256 与回合 12 清单一致可让后续回合继续使用哈希证据链；(b) BUG-004 的**永久回归守卫**已由 RD 的两条标准用例承担（`generation_requests` 25 passed 中无 ignored），其断言强度经我抽查（真实 DB 列 + 冻结载荷）；(c) QA 复现用例仍可 `-- --ignored` 单跑（命令 2 即此路径）。**未修改文件，无需恢复。**
2. **S4 过期报价用 DB 合成**：HTTP 层无法产生过期报价（服务端时钟 + 触发器冻结 `expires_at`），合成时保持"载荷内 `expiresAt` 与列同源"（与真实过期报价形状一致）。**首轮脚本只改了列、未同步载荷内 `expiresAt`，触发一次夹具自伤断言失败**（`S4 回读 expiresAt 必须 = DB 冻结值`）；修正夹具后全绿。陷阱已记入 `llmdoc/decisions.md`。
3. **不重验 T11 其余 AC**：派发范围为 BUG-004 + 回归；AC-028–AC-034 回合 12 的独立证据仍有效，本回合未重跑，不冒充新证据。
4. **未采信 RD 结论**：RD 报告的命令与哈希均经 QA 独立复跑/重建（命令 1–12），其中 dist 哈希为 QA 自建产物比对，非引用 RD 文件。

## 非阻断观察（不计入阻断项，不改判定）

1. **P3｜`expiresAt` 来自冻结载荷而非列**：GET 的 `expiresAt`（与其余冻结输入一样）取自 `quote_json`；正常路径两者同源写入（命令 9 的 S1/S2/S3 与 RD 用例均验证逐值相等），0006 触发器禁止改列，**当前无发散路径**。若将来出现"只更新列/只更新载荷"的代码路径，会重新出现回读不一致——建议 T15（重试/替换）改动时复核此不变量。
2. **P3｜重放预留数组顺序**（回合 12 建议 1）：仍未处理（首建按写入序 `[tripo, manual_ai]` vs 重放按 provider 排序）。RD §T11-10-6 已声明留待协调者排期；本回合未改变其状态。
3. **P3｜`/tmp` 遗留非本轮产物**：`/tmp/em-t09-*`、`/tmp/em-t10-*`、`/tmp/em-t11-debug` 等（07:47–10:05）为更早回合留下，非回合 13 创建；本轮自建目录/进程已清零（脚本 trap + 手动复核）。

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实 Provider 与真实计费未验证**（T12/T14/T23）；本轮 `provider_attempts=0`。
2. **AC-030/AC-033/AC-034 的 UI/Playwright 侧未执行**（T16/T17）；`/import/*` 仍是"尚未实现"占位。
3. **真实墙钟 10 分钟过期未等待**：过期语义以 DB 合成（真实二进制）+ 服务层注入（进程内）证明。
4. **四状态核对为单管理员、单进程**；多进程并发下的回读未构造（SQLite 单写者语义，与回合 12 结论一致）。
5. **`quote_json` 冻结依赖 0006 触发器 + 服务端只读路径**；直接 DB 写入（如本轮夹具合成）可绕过——属运维/迁移边界，非产品路径。
6. **未重验** T11 其余 AC、v5→v6 迁移链、中断断点等（回合 12 证据仍有效；本回合只跑回归）。

## 非代码知识与限制（跨卡复用）

1. **`time` crate RFC3339 小数位规则（新）**：`to_rfc3339()` 在小数部分为 0 时**整体省略小数**，否则打印**去掉尾随 0** 的最短位数（`…:50.520Z` → `…:50.52Z`；`…:50.000Z` → `…:50Z`）。用"毫秒 → 字符串"复算 DB 值时必须实现同一规则，否则会在尾数为 0 时误报缺陷（本轮脚本首版用固定 3 位，已改）。
2. **DB 合成"过期报价"夹具必须同步 `quote_json.expiresAt` 与 `quotes.expires_at`（新）**：真实路径两者同源写入；只改列会让"回读取载荷值"看起来像缺陷（本轮夹具自伤教训）。脚本 `artifacts/web-mvp/t11-qa-r13/qa-bug004-four-states.sh` 的 S4/S4b 段可直接复用（含载荷内 id 同步替换）。
3. **回读路由的验收序列（回合 12 结论的落地验证）**：状态类字段必须来自持久层当前列、冻结输入来自冻结载荷；验收必须"变更状态再读 + DB 列交叉核对"，仅比响应会漏掉 BUG-004 类缺陷。
4. **修复复验的最小充分集（本轮模板）**：单跑缺陷复现用例（应转绿）→ 读修复点代码 → 四状态 × DB 交叉核对（真实二进制 + sqlite3）→ 全量回归 + `contracts --check` 漂移检查 + 独立重建 dist 比对哈希。避免"只看 RD 的日志"。
5. **含中文字符的 shell 脚本**：`$VAR（` 仍会被解析为变量名的一部分（本轮再次踩到，`${VAR}` 解决）——与回合 12 知识 9 一致，属高频陷阱。
6. 以上 1–5 已追加到 `llmdoc/decisions.md`「T11 验收知识（QA 回合 13 补充）」。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–9 | 2026-09-12 | slice | T01–T09 | 1/2 | PASS | 本文件上半部分 |
| 10 | 2026-09-12 | slice | T10 | 2（ui_revision 2） | **FAIL** | 回合 10 节 |
| 11 | 2026-09-12 | slice | T10（BUG-003 复验） | 2（ui_revision 2） | **PASS** | 回合 11 节 |
| 12 | 2026-09-12 | slice | T11 | 2（ui_revision 2） | **FAIL** | 回合 12 节 |
| 13 | 2026-09-12 | slice | T11（BUG-004 复验） | 2（ui_revision 2） | **PASS** | 本节 |

- 交接给协调者：(a) `qa_result: PASS`（仅 T11 切片）、`qa_round: 13`、`qa_history` 追加回合 13（PASS / slice / [T11]）；(b) **BUG-004 → CLOSED**，`open_defects` 清空；(c) **T11 进入 `accepted_tasks`**：`{task_id: T11, prd_revision: 2, qa_round: 13, accepted_ac_ids: [AC-028, AC-029, AC-030（API 语义侧）, AC-031, AC-032, AC-033（服务端侧）, AC-034（服务端侧）], status: accepted}`——AC-030/033/034 的 UI/Playwright 子句仍属 T16/T17，**未随本卡验收**；(d) 非阻断建议：回合 12 建议 1（预留数组顺序）仍未处理，建议排期；建议 2（计数文本）RD 已在 §T11-2 修正为 23/25。
- 交接给 RD：**无**（BUG-004 已 CLOSED，无待修项）。
- 全项目状态提醒：本判定只覆盖 T11 切片与 BUG-004 复验；T12–T23（真实适配器、任务中心 UI、阅读器、备份、单二进制正式 smoke、真实 API 授权验收）仍未实现/验收，不得据此推断整体 MVP 完成。
- 证据目录：`artifacts/web-mvp/t11-qa-r13/`——`qa-t11-independent-r13.log`、`bug004-repro-r13.log`、`generation-requests-r13.log`、`workspace-check-contracts-r13.log`、`dist-r13.log`、`dist-sha256-r13.txt`、`smoke-bootstrap-1.log`、`smoke-bootstrap-2.log`、`failpoint-gate-r13.txt`、`qa-bug004-four-states.log`、`manifest-qa-r13.txt`；脚本 `qa-bug004-four-states.sh`。

---

# 回合 14 · T12（Tripo v3 HTTP 适配器）

**结果：PASS（仅 T12 切片范围；无新增缺陷）** · 回合：14 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T12]；scope=slice；AC-041 全部子句 + 卡内项 + 三项特别评估）

- QA 执行时间：2026-09-12 11:49–12:03（本地 UTC+8；日志内时间戳为 UTC 03:49–04:03Z）；执行者：QA 子 agent（回合 14）。
- 独立性声明：`implementation.md` §T12 与 `artifacts/web-mvp/t12-rd/**` 只作线索，**未用作任何通过证据**。本回合结论全部来自 QA 现场执行：QA 自写 Python fixture（`qa-tripo-fixture.py`，与 RD 的 `tripo-fixture.py` 相互独立实现，含 10 个场景）+ 自写驱动/验算脚本（`qa-t12-scenario.sh`、`qa-t12-verify.py`、`qa-t12-run-all.sh`）驱动**真实 dist 二进制**（curl 建单 → 后台执行器真实跑链 → `sqlite3` 交叉核对）；QA 自写 Rust 用例 `crates/server/tests/qa_t12_independent.rs`（3 条）；全量回归；**QA 独立重建 dist** 比对哈希。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）= 派发包；REQ-027 / AC-041 仍为 [必选]；AC-042（真实链路）属 T23，本回合未验、不冒充。`state.yaml`：phase=qa_running、prd_revision=2、current_tasks=[T12]、open_defects=[]。
- **state.yaml 两处字段异常（QA 不改 state，交协调者核对）**：(a) `qa_round: 11` 与 `qa_history`（已 13 轮）不一致；(b) `accepted_tasks` 中 T10 条目记为 `qa_round: 14`（应为 11）。
- 本回合不覆盖：全项目其它未实现切片（T13–T23）——T12 通过不代表整体 MVP 完成。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb`（init），T01–T12 未提交；`git status --porcelain` 与回合 13 同形（4 项已跟踪文件 modified + 未跟踪目录），**未发现预期外改动**（QA 新增仅 `crates/server/tests/qa_t12_independent.rs` 与 `artifacts/web-mvp/t12-qa/**`） |
| 平台／工具链 | macOS Darwin 25.6.0 · aarch64-apple-darwin；rustc/cargo 1.98.1 |
| 交付产物 | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`2ec8a45be0f63ef257d886e9b341762d0e8b21f7c6a81907657f763c15067f5d`**（20 880 880 B）——**QA 独立重建，与 RD §T12-7 报告值一致**；`SHA256SUMS` 亦一致（确定性构建旁证） |
| 数据／fixture | 每个场景独立临时 data-dir（脚本 trap + 手动复核，`/tmp/qa-t12-*` 无残留、无残留进程）；受控假凭据（`qa-fake-tripo-key-9931`）；**0 次真实付费调用**；全部 HTTP 目标 = `127.0.0.1` fixture（10 场景 lsof 采样非 loopback = 0） |
| 未验证 | 真实 Provider／真实计费／真实 TLS（AC-042，T23）；UI-038 阶段明细（T17）；`model_download`／`model_validate`（T13）；`GET /jobs` 展示（T15）；连接层超时（TCP connect）未单独触发 |

## 命令与原始结果（QA 现场执行，2026-09-12；日志在 `artifacts/web-mvp/t12-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test tripo_contract` | exit 0，**18 passed / 0 failed / 0 ignored**（`qa-tripo-contract.log`；另直跑测试二进制一次同样 18 passed，`qa-tripo-contract-direct-run.log`） |
| 2 | `cargo test -p everything-manual --test qa_t12_independent`（**QA 新增**） | exit 0，**3 passed / 0 failed**（超时分类 + 每次调用只发一次；重定向不跟随且不转发凭据） |
| 3 | `cargo xtask check`（终版，含 QA 新用例） | exit 0，**7/7 `[通过]` → `全部检查通过。`**；其中 `cargo test --workspace` = **346 passed / 0 failed / 3 ignored**（`qa-xtask-check-final.log`；3 ignored = BUG-003/BUG-004 两条复现用例 + 既有 `items.rs` 文档块 doctest） |
| 4 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`（本卡无 DTO 变更，无漂移） |
| 5 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0，sha256 **`2ec8a45b…`**（= RD 值；`qa-dist.log`、`qa-dist-hash-before/after.txt`） |
| 6 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` | exit 0，1 项 `[准备]` + **7 项 `[检查]`** 全过；日志含 `job_executor_no_handlers` + `provider_not_configured … 不回退 mock`（`qa-smoke-bootstrap.log`） |
| 7 | `bash artifacts/web-mvp/t12-qa/qa-t12-run-all.sh`（**QA 自写；真实 dist 二进制 + QA 自建 fixture**） | exit 0，**10/10 场景 PASS**（逐场景断言 61/41/42/40/40/12/62/40/39/39 项，共 416 项全过；`qa-run-all.log` + 各 `qa-<场景>-run.log`） |
| 8 | lsof 采样（场景 7 中每 0.2s 采样服务进程） | 10 个场景全部 **非 loopback 连接 = 0**（`qa-<场景>-lsof.txt`） |
| 9 | 测试二进制网络观测（`lsof -p <tripo_contract> -a -i -P -n`，0.05s 采样） | 仅 `127.0.0.1` LISTEN（进程内 fixture），无对外连接（`qa-tripo-contract-lsof.txt`） |
| 10 | `cargo test … --test qa_t10_independent -- --ignored` / `--test qa_t11_independent -- --ignored` | 两条过时 ignore 用例单跑均 **exit 0 / 1 passed**（BUG-003/BUG-004 修复在当前工作树仍成立；见非阻断建议 2） |

## AC-041 验收矩阵（逐子句；命令 ↔ 证据）

| 子句 | 期望 | 实测（QA 现场） | 结论 |
| --- | --- | --- | --- |
| 上传 multipart 字段 `file` | `POST /v3/files`，`Content-Disposition: form-data; name="file"` | QA fixture 逐字节解析 multipart：唯一 part 字段名 = `file`，filename=`front.jpg`/`left.png`，Content-Type=`image/jpeg`/`image/png`，图片字节 sha256 = 源文件（happy/disconnect/…全部场景） | PASS |
| Authorization | `Bearer <配置键>`；日志/记录脱敏 | 线上头 = `Bearer qa-fake-tripo-key-9931`（= 配置值）；serve 日志 0 次出现该键；RD 用例断言记录中为 `Bearer [REDACTED]` 且 Debug 不含 canary | PASS |
| token opaque（不 trim/截断） | 原样保存并用于提交 | `token_verbatim` 场景：token=` TOKEN with Spaces_1 ` 在库内与提交体中逐字符一致 | PASS |
| 请求体 `inputs`（front + 侧视图 view-key） | `[{"front":…},{"left":…}]` | happy 场景：`inputs` 恰 2 项且为 front/left；缺失方向不出现（back/right 未选则无对象） | PASS |
| 冻结参数 | `model=v3.1-20260211`、texture/pbr=true、quality=standard、`face_limit=100000`、quad=false、generate_parts=false | happy：逐字段相等，且**顶层字段集合恰为冻结的 9 项**（无多余字段） | PASS |
| 无 v2 字段形态 | 无 `model_version`/`files`/`type` | 全部 10 场景提交体逐条反断言 = 0 命中 | PASS |
| HTTP 200 但 `code != 0` | 业务失败（读 message/suggestion、可证明未被接受） | `business_error` 场景：submit=failed、attempt=failed、last_error 含 `code=1201`+`invalid image token`、账本 `released`、付费 POST 计数 1（不重发） | PASS |
| success 缺可下载模型 | 不算成功、保留 task_id、不重购 | `no_model` 场景：poll=`retry_wait`、`rawStatus=success`、usage 无 `modelUrl`、付费提交仍 1 次、账本保留 `reserved` | PASS |
| 未知状态保留原值 | 原值落库 + 可诊断等待 | `unknown` 场景：`rawStatus=qa_new_state_zz`、`normalizedStatus=unrecognized`、poll=`waiting_provider`（6 次查询后仍未猜测） | PASS |
| 429 退避 | 可证明未被接受 → 退避重试、预留保留 | `submit_429`（Retry-After: 1）：提交 2 次（同 body hash）、attempt#1=failed+429、attempt#2=accepted、最终 settled=3000 | PASS |
| POST 已被接受后断连不自动重发 | 结果未知、绝不重发 | `disconnect`：付费 POST 计数 **1**；attempt=`unknown`（无 task ID）、stage=`submission_unknown`、账本 `unknown`（actual NULL）；**强行改回 queued + SIGKILL 重启后仍为 1**；无任何查询请求 | PASS |
| 任务查询失败 ≠ 生成失败 | 退避重试、不改变购买事实 | `poll_503`：第 1 次查询 503 → 重试 → 成功；付费提交 1 次、最终 settled=3000（另 RD 用例覆盖 429/`Retry-After` 截断） | PASS |
| credits 精确解析与记录 | 十进制字面量 → `creditMinor`，来源字段落库 | happy：`literal="30"`→`creditMinor=3000`、`sourceField=credits_consumed`、`currency=credit_minor`、账本 settled/actual=3000；`no_billing`（无计费字段）→ 阶段照常成功但**不结算**（reserved、actual NULL，不填 0） | PASS |
| 错误信息脱敏 | 不含完整签名 URL 查询串与密钥 | serve 日志 `modelUrl=https://cdn.example.invalid/qa/model.glb?[redacted]`、签名 canary 0 次、测试键 0 次；DB 中 `modelUrl` 保留完整签名（供 T13 下载用，属设计） | PASS |
| 连接/整体超时明确 | 显式超时并生效 | 代码：connect 10s / request 60s / upload 180s；QA 新用例用 300ms 超时 + fixture 持连验证**超时确实触发**且归类 `Transport`（不可证明未被接受）、一次调用只发 1 请求 | PASS |
| 任务查询失败的分类边界 | 5xx/传输 → Retryable；3xx → 失败（不跟随重定向） | QA 新用例：302 不跟随、不把 bearer 带到 Location、只发 1 次请求、`is_definitively_refused()=true` | PASS |
| 与执行器集成（不改变 T10 语义） | 注册 3 阶段、E2E 可达、恢复语义不变 | `tripo_executor` 注册恰 3 阶段（不注册 model_download/manual_extract）；10 场景全部经真实执行器跑链；`jobs_recovery` 25 passed、`qa_t10_r11_verify` 4 passed、BUG-003 复现用例单跑通过 | PASS |
| 零真实外网 | 测试进程与手工端到端均无外呼 | lsof 采样（服务进程 10 场景 + 测试二进制）：非 loopback = 0；fixture 只绑定 `127.0.0.1`；全部 base_url 来自 fixture | PASS |

## 三项特别评估（派发要求逐项给结论）

### 评估 1：官方文档不可达导致的形态假设风险 —— **结论：当前容错是"安全方向"，不构成 T12 阻断；T23 收敛点明确，建议再加两条守卫**

- 事实核对：`developers.tripo3d.ai` 在本机不可达（RD 记录 + 本回合验证期间未发起任何对该域的请求）；字段形态全部依据项目内 2026-09-11 冻结核对记录（`contracts.md` §6、`architecture.md` §5.3）。**QA 逐字段核对：适配器发出的请求体与 `contracts.md` §6 的冻结示例逐字段一致**（模型/参数/`inputs` 扁平形态/路径），即"实现与冻结合同一致"这一 T12 可验收的基准成立；"冻结合同本身是否等于线上真实形态"属 T23 范围。
- **容错不会把真实失败当成功**（QA 用两个场景独立验证）：
  - 上传 token 只认 `file_token`/`image_token` 两个候选；返回其它字段（`token_unknown` 场景）→ 上传阶段 `retry_wait` 且 last_error 明示"候选 file_token/image_token、dataKeys=[token_value]"、**付费 POST 计数 = 0**——是可见失败，不是静默成功。
  - success 判定要求 `code==0` **且** `output.model_url` 非空；缺模型 → `retry_wait`（`no_model` 场景）。
- **容错不会把非 token 字段当 token**：候选名单封闭（各 2 个），不模糊匹配；`opaque_string` 只接受字符串/数字原文本；实际来源字段 `tokenField`/`sourceField` 落库可审计。
- **容错不会伪造金额**：无计费字段 → 不结算（`no_billing` 场景：`reserved`/actual NULL）；无法精确解析 → `billingProblem` + 不结算（RD 单测）。
- 残余风险与建议：(a) 若线上真实字段名不在候选内，表现为**可见失败 + 人工诊断**（不会错账），但会让首真实链路失败一次——T23 应有预算/时间应对；(b) 建议在 T23 交接清单中显式列出待收敛项（上传 token 字段名、`inputs` 扁平 vs 嵌套形态、计费字段名、状态全集），并在收敛后**删掉多余候选**（缩小歧义面）；(c) 无需在本卡加更严格守卫——本轮已证明"认不出即失败"，再加严格性只会增加误伤风险。

### 评估 2：单图 `image-to-model` 未实现 —— **结论：与当前范围一致，不是本卡遗漏、不是缺陷**

- 逐条核对：PRD REQ-027 的输入是"多视图图片（front + ≥1 侧视图）"；architecture §5.3 首版流程为"上传 → **多视图**生成 → …"，并明确"缺照片要求用户补齐，**不用 AI 造图代替实物照片**"；`validation-release.md` §3 Tripo 行与 T12 卡（implementation-plan）均只列多视图；PRD §2.2 非目标未列"单图模式"，也从未承诺。→ 单图模式属**未立项能力**，RD 不做符合"不顺手扩展范围"的约束（ADR-022 第 9 条已记录）。
- 落点建议（非阻断）：若产品要单图路径（含"是否允许用 AI 造图补缺失视图"的告知问题），应由 PM 开新 REQ + UI 交互 + 新切片；不得由 RD 在 T13/T15 顺手加入。**本项记录为"范围外"，不计缺陷。**

### 评估 3：付费 POST 不重发的守卫强度 —— **结论：当前构建下多层守卫成立；发现一处"依赖 feature 开关的潜在陷阱"，记 P3 非阻断建议**

逐层核对（读源码 + 字节级证据）：

1. **应用层**：`providers/tripo/client.rs` 无重试层（无 tower 中间件、无库内重试）；`submit_multiview` 一次调用只 `execute` 一次；执行器仅在 `resume_hint` 判定"无未决事实"时才调用处理器，`submitting/unknown` 一律直接落 `submission_unknown`（`jobs/executor.rs`）；处理器另有"accepted 但无 task ID → 拒发"的防御；`begin_intent` 拒绝在存在未对账 attempt 时新建。
2. **reqwest 0.13.5 层（关键发现）**：该版本**默认挂了一个 tower 重试层**（`retry::Builder::default()`：`Classifier::ProtocolNacks`、`max_retries_per_request=2`、无 scope/budget）。但其唯一判定函数 `is_retryable_error()` 的所有 `true` 分支都在 `http2`/`http3` feature 内——本项目以 `default-features=false`（json/multipart/stream/rustls）构建，**该层恒不重试**。旁证：`h2` 不在 Cargo.lock；`hyper` 只启 `http1`；dist 二进制 `strings`/`nm` 无 h2/`REFUSED_STREAM`。**若将来为性能启用 `reqwest/http2`，默认策略会重试 h2 `GOAWAY(NO_ERROR)`/`REFUSED_STREAM`**——这是唯一能绕过应用层守卫的路径，故记 P3 建议（显式 `retry(reqwest::retry::never())` + 注释）。
3. **hyper-util 层**：`retry_canceled_requests`（默认 true）只在"请求尚未序列化到连接"（`take_message()` 语义：字节从未写出）且连接为复用连接时重发一次——**不会复制一条服务器已收到的请求**；请求已写出后连接断开走 `TrySendError::Nope`，不重试。
4. **执行器重试路径**：仅"可证明未被接受"的错误允许新 attempt（429/4xx/业务 code/3xx）；本回合 `submit_429` 场景实测"429 → 新 attempt → 成功"恰好 2 次付费 POST，且同 body hash；`disconnect`/超时 → 无第二次 POST（含强改队列 + SIGKILL 重启）。

## 缺陷

**无**（本回合未发现 T12 范围内的功能缺陷；未关闭验收缺陷 = 0）。以下 3 条为非阻断建议，不要求 T12 返工。

## 非阻断建议（P3，供排期/后续卡处理，不改变本次 PASS）

1. **P3｜付费 client 显式关闭 reqwest 默认重试策略**：`TripoClient::new` 加 `retry(reqwest::retry::never())`（reqwest 0.13.5 提供），并在代码注释/ADR 记录"付费 client 不得启用 http2/http3 默认重试"。理由见评估 3 第 2 层；当前无实际违规（feature 未启用），属"消除未来误开启的陷阱"。落点：T13（同 client 复用下载需独立 client）或 T15（retry/reconcile 定稿时）。
2. **P3｜两条过时 `#[ignore]` 应转正**：`qa_t10_independent.rs` 的 BUG-003 复现、`qa_t11_independent.rs` 的 BUG-004 复现，两缺陷均已 CLOSED，本轮 `--ignored` 单跑**均通过**（exit 0）。建议去除 ignore 作为常驻回归守卫（保留即可继续被 3 ignored 计数掩盖）。本回合未改文件（保持既有哈希证据链），落点：协调者排期或下一位 QA 顺手处理。
3. **P3｜同内容双视图会提交同一 token 两次**：上传按内容哈希缓存（ADR-022 第 5 条），若用户把同一张照片同时用作 front 与 left（两个 photo 行指向同一 asset），提交体将出现同一 token 两次。当前 API 可达性低（T11 禁止重复 photoId，但未禁止两行同 asset），影响为供应商业务错误（可见失败、单次付费）。**未构造端到端复现（本回合未验证）**；建议 T23 若遇到再决定是否加 `needs_input`。

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实 Provider／真实协议／真实计费未验证**（AC-042 属 T23）：字段名、`inputs` 形态、TLS/区域地址、429/`Retry-After` 真实形态、真实 credits 与上界偏差均待 T23。
2. **UI-038（阶段明细）未验**：属 T17；本回合未构造任何前端证据。
3. **T13/T14/T15 能力**：`model_download`/`model_validate`/说明书 AI/`GET /jobs` 未实现——Tripo 链成功的场景里父 job 因 manual 分支延后保持 `running`（预期；未假成功），失败/未知场景如实为 `failed`/`submission_unknown`/`waiting_provider`。
4. **TCP 连接层超时未单独触发**（只验证了"整体/读超时"）；慢连接/黑洞 IP 未构造（避免任何非回环流量）。
5. **`modelUrl` 出网脱敏**：当前无任何 HTTP DTO 暴露 `usage_json`（仅 `POST /items/{id}/jobs` 存在，实测确认），T15/T17 上线后需按 §T12-6 约束复核。
6. **崩溃断点注入（PAID_* failpoints）未在本回合重跑**：属 T10 证据范围（回合 11 已验证）；本回合用"强改队列 + SIGKILL"覆盖了恢复语义的最终防线。
7. **"success 缺模型"退避耗尽 → `failed`**：本回合观察到 `retry_wait`（真实时钟窗口内），耗尽的终态由 RD 进程内用例（`success_without_model_is_not_success_and_does_not_repurchase`）覆盖，未在真实二进制上等待 ~62s 退避跑满。

## 非代码知识与限制（跨卡复用）

1. **reqwest 0.13 默认重试层与 feature 门控**（新，重要）：默认 `ProtocolNacks` 策略只在启用 `http2`/`http3` 时可能生效；本项目未启用故为死代码，但**启用即改变付费安全边界**。验证手法：`cargo tree -e features -i reqwest` + `Cargo.lock` 无 `h2` + dist `strings/nm` 反证。已记入 decisions.md（T12 验收知识）。
2. **hyper-util `take_message()` 语义**：返回 message ⇔ 请求未序列化到连接；付费安全性论证必须用它区分"未发出的请求"与"可能已被接受的未知结果"。
3. **QA 10 场景 fixture 清单可直接复用于 T13/T23**（脚本 + 断言中文注释齐全）：`artifacts/web-mvp/t12-qa/qa-tripo-fixture.py`、`qa-t12-verify.py`；T23 只需改 base_url + 预算文件。
4. **"容错方向"验收模板**：对"候选字段名容错"类设计，必须用"非候选字段"与"缺可选字段"两个场景分别证明不伪造成功/不伪造金额（本轮 `token_unknown`/`no_billing`）。
5. **lsof 采样必须写 `-a`**（`lsof -p PID -a -i -P -n`），否则 `-p` 与 `-i` 为 OR 关系；含中文字符脚本的 `$VAR（` 陷阱本轮未再踩（统一 `${VAR}`）。
6. 以上已追加至 `llmdoc/decisions.md`「T12 验收知识」；未改动 `validation-release.md`（命令合同无变化）。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–13 | 2026-09-12 | slice | T01–T11 | 1/2 | PASS（10/12 FAIL → 修复后 PASS） | 本文件上半部分 |
| 14 | 2026-09-12 | slice | T12 | 2（ui_revision 2） | **PASS** | 本节 |

- 交接给协调者：(a) `qa_result: PASS`（仅 T12 切片）、`qa_round: 14`、`qa_history` 追加回合 14（PASS / slice / [T12]）；(b) `open_defects` 仍为空；(c) **T12 进入 `accepted_tasks`**：`{task_id: T12, prd_revision: 2, qa_round: 14, accepted_ac_ids: [AC-041（fixture/协议侧全部子句）], status: accepted}`——AC-041 的 UI-038 展示子句（未知状态原值显示、429 退避展示）属 T17，**未随本卡验收**，届时按 T17 重验；(d) **state.yaml 两处字段异常**请协调者核对修正（`qa_round` 落后、T10 条目 `qa_round: 14`）。
- 交接给 RD：**无必须修复项**；3 条 P3 建议（显式 `retry::never()`、过时 ignore 转正、同内容双视图边缘）由协调者决定排期，不阻断 T13。
- 交接给 PM：无需求歧义需裁定（单图 `image-to-model` 属范围外，若要做请开新 REQ）。
- 全项目状态提醒：T12 PASS **只覆盖 Tripo v3 适配器切片**（fixture 协议层）；T13–T23（下载/校验、说明书 AI、组装、任务中心、阅读器、备份、单二进制正式 smoke、真实授权链路）仍未实现/验收。
- 证据目录：`artifacts/web-mvp/t12-qa/`——QA 脚本 `qa-tripo-fixture.py`、`qa-t12-scenario.sh`、`qa-t12-verify.py`、`qa-t12-run-all.sh`；日志 `qa-run-all.log`、`qa-<场景>-run.log`（10 场景）、`qa-<场景>-fixture.jsonl`（字节级请求记录）、`qa-<场景>-serve.log`、`qa-<场景>-lsof.txt`、`qa-<场景>-manual.sqlite3`、`qa-tripo-contract.log`、`qa-workspace-tests.log`、`qa-xtask-check-final.log`、`qa-dist.log`、`qa-dist-hash-before/after.txt`、`qa-smoke-bootstrap.log`、`qa-tripo-contract-direct-run.log`、`qa-tripo-contract-lsof.txt`；QA 新用例 `crates/server/tests/qa_t12_independent.rs`。

---

# 回合 15 · T13（模型下载、校验与不可变版本）

**结果：PASS（仅 T13 切片范围；无新增缺陷，3 条 P3 观察）** · 回合：15 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T13]；AC-043、AC-044 全部子句 + 卡内项 + 回归链）

- QA 执行时间：2026-09-12 12:39–12:52（本地 UTC+8）；执行者：QA 子 agent（回合 15）。
- 独立性声明：`implementation.md` §T13 与 `artifacts/web-mvp/t13-rd/**` **只作线索，未用作任何通过证据**。本回合结论全部来自 QA 现场构造与执行：
  - **QA 自写 15 条集成用例** `crates/server/tests/qa_t13_independent.rs`（GLB 负例字节全部手写、不调用 `test_support::generate::build_glb`）；
  - **QA 自写原始 TCP fixture**（`QaServer`，只用 `std::net`，逐请求记录方法/target/全部请求头/连接数，可脚本化 302／chunked／半关闭／断连），**不复用 T05 `FixtureServer` 场景与 RD 的 `model-fixture.py`**；
  - **QA 自写发布门禁脚本** `artifacts/web-mvp/t13-qa/qa-t13-release-gate.py`（自带 fixture + 断言，独立于 RD 的 `verify-release-gate.sh`）；
  - QA 独立重建 dist、独立跑全量回归与 schema／lsof 观测。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）＝ 派发包；REQ-028 / AC-043 / AC-044 仍为 [必选]。`state.yaml`：phase=qa_running、prd_revision=2、current_tasks=[T13]、open_defects=[]、qa_round=15。
- 本回合不覆盖：T14/T15/T17/T18/T19/T20/T22/T23 的能力（说明书 AI、组装草稿、任务中心 UI、浏览器渲染校验、发布、备份、正式 smoke、真实授权链路）。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb`（init；T01–T13 仍未提交）。进入时 `git status --porcelain` 与回合 14 同形（4 项已跟踪 modified + 未跟踪目录）；**QA 新增仅** `crates/server/tests/qa_t13_independent.rs` 与 `artifacts/web-mvp/t13-qa/**`，退出时 `git status` 无预期外改动 |
| 平台／工具链 | macOS Darwin 25.6.0 · aarch64-apple-darwin；rustc/cargo 1.98.1；python3 3.14.2；node v26.0.0 |
| 交付产物 | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`24264bafb40aa5b1d737909b73549d8d066ccf142971403def8ba48383ab81c2`**（21 196 208 B）——**QA 独立 `cargo xtask dist` 重建，与 RD §T13-7 报告值一致**（确定性构建旁证） |
| 数据／fixture | 每个用例独立临时 data-dir（自动清理，实测无残留）；全程受控假凭据（`qa13-*`、`qa-gate-*`）；**0 次真实付费调用**；全部 HTTP 目标 = 本机 fixture（`127.0.0.1:随机端口`） |
| 零外网观测（QA 自己采样） | ① QA 用例二进制：`lsof -p PID -a -i -P -n` 62 次采样，非 loopback **0**（`qa-test-lsof.txt`）；② RD 套件 `model_assets` 二进制：145 次采样，非 loopback **0**（`qa-model-assets-lsof.txt`）；③ 发布门禁中的 `serve` 进程：采样期间非 loopback **0**（`qa-gate-report.json.non_loopback_connections=[]`） |
| 未验证 | 真实 CDN／真实签名 URL 形态与 TLS 链（AC-042/T23）；生产 **https 成功下载**未端到端跑过（见"未覆盖边界"）；GPU 可绘制性（T18）；UI-039 文案（T17） |

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/t13-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test qa_t13_independent`（**QA 新增**） | exit 0，**15 passed / 0 failed / 0 ignored**（`qa-t13-independent.log`；`--nocapture` 一轮含 P3 观察输出） |
| 2 | `cargo test -p everything-manual --test model_assets`（RD 套件，基线） | exit 0，**22 passed / 0 failed / 0 ignored**（`qa-model-assets-baseline.log`） |
| 3 | `cargo xtask check`（终版，含 QA 新用例） | exit 0，**7/7 `[通过]` → `全部检查通过。`**；`cargo test --workspace` = **391 passed / 0 failed / 3 ignored**（= RD 基线 376 + QA 15；3 ignored 仍为 BUG-003/BUG-004 复现用例 + `items.rs` doctest）（`qa-xtask-check-final.log`） |
| 4 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`（本卡无 DTO 变更）（`qa-contracts-check.log`） |
| 5 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0，sha256 `24264baf…`（= RD 值；`qa-dist.log`、`qa-dist-hash.txt`） |
| 6 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` | exit 0，1 项 `[准备]` + 7 项 `[检查]` 全过（`qa-smoke-bootstrap.log`） |
| 7 | `cargo test -p everything-manual --test tripo_contract` / `--test qa_t12_independent` | **18 passed** / **3 passed**（T12 语义未因 `retry::never()` 变更或阶段注册数 3→5 而破坏；`qa-tripo-contract.log`、`qa-t12-independent.log`） |
| 8 | `python3 artifacts/web-mvp/t13-qa/qa-t13-release-gate.py --binary <dist> …`（**QA 自写发布门禁**） | exit 0，**25/25 项通过**（`qa-release-gate.log`、`qa-gate-report.json`、`qa-gate-fixture.jsonl`、`qa-gate-serve.log`）：误配 `allow_local_fixture=true` 的发布二进制 → `needs_input`（`download_insecure_scheme`）、fixture **0 次**模型请求、付费提交恰 1 次、`model` 资产 0、`model_revisions` 0、日志 `download_local_fixture_ignored`、日志无签名 canary |
| 9 | 发布二进制 schema 核对（QA 脚本：`init` + `serve` + `sqlite3`） | 6 个迁移（0001…0006）全部 success；`model_revisions` 来自 **0001**（CHECK `pending/validated/rejected`、FK→items/assets/provider_attempts、`model_revisions_item` 索引）；共 22 张表；`migrations/` 无 T13 新迁移（`qa-schema-check.log`） |
| 10 | `download.allowed_hosts` 配置负例（发布二进制 `check`） | 默认空名单 → `allowed_hosts=[(空：拒绝一切下载；未配置允许域时不猜测供应商 CDN 域名)]`；`Cdn.Example.COM` → 归一化 `cdn.example.com`；带 scheme／通配符／端口 → exit 3 并指认；未知键 → exit 3（`qa-download-config.log`） |
| 11 | `cargo tree -p everything-manual -e features -i reqwest` + 源码 grep | reqwest 仅 json/multipart/stream/rustls（**无 http2/http3**，`h2` 不在 Cargo.lock）；`download.rs:469` 与 `client.rs:222` 均有 `retry(reqwest::retry::never())`；`download.rs:466` `redirect::Policy::none()`；全仓库无 `default_headers`，bearer 只在 `client.rs:293` 逐请求 `bearer_auth` |

## AC-043 验收矩阵（逐子句）

| 子句 | 期望 | 实测（QA 现场，全部为 QA 自建用例） | 结论 |
| --- | --- | --- | --- |
| 下载 client 不带 API Authorization | CDN 请求无 `authorization`/`cookie`/`x-api-key` 等 | `qa_pin_ip_keeps_hostname_and_sends_no_credentials`：CDN 请求头集合里 6 个敏感头全无；**同一 fixture 上 `TripoClient::get_task` 记录到 `authorization: Bearer qa13-canary-api-key`（对照证明凭据只在 API client）**；`qa_valid_model_creates_immutable_validated_revision` / `qa_over_budget_…` 亦逐请求断言 CDN 无凭据 | PASS |
| HTTPS + 允许域 | 空名单拒绝一切；未命中拒绝；非法配置报错 | `qa_dns_rebinding_mixed_answers_and_address_classes_are_refused`：空名单 → `download_host_not_allowed`；`qa_download_config.log`：非法形态 exit 3 | PASS |
| 每跳重定向与实际连接 IP 校验 | 302 → 私网/非允许域/非 http(s) 必须拒绝且不连接 | `qa_redirect_negatives_never_reach_the_next_hop`：302→`http://10.1.2.3/x.glb`（允许域含该 IP）→ `download_forbidden_address`（消息含"私网地址"）、第 2 跳 **0 连接**；302→未允许域 → `download_host_not_allowed`、第二 fixture **0 连接**；302→`ftp://` → `download_insecure_scheme`；302→`file://` → fail-closed 拒绝（实测 `download_invalid_url`）；302 无 Location → `download_bad_redirect`；300 → `download_http_status` 不跟随；自循环 → `download_too_many_redirects` 且**恰好 6 个请求**（上限 5 跳）；全部路径 0 blob、0 tmp 残留 | PASS |
| 拒绝私网/回环/链路本地 | 私网/链路本地任何配置都拒绝；回环仅两道门同时成立 | `qa_dns_rebinding_…`：注入解析器 → `10.1.2.3` 拒绝（0 连接）；混合答案 `[93.184.216.34, 192.168.1.9]` 整次拒绝；链路本地/CGNAT/组播/文档保留/基准测试/保留网段逐一拒绝且消息含对应分类；生产策略 `https://127.0.0.1` → `download_forbidden_address`（含"回环地址"）；`local_fixture_allowed()` 在"测试构建+显式配置"下 true、关掉配置即 false | PASS |
| 流式大小与 sha256 校验 | 落盘字节 = 响应字节、sha256 一致；两道大小门 | `qa_pin_ip_…`：sha256 与 QA 字节一致、文件存在、仅 1 次请求；`qa_size_gates_truncation_retry_and_disk_full`：声明长度 8 MiB>1 KiB 上限 → `download_too_large(declared=Some)`；无 Content-Length 的 chunked 8 KiB>1 KiB → `download_too_large(declared=None)`（流中计数第二道）；磁盘满注入 → `download_insufficient_storage`；以上均 0 blob / 0 `.part` | PASS |
| GLB 检查（magic/version/长度/chunk/JSON/accessor 边界/内嵌资源/扩展） | 每项有稳定错误码；拒绝不改字节 | 4 条 GLB 用例（QA 手写字节）：容器（magic/version=1,3/声明长度±4/头不完整/JSON 长度非 4 倍数/JSON 长度 0/首 chunk 非 JSON/第三 chunk/JSON 不可解析/JSON 非对象）、布局（count 越界/未对齐/byteStride 3 与 4/视图越 buffer/buffer 越 BIN/两个 buffer/引用 buffer 1/引用不存在视图/无 bufferView/componentType 5127/type VEC5/sparse/无 BIN chunk）、几何（缺 POSITION/VEC2/count 0/无索引 4 顶点/mode=1/mode=7/无 mesh/索引 componentType BYTE/NaN/Infinity/索引 9 ≥ 4）、资源（buffer 与 image 的 https 与 `data:` 外链/无 bufferView/webp MIME/非 PNG 数据/PNG 尺寸 0/required extension/asset.version≠2.0）全部按预期码拒绝；`qa_budget_negatives_and_original_file_is_untouched`：面数与贴图超预算 → `gltf_face_limit`/`gltf_texture_limit`（`is_budget()`），**失败后文件 sha256 与字节不变** | PASS |
| 不可变 model revision + 临时 URL 不当永久地址 | validated revision 含 asset/sha/bounds；临时 URL 不落成永久地址 | `qa_valid_model_creates_immutable_validated_revision`：下载→校验成功；`usage` 与 DB 交叉：`validated` 一行、`bounds` = QA 字节 AABB（min[-1,-1,0]/max[1,1,0]/triangles=2/vertices=4/maxTextureDimension=8）、关联付费 attempt、`asset(purpose=model)` 指向同一 blob、blob 字节逐字节一致；**重跑校验不产生第二行、bounds 不变、不重新下载**；全库文本列扫描签名 canary 仅命中 `job_stages.usage_json`（且 `stage_kind=tripo_poll`，见 P3-1） | PASS |

## AC-044 验收矩阵（逐子句）

| 子句 | 期望 | 实测 | 结论 |
| --- | --- | --- | --- |
| 链接过期 → 重新查询已知任务取新链接、不重新购买 | 403/404/410 触发 `GET /tasks/{id}`；付费 POST 计数不增 | `qa_refreshed_link_is_revalidated_and_never_repurchases`：过期链接请求 1 次得 403 → **第 2 次任务查询** → 用新链接 → **新链接仍做允许域校验**（指向未允许域 → `needs_input download_host_not_allowed`）；CDN 连接仅 1 次；任务查询 ≥2 次；**付费提交恒 1 次**；API 侧只出现"上传/提交/查询"三类请求；无 model 资产、无 revision、无 tmp 残留 | PASS |
| 断连可安全重试下载 | 半关闭截断 → 可重试分类；重试整文件重下 | `qa_size_gates_…`：半关闭截断 → `download_transport`（`is_retryable()`）、0 blob、0 tmp；重试成功且 sha256 正确；**第 2 次请求不含 `range` 头**（整文件重下）；断连（不发任何字节）同样归类可重试 | PASS |
| 超面数 / 外链 buffer-image / 不支持扩展 / 截断 GLB → `needs_input` + 保留原始模型与错误，不静默改坏、不自动降预算 | 缺项码稳定、文案可行动、原件保留 | `qa_over_budget_preserves_original_bytes_and_never_repurchases`（执行器链路，注入 2 面上限 1）：`needs_input` 含 `gltf_face_limit` + "原始模型已保留" + "未自动降预算"，**无"自动修复"入口**；blob 字节 = 服务端字节；`rejected` revision（bounds NULL）指向该 blob；CDN 仅 1 次请求；付费提交 1 次；重跑校验不产生第二行；`qa_truncated_glb_at_handler_level_keeps_original`：截断 GLB（下载成功、内容损坏）→ `needs_input`（`glb_declared_length`/`glb_chunk_layout`）+ 原文保留 + 不重下不重购；外链/扩展/sparse 的稳定码另由 GLB 层 4 条用例覆盖 | PASS |
| 执行器路径的 DNS 重绑定 | 允许域解析到私网 → 拒绝、0 连接、不重购 | `qa_rebinding_at_executor_level_blocks_download`：注入解析器把允许域映射到 `10.1.2.3` → `needs_input download_forbidden_address`；CDN fixture **0 连接**；付费提交 1 次；无 model 资产 | PASS |

## 卡内项与回归（T13 卡 + validation-release §3 下载行 + architecture §7）

| 检查项 | 实测 | 判定 |
| --- | --- | --- |
| bearer 不泄露到 CDN（fixture 请求头断言） | 见 AC-043 第 1 行；含"API client 带 bearer"对照 | PASS |
| DNS 重绑定防护（连接 pin 到已验证 IP 且保留 hostname/SNI） | **Host 头可观察证据**：允许域名 `pin.qa13.test` 经注入解析器 → 连接落到本机 fixture，收到的 `Host: pin.qa13.test:<port>`（原域名保留），且该 `.test` 保留域在系统解析下不可解析——连接成功本身就证明"用了验证过的 IP、没有再次用系统解析"。**TLS SNI 未用 TLS fixture 直接观察**（见未覆盖边界） | PASS（SNI 记为未直接观测） |
| Redirect 关闭或逐跳验证 | `redirect::Policy::none()` + 6 条重定向负例（含"第 2 跳 0 连接"） | PASS |
| `retry::never()` 已落实 | 下载 client 与付费 client 各 1 处显式调用；无 http2/http3 feature（默认重试层恒为死代码）；`tripo_contract` 18 + `qa_t12_independent` 3 仍全绿（付费不重发语义未破坏） | PASS |
| 测试放行需要"测试构建+显式配置"两道门；发布构建误配也不放行 | 测试构建侧：`local_fixture_allowed()` 随配置开关变化、回环/http 仅在开启时可用；**发布构建侧（QA 自写脚本）**：dist 二进制 + 误配 `allow_local_fixture=true` → `needs_input download_insecure_scheme`、fixture 0 次模型请求、模型 0 资产、0 revision、付费仍 1 次、日志 `download_local_fixture_ignored` | PASS |
| 原件保留策略可查 | `rejected` revision + blob 保留 + "原始模型已保留"文案（两条链路实测）；RD 记录见 §T13-5/§T13-8（本次未复核文档措辞之外的实现差异） | PASS |
| 不可变版本幂等（`(item_id, sha256)`） | 重跑校验/重跑下载均不产生第二行、不改 bounds、不重下（QA 实测两处：rejected 与 validated 各一次） | PASS（并发竞态见 P3-3） |

## SSRF / GLB 负例的独立性说明（本回合构造了什么、观察到什么）

1. **自建原始 TCP fixture（不复用 T05 场景设施）**：`QaServer` 只绑定 `127.0.0.1:0`，逐请求记录 `method/target/全部请求头/到达序号`，并把**连接数**与**请求数**分开计数——因此"第 2 跳 0 连接"是直接观测而不是推断。可脚本化响应含 302／`transfer-encoding: chunked`／半关闭截断／不发字节直接断开。
2. **302 → 私网**：把 `10.1.2.3` **同时**放进允许域名单（最严苛形态：只剩地址校验这一道防线）→ 仍拒绝 `download_forbidden_address`，且被指向的第二 fixture 连接数 0。
3. **DNS 重绑定**：注入自写解析器（`QaResolver`）分别返回"私网唯一答案""公网+私网混合答案""空解析"，并把**允许域映射到回环**做正对照——后者成功连接证明实现把连接 pin 到"解析后校验过的 IP"，而不是再解析一次。
4. **逐跳上限**：同一路径自循环 302 → 观察**恰好 6 个请求**（`max_redirects=5` + 第 6 次的拒绝），证明不是"解析 Location 就跟随"。
5. **截断与两道大小门**：半关闭截断（有 Content-Length 但只写一半）、chunked 无声明长度超限、声明长度超限、磁盘满注入——四种形态都断言 0 blob / 0 `.part`，并在重试请求上断言**无 `range` 头**。
6. **GLB 负例字节全部手写**：`qa_png_ihdr`/`qa_bin`/`qa_assemble` 由 QA 写码生成（4 顶点 2 三角面 + 33 字节 IHDR），因此"正常样例通过"先行确认，再逐项注入 30 余种畸形；其中 NaN/Infinity/索引越界是直接改 BIN 位模式。
7. **发布门禁**：QA 自写 Python（自带 fixture + 断言 + `lsof` 采样），与 RD 脚本仅共享产品公开 API 的调用顺序；观察点是"fixture 请求日志 + DB 计数 + serve 日志事件"三处交叉。

## 缺陷

**无**（本回合未发现 T13 范围内的功能缺陷；未关闭验收缺陷 = 0）。以下 3 条为 P3 观察，**不阻断 T13，也不要求返工**。

### P3-1｜临时签名 URL 永久留在 `tripo_poll` 阶段事实中（T12 设计，T13 不扩大）

- 严重度／状态：P3 / OPEN（跨卡，不是 T13 缺陷）
- 对应 REQ / AC：REQ-028 / AC-043（"临时供应商 URL 未被当作永久地址保存"的解释边界）
- 环境与输入：QA 自建链路（`qa_valid_model_creates_immutable_validated_revision` 等）；全库文本列扫描 canary
- 期望与实际：期望"临时 URL 不作为永久地址"；实际 T13 所属位置（`assets`、`model_revisions`、`model_download`/`model_validate` 的 `usage_json`）**0 命中**（已逐条断言），唯一命中是 `job_stages.usage_json` 的 **`tripo_poll` 观察事实**（保存完整签名 URL 供下载阶段取用；DB 中签名整串长期保留）。
- 影响：非 AC-043 违规（模型版本/资产/发布均指向本地内容寻址 blob），但**保留窗口等于 job 生命周期**；出网脱敏属 T15/T17、导出不得含临时 URL 属 T20（ADR-022 第 8 条、§T13-8 第 9 条已记录）。
- 建议：T15/T17 上线后按 DTO 脱敏复核；若 PM 希望缩短保留，可在模型 `validated` 后把 poll 事实中的 URL 替换为 host 摘要（需新需求，不由 QA 决定）。
- 证据：`qa-t13-independent.log`（canary_hits/usage_hit_stages 输出）、`qa-gate-report.json`

### P3-2｜SOF 落在贴图首 64 KiB 之外的合法 JPEG 会被判"尺寸不可判定"（误拒）

- 严重度／状态：P3 / OPEN（fail-closed，保留原件）
- 对应 REQ / AC：REQ-028 / AC-043（贴图尺寸检查）、§T13-3
- 环境与输入：`qa_jpeg_with_sof_beyond_the_64kib_window_is_rejected`（QA 构造：SOI + 最大 APP1(65533 B) + APP2(998 B) + SOF0，SOF 偏移 **66541** > 读窗口 65536）
- 期望与实际：期望"可被浏览器解码的合法内嵌 JPEG 通过校验"；实际 `gltf_image_invalid`（"尺寸不可判定"）→ `needs_input`。
- 影响：真实供应商产物若带较大 EXIF/ICC 段（>64 KiB 元数据）会被拒；用户侧只能更换资料/重新生成（无自动修复入口，符合"拒绝而不是猜测"的方向）。当前 64 KiB 窗口是代码常量（`HEADER_WINDOW`），未在 §T13-3 能力清单中说明。
- 建议：把该限制写入 §T13-3/§T13-8（文档一致性）；或按 marker 链流式扫描（读到 SOF 或 bufferView 结束）以避免误拒。**属 PM/RD 取舍，不在本回合修**。
- 证据：`qa-t13-independent.log`（println 行）、`crates/server/src/assets/glb/mod.rs`（`HEADER_WINDOW = 64 * 1024`）

### P3-3｜`model_revisions` 无 `(item_id, sha256)` 唯一约束（并发竞态未复现）

- 严重度／状态：P3 / OPEN（理论竞态，本回合**未构造复现**）
- 对应 REQ / AC：REQ-028 / AC-043（"不可变 model revision"）、contracts §2
- 期望与实际：RD 记录"以 `(item_id, sha256)` 幂等"；实测 `get_or_create` 是"事务内 SELECT→INSERT"的**应用层**幂等，schema 只有主键与 `model_revisions_item` 索引，**无 UNIQUE**（`qa-unique-indexes.log`：现有 UNIQUE 只有 `cost_ledger_active_reservation`、`photos_item_view_unique`、`provider_attempts_unresolved_stage`）。同一物品的两个 job 同时对同一内容校验时理论上可产生两行。
- 影响：低（不违反字节不可变；多一行同内容 revision 会干扰 T18/T19 的"选哪一行"）。单 stage 重试路径已实测幂等。
- 建议：若 PM/RD 认为值得收口，在后续迁移加 `UNIQUE(item_id, sha256)`（或文档明确"不保证跨 job 唯一"）；T15 组装草稿取 revision 时应按现有语义（取该 job 的 validated revision）。
- 证据：`qa-unique-indexes.log`、`crates/server/src/storage/repo/model_revisions.rs`

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实供应商与真实 CDN（AC-042 / T23）**：真实 `output.model_url` 域名与签名形态、真实 CDN 是否要求额外请求头（如 Referer）、真实 150 MiB 模型下载耗时与超时取值、`download.allowed_hosts` 的部署值——均待 T23；域名名单**默认空**属设计（`download_host_not_allowed` 明确提示，不猜域名）。
2. **生产 HTTPS 成功路径未端到端跑过**：本机 fixture 只有明文 http + 回环（生产策略下必然被拒），因此"rustls 信任根 + `resolve_to_addrs` + TLS SNI/证书校验"的**成功**组合未被观测；只有拒绝路径与"Host 头保留域名"被观测。T22/T23 需在真实 https 目标上补一次成功下载。
3. **父 job 终态**：T13 完成后模型分支成功，父 job 仍因 `manual_merge`/`assemble_draft`（T14/T15）延后停在 `running`——非本卡缺陷，T15 验收时以阶段状态与 `model_revisions` 为观察点（与 §T13-8 第 7 条一致）。
4. **UI-039 文案与"原始模型已保留"的前端呈现**属 T17；本回合只验证服务端 `needs_input` 缺项码与文案字符串。
5. **GPU 可绘制性**：CPU 结构校验不证明浏览器能渲染（T18 二次验证 + `modelReview.loaded`）。
6. **`model_validate` 的 CPU 耗时**：本回合只用 2 面/8px 的 QA 微型模型（毫秒级），100k 面/4K 贴图的校验耗时与内存未测（属 T21 性能记录）。
7. **`rejected` revision 的清理/归档策略**：无需求、无实现（§T13-8 第 6 条），保留量增长风险留待后续需求。
8. **`allowed_hosts` 的 IDN／尾点（`cdn.example.com.`）等边界**未测（当前为精确字符串匹配，尾点会不匹配 → fail-closed）。

## 非代码知识与限制（跨卡复用）

1. **"连接 pin 到已校验 IP"的可观测证明法**（新）：给允许域注入"只在解析器里存在"的映射（解析到回环），URL 用**域名**而非 IP 字面量。若实现未 pin（再次用系统解析），`.test` 保留域不可能解析成功 → 连接失败；成功 + fixture 收到 `Host: <域名>:<port>` 同时证明"pin 生效"与"hostname/SNI 保留"。T23 可用同一手法核对真实域名（把 `resolve_to_addrs` 换成真实 DNS 后断言 Host/SNI）。
2. **"第 N 跳 0 连接"必须用独立连接计数**：请求计数无法区分"连上但没发请求"与"根本没连"；QA fixture 因此把 `connections` 与 `requests` 分开计数（`QaServer::connections()`）。
3. **两道大小门的判别**：`download_too_large` 的 `declared` 字段（`Some`=声明长度门、`None`=流中计数门）是区分两道门的可断言语柄；chunked 响应要用 `transfer-encoding: chunked`（无 Content-Length）才能真正触发第二道。
4. **发布门禁必须用发布产物 + 误配**：只跑测试构建的"两道门"单测无法证明发布构建不放行；QA 用 dist 二进制 + 故意 `allow_local_fixture=true` + 真任务走链，并把"fixture 0 次模型请求 + 0 资产 + 付费仍 1 次"作为三段证据。
5. **schema 观测的现成入口**：`_sqlx_migrations`（不是 `schema_migrations`）；列清单用 `SELECT name FROM pragma_table_info(?)` 可带 bind（sqlx 0.9 的动态 SQL 需 `AssertSqlSafe`，PRAGMA 直接拼字符串会被拒绝）。
6. **全库 canary 扫描**（跨卡复用）：`SELECT COUNT(*) FROM {table} WHERE instr(CAST({col} AS TEXT), ?) > 0` 遍历 `sqlite_master` 的表与列，能发现"临时 URL/密钥被写进某个没被想到的表/列"——比逐表断言更彻底（本回合正是它发现了 P3-1 的命中位置）。T15/T20（DTO 脱敏、导出无临时 URL）应复用该手法。
7. 以上已同步 `llmdoc/decisions.md`「T13 验收知识」；`validation-release.md` 命令合同无变化，未改动。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–14 | 2026-09-12 | slice | T01–T12 | 1/2 | PASS（10/12 FAIL → 修复后 PASS） | 本文件上半部分 |
| 15 | 2026-09-12 | slice | T13 | 2（ui_revision 2） | **PASS** | 本节 |

- 交接给协调者：(a) `qa_result: PASS`（仅 T13 切片）、`qa_round: 15`、`qa_history` 追加回合 15（PASS / slice / [T13]）；(b) `open_defects` 仍为空（3 条 P3 观察不构成缺陷）；(c) **T13 进入 `accepted_tasks`**：`{task_id: T13, prd_revision: 2, qa_round: 15, accepted_ac_ids: [AC-043, AC-044], status: accepted}`——AC-043/AC-044 的**服务端与下载/校验侧全部子句**已覆盖；UI-039 文案呈现与父 job 终态分别属 T17/T15，**未随本卡验收**，届时按各自卡重验；(d) P3-1/P3-2 建议转 T15/T17 或由 PM 决定是否补文档/新需求；P3-3 属可选收口。
- 交接给 RD：**无必须修复项**。
- 交接给 PM：无需求歧义需裁定；如需支持"带大量 EXIF/ICC 元数据的 JPEG 贴图"（P3-2）或缩短临时 URL 保留窗口（P3-1），请开需求或明确接受当前限制。
- 全项目状态提醒：T13 PASS **只覆盖"模型下载、校验与不可变版本"切片**（fixture 与自建负例）；T14–T23（说明书 AI、组装草稿、任务中心、阅读器、热点/发布、备份导出、正式单二进制 smoke、真实授权链路与目标平台）仍未实现/验收，MVP 尚未完成。
- QA 产出：`crates/server/tests/qa_t13_independent.rs`（15 条常驻用例，无 `#[ignore]`；QA 新增，属允许写入范围）；证据目录 `artifacts/web-mvp/t13-qa/`——`qa-t13-independent.log`、`qa-release-gate.log`、`qa-t13-release-gate.py`、`qa-gate-report.json`、`qa-gate-fixture.jsonl`、`qa-gate-serve.log`、`qa-schema-check.log`、`qa-unique-indexes.log`、`qa-download-config.log`、`qa-test-lsof.txt`、`qa-model-assets-lsof.txt`、`qa-dist.log`、`qa-dist-hash.txt`、`qa-smoke-bootstrap.log`、`qa-contracts-check.log`、`qa-tripo-contract.log`、`qa-t12-independent.log`、`qa-xtask-check-final.log`、`qa-source-hashes.txt`。

# 回合 16 · T14（说明书 AI 适配与证据校验）

**结果：PASS（仅 T14 切片范围；无新增缺陷，3 条 P3 观察）** · 回合：16 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T14]；AC-045、AC-046 全部子句 + 卡内项 + 回归链 + RD 改动核对）

- QA 执行时间：2026-09-12 13:45–14:35（本地 UTC+8）；执行者：QA 子 agent（回合 16）。
- **独立性声明**：`implementation.md` §T14 与 `artifacts/web-mvp/t14-rd/**` **只作线索，未用作任何通过证据**。本回合结论全部来自 QA 现场构造与执行：
  - **QA 自写 14 条集成用例** `crates/server/tests/qa_t14_independent.rs`（QA 新增，属允许写入范围）；
  - **QA 自写原始 TCP fixture**（`QaHttp`，只用 `std::net`：连接数与请求数分开计数、按"批内页内容标记"路由响应、可脚本化"声明完整 `content-length` 却只写一半"的截断响应），**不复用 T05 `FixtureServer` 场景，也不使用 RD 的 `manual-ai-fixture.py`**；
  - 拒答 / incomplete / 截断 JSON / "JSON 前后夹带解释文字"的抢救试探 / 伪造页（99 与 0 页）/ 幽灵部件引用 / 超长 / 超量 / 重复局部 id / 未知字段 / 注入文本的**响应体与页文字全部由 QA 现场构造**，不使用 `tests/fixtures/responses/manual_ai/**` 的样例文件；
  - QA 自写 lsof 观测脚本（**带正对照**）与源码哈希清单。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）＝ 派发包；REQ-029 / AC-045 / AC-046 仍为 [必选]。`state.yaml`：phase=qa_running、prd_revision=2、current_tasks=[T14]、open_defects=[]、qa_round=16。
- 本回合不覆盖：T15/T16/T17/T18/T19/T20/T22/T23（组装草稿、前端、发布、备份、正式单二进制 smoke、真实授权链路）与 UI-040/UI-041 的前端呈现。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb`（init；T01–T14 仍未提交）；QA 新增仅 `crates/server/tests/qa_t14_independent.rs` 与 `artifacts/web-mvp/t14-qa/**`；关键源码/测试 sha256 与 `git status` 见 `qa-source-hashes.txt` |
| 平台／工具链 | macOS Darwin 25.6.0 · aarch64-apple-darwin；rustc/cargo 1.98.1；node v26.0.0 |
| 交付产物 | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`22a4fc4f5d84036883e2f0db6923257bc5d79088b8e4932916849603f02a7b72`**（21 859 648 B）——**QA 独立 `cargo xtask dist` 重建，与 RD §T14-9 记录值、磁盘既有二进制三处一致**（确定性构建 + 该二进制确由当前源码产出） |
| 数据／fixture | 每用例独立临时 data-dir（自动清理，实测无残留）；受控假凭据 canary（`canary-qa-t14-not-a-real-key`）；**0 次真实付费/外网调用**；全部 HTTP 目标 = 本机 `127.0.0.1:<随机端口>` fixture |
| 零外网观测（QA 自己采样） | 用例二进制 `lsof -nP -iTCP -a -p <pid>` 采样 23 次：**97 条 TCP 采样行（LISTEN 85 / ESTABLISHED 12）**，非回环对端 **0**；**正对照成立**（1.5s 慢响应用例把连接窗口拉到可采样范围，见 `qa-t14_slow_provider_response_still_processed_loopback_only`）（`qa-lsof-loopback-only.log`、脚本 `qa-lsof-loopback-only.sh`） |
| 未验证 | 真实模型效果、真实 token 计量与账单（T23）；UI 呈现（T17）；merge→assemble_draft（T15）；正式发布包在冷目录下的说明书分支端到端（T22/T23，本回合只做 `smoke-bootstrap` 与适配器级 fixture 验证） |

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/t14-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test qa_t14_independent`（**QA 新增**） | exit 0，**14 passed / 0 failed / 0 ignored**；连续 4 轮全绿（`qa-t14-independent.log`；含 `--nocapture` 观察值 `qa-t14-independent-nocapture.log`） |
| 2 | `cargo test -p everything-manual --test manual_ai_contract`（RD 套件基线复跑，非通过依据） | exit 0，**19 passed / 0 failed**（`manual-ai-contract` 行见 `qa-xtask-check-final.log`） |
| 3 | `cargo test --workspace` | exit 0，**445 passed / 0 failed / 3 ignored**（回合 15 记录基线 391 → +54：T14 新增用例 19+12+8=39、providers 接线单测 +1、QA T14 用例 14；3 ignored 仍为 BUG-003/BUG-004 复现用例 + `items.rs` doctest，与 T13 口径一致） |
| 4 | `cargo xtask check`（终版，含 QA 新用例） | exit 0，**7/7 `[通过]`**（fmt / clippy `-D warnings` / workspace 测试 / 前端 lint / typecheck / vitest 46 / 合同检查）（`qa-xtask-check-final.log`） |
| 5 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`（T14 无 HTTP DTO 变更，与 RD 声明一致） |
| 6 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0，sha256 `22a4fc4f…`（= RD 值；`qa-dist.log`、`qa-dist-hash.txt`） |
| 7 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` | exit 0，1 项 `[准备]` + 7 项 `[检查]` 全过（`qa-smoke-bootstrap.log`） |
| 8 | `bash artifacts/web-mvp/t14-qa/qa-lsof-loopback-only.sh <qa_t14 二进制> <log>` | exit 0，14 passed；97 采样行 / ESTABLISHED 12 / **非回环 0**（`qa-lsof-loopback-only.log`） |
| 9 | 过程修正记录（诚实登记） | ① QA 自写用例首版含 2 处 clippy 违规（`too_many_arguments`、`assertions_on_constants`）→ **QA 只改自己的测试文件**（不改生产代码），随后 clippy 0 warning；② 首个 workspace 全量跑因**磁盘满**（ENOSPC，链接失败）→ QA 清理 `target/debug/incremental`（4.8 GiB 纯构建缓存，可再生）后重跑，与实现无关 |

## AC 验收矩阵（AC-045 / AC-046 全部子句 + 卡内项）

| AC / 条目 | 期望（可观察） | 实际（QA 实测） | 结论 | 命令／证据 |
| --- | --- | --- | --- | --- |
| AC-045 · Responses `input` 形态 | `input` 含 `input_text` 与必要 `input_image`（JPEG data URL） | 2 页（1 文字 + 1 扫描）请求体：`content[0].type=input_text`、`content[1].type=input_image`；data URL 前缀 `data:image/jpeg;base64,`，**QA 侧独立 base64 解码后与上传页图逐字节一致**；纯文字页批次 `content` 长度恰为 1（不发图） | PASS | `qa_t14_request_bytes_and_scan_page_image`、`qa_t14_batches_are_limited_and_covered_with_independent_assets`（`qa-t14-independent.log`） |
| AC-045 · `text.format` strict | `text.format.type=json_schema`、`name=manual_extract_v1`、`strict=true`、全部 required、`additionalProperties=false`、`max_output_tokens` 受限 | 逐字段断言通过；schema **递归**断言（root + 每个嵌套对象：`additionalProperties=false` 且 `required` 覆盖全部属性）；`max_output_tokens=4096`（∈1..=4096）、`store=false` | PASS | 同上 |
| 卡内项 · 不得出现 Chat Completions `response_format` | 请求字节中无 `response_format`，也无 `tools`/`functions`/`tool_choice`/`web_search`/URL 参数 | **原始字节级**断言：`"response_format"`、`"tools"`、`"functions"`、`"tool_choice"`、`"web_search"`、`"tool_resources"` 均不存在；JSON 键同样不存在（注入用例二次覆盖） | PASS | 同上 + `qa_t14_page_instructions_cannot_change_budget_or_reach_network` |
| AC-045 · 批次 ≤5 页与覆盖率 | ≤5 页/批并记录覆盖率 | 7 页 → **恰 2 次请求**：一批含页 1..5、另一批含页 6..7（提示词页标记逐页核对，无重叠/遗漏）；合并结果 `coverage.complete=true`、`pageCount=7`、`pages=[1..7]`、`batches[].pages` 记录"哪些页由哪批覆盖" | PASS | `qa_t14_batches_are_limited_and_covered_with_independent_assets` |
| AC-045 · 拒答/截断/畸形 JSON 不产生正式知识 | 各形态一律不产出正式知识，且无"正则抢救" | 5 批 5 形态：拒答→`manual_ai_refusal`、`incomplete`(max_output_tokens)→`manual_ai_incomplete`、截断 JSON→`manual_ai_invalid_format`、**"JSON 前后夹带 Sure!/Hope this helps!"→`manual_ai_invalid_format`**、无 `output_text`→`manual_ai_empty_output`；每批 `needs_input`、批次结果 `producedKnowledge=false` 且 parts/steps/specs/uncertainties **全为空**、诊断 sha 非空、每批恰 1 次请求（不自动重试） | PASS | `qa_t14_rejections_never_produce_knowledge_and_lock_merge` |
| AC-045 · 引用不存在页被服务端拒绝 | 引用必须存在于本批输入页 | 页 99 → `manual_ai_page_reference_invalid`；页 0（0-based 误用）同码；`step.partIds=["ghost"]` → `manual_ai_part_reference_invalid`；三者均零实体、merge 保持 `queued`、无合并资产 | PASS | `qa_t14_fake_page_and_part_references_are_rejected` |
| AC-045 · 扫描页走页图 | 扫描页发页图，出处标记 `derived` | 扫描页在提示词登记为"[第 2 页]（页图，见输入图片）"且只有它带图片；该页 evidence `derived=true`、文字页 `derived=false`、`bbox=null`（不捏造框） | PASS | `qa_t14_request_bytes_and_scan_page_image` |
| 卡内项 · 服务端二次校验 | schema / 字符串长度 / 实体数 / 引用页集合 / 部件引用关系 | 5 批 5 形态：description 1201 字符→`manual_ai_string_too_long`；241 实体→`manual_ai_entity_limit`；多出 `confidence` 字段→`manual_ai_schema_violation`；重复局部 id→`manual_ai_duplicate_id`；未知键→`manual_ai_schema_violation`；全部零实体 | PASS | `qa_t14_server_side_second_validation_catches_oversized_and_unknown_fields` |
| 卡内项 · `confidence` 不被当已验真概率 | 不向模型索取、不被接受、不进入产物 | 请求 schema 不含 `confidence`；响应带 `confidence` → schema 违规零实体；持久化批次结果与合并结果串中**无 `confidence`**；全部实体 `reviewStatus=needs_review` | PASS | 上两行用例 + `qa_t14_request_bytes_and_scan_page_image` |
| 卡内项 · 不得用正则抢救畸形 JSON | 畸形 JSON 不产生任何实体 | 抢救试探（合法 JSON 外包解释文字）与截断 JSON 的产物实体全部为空；另确认 `output_text` 非合法 JSON 与"JSON 合法但形状不符"分开报码 | PASS | `qa_t14_rejections_never_produce_knowledge_and_lock_merge` |
| AC-046 · 每批独立持久身份与结果资产 | 每批独立 id/资产 | 两批 stage/attempt/resultAsset/`responseId` 各自不同；`page_set` 分别为 [1..5] 与 [6,7] | PASS | `qa_t14_batches_are_limited_and_covered_with_independent_assets` |
| AC-046 · 全部成功且覆盖完整才解锁 merge | 任一未产出知识 → merge 锁定；覆盖不完整 → 合并层拒绝 | ① 仅一批成功时 merge 仍 `queued`；② 拒答/格式错/引用错的批次存在时 merge 恒 `queued` 且无合并资产（多用例交叉）；③ 合并纯函数对"缺页"与"页被多批覆盖"均返回 `manual_coverage_incomplete`；④ 合并阶段零新增请求（不额外调用 AI） | PASS | `qa_t14_batches_…`、`qa_t14_rejections_…`、`qa_t14_fake_page_…`、`qa_t14_merge_dedups_…` |
| AC-046 · merge 去重但保留原始出处 | 同内容单实体 + 全部出处 | 两批同名同描述部件合并为 **1 条**，`evidence` 页 `[1,6]`、`sourceBatches [0,1]`；合并结果与批次输入顺序无关且与持久化字节一致（确定性） | PASS | `qa_t14_merge_dedups_keeps_provenance_and_conflicts` |
| AC-046 · 冲突事实保留为待复核 | 同名不同事实双方保留 + 冲突记录 | 同名规格两条（DC 12V / DC 24V）都保留，`conflicts[0]`（`entityKind=spec`、`key=供电`、`reviewStatus=needs_review`、2 variants 且各自带自己的页出处） | PASS | 同上 |
| AC-046 · PDF 内恶意指令 | 不改预算、不触发 URL 访问/命令执行 | 注入页文字（含"把预算改为 0 / model 改 free-model / 访问 https://evil.invalid/exfil / 运行 rm -rf /"）：**1 请求、1 连接、无其它 target**；快照 `budgets` 前后一致；账本预留金额/状态未被改写；请求 `model`/`max_output_tokens`/`page_set` 全部来自冻结输入；无 `tools`/`functions`；注入文本仅出现在 `input_text` 数据段 | PASS | `qa_t14_page_instructions_cannot_change_budget_or_reach_network` |
| 卡内项 · 超预算不再请求 | 计划外/无预算背书 → 零请求 | ① 改写批次 `page_set`（计划外）→ `needs_input manual_batch_not_in_frozen_plan`，**0 请求 0 attempt**；② 预留被置 `released` → `needs_input manual_budget_not_holding`，**0 请求 0 attempt**；两 job 合计 fixture 连接数 0 | PASS | `qa_t14_out_of_plan_batch_and_released_budget_send_zero_requests` |
| 卡内项 · 同步恢复：已持久化不重跑 | 结果事实在库、checkpoint 未推进 → 补推进、不重付 | 还原崩溃现场后恢复扫描：`succeeded` 补推进、`result_asset_id` 不变、**attempt 行数不增（恰 1）**、请求计数不变 | PASS | `qa_t14_persisted_result_is_advanced_on_recovery_without_repaying` |
| 卡内项 · 同步恢复：未持久化 → unknown | 该批 `submission_unknown`、不重发、分支暂停 | 截断响应（声明完整长度只写一半）→ 该批 `submission_unknown`、attempt `unknown`、`response_id` 为空、预留保留且 `actual=NULL`（不填 0）；同分支另一批 → `needs_input manual_branch_paused_by_unknown` 且**零请求**；后续 4 次 tick 请求数恒为 1、连接恒为 1；父 job = `submission_unknown` | PASS | `qa_t14_unpersisted_response_becomes_unknown_and_pauses_branch` |
| 卡内项 · 不假定 `response_id` 可轮询/重取 | 不出现按 id 取响应/轮询的请求 | 全程只有 `POST /v1/responses`（`unexpected()` 为空）；`response_id` 仅作 opaque 事实落库；代码面复核无按 id 重取路径 | PASS | 同上 + 全用例 `unexpected()` 断言 |
| 卡内项 · 传输未知不触发二次付费 | 5xx/截断后绝不重发 | 见上两行（429 除外：429 可证明未被处理，属允许的退避重试） | PASS | 同上 |
| 卡内项 · 429 退避 | 尊重 `Retry-After`、客户端无隐式重试 | 429（`Retry-After: 30`）→ `retry_wait`、attempt `failed`、`next_run_at ≥ now+20s`；提前 tick（+10s）**不重发**；过窗口后重发且**两次请求字节完全一致**（确定性），最终 `succeeded` | PASS | `qa_t14_rate_limit_waits_retry_after_before_second_request` |
| 卡内项 · 崩溃恢复 × 拒答（边界观察） | 拒答批次在任何恢复路径下都不得产出正式知识 | 拒答批次被补推进为 `succeeded`（诊断结果资产被当作"结果事实"）后，merge 仍 `needs_input`（`manual_batch_without_knowledge`）、无合并资产、批次结果仍零知识、请求数不变 | PASS（观察见 P3-1） | `qa_t14_refusal_crash_before_checkpoint_never_yields_merged_knowledge`（`--nocapture` 观察行：`batch.status=succeeded merge.status=needs_input merge.asset=None`） |

## RD 改动核对（派发要求：`jobs/submission.rs` 与执行器日志）

| 改动 | 核对方式 | 结论 |
| --- | --- | --- |
| `SubmissionWindow::begin_intent`：`pool.begin()` → `pool.begin_with("BEGIN IMMEDIATE")` | 逐行读实现 + **QA 独立构造竞争用例** + T10 全量回归 | **不改变 T10 语义**。五步顺序（intent → submitting → HTTP → 事实 → epoch 推进）、未决 attempt 拒绝规则（`submitting`/`unknown` → Err 不新建；`intent` → 标 failed 后可安全重领；`accepted`/`failed` → 允许新 intent）、failpoint 位置（`PAID_AFTER_INTENT_BEFORE_SUBMITTING`）与"事实写入不带 epoch guard、状态推进才带 guard"的拆分全部未变；唯一变化是**事务一开始就取写锁**（读-判-写仍在同一事务内，原子性不降级）。QA 实测：活跃写者（另连接 `BEGIN IMMEDIATE` + UPDATE，持锁 400ms）期间 `begin_intent` 在 20s 预算内**等待后成功**（无 `database is locked`），attempt 恰 1 行；随后 `mark_submitting` 后再次 `begin_intent` → Err（含"未对账"）且 attempt 仍 1 行（不产生第二次付费）。T10 回归：`jobs_recovery` 25、`qa_t10_independent` 7（1 ignored 为既有复现用例）、`qa_t10_r11_verify` 4 全绿 |
| 必要性评估 | 读 WAL/deferred 语义 + 现象来源 | **认可**。`begin_intent` 是"先读（查未决 attempt）后写（插 intent）"事务；WAL 下 deferred 事务的读→写升级遇活跃写者会立即 `SQLITE_BUSY`（`busy_timeout` 不等待），把一次付费批次误判为可重试失败（安全但浪费重试额度并产生错误的失败记录）。改为 `BEGIN IMMEDIATE` 与 `job_stages::claim_next` 同模式；代价是每个付费提交提前持写锁，链路为短事务（毫秒级）且受 `busy_timeout=5s` 限制。**残留风险**：极端写竞争下可能等满 5s 后失败——此时 intent 尚未创建，阶段按可重试失败处理，**不会重复付费**（QA 未构造该极端形态，见"未覆盖边界"） |
| 执行器 `job_stage_handler_error` 新增脱敏 `detail` 字段 | 读 `JobError`/`StorageError` 的 `Display` + 扫描 T14 路径错误串 + RD 冒烟日志 canary 扫描 | **必要且当前无泄密面**。`JobError::Handler` 的消息由处理器构造（未决 attempt 状态、结算/预算说明、缺项摘要），不含 API key、页原文或完整响应；`JobError::Storage` 的 `detail` 来自 SQLite 消息（表/列/约束名与我们自己的触发器文案），不含用户正文；manual_ai 客户端的错误在进入日志前已 `redact_url_query` + 截断（单测覆盖）；RD 冒烟日志中 canary/`Bearer`/`sk-` 命中 **0**。**维护注意**：该日志字段会原样打印未来处理器塞进错误消息的内容——后续卡若把页原文/签名 URL 写进错误串会外泄（已记入 llmdoc 决策「T14 验收知识」） |

## 注入防护与拒绝路径的独立性说明（本回合构造了什么、观察到什么）

1. **原始 TCP fixture 而非场景设施**：`QaHttp` 只 `bind 127.0.0.1:0`，逐请求记录 `method/target/全部请求头/原始字节`，并把**连接数与请求数分开计数**——因此"资料里的 URL 未被访问"是"1 条连接、1 个请求、无其它 target"的直接观测，而不是推断（`unexpected()` 对任何非 `POST /v1/responses` 的目标立即失败）。
2. **正对照先于结论**：lsof 采样对毫秒级短连接几乎看不见（首轮 76 行采样里 ESTABLISHED 为 0）。因此 QA 专门加入 `delayed(1_500)` 的慢响应用例，把连接窗口拉到可采样范围；终版观测 **ESTABLISHED 12 行且全为回环**，非回环 0——"0" 因此有采样灵敏度背书。
3. **拒绝路径的响应体由 QA 现场构造**：拒答用 `content[].type=refusal`；截断用 `status=incomplete` + `incomplete_details.reason=max_output_tokens`；畸形用"合法 JSON 外包解释文字"（专门诱导"正则抢救"）与半截 JSON；伪造页用 99 与 0（0-based 误用）；部件引用用 `["ghost"]`。断言落在**服务端产物**（`needs_input` 码 + 批次结果资产的 `producedKnowledge=false` 与四类实体计数为 0 + merge 未解锁），不依赖 fixture 自证。
4. **不采信 RD 的注入结论**：QA 自己的注入页文字含"改预算/改模型/访问 URL/执行命令"四类要求，逐项核对快照预算 JSON 前后相等、账本预留未改写、请求 `model`/`max_output_tokens`/`page_set` 仍来自冻结输入；另做静态复核：`crates/server/src/providers/manual_ai/**` 与 `crates/core/src/knowledge.rs` **不存在** `Command`/进程执行调用，唯一 HTTP 客户端不跟随重定向且无工具参数。
5. **响应路由与领取顺序解耦**：`claim_next` 的排序键在同一毫秒内不保证 batch_index 顺序（UUIDv7 低位随机），因此多批用例按**批内页内容标记**选择响应（`QA-PAGE-N-CONTENT`），避免"哪批拿到哪个脚本"随并发调度漂移。
6. **fixture 自身的坑已定位并修正**（不是产品缺陷）：macOS/BSD 下 `accept()` 返回的 socket **继承监听 socket 的 `O_NONBLOCK`**，不显式改回阻塞时 `read()` 会在客户端字节到达前返回 `EAGAIN`，表现为**随机**的"传输失败：请求发送失败"——首轮 12 用例中有 2–3 个随机失败，修正（`set_nonblocking(false)` + 读超时）后连续 4 轮全绿。该陷阱记入 llmdoc，供 T15/T21/T22 的 fixture 复用。

## 缺陷

**无新增缺陷（未关闭验收缺陷 = 0）**。以下 3 条为 P3 观察，**不阻断 T14，也不要求返工**。

### P3-1｜拒答批次在"结果事实已落库、checkpoint 未推进"的崩溃路径会被补推进为 `succeeded`

- 严重度／状态：P3 / OPEN（跨卡语义，T10 恢复矩阵 × T14 诊断资产的交点）
- 对应 REQ / AC：REQ-029 / AC-046（"全部批次成功且页覆盖完整才解锁 merge"的展示口径）
- 环境与输入：QA 构造（`qa_t14_refusal_crash_before_checkpoint_never_yields_merged_knowledge`）：单批 3 页 → 拒答 → 阶段落在 `needs_input`；随后 QA 把阶段还原为 `running` + 租约过期（等价于进程在"结果已持久化、checkpoint 未推进"处崩溃）→ `recover_expired_leases()`
- 期望与实际：期望恢复路径下该批仍表现为"未产出知识"（如 `needs_input`）；实际 `recover::plan` 的"`result_asset_id` 非空 → `Succeed`"规则对拒答的诊断结果资产同样生效，阶段被补推进为 **`succeeded`**（`--nocapture` 观察：`batch.status=succeeded merge.status=needs_input merge.asset=None`）
- 影响：**不产生正式知识**（合并层按 `producedKnowledge` 拒收 → merge `needs_input manual_batch_without_knowledge`）、不重复付费（请求数不变）；风险仅在展示层——T15/T17 若按阶段状态展示"批次成功"，可能让人误以为该批产出了知识。正常路径（无崩溃）不受影响，仍是 `needs_input` + 稳定错误码
- 建议：T15/T17 以批次结果资产的 `producedKnowledge`/`outcome` 作为"是否产出知识"的判据，不要只看阶段状态；若 PM/RD 希望阶段状态也如实反映，可让恢复矩阵在 `Succeed` 前读取结果事实的 `producedKnowledge`（需新需求或 T15 内调整，QA 不自行改口径）
- 证据：`crates/server/tests/qa_t14_independent.rs`、`artifacts/web-mvp/t14-qa/qa-t14-independent-nocapture.log`（QA 观察行）

### P3-2｜成功批次的原始供应商响应也长期留存（诊断 blob 无保留窗口）

- 严重度／状态：P3 / OPEN（契约允许的"受限诊断路径"，缺保留策略）
- 对应 REQ / AC：REQ-029 / AC-045（诊断路径）、REQ-043（隐私与日志）
- 环境与输入：`qa_t14_request_bytes_and_scan_page_image`（成功批次）断言 `usage.diagnosticSha256` 非空
- 期望与实际：契约只要求"保留**短**错误摘要及原始响应的**受限**诊断路径"；实现把**每一次**批次（含成功）的完整原始响应以 blob + `assets` 行（purpose 复用 `page_text`、mime `application/json`）持久化，无清理/保留窗口/大小回收策略；内容含模型读出的页文本（即资料的派生文本）
- 影响：盘占用约为"提取结果 ×2"，数据保留面比"短摘要"更宽；不违反任何当前必选 AC（不出现在任何 HTTP DTO，只有管理员会话可经资产路由读取自己的数据）
- 建议：由 PM/T15/T23 决定保留策略（例如成功路径只留 `responseId` + 摘要、失败路径保留完整 blob，或统一在任务终态后清理）
- 证据：`crates/server/src/providers/manual_ai/handlers.rs`（步骤 6 在成败判定之前执行）、`store.rs` 模块文档、`qa-t14-independent.log`

### P3-3｜批次结果资产复用 `page_text` purpose（RD 已记录，QA 确认无副作用）

- 严重度／状态：P3 / OPEN（文档/维护性，非功能缺陷）
- 对应 REQ / AC：REQ-029 / AC-046（"每批独立结果资产"的落地形态）
- 期望与实际：期望有"批次结果"语义的 purpose；实际复用 `page_text`（`assets.purpose` CHECK 冻结，新增取值需重建表 + 迁移，不在 T14 允许范围）——ADR-024 第 2 条已记录同一结论
- 影响：QA 复核**无按 purpose 的既有查询会误伤**（`pages` 按 id 引用、`page_quote_inputs` 按 id JOIN、报价/准备不按 purpose 扫描）；风险在后续卡若引入"按 purpose 聚合"的逻辑或导出/备份清单按 purpose 分类时可能误分类
- 建议：后续迁移加 `manual_batch` 专用 purpose（一次性重建 + 既有测试同步），并在导出/备份（T20）说明批次结果与诊断资产的处置
- 证据：`crates/server/src/providers/manual_ai/store.rs`、`llmdoc/decisions.md` ADR-024 第 2 条

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实供应商（T23）**：真实 Responses 的 `store` 支持情况、refusal/incomplete 的真实字段形态、真实 token 计量与账单、真实拒答率/截断率——本回合全部为 QA 构造样例；`max_output_tokens=4096` 只是本应用的单批上限，不是供应商保证。
2. **两个批次真正并发**：执行器每 tick 只领取一个阶段，QA 未构造 `manual_ai_batch_limit=2` 下"两个 `manual_extract` 同时发请求"的形态（并发上限本身由 T10 的 claim 谓词测试覆盖）；同分支"一批 unknown 时另一批暂停"已观测。
3. **极端写竞争**：`BEGIN IMMEDIATE` 只测到 400ms 级活跃写者；`busy_timeout(5s)` 耗尽的极端形态未构造（按设计此时 intent 未创建、可安全重领、不重复付费）。
4. **规模边界**：本回合最多 23 页 → 5 批；100 页上限（20 批）与页文字上限（PRD §5.3 的"页文字另设更高上限"）未压测。
5. **PDF 准备侧约束**：页图长边 ≤2000px/白底、"页图确实来自原 PDF"（客户端派生，`clientDerived`）属 T09/T21；本回合的页资产是直接构造的准备记录。
6. **失败批次的"重新授权重算"入口**：属 T15 的重试/对账端点；本卡只验处理器侧语义（QA 未复跑 RD 的 `retry_after_needs_input_clears_stale_result_before_new_attempt`）。
7. **前端呈现**：UI-040（出处可跳转、不含"置信度 xx%"措辞）与 UI-041（覆盖率、失败批次重试、merge 禁用说明）属 T17。
8. **下游消费**：`manual_merge` → `assemble_draft` 与草稿的 `needs_review` 展示属 T15/T19；本回合止于 merge 结果资产。
9. **正式发布包的说明书分支端到端**：本回合只跑 `smoke-bootstrap`（内嵌页面/静态资源/health）与适配器级 fixture 验证；"真实发布二进制 + 冷目录 + 完整说明书分支"属 T22/T23（RD 的 `smoke-t14-e2e.sh` 未被 QA 用作证据）。
10. **既有 `#[ignore]` 用例**：`qa_t10_independent::qa_total_wait_budget_must_trigger_on_real_poll_shape` 与 `qa_t11_independent::qa_defect_get_estimate_reflects_confirmation_and_consumption` 仍带"修复后移除 ignore"的注释（BUG-003/BUG-004 已在回合 11/13 修复并 PASS）。属跨回合卫生问题，不影响本回合 AC；建议协调者决定是启用它们还是更新注释。

## 非代码知识与限制（跨卡复用）

1. **macOS/BSD 的 `accept()` 继承 `O_NONBLOCK`**（新，跨卡复用）：`TcpListener::set_nonblocking(true)` 之后 accept 出来的 socket 在 macOS 上仍是非阻塞，`read()` 会在客户端字节到达前返回 `EAGAIN`，症状是**随机**的"传输失败/请求发送失败"，极易被误判为产品缺陷。本机 fixture 必须在 accept 后 `set_nonblocking(false)`（并设读超时）。已同步 `llmdoc/decisions.md`「T14 验收知识」。
2. **多批用例的响应路由要按键不要按序**：`claim_next` 排序键含 UUIDv7 低位随机，批次领取顺序不保证等于 `batch_index`；用"批内页内容标记"路由响应可让断言与调度解耦。
3. **lsof 观测需要正对照**：毫秒级短连接在 200ms 采样下几乎不可见；先制造一个 1.5s 慢响应证明采样能看见 ESTABLISHED，再宣称"非回环 0"。另：汇总行自身含 `TCP ` 字样，脚本必须先给采样行打前缀再统计。
4. **恢复矩阵对"拒答诊断结果资产"的效果**（P3-1 的根因摘要）：`recover::plan` 以 `result_asset_id` 非空判定"结果已持久化 → 补推进"，而 T14 的失败路径也会写结果资产；知识侧安全（merge 按 `producedKnowledge` 拒收），展示侧需后续卡注意。
5. **执行器错误日志的脱敏边界**：`job_stage_handler_error.detail` 打印 `JobError` 的 Display——当前无泄密面，但它是"未来处理器错误消息"的转发通道；新增处理器时不要在错误消息里放页原文、签名 URL 或密钥（已记入 llmdoc）。
6. **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「T14 验收知识」（上述 1–5）；`llmdoc/validation-release.md` 命令合同**无变化**（本回合未发现需要修订的验收命令或阈值）；本报告即验收依据与未覆盖边界的正式记录。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–15 | 2026-09-12 | slice | T01–T13 | 1/2 | PASS（回合 10、12 各一次 FAIL → 修复后 PASS） | 本文件上半部分 |
| 16 | 2026-09-12 | slice | T14 | 2（ui_revision 2） | **PASS** | 本节 |

- 交接给协调者：(a) `qa_result: PASS`（仅 T14 切片）、`qa_round: 16`、`qa_history` 追加回合 16（PASS / slice / [T14]）；(b) `open_defects` 仍为空（3 条 P3 观察不构成缺陷）；(c) **T14 进入 `accepted_tasks`**：`{task_id: T14, prd_revision: 2, qa_round: 16, accepted_ac_ids: [AC-045, AC-046], status: accepted}`——两条 AC 的**服务端/适配器侧全部子句**已覆盖（前端 UI-040/UI-041 属 T17，未随本卡验收）；(d) P3-1 建议转 T15/T17（展示口径）、P3-2 与 P3-3 建议转 PM/T15/T20 决定保留策略与 purpose 收口。
- 交接给 RD：**无必须修复项**。可选后续：P3-1 的恢复矩阵判据（如需在 T15 内调整）与 P3-3 的 purpose 迁移。
- 交接给 PM：无需求歧义需裁定；如需缩短诊断 blob 保留窗口（P3-2）或新增 `manual_batch` purpose（P3-3），请开需求或明确接受当前限制。
- 全项目状态提醒：T14 PASS **只覆盖"说明书 AI 适配与证据校验"切片**（QA 构造 fixture 与拒绝/注入负例，零真实外网、零真实付费）；T15–T23（组装草稿、前端、任务中心、阅读器、热点/发布、备份导出、正式单二进制 smoke、真实授权链路与目标平台）仍未实现/验收，MVP 尚未完成。
- QA 产出：`crates/server/tests/qa_t14_independent.rs`（14 条常驻用例，无 `#[ignore]`；QA 新增，属允许写入范围）；证据目录 `artifacts/web-mvp/t14-qa/`——`qa-t14-independent.log`、`qa-t14-independent-nocapture.log`、`qa-workspace-tests-final.log`、`qa-xtask-check-final.log`、`qa-contracts-check.log`、`qa-dist.log`、`qa-dist-hash.txt`、`qa-smoke-bootstrap.log`、`qa-lsof-loopback-only.sh`、`qa-lsof-loopback-only.log`、`qa-source-hashes.txt`。

# 回合 17 · T15（两条分支组装草稿）

**结果：PASS（仅 T15 切片范围；无新增缺陷，3 条 P3 观察）** · 回合：17 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T15]；AC-047、AC-048 全部子句 + AC-037/AC-039/AC-040 端点侧 + 卡内项 + 回归链 + assemble 领取放宽核对 + T14 P3-1 生效核对）

- QA 执行时间：2026-09-12 14:40–15:35（本地 UTC+8）；执行者：QA 子 agent（回合 17）。
- **独立性声明**：`implementation.md` §T15 与 `artifacts/web-mvp/t15-rd/**`（含 `smoke-t15-e2e.py`）**只作线索，未用作任何通过证据**。本回合结论全部来自 QA 现场构造与执行：
  - **QA 自写 11 条集成用例** `crates/server/tests/qa_t15_independent.rs`（QA 新增，属允许写入范围）：`qa_t15_full_chain_assembles_needs_review_draft_and_never_publishes`、`qa_t15_crash_replay_does_not_create_a_second_draft`、`qa_t15_blocked_model_branch_yields_partial_draft_and_gates_retry`、`qa_t15_upstream_unknown_blocks_assembly_until_reconciled`、`qa_t15_attach_remote_task_verifies_queries_and_does_not_repurchase`、`qa_t15_record_no_task_requires_evidence_and_keeps_reservation`、`qa_t15_cancel_keeps_submitted_state_and_adds_no_paid_steps`、`qa_t15_retry_only_knowledge_branch_preserves_model_artifacts`、`qa_t15_draft_patch_contract_and_no_publish_side_effects`、`qa_t15_refused_batch_recovery_never_reports_success`、`qa_t15_assembly_revision_follows_content_change_only`；
  - **QA 自写原始 TCP fixture**（`QaHttp`，只用 `std::net`：按「方法 + 路径（精确/前缀）+ 请求体标记」三元组路由，未命中即记 unexpected 并返回 501），**不复用 T05 `FixtureServer` 场景设施，也不使用 RD 的 `t15-fixture.py` / `smoke-t15-e2e.py`**；Tripo 上传/提交/查询/模型 CDN、说明书 AI 的成功/拒答/截断响应体全部现场构造；
  - 全链路、取消、重试、对账、崩溃恢复、P3-1 现场**全部由 QA 自己驱动**（真实执行器 + 真实 HTTP 路由 + 真实 SQLite + 临时 data-dir），断言落在数据库状态、API 响应、账本与审计，不依赖 fixture 自证；
  - 零外网观测为 QA 自采样（`qa-lsof-loopback-only.sh`，含正对照）。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）＝ 派发包；REQ-030（主）/REQ-025/REQ-026、AC-047/048/037/039/040 仍为 [必选]。`state.yaml`：phase=qa_running、prd_revision=2、current_tasks=[T15]、open_defects=[]、qa_round=17。
- 本回合不覆盖：T16/T17/T18/T19/T20/T22/T23（任务中心/草稿页前端、发布、阅读器、备份、正式单二进制 smoke、真实授权链路）与 AC-048/AC-039 的 **UI 文案侧**（`import-flow.spec.ts`、`job-recovery.spec.ts` 属 T16/T17）。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb`（T01–T15 仍未提交）；QA 新增仅 `crates/server/tests/qa_t15_independent.rs` 与 `artifacts/web-mvp/t15-qa/**`；关键源码/测试 sha256 与 `git status` 见 `qa-source-hashes.txt` |
| 平台／工具链 | macOS Darwin 25.6.0 · aarch64-apple-darwin；rustc/cargo 1.98.1；node v26.0.0 |
| 交付产物 | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`f6bc9e2a1380a8c018aaeee2ca9f5abf97cd8f9198940a54d39e1bfee09ec85c`**（22 494 896 B）——**QA 独立 `cargo xtask dist` 两次重建哈希一致，且与 RD `artifacts/web-mvp/t15-rd/dist-hash.txt`、磁盘既有二进制的哈希三处一致**（确定性构建 + 该二进制确由当前源码产出）。注：RD §T15-10 表内记录的 size「22 434 320 B」与实际不符，见非阻断观察 P3-2 |
| 数据／fixture | 每用例独立临时 data-dir（自动清理，实测无残留）；受控假凭据 canary（`canary-qa-t15-not-a-real-key`）；模型下载走测试构建的显式回环放行（`download.allow_local_fixture=true` + `allowed_hosts=["127.0.0.1"]`）；**0 次真实付费/外网调用** |
| 零外网观测（QA 自己采样） | ① 常规跑：采样 67 次、130 行（LISTEN 64 / ESTABLISHED 2 / **非回环 0**）；② **正对照**（`QA_T15_SLOW_MS=800` 拉长 fixture 连接窗口）：采样 453 次、1668 行（LISTEN 453 全部 `127.0.0.1:*` / ESTABLISHED 762 全部 `127.0.0.1→127.0.0.1` / **非回环 0**）——正对照证明采样对 ESTABLISHED 有灵敏度（`qa-t15-lsof-loopback-only.log`、`qa-t15-slow-lsof-loopback-only.log`、脚本 `qa-lsof-loopback-only.sh`） |
| 未验证 | 真实 Tripo 账户的 `attachRemoteTask`（需真实账户 ID，T23）；取消后远端最终状态的人工核对；AC-048/AC-039 的 UI 文案与 Playwright 用例（T16/T17）；正式发布包的完整生成链路（T21/T22 测试构建侧、T23 真实侧） |

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/t15-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test qa_t15_independent`（**QA 新增**） | exit 0，**11 passed / 0 failed / 0 ignored**；连续 4 轮全绿（`qa-t15-independent-3rounds.log`、`qa-t15-independent-nocapture.log`） |
| 2 | `cargo test -p everything-manual --test pipeline`（RD 套件基线复跑，非通过依据） | exit 0，**11 passed / 0 failed**（`qa-pipeline-baseline.log`） |
| 3 | `cargo test --workspace` | exit 0，**471 passed / 0 failed / 3 ignored**（30 个测试目标，计数按日志逐目标求和；3 ignored 仍为 BUG-003/BUG-004 复现用例 + `items.rs` doctest，与 T13/T14 口径一致）（`qa-workspace-tests.log`） |
| 4 | `cargo xtask check`（终版，含 QA 新用例） | exit 0，**7/7 `[通过]`**（fmt / clippy `-D warnings` / workspace 测试 471 / 前端 lint / typecheck / vitest / 合同检查）（`qa-xtask-check-final.log`） |
| 5 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`（`qa-contracts-check.log`） |
| 6 | `cargo xtask dist --target aarch64-apple-darwin`（两次） | exit 0，两次 sha256 均 `f6bc9e2a…`（22 494 896 B）（`qa-dist-1.log`、`qa-dist-2.log`） |
| 7 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` | exit 0，1 项 `[准备]` + 7 项 `[检查]` 全过（含 `/api/unknown` JSON 404、SPA 路由刷新、缺资源 404）（`qa-smoke-bootstrap.log`） |
| 8 | `bash artifacts/web-mvp/t15-qa/qa-lsof-loopback-only.sh <qa_t15 二进制> <前缀> 0` 与 `… 800` | 两次 exit 0；**非回环 ESTABLISHED 均为 0**（明细见上表） |

## AC 验收矩阵（逐子句）

| AC / 条目 | 期望（可观察） | 实际（QA 实测） | 结论 | 命令／证据 |
| --- | --- | --- | --- | --- |
| AC-047 · 组装出 draft 且 `needs_review` | fixture 全链路 → 草稿 `needs_review` | 3 页文字 + front/left 照片 → 两分支各自完成 → 组装：job `succeeded`、草稿 `status=needs_review`、`completeness=complete`、`modelRevisionId` 非空、`knowledge.knowledge.parts` = 3（与页数一致）、外壳 `schemaVersion=manual_draft_v1`、`missing=[]`、`review=null`（modelReview 属 T19）；组装阶段 `usage_json` 记录 `draftId/revision/completeness` | PASS | 用例 1（`qa-t15-independent.log`） |
| AC-047 · job `succeeded` ≠ published | 无 release、无 publish 路径 | 全链路成功时 `SELECT COUNT(*) FROM manual_releases` = **0**；`POST /items/{id}/drafts/{id}/publish` → **404**；草稿 `notices` 固定含「生成完成不等于已发布」与「不存在自动发布路径」；任务列表行 `status=succeeded` 且 `stageSummary.succeeded == total`（不是百分比） | PASS | 用例 1 |
| AC-047 · 模型成功知识失败只重提取知识 | 只重跑知识分支；模型成果保留 | 说明书 AI 首跑拒答 → 批次 `needs_input`、`knowledgeProduced=false`、job `needs_input`、组装保持 `queued`；`POST retry` 只重跑该批次 → 第二次说明书请求成功 → job `succeeded`、草稿 `complete`；**Tripo 付费提交计数不变（1）**、`model_validate` 的 `usage_json`（含 `modelRevisionId`）逐字段不变、快照（budgets/providerConfig/promptVersion/priceVersion）前后完全相等 | PASS | 用例 8 |
| AC-047 · 重启不重复创建 draft | 同一快照至多一份草稿、同 id/同 revision | 还原「草稿已写、组装 checkpoint 未推进」的崩溃现场（`status=running`、`lease_until=1`、清 result/usage）→ 恢复扫描重领 → 重跑 upsert：草稿 **id 与 revision 均不变**、`knowledge_json` 逐字节相等、`manual_drafts` 行数仍 1、**供应商请求总数不变**、`draft_assembled` 审计不追加 | PASS | 用例 2 |
| AC-047 · 部分成功可展示 | 分支头阻塞时草稿仍产出并标缺项 | 模型分支头 `model_validate=needs_input`（截断 GLB 不静默改坏）+ 知识完整 → 组装成功：`completeness=partial`、`missing[0].code=model_branch_incomplete`、`modelRevisionId=null`、知识部件仍在、草稿 `needs_review`、**父 job `needs_input`（不冒充 succeeded）**、releases=0 | PASS | 用例 3（观察行见下） |
| AC-047/放宽核对 · 上游阻塞不提前组装 | 分支头仍 `queued` 时不组装 | ① 知识侧：批次 `submission_unknown`、`manual_merge=queued`、`assemble_draft=queued`、`drafts=0`、job `submission_unknown`；② 模型侧：`tripo_submit=submission_unknown`、`model_validate=queued`、`assemble=queued`、`drafts=0`；③ 拒答批次 `needs_input` + 另一批 `succeeded` 时 merge 仍 `queued`、`drafts=0` | PASS | 用例 4/5/10（观察行 `QA-OBSERVE[upstream-unknown] job="submission_unknown" merge=queued assemble=queued drafts=0`） |
| AC-048 · releases 为空 | 生成成功后 releases 列表为空 | 9 个用例在各自成功/部分成功/取消等终态均断言 `manual_releases` 计数 = 0；代码面复核：`crates/server/src/**`、`crates/core/src/**`、`apps/web/src/**` 中 **不存在任何写 `manual_releases` 的代码路径**（仅注释与迁移/触发器），前端不存在 `publish` 调用 | PASS | 用例 1/3/4/5/6/8/9/11 + 静态复核 |
| AC-048 · 草稿 `needs_review` + 明确提示需人工确认 | API 明示 | 草稿响应 `status=needs_review` + 固定 `notices`（两条，「不等于已发布」/「不存在自动发布路径」）；publish 路由 404 → 服务端不提供任何「确认后自动发布」的入口 | PASS（API 侧） | 用例 1/9 |
| AC-048 · 界面提示（UI 侧） | 界面明确提示人工确认后才能发布 | **NOT_RUN**：草稿/任务页面前端属 T16/T17（`import-flow.spec.ts` 未在本卡范围）；本卡只交付 API 语义与 notices | NOT_RUN（范围外） | — |
| AC-037 · attempt `submission_unknown` + 暂停分支 | 未知提交暂停该分支后续购买 | 模型侧：`tripo_submit` 截断响应（无 task ID）→ `submission_unknown`、attempt `submitting/unknown`、`remote_task_id=null`；下游 `model_download/model_validate` 保持 `queued`（未解锁）、组装保持 `queued`、**drafts=0**；知识侧：截断响应 → 该批 `submission_unknown`、`response_id` 不假定可轮询（后续对该批的 attach 被拒） | PASS | 用例 4/5/6 |
| AC-037 · 不提供一键盲目重试 | unknown 不得从此盲重试（被拒且无副作用） | 知识批 unknown + `POST retry` → 422 `stageNotRetryable`，批次状态不变、attempt 数不变、**请求数不变（仍 1）**；模型侧 unknown + `POST retry` → 422 `stageNotRetryable`、付费提交数不变 | PASS | 用例 4/5 |
| AC-037 · `attachRemoteTask` 仅 Tripo + 查询验证 + 二次确认 | 同步链路明拒；验证失败无副作用；正例不重购 | ① 同步 Manual AI → 422 `attachRemoteTaskUnsupported`（请求数不变）；② 缺 `remoteTaskId` / 缺 `acknowledgeMatches` → 422 字段级；③ 幽灵任务（fixture 404）→ 422 `remoteTaskVerificationFailed`，**attempt/阶段不变、未写远端 ID、审计不新增**；④ 正例：**服务端用当前 Tripo 凭据真实 `GET /v3/tasks/{id}` 查询一次（QA 断言请求带 `Bearer`）** → 阶段回 `queued`、attempt `remote_task_id` 落库（null→值）、审计 `job_reconcile_attach_remote_task` 含 `acknowledgedMatches=true`；⑤ 继续推进**付费提交计数不变**（不重新购买）→ 全链路 `succeeded` + 完整草稿 | PASS | 用例 5 |
| AC-037 · `recordNoTask` 需证据 | 缺证据拒绝；正例：声明 + 不释放预留 | 缺 `evidence` → 422 字段级、阶段仍 `submission_unknown`；正例 → attempt `failed`、阶段 `needs_input`、**Tripo 预留保持 `reserved`（非 Released）且 `actual=null`（不填 0）**、审计 metadata 含 `providerProof=false`/`reservationReleased=false`；随后显式 retry 产生**新 attempt 而不新建预留**（账本条目仍 1 条）、付费提交 = 2（一次未知 + 一次人工授权）→ 最终完整草稿 | PASS | 用例 6 |
| AC-037 · `authorizeReplacement` 需再次预算确认 | ack + 覆盖冻结上界；保留旧未决账务 | 缺 `acknowledgeDuplicateRisk` → 422；`limits.manualAiUsdMicros=1`（低于冻结上界）→ 422 `budgetBelowPlannedUpperBound` 且**零请求**；正例 → 200 `stageStatus=queued`、notice 含「重复收费」、旧 attempt `failed`、**未决预留仍 `reserved`/`actual=null`（不新建预留、不释放）**、审计 1 行 → 替代提交只发 1 次说明书请求（合计 2）并补齐完整草稿、模型分支未被重购 | PASS | 用例 4 |
| AC-037 · unknown 预留不自动释放 | 等待对账，不填 0 | 见上两行；另在用例 4 断言 `state ∈ {Reserved, Unknown}` 且 `actual=None` | PASS | 用例 4/6 |
| AC-037 · 审计保留 | 每个对账动作留审计 | `job_reconcile_attach_remote_task`=1、`job_reconcile_record_no_task`=1、`job_reconcile_authorize_replacement`=1（各自 metadata 含关键确认位与「无副作用」事实） | PASS | 用例 4/5/6 |
| AC-037 · 仅管理员 | 未登录不可对账 | 未登录 `POST /jobs/{id}/reconcile` → **401**；缺 CSRF → **403** | PASS | 用例 5 |
| AC-039 · 未提交阶段停止推进 | 未提交阶段 `cancelled`、执行器不再领取 | `tripo_poll=waiting_provider` 时取消：`freeze_inputs/manual_extract/manual_merge/tripo_upload/tripo_submit=succeeded`（成果保留）、`model_download/model_validate/assemble_draft=cancelled`；取消后 8 次 tick **请求总数不变**（无新付费步骤、也无轮询） | PASS | 用例 7 |
| AC-039 · 已提交阶段保留查询与账务 | 不撤单、账务不静默释放 | `preservedStages` 含 `tripo_poll=waiting_provider`；取消后 `GET /jobs/{id}` 仍 200 且 `attempts`、`reservations` 可读（QA 断言非空）；账本无 `Released` 条目 | PASS | 用例 7 |
| AC-039 · 不声称已取消远端付费操作 | 响应文案 | 响应 `notice` 含「不撤销」；preserved 与 stagesCancelled 如实；再次取消已终态任务 → 422 `cancelNotNeeded` 且不追加审计 | PASS | 用例 7 |
| AC-039 · 审计 | 产生 audit_event | `job_cancelled` 恰 1 行（metadata：`remoteCancellationNotClaimed: true`、取消阶段数、保留阶段） | PASS | 用例 7 |
| AC-040 · 只重跑指定可重试阶段、成果保留 | 其它阶段不被覆盖 | 知识分支重试：模型分支 `usage_json` 不变、付费提交计数不变；模型分支重试（recordNoTask 后）：`manual_merge` 结果资产 id 前后相同、账本条目数不变 | PASS | 用例 6/8 |
| AC-040 · If-Match + Idempotency-Key | 缺 428/422；重放不产生第二个 attempt | 缺 If-Match → 428；缺 `Idempotency-Key` → 422 字段级；缺 CSRF → 403；同 key 同 body 重放 → 200 + `x-idempotent-replay: true`；同 key 不同 body → 409 | PASS | 用例 8 |
| AC-040 · unknown 不得从此盲重试 | 被拒且无副作用 | 见 AC-037 行（知识/模型两侧） | PASS | 用例 4/5 |
| AC-040 · 重试不改模型/质量预设 | 快照不变 | 重试前后 `budgets/providerConfig/promptVersion/priceVersion` 完全相等（快照比对）；重试 notice 明示「不改变模型/质量预设」 | PASS | 用例 8 |
| 卡内项 · `assemble_draft` 幂等（内容比较 upsert） | 内容相同不写库；内容变化才 +1 | 相同：见 AC-047 重启行；变化：改写合并结果事实后组装 → 同一草稿行、`revision 2→3`、`status` 从人工 `ready` **拉回 `needs_review`**、知识聚合已更新；再次组装（内容未变）→ `revision` 停留 3、不写库 | PASS | 用例 2/11 |
| 卡内项 · 草稿读取 ETag / PATCH 边界 | ETag；428/412/422/404；不提供绕过 T19 的入口 | `GET` 返回 `ETag: "r1"`；PATCH 缺 If-Match → 428；非法 If-Match → 422；空 body → 422；未知字段（`knowledgeJson`）→ 422；`needs_review→ready` → 200 + `"r2"`（releases 仍 0）；同状态幂等 → 仍 `"r2"`；stale → 412 + `details.currentRevision=2`；跨物品 GET/PATCH → 404；知识内容与 missing 不因状态变更被改写；`draft_status_changed` 审计 1 行 | PASS | 用例 9 |
| 卡内项 · T14 P3-1 生效 | 拒答/无知识批次在「结果已落库、checkpoint 未推进」的恢复路径**不得**被补推进为成功知识 | 6 页 → 2 批：批次 0（页 1..5）拒答 → `needs_input`（`result_asset_id` 非空、`usage.producedKnowledge=false`）；**正对照**：批次 1（页 6，扫描页）`producedKnowledge=true` → 恢复补推进 `succeeded`（report.succeeded=1）；随后还原批次 0 的崩溃现场 → 恢复扫描 report `needs_input=1 / succeeded=0`，批次 0 回到 `needs_input`、缺项沿用 `manual_ai_refusal`、`manual_extract` 中 succeeded 仅 1、merge 仍 `queued`、`drafts=0`、**请求数不变**；任务详情 `stages[].knowledgeProduced=false` | PASS | 用例 10（观察行 `QA-OBSERVE[p3-1-recovery] refused_batch=needs_input produced_batch=succeeded producedKnowledge=Some(Bool(false)) merge=queued assemble=queued drafts=0 manual_requests=2`） |
| 卡内项 · 回归（T10 语义不被破坏） | 上游阻塞锁死下游、聚合优先级 | `cargo test --workspace` 全绿，含 `jobs_recovery`（25）与 T10/T11/T12/T13/T14 的 QA 独立套件；RD `pipeline` 11 用例复跑通过 | PASS | 命令 #2/#3/#4 |

## assemble「依赖已定性」放宽的核对结论（派发要求特别核对项）

**结论：未发现语义漏洞；放宽只对分支头生效，上游未定性时组装仍被阻塞。** 依据（QA 自己构造的现场 + 代码级逐条核对）：

1. `CLAIM_CANDIDATE_SQL` 对 `assemble_draft` 的依赖判据是「依赖阶段**不处于** `queued/retry_wait/running/waiting_provider`」（即 `succeeded/failed/needs_input/submission_unknown/cancelled`），普通阶段仍是「全部 `succeeded`」；组装的依赖边只有 `manual_merge` 与 `model_validate` 两条**分支头**（`crates/core/src/jobs.rs` 的 `stage_dependency(AssembleDraft) = Fixed(&[ManualMerge, ModelValidate])`），**不直接依赖各批次**。
2. 于是「上游批次未知/缺项」时分支头保持 `queued`（批次未 succeeded → merge 不解锁），组装**不可领取**——QA 实测三组现场均为 `drafts=0`（知识 unknown、模型 unknown、拒答批 + 成功批并存）。因此**不会**提前产出「部分成功」草稿误导用户；未知期间的展示口径是任务详情的阶段状态 + `submission_unknown` 聚合。
3. 只有分支头**自身已定性**才组装：`model_validate=needs_input`（截断 GLB）→ 部分草稿 `missing=model_branch_incomplete`；`manual_merge` 侧的未知/失败同理。缺项文案已区分 unknown（「先对账，不自动重试/不重复购买」）与 needs_input/failed（「可对该阶段重试」），不会把 unknown 说成可重试。
4. **父 job 状态**：合同 §5「最后组装完成 → 父 job `succeeded`；draft.status = needs_review」在本卡被**收紧**为「全部阶段 succeeded 才 succeeded」——部分组装时父 job 显示阻塞状态（`needs_input`）。这与合同 §5 事件表「必需阶段含 unknown／needs_input → 父 job 展示对应状态与阻塞分支」一致，且避免把部分草稿冒充成完整产物（ADR-025 第 2 条已记录该取舍）。QA 判定：**不构成缺陷**，属保守方向的解释；措辞差异已随本报告留痕，供 PM 知悉（如需改回「组装完成即 succeeded」，属语义变更，应由 PM 裁定）。
5. 残留边界（如实记录）：`requeue_succeeded_dependents`（把已成功组装拉回队列）在**本卡可达的失败形态**下较难触达——模型分支头 `needs_input` 时模型侧重试被预算门槛拒绝（见 P3-1），知识分支头 `needs_input` 需要批次已全部 `succeeded`（此时批次不可重试）。QA 用「改写合并事实 + 崩溃重跑」直接验证了内容变化→`revision+1`/回到 `needs_review`（用例 11），但**未**通过公开 API 构造出「部分草稿 → 补充后自动升为完整草稿」的完整链路；记入未覆盖边界，建议 T17/T21 明确 UI 的补救入口。

## 缺陷

**无新增缺陷（未关闭验收缺陷 = 0）**。以下 3 条为 P3 观察，**不阻断 T15，也不要求返工**。

### P3-1｜模型分支头 `needs_input` 时重试被预算门槛拒绝，且缺项文案仍写「可对该阶段重试」

- 严重度／状态：P3 / OPEN（可用性/文案一致性；跨 T17/T21）
- 对应 REQ / AC：REQ-026 / AC-040（「只重跑指定可重试阶段」的可用性）、REQ-023（预算语义）
- 环境与输入：QA 用例 3：模型远端成功（`tripo_poll` 成功即结算 → 账本 `tripo=settled`）→ `model_validate` 因截断 GLB 进入 `needs_input` → 组装产出部分草稿 → 对 `model_validate` 调 `POST /jobs/{id}/retry`
- 期望与实际：期望能重跑该本地校验阶段（重下/重校验不产生新费用）；实际 **422 `budgetNotHolding`**（`crates/server/src/jobs/control.rs` 对任何 Model 分支阶段要求该分支预留仍 `reserved/unknown`。观察原始值：`QA-OBSERVE[model-head-retry] status=422 reason="budgetNotHolding" ledger=[("manual_ai","reserved"), ("tripo","settled")] model_validate=needs_input assemble=succeeded draft_completeness="partial"`）。拒绝无副作用（阶段仍 `needs_input`、付费提交仍 1 次）
- 影响：可证明「未被接受/已定性」的模型侧阻塞（如临时截断下载、外链/超面数需要换资料后重校验）在公开 API 上**没有重试入口**，只能新建任务（新快照 + 新预算确认 + 新购买）；同时草稿 `missing[]` 的文案是「请补齐后可对该阶段重试」，与该门槛矛盾，可能误导用户反复尝试
- 建议：T17/T21 明确 UI 与文案（要么允许「无外呼的本地阶段」重试，要么把文案改为「需新建任务/新报价」）；如果选择放开，须由 PM/RD 明确「何处仍必须要求预算背书」（如任何会重新外呼的阶段）
- 证据：`crates/server/tests/qa_t15_independent.rs::qa_t15_blocked_model_branch_yields_partial_draft_and_gates_retry`、`artifacts/web-mvp/t15-qa/qa-t15-independent-nocapture.log`、`crates/server/src/jobs/control.rs`（3b 预算背书块）

### P3-2｜`implementation.md` §T15-10 记录的 dist 体积与实际产物不符

- 严重度／状态：P3 / OPEN（文档准确性，非产品缺陷）
- 对应 REQ / AC：交付证据链（validation-release §1「每个结果绑定 binary hash」）
- 期望与实际：§T15-10 第 7 行记 `f6bc9e2a…（22 434 320 B）`；实际 QA 两次独立重建与磁盘产物均为 **22 494 896 B**，且哈希与 `artifacts/web-mvp/t15-rd/dist-hash.txt` 一致（哈希相同 ⇒ 同一字节序列，体积必须相同）
- 影响：仅影响文档可信度；不影响产品行为与发布门禁。发布报告（T22）应以实际 `stat`/`SHA256SUMS` 为准
- 建议：RD/协调者修订该数字（或说明其来源），避免后续误把体积当作比对依据
- 证据：`qa-dist-1.log`、`qa-dist-2.log`、`qa-source-hashes.txt`、`artifacts/web-mvp/t15-rd/dist-hash.txt`

### P3-3｜UI 侧提示与 Playwright 覆盖仍待 T16/T17（本卡只验 API 侧）

- 严重度／状态：P3 / OPEN（范围提醒，非本卡缺陷）
- 对应 REQ / AC：AC-048（界面提示）、AC-039（UI 文案）、AC-037（任务中心显示对账入口而非重试按钮）
- 期望与实际：本卡只交付端点与 DTO（`notices`、`notice`、`stageSummary`、`needsInput`、`knowledgeProduced`、`draftId`）；`import-flow.spec.ts` / `job-recovery.spec.ts`（T16/T17）尚未实现，前端 `apps/web/src` 中亦无 jobs/drafts 页面
- 影响：不能据本卡 PASS 宣称「界面已提示需人工确认后才能发布」
- 证据：`apps/web/src`（features: auth/import/library/settings/shell）、PRD §7 切片表

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实供应商（T23）**：真实 Tripo 账户的 `attachRemoteTask`（需真实任务 ID 与账户可访问性）、取消后远端实际终态与退款/计费核对、真实结算金额与 `manual_ai` 预留的最终状态。
2. **`manual_ai` 预留的收尾**：本卡观察到说明书 AI 预留长期停留在 `reserved`（`actual=null`），未被 `settle/release`（RD 冒烟 A 段同样如此）；属 T14/T23 的账务收尾语义，本回合未构造真实用量结算场景，仅记录观察。
3. **「部分草稿 → 自动补齐为完整草稿」的公开 API 链路**：见上「放宽核对结论」第 5 点（`requeue_succeeded_dependents` 的可达性受限；QA 用注入内容变化 + 崩溃重跑验证了 upsert 语义，未构造端到端补齐）。
4. **多批分支暂停的完整恢复链**：批次 A `submission_unknown` + 批次 B 被暂停（`manual_branch_paused_by_unknown`）→ 对账 A → 再重试 B 的链路未在 T15 构造（批次级暂停由 T14 验收；本卡的 `branchSubmissionUnknown` 拒绝在单批场景覆盖）。
5. **写入竞争与忙等**：`BEGIN IMMEDIATE` 与 `busy_timeout=5s` 的极端竞争（沿用 T14 结论，未在 T15 重测）。
6. **规模**：本回合最多 6 页 / 2 批；100 页（20 批）与 `GET /jobs` 的 N+1（分页 ≤100）未压测。
7. **前端/浏览器**：本卡无 UI 交付；Playwright 未执行（无相关 spec）。
8. **正式发布包生成链路**：本回合只跑 `smoke-bootstrap`（内嵌页面/静态资源/health/JSON 404/SPA 路由）；「发布二进制 + 冷目录 + 完整两分支生成」属 T22/T23。
9. **既有 `#[ignore]` 用例（跨回合卫生）**：`qa_t10_independent::qa_total_wait_budget_must_trigger_on_real_poll_shape` 与 `qa_t11_independent::qa_defect_get_estimate_reflects_confirmation_and_consumption` 仍带「修复后移除 ignore」注释（BUG-003/BUG-004 已于回合 11/13 修复并 PASS）；建议协调者决定启用或更新注释。

## 非代码知识与限制（跨卡复用）

1. **原始 TCP fixture 的路由要三元组**（新，跨卡复用）：`(方法, 路径精确/前缀, 请求体标记)` 命中即消费脚本，能同时驱动「同一 host 上的多供应商多路径」（Tripo 上传/提交/查询/CDN + 说明书 AI），比按到达顺序的队列更抗调度漂移；未命中必须**显式失败**（QA 用 501 + unexpected 列表），否则会把路由错误伪装成产品失败。
2. **lsof 观测的正对照做法**（新）：把 fixture 的响应延迟做成环境变量开关（`QA_T15_SLOW_MS`，默认 0，不影响正常跑），用同一条命令跑「常规」与「正对照」两遍：常规跑 ESTABLISHED 仅 2 行（全部回环），正对照 ESTABLISHED 762 行（全部 `127.0.0.1→127.0.0.1`）——「非回环 0」因此有灵敏度背书。
3. **`recover::plan` 的 P3-1 修正口径**（复核结论）：恢复补推进以 `usage_json.producedKnowledge` 为准，且**只认显式 `false`**；字段缺失仍是 T10 语义。任务详情的 `stages[].knowledgeProduced` 与恢复判据同源——后续卡不要另起判据，避免展示与恢复再次分叉。
4. **`tripo_poll` 成功即结算**：`settled` 不占预算 → 任何 Model 分支阶段的 retry 都会被 `budgetNotHolding` 拒绝（P3-1）。写测试或做 UI 时不要把「模型侧可重试」当作既有事实。
5. **草稿 revision 的递增基准**：状态 PATCH 也会 +1（`needs_review→ready` = `r2`），内容变化在**当前** revision 上再 +1；断言不要以「首次组装时的 revision + 1」为基准。
6. **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「T15 验收知识」（上述 1–5 与 P3 摘要）；`llmdoc/validation-release.md` 命令合同**无变化**（本回合未发现需要修订的验收命令或阈值）；本报告即验收依据与未覆盖边界的正式记录。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–15 | 2026-09-12 | slice | T01–T13 | 1/2 | PASS（回合 10、12 各一次 FAIL → 修复后 PASS） | 本文件上半部分 |
| 16 | 2026-09-12 | slice | T14 | 2（ui_revision 2） | **PASS** | 本文件 |
| 17 | 2026-09-12 | slice | T15 | 2（ui_revision 2） | **PASS** | 本节 |

- 交接给协调者：(a) `qa_result: PASS`（仅 T15 切片）、`qa_round: 17`、`qa_history` 追加回合 17（PASS / slice / [T15]）；(b) `open_defects` 仍为空（3 条 P3 观察不构成缺陷）；(c) **T15 进入 `accepted_tasks`**：`{task_id: T15, prd_revision: 2, qa_round: 17, accepted_ac_ids: [AC-047, AC-048, "AC-037/039/040 端点侧"], status: accepted}`——AC-048 的 **UI 文案侧**与 AC-039 的 **UI 文案侧**归 T16/T17，AC-037 的「任务中心对账入口」UI 归 T17；(d) **里程碑 M2（T12–T15）本卡为最后一张，T15 PASS 后 M2 的生成闭环切片全部验收完毕**（M3 的 T16–T20 尚未开始）。
- 交接给 RD：**无必须修复项**。可选后续：P3-1（模型分支头重试门槛与其文案一致性）、P3-2（§T15-10 的体积数字）。
- 交接给 PM：无需求歧义需裁定。如需变更「父 job 在部分组装时的状态口径」（当前收紧为不置 succeeded）或「模型分支头是否需要免预算重试入口」，请开需求/修订，QA 不自行改口径。
- 全项目状态提醒：T15 PASS **只覆盖「两条分支组装草稿 + 任务控制端点」切片**（QA 构造 fixture，零真实外网、零真实付费）；T16–T23（向导/任务中心/阅读器/热点校准/发布/导出备份/正式单文件 smoke/真实授权链路与目标平台）仍未实现/验收，**MVP 尚未完成**。
- QA 产出：`crates/server/tests/qa_t15_independent.rs`（11 条常驻用例，无 `#[ignore]`；QA 新增，属允许写入范围）；证据目录 `artifacts/web-mvp/t15-qa/`——`qa-t15-independent.log`、`qa-t15-independent-3rounds.log`、`qa-t15-independent-nocapture.log`、`qa-pipeline-baseline.log`、`qa-workspace-tests.log`、`qa-xtask-check-final.log`、`qa-contracts-check.log`、`qa-dist-1.log`、`qa-dist-2.log`、`qa-smoke-bootstrap.log`、`qa-clippy.log`、`qa-lsof-loopback-only.sh`、`qa-t15-lsof-loopback-only.log`、`qa-t15-slow-lsof-loopback-only.log`、`qa-source-hashes.txt`。

# 回合 18 · T16（资料库和新建向导）

**结果：FAIL（仅 T16 切片；1 条新增缺陷 BUG-005（P2、OPEN、阻断 UI-005 行布局）→ 交 RD 修复后由 QA 复验）** · 回合：18 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T16]；AC-026、AC-030（Playwright 侧）、AC-048（UI 侧）+ 卡内 UI 条目 UI-005/006/009–026 + 回归链 + 4 条风险点专项）

- QA 执行时间：2026-09-12 16:05–17:15（本地 UTC+8）；执行者：QA 子 agent（回合 18）。
- **独立性声明**：`implementation.md` §T16 与 `artifacts/web-mvp/t16-rd/**`（含 17 张截图与 `final-chain2.log`）**只作线索，未用作任何通过证据**。本回合结论全部来自 QA 现场执行：
  - **QA 自写 12 条 Playwright 用例** `apps/web/tests/e2e/qa-t16-independent.spec.ts`（QA 新增，属允许写入范围）：资料库三态与禁用入口、第 2/3/5 步刷新、AC-030 逐项与未确认拒绝、幂等键复用与重放/冲突、受理后文案与禁用措辞、同视图占用 422、缺侧面视图、第 2/5 步失败态恢复、窄屏/桌面断点、磁盘满文案（注入服务端形状 413）、无准备指针诚实性、行布局几何（BUG-005 复现，`test.fixme` 保留）；
  - **主动复跑 RD 的 `import-flow.spec.ts`（6 通过）**：只作回归线索，不作为任何 AC 的通过依据（QA 自有用例独立断言同一批语义）；
  - 证据来自 DOM 文本、网络层请求/响应头、服务端事实（`GET /jobs|photos|documents`）、`getComputedStyle`／`boundingBox` 几何与 24 张现场截图；
  - **零外网观测（QA 自采样三路）**：① 浏览器 `route` 阻断并断言 `external == []`（3 个用例）；② 服务端日志出现的 URL 主机只有 `127.0.0.1:1`（假 Provider，连接被拒）与 `127.0.0.1:18080`（自身），无任何非回环 URL；③ `qa-lsof-loopback-only.sh` 12 轮采样本套件进程族（playwright/vite/serve/chrome-headless-shell）得到 **0 条非回环活动连接**（`qa-t16-lsof-loopback-only.log`）。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）＝ 派发包；REQ-016/REQ-021/REQ-030 与 AC-026/AC-030/AC-048 仍为 [必选]。`state.yaml`：phase=qa_running、prd_revision=2、ui_revision=2、current_tasks=[T16]、open_defects=[]、qa_round=18。
- 本回合不覆盖：T17–T23（任务中心/轮询、草稿页与发布、阅读器与热点校准、导出备份、正式单包冷启动、真实授权链路）；AC-048 的「生成成功后」UI 子句（见未覆盖边界 1）。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb`（T01–T16 仍未提交）；QA 新增仅 `apps/web/tests/e2e/qa-t16-independent.spec.ts` 与 `artifacts/web-mvp/t16-qa/**`；关键源码/测试/构建产物 sha256 见 `qa-source-hashes.txt` |
| 平台／浏览器 | macOS Darwin 25.6.0 · aarch64-apple-darwin；node v26.0.0 · npm 11.12.1；rustc/cargo 1.98.1；Playwright 1.60.0（chromium headless shell build 1223，`chromium_headless_shell-1223`） |
| 前端 | Vite 8.3.0 dev（e2e 端口 15173，`/api` 代理 127.0.0.1:18080）＋ `npm run build` 产物 `dist/assets/index-RBZqRvKh.js`（384.52 kB，sha256 `9857eec2…`） |
| 后端 | 真实 Rust 二进制（debug，e2e globalSetup 构建）＋ 每轮全新临时 data-dir（`init` 自动建管理员）＋ `price-catalog.example.toml` 副本；`providers.*.base_url = http://127.0.0.1:1`（保留端口，连接必被拒），密钥为套件专用假环境变量 `EM_E2E_TRIPO_KEY/EM_E2E_MANUAL_AI_KEY` |
| 数据／fixture | e2e 造数全部走公开 HTTP 合同（items/documents/photos/preparations）；页图/页文字为 T05 fixture；**0 次真实外网／0 次付费调用**（三路观测见上） |
| 未验证 | 真实 Provider 链路与真实结算（T23）；真实磁盘满的 statvfs 路径在浏览器链路的端到端复现（本回合以注入服务端形状响应 + T06 Rust 用例覆盖）；加密/超页 PDF 拒绝（T09 已验收，未重跑）；100 页规模；「生成成功后」的草稿/发布 UI（T18/T19） |

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/t16-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `npm --prefix apps/web run test:e2e -- import-flow.spec.ts`（复跑 RD spec，回归线索） | exit 0，**6 passed / 0 failed**（18.7 s）（`e2e-rd-spec.log`） |
| 2 | `npm --prefix apps/web run test:e2e -- qa-t16-independent.spec.ts`（**QA 新增**，终版） | exit 0，**11 passed / 0 failed / 1 skipped**（skipped = QA-10 因 BUG-005 标 `test.fixme` 保留复现）（`e2e-qa-independent.log`） |
| 3 | 同上 `-g QA-10`（BUG-005 复现，fixme 前实测） | exit 1，名称列宽 **22.8px**（1280px 视口，期望 ≥120）；几何明细 `library-row-geometry.json`（`e2e-qa-layout-probe.log`） |
| 4 | 同上 `-g QA-4`（幂等重放/冲突补充断言） | exit 0，**1 passed**（同键同 body 重放 202 + `x-idempotent-replay: true`；同键不同 body 409；仍 1 个 job）（`e2e-qa4-replay.log`） |
| 5 | `npm --prefix apps/web run test:e2e`（全量，终版） | exit 0，**33 passed / 0 failed / 1 skipped**（22 既有 + QA 11；skipped = BUG-005 复现）（`e2e-full.log`） |
| 6 | `npm --prefix apps/web run typecheck` | exit 0（`web-checks.log`） |
| 7 | `npm --prefix apps/web run lint` | exit 0，0 warning（`--max-warnings=0`）（`web-checks.log`） |
| 8 | `npm --prefix apps/web run test -- --run` | exit 0，**9 files / 62 passed**（`web-checks.log`） |
| 9 | `npm --prefix apps/web run build` | exit 0；`index-RBZqRvKh.js` 384.52 kB + `PreparePage-CyhHv_04.js` 442.69 kB + `pdf.worker.min-Dswkl-cV.mjs`；`vendor/pdfjs`（cmaps=169/standard_fonts=16/wasm=13/iccs=2）写入（`web-checks.log`） |
| 10 | `cargo xtask check` | exit 0，**fmt / clippy `-D warnings` / workspace 测试 / npm lint / typecheck / vitest / contracts --check 全部 `[通过]`**（`qa-xtask-check-full.log`） |
| 11 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`（`cargo-checks.log`） |
| 12 | `cargo test --workspace` | exit 0，**471 passed / 0 failed / 3 ignored**（30 个目标逐目标求和；3 ignored 为既有 BUG-003/BUG-004 复现 + T07 doctest，与 T13–T15 口径一致）（`cargo-checks.log`） |
| 13 | `bash artifacts/web-mvp/t16-qa/qa-lsof-loopback-only.sh <log> 12` | exit 0；12 轮采样本套件进程族，**非回环活动连接 0**（`qa-t16-lsof-loopback-only.log`） |
| 14 | dist 内嵌 UI 复核（dist 二进制 + 临时 data-dir + curl） | 见「发布产物」小节 |

## AC 验收矩阵（逐子句）

| AC / 子句 | 期望（可观察） | 实际（QA 实测） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| AC-026 · 刷新后已上传资料不丢 | 中途刷新后资料仍在 | 第 2 步（说明书）、第 3 步（front/left 照片）、第 5 步（报价恢复、勾选复位、生成仍禁用）刷新前后服务端事实一致；`POST assets/photos` 计数 **0**；第 5 步刷新后准备状态仍由服务端 `GET /preparations/{id}` 判定（缺项显示「资料已齐」） | **PASS** | QA-2；截图 `05/06/07-*` |
| AC-026 · 缺 front／视图不足时生成不可用并说明缺项 | 按钮禁用 + 缺项文案常驻 | 缺 front：`generation-gaps` 列出「缺少 front（正面）视图照片」+「去补充视图」链接、按钮 disabled、原因与按钮 `aria-describedby` 关联；只有 front 缺侧面：列出「缺少侧面视图：left/back/right 至少需要一张」、且**前置不满足时不请求报价**（`POST /estimates` 计数 0）；detail 槽位常驻「不发送给 Tripo」 | **PASS** | QA-6/QA-7；截图 `16/17-*` |
| AC-026 · 重复点击/重试只产生一个 job（服务端幂等） | 恰 1 个 job | ① 失败重试：首次 POST 被真实发出后丢弃响应（模拟响应丢失）→ 重试请求的 `Idempotency-Key` 与首次**完全相同**（网络层实测）→ 202 + `x-idempotent-replay: true`；② 直接打合同：同键同 body 重放 202 同 job，同键不同 body 409 `IDEMPOTENCY_CONFLICT`；③ 全程 `GET /jobs?itemId=` **恒为 1 个**且与页面显示 id 一致；④ 提交中按钮 disabled + `aria-busy=true` | **PASS** | QA-4；`e2e-qa4-replay.log` |
| AC-026 · 所有页面有可观察的空/加载/失败态与可行动错误 | 三态可观察、错误可行动 | 资料库：空态（「还没有物品」+主按钮）／延迟响应下骨架「正在加载物品…」／503 后「加载失败」+重试（点击恢复）；第 2 步：清单 503 →「说明书清单加载失败」+重试恢复；第 5 步：报价 503 →「无法获取报价」+「重试获取报价」+「查看服务状态」；视图槽位空态；占用 422 → 真实服务端拒绝的可行动文案 | **PASS** | QA-1/QA-6/QA-8；截图 `01/02/03/18/19-*` |
| AC-030 · 确认页逐项列出发送范围 | 视图／页范围／页图／型号文本／模型名／价格版本／预算上界 | 左栏：`Tripo（credits）30.00 credits 保守上界`、`说明书 AI（USD）0.008504 USD 保守上界`（分列不相加，逐字等于独立 API 报价的 `upperBoundDisplay`）、价格版本 2026-09-11、快照日期、`pageCount/pageRange`、`expiresAt` 倒计时；右栏 `send-scope`：Tripo 视图（正面/左侧 + photoId + sha256）、模型 `v3.1-20260211`（预设 + 参数 face_limit/texture/pbr/quality）、「发送给说明书 AI」的物品身份文本（名称·型号）、页范围、页文字页、页图页、模型 `gpt-5-mini` + prompt 版本 + maxOutputTokens | **PASS** | QA-3；截图 `08-*` |
| AC-030 · 默认不预勾选 | 勾选框默认未选中 | 原生 checkbox 默认未勾选；未确认前 `POST .../confirm` 计数 **0**；生成按钮禁用且原因写「需要先勾选确认」；页面无「全选」预勾选入口 | **PASS** | QA-3/QA-2 |
| AC-030 · 未确认的提交被拒（不建 job） | 422 且无 job | 直接打合同：`POST /items/{id}/jobs`（合法 body + 有效报价 + 新幂等键）→ **422**，`details.reason=confirmationRequired`；随后 `GET /jobs?itemId=` = **0**；确认后（勾选 → `/confirm` 恰 1 次 → 页面「已确认发送范围（时间）」）才可提交 | **PASS** | QA-3 |
| AC-048（UI 侧）· 无自动发布路径 | 界面无发布入口 | 受理面板与全页：无「自动发布/一键发布/发布成功/已发布」文本；`role=button/link` 名为「发布」的元素数量 **0**；文案只写「任务已受理（202）…不代表生成结果：请到任务中心查看阶段状态（任务中心由 T17 交付）」 | **PASS** | QA-5；截图 `14-*` |
| AC-048（UI 侧）· 生成成功后提示需人工确认才能发布 | 生成完成后界面提示 | **NOT_RUN**：本回合 Provider 指向 `127.0.0.1:1`，job 永远停在 queued/retry（不可达「生成成功」状态），草稿页横幅属 T18/T19；本轮只证明「当前不存在任何自动发布路径与入口」（与 T15 的服务端 `manual_releases=0` 证据合起来覆盖该 AC 的可测部分） | **NOT_RUN（范围外）** | 未覆盖边界 1 |

## 卡内 UI 条目矩阵（PRD §6.2）

| UI ID | 期望要点 | QA 实测 | 结论 |
| --- | --- | --- | --- |
| UI-005 | 库列表/空态/分页/归档开关/行内入口 | 空态、骨架、失败+重试、归档开关、搜索范围提示、游标提示均在；`继续准备` 可用；`查看任务`/`打开说明书` **disabled + title + 常驻原因**（不假装可用）——**但行布局被 T16 新增的说明文字挤坏（BUG-005）** | **FAIL**（BUG-005） |
| UI-006 | 表单字段与 REQ-010 一致、**无物品级来源链接** | 字段恰为 名称/准确型号/品牌/变体；页面注明「物品不保存来源链接」；实测 `POST /items` 请求体键集合 = `{brand,model,name,variant}`（无 sourceUrl） | PASS |
| UI-009 | 上传控件、字节进度、413 分流、磁盘满文案 | 上传完成后出现资产/槽位（QA-5 全流程 UI 真实上传）；复跑 RD spec（QA 亲跑，6 通过）内含真实 21 MiB 照片 → 413 错误卡片「文件超过大小上限」+ 重试/移除；`insufficientStorage` 分支 QA 用**服务端形状注入**（字段/文案逐字取自 `crates/server/src/assets/{error,blob_store}.rs`）→「磁盘空间不足」+「需要 10 MiB／当前可用 1 MiB」+ 清理提示 + 重试/移除，且未产生任何资产 | PASS（真实磁盘满未触发，见未覆盖边界 2） |
| UI-010 | 失败重试/移除、无孤儿 | 注入断连 → 错误卡片「上传失败」+ 重试；重试真实重发并成功替换为缩略图；「移除」后卡片消失；服务端照片集合与预期一致 | PASS |
| UI-011 | 绑定说明书、出处链接仅记录 | 上传→「待绑定文件」→绑定→「已绑定的说明书」（标题/sha256/绑定时间）；出处字段旁固定提示「来源链接仅作出处记录，服务器不会访问该地址。」；全程无对该地址的请求（external 阻断断言 + 服务端日志无非回环 URL） | PASS |
| UI-012 | 五槽位、同视图占用、detail 说明 | 五槽位（front 标「必需」、detail 注明「不发送给 Tripo」）；并发窗口下第二张 front → **真实 422 `viewOccupied`** → 「已有照片…可用『替换照片』更新内容，或把另一张照片改到其它槽位」；刷新后按服务端事实显示；`POST /estimates` 的 `photoIds` **不含 detail**（实测 eq `[front,left]`） | PASS |
| UI-013 | 缺项提示与按钮关联 | `generation-gaps` 常驻（词表与 `crates/server/src/generation/estimate.rs` 的 `missingFrontView/missingSideView/preparationNotReady` 一致）；每条给「去修复」链接；按钮 `aria-describedby=generate-reason` | PASS |
| UI-014–UI-018 | 第 4 步逐页进度、可取消、关闭提示、拒绝文案 | 复用 T09 未重写；DOM 实测进度为「第 n / N 页」「已完成 n / N 页」+ `role=progressbar`（`aria-valuenow`=已完成页数），正文**无 `%`／无「总进度」／无「预计剩余」**；常驻「准备需要保持本标签页打开；关闭标签页会中断准备…」；封存后「准备完成（ready）」+ clientDerived 说明。加密/超页拒绝属 T09 已验收（未重跑） | PASS |
| UI-019 | 五步 URL 导航、刷新恢复 | 步骤条 `aria-current="step"` 正确；上一步/下一步是纯 `<Link>`（只改 URL）；第 2 步未绑定说明书时「下一步」禁用并给原因 | PASS |
| UI-020/025 | 防重复提交、202 只表示入队 | 提交中「正在创建任务…」+ disabled + `aria-busy=true`；202 后进入受理面板并显示 job id；无「生成成功」类客户端预测文案 | PASS |
| UI-021 | 冻结快照提示 | 物品页/编辑页常驻提示「进行中任务使用冻结的资料快照…修改用于下次生成，需要重新报价并确认；本版本不提供修改使用中快照的入口」；查询失败静默（不升级为页面错误） | PASS |
| UI-022/023/026 | 分列报价、过期手动重报、拒绝恢复 | 分列 credits/USD（不与相加）、价格版本/快照/有效期倒计时、保守上界与 `budgetNotice` 原文；过期注入 → 「已过期」+ 生成禁用 +「重新获取报价」（不自动重报）；预算低于上界 → 禁用 + 原因；提交失败按 `quoteExpired/inputChanged/IDEMPOTENCY_CONFLICT/quoteAlreadyUsed/budgetBelowPlannedUpperBound/confirmationRequired` 分别给恢复路径（代码核对 + 409/422 实测） | PASS |
| UI-024 | 告知与确认、默认不勾选 | 见 AC-030 三行；勾选框为原生控件 + `aria-describedby` 说明「默认不勾选；勾选动作写入服务端审计事件」 | PASS |

## 4 条风险点专项核对（派发包指定）

1. **禁用入口不得假装可用 —— 通过（1 条观察）**：资料库行内「查看任务」「打开说明书」为真 `disabled` 按钮 + `title`（T17 / T18+T19）+ 行内常驻原因文本（截图 `04-*`）；物品页「下一步」区写明「任务中心（T17）与阅读器（T18/T19）尚未交付…」；`/jobs`、`/jobs/:id`、草稿、releases、阅读器均为占位页且写明「尚未实现/还没有实现」（QA-5 实测占位页文案）。**观察（非缺陷）**：顶栏「任务中心」是可用链接 → 指向占位页（页面第一屏写明尚未实现），属"不假装可用"的可接受形态；T17 交付时应替换该占位页。
2. **禁止线性总百分比 —— 通过**：准备页 DOM 实测「第 n / N 页」/「已完成 n / N 页」，页面正文不含 `%`、不含「总进度」「预计剩余」（QA-5 断言）；报价页只有倒计时与上界；受理页无任何进度百分比。禁用措辞的覆盖面 = QA e2e 正文扫描（准备页 + 受理后的确认页，清单 `总进度/预计剩余/已自动校准/自动发布/一键发布/零费用/离线可用/已证明页图来自原 PDF/重试不会重复收费`）+ QA 亲跑 vitest 中的 `shell.test.tsx`（对资料库渲染 HTML 的 8 条清单）+ QA 源码级 grep（`apps/web/src` 命中仅为注释中对规则的引用，非展示文案）。
3. **前端不判断远端成功 —— 通过**：受理面板只写「任务已受理（202）…不代表生成结果」；网络错误时页面写「提交被拒绝 / 无法连接服务…网络错误没有产生第二次建单：重试会复用同一个幂等键」，**不写"成功"**；全页文本扫描无 `生成成功|已发布|发布成功|已经发布`（QA-4/QA-5）。
4. **幂等键确实由前端复用 —— 通过**：网络层实测同一轮提交的两次 POST `Idempotency-Key` 相同（首次响应被丢弃后重试），服务端回 `x-idempotent-replay: true`，`GET /jobs?itemId=` 恒 1 个（QA-4）；前端在报价刷新/422/幂等冲突后重置键的代码路径已核对（`ConfirmStepPage.submit`）。

## 缺陷

### BUG-005 · 资料库列表行被 T16 新增的说明文字撑坏：身份列在 1280/1366px 计算宽度 0px，名称逐字换行，单行高 358px

- 严重度／状态：**P2 / OPEN**（T16 引入的 UI-005 布局回归；阻断本切片 PASS，修复后由 QA 复验）
- 对应 REQ / UI / AC：REQ-010 / **UI-005**（"成功：行显示名称、型号、状态、最近使用时间"）/ AC-016 的消费侧
- 环境与输入：真实 Chrome（Playwright chromium）+ 真实后端；视口 1024–1920px；任意一行（本次测量物品名「QA 行布局测量」，型号 `T16-QA 行布局测量`）；行内入口与说明文字均为 T16 新增（T08 的同一行渲染正常，见 `artifacts/web-mvp/t08-rd/08-library-with-item.png` 对照）
- 复现步骤：① `npm --prefix apps/web run test:e2e -- qa-t16-independent.spec.ts -g QA-10`（当前因 `test.fixme` 跳过；去掉 fixme 即为复现，实测 exit 1）；② 或登录后打开 `/`，在 1280×900 视口测量 `.item-row__name` 的 `boundingBox()` 与 `.item-row` 的 `getComputedStyle().gridTemplateColumns`
- 期望与实际：
  - 期望：`.item-row` 的 `grid-template-columns: minmax(0,2fr) 1fr 1fr auto` 中身份列占主要宽度（T08 时代名称/型号单行可读）；说明文字位于独立的整行，不参与列宽竞争
  - 实际（`library-row-geometry.json` 实测）：1280px 时计算网格 `0px 31.6px 45.6px 702.8px`——**身份列 0px**、名称元素宽 **22.8px**（逐字换行、高 146px）、状态列 31.6px、时间列 45.6px、actions 列 702.8px；单行高 **357.7px**；1366px 同为 **0px 身份列/22.8px 名称**；1024px 名称 32px；1440px 48px；1600/1920px 90.7px（行高仍 127px）。即 1024–1920px 全区间均不达标（QA-10 阈值 120px）
- 根因（QA 代码级定位）：`.item-row__actions` 无任何 CSS 规则（`styles.css` 中不存在 `.item-row__actions{display:flex}`），是普通 block；其内部 `.item-row__note` 是 inline 文本（`flex-basis:100%` 因父级非 flex 容器而失效）。grid 末端 `auto` 轨道按说明文字 max-content（约 702px）定宽，三个 `fr` 轨道被压缩到合计约 77px → 身份列 0px。**T16 新增的整句禁用原因文字**（`LibraryPage.tsx` 的 `.item-row__note`）是触发条件
- 影响：资料库是全站入口页，任意物品行在常见桌面宽度（1280/1366）下名称/型号/状态/时间均不可读（逐字竖排、单行近 358px 高，20 行将超过 7000px）；不影响键盘可达性、数据正确性与其它页面的功能；不涉及费用/安全
- 证据路径：`artifacts/web-mvp/t16-qa/library-row-geometry.json`（6 个宽度的几何 + `getComputedStyle` 采样）、`e2e-qa-layout-probe.log`（fixme 前实测 exit 1，Received 22.765625）、截图 `screenshots/22-library-row-geometry.png` 与 `04-library-row-disabled.png`；对照 `artifacts/web-mvp/t08-rd/08-library-with-item.png`
- 回归范围：`apps/web/src/features/library/LibraryPage.tsx`（行内入口与说明文字）与 `apps/web/src/styles.css` 的 `.item-row*` 规则；修复后需在 1024/1280/1366/1440/1600/1920px 六档复核（QA-10 已固化为回归守卫，RD 修好后请移除 `test.fixme` 并转绿）
- RD 修复摘要引用：`implementation.md` **§T16-FIX**（2026-09-12，RD 修复回合）——`.item-row__note` 移出 `.item-row__actions` 成为行直接子元素（`grid-column: 1 / -1`）、四列显式 `minmax(0, …)` + `.item-row > * { min-width: 0 }`、`.item-row__actions` 改 `display:flex; flex-wrap:wrap`、窄屏单列覆盖移到基础规则之后（修复媒体查询源顺序死规则）。
- QA 复验结果与日期：**CLOSED（2026-09-12，回合 19）**——解除 QA-10 `test.fixme` 后该用例 **1 passed**；QA 独立实测名称/身份列 1024–1920px = **296.0/228.0/271.0/308.0/346.0/346.0px**（阈值 120）、行高 **108.7–129.1px**（阈值 140）、说明文字独占整行且不在四列竞争内；窄屏 767/375px 恢复单列堆叠（计算网格 1 轨道、名称 695.0/303.0px）且摘要抽屉可开可关；禁用原因的 `aria-describedby` 关联与可见性完好。详见本文件「回合 19」节。

**其它缺陷：无。**（本回合其余项全部 PASS；下述观察不阻断）

## 非阻断观察（供 RD/T17/T21/PM）

1. **顶栏「任务中心」入口指向占位页**（T17 未交付）：占位页写明「还没有实现」，属如实呈现；建议 T17 交付时替换并从 `PlaceholderPage` 移除。
2. **向导第 5 步依赖 `sessionStorage` 会话指针**（RD ADR-026 第 1 条已记录）：QA 独立复现了"无指针"的诚实处理——缺项显示「还没有可用的资料准备记录」+「去准备」链接、生成禁用、**不请求报价、不伪造状态**；换浏览器/清存储需回第 4 步重进（建议后续卡加 `GET /items/{id}/preparations`，属合同新增）。
3. **e2e 证据目录与 RD 共享仍会互相覆盖**（T09 回合 9 的 P3-2 仍开放）：`playwright.config.ts` 固定写 `artifacts/web-mvp/t09-rd/`；本回合 4 次 e2e 运行再次覆盖了该目录的 `e2e-server.log` 与 `playwright-output/`。QA 已把最后一次服务端日志另存为 `artifacts/web-mvp/t16-qa/e2e-server.qa-t16.log`；建议 T21/T22 前改为按运行/角色隔离（`PLAYWRIGHT_*` 环境变量或时间戳子目录）。
4. **AC-030 的 `audit_events` 断言仍在服务端侧**（T11 用例）：本回合只观察「勾选 → `POST .../confirm` 恰 1 次 → 页面『已确认发送范围（时间）』」，未核对审计行。
5. **进入第 5 步会自动获取一次报价**（RD 观察）：同输入签名只请求一次、不收费、不写账本；与 U-05（过期不自动重报）不冲突。
6. **真实 413 超限的错误卡片文案为「文件超过大小上限」**（`details.reason` 缺省分支）：符合 UI-009 的分流要求（未落成通用错误）；`insufficientStorage` 分支以注入实测（见 UI-009 行）。

## 未覆盖边界（本回合记录，不冒充通过）

1. **「生成成功」之后的 UI（AC-048 的 UI 子句）不可达**：Provider 指向 `127.0.0.1:1`，job 只到 202 受理；草稿页横幅「已生成可复核草稿（needs_review）——生成完成不等于已发布」属 T18/T19 交付物。AC-048 的可测部分（无自动发布路径/无发布入口/不判断远端成功）本回合 PASS，其余标 NOT_RUN，**不得据本报告宣称 AC-048 整体通过**。
2. **真实磁盘满（`insufficientStorage`）未在浏览器链路端到端触发**：服务端 `SpaceProbe` 注入属进程内测试（T06 `assets.rs` 已覆盖 413 + `details.reason` + 不半提交）；本回合验证的是前端渲染与"无资产产生"。真实 statvfs 路径 + 写流中途 ENOSPC 的 500 边界（T06 的 P3）仍在。
3. **真实 Provider／付费链路（T23）**；**加密/超页 PDF 拒绝文案**（T09 已验收，本回合未重跑）；**100 页规模与准备性能**（T21）；**任务中心轮询语义**（T17）。
4. **e2e 内嵌 dist 文案逐串核对**：本回合做了 `npm run build` + `cargo xtask dist`（两次哈希一致）+ `smoke-bootstrap` + 关键文案 grep（见"发布产物"），但未逐串比对全部 UI 文案。
5. **断网/离线首次加载**（PDF.js 本地资源）属 T09 已验收范围，未重跑。

## 发布产物（T16 相关的单二进制路径）

| 项 | 结果 |
| --- | --- |
| `cargo xtask dist --target aarch64-apple-darwin`（两次） | 均 exit 0；两次 sha256 一致 **`5c33ce93adaea07d00560a353eeda97aecd3226abfed323868f2412dc08f48cf`**（22 544 432 B），与 `SHA256SUMS`、RD `artifacts/web-mvp/t16-rd/dist-final3.log` 三处一致（`qa-dist.log`） |
| `cargo xtask smoke-bootstrap --binary <绝对路径>` | exit 0；1 项 `[准备]` + 7 项 `[检查]` 全过（页面/静态资源 `index-RBZqRvKh.js`/health live+ready/`/api/unknown` JSON 404/SPA 深链接/缺失资源 404）（`qa-smoke-bootstrap.log`） |
| 内嵌 UI 复核（QA 自写脚本 `qa-check-embedded-strings.sh`：dist 二进制 + 临时 data-dir + curl，exit 0） | 内嵌 JS 含 6 条 T16 前端文案：「已受理（202）」「不代表生成结果」「发送给说明书 AI」「将发送的资料与确认」「重新获取报价」「生成 3D 与说明书草稿」；二进制字节另含服务端 `BUDGET_NOTICE` 两条（「不是供应商账户级硬封顶」「供应商实际计费以账单为准」）——预算语义文案来自服务端 `budgetNotice`，不在前端 JS（`qa-dist-embedded-strings.log`） |

## 非代码知识与限制（跨卡复用）

1. **Playwright `request.headers()` 的可见头**：实测 `Idempotency-Key` **可见**（本回合以它做幂等键复用断言）；CSRF 头是否可见见 `playwright-request-headers.json` 的采样（T09 用原生 CDP `requestWillBeSent` 时看不到 `x-csrf-token`，Playwright 与裸 CDP 行为不同——T17/T19 断言 CSRF 前先看该文件）。
2. **fixture 页码陷阱**：`tests/fixtures/assets/sample-manual-text.pdf` 是 **2 页**（PDF.js 实际解析）；用朴素正则数 `/Type /Page` 会得到 3。准备进度的断言请写 `/第 \d+ \/ \d+ 页/` 或读 PDF.js 的 `numPages`，不要写死页数（本回合第一版即因此误判一次）。
3. **行式列表新增整句说明文字的布局回归检查法**：给 grid 行加"常驻原因/说明"这类长文本时，`auto`／`max-content` 轨道会按文本 max-content 抢宽并把 `fr` 列压到 0——必须实测 `getComputedStyle(row).gridTemplateColumns` + `boundingBox()`（Playwright 两行代码即可），或在新增前把说明文字放到独立的整行容器。T17 的任务中心列表会加阶段/费用列，**同类风险高**。
4. **BUG-005 根因摘要**：`.item-row__actions` 无 CSS（非 flex 容器）而 `.item-row__note` 依赖 `flex-basis:100%` → grid `auto` 轨道被 702px 说明文字撑开，1280/1366px 下身份列计算宽度 0px（详见缺陷段）。
5. **e2e 证据目录共享**：见非阻断观察 3（T21/T22 前应隔离，否则每次运行覆盖前一角色/前一回合的 `e2e-server.log` 与 trace）。
6. **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「T16 验收知识」（上述 1–5 + 风险点结论摘要）；`llmdoc/validation-release.md` 命令合同**无变化**（本回合未发现需要修订的验收命令或阈值）；本报告即验收依据、缺陷与未覆盖边界的正式记录。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–16 | 2026-09-12 | slice | T01–T14 | 1/2 | PASS（回合 10、12 各一次 FAIL → 修复后 PASS） | 本文件上半部分 |
| 17 | 2026-09-12 | slice | T15 | 2（ui_revision 2） | **PASS** | 本文件 |
| 18 | 2026-09-12 | slice | T16 | 2（ui_revision 2） | **FAIL**（BUG-005 P2 阻断 UI-005 行布局；其余全部通过） | 本节 |

- 交接给 RD（**必须修复后由 QA 复验**）：**BUG-005（P2，OPEN）**——资料库行身份列被 T16 新增的整句说明文字挤到 0px（1280/1366px 下名称逐字换行、单行高 358px）。修复方向建议（RD 自行决定）：把禁用原因移出行内四列 grid（例如作为 `grid-column: 1 / -1` 的整行元素或行下方独立段落），或给 `.item-row__actions` 明确的布局与宽度上限（如 `display:flex; flex-wrap:wrap; max-width:…` 并让说明独占一行）。**不得**通过删除"禁用原因常驻"要求（REQ-039/AC-060 的"不能坏掉无解释"）、隐藏说明文字或改 PRD 来规避。修复后请在 1024/1280/1366/1440/1600/1920px 六档复核，并移除 `apps/web/tests/e2e/qa-t16-independent.spec.ts` 中 QA-10 的 `test.fixme`（该用例即回归守卫）。
- 交接给协调者：(a) `qa_result: FAIL`、`qa_round: 18`、`open_defects` 追加 `{id: BUG-005, severity: P2, ui: UI-005, status: OPEN, owner: rd}`；(b) `qa_history` 追加回合 18（result: FAIL，scope: slice，task_ids: [T16]）；(c) **T16 不进入 `accepted_tasks`**；(d) 除 BUG-005 外，AC-026 全部子句、AC-030（Playwright 侧）全部子句、AC-048 的"无自动发布路径"子句与全部卡内 UI 条目（UI-006/009–026）均已独立通过，缺陷修复后只需复验 BUG-005 + 全量回归（回合 19），**不要求重验全部 AC**；(e) QA 新增 `apps/web/tests/e2e/qa-t16-independent.spec.ts`（12 用例，其中 QA-10 因 BUG-005 `test.fixme`；后续全量 `test:e2e` 会一并运行）。
- 交接给 PM：无需求歧义需裁定。AC-048 的"生成成功后界面提示"子句、草稿页横幅与发布入口的最终形态属 T18/T19，请在该切片派发时保持原 AC 文本（QA 不在本轮自行接受偏离）。
- 全项目状态提醒：T16 的 FAIL **只覆盖「资料库与新建向导」切片**（真实浏览器 + 真实后端 + `127.0.0.1:1` 假 Provider，零真实外网/零付费）；T17–T23（任务中心/草稿与发布/阅读器与热点校准/导出备份/正式单包冷启动/真实授权链路）仍未实现/验收，**MVP 尚未完成**。
- QA 产出与证据：`apps/web/tests/e2e/qa-t16-independent.spec.ts`（12 用例）；`artifacts/web-mvp/t16-qa/`——`e2e-rd-spec.log`、`e2e-qa-independent.log`、`e2e-qa-layout-probe.log`、`e2e-qa4-replay.log`、`e2e-full.log`、`web-checks.log`、`cargo-checks.log`、`qa-xtask-check-full.log`、`qa-dist-1.log`、`qa-dist-2.log`、`dist-hash.txt`、`qa-smoke-bootstrap.log`、`qa-dist-embedded-strings.log`、`qa-lsof-loopback-only.sh`、`qa-t16-lsof-loopback-only.log`、`qa-source-hashes.txt`、`library-row-geometry.json`、`playwright-request-headers.json`、`e2e-server.qa-t16.log`、`screenshots/`（24 张）。

---

# 回合 19 · T16（BUG-005 修复复验 + 并发 POST /photos 500 复现定级）

**结果：PASS（仅 T16 切片；BUG-005（P2）→ CLOSED；新增 BUG-006（P2，OPEN，非 T16 改动范围、不阻断本切片）→ 交 RD／协调者作独立后端加固）** · 回合：19 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T16]；复验 BUG-005 + 全量回归 + RD 报告的并发 500 复现与定级）

- QA 执行时间：2026-09-12 16:40–17:10（本地 UTC+8）；执行者：QA 子 agent（回合 19）。
- **独立性声明**：`implementation.md` §T16-FIX 与 `artifacts/web-mvp/t16-rd-fix/**`（含 RD 几何表、`after-fix.log`、`qa10-recheck.log`）**只作线索**。本回合全部通过证据来自 QA 现场执行：
  - QA 自有用例 QA-10 **解除 `test.fixme`** 后的真实运行（断言未改动，阈值仍为名称/身份列 ≥120px）；
  - QA 自写**临时探针**（`qa-t16-r19-probe.spec.ts` / `qa-t16-r19-probe2.spec.ts`，运行后删除，原始 stdout 存证）复核窄屏单列、说明文字结构位置与 `aria-describedby` 关联；
  - QA **主动运行** RD 守卫 spec（只作交叉核对，不作为通过依据）并核对它写出的几何与 QA 自测一致；
  - QA 自写并发复现脚本三条（确定性外部写锁 / 自然并发 / 聚焦量化）+ 纯 SQLite 机制对照实验。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）＝ 派发包；`state.yaml`：phase=qa_running、qa_round=19、prd_revision=2、ui_revision=2、current_tasks=[T16]、open_defects=[BUG-005(fixed_pending_reverify)]。
- 本回合不覆盖：T17–T23；AC-048 的「生成成功后」UI 子句（与回合 18 相同，见未覆盖边界）；BUG-006 的修复（缺陷未修，QA 不改生产代码）。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb`；本回合 QA 唯一常驻改动 = 解除 `apps/web/tests/e2e/qa-t16-independent.spec.ts` QA-10 的 `test.fixme`（其余用例与文件其余部分未动）；关键文件 sha256 见 `qa-source-hashes.txt` |
| 平台／浏览器 | macOS Darwin 25.6.0 · aarch64-apple-darwin；node v26.0.0 · npm 11.12.1；Playwright 1.60.0（chromium headless shell）；Rust 1.98.1 |
| 前端 | Vite 8.3.0 dev（e2e 端口 15173，代理 127.0.0.1:18080）＋ `npm run build` 产物 `dist/assets/index-C4vQ2t8_.js` · `index-DNtWi9LH.css` |
| 后端 | 真实 Rust 二进制（debug，e2e globalSetup 构建；另用 `target/debug/everything-manual` 直接起临时 data-dir 做并发复现，端口 18091/18092/18093）+ 每轮全新临时 data-dir + `providers.*.base_url = http://127.0.0.1:1`（连接必被拒） |
| 数据／fixture | e2e 造数走公开 HTTP 合同；并发复现用 `tests/fixtures/assets/sample-photo-front.jpg`（真实 multipart 上传）；**0 次真实外网／0 次付费调用** |
| 未验证 | 真实 Provider 与结算（T23）；AC-048「生成成功后」UI；BUG-006 的修复后行为；100 页规模；真实磁盘满 statvfs 端到端 |

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/t16-qa-r19/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `npm --prefix apps/web run test:e2e -- qa-t16-independent.spec.ts -g QA-10`（**已解除 fixme**） | exit 0，**1 passed**（5.4s）（`e2e-qa10-unfixed.log`）；几何写入 `qa10-r19-library-row-geometry.json` |
| 2 | 临时探针 `qa-t16-r19-probe.spec.ts`（QA 自写，运行后删除） | exit 0，**1 passed**；1280/1024/1366/1440/1600/1920px 四轨道 + 767/375px **单轨道**堆叠 + 375px 抽屉开/Esc 关（`probe-r19-raw.log`，10 条 PROBE 行） |
| 3 | 临时探针 `qa-t16-r19-probe2.spec.ts`（运行后删除） | exit 0，**1 passed**；说明文字 id 可在 document 解析、两个禁用按钮 `aria-describedby` 均指向它、可见性非 none/hidden（`probe2-r19-raw.log`） |
| 4 | `npm --prefix apps/web run test:e2e -- library-row-layout.spec.ts`（RD 守卫，交叉核对） | exit 0，**2 passed**（`e2e-rd-guard-r19.log`）；其输出的 6 档几何与 QA-10 完全一致（`r19-rd-guard-geometry*.json`） |
| 5 | `npm --prefix apps/web run test:e2e`（全量，第 1 次） | exit 0，**36 passed / 0 failed / 0 skipped**（1.5m；`e2e-full-r19-run1.log`；后端日志 0 条 `database is locked`） |
| 6 | `npm --prefix apps/web run test:e2e`（全量，第 2 次，抖动复核） | exit 0，**36 passed / 0 failed / 0 skipped**（1.5m；`e2e-full-r19-run2.log`；0 条 `database is locked`；其 QA-10 几何与 #1 单独运行逐字段一致，见两份 `qa10-r19-*.json` 对比） |
| 7 | `npm --prefix apps/web run typecheck` / `run lint` / `run test -- --run` | 全部 exit 0；lint 0 warning；**9 files / 62 passed**（`web-checks-r19.log`） |
| 8 | `cargo xtask check` | exit 0，**7/7 `[通过]`**（fmt / clippy `-D warnings` / workspace 测试 / npm lint / typecheck / vitest / contracts --check；`xtask-check-r19.log`） |
| 9 | `cargo test --workspace`（xtask 内） | **471 passed / 0 failed / 3 ignored**（30 个目标求和，与 T13–T19 口径一致） |
| 10 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0；sha256 **`ed26b6a41ececa6514d854dd440bb4504c2b128df460a3409ac12b4132f66189`**（22 544 432 B）——**与 RD §T16-FIX-6 声称完全一致**（RD 两次一致；QA 独立复现第三次一致）（`dist-r19.log`） |
| 11 | `cargo xtask smoke-bootstrap --binary <dist 二进制绝对路径>` | exit 0；1 项 `[准备]` + 7 项 `[检查]` 全过（内嵌资源 `index-C4vQ2t8_.js`、health live+ready、JSON 404、SPA 深链接、缺失资源 404；在**仅含二进制的临时目录**内启动）（`smoke-bootstrap-r19.log`） |
| 12 | `bash repro-conc-photos-500.sh`（并发 500 确定性复现 A） | 见缺陷 BUG-006（`repro-A-writer-lock.log`） |
| 13 | `bash repro-conc-photos-500-natural.sh 40`（自然并发 B） | 见缺陷 BUG-006（`repro-B-natural-concurrency.log`） |
| 14 | `bash repro-conc-photos-500-focused.sh`（聚焦量化 C + 服务端日志留存） | 见缺陷 BUG-006（`repro-C-focused.log`、`repro-C-server.log`） |
| 15 | 纯 SQLite 机制对照（python3 + sqlite3，WAL/busy_timeout=5000） | 见缺陷 BUG-006 根因段（`sqlite-mechanism-control.log`） |

## BUG-005 复验矩阵（逐项）

| 项 | 期望（UI-005 / §6.1.1 / REQ-039） | QA 实测（回合 19） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| QA-10 断言（解除 fixme 后原样） | 1024–1920px 名称列 ≥120px | 6 档全过，**1 passed**（回合 18 同断言 fixme 前：1280px 测得 22.8px 失败） | **PASS** | `e2e-qa10-unfixed.log` |
| 身份/名称列宽（QA 独立量测） | ≥120px（QA-10 阈值；RD 声称 ≥228px） | 1024 **296.0** / 1280 **228.0** / 1366 **271.0** / 1440 **308.0** / 1600 **346.0** / 1920 **346.0**（px）；名称元素高 25.6px＝单行；与 RD 表逐档一致（交叉核对通过） | **PASS** | `qa10-r19-library-row-geometry.json`、`probe-r19-raw.log` |
| 行高（QA 独立量测） | 可读（RD 声称 108–129px；BUG-005 时为 358px） | 108.7 / **129.1**（1280 实测；时间列「2026-09-12 16:29」折行所致）/ 108.7 / 108.7 / 108.7 / 108.7 px；行高上限阈值 140（RD 守卫）未破 | **PASS** | 同上 |
| 说明文字不参与列宽竞争 | 独占整行、在操作行之下 | `noteIsDirectChild=true`、`gridColumn: 1 / -1`、note.y − actions.y = **59.4px**（1280: 69.6px）；note 宽 = 行宽 − 24px 内边距 | **PASS** | `probe-r19-raw.log` |
| 四列轨道语义 | 名称/型号/状态/最近使用（U-01） | 计算网格 4 轨道且为 `296px 148px 148px 324px` 等显式像素（不再是 0/31.6/45.6/702.8）；状态列显示「在准备」、时间列「更新于 2026-09-12 16:xx」 | **PASS** | `qa10-r19-library-row-geometry.json`、截图 `t16-qa/screenshots/22-library-row-geometry.png`（QA 本次运行重拍） |
| 窄屏 <768px 回归（顺带修的媒体查询死规则） | 单栏堆叠 + 抽屉（§6.1.1） | **767px：计算网格单轨道 695px**（名称 695.0）与 **375px：单轨道 303px**（名称 303.0）——死规则已生效；375px 抽屉「资料库摘要」可开、Esc 可关 | **PASS** | `probe-r19-raw.log`（PROBE 767/375 行 + drawer-375 行） |
| 禁用入口的解释未被弱化（REQ-039/AC-060） | 禁用原因常驻且与按钮关联 | 说明文字 `任务中心（T17）与阅读器（T18/T19）尚未交付…` 仍可见（display=block、visibility=visible）；两个禁用按钮 `aria-describedby` 指向同 id 且 `document.getElementById` 可解析；`继续准备` 链接仍为 `/import/prepare` | **PASS** | `probe2-r19-raw.log` |
| RD 守卫 spec 存在且非空转 | `apps/web/tests/e2e/library-row-layout.spec.ts` 断言几何 | 文件存在（2 用例）；**实断言**：名称/身份 ≥120px、行高 ≤140px、说明行宽 ≥ 行宽−32px、说明在操作行之下、轨道数=4（窄屏 =1、名称 ≥200px）；QA 本次运行其输出的 6 档数值与 QA 独立测量逐字段一致；其阈值对我回合 18 实测的 22.8px 必然失败 → 有判别力、非空转 | **PASS** | `library-row-layout.spec.ts`、`r19-rd-guard-geometry*.json`、`e2e-rd-guard-r19.log` |
| 全量回归 | 36 passed / 0 skipped | **两次均 36 passed / 0 failed / 0 skipped**（含 QA-10 转绿；RC `.item-row` 相关无其它用例回退） | **PASS** | `e2e-full-r19-run1.log`、`e2e-full-r19-run2.log` |
| 工程链与发布产物 | 不破坏既有证据链 | typecheck/lint/vitest 62 全过；`xtask check` 7/7；workspace 471/0/3；dist sha256 与 RD 声称一致（QA 第三次复现）；smoke 1+7 全过 | **PASS** | 见上表 #7–#11 |

**BUG-005 处置：CLOSED（P2，2026-09-12 回合 19）。** 原缺陷的四路证据（几何 JSON / 复现日志 / 截图 / 根因）保留在回合 18 节；本回合以解除 fixme 的同一断言 + 独立量测确认修复，且未发现修复引入的回退。

## 缺陷

### BUG-006 · 并发写时 `POST /items/{id}/photos` 返回 500 `database is locked`（deferred 事务读→写升级立即失败；同类 T14 已在 `submission.rs` 用 `BEGIN IMMEDIATE` 修过一次）

- 严重度／状态：**P2 / OPEN**（建议 P2：可重试、无数据损坏/费用/安全影响；但真实并发下**大概率失败**，且使合同承诺的「同视图第二张 → 422 `viewOccupied`」在高并发窗口退化为 500 通用内部错误）
- 对应 REQ / UI / AC：**REQ-013 / AC-021**（"同一快照每视图最多一张（第二张被拒或覆盖规则明确）"）与 **UI-012**（失败文案应为「该视图已有照片，请先移除或改选」）；**无 T16 直接 AC**（T16 未改后端，见下"是否阻断"）
- 环境与输入：真实 Rust 二进制（debug）+ 全新临时 data-dir + curl（公开 HTTP 合同）；端口 18091/18092/18093；Provider 未参与
- 复现步骤（三条，脚本与原始日志均在 `artifacts/web-mvp/t16-qa-r19/`）：
  1. **确定性（外部写者持锁）**：`bash repro-conc-photos-500.sh <log>` —— 起服务→登录→建物品→上传 photo 资产→python3 持 `BEGIN IMMEDIATE` 4s→期间 `POST /photos`。
  2. **自然并发（无外部锁）**：`bash repro-conc-photos-500-natural.sh <log> 40` —— 40 轮 ×（4 并发同视图 + 4 并发异视图 + 2 并发建物品）。
  3. **聚焦量化**：`bash repro-conc-photos-500-focused.sh <log> <server-log>` —— S1 顺序单发 ×20 / S2 同视图 4 并发 ×10 / S3 异视图 4 并发 ×10。
- 期望与实际：
  - 期望：并发写不让用户看到 500；锁竞争应按 `busy_timeout=5s` 等待，或在真冲突时按合同给 **422 `viewOccupied`**（模块文档即如此承诺："并发窗口由 `photos_item_view_unique` 兜底，同样映射为该 422"）。T14 已确立本仓库修法：`begin_with("BEGIN IMMEDIATE")`。
  - 实际（QA 实测）：
    - **A（确定性）**：对照 `POST /photos view=front` → **201**；持锁期间 `POST /photos view=left` → **500** `{"error":{"code":"INTERNAL",…},"requestId":"01a094cd-263b-7645-a9a4-44441598b604"}`，服务端同一 requestId 记 `存储错误…数据库错误：error returned from database: (code: 5) database is locked`，`durationMs=3.59`（**未等待 5s**）；锁释放后同一请求 → **201**。
    - **B（自然并发）**：400 个请求 → **500×237 / 201×161 / 422×2**；服务端 237 条 `database is locked`。
    - **C（聚焦）**：S1 顺序单发 **20/20 = 201（0 失败）**；S2 同视图 4 并发 40 个请求 → **201×10 / 422×2 / 500×28（70%）**（30 个"应被拒"的请求里只有 2 个拿到合同承诺的 422，其余 28 个是 500）；S3 异视图 4 并发 40 个请求 → **201×11 / 500×29（72.5%）**；服务端日志 57 条 locked = **54× code 5（SQLITE_BUSY）+ 3× code 517（SQLITE_BUSY_SNAPSHOT）**，**全部在 `POST …/photos`**（`grep` 路径分布）。
  - **根因（QA 代码级 + 机制实验双重确认）**：`crates/server/src/http/photos.rs:90`（POST；`:292` PATCH 同形）用 `connection.begin()`（deferred）先 `view_occupant`（SELECT，建立读快照）再 `photos::create`（INSERT，升级为写）。WAL 下 deferred 事务的读→写升级**不能等待**：纯 SQLite 对照实验（同库、`busy_timeout=5000`、另一连接持 `BEGIN IMMEDIATE`）——（a）`SELECT→INSERT` deferred：**0.001s 立即失败** `database is locked`；（b）`INSERT` 先写（无先读快照）：**等待 2.14s 后成功**；（c）锁释放后 `SELECT→INSERT`：成功。即 `busy_timeout` 对"先读后写"的 deferred 事务不生效——与 T14 已修的 `SubmissionWindow::begin_intent`（`crates/server/src/jobs/submission.rs:121`）**同类同修法**；同类的 `pool.begin_with("BEGIN IMMEDIATE")` 也见 `job_stages.rs:297`。执行器 `idle_poll=250ms` 且每次 `claim_next` 都 `BEGIN IMMEDIATE`（即使空闲也周期性取写锁），是自然并发下的主要竞争写者。
- 影响：T16 向导第 3 步（视图排列）上传照片是写 `POST /photos`；任何"同一时刻另有写者"（同一用户两个标签页/两台设备、前端并发请求、后台执行器 tick、批量脚本）都会让该请求以通用 500 失败。前端行为正确（QA 回合 18 已验：失败如实报错、可重试、不伪造成功），因此是**稳定性/合同语义**问题，不是数据损坏或费用问题。RD 全量 e2e 首跑命中 1 次（`database is locked`），QA 本回合两次全量均未命中（S1 量级 = 0/20）——属"低概率自然命中、高概率对抗命中"。
- 证据路径：`repro-A-writer-lock.log`、`repro-B-natural-concurrency.log`、`repro-C-focused.log`、`repro-C-server.log`（含 57 条 locked 与 requestId）、`sqlite-mechanism-control.log`、三条 `repro-conc-photos-500*.sh`（可重跑）
- 回归范围：`crates/server/src/http/photos.rs`（POST `create_photo` 与 PATCH `patch_photo` 两处事务）→ 建议改 `BEGIN IMMEDIATE`（或把占用检查并入 INSERT 的原子语句 / 捕获 `SQLITE_BUSY*` 转 409/422）；**同类风险面**：`crates/server/src` 下 22 处 `.begin()` 中凡"先读后写"者（如 `drafts/service.rs`、`preparations.rs`、`jobs/control.rs`、`generation/*`）需逐个甄别；执行器 tick 的写锁频率（250ms）可作压测输入。修好后建议按 BUG-003/004 惯例在 `crates/server/tests/` 加 `#[ignore]` 守卫（本回合三条脚本可直接改造），由 QA 复验。
- RD 修复摘要引用：`implementation.md` §BUG-006-1～10（统一 `storage::tx::begin_write*` = `BEGIN IMMEDIATE`；24 个事务点甄别 8 必修 + 14 防御 + 2 收敛；新增 `crates/server/tests/photos_concurrency.rs` 5 用例；证据 `artifacts/web-mvp/bug006-rd/**`）
- QA 复验结果与日期：**CLOSED（P2，2026-09-12，回合 20）**——三条脚本原样重跑 500×0（A 锁内 201/等待 3.39s、B 400 请求 0×500、C 0×500 且 0 条 locked）；QA 自写 337 请求独立压测 0×500 且合同状态码正确；`cargo xtask check` 7/7、workspace 476/0/3、全量 e2e 36/36 ×2。详见本文件「回合 20」节。

**其它缺陷：无。** 本回合无 T16 范围内未关闭缺陷。

## BUG-006 是否阻断 T16 切片（QA 判定与理由）

**不阻断 T16 切片 PASS。** 理由：
1. **不在本切片改动范围**：QA 以文件 mtime + 源码核对确认修复回合只改了 `apps/web/src/styles.css`、`apps/web/src/features/library/LibraryPage.tsx`（+ 新增 RD 守卫 spec）；`crates/**`、PRD、合同均未变，`contracts --check` 两份 `[一致]`。BUG-006 是 **T07 已交付后端**（AC-021）的既有缺陷，非 T16 引入。
2. **T16 的必选 AC 与卡内 UI 条目本回合全部通过**：AC-026 全部子句、AC-030（Playwright 侧）、AC-048「无自动发布路径」子句与 UI-005/006/009–026 见回合 18 + 本回合复验；两次全量 36/36 绿。
3. **用户可见行为不破**：失败时前端如实报错并给重试（不伪造成功、不产生半提交数据；事务失败即回滚）。
4. **不因此放过**：BUG-006 已按 P2 OPEN 立案，**应在 MVP 全量验收（T23）之前修复并由 QA 复验**；若在 T17–T22 期间再次命中（e2e 全量出现 `database is locked`），按本缺陷处理，不重复立案。

## 非阻断观察（供协调者/RD/T17+）

1. **BUG-005 的"时间列 1280px 折行"** 仍是 RD §T16-FIX-7 观察 2 的形态（114px 列宽 → 行高 129.1px，阈值内）；QA 本回合实测复现该数值，**非缺陷**；若要时间单行，属 PM/UI 的后续取舍。
2. **e2e 证据目录共享仍会互相覆盖**（T09 回合 9 的 P3-2 仍开放）：本回合 4 次 e2e 运行再次覆盖 `artifacts/web-mvp/t09-rd/{e2e-server.log,playwright-output/}`；QA 已把每次运行的服务端日志另存为 `e2e-server.qa10.log` / `.rd-guard.log` / `.full-r19.log` / `.full-r19-run2.log`。建议 T21/T22 前改为按运行隔离。
3. **RD 守卫 spec 会在每次运行时重写 `artifacts/web-mvp/t16-rd-fix/*.json` 与 3 张截图**（本回合 QA 运行时已覆盖 RD 原文件；QA 运行前的副本另存为 `r18-rd-fix-*.json`）。若需长期保留首版证据，建议同样按运行隔离。
4. **新增 llmdoc 是否够**：本回合新增验收知识已写入 `llmdoc/decisions.md`「T16 BUG-005 复验与并发 500 复现知识（回合 19）」；`validation-release.md` 命令合同无变化（未新增命令或阈值——BUG-006 的建议守卫用例属修复卡范围）。

## 未覆盖边界（本回合记录，不冒充通过）

1. **BUG-006 未修复**：本回合只复现与定级，未改生产代码；修复后需 QA 按上面三条脚本复验（含 `BEGIN IMMEDIATE` 后 busy_timeout 语义与 422 语义恢复）。
2. **AC-048「生成成功后提示需人工确认才能发布」子句**：与回合 18 相同不可达（Provider 指向 `127.0.0.1:1`，job 停在 queued/retry），草稿/发布 UI 属 T18/T19；**不得**据本报告宣称 AC-048 整体通过。
3. **真实 Provider／付费链路（T23）**；加密/超页 PDF 拒绝（T09 已验收，本回合未重跑）；100 页规模性能（T21）；任务中心轮询语义（T17）。
4. **真实磁盘满 statvfs 端到端**（回合 18 以服务端形状注入验证前端；服务端由 T06 `SpaceProbe` 覆盖）。
5. **BUG-006 在真实用户负载下的频率**：本回合用固定并发脚本量化（S2/S3），未做长时间随机负载画像；S1（顺序单发）20 次未命中说明自然命中率低但非零（RD 全量首跑命中 1 次）。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–17 | 2026-09-12 | slice | T01–T15 | 1/2 | PASS（回合 10、12 各 FAIL→修复后 PASS） | 本文件上半部分 |
| 18 | 2026-09-12 | slice | T16 | 2（ui_revision 2） | **FAIL**（BUG-005 P2 阻断行布局；其余全部通过） | 本文件 |
| 19 | 2026-09-12 | slice | T16 | 2（ui_revision 2） | **PASS**（BUG-005 → CLOSED；新增 BUG-006 P2 OPEN，不阻断本切片） | 本节 |

- 交接给 RD（**必须修复后由 QA 复验**）：**BUG-006（P2，OPEN）**——`POST/PATCH /items/{id}/photos` 的 deferred 事务读→写升级在活跃写者下**立即** `database is locked`（code 5/517）→ 500。建议：photos 两处事务改 `BEGIN IMMEDIATE`（与 T14 `submission.rs:121`、`job_stages.rs:297` 同修法），并顺带甄别 `crates/server/src` 其它"先读后写"的 `.begin()` 事务；修后在 `crates/server/tests/` 加 `#[ignore]` 守卫（可用本回合三条脚本改造），由 QA 复验。
- 交接给协调者：(a) `qa_result: PASS`（仅 T16 切片）、`qa_round: 19`、`qa_history` 追加回合 19（PASS / slice / [T16]）；(b) **BUG-005 → CLOSED**（`open_defects` 中该条置 closed，保留摘要）；(c) `open_defects` 追加 `{id: BUG-006, severity: P2, ac: "AC-021 / REQ-013 / UI-012", status: OPEN, owner: rd, blocking: false, note: "并发 POST /photos 500 database is locked；不阻断 T16；MVP 全量验收前必修"}`；(d) **T16 进入 `accepted_tasks`**：`{task_id: T16, prd_revision: 2, qa_round: 19, accepted_ac_ids: [AC-026, "AC-030（Playwright 侧）", "AC-048（UI 侧，无自动发布路径子句）", AC-016 前端消费侧/UI-005/006/009–026], status: accepted}`；(e) QA 常驻改动仅 QA-10 的 `test.fixme` 解除（回归守卫转绿），请勿回退。
- 交接给 PM：无需求歧义需裁定。若要"时间列单行"（观察 1）或调整 1280px 行高外观，属 UI 取舍，请走 PRD/UI 修订流程；QA 不在本轮自行接受偏离。
- 全项目状态提醒：T16 切片通过**只覆盖「资料库与新建向导」**（真实浏览器 + 真实后端 + 假 Provider，零真实外网/零付费）；T17–T23（任务中心/草稿与发布/阅读器与热点校准/导出备份/正式单包冷启动/真实授权链路）仍未实现/验收；**MVP 尚未完成**；BUG-006 是已明确的后端遗留项。
- QA 产出与证据：`apps/web/tests/e2e/qa-t16-independent.spec.ts`（QA-10 的 `test.fixme` 已解除，其余未动）；`artifacts/web-mvp/t16-qa-r19/`——`e2e-qa10-unfixed.log`、`qa10-r19-library-row-geometry.json`（单跑 QA-10）与 `qa10-r19-run2-library-row-geometry.json`（全量第 2 次）、`probe-r19-raw.log`、`probe2-r19-raw.log`、`e2e-rd-guard-r19.log`、`r19-rd-guard-geometry*.json`、`e2e-full-r19-run1.log`、`e2e-full-r19-run2.log`、`e2e-server.*.log`（4 份）、`web-checks-r19.log`、`xtask-check-r19.log`、`dist-r19.log`、`smoke-bootstrap-r19.log`、`qa-source-hashes.txt`、`repro-conc-photos-500*.sh`（3 条）、`repro-A/B/C*.log`、`repro-C-server.log`、`sqlite-mechanism-control.log`、`r18-rd-fix-*.json`（RD 原证据副本）；临时探针 spec 运行后已删除（原始 stdout 已留证）。

# 回合 20 · BUG-006 修复复验（并发写 `database is locked`）

**结果：PASS（BUG-006（P2）→ CLOSED；无新增产品缺陷）** · 回合：20 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[BUG-006]；复验 BUG-006：三条自有脚本原样重跑 + QA 自写独立并发压测 + 掩盖核对 + 事务点甄别抽查 + 全量回归 + QA-10 跨档瞬态判定）

- QA 执行时间：2026-09-12 17:26–17:45（本地 UTC+8）；执行者：QA 子 agent（回合 20）。
- **独立性声明**：`implementation.md` §BUG-006 与 `artifacts/web-mvp/bug006-rd/**`（RD 的 pre/post 日志、判别力实验、守卫用例）**只作线索与交叉核对**。本回合全部通过证据来自 QA 现场执行：QA 回合 19 的三条复现脚本**原样重跑**（未改一行；脚本 mtime 仍为回合 19 的 16:47–16:59，sha256 见 `r20-script-hashes.txt`）、QA **自写**独立压测脚本（337 请求，不复用 RD 用例）、QA **自写** Playwright 逐帧探针（rAF 级采样）、源码逐个抽查与全局复核计数、全量回归命令。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）＝ 派发包；`state.yaml`：phase=qa_running、qa_round=20、prd_revision=2、ui_revision=2、current_tasks=[BUG-006]、open_defects=[BUG-006(fixed_pending_reverify)]。
- 本回合不覆盖：T17–T23；AC-048「生成成功后」UI 子句；真实 Provider／付费链路（T23）；100 页规模；真实磁盘满 statvfs 端到端（与回合 19 相同）。
- **修复回合改动范围核对（mtime + sha256）**：`crates/server/**` 16 个文件（含新增 `storage/tx.rs`、`tests/photos_concurrency.rs`）；`apps/web/**` **零改动**——`styles.css`、`LibraryPage.tsx`、`library-row-layout.spec.ts`、`qa-t16-independent.spec.ts` 的 sha256 与回合 19 记录（`t16-qa-r19/qa-source-hashes.txt`）**逐字节一致**；`migrations/**`、`llmdoc/contracts.md`、`prd.md` 未动，`contracts --check` 两份 `[一致]`。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb`（与本分支起点一致，未提交）；本回合 QA 无生产代码改动；临时探针 spec 运行后已删除（副本 `artifacts/web-mvp/bug006-qa/qa-r20-layout-probe.spec.ts.txt`） |
| 平台／浏览器 | macOS Darwin 25.6.0 · aarch64-apple-darwin；node v26.0.0 · npm 11.12.1；Playwright 1.60.0（chromium headless shell）；Rust 1.98.1 |
| 后端 | 真实 Rust 二进制（debug，`cargo build -p everything-manual` 已是最新，sha256 `02d53620…`）——三条复现脚本用端口 18091/18092/18093，QA 自写压测用 18095，均全新临时 data-dir + `providers.*.base_url = http://127.0.0.1:1`（连接必被拒）；线上 e2e 走 globalSetup 起的真实二进制（18080）+ Vite（15173） |
| 数据／fixture | 公开 HTTP 合同造数；真实 multipart 上传 `sample-photo-front.jpg` / `sample-photo-left.png` / `sample-manual-text.pdf`；**0 次真实外网／0 次付费调用** |
| 未验证 | 真实 Provider 与结算（T23）；AC-048「生成成功后」UI；锁被持超过 `busy_timeout=5s` 的真实长事务（本回合最长人为持锁 4s，见观察 4） |

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/bug006-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `bash artifacts/web-mvp/t16-qa-r19/repro-conc-photos-500.sh …`（脚本 A 原样重跑） | 锁内 `POST /photos view=left` → **201**，`durationMs=3394.67`（等待写锁释放，不再 3ms 级 500）；锁释放后同视图重复 → **422 `viewOccupied`**；对照 POST front → 201（1.17ms）；服务端 0 条 locked（`r20-repro-A.log`） |
| 2 | `bash …/repro-conc-photos-500-natural.sh <log> 40`（脚本 B 原样重跑） | 400 请求 → **201×280 / 422×120 / 500×0**（理论分布逐项吻合）；服务端 `database is locked` **0 条**（`r20-repro-B.log`） |
| 3 | `bash …/repro-conc-photos-500-focused.sh <log> <serverlog>`（脚本 C 原样重跑） | S1 **201×20**；S2 **201×10 / 422×30 / 500×0**（每轮恰好 1×201+3×422）；S3 **201×40 / 500×0**；服务端 `code 5 = 0`、`code 517 = 0`、`status":500 = 0`（`r20-repro-C.log`、`r20-repro-C-server.log`） |
| 4 | `bash artifacts/web-mvp/bug006-qa/qa-r20-independent-stress.sh … `（**QA 自写**独立压测，6 阶段 337 请求） | exit 0；**总判定 PASS**：0×500、0×database is locked、全部响应 ∈ {200,201,422}（`r20-independent-stress.log`、`r20-stress-server.log`） |
| 5 | `cargo test -p everything-manual --test photos_concurrency`（RD 守卫 ×3 连跑） | 3 次均 **5 passed / 0 failed**（约 1.4s）（`r20-photos-concurrency-x3.log`） |
| 6 | `cargo test --workspace` | exit 0；**476 passed / 0 failed / 3 ignored**（31 个目标求和；3 ignored ＝ 既有 BUG-003/004 QA `#[ignore]` 守卫）（`r20-cargo-test-workspace-full.log`） |
| 7 | `cargo xtask check` | exit 0；**7/7 `[通过]`**（fmt / clippy `-D warnings` / workspace 测试 / npm lint / typecheck / vitest 62 / contracts --check）（`r20-xtask-check.log`） |
| 8 | `cargo xtask contracts --check`（单独再跑） | exit 0；`[一致] contracts/openapi.json` + `[一致] apps/web/src/api/generated.ts`（`r20-contracts-check.log`） |
| 9 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0；sha256 **`dc05f695bb3f1ac3d27c53b2a41f68a16695cf5c43c4c5a89fe4d3039ac420da`**（22 570 272 B）——**与 RD §BUG-006-7 声称一致**（QA 独立复现）；`r20-dist.log`、`r20-dist-sha256.txt` |
| 10 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` | exit 0；1 项 `[准备]` + 7 项 `[检查]` 全过（内嵌 `index-C4vQ2t8_.js`、health live+ready、JSON 404、SPA 深链接、缺失资源 404；仅含二进制的临时目录内启动）（`r20-smoke-bootstrap.log`） |
| 11 | `npm --prefix apps/web run test:e2e`（全量，第 1 次） | exit 0；**36 passed / 0 failed / 0 skipped**（1.5m）；后端日志 **0 条 `database is locked`、0 条 `status:500`**（`r20-e2e-full-run1.log`、`r20-e2e-server.full-run1.log`） |
| 12 | `npm --prefix apps/web run test:e2e`（全量，第 2 次，抖动复核） | exit 0；**36 passed / 0 failed / 0 skipped**；后端日志 **0 条 locked、0 条 500**（`r20-e2e-full-run2.log`、`r20-e2e-server.full-run2.log`） |
| 13 | `npm --prefix apps/web run test:e2e -- qa-r20-layout-probe.spec.ts`（**QA 自写**逐帧探针，运行后删除） | exit 0；**1 passed**；5 个 1024→1280 跨档周期逐帧（约 74 帧/周期）+ 边界 1276/1278/1280/1282 采样（`r20-layout-probe-run.log`、`r20-layout-probe.json`） |

## BUG-006 复验矩阵（修复前 → 修复后；同一批脚本、同一口径）

| 场景 | 修复前（回合 19 实测／RD pre-fix） | **修复后（QA 回合 20 实测）** | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| A 对照（无并发写者） | 201（1.096ms） | 201（**1.170ms**，快速路径无劣化） | PASS | `r20-repro-A.log` |
| A 锁内 `POST /photos`（外部写者持锁 4s） | **500** `INTERNAL`，`durationMs=3.590`（未等待），服务端 code 5 | **201**，`durationMs=3394.67`（按 `busy_timeout` 等到释放；理论等待≈3.3s，偏差≈95ms 为释放→唤醒开销） | PASS | 同上 |
| A 锁释放后同视图再发 | 201（新建成功） | **422 `viewOccupied`**（0.627ms）——合同语义恢复 | PASS | 同上 |
| B 自然并发 400 请求 | 500×237 / 201×161 / 422×2；server 237 条 locked | **201×280 / 422×120 / 500×0**；server **0 条 locked** | PASS | `r20-repro-B.log` |
| C S1 顺序单发 ×20 | 201×20 | 201×20（不变） | PASS | `r20-repro-C.log` |
| C S2 同视图 4 并发 ×10 轮 | 201×10 / 422×2 / **500×28（70%）** | **201×10 / 422×30 / 500×0**（每轮恰好 1×201+3×422） | PASS | 同上 |
| C S3 异视图 4 并发 ×10 轮 | 201×10 / **500×30（72.5%）** | **201×40 / 500×0** | PASS | 同上 |
| C 服务端日志 | 57× code 5 + 3× code 517、`status:500` 若干 | **code5=0 / code517=0 / status:500=0** | PASS | `r20-repro-C-server.log` |
| 修复前"SQLite 侧验证：BEGIN IMMEDIATE 被拒" | 锁内仍有写者 → 被拒 | 变为"意外：拿到了写锁"——**脚本时序副产物**（POST 现在等到锁释放才返回，检查时外部写者已退出），非缺陷；见观察 5 | 说明 | `r20-repro-A.log` |

## 独立并发压测（QA 自写 `qa-r20-independent-stress.sh`；真实二进制 + 真实 HTTP + 执行器 250ms 常驻写者）

| 阶段 | 场景 | 并发设计 | 结果 |
| --- | --- | --- | --- |
| P1 | 同物品多视图并发 | 15 轮 × 4 视图同发（front/left/back/right） | **201×60，异常×0** |
| P2 | 同视图并发（超连接池） | 10 轮 × 6 请求同发同一 view（连接池上限 4） | **201×10 + 422×50**；50 个 422 的 `details.reason` **全部 = `viewOccupied`**；每轮落库核对该视图**恰好 1 张**（10/10） |
| P3 | 跨物品并发写 | 8 物品 × 4 视图 = 32 请求齐发 | **201×32，异常×0** |
| P4 | 与常驻外部写者同时写 | python 写者循环 `BEGIN IMMEDIATE`+INSERT（485 次持锁，~4ms/次；50ms 探针 500 次抢锁 5 次被拒＝竞争成立）+ 10 轮同物品 4 视图 + 10 轮同视图 4 并发 | **201×50 / 422×30，异常×0**（每轮同视图 1×201+3×422 全部正确） |
| P5 | prepare/complete 与照片写并发 | 5 轮：（PUT 第 1/2 页 ∥ 4 视图照片写）；3 轮含（complete ∥ detail 视图 2 并发写） | **200×15（页写 10 + 封存 5）/ 201×25 / 422×5，异常×0** |
| P6 | 60 并发突发 | 3 物品 × 5 视图 × 4 份混合同视图冲突 | **201×15 / 422×45，异常×0**（每物品每视图恰好 1 张） |
| P7 | 服务端日志 | 全阶段汇总 | `database is locked=0`、`code5=0`、`code517=0`、`status:500=0` |

合计 **337 个请求**，全部响应 ∈ {200, 201, 422}，0 个 500，0 条锁错误；422 全部携带合同承诺的 `viewOccupied`。

## 掩盖核对（未用 busy_timeout／全局锁／串行化／重试层）

| 项 | 核对方法 | 结果 |
| --- | --- | --- |
| `busy_timeout` 未被提高 | 全仓 `busy_timeout` 设置点 | 仅 `storage/db.rs:170 .busy_timeout(BUSY_TIMEOUT)`，常量 `BUSY_TIMEOUT = 5s`；`ConnectionSettings::matches_expected` 仍断言 5000ms |
| 未加全局写锁／串行化 | `grep Mutex/RwLock/Semaphore crates/server/src` | 仅日志、登录限速、failpoints 测试注册表、blob_store 脚本化——**数据库写路径无新增同步原语**；`Connection`/`Transaction` 用法未变 |
| 未加吞错重试层 | `grep SQLITE_BUSY/"is locked"/retry` 于 `storage/**`、`http/**`、`tx.rs`、`photos.rs` | 无任何 BUSY 捕获/重试/退避代码；`photos.rs`、`tx.rs` 无 `retry` 字样；既有 `claim_next` 4 次尝试循环为 T10 既有逻辑（未改） |
| 事务开启点单一来源 | `grep '"BEGIN IMMEDIATE"'` | 全仓仅 `storage/tx.rs:25` 一处字符串常量 |
| 连接池未被降到 1 | `POOL_MAX_CONNECTIONS` | `= 4`（`storage/db.rs:25`），未变 |
| 并发上限未变 | `config/mod.rs` | `MAX_REMOTE_GENERATION_CONCURRENCY = 2`、`MAX_MANUAL_AI_BATCH_CONCURRENCY = 2`，未变 |
| 执行器写锁频率未被调低 | `jobs/executor.rs` | `idle_poll = 250ms` 未变；`claim_next` 仍每次 tick `BEGIN IMMEDIATE`（空闲也取写锁） |
| 延迟未异常膨胀 | A 场景等待时间 vs 外部持锁时长；对照路径延迟 | 锁内请求等待 3394.67ms ≈ 理论剩余锁期 3.3s（+95ms），**未触 5s 超时、无额外惩罚**；对照 POST 修复前 1.096ms → 修复后 1.170ms（**快速路径无回归**）；锁释放后 422 仅 0.627ms |

## 甄别清单抽查（QA 全局复核 + 逐点源码核对）

**全局复核计数**：`grep -rn "\.begin()\|begin_with\|begin_write" crates/server/src` → 生产代码**已无任何 deferred `.begin()` 调用**（唯一命中是 `tx.rs` 的模块文档注释）；`begin_write(`/`begin_write_pool(` 运行时调用点 **23 处** + `storage/repo/items.rs:20` 文档示例 1 处 = **24 处**，与 RD 清单**逐行对应**；`"BEGIN IMMEDIATE"` 字符串仅 `tx.rs:25` 一处。**未发现遗漏的 deferred 读→写路径**（全部写事务现在都在开启时取写锁）。

**逐点抽查（≥5，实查 14 处）**：

| # | 位置 | 事务内首条语句 | RD 分类 | QA 核对 |
| --- | --- | --- | --- | --- |
| 1 | `http/photos.rs:92` `create_photo` | `view_occupant`（SELECT）→ `photos::create`（INSERT） | A 必修 | ✅ `begin_write` |
| 2 | `http/photos.rs:296` `patch_photo` | `view_occupant`（SELECT）→ `photos::update`（UPDATE） | A 必修 | ✅ `begin_write`（并发兜底走唯一索引→422 分支保留） |
| 3 | `jobs/control.rs:244` `cancel_job` | `jobs::cancel` 内部先读 job 状态 | A | ✅ `begin_write` + 保留原委注释 |
| 4 | `jobs/control.rs:776` `attach_remote_task` | `record_remote_task_id` 先读现状 | A | ✅ `begin_write` |
| 5 | `drafts/service.rs:330` `patch_draft_status` | `read_draft`（SELECT，归属+revision） | A | ✅ `begin_write` |
| 6 | `storage/repo/preparations.rs:252` `write_page` | `load_in_tx`（SELECT）+ 页/资产读取 | A | ✅ `begin_write` |
| 7 | `storage/repo/preparations.rs:405` `complete` | `load_in_tx` + 页集合校验（读） | A | ✅ `begin_write` |
| 8 | `providers/tripo/handlers.rs:1441` `record_revision` | `get_or_create` 先 `find_by_sha`（SELECT） | A | ✅ `begin_write` |
| 9 | `jobs/executor.rs:293` `recover_one` | `take_over_expired`（条件 UPDATE） | B 写先 | ✅ `begin_write_pool` |
| 10 | `jobs/executor.rs:623` `advance` | `job_stages::advance`（UPDATE） | B | ✅ `begin_write_pool` |
| 11 | `assets/upload.rs:135` `commit_metadata` | `blobs::insert_if_absent`（INSERT..ON CONFLICT） | B | ✅ `begin_write` |
| 12 | `generation/estimate.rs:268` `confirm_quote` | `quotes::mark_confirmed`（条件 UPDATE） | B | ✅ `begin_write` |
| 13 | `drafts/service.rs:245` `assemble_draft` | `upsert_assembled`（INSERT..ON CONFLICT） | B | ✅ `begin_write` |
| 14 | `storage/repo/job_stages.rs:297` `claim_next` | 容量谓词 SELECT（原即 IMMEDIATE） | C 收敛 | ✅ `begin_write_pool`（注释指向 `storage::tx`） |

结论：RD 的"8 必修 + 14 防御 + 2 收敛 = 24"甄别**与源码一致**；"写先"分类抽查（#9–13）均属实；无事务点被漏改。

## QA-10 跨档瞬态判定（产品缺陷 or 测试自身瞬态）

**判定：测试自身的跨档瞬态（QA-10 测量法缺陷），不是产品布局缺陷；不按 BUG 立案。** 证据四路：

1. **逐帧探针（QA 自写）**：5 个 1024→1280 周期各采样 ~74 帧（rAF，页内原子读取）：切换后**第一帧即已是稳定宽屏态**（row 840 / identity 228 / class `page-layout--wide`），此后 1.2s 内**逐帧恒定**；`identity < 120 或 null 的帧数 = 0`；每周期在 1280 只出现 **1 种状态**（无回摆、无振荡）。边界往返采样（1276→1278→1280→1282→1280→1278→1276）：1276/1278 = `--mid` 态（row 1076 / identity **346**，仍 ≥120 可读），1280/1282 = `--wide` 态（228/229），每个宽度落定后稳定，**无临界抖动**。
2. **RD 首跑失败样本的构成**：其"1280"条目的 row=1076、identity=346、`grid="346px 173px 173px 324px"`（＝ mid 态）与 status.x=276/time.x=402/actions.x=528/note=816（＝ wide 态）**互相矛盾**——单次布局不可能产生该组合，说明 7 次独立 `boundingBox()`/`evaluate()` 往返**横跨了 React 断点重挂载**；`name: null` 即重挂载窗口内 `.item-row__name` 无布局盒。**两侧状态本身都满足断言**（mid@1280 identity=346、wide@1280=228），失败只来自跨档混合快照。
3. **本回合全量 e2e 也复现了同一竞态的非致命形态**：第 2 次全量 1280 档的 `styles` 读出**空字符串**（`getComputedStyle` 于已脱离渲染的行元素＝同一重挂载窗口），而几何断言用的 boundingBox 已是正确 wide 态（840/228）→ 用例仍通过。证明竞态真实存在、且与产品布局无关。
4. **对照实现**：RD 守卫 `library-row-layout.spec.ts` 用**单次 `evaluate` 原子测量 + `waitForTimeout(100)` 等断点重挂载**，RD 两次全量 + QA 两次全量均 2/2 通过；QA 回合 19 的知识条目（decisions.md）已写明该正确形态，但 **QA-10（回合 18 编写）仍用 7 次分散测量且不等待落定**，属旧写法遗留。

**建议**：加固 QA-10（不改断言、不放宽阈值）：① 改为单次 `evaluate` 原子取整行几何（同 RD 守卫/本回合探针）；② `setViewportSize` 后等待断点落定（如等 `.page-layout` 类名或 `expect.poll` 网格字号）。**本回合按"不得修改既有测试文件"约束未改**，交协调者决定派卡或授权 QA 下回合修正（属 QA 侧测试质量项，P3，不阻断任何切片）。在加固前，QA-10 存在低概率假 FAIL（RD 记录 1/2 次首跑命中；QA 回合 19 两次全量 + 本回合两次全量 + 探针 5 周期未命中）——**若后续回合 QA-10 失败，先按本条识别跨档瞬态，不得据此判产品回退**。

## 缺陷

### BUG-006 · 并发写时 `POST /items/{id}/photos` 返回 500 `database is locked`

- 严重度／状态：P2 / **CLOSED（2026-09-12，回合 20）**（原立案与根因、修复前证据见本文件「回合 19」节，此处不覆盖）
- QA 复验结论：**CLOSED**。判据（回合 19 预先约定）：三条脚本复跑 0×500、合同状态码正确、锁竞争按 `busy_timeout` 等待。实测：A 锁内 201/等待 3.39s、释放后 422 `viewOccupied`；B 400 请求 500×0、服务端 0 条 locked；C 全 0；QA 独立压测 337 请求 0×500、422 全为 `viewOccupied`；全量 e2e 服务端 0 条 locked（2 次）。
- 证据路径：`artifacts/web-mvp/bug006-qa/`——`r20-repro-A.log`、`r20-repro-B.log`、`r20-repro-C.log`、`r20-repro-C-server.log`、`r20-independent-stress.log`、`r20-stress-server.log`、`r20-stress-server-statuses.txt`、`r20-stress-server-writer.log`。
- RD 修复摘要引用：`implementation.md` §BUG-006（`storage::tx::begin_write*` 统一 `BEGIN IMMEDIATE`）；ADR-027 状态 verified；QA 复核：修复方案与 ADR 一致，**未使用**加大 busy_timeout／全局互斥／普通重试层等掩盖手段。

**其它缺陷：无新增产品缺陷。** 本回合未发现新的产品级问题。

## 非阻断观察（供协调者/RD/后续回合）

1. **QA-10 测量法（测试侧，P3）**：见上节；加固建议已给出，未修改文件。**不阻断任何切片**，但会偶发假 FAIL。
2. **复现脚本 A 的服务端清理遗漏（QA 脚本，P3）**：`repro-conc-photos-500.sh` 的 `cleanup` 在父 shell 的 EXIT trap 中读 `${SERVER_PID:-}`，而 `SERVER_PID` 是在 `{ … } | tee` 子 shell 内设置的 → 脚本结束后**服务端进程残留**（`kill -0` 检查不到）。本回合发现回合 19 与本回合各留下一个 18091 端口进程（数据目录已删、仅监听），**已清理**（`kill 294`，端口已释放）。修复建议：与脚本 B/C 一样在 cleanup 内加 `pkill -f "listen 127.0.0.1:18091"` 兜底（不修改历史脚本，保留原证据）。这也是 decisions.md 回合 19 记录的同类 bash 陷阱的直接后果。
3. **证据目录共享仍会互相覆盖**（T09 回合 9 的 P3-2 仍开放）：本回合 4 次 e2e/探针运行再次覆盖 `artifacts/web-mvp/t09-rd/{e2e-server.log,playwright-output/}` 与 `artifacts/web-mvp/t16-rd-fix/*geometry*.json`；QA 已先备份（`bug006-qa/t09-rd-before-r20/`、`bug006-qa/t16-rd-fix-before-r20/`）并把本轮服务端日志另存 `r20-e2e-server.full-run1/2.log`、几何另存 `r20-e2e-run2-library-row-geometry.json`。建议 T21/T22 前改为按运行隔离。
4. **真锁超时仍为 500（RD 已记录，非缺陷）**：若写事务持锁 > `busy_timeout=5s`，`BEGIN IMMEDIATE` 失败走既有「服务器内部错误：数据库暂不可用」（500）。合同未定义"数据库忙"专用码，本卡未新增语义；本回合最长人为持锁 4s 未触边界，自然负载未观察到。
5. **A 脚本"SQLite 侧验证：意外拿到了写锁"行**：修复后 POST 会等到锁释放才返回，随后该检查自然能拿到锁——是脚本时序的副产物，**不是缺陷**，但阅读该日志时勿误判。
6. **`t16-qa/library-row-geometry.json` 的 `styles` 字段在跨档时可能为空字符串**（本回合 run2 命中）：与观察 1 同源；该字段未被断言，不影响结论；若未来要用它做证据，先按观察 1 的加固。

## 未覆盖边界（本回合记录，不冒充通过）

1. T17–T23（任务中心/草稿与发布/阅读器与热点校准/导出备份/正式单包冷启动/真实授权链路）仍未实现/验收；**MVP 尚未完成**。
2. AC-048「生成成功后提示需人工确认才能发布」子句：与回合 18/19 相同不可达（Provider 指向 `127.0.0.1:1`）；**不得**据本报告宣称 AC-048 整体通过。
3. 真实 Provider／付费链路（T23）；加密/超页 PDF 拒绝（T09 已验收，本回合未重跑）；100 页规模性能（T21）；任务中心轮询语义（T17）。
4. 真实磁盘满 statvfs 端到端；`>5s` 长事务超时的用户可见语义（观察 4）。
5. 长时间随机负载画像（本回合为 337 请求的定项压测 + 人工常驻写者，非长时间随机负载）。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–17 | 2026-09-12 | slice | T01–T15 | 1/2 | PASS（回合 10、12 各 FAIL→修复后 PASS） | 本文件上半部分 |
| 18 | 2026-09-12 | slice | T16 | 2 | **FAIL**（BUG-005） | 本文件 |
| 19 | 2026-09-12 | slice | T16 | 2 | **PASS**（BUG-005 → CLOSED；BUG-006 P2 立案） | 本文件 |
| 20 | 2026-09-12 | slice | BUG-006 | 2（ui_revision 2） | **PASS**（BUG-006 → CLOSED；无新增产品缺陷） | 本节 |

- 交接给 RD：**无必修项**。BUG-006 已 CLOSED。可选改进（非缺陷）：观察 2 的脚本 A cleanup 兜底属 QA 侧；RD 侧无待办。
- 交接给协调者：(a) `qa_result: PASS`、`qa_round: 20`、`qa_history` 追加 `{round: 20, scope: slice, task_ids: [BUG-006], prd_revision: 2, result: PASS}`；(b) **BUG-006 → CLOSED**（保留原摘要与回合 19 证据链）；(c) T16 的 `accepted_tasks` 条目**不变**（回合 19 已 accepted；本回合仅后端缺陷复验，未触及 T16 AC/UI）；(d) 观察 1 的 QA-10 加固建议请派卡或授权（P3，测试侧）；(e) 本回合未改任何生产代码、未改既有测试文件，唯一常驻改动是新增探针 spec 的**运行后删除**（副本在 artifacts）。
- 交接给 PM：无需求歧义需裁定。时间列 1280px 折行（回合 19 观察 1）仍为 UI 取舍项，未变。
- 全项目状态提醒：BUG-006 关闭后，回合 19 遗留的"MVP 全量验收前必修"后端项已清零；T17–T23 仍未实现/验收，**MVP 尚未完成**。
- QA 产出与证据：`artifacts/web-mvp/bug006-qa/`——`r20-repro-A/B/C*.log`、`r20-script-hashes.txt`（三条脚本原样证据）、`qa-r20-independent-stress.sh` + `r20-independent-stress.log` + `r20-stress-server*.{log,txt}`、`r20-photos-concurrency-x3.log`、`r20-cargo-test-workspace-full.log`、`r20-xtask-check.log`、`r20-contracts-check.log`、`r20-dist.log` + `r20-dist-sha256.txt`、`r20-smoke-bootstrap.log`、`r20-e2e-full-run{1,2}.log` + `r20-e2e-server.full-run{1,2}.log` + `r20-e2e-run2-library-row-geometry.json`、`r20-layout-probe-run.log` + `r20-layout-probe.json` + `qa-r20-layout-probe.spec.ts.txt`（探针副本）、`t09-rd-before-r20/`、`t16-rd-fix-before-r20/`（共享目录运行前备份）；探针 spec 已从 `apps/web/tests/e2e/` 删除。

---

# 回合 21 · T17（任务中心与可行动错误）

**结果：PASS（仅 T17 切片；0 个新增产品缺陷；4 条非阻断观察交协调者／PM）** · 回合：21 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T17]；AC-049 全部子句、AC-033/AC-034 的 **UI 侧**、§6.2 **UI-027–UI-039**、**T15 P3① 复验**、QA 自身过时断言更新、回归链 + 单二进制 smoke）

- QA 执行时间：2026-09-12 18:30–19:05（UTC+8；下述日志时间戳为 UTC，即 10:30–11:05Z）；执行者：QA 子 agent（回合 21）。
- **独立性声明**：`implementation.md` §T17 与 `artifacts/web-mvp/t17-rd/**` 只作线索，**未用作任何通过证据**。本回合结论全部来自 QA 现场执行：
  - **QA 自写 Playwright 独立验收** `apps/web/tests/e2e/qa-t17-independent.spec.ts`（QA 新增，8 用例；观察方式与 RD 的 `job-recovery.spec.ts` 故意不同：金额逐字段与 API 对账、轮询按请求时间戳测间隔、只掐 GET 制造陈旧 ETag 等）；
  - RD 的 `job-recovery.spec.ts` 由 QA **自己重跑**（未采信 RD 日志）：8 passed；
  - 断网、关浏览器、服务重启、412、对账、重试准入**全部由 QA 亲自驱动并观测**（借 RD 的 `job-recovery-harness.ts` 设施，注意其两条硬约束：后端须声明 `public_origin`，且必须是 `--features job-failpoints` 的**测试构建**才能放行回环模型下载）；
  - 零外网为 QA 自采样（`qa-r21-lsof-loopback-only.sh`，含灵敏度说明）；unknown 实际费用直接读 SQLite `cost_ledger` 留证。
- PRD 修订核对：`prd.md` §9.1 修订 2（ui_revision 2）＝ 派发包；REQ-031/REQ-023/REQ-026 与 AC-049、AC-033、AC-034 仍为 [必选]。`state.yaml`：phase=qa_running、prd_revision=2、current_tasks=[T17]、open_defects=[]、qa_round=21。
- 本回合不覆盖：T18–T23（阅读器/热点校准与发布/导出备份/正式多平台包/真实授权链路）；**MVP 尚未完成**。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb`（全项目仍未提交，`git status` 与开工快照一致：既有 ` M .gitignore/contracts.md/decisions.md/state.yaml` 非本轮产生）；QA 新增 `apps/web/tests/e2e/qa-t17-independent.spec.ts`、`artifacts/web-mvp/t17-qa/**`，并**只做事实更新**修改自身旧用例 `qa-t16-independent.spec.ts`（见下节） |
| 平台／工具链 | macOS Darwin 25.6.0 · aarch64-apple-darwin；cargo/rustc 1.98.1；node v26.0.0；Playwright Chromium（Desktop Chrome） |
| 交付产物（QA 独立重建） | `dist/aarch64-apple-darwin/everything-manual`，sha256 **`f08444b9e4af82ef26c0de4435eb434d266cbe79b1760deed0d0640f4229c924`**（22 627 728 B）+ SHA256SUMS/licenses.json/build-info.json；`smoke-bootstrap` 全过。**注**：RD §T17-8 #12 记录的 `3262691d…`（18:13Z 构建）**早于** 18:22–18:23 的 5 个前端源文件修改（`jobs.ts`、`JobsListPage.tsx`、`LibraryPage.tsx`、`ItemOverviewPage.tsx`、`ConfirmStepPage.tsx`），与最终工作树不对应；QA 用最终工作树重建并验证内嵌了最终文案（`阅读器（T18/T19）尚未交付`、`本物品的任务`、`查看任务详情` 各命中 1 次）——见非阻断观察 3 |
| 数据／fixture | 每用例独立临时 data-dir（运行后无残留）；provider 指向本机 fixture（随机端口）或 `127.0.0.1:1`；凭据为假环境变量（`EM_T17_*` / `EM_E2E_*`）；**0 次真实外网/付费调用** |
| 零外网观测（QA 自己采样） | 最终全量 e2e 运行期间：507 次采样、9930 行 `lsof -nP -iTCP`，**ESTABLISHED 非回环 = 0**（回环 ESTABLISHED 5927 行，含后端↔fixture、浏览器↔Vite——证明采样有灵敏度）；QA spec 单独运行：590 次采样、9674 行、非回环 ESTABLISHED = 0（`r21-e2e-full-final-lsof-loopback-only.log`、`r21-qa-spec-lsof-loopback-only.log`、脚本 `qa-r21-lsof-loopback-only.sh`） |
| 未验证 | 真实供应商账户下的对账三动作与真实费用（T23）；`retry_wait` 的浏览器呈现（无 fixture 通路，见未覆盖边界）；30 分钟总等待转 `needs_input` 的端到端 UI 呈现；100 页/多批规模下轮询开销；顶栏任务计数徽标（未实现，观察 2） |

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/t17-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `npm --prefix apps/web run test:e2e -- job-recovery.spec.ts`（RD 套件，QA 重跑） | exit 0，**8 passed / 0 failed**（1.1m）（`r21-rd-spec-job-recovery.log`） |
| 2 | `npm --prefix apps/web run test:e2e -- qa-t17-independent.spec.ts`（**QA 新增 8 用例**） | exit 0，**8 passed / 0 failed**（`r21-qa-spec-run.log` + `r21-qa-spec-lsof-loopback-only.log`；迭代过程 `r21-qa-spec-independent.log`、`-run2.log`、`r21-qa-spec-test5-iter{1,2}.log`，3 处失败均为**QA 用例自身缺陷**，见"非代码知识"） |
| 3 | `npm --prefix apps/web run test:e2e`（**全量**：52 = import-flow 6 + job-recovery 8 + library-row-layout 2 + pdf-preparation 10 + qa-t09 6 + qa-t16 12 + qa-t17 8） | exit 0，**52 passed / 0 failed / 0 skipped**（4.0m）（`r21-e2e-full-final-run.log`；首轮同结果 `r21-e2e-full-suite.log`）。qa-t16 的 12 条含本回合更新的断言 |
| 4 | `npm --prefix apps/web run typecheck` / `lint` / `test -- --run` / `build` | 全部 exit 0；vitest **79 passed / 0 failed**（11 文件，含 T17 的 17 条单测）（`r21-typecheck.log`、`r21-lint.log`、`r21-vitest.log`、`r21-web-build.log`） |
| 5 | `cargo test --workspace` | exit 0，**477 passed / 0 failed / 3 ignored**（3 ignored = BUG-003/BUG-004 历史复现用例 + `items.rs` doctest，与回合 17/19/20 口径一致，非本轮新增）（`r21-cargo-test-workspace.log`） |
| 6 | `cargo xtask check` | exit 0，**7/7 [通过]**：fmt / clippy `-D warnings` / workspace 测试 / 前端 lint / typecheck / vitest / `contracts --check`（`r21-xtask-check.log`） |
| 7 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`，未修改工作树（`r21-contracts-check.log`） |
| 8 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0，sha256 **`f08444b9…`**（22 627 728 B）（`r21-dist.log`、`r21-dist-sha256.txt`） |
| 9 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` | exit 0，7 项检查全过（内嵌页面/静态资源/`health/live`/`health/ready`/未知 API JSON 404/SPA 路由刷新/缺资源 404）；日志显示 Provider 未配置时**不回退 mock**（`r21-smoke-bootstrap.log`） |
| 10 | `bash qa-r21-lsof-loopback-only.sh … npm --prefix apps/web run test:e2e` | exit 0；**非回环 ESTABLISHED = 0**（见上表） |

## AC 验收矩阵（逐子句）

| AC / UI | 期望（可观察） | 实际（QA 实测） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| AC-049 · 状态区分 | 本地准备/排队/供应商进度/等待人工/失败/unknown 各有区分 | 列表行/详情状态标签取自服务端状态：`本地阶段进行中`（manual 延迟窗口内实测）、`等待人工补齐`、`付费提交结果未知（等待对账）`、`失败`、`已完成（可复核草稿已产出，尚未发布）`、`已取消`；筛选按分类（已结束/等待人工/等待对账/失败）同屏生效；UI-030 的九状态映射与分类在 `status.ts`（颜色仅辅助、文本为准）。**透明度说明**：`排队中` 与 `等待供应商` 两个任务级标签本回合未构造到可观察窗口（瞬态），只做了映射读码核对（阶段级 `等待供应商` 在 T12 端点侧已验收） | PASS（含 2 个瞬态标签的读码核对，非浏览器实测） | QA spec 用例 1/2/4/6/8（截图 `01/03/03b/07/10`） |
| AC-049 · 可见页约 2 秒 | 可见页轮询≈2s | 详情页连续 8s 采样请求时间戳：≥3 次、**最大间隔 ≤3.5s**（实测间隔约 2s） | PASS | 用例 6（`r21-qa-spec-run.log`） |
| AC-049 · 后台 15 秒 | 页面不可见降为≈15s | 转后台后 **前 10 秒 0 次**（2 秒轮询会给约 5 次），20 秒内 1–3 次、首个间隔 ≤20.5s；恢复可见后继续轮询 | PASS | 用例 6（同上；headless 下 `bringToFront()` 不产生 hidden，见"非代码知识"1） |
| AC-049 · 终态停止 | 终态后停止轮询 | 详情（终态）：6 秒内 0 次；**列表（全部已加载行终态）：6 秒内 0 次**；非终态行存在时恢复轮询 | PASS | 用例 1/6 |
| AC-049 · 断网不误报业务失败 | 网络错误 ≠ 业务失败 | 掐断 `/api/v1/**` 后：详情与列表都出现 `role="status"` 的「网络连接异常，正在自动重试（本地状态未变…）」，列表另标「数据可能已过期」；状态标签保持服务端最后事实、**不含「失败」**；恢复后自动回到服务端实际状态 | PASS | 用例 3/6（截图 `05`、`12`） |
| AC-049 · 刷新/重启从数据库恢复 | 状态来自 DB | 断网期间在服务端取消任务 → 恢复后界面显示 `已取消`（不是缓存的旧状态、不是猜测）；`backend.restart`（同 data-dir）后 `page.reload()` 仍显示 `等待人工补齐` 且 `revision` 不回退；关浏览器期间服务端继续推进（`running → needs_input`）并由重开的页面读到新状态 | PASS | 用例 2/3/6（截图 `04`、`06`） |
| AC-049 · unknown 显示对账而非重试 | 无重试按钮、有对账入口 | `stage-status=结果未知（等待对账）`；`stage-retry-button` **计数 0**（不是禁用而是不渲染）；对账面板可见且含三动作；`recordNoTask` 空证据时提交按钮禁用、填写后可提交（提交后阶段转 `needs_input`，服务端付费提交计数不变） | PASS | 用例 4（截图 `07`） |
| AC-049 · 不显示虚假总进度 | 无「总进度 100%」、无百分比、无预计剩余 | 列表 + 详情整页文本：无 `总进度`、无 `\d+%`、无 `自动发布`；阶段用「共 N（已完成/进行中/缺项/待对账/失败/已取消）」计数与阶段列表 | PASS | 用例 1/8；发布包 bundle 同样无 `总进度`/`自动发布` 字符串 |
| AC-033（UI）· 分列与单位 | credits 与 USD 分列、不相加成无单位数字 | 列表行与详情费用区块的每个金额元素文本**逐字段等于**服务端 `reservedDisplay`（如 `30.00 credits`、`0.01 USD`）；credits 与 USD 同屏各自带单位；费用区块文本无「合计/总计/total」 | PASS | 用例 1（与 API 对账） |
| AC-033（UI）· 已消耗/预留/unknown 保留 | 三种语义可见；unknown 不显示为 0 | `已结算（按供应商计费事实）`+「已消耗」标签（tripo 实际 30.00 credits）、`预留中（已占用预算）`（进行中任务实测，金额非 0）、`未决预留（等待对账）` 显示 30.00 credits；SQLite `cost_ledger` 导出：`tripo|3000|NULL|unknown`（**actual 仍为 NULL，未被填 0**）、`tripo|3000|NULL|released`、`tripo|3000|3000|settled`、`manual_ai|8504|NULL|reserved` | PASS | 用例 1/2/4/6 + `r21-cost-ledger-dump.txt`（截图 `03b`、`07`） |
| AC-033（UI）· 预算语义说明 | 明示「应用侧控制、非供应商账户级封顶」 | 详情费用区块常驻服务端 `budgetNotice`（实测含「不是供应商账户级硬封顶」） | PASS | 用例 1 |
| AC-034（UI）· 不自动降质量/换模型/加阶段 | 无此类快捷入口 | 列表 + 详情均**无**按钮/链接/单选项名称含 降质量/换模型/增加阶段/自动修复/一键重试/立即重试/自动校准；失败页只在说明文字里写「应用不会自动降质量、换模型或增加处理阶段」 | PASS | 用例 1/8（`expectNoShortcutControls`） |
| AC-034（UI）· 超出上界被拒且说明原因 | 拒绝且给真实恢复路径 | 模型分支 `model_validate=needs_input`：详情 `retry.allowed=false`、`reason=budgetNotHolding`，界面显示服务端原因（含「重新获取报价并确认预算」）+「该阶段当前不提供重试入口」，且**不渲染**重试按钮 | PASS | 用例 5（截图 `08`） |
| AC-034（UI）· 重生成需新报价确认 | 无静默降级路径 | 任务中心不存在任何「重新生成/降质量重试」入口；被拒原因指向重新报价/新建任务（向导第 5 步的报价确认由 T16 已验收路径承载） | PASS | 用例 1/5/8 + 源码复核（`features/jobs/**` 无此类控件） |
| AC-034（UI）· unknown 实际费用不被填 0 | actual 不写 0 | `cost_ledger` 的 unknown 行 `actual IS NULL`（schema CHECK 也禁止 unknown 带 actual）；UI 显示未决预留金额而非 0；对账 `recordNoTask` 后预留仍为 `unknown`（不自动释放/清零） | PASS | 用例 4 + `r21-cost-ledger-dump.txt` |
| UI-027 费用区块 | 分列 + unknown + 语义说明；失败不显示 0 | 见 AC-033 三行；读取失败时费用区块不渲染（错误面板 + 重试），不出现 0 金额 | PASS | 用例 1/2/4；源码 `CostBreakdown.tsx` |
| UI-028 超出上界呈现 | 拒绝 + 无降级开关 + 重生成需新报价 | 见 AC-034 三行 | PASS | 用例 5/8 |
| UI-029 列表与轮询 | 游标分页、状态/阶段摘要/费用、空/加载/网络态、最后更新时间 | 列表渲染物品名·型号、状态标签、`阶段：共…`、分列费用、终态行草稿入口；空态「还没有任务」+ 去新建物品；网络态保留旧数据 + 「最后更新时间…（网络异常，数据可能已过期）」；`/jobs?itemId=` 服务端过滤由资料库行内「查看任务」进入（范围提示可见） | PASS | 用例 1/3/6（截图 `01`、`12`） |
| UI-030 状态区分与下一步 | 九状态独立标签 + 建议操作；不显示猜测值；无线性百分比 | `status.ts` 九状态各有 label/category/nextStep（单测 8 条 + 页面实测：详情「下一步：…」按状态变化）；未知取值原样显示 `未知状态（…）`；无任何百分比 | PASS | 用例 1/2/4/6/8 + 单测 `status.test.ts` |
| UI-031 needs_input 缺项与下一步 | 列可行动缺项、保留 task_id、给「去补齐」链接、不默认「重新生成」 | 拒答批次显示服务端缺项消息 + 「去检查 PDF 准备」链接；`recovery-summary` 常驻说明；无「重新生成新任务」默认入口；30 分钟超时文案由服务端 `needsInput[].message` 提供（含「已停止自动等待，远端任务 ID 已保留」）并被界面原样渲染 | PASS（UI 呈现层）／30 分钟文案未做端到端触发（见未覆盖边界） | 用例 5；`crates/server/src/jobs/executor.rs:909` 文案 + `jobs_recovery.rs` 断言 |
| UI-032 retry_wait 展示 | 「第 n/5 次重试，将在 X 秒后重试」+ 倒计时；退避期间无「立即重试」 | **偏差**：状态标签为「退避重试中」、显示「尝试 N 次」与「下次运行约 <绝对时间>」，**没有秒级倒计时、也没有 n/5 次数**；「退避期间不提供立即重试」满足（重试入口只在 `failed/needs_input` 渲染）；发布包 bundle 中不存在 `秒后重试`/`次重试` 字符串 | **PASS（半）→ 观察 1（P3，交 PM）** | 源码 `JobStageList.tsx:154-158`；`grep apps/web/dist/assets/*.js` |
| UI-033 断网不误报 | 网络问题 vs 业务失败分开 | 见 AC-049 断网行（详情 + 列表两处） | PASS | 用例 3/6（截图 `05`、`12`） |
| UI-034 unknown 展示与对账入口 | 不渲染重试（非禁用）、主操作对账、预留未决 | 见 AC-049 unknown 行；另有 `reconcile-summary` 任务级提示；Manual AI 分支不显示 `attachRemoteTask`（由服务端 `submissionStyle` 决定，QA 实测 `manual_extract=syncResponse`、`tripo_submit=asyncRemoteTask`） | PASS | 用例 4 |
| UI-035 对账三动作 | 各自后果与二次确认；recordNoTask 需证据；authorizeReplacement 二次确认 | `recordNoTask`：空证据禁用提交（实测），提交后阶段转 `needs_input` 且预留保留；`attachRemoteTask`：仅异步链路出现 + 「我确认…（二次确认）」勾选 + 失败不修改记录（文案）；`authorizeReplacement`：按钮进入确认抽屉，写明「新的付费请求/可能重复收费/旧未决账务保留」，需再次预算确认（低于上界被服务端拒绝——T15 端点侧已验收） | PASS（三动作 UI 与后果提示；真实供应商验证属 T23） | 用例 4（截图 `07`）；组件测试 `jobDetail.test.tsx` 170–227 |
| UI-036 取消 | 确认对话写明「不取消已提交的付费操作」；终态不显示取消 | 取消抽屉固定文案「已提交给供应商的付费操作不会被撤销（不保证供应商撤单）…未决预留不自动释放」；确认后状态转 `已取消` 并显示 `cancel-not-needed`；终态任务无取消按钮 | PASS | 用例 1/7 |
| UI-037 按分支重试 | 只列可重试阶段、不改变预设、unknown 不提供 | 只有 `failed/needs_input` 且服务端 `retry.allowed=true` 才渲染按钮；文案「只重跑该阶段…不改变模型/质量预设；重试需要该分支仍有预算背书」；unknown 阶段不渲染 | PASS | 用例 4/5 |
| UI-038 阶段明细 | 逐阶段展示状态/用时/尝试/错误/产物；未知状态原值 | 9 类阶段按种类列出（批次带序号）、页范围、尝试/查询次数、`nextRunAt`、错误摘要、`knowledgeProduced=false` 提示「未产出正式知识（拒答/截断/格式错/校验失败）：不自动重试、不重复付费」；无编造百分比/剩余时间 | PASS | 用例 1/4/5（截图 `07`、`08`） |
| UI-039 校验失败与 needs_input | 原因可读 + 原始模型保留 + 不提供「降低面数/自动修复」 | 截断 GLB 链路：`model_validate` 缺项消息 + 不渲染降级控件；`needs_input` 阶段显示服务端缺项（超面数/外链等情形在 T13 端点侧已验收） | PASS | 用例 5/8 + `expectNoShortcutControls` |
| **T15 P3① 复验** | 详情「不可重试」必须与端点同判据；不得再出现「可对该阶段重试」文案 | 详见下节 | PASS | 用例 5 |
| 卡内项 · 关浏览器后服务端继续 | 任务不受浏览器影响 | 页面关闭 → 服务端继续推进至 `needs_input`；重开后读到新状态（状态串与关闭前不同且来自服务端） | PASS | 用例 2 |
| 卡内项 · 服务重启恢复 | DB 为准 | 见 AC-049 恢复行 | PASS | 用例 6 |
| 卡内项 · 重复操作不多收费 | 幂等/状态机不产生第二次付费 | 对账提交中按钮禁用 + 重放同请求体 → 422 无副作用；付费提交计数（fixture）全程不增；被拒重试、取消均不新增提交；**重试（被允许）只重新请求说明书 AI，不新增 tripo 提交** | PASS | 用例 4/5 |

## T15 P3① 复验结论（回合 17 遗留项）

| 判据 | 实测 | 结论 |
| --- | --- | --- |
| 模型分支头 `needs_input` 时详情不得出现「可重试」文案或无效入口 | 详情 `stages[model_validate].retry = {allowed:false, reason:"budgetNotHolding", message:"…请重新获取报价并确认预算后再执行"}`；界面 `stage-retry-denied` 可见、`stage-retry-reason` 含「重新获取报价」、该行 `stage-retry-button` **计数 0**；整页文本不含「可对该阶段重试」「可对失败阶段重试」 | PASS |
| 详情判据与端点必须同源（不能"写着不可重试、其实能重"或反之） | QA 用同一 ETag + 新幂等键真的 POST `/jobs/{id}/retry`：**422**，`error.details.reason` 与 `error.message` **逐字等于**详情的 `retry.reason`/`retry.message`；被拒后无副作用（付费提交计数不变） | PASS |
| 允许的重试必须真的生效（不是沉默按钮） | 对 `retry.allowed=true` 的知识批次点击「重试该阶段」→ 服务端重新请求说明书 AI（fixture 计数 +1）→ 该批 `succeeded`、`knowledgeProduced=true`；tripo 付费提交计数不变 | PASS |
| 草稿缺项文案不再承诺可重试 | 「知识完整 + 模型分支缺项」链路产出 partial 草稿：草稿 JSON **不含**「可对该阶段重试」、**含**「恢复动作」（服务端文案「请在任务中心查看该阶段可用的恢复动作（重试需要该分支仍有预算背书）」）；父 job 保持 `needs_input` 不冒充 `succeeded` | PASS |
| 是否需要 PM 新裁定 | RD 选择「文案/入口如实」而非放宽预算门槛（ADR-028 第 3 条）。QA 认同：AC-040 已验收语义为「预留不占预算即拒绝」，放宽属需求变更。若 PM 认为本地校验/下载类阶段应免预算重试，需新裁定并重开 AC-040 | 交 PM（非阻断） |

## QA 自身验收测试的事实更新（回合 18 的过时断言）

RD 报告的全量 e2e 2 条失败来自 QA 在回合 18 写的"任务中心尚未交付"事实断言。按派发授权，QA 在**确认 T17 行为正确后**（命令 #1/#2/#3 全绿）只做事实更新，**未放宽任何守卫**：

| 位置 | 原断言（已不成立的事实） | 更新后的断言 | 说明 |
| --- | --- | --- | --- |
| `qa-t16-independent.spec.ts`（QA-1 · 物品概览） | 文本「任务中心（T17）与阅读器（T18/T19）尚未交付」可见 | 「阅读器（T18/T19）尚未交付」可见 + **「本物品的任务」链接 href 匹配 `/jobs?itemId=`** + 「查看任务/打开说明书」链接计数 0 保持 | 未交付能力仍无可用入口（守卫不变） |
| 同上（QA-1 · 资料库行内入口） | 「查看任务」是**禁用按钮**且 `title` 含 T17 | 「查看任务」现在是**可用链接**（href 匹配 `/jobs?itemId=`）；「打开说明书」仍禁用且 `title` 含 T18 | 该断言在回合 18 被 :287 提前失败遮蔽，RD 未列出；QA 一并对齐（同属 T17 交付导致的事实变化） |
| 同上（QA-5 · 受理面板） | 点击「查看任务」后出现占位页文案「还没有实现」 | 进入**真实任务详情页**：URL 匹配 `/jobs/<uuid>`、`阶段明细` 可见、页面无「发布」入口 | AC-048 的"无自动发布路径"在任务页同样断言 |

更新后全量 e2e：`qa-t16-independent.spec.ts` **12/12**、整体 **52/52** 全绿（命令 #3）。

## 缺陷

**无新增产品缺陷。** 本回合未发现需要 RD 修复的问题；T17 切片内 AC-049、AC-033/AC-034 的 UI 侧与 UI-027–UI-039（除下述观察 1 的字面倒计时）均通过。

## 非阻断观察（交协调者／PM；不计为缺陷）

1. **UI-032 的字面「倒计时」与「第 n/5 次重试」未实现（P3，交 PM 裁定）**：实测界面为状态标签「退避重试中」+「尝试 N 次」+「下次运行约 <绝对时间>」，无秒级实时倒计时、无 `n/5`；发布包 bundle 不含 `秒后重试`/`次重试`。AC-036 的必选可观察项（状态转换、退避序列、上限、Retry-After）由 `cargo test` 侧承载且已通过；UI-032 属 §6.2 交互合同。**请 PM 明确**：是要求在后续卡实现倒计时，还是把「绝对时间 + 尝试次数」记为可接受偏离（若要求实现，请派卡，不阻断本切片）。
2. **顶栏「进行中任务计数徽标」未实现（P3，RD §T17-10 第 1 条已披露）**：PRD §6.1.1 要求任务中心入口带计数徽标；现仅「任务中心」入口（无徽标），且无独立 UI ID。属全局轮询取舍，请 PM 决定间隔/预算后派卡。
3. **RD 记录的 dist/smoke 证据与最终工作树不一致（P3，文档一致性）**：`implementation.md` §T17-8 #12 的 sha256 `3262691d…` 对应 18:13Z 构建，而 5 个前端源文件在 18:22–18:23 才修改（跨页文案同步）；RD 的 #2–#8、#11 也在该修改之前。QA 已在最终工作树重跑全套（命令 #4–#9）并验证 dist 内嵌最终文案，结论不受影响；建议后续 RD 在收尾改动后重跑 dist/smoke 并更新记录（本回合**未改** implementation.md）。
4. **证据目录共享导致互相覆盖（沿用回合 9 的 P3-2）**：本回合多次运行覆盖了 `artifacts/web-mvp/t17-rd/screenshots/**` 与 `artifacts/web-mvp/t09-rd/playwright-output/**`（RD 原截图被同场景的新截图替换，路径仍有效）。建议 T21/T22 前按运行隔离。

## 非代码知识与限制

1. **headless Chromium 无法用 `bringToFront()` 制造 `visibilityState=hidden`**（QA 回合 21 探针实测：两个标签页均为 `visible`，`r21-visibility-probe.log` + `qa21-visibility-probe.spec.ts.txt`）。因此 AC-049「后台 15 秒」只能用 `visibilityState` 改写 + `visibilitychange` 事件（与应用读取的同一 API 面）验证；真实浏览器的后台节流仍属未覆盖边界。
2. **QA 用例自身踩到的三个陷阱（供后续复用，不进入产品结论）**：① 付费提交基线必须在**建单之后**采集（`seedJob` 自身会产生 1 次提交）；② 拒答链路下 `needs_input → queued → running → needs_input` 会**快于 250ms**，不能把「离开 needs_input」当作稳定中间态断言（应先切换 fixture 脚本再点重试）；③ 在**已登录**页面再调用登录辅助函数会等待登录表单直到测试超时——跨路由复用会话时不要再走登录表单。④ 两条分支都缺项时不会产出 partial 草稿（T15 的「上游阻塞不提前组装」），要验证草稿缺项文案须构造「知识完整 + 模型分支缺项」。
3. **e2e 设施约束（沿用并复核 RD §T17-9）**：`job-recovery*.ts` 的后端必须声明 `public_origin = http://127.0.0.1:<E2E_WEB_PORT>`（否则改写后的请求被 Origin 校验拒 403），且必须用 `--features job-failpoints` 的**测试构建**才能放行回环模型下载；CSRF token 与会话 cookie 绑定（每次重新登录后必须重新取 token）。
4. **unknown 的账本保证是结构性的**：`migrations/0001_core_schema.sql` 的 `CHECK (state <> 'unknown' OR actual IS NULL)` 加上对账 `recordNoTask` 的 `reservationReleased:false`，使"未决预留不被填 0/不自动释放"同时有 schema 与业务两层保证（QA 用 `cost_ledger` 导出复核）。
5. **RD 的 T17 证据链结论与 QA 复核一致**（8 passed 的 e2e、79 单测、477 workspace、7/7 xtask check、contracts 一致），但 QA 独立重跑了全部命令；RD 记录的 dist 除外（观察 3）。

## 未覆盖边界（本回合记录，不冒充通过）

1. T18–T23（阅读器/热点校准与发布/导出备份/正式多平台单包/真实授权链路）仍未实现/验收；**MVP 尚未完成**。
2. `retry_wait` 的浏览器呈现（UI-032）：本机 fixture 没有"安全临时失败（429/5xx on 查询/上传）"脚本，无法在浏览器里产生 `retry_wait` → 该项**未做端到端观察**，观察 1 的结论来自源码与 bundle 字符串核对。
3. 30 分钟总等待转 `needs_input` 的端到端 UI 触发（服务端文案由 `jobs_recovery.rs` 断言，本回合未在浏览器中触发该路径）。
4. Manual AI 同步链路 `submission_unknown` 的 UI（对账面板不提供 `attachRemoteTask`）：fixture 无该模式，只在服务端 `submissionStyle` 字段与组件测试层面覆盖，**未做浏览器端到端**。
5. 真实供应商账户下的对账三动作与真实费用（T23 需真任务 ID 与预算）；顶栏任务计数徽标；100 页/20 批规模下轮询与列表开销（T21）。
6. 真实浏览器后台节流的精确间隔（见"非代码知识"1）。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–17 | 2026-09-12 | slice | T01–T15 | 1/2 | PASS（回合 10、12 各 FAIL→修复后 PASS） | 本文件上半部分 |
| 18 | 2026-09-12 | slice | T16 | 2 | **FAIL**（BUG-005） | 本文件 |
| 19 | 2026-09-12 | slice | T16 | 2 | **PASS**（BUG-005 → CLOSED；BUG-006 立案） | 本文件 |
| 20 | 2026-09-12 | slice | BUG-006 | 2 | **PASS**（BUG-006 → CLOSED） | 本文件 |
| 21 | 2026-09-12 | slice | T17 | 2（ui_revision 2） | **PASS**（0 新增缺陷；4 条非阻断观察） | 本节 |

- 交接给 RD：**无必修项**。可选（非缺陷，经协调者派卡后再做）：观察 1 若 PM 要求实现 UI-032 倒计时；观察 3 的 dist/smoke 记录刷新。
- 交接给协调者：(a) `qa_result: PASS`、`qa_round: 21`、`qa_history` 追加 `{round: 21, scope: slice, task_ids: [T17], prd_revision: 2, result: PASS}`；(b) `accepted_tasks` 追加 `{task_id: T17, prd_revision: 2, qa_round: 21, accepted_ac_ids: [AC-049, "AC-033 UI 侧", "AC-034 UI 侧", "UI-027–UI-039（UI-032 见观察 1）"], status: accepted}`（**条目格式由主会话按 state 约定落盘**）；(c) `carry_over` 中 T15 P3① 条目已复验通过，可移除并把观察 1–4 记入新的 carry_over；(d) T16 的 accepted 条目不变（qa-t16 只做事实更新，AC-026/AC-048 语义未变）；(e) 本回合**未改** prd.md / state.yaml / 规范文档，未改生产代码；改动仅 `qa-t17-independent.spec.ts`（新增）、`qa-t16-independent.spec.ts`（3 处事实更新）、`artifacts/web-mvp/t17-qa/**` 与本报告。
- 交接给 PM：观察 1（UI-032 字面倒计时是否要求）与观察 2（顶栏计数徽标）需要产品裁定；两者均不阻断 T17 切片。
- 全项目状态提醒：T17 通过后进度为 T01–T17 已验收；T18–T23 未交付，**MVP 尚未完成**；完整发布仍需 T21 全量回归（含真实平台/浏览器矩阵）、T22 冷目录 smoke、T23 授权后的真实 Provider 验证。
- **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「T17 验收知识」（跨卡复用的 9 条手法 + 4 条未关闭观察：headless 无 `hidden`、轮询测量法、状态瞬态陷阱、付费基线时机、unknown 账本核对法、控件角色断言、e2e 设施四约束、产物/工作树一致性核对）；`llmdoc/validation-release.md` 命令合同**无变化**（本回合未发现需要修订的验收命令或阈值）；本报告即验收依据、缺陷与未覆盖边界的正式记录。QA 未改 prd.md／state.yaml／规范文档语义、未改生产代码。
- QA 产出与证据：`artifacts/web-mvp/t17-qa/`——`r21-rd-spec-job-recovery.log`、`r21-qa-spec-independent.log`、`r21-qa-spec-independent-run2.log`、`r21-qa-spec-test5-iter{1,2}.log`、`r21-qa-spec-run.log` + `r21-qa-spec-lsof-loopback-only.log`、`r21-e2e-full-suite.log`、`r21-e2e-full-final-run.log` + `r21-e2e-full-final-lsof-loopback-only.log`、`r21-visibility-probe.log` + `qa21-visibility-probe.spec.ts.txt`、`r21-cost-ledger-dump.txt`、`r21-{typecheck,lint,vitest,web-build,cargo-test-workspace,xtask-check,contracts-check,dist,dist-sha256,smoke-bootstrap}.log|txt`、`qa-r21-lsof-loopback-only.sh`、`screenshots/01–12`（13 张）。测试代码：`apps/web/tests/e2e/qa-t17-independent.spec.ts`（QA 新增，8 用例）。

---

# 回合 22 · T18（GLB 阅读器和资源恢复）

**结果：FAIL（1 个 P2 缺陷 BUG-007）** · 回合：22 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T18]；AC-050、AC-051、AC-062/REQ-040 资源侧、AC-057 阅读侧、AC-060 前端侧；UI-043/044/045/059）

- QA 执行时间：2026-09-12 19:46–20:06（本地 UTC+8）；执行者：QA 子 agent。**独立复现**：自建"真实链路"e2e（真实草稿 + 真实资产字节）、独立 WebGL 原型计数、独立坐标性质测试；未采信 RD 的 §T18 结论与自测数字（RD 的 `viewer.spec.ts`、`coordinates.test.ts` 由 QA **重跑**核对，不作为结论来源）。
- 派发核对：`state.yaml` current_tasks=[T18]、prd_revision=2、ui_revision=2；`prd.md` §9.1 修订 2 与 UI 同步记录一致；`implementation.md` §T18 声明 RD_READY；派发时开放缺陷 0（本回合新立案 BUG-007）。
- 结论口径：本报告只覆盖 T18 切片；T19–T23 未实现，不作为本回合缺陷；**MVP 尚未完成**。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb44ee2a6088f440eade3f1424546fbb62d`（"init"，仅 1 次提交）；T18 交付物**未提交**（`apps/`、`crates/`、`tests/`、`xtask/` 等均为未跟踪；`git status` 与派发前一致，无预期外改动） |
| T18 源文件快照 | `artifacts/web-mvp/t18-qa/t18-file-hashes.txt`（`src/features/viewer/**`、`tests/e2e/**`、`App.tsx`、`api/{client,endpoints}.ts`、`package.json` 逐文件 sha256）+ 汇总 `3e39a52279c455a7423a51e4fe99716dedb54068aa8aaa7252d315cb13ce6f97`（19:58 CST） |
| 平台 | macOS Darwin 25.6.0 · aarch64-apple-darwin（Apple M1，8 核） |
| 工具链／浏览器 | rustc/cargo 1.98.1；node v26.0.0；npm 11.12.1；@playwright/test 1.60.0（bundled Chromium 1223，headless，真实浏览器） |
| 前端依赖 | three@0.186.0、@react-three/fiber@9.4.2、@types/three@0.186.0（与 ADR-029 一致；未用 `--legacy-peer-deps`） |
| 交付二进制（dist 复核） | `dist/aarch64-apple-darwin/everything-manual` sha256 `6665f5a56c41ab6ea3cbb82489a0a268c5377f87489684a729bebeb6a81e796f`（23 618 464 B）——QA 独立重建，与 RD §T18-9 #11 记录**逐字节一致**，说明验收对象就是 RD 交付的源码状态 |
| 数据／Provider（fixture 与真实分开） | **fixture 验收**：① e2e `globalSetup` 后端（provider 指向 127.0.0.1:1）；② **测试构建后端（`--features job-failpoints`）+ 本机 fixture 供应商**（真实流水线，产出真实草稿与真实模型资产；HTTP fixture 计数：upload 4 / submit 2 / task 2 / manual 2 / cdn 2，全部本机回环）。**真实 Provider 验收：未做**（无授权，属 T23）。**零真实外网**：所有 e2e 用例断言非本机请求 0 条 |
| 未验证（非本回合范围） | AC-061 的 p95 帧耗时（T21 具名设备 + 人工复核，本回合未复跑 RD 的 perf 脚本）、真实 Provider/费用（T23）、Safari、真实 Chrome channel、多材质大模型、窄屏性能 |

## 验收方法与独立观察手段（结论不依赖 RD 的账本）

1. **真实端到端链路（QA 新增）**：`qa-t18-independent.spec.ts` 用测试构建后端 + 本机 fixture 供应商走**真实流水线**（Tripo 分支真实提交/查询/下载 fixture GLB → `model_validate` → `assemble_draft` 落库），浏览器只做源站改写（Vite 15173 → 测试后端端口），**草稿 DTO 与模型字节全部来自真实后端**；用例断言 `route.fulfill` 计数为 0（无正常响应伪造）。
2. **独立 GL 计数**：在 `WebGL2RenderingContext.prototype` 上包 `create/delete/draw` 计数（累加器跨上下文），不读 RD 的 `window.__EM_VIEWER__`；账本只用于对照。
3. **独立坐标重算**：世界坐标用第一性原理 `(局部 − 包围盒中心)·(1/半径)` 重算并与桥的往返误差（<1e-6）比对；另有 10 条随机化性质测试（含与 three `Object3D` 交叉验证）。
4. **故障注入**：加载态（延迟 1.5 s）与错误态（500）都作用在**真实端点**上；WebGL 不可用用 `getContext` 返回 null（真实降级路径）。
5. **可观察证据**：截图 + JSON（`r22-*.json`）+ 日志 + CDP 指标；上下文丢失用 `WEBGL_lose_context`（与 three `forceContextLoss` 同一 API）。

## AC 验收矩阵（逐子句）

| AC / UI | 期望（可观察） | 实际（QA 实测） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| AC-050 · 懒加载 | 3D 模块懒加载 | 资料库首屏：`three/@react-three/ViewerStage` 与 pdfjs 相关模块请求 **0 条**、模型/PDF 字节请求 0 次；进入阅读页后 three 请求出现；`dist/index.html` 只 preload 入口 + jsx-runtime + CSS（无 ViewerStage chunk）；入口 chunk `WebGLRenderer` 字符串 0 次、ViewerStage chunk 6 次 | PASS | `qa-t18-independent-run4.log`（QA-T18-1/6）+ `apps/web/dist/index.html` |
| AC-050 · 三轴操作与复位 | OrbitControls 旋转/缩放/复位（+键盘等价控件） | 真实模型：拖动改变位姿、滚轮改变相机-目标距离、键盘「复位视角」回到初始取景（<1e-3）、「适配模型」重新取景且 targetLocal=包围盒中心；RD 用例 2 在 QA 重跑中 8 passed | PASS | QA-T18-2 + `artifacts/web-mvp/t18-qa/viewer-e2e.log` |
| AC-050 · loading 可观察 | 有加载态 | 延迟注入下 `viewer-loading` 显示「正在加载模型…（已接收 N 字节）」；就绪后状态行「模型已加载，可旋转/缩放。」 | PASS | QA-T18-5（`qa-t18-independent-run4.log`） |
| AC-050 · error 可观察 | 有错误态 + 重试 | 500 注入：`viewer-error`「模型加载失败：…」+「重试加载」；撤掉注入后点重试**真的**加载成功（真实端点） | PASS | QA-T18-5（截图 `r22-04-error-state-with-text-pdf.png`） |
| AC-050 · context lost → restored | restored 后重建渲染，不只刷新 | **自动 restored 分支**：状态「3D 显示已中断，正在重建…」→「3D 显示已恢复。」，按钮重新启用，帧计数与**独立 draw 计数**继续增长，相机位姿保留（1e-4）。**手动「立即重建」分支**：模型重新加载并真实绘制（modelsAlive=1，draw 25→335），但面板状态与控件不恢复（状态停「已中断」，8 s 后变「浏览器 3D 上下文不可用」；「复位视角/适配模型」永久 `disabled`） | **部分 PASS／手动重建分支 FAIL → BUG-007（P2）** | QA-T18-3；`r22-rebuild-state.json`；`screenshots/r22-03b-after-rebuild.png` |
| AC-050 · 卸载后释放资源 | geometry/material/texture 释放 | 每次换草稿：账本存活 1/1/1、`modelsLoaded−modelsDisposed==1`；**QA 独立**每轮真实 `gl.deleteBuffer×4 + gl.deleteTexture×1`；离开阅读页后账本 `disposed==created`、alive 0/0/0，且真实删除发生 | PASS | QA-T18-4 + `r22-resource-trend.json` |
| AC-050 · 文本/PDF 不依赖 WebGL | 3D 失败仍可读文字与 PDF | 真实数据（真实草稿 + 真实 PDF 字节）：3D 500 失败时左栏「后盖」、右栏「取下后盖」、原文 canvas 有非白像素；WebGL 完全不可用时同前，且 **three chunk 0 条请求**（不浪费下载） | PASS | QA-T18-5/6（截图 `r22-04/05`） |
| AC-051 · 同一局部点一致 | 各种旋转/缩放下世界位置一致（`coordinates.test.ts`） | RD 15 条 + **QA 新增 10 条**全绿：200 组随机「旋转 + 非均匀缩放 + 平移」往返 ≤1e-9；与 three `Object3D` 交叉一致；显示变换（居中+等比例缩放）独立重算 ≤1e-9；两级组合变换；法线逆转置垂直于两条变换后切线；相机 up 逆向一致；stale/NaN 判定；fit 语义。真实模型 3 个采样点在旋转/缩放/复位/适配后世界坐标不变（<1e-6） | PASS | `qa-t18-independent.test.ts`（10 passed）、`coordinates.test.ts`（15 passed）、QA-T18-2 |
| AC-051 · 换模型无旧资源/旧热点 | 不串旧资源/旧热点 | 真实链路 10 次换草稿：桥的 `assetId/sha256` == 当前草稿、`reader-context` 只含当前草稿 id（无上一份残留）；每轮真实删除旧资源。`stale` 热点不显示**在真实链路无法构造**（真实草稿无 `hotspots` 字段，热点属 T19）——该子句目前只有 RD 的合成 DTO 用例 | PASS（资源/草稿残留 = QA 实测；热点子句 = RD 合成 DTO 覆盖，局限见 N1/未覆盖边界） | QA-T18-4；RD `viewer.spec.ts` 用例 4（QA 重跑） |
| AC-062 · 首屏不强制加载 3D/PDF | 库列表不加载 3D/PDF | 见 AC-050 懒加载行（3D 与 pdfjs 双 0 条；模型/PDF 字节 0 次） | PASS | QA-T18-1/6 |
| AC-062 · 10 次切换无持续增长 | 资源数与内存无持续增长趋势 | 真实链路 10 次切换：存活恒 1/1/1；每轮真实删除 4 buffers + 1 texture；WebGL 上下文 12（1 探测 + 11 挂载）且每次卸载都被显式释放（context lost ≥ 10）；CDP `JSHeapUsedSize` 首末比 **1.05**（10 次采样有波动无趋势） | PASS | `r22-resource-trend.json`、`qa-t18-independent-run5-t4.log` |
| AC-057 阅读侧 | 部件列表为 3D 热点的文字替代；非 3D 操作键盘可达、可见 focus | 3D 不可用时左栏部件列表可用（真实部件「后盖」）；「改用文字阅读」键盘聚焦有可见 focus（outlineStyle≠none），回车后焦点落到 `parts-panel`；步骤/引用按钮为真实按钮（键盘可达） | PASS | QA-T18-6（截图 `r22-05-*`）+ RD `viewer.spec.ts` 用例 6（QA 重跑） |
| AC-060 前端侧 | 减少动效、错误关联 | `prefers-reduced-motion: reduce` 下阅读器照常工作（本来无相机动画/过渡）；状态行 `role="status"`；工具栏按钮 `disabled` 有原因文案 | PASS（本卡前端侧子集；窄屏/表单关联属 T16 已验收路径） | QA-T18-6 + 源码 `ViewerPanel.tsx` |
| 卡内 · asset-root 坐标语义 | 显示居中/缩放在外层 group；读写同源 | 源码复核：display group `scale.setScalar(...)`（等比例）、asset-root 自身变换从不写；`createAssetRoot` 读写都刷新 `matrixWorld`（同一对象 `worldToLocal/localToWorld`）；`composeTransforms` 仅等比例父级成立（QA 用反向用例固化该边界） | PASS | 源码 + `qa-t18-independent.test.ts` |
| 卡内 · 不用 mesh.uuid／易变节点名 | 身份 = `modelRevisionId + modelSha256` | `grep` viewer/**：无 `uuid`/`getObjectByName`/节点名锚点；`checkAnchor` 按版本+哈希判 `current/staleRevision/staleSha/invalid`（含 NaN/Infinity） | PASS | 源码 + 测试 |
| 卡内 · 非均匀缩放法线 | 逆转置处理 | 200/100 组随机非均匀缩放：变换后法线与两条变换后切线垂直（≤1e-9）且归一；"朴素做法不垂直"的反例成立 | PASS | `qa-t18-independent.test.ts` |
| 卡内 · raycast 约定（不越界 T19） | 热点创建属 T19，本卡只需坐标层可复用 | 热点标记 `raycast={() => null}` + `userData.emHotspot`（供 T19 排除）；坐标层与桥提供 `toLocal/toWorld/normalToWorld/checkAnchor/makeAnchor/computeFit`（§5.4 可复用接口）；`features/viewer/**` 无热点创建/绑定/发布写入（无 PATCH/POST） | PASS（接口可复用、无越界实现） | 源码复核 |

## 重点评估项：RD 的"草稿 DTO 与模型字节由浏览器侧拦截提供"

**结论：判定为"不充分的验收论证"（非阻断，交 RD/协调者按 N1 更正与补测）**，依据如下。

1. **红线核对（"不用 mock 冒充"）**：RD 在 `implementation.md` §T18-8 与 ADR-029 第 9 条**主动披露**了拦截，不是隐瞒；GLB 字节是仓库内真实 fixture 文件的逐字节实体，请求路径/客户端代码真实，且会话、物品、document、preparation、页资产、**原 PDF 字节**都走真实后端。因此**不构成"用 mock 冒充已测"的欺骗**；但"阅读器能否消费服务端真实产出的草稿"这一集成问题在 RD 的证据里**完全未被覆盖**。
2. **当前服务端能力下存在真实端到端路径（QA 已实测跑通）**：
   - `GET /assets/{id}/content`（T06）**已可用**、无测试构建门控，按 asset id 直接服务 data-dir 中的模型字节；
   - `GET /items/{id}/drafts/{draftId}`（T15）**已可用**，真实草稿携带 `knowledge.model.{revisionId, sha256, assetId, validationState}` 与合并知识；
   - 唯一缺的是"造出带模型版本的草稿"：正常构建不能用本机 fixture 供应商，但**测试构建（`--features job-failpoints`）+ 显式测试配置**（`download.allowed_hosts=["127.0.0.1"]`、`allow_local_fixture=true`、Provider base_url 指向本机 fixture）即可用真实流水线产出草稿（与 T17 QA 同一设施，属 validation-release §2 允许的 fixture 准入方式）。
   - QA 据此新增 `qa-t18-independent.spec.ts` 并跑通：**真实草稿 + 真实资产字节 + 真实 PDF** 驱动阅读器（模型 sha256 与草稿一致、12 三角面、真实部件/步骤、原文 canvas 有像素）。→ **RD 论证中"服务端在 T18 尚无模型版本→资产端点"不成立**；"T19 交付后才能替换拦截"也**不是必需**（T19 新增的是 release 读取端点，与草稿/资产读取是两条路径）。
3. **复验要求（非阻断）**：
   - 更正 `implementation.md` §T18-8 与 ADR-029 第 9 条的表述（或注明已由本回合实测推翻），避免后续角色误判"没有真实路径"；
   - **T19 门禁要求**：release 阅读器必须至少有一条**不拦截草稿/模型字节**的真实链路用例（可直接复用 QA 的本机 fixture 流水线法，见 `qa-t18-independent.spec.ts` 的 `installRouting`/`buildTestServerBinary` 组合）；
   - RD 现有拦截用例可保留（对"热点 stale 不显示"等 T19 前无法真实构造的分支仍有价值），但断言不能只建立在合成 DTO 上。

## 缺陷

### BUG-007 · 手动「立即重建」成功后阅读器状态与控件不恢复（状态假报"上下文不可用"、键盘等价控件永久禁用）

- 严重度／状态：**P2 / OPEN**
- 对应 REQ / UI / AC：REQ-032；UI-044（"重建中禁用交互"、成功恢复；禁用"只把刷新页面当唯一手段"）、UI-043（键盘等价控件）、AC-050
- 环境与输入：macOS Darwin 25.6.0 arm64；Playwright Chromium 1223（headless，@playwright/test 1.60.0）；真实后端（测试构建 + 本机 fixture 供应商）+ 真实草稿/模型；路由 `/items/:itemId/drafts/:draftId/review`
- 复现步骤：
  1. 打开阅读页并等模型渲染（独立 draw 计数 > 0）；
  2. 对 `[data-testid="viewer-canvas"]` 的 webgl2 上下文调用 `getExtension("WEBGL_lose_context").loseContext()`；
  3. 确认状态行「3D 显示已中断，正在重建…」且「立即重建」可见（此时符合预期）；
  4. 点击「立即重建」，等新 canvas 与模型加载完成（可观察到模型重新渲染）；
  5. 观察状态行与工具栏按钮。
- 期望：重建完成后状态回到可用（如「模型已加载，可旋转/缩放。」），「复位视角」「适配模型」恢复 enabled，错误文案只在确实不可用时出现。
- 实际：模型确实重建并渲染（`modelsAlive=1`；draw 25→335），但状态行停留「3D 显示已中断，正在重建…」，8 秒后升级为「浏览器 3D 上下文不可用：请使用文字阅读。」；`复位视角/适配模型` 保持 `disabled`（AC-050 的复位与 UI-043 的键盘等价路径不可用）；用户只能整页刷新才能回到可用状态。
- 证据路径：`artifacts/web-mvp/t18-qa/r22-rebuild-state.json`、`artifacts/web-mvp/t18-qa/screenshots/r22-03b-after-rebuild.png`、`artifacts/web-mvp/t18-qa/qa-t18-independent-run4.log`（用例 QA-T18-3 在此断言失败）；一键复现：`npm --prefix apps/web run test:e2e -- qa-t18-independent.spec.ts -g "QA-T18-3"`
- 回归范围：`apps/web/src/features/viewer/ViewerPanel.tsx`（`contextState`/`contextUnusable`/`unavailableTimer` 在 rebuild 路径的重置）、`apps/web/src/features/viewer/ViewerStage.tsx`（重建挂载时是否上报 `ok`）；建议补断言：重建完成后 ① 状态文案恢复 ② 两个工具栏按钮 enabled ③「立即重建」不再显示，并纳入 `viewer.spec.ts` 用例 3。
- 根因摘要（QA 代码复核，供 RD 参考，不是替 RD 定方案）：`ViewerPanel.contextState` 只由 stage 的 `webglcontextlost/restored` 回调更新；`rebuild()` 重新挂载 Canvas 后新 `StageScene` 挂载时**不上报 `ok`**，因此 `contextState` 永久停留在 `lost`，`interactive = ready && stageReady && !lost` 永久为 false；丢失时启动的 8 s `unavailableTimer` 在点击重建时也未清除，到期把 `contextUnusable` 置 true。
- RD 修复摘要引用：`implementation.md` §T18-13（2026-09-12 修复回合；挂载上报 `ok`、`unavailableTimer` 清理/重启、`restoring` 状态、显式位姿保留 `restorePoseRef`+`applyPose`）。
- QA 复验结果与日期：**CLOSED**（2026-09-12，QA 回合 23）——一键复现命令 exit 0；QA 新增独立语义用例（状态/控件/8.6 秒稳定/位姿策略）与 RD 用例 3/9 重跑全部通过；证据见「回合 23 · T18（BUG-007 复验）」。修复前失败现场保留于 `artifacts/web-mvp/t18-rd-fix/repro-before-*`（QA 现场文件按用例自身行为会被覆盖）。

## 非阻断观察

1. **N1（文档更正 + T19 门禁）**：见上节第 3 点——§T18-8 / ADR-029 第 9 条"服务端无模型读取端点"与实测不符；`viewer.spec.ts` 的草稿 DTO 拦截应在 T19 前至少有一条真实链路用例替代。不阻断本切片（QA 已用真实链路独立覆盖该集成风险）。
2. **N2（热点子句的覆盖来源）**：AC-051 的"换模型后旧热点不串入"目前只在 RD 的合成 DTO 用例中验证；T19 落库热点后必须补真实链路用例（见未覆盖边界 4）。
3. **N3（证据目录共享，沿用回合 9 的 P3-2）**：本回合的 Playwright `outputDir`（`artifacts/web-mvp/t09-rd/playwright-output/**`）会被各次运行覆盖或清理；QA 的失败现场截图已另存 `artifacts/web-mvp/t18-qa/screenshots/`。建议 T21/T22 前按运行隔离输出目录。
4. **N4（RD 的 100k 面性能数字未独立复跑）**：`artifacts/web-mvp/t18-rd/perf-result.json`（真实 Chrome 152 / Apple M1 / ANGLE Metal / 2 223 572 B 模型 / p95 18.7 ms）与 implementation.md §T18-7 一致，但 QA 本回合**未**复跑该脚本——AC-061 的判定属 T21 具名设备 + 人工复核，且 headless Chromium 不作达标依据。本报告不把它写成"已独立验证"。

## 非代码知识与限制

1. **真实链路 e2e 的可复用手法**（已追加到 `llmdoc/decisions.md`）：测试构建 + 显式测试配置 + 本机 fixture 供应商 + Vite 源站改写（后端配置必须声明 `public_origin = http://127.0.0.1:<页面端口>`，否则修改请求被 403）⇒ 浏览器消费**真实草稿与真实资产字节**，且零外网零付费。比"浏览器侧伪造 DTO"强，可直接被 T19/T20 复用。
2. **独立 WebGL 计数的正确姿势与坑**：必须包在 **WebGL 原型**上；包在上下文实例上会在 SPA 换路由 /「立即重建」后**冻结**（QA 首轮实测计数停止增长，导致"常驻资源稳定"的假通过——这正是"不能只信一个观测器"的例子）。坑：three 每个渲染器实例自建 4 张 empty texture（`WebGLState`：TEXTURE_2D / CUBE_MAP / 2D_ARRAY / 3D），所以"累计 `createTexture` 随上下文数增长"**不是**泄漏指标；应改用"每轮真实删除数 > 0 + 每次卸载 context lost + 账本存活恒定 + 堆趋势"。
3. **`performance.memory.usedJSHeapSize` 可能连续 10 次给出完全相同数值**（RD 记录 42.1 MB 恒定，属量化/GC 时机）；观察趋势改用 CDP `Performance.getMetrics` 的 `JSHeapUsedSize`（本回合 10 次采样 24.5–33.1 MB，首末比 1.05）。
4. **`composeTransforms` 只允许等比例的外层 display 缩放**（非均匀父级产生剪切，逐分量组合 ≠ 真实矩阵链）；写测试时不要用非均匀 display（QA 首轮踩到并已用反向用例固化该契约边界）。
5. **真实草稿的形状**：`knowledge.model.bounds` 已由服务端给出（`{min,max,triangles,vertices}`）；`knowledge.hotspots` 在 T19 前**不存在**——凡"热点"相关断言都不能声称已在真实链路验证。
6. **重建类降级路径的断言教训（BUG-007 根因的泛化）**：涉及"重建/重挂载"的恢复路径，UI 断言不能只查帧数/模型存活，必须查**状态文案与控件可用性**（RD 的用例 3 只查了帧数与存活，因此绿灯）。

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实 Provider（Tripo / 说明书 AI）与真实费用**：本回合全部走本机 fixture（零外网零付费）；真实协议、产物形状、费用须 T23 授权后另验，且与 fixture 结果分开记录。
2. **AC-061 帧耗时**（100k 面、p95 ≤33 ms）：属 T21 具名设备 + 人工复核；本回合未复跑 perf 脚本、未在真实 Chrome channel 测量；headless 不作达标依据。
3. **100k 面模型的"真实链路"**：超过 100000 面预算的模型会被服务端 `model_validate` 拒（needs_input），因此"真实流水线 → 阅读器"的大模型路径无法在 fixture 下构造（RD 的 perf 脚本是浏览器直接加载，绕过服务端）。
4. **热点可视化 / stale 过滤的真实链路**：真实草稿无 `hotspots`（T19），只有合成 DTO 覆盖；热点标记的渲染、`raycast` 排除、点击拾取留待 T19。
5. **手动重建后的相机位姿保留**：UI-044 的"能重建时"允许不保留；本回合只验证自动 restored 分支位姿保留（1e-4），未量化手动重建的位姿行为。
6. **多材质/多贴图/骨骼/大贴图模型、Firefox/Edge/Safari、窄屏性能**：未测（PRD 性能目标只针对桌面；浏览器矩阵属 T21）。
7. **T19–T23 功能**（热点校准、发布、导出备份、多平台单包、真实授权链路）：未实现/未验收，**MVP 尚未完成**。

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/t18-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `npm --prefix apps/web run test:e2e -- viewer.spec.ts`（RD 套件，QA 重跑） | exit 0，**8 passed / 0 failed**（39.3 s）（`viewer-e2e.log`） |
| 2 | `npm --prefix apps/web run test:e2e -- qa-t18-independent.spec.ts`（**QA 新增 6 用例**，真实链路） | exit 1，**5 passed / 1 failed**；唯一失败 = BUG-007 的现场断言（QA-T18-3 的手动重建后状态/控件检查）（`qa-t18-independent-run4.log`；仅 QA-T18-4 单跑 = pass，`qa-t18-independent-run5-t4.log`） |
| 3 | `npm --prefix apps/web run test -- --run src/features/viewer/coordinates.test.ts` | exit 0，**15 passed**（RD 目标测试，QA 重跑） |
| 4 | `npm --prefix apps/web run test -- --run src/features/viewer/qa-t18-independent.test.ts`（**QA 新增 10 用例**） | exit 0，**10 passed** |
| 5 | `npm --prefix apps/web run typecheck` / `lint` / `test -- --run` / `build` | 全部 exit 0；vitest **15 files / 121 passed**（含 QA 新增 10 条；`typecheck.log`、`lint.log`、`web-test.log`、`build.log`） |
| 6 | `cargo test --workspace` | exit 0，**477 passed / 0 failed**（31 个测试目标；`cargo-test-workspace.log`） |
| 7 | `cargo xtask check` | exit 0，**7/7 `[通过]`**（fmt / clippy -D warnings / workspace 测试 / 前端 lint / typecheck / vitest / contracts --check；`xtask-check-final.log`）。注：首跑因 QA 新写文件当时含类型/lint 错误而 exit 1（`xtask-check.log`），修正 QA 文件后复跑通过 |
| 8 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`，未改工作树（`contracts-check.log`） |
| 9 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0，sha256 **`6665f5a56c41ab6ea3cbb82489a0a268c5377f87489684a729bebeb6a81e796f`**（23 618 464 B）——与 RD §T18-9 #11 记录一致（`dist.log`、`dist-sha256.txt`） |
| 10 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` | exit 0，内嵌页面/静态资源/health/未知 API JSON 404/SPA 路由刷新等检查全过（`smoke-bootstrap.log`） |
| 11 | `npm --prefix apps/web run test:e2e`（**全量**：60 = RD 既有 + QA 新增 6） | exit 1，**65 passed / 1 failed / 0 skipped**（5.1 m）；唯一失败 = **BUG-007 的现场断言**（QA-T18-3）；RD 的 `viewer.spec.ts` 8/8、其余既有 52 条全绿（分文件：import-flow 6 / job-recovery 8 / library-row-layout 2 / pdf-preparation 10 / qa-t09 6 / qa-t16 12 / qa-t17 8）（`e2e-full.log`） |
| 12 | 离线核对：e2e 用例断言非本机请求 0 条（`routing.external == []`）+ `dist/` 扫描无 CDN 引用（pdfjs 资源本地 `dist/vendor/pdfjs/{cmaps,standard_fonts,wasm,iccs}`，3.9 MB） | 通过（`qa-t18-independent-run4.log`；`apps/web/dist/vendor/`） |

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–21 | 2026-09-12 | slice | T01–T17、（BUG-005/006） | 1/2 | PASS（回合 10、12、18 各 FAIL → 修复后 PASS） | 本文件上半部分 |
| 22 | 2026-09-12 | slice | T18 | 2（ui_revision 2） | **FAIL**（BUG-007，P2；1 条非阻断观察 N1） | 本节 |

- **交接给 RD（必修）**：BUG-007——手动「立即重建」成功后 `ViewerPanel` 状态与控件不恢复（状态假报"上下文不可用"、`复位视角/适配模型` 永久 `disabled`），违反 UI-044/AC-050 的恢复语义。修好后**必须由 QA 复验**（`npm --prefix apps/web run test:e2e -- qa-t18-independent.spec.ts -g "QA-T18-3"` + RD 自己的 `viewer.spec.ts` 用例 3 建议补同款断言）。另请按 N1 更正 §T18-8/ADR-029 表述（可随手做，不计为缺陷）。
- **交接给协调者**：(a) `qa_result: FAIL`、`qa_round: 22`、`phase: rd_fixing`；(b) `qa_history` 追加 `{round: 22, scope: slice, task_ids: [T18], prd_revision: 2, result: FAIL}`；(c) `open_defects` 记 `BUG-007 (P2, OPEN)`；(d) T18 **不进入** `accepted_tasks`（AC-050 的手动重建子句未通过）；(e) 本回合新增 QA 测试文件两个（`apps/web/src/features/viewer/qa-t18-independent.test.ts`、`apps/web/tests/e2e/qa-t18-independent.spec.ts`），RD 修复后 QA 复验时保留；(f) 本回合**未改** prd.md / state.yaml / 规范文档，未改生产代码；改动仅 QA 测试文件、`artifacts/web-mvp/t18-qa/**`、`llmdoc/decisions.md`（追加验收知识）与本报告。
- **交接给 PM**：无强制项。可选裁定：手动重建后相机位姿是否必须保留（UI-044 现文案"能重建时"允许不保留，RD 现状为重新取景；如 PM 要求保留请派卡）。
- **全项目状态提醒**：T01–T17 已验收；T18 因 BUG-007 需修复后复验；T19–T23 未交付，**MVP 尚未完成**；完整发布仍需 T21 全量回归、T22 冷目录 smoke（多平台）、T23 授权后的真实 Provider 验证。
- **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「T18 验收知识（QA 回合 22）」——真实链路 e2e 手法、独立 WebGL 计数法与其坑（three 每渲染器 4 张 empty texture）、CDP 堆观测、`composeTransforms` 等比例显示契约、真实草稿形状（无 hotspots）、重建类降级路径的断言教训、BUG-007 根因摘要。`llmdoc/validation-release.md` 的命令合同**无变化**（未发现需修订的验收命令或阈值）。本报告即验收依据、缺陷与未覆盖边界的正式记录。
- **QA 产出与证据**：`artifacts/web-mvp/t18-qa/`——`t18-file-hashes.txt`/`t18-tree-hash.txt`、`typecheck.log`、`lint.log`、`web-test.log`、`build.log`、`viewer-e2e.log`、`qa-t18-independent.log`（首跑）、`qa-t18-independent-run{2,3,4}.log`、`qa-t18-independent-run5-t4.log`、`cargo-test-workspace.log`、`xtask-check.log`（首跑，含 QA 文件自身错误）、`xtask-check-final.log`、`contracts-check.log`、`dist.log`、`dist-sha256.txt`、`smoke-bootstrap.log`、`e2e-full.log`、`r22-real-drafts.json`、`r22-rebuild-state.json`、`r22-resource-trend.json`、`screenshots/r22-01…r22-05`（6 张）。测试代码：`apps/web/tests/e2e/qa-t18-independent.spec.ts`（6 用例）、`apps/web/src/features/viewer/qa-t18-independent.test.ts`（10 用例）。

# 回合 23 · T18（BUG-007 复验）

**结果：PASS（BUG-007 → CLOSED）** · 回合：23 · PRD 修订：2（ui_revision 2）· 范围：切片（task_ids=[T18]；AC-050、AC-051、AC-062/REQ-040 资源侧、AC-057 阅读侧、AC-060 前端侧；UI-043/044/045/059）

- QA 执行时间：2026-09-12 20:54–21:16（本地 UTC+8）；执行者：QA 子 agent。**不采信 RD 结论**：复验命令全部由 QA 重跑；RD 的 §T18-13 修复说明只作核对线索，代码与运行结果是唯一依据。
- 复验纪律：QA 既有测试文件（`qa-t18-independent.spec.ts`、`qa-t18-independent.test.ts`）**逐字节未被改动**（sha256 与回合 22 快照一致）；本回合**新增**独立用例 2 个文件（不改既有）；未改生产代码、prd.md、state.yaml 与规范文档。
- 派发核对：`state.yaml` current_tasks=[T18]、prd_revision=2、ui_revision=2、qa_round=23、open_defects=`BUG-007 (P2, fixed_pending_reverify)`；`prd.md` 修订 2 / ui_revision 2 与 §6.2 UI-044 原文一致（"成功：`restored` 后重建渲染并保留当前相机…（能重建时）；禁用：不把「刷新页面」作为唯一手段"）。
- 结论口径：本报告只覆盖 T18 切片复验；T19–T23 未实现，不作为本回合缺陷；**MVP 尚未完成**。

## 环境与交付版本

| 项 | 值 |
| --- | --- |
| 仓库／工作树 | `/Users/qsyj/Code/rust/everything-manual`；git HEAD `cb69afb44ee2a6088f440eade3f1424546fbb62d`（"init"）；T18 交付物未提交；`git status` 与派发前一致（本回合改动仅：QA 报告、`artifacts/web-mvp/t18-qa-r23/**`、2 个新增 QA 测试文件、`llmdoc/decisions.md` 追加验收知识） |
| 修复后源码快照 | `artifacts/web-mvp/t18-qa-r23/r23-file-hashes.txt`（`src/features/viewer/**`、`tests/e2e/**`、`App.tsx`、`api/*.ts`、`package.json` 逐文件 sha256）。与回合 22 快照的差异**仅 4 个文件**：`ViewerPanel.tsx`、`ViewerStage.tsx`、`tests/e2e/viewer.spec.ts`、`tests/e2e/viewer-harness.ts`——与 §T18-13 声明的改动面一致；`ReviewWorkspacePage.tsx` 等其余文件哈希不变。复验期间源码哈希未再变化（复跑前后一致） |
| 平台 | macOS Darwin 25.6.0 · aarch64-apple-darwin（Apple M1，8 核） |
| 工具链／浏览器 | rustc/cargo 1.98.1；node v26.0.0；npm 11.12.1；@playwright/test 1.60.0（bundled Chromium 1223，headless，真实浏览器） |
| 数据／Provider（fixture 与真实分开） | **fixture 验收**：① e2e `globalSetup` 后端；② 测试构建后端（`--features job-failpoints`）+ 本机 fixture 供应商（真实流水线产真实草稿/资产）。**真实 Provider 验收：未做**（无授权，属 T23）。**零真实外网**：QA 用例断言非本机请求 0 条（`external == []`） |
| 交付二进制（dist 复核） | `dist/aarch64-apple-darwin/everything-manual` sha256 **`200b627e3f45316d752682f72782fe691b5d443a26398d69c35076be946f569a`**（23 618 464 B）——QA 独立重建，与 RD §T18-13 #13 记录**逐字节一致**，说明复验对象就是 RD 交付的源码状态 |
| 未验证（非本回合范围） | AC-061 p95 帧耗时（T21 具名设备 + 人工，本回合未复跑 perf 脚本）、真实 Provider/费用（T23）、Safari、真实 Chrome channel、经典（占宽）滚动条环境（见下）、T19–T23 功能 |

## BUG-007 复验（结论：CLOSED）

四条独立证据（全部由 QA 现场执行；命令见文末表）：

1. **一键复现命令**（回合 22 立案时的失败命令）`npm --prefix apps/web run test:e2e -- qa-t18-independent.spec.ts -g "QA-T18-3"` → **exit 0，1 passed（10.9 s）**；用例自落的现场 JSON 恢复为可用态：`statusText="模型已加载，可旋转/缩放。"`、`resetButtonDisabled=false`、`rebuildButtonVisible=false`、`modelsAlive=1`、独立 draw 27→45。证据：`r23-qa-t18-3.log`、`r23-qa-t18-3-rebuild-state.json`、`r23-qa-t18-3-{restored,after-rebuild}.png`。
2. **QA 新增独立语义用例**（真实链路 + 原型层独立 draw 计数；`qa-t18-r23-manual-rebuild.spec.ts`）→ **exit 0，1 passed（11.5 s）**：手动「立即重建」后 ① 状态回「模型已加载，可旋转/缩放。」；②「复位视角」「适配模型」enabled；③ **8.6 秒后仍为可用态**（无「不可用」文案、重建入口仍不出现、控件仍可用）；④ 重建后拖动仍能旋转（位姿变化 >0.01，独立 draw 计数 19→555 持续增长）；⑤ 重建后位姿 ≈ 重建前位姿（实测差 ~1e-15，断言阈值 1e-3）；⑥「复位视角」回到默认取景（实测 == 加载后默认取景 `[0, ~0, 6.42]`，<1e-3）。证据：`r23-manual-rebuild.log`、`r23-manual-rebuild-state.json`、`screenshots/r23-manual-rebuild-8s-after.png`。
3. **自动恢复分支不回归**（回合 22 判 PASS 的分支）：QA-T18-3 的 restored 段（真实绘制恢复 + 「3D 显示已恢复。」+ 双按钮 enabled + 位姿保留 1e-4）在 QA 重跑中通过；RD 用例 3 的自动分支（位姿 1e-5、状态文案）亦通过。
4. **RD 自己的用例 3 扩展断言**（状态文案/双按钮/无重建入口/位姿 1e-3/8.5 s 后稳定/「适配模型」+「复位视角」回初始取景）与**新增用例 9**（真实链路、零拦截）在 QA 重跑 `viewer.spec.ts` 中 **9 passed**。

**修复复核（代码级，与 §T18-13 声明一致）**：`ViewerStage` 上下文监听 effect 挂载时上报一次 `ok`；`ViewerPanel` 收到 `ok` 时清除 `unavailableTimer` 并清 `contextUnusable`；`rebuild()` 为显式状态迁移（捕获 `stageApi.pose()` → `clearTimers()` → 清不可用/提示 → `setStageReady(false)` → `contextState="restoring"` → 重启 8 s 兜底 → `stageApi.rebuild()`）；`restoring` 与 `lost` 同文案/同禁用且不显示重建入口；`restorePoseRef` + `applyPose()`（读写同一对变换 `readCameraPose`/`applyCameraPose`，`up` 归一化）在模型就绪 fit **之后**消费一次；`reset()` 仍回默认取景。回合 22 立案的两个根因（新挂载不上报 `ok`、8 s timer 未清除）均已被覆盖并有回归断言。

## N1 处理核对

| 项 | 核对结果 |
| --- | --- |
| `implementation.md` §T18-8 订正 | **已订正**（标注"原'必须拦截'表述与实测不符"）：列明 `GET /assets/{id}/content`（T06）与 `GET /items/{id}/drafts/{draftId}`（T15）均可用、无测试构建门控；真实链路为首选，拦截仅留给真实链路无法构造的注入场景（stale 热点、500 注入） |
| `implementation.md` §T18-10 第 1 条 | **已同步订正**（草稿/资产端点可用；只有"造草稿"需测试构建 + fixture 供应商；release 读取端点属 T19，非本卡前置） |
| `decisions.md` ADR-029 第 9 条 | **已订正**（注明订正日期与"QA 回合 22 实测推翻原表述"）；第 5 条补充了 BUG-007 的状态机订正与断言教训 |
| 新增真实链路用例（`viewer.spec.ts` 用例 9） | **确实不拦截草稿/模型字节**：`installRealBackendRouting`（`viewer-harness.ts:204-232`）只做源站改写（Vite 端口 → 测试后端）+ 非本机阻断，**函数体内没有任何 `route.fulfill`**；用例 9 仅调用该函数。数据断言：模型 `sha256` == 草稿 sha256 == fixture GLB 真实哈希、12 三角面、真实部件/步骤文案、原文 canvas 有像素、`assetRequests` 命中真实资产端点、`external == []`、`fixture.counts.cdn > 0` |
| 局限（非阻断，建议） | `routing.fulfilled` 计数器**从未自增**（该函数中不存在调用点）→ `expect(routing.fulfilled).toBe(0)` 是"守卫"而非证明（结构上不可能失败）。真实证据在函数体本身（无 fulfill 路径）+ `assetRequests` + sha256。建议后续改为路由内真实计数或在注释标明其守卫性质 |
| QA 真实链路对照用例仍成立 | **成立**：`qa-t18-independent.spec.ts` 6/6 通过（真实草稿 + 真实资产字节 + QA 自有路由与计数器，未被 RD 改动影响） |
| T19 门禁要求登记 | 已在"对 T19 的验收要求"登记（见下）。llmdoc 中**未发现**显式的"release 阅读器必须有不拦截字节的真实链路用例"条款，建议协调者转入 T19 派发说明 |

## 1280px 断点边界"整页重挂载"调查（RD §T18-13 非阻断发现 1）

**判定：测试脆弱（Playwright fullPage 截图诱发的瞬态），不是产品问题；RD 的机理描述需订正。不记 BUG。**

- **对照实验（同视口 1280×720、同用户交互、但不截图）**：`qa-t18-r23-layout-boundary.spec.ts` 的 QA-R23-LB2（空闲 2 s + 拖动旋转 + 复位/适配按钮）→ **0 断点瞬态、0 重挂载**（3/3 通过）。
- **复现实验（含一次 fullPage 截图）**：瞬态**只出现在截图窗口内**（截图窗口外为 0，LB 用例断言通过）；多路证据（`matchMedia` change、1279/1269 探针查询、5 ms 轮询、rAF 采样、ResizeObserver、window resize）一致显示截图期间模拟视口被瞬时改为 **1×1**（`iw=cw=visualViewport=1`，低于全部断点 → narrow），恢复后再变回；面板状态节点身份跟踪显示阅读器面板在瞬态期间与恢复后**各重挂载一次**（整页换树、模型重载、相机复位）。
- **1440×900 同样复现**（QA-R23-LB3：截图窗口内 1×1、面板两次重挂载）→ 证明该现象**与 1280 断点无关**，"1280−15 < 1280"的机理不成立；RD 的"1440 固定视口 + `finiteDistance()`"组合仍能保持用例稳定，但有效成分是**等待位姿可读**（截图会重挂载整页），不是"不跨断点"。
- **产品侧结论**：真实用户在稳定宽度下不会遇到该瞬态（应用自身内容变化不改变视口宽度）；跨断点 resize 时整页换树是响应式设计的既有取舍（会重载模型、丢相机位姿）——建议 T21 评估"布局换树时保住阅读器状态"的韧性改进，**不作为 T18 缺陷**。
- **测试侧建议**：e2e 中避免在测量窗口附近做 `fullPage` 截图；截图后若需继续测量，先等面板回到可用态再取数（本回合新增用例已按此实现）。`scrollbar-gutter` 一类方案**不能**解决该问题（机制是模拟视口 1×1，与滚动条无关）。
- **环境限制**：本机 Playwright 滚动条为 overlay（`::‑webkit‑scrollbar { width: 15px }` 不占宽，`clientWidth` 仍 1280；`r23-layout-scrollbar.json`），"经典占宽滚动条出现/消失是否影响断点"**无法在本环境实测**，交 T21 浏览器矩阵。

## AC 验收矩阵（复验子集；T18 其余 AC 沿用回合 22 结论）

| AC / UI | 期望（可观察） | 实际（QA 实测） | 结论 | 证据 |
| --- | --- | --- | --- | --- |
| AC-050 · context lost → restored（自动分支） | restored 后重建渲染、位姿保留、控件恢复 | 真实链路：独立 draw 计数继续增长、状态「3D 显示已恢复。」、双按钮 enabled、位姿保留 1e-4；RD 用例 3 同向（1e-5） | PASS（不回归） | QA-T18-3 重跑；`r23-qa-t18-3.log`；`r23-qa18-plus-viewer.log` |
| AC-050 · 手动「立即重建」（BUG-007 现场） | 重建后回到可用态、控件可用、不误报不可用、重建入口消失 | 状态「模型已加载，可旋转/缩放。」；双按钮 enabled；**8.6 s 后仍可用**；无重建入口；独立 draw 19→555；位姿保留（~1e-15） | **PASS（BUG-007 CLOSED）** | QA-R23-1；QA-T18-3；RD 用例 3 |
| UI-043/UI-044 · 键盘等价控件与重建语义 | 重建成功后键盘路径可用；重建中禁用交互、不以刷新为唯一手段 | 双按钮 enabled 且「复位视角」实测回默认取景；重建期间 `restoring` 同 `lost` 语义（源码复核 + 状态行文案） | PASS | QA-R23-1；源码 |
| AC-051 / AC-062 / AC-057 / AC-060（T18 子集） | 同回合 22 | QA-T18-1…6 重跑全绿；RD 用例 1/4/5/6/7/8/9 全绿 | PASS（不回归） | `r23-qa18-plus-viewer.log`；`r23-e2e-full.log` |
| 卡内 · 位姿策略与实现一致 | 重建后套用重建前位姿、reset 回默认取景 | 实测：重建后 ≈ 重建前（1e-15 级）；reset → 默认取景（<1e-3） | PASS | QA-R23-1 |

## 命令与原始结果（QA 现场执行；日志 `artifacts/web-mvp/t18-qa-r23/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `npm --prefix apps/web run test:e2e -- qa-t18-independent.spec.ts -g "QA-T18-3"`（BUG-007 一键复现） | **exit 0，1 passed（10.9 s）**（`r23-qa-t18-3.log`） |
| 2 | `npm --prefix apps/web run test:e2e -- qa-t18-r23-manual-rebuild.spec.ts`（QA 新增独立语义用例） | **exit 0，1 passed（17.3 s 含 beforeAll）**（`r23-manual-rebuild.log`） |
| 3 | `npm --prefix apps/web run test:e2e -- qa-t18-independent.spec.ts viewer.spec.ts`（QA 6 + RD 9） | **exit 0，15 passed（1.2 m）**（`r23-qa18-plus-viewer.log`） |
| 4 | `npm --prefix apps/web run test:e2e -- qa-t18-r23-layout-boundary.spec.ts --repeat-each=3` | **exit 0，6 passed**（`r23-layout-boundary-run6.log`；另 `-run7` 3 passed） |
| 5 | `npm --prefix apps/web run test:e2e`（**全量**：71 = 60 既有 + QA22 6 + RD 用例 9 + 本回合 4） | **exit 0，71 passed / 0 failed / 0 skipped（5.7 m）**；分文件：import-flow 6 / job-recovery 8 / library-row-layout 2 / pdf-preparation 10 / qa-t09 6 / qa-t16 12 / qa-t17 8 / qa-t18-independent 6 / qa-t18-r23-layout-boundary 3 / qa-t18-r23-manual-rebuild 1 / viewer 9（`r23-e2e-full.log`） |
| 6 | `npm --prefix apps/web run typecheck` / `lint` / `test -- --run` / `build` | 全部 exit 0；vitest **15 files / 121 passed**；build 入口 408.83 kB(gzip 123.86) + ViewerStage 967.43 kB + prepare(pdfjs) 432.92 kB（`r23-typecheck.log`、`r23-lint.log`、`r23-web-test.log`、`r23-build.log`） |
| 7 | `cargo test --workspace` | exit 0，**477 passed / 0 failed**（31 个测试目标；`r23-cargo-test-workspace.log`） |
| 8 | `cargo xtask check` | exit 0，**7/7 `[通过]`**（fmt / clippy -D warnings / workspace 测试 / 前端 lint / typecheck / vitest / contracts --check；`r23-xtask-check.log`） |
| 9 | `cargo xtask contracts --check` | exit 0，`[一致] contracts/openapi.json`、`[一致] apps/web/src/api/generated.ts`，未改工作树（`r23-contracts-check.log`） |
| 10 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0，sha256 **`200b627e3f45316d752682f72782fe691b5d443a26398d69c35076be946f569a`**（23 618 464 B）——与 RD §T18-13 #13 一致（`r23-dist.log`、`r23-dist-sha256.txt`） |
| 11 | `cargo xtask smoke-bootstrap --binary <dist 绝对路径>` | exit 0，1 项 `[准备]` + **7 项 `[检查]`** 全过（内嵌页面/静态资源/health/未知 API JSON 404/SPA 路由刷新；`r23-smoke-bootstrap.log`） |
| 12 | 离线与不伪造核对 | QA 用例断言 `external == []`（零真实外网，含非本机阻断并记录）；真实链路用例 `route.fulfill` 计数为 0（守卫）且 `assetRequests` 命中真实端点（`r23-qa18-plus-viewer.log`、`r23-manual-rebuild-state.json` 的 `externalRequests: []`、`routingFulfilledOk: 0`） |
| 13 | 清理核对 | 全部测试进程已退出；`/var/folders/.../em-t17-qa-t18-*`、`em-web-mvp-e2e` 等临时 data-dir 已清理；`git status` 与派发前一致，无预期外改动 |

## 缺陷

- **BUG-007（P2）→ CLOSED**：手动「立即重建」后状态与控件不恢复（UI-044/AC-050）。RD 修复见 `implementation.md` §T18-13；QA 独立复验四条证据通过（见上）。修复前失败现场保留于 `artifacts/web-mvp/t18-rd-fix/repro-before-*`（QA 现场文件会被用例自身覆盖，RD 已另存）。
- 本回合**未发现新的 P1/P2 缺陷**。

## 非阻断观察（新增；回合 22 的 N1 已由 RD 处理并复验，N2/N3/N4 沿用）

1. **N5（真实链路"零伪造"计数的强度）**：`RealBackendRouting.fulfilled` 从未自增 → 断言恒真（守卫性质）。建议改为路由内真实计数或注释标明，避免被后续角色误当强证据。
2. **N6（测试基础设施风险：fullPage 截图 → 整页重挂载）**：机制与证据见上节。对**既有**含"截图后紧接测量"用法的用例（含 QA 回合 22 的 `qa-t18-independent.spec.ts` 与 RD 的 `captureTo` 用法）存在偶发脆弱风险。建议 T19 起新用例遵循"测量窗口不做 fullPage 截图 / 截图后先等恢复"，并考虑给 `captureTo` 增加恢复等待（属测试代码改进，非产品缺陷）。

## 对 T19 的验收要求（QA 登记；建议协调者写入 T19 派发说明）

1. **真实链路门禁**：release 阅读器至少一条**不拦截草稿/模型/release 字节**的真实链路用例（可复用"测试构建 + 本机 fixture 供应商 + 源站改写"设施；样例 `qa-t18-independent.spec.ts` / `viewer.spec.ts` 用例 9）；合成 DTO 用例不得作为该子句的唯一证据。
2. **热点 stale 子句真实化**：T19 落库热点后，"换模型不串旧热点"必须有真实链路覆盖（替换/补充现有合成 DTO 用例，对应沿用中的 N2）。
3. **重建状态机复用**：若 release 阅读器复用 `ViewerPanel`/`ViewerStage`，沿用本回合的断言集合（状态文案 + 双按钮可用 + **≥8.6 秒后仍可用** + 重建入口消失 + 位姿策略），防止 BUG-007 类回归。
4. （可选）视角保存与跨断点重挂载的交互：若 T19 实现"视角保存"，建议顺带评估布局换树（跨 1280 断点 resize）时位姿是否可保留。

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实 Provider（Tripo / 说明书 AI）与真实费用**：全部走本机 fixture（零外网零付费）；须 T23 授权后另验，与 fixture 结果分开记录。
2. **AC-061 帧耗时**（100k 面、p95 ≤33 ms）：属 T21 具名设备 + 人工复核；本回合未复跑 perf 脚本。
3. **经典（占宽）滚动条环境**：本机 Playwright 为 overlay 滚动条，无法模拟"滚动条占宽"；断点与滚动条的交互只做了机理分析（对照组 0 瞬态），未在占宽环境实测。
4. **`restoring` 中间态的逐帧观察**（重建期间保持禁用与文案）：以源码复核 + 状态机语义为准；该窗口极短（本回合未观测到稳定的中间态采样点），未做逐帧断言。
5. **多材质/多贴图/骨骼/大贴图模型、Firefox/Edge/Safari、窄屏性能**：未测（浏览器矩阵属 T21）。
6. **T19–T23 功能**（热点校准、发布、导出备份、多平台单包、真实授权链路）：未实现/未验收，**MVP 尚未完成**。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 1–21 | 2026-09-12 | slice | T01–T17、（BUG-005/006） | 1/2 | PASS（回合 10、12、18 各 FAIL → 修复后 PASS） | 本文件上半部分 |
| 22 | 2026-09-12 | slice | T18 | 2（ui_revision 2） | FAIL（BUG-007，P2；非阻断观察 N1） | 第 22 节 |
| 23 | 2026-09-12 | slice | T18（BUG-007 复验） | 2（ui_revision 2） | **PASS**（BUG-007 CLOSED；新增 N5/N6 观察与 4 条 T19 门禁要求） | 本节 |

- **交接给协调者**：(a) `qa_result: PASS`、`qa_round: 23`、`phase: qa_passed`（可进入下一卡/T19 派发）；(b) `qa_history` 追加 `{round: 23, scope: slice, task_ids: [T18], prd_revision: 2, result: PASS}`；(c) `open_defects` 中 BUG-007 置 **CLOSED**（复验证据见本节）；(d) T18 进入 `accepted_tasks`（accepted_ac_ids：AC-050 全子句、AC-051、AC-062/REQ-040 资源侧、AC-057 阅读侧、AC-060 前端侧；UI-043/044/045/059）；(e) 将"对 T19 的验收要求"（本节 4 条）纳入 T19 派发说明；(f) 本回合新增 QA 测试文件 2 个（`apps/web/tests/e2e/qa-t18-r23-manual-rebuild.spec.ts`、`apps/web/tests/e2e/qa-t18-r23-layout-boundary.spec.ts`）保留；(g) 本回合未改 prd.md / state.yaml / 规范文档，未改生产代码与既有 QA 测试文件。
- **交接给 RD**：无必修项。可选（测试侧，非阻断）：按 N5 把 `fulfilled` 改成真实计数或在注释标明守卫性质；按 N6 给 `captureTo` 增加"截图后等恢复"等待（涉及既有用例时随下次触碰该文件的卡处理）。
- **交接给 PM**：无强制项。可选：跨断点 resize 的整页换树（丢相机位姿）是否要求韧性改进（现按 UI-044"能重建时"不要求；建议留 T21/后续卡评估）。
- **全项目状态提醒**：T01–T18 已验收（T18 本回合通过）；T19–T23 未交付，**MVP 尚未完成**；完整发布仍需 T21 全量回归、T22 冷目录 smoke（多平台）、T23 授权后的真实 Provider 验证。
- **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「T18 验收知识（QA 回合 23）」——手动重建完整语义清单、**Playwright fullPage 截图的 1×1 瞬态与整页重挂载**、NaN 断言陷阱、真实链路"零伪造"证据分层、占宽滚动条环境限制。`llmdoc/validation-release.md` 的命令合同**无变化**（未发现需修订的验收命令或阈值）。本报告即验收依据、缺陷与未覆盖边界的正式记录。
- **QA 产出与证据**：`artifacts/web-mvp/t18-qa-r23/`——`r23-qa-t18-3.log`、`r23-qa-t18-3-rebuild-state.json`、`r23-qa-t18-3-{restored,after-rebuild}.png`、`r23-manual-rebuild.log`、`r23-manual-rebuild-state.json`、`screenshots/r23-manual-rebuild-8s-after.png`、`r23-qa18-plus-viewer.log`、`r23-layout-boundary{,-run2…-run7}.log`、`r23-layout-boundary.json`、`r23-layout-control.json`、`r23-layout-1440.json`、`r23-layout-scrollbar.json`（占宽滚动条模拟不受支持，见环境限制）、`r23-e2e-full.log`、`r23-typecheck.log`、`r23-lint.log`、`r23-web-test.log`、`r23-build.log`、`r23-cargo-test-workspace.log`、`r23-xtask-check.log`、`r23-contracts-check.log`、`r23-dist.log`、`r23-dist-sha256.txt`、`r23-smoke-bootstrap.log`、`r23-file-hashes.txt`。测试代码：`apps/web/tests/e2e/qa-t18-r23-manual-rebuild.spec.ts`（1 用例）、`apps/web/tests/e2e/qa-t18-r23-layout-boundary.spec.ts`（3 用例）。

# 回合 24 · T19（热点校准、步骤联动与发布）

结果：**PASS**（仅本切片 T19；全项目尚未完成）· 回合：24 · PRD 修订：2（ui_revision 2）· 范围：切片

## 环境与交付版本

- 工作树：`3138496`（init）+ 未提交工作树（RD T19 收尾 + 本回合 QA 新增测试/证据；QA 未改生产代码）。关键源文件 sha256 清单：`artifacts/web-mvp/t19-qa/qa-file-hashes.txt`。
- 工具链：rustc/cargo 1.98.1（2026-09-01）、Node v26.0.0、Playwright 1.60.0、Chromium **148.0.7778.96**（Chrome for Testing，`chromium-1223`）；macOS Darwin 25.6.0 arm64。
- 后端形态：**测试构建**（`cargo build -p everything-manual --features job-failpoints`）+ 显式测试配置（provider `base_url` 指本机 fixture、`allowed_hosts=["127.0.0.1"]`、`allow_local_fixture=true`、`public_origin=http://127.0.0.1:<页面端口>`）。Tripo v3 / 说明书 AI / 模型 CDN 全部在 `127.0.0.1` 随机端口。
- **零真实外网、零付费（QA 自己观测）**：浏览器侧路由只做源站改写 + 非本机请求 abort（计数）；QA 7 个用例与 RD 8 个用例均断言 `route.fulfill` 正常响应计数 = 0、`external == []`；Rust 侧全部 HTTP 指向本机 fixture（T05 设施）。
- dist 产物：`cargo xtask dist --target aarch64-apple-darwin` exit 0，sha256 `c9a0e7fc691fd2aa918dcf3d220e743aae0c7c868c9333997c0cddc58579da6f`（24 441 344 B；与 RD 终版记录一致 → 冻结树上可复现）；`smoke-bootstrap` exit 0（1×[准备] + 7×[检查]）。

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/t19-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test publishing` | exit 0，**7 passed / 0 failed**（`qa-publishing-1.log`）；另 5 次连跑各 7 passed（`qa-publishing-stab-1..5.log`）|
| 2 | `npm --prefix apps/web run test:e2e -- qa-t19-independent.spec.ts`（**QA 自证 7 用例**，最终修订）| exit 0，**7 passed / 0 failed**（`qa-t19-spec-final3.log`）|
| 3 | `npm --prefix apps/web run typecheck`；`lint` | 均 exit 0（`qa-typecheck.log`、`qa-lint.log`）|
| 4 | `npm --prefix apps/web run test -- --run` | exit 0，**130 passed（16 files）**（`qa-vitest.log`）|
| 5 | `cargo test --workspace` | exit 0，**489 passed / 0 failed / 3 ignored（32 targets）**（`qa-cargo-test-workspace.log`）|
| 6 | `cargo xtask check` | exit 0，**7/7 [通过]**（`qa-xtask-check.log`）|
| 7 | `cargo xtask contracts --check` | exit 0，两份 `[一致]`（同日志）|
| 8 | `cargo xtask dist` + `cargo xtask smoke-bootstrap` | 均 exit 0（`qa-dist-smoke.log`）|
| 9 | `npm --prefix apps/web run test:e2e`（**全量**，冻结树）| exit 0，**86 passed / 0 failed（7.0 m）**（`qa-e2e-full-final.log`；含 RD T19-1..8 = 第 17–24 条、QA19-1..7 = 第 71–77 条、`viewer.spec.ts` 真实链路）|

（过程说明：首次全量 e2e（`qa-e2e-full.log`，85 passed）执行时 QA 用例尚为 6 用例的中间修订；其后 QA 用例又修正 2 处并新增 QA19-7，最终以第 9 行的 86/86 为准。）

## AC 验收矩阵（逐子句；QA 自证用例 = `apps/web/tests/e2e/qa-t19-independent.spec.ts`）

| AC | 子句 → 结论 | QA 独立证据 | 同源复核 |
| --- | --- | --- | --- |
| AC-052 | 保存 revision+sha+positionLocal 且有限 → PASS；人工直接拾取 → confirmed → PASS；unbound anchor=null（非 [0,0,0]）→ PASS；拖动不误触 → PASS；raycast 只含模型 mesh → PASS；步骤视角（REQ-033 的 CameraPose）可保存/复现 → PASS | QA19-1（落库 anchor 与拾取点逐轴 <1e-9、全 `Number.isFinite`；拖动 130px 后热点数 0 且相机确实变化；**长按 750ms 不建点**；放大到标记屏幕半径 45–77px 后在标记覆盖内、18px 选择半径外点击 → 拾取点仍落在包围盒内且贴面——若射线命中标记球体会外向越界 ~0.03）；QA19-5（`unbound`+`[0,0,0]` → 422“占位”；unbound→candidate→confirmed 三段链）；QA19-6（视角：草稿 `stepPoses` 与保存瞬间位姿逐轴 <1e-6、「回到该视角」误差 <1e-3、清除生效、fov∈[1,179]）| RD 用例 1 复跑；源码：`ViewerStage` 阈值 6px/600ms、`raycast={() => null}`、`intersectObject(root.object,true)`、`has_finite_values` |
| AC-053 | 重新生成 → 旧绑定 stale 且不作有效热点 → PASS；API 拒绝旧 sha 的 confirmed → PASS；旧发布版指向旧模型且可读 → PASS | QA19-2（**真实两轮流水线**：第一轮模型 sha == 现场 fixture 字节；切换 CDN 字节后同物品新任务 → 新 revision/sha；旧热点继承为 `stale`（旧 anchor 保留）；旧 sha 提交 → 422“旧模型版本”；发布 → 422 含 `hotspotMissing`（stale 不得当有效热点用）；浏览器：stale 区块 + `part-hotspot-*` “（1 个已失效）+”已失效热点：2” + 桥 `anchors()`=0；UI「在新模型上重新绑定」→ confirmed + 新 sha；旧 release 的 manifestSha256/字节与模型资产字节（== 旧 sha）逐字节不变）| RD `publishing.rs` 用例 3 复跑；`crates/server/src/drafts/aggregate.rs::normalize_stale_hotspots` |
| AC-054 | 实体级 confirmed/needs_review → PASS；userEdited 保留出处 → PASS；Evidence 1-based + bbox=null 仍可跳页 → PASS；部件引用存在 → PASS（见未覆盖边界 2）；modelReview 用户声明、服务器赋值 checkedAt → PASS；换模型清空 → PASS；快照只读 → PASS | QA19-5（confirmed↔needs_review 切换；`userEdited` → `editedAt/editedBy` 服务器赋值且快照 `name` 不变；`knowledgeJson` 直改 422；`checkedAt` 客户端写 422；`userConfirmed` 无 loaded 422；`loaded` 单真 → `loadedAt/checkedAt` 服务器赋值 + 模型身份；双真 → `userConfirmedAt`）；QA19-2（换模型 `modelReview` 清空）；QA19-4（UI 两个声明按钮门槛 + “事实确认/几何校准”文案区分）；QA19-1/7（bbox=null 的出处按钮跳到 1-based 页）| RD 用例 2 复跑；`aggregate.rs::validate_model_review` |
| AC-055 | 201 不可变 release；manifest 记录 draftRevision/modelRevisionId/资产 sha256 → PASS；幂等重放 → PASS；发布后改 draft 不改 release 字节与哈希 → PASS | QA19-3（201 + `draftRevisionAfterPublish`=r+1；同键重放 201 + `x-idempotent-replay:true` + 同一 release id；**发布后 PATCH 草稿 → `manifestSha256` 不变、manifest 资产字节逐字节相等、releases 仍为 1**）；QA19-2（manifest.model.sha256/assetId 与真实资产字节哈希一致）| RD 用例 5 复跑 |
| AC-056 | 未确认知识/缺热点/stale/modelReview 未完成 → 422 明细 → PASS；并发 412 → PASS；不自动隐藏/不伪造确认 → PASS；仅双真且匹配才可发布 → PASS；无自动发布 → PASS | QA19-3（缺 If-Match 428；缺 Idempotency-Key 422；空草稿 → 422 `issues[]` 含 `knowledgeUnreviewed/hotspotMissing/modelReviewMissing`；实体+热点齐备但 loaded 单真 → 422 `modelReviewIncomplete`；过期 If-Match → 412 + `currentRevision`；同键不同 body 409；被拒发布不产生 release）；QA19-2（stale → 422）；QA19-4（UI 冲突面板“当前 r”+ 刷新恢复 + **窄屏发布成功**）；QA19-5（可发布草稿未显式发布 → releases 空）| RD 用例 4/6 复跑；`releases/invariants.rs` |
| AC-057 | 四方联动 → PASS；步骤前进/后退/跳步不累积错误 → PASS；引用跳 1-based 且与页图/文字一致 → PASS；部件列表为文字替代 → PASS；非 3D 操作键盘可达 + 可见 focus → PASS | QA19-6（**多步** 2 步：前进/后退/跳步 + `aria-current` 一致 + 无错误残留文案）；QA19-7（阅读器：部件点击 → 定位热点 + `reader-notice`、步骤导航、1-based 原文、发布版无编辑/发布入口、桥 anchors 与热点一致）；QA19-1（点热点 → 部件行 `aria-current`）；QA19-4（键盘模态 focus 环 + Enter 选中；无裸“确认”按钮）| RD 用例 2/3/6/8 复跑；`viewer.spec.ts` 真实链路用例在全量回归中 |
| 卡内边界 | 窄屏 <768px 禁用几何校准/视角保存 + 解释 + 保留只读/文字确认/发布 + 拉宽恢复 → PASS；「事实确认」与「几何校准」区分 → PASS；§6.3.2 禁用措辞 → PASS；无自动发布路径 → PASS | QA19-4（拾取开关/绑定/保存视角 disabled + “≥768px”说明；文字确认可用；发布缺项禁用并写明原因；**不变量补齐后窄屏发布 201 成功**；拉宽即恢复；禁用措辞否定语境扫描 0 正向命中；无自动发布角色入口）；QA19-7（阅读器同扫描）；QA19-1/7（页面加载 `publish` 请求 = 0）| RD 用例 7 复跑；`crate::releases` 为 `manual_releases` 唯一写入者（源码复核） |

## 三项 T19 门禁核对（QA 回合 23 登记）

1. **release 真实链路用例门禁（不拦截草稿/模型字节）→ 满足**：QA 自证 QA19-2/3/7 全程真实链路（测试构建 + 本机 fixture 供应商 + 源站改写；`route.fulfill` 计数恒 0、`external` 空；模型/PDF 字节经真实 `GET /assets/{id}/content`，manifest 字节现场 sha256 比对）；RD 用例 6 复跑通过。合成 DTO 未作为任何子句的证据。
2. **热点 stale 真实链路覆盖（非合成 DTO）→ 满足**：QA19-2 用**自建可切换字节 CDN** 在同一物品跑两轮真实流水线（新模型经真实下载 + T13 校验 + 新 revision 落库）；RD `publishing.rs` 用例 3 复跑 + 5 次稳定性连跑全绿。
3. **重建状态机断言集合纳入回归 → 满足**：`manual-review.spec.ts` T19-8（在校准页复跑 BUG-007 断言集：状态文案/交互禁用/立即重建/恢复后 8.5s 仍正常）在两次全量 e2e 中均通过；`viewer.spec.ts` 用例 3 在同一全量回归中。

## cameraPose 裁定（QA 独立判定）

- **事实**：`contracts.md` §2 的 Hotspot 行列出 `cameraPose=null`；实现中 `Hotspot`（`drafts/aggregate.rs`）只有 `id/partId/status/anchor`，**没有** `cameraPose` 字段；视角按 REQ-033/UI-050 落在**步骤级** `knowledge.stepPoses`（发布 manifest 冻结）。PATCH 的 `HotspotUpsert` 为 `deny_unknown_fields`——客户端今天发 `hotspot.cameraPose` 会被 422 拒绝（不会静默吞掉）。
- **判定 1（是否偏离合同）**：字面上偏离 contracts §2 的字段清单（缺 `cameraPose` 键，而非以 `null` 占位）。
- **判定 2（是否影响必选 AC）**：**不影响**。AC-052–AC-057 无一条以热点级 cameraPose 为可观察项；REQ-033 的“可保存步骤视角”与 UI-050 均由步骤级实现交付，且已由 QA19-6 在真实浏览器中复现（保存/回到/清除 + 草稿持久化 + 服务端数值校验）。
- **判定 3（结论）**：**非阻断**。建议协调者/PM 做**合同留痕**（在 contracts §2 注明 MVP 视角为步骤级 `knowledge.stepPoses`、Hotspot 不含 cameraPose；若未来要求热点级字段，属 PM 新修订 + 新 AC）。QA 未修改规范文档。

## 缺陷

本回合 **0 个 P0/P1/P2 缺陷**，未新增 BUG 编号（无开放验收缺陷）。

## 非阻断观察（P3；供 RD/PM/T20/T21）

1. **N1 · modelReview 面板把 Markdown 粗体标记原样渲染**：`KnowledgeReviewPanel` 文案 `这是**你的复核声明**，不是服务端 GPU 测试结论…` 在界面显示字面 `**`。DOM 实测 `modelReviewText` 含 `**`（`qa19-4-narrow.json` 的 `modelReviewDeclaresLiteralAsterisks: true`）。不违反必选 AC 的可观察项（声明语义与说明齐备），建议下次触碰该文件时去掉星号或改为 `<strong>`。
2. **N2 · 发布 412 的两处 UI-056 偏差**：并发发布 412 后 ① 发布按钮**未按 UI-056「刷新前禁用发布按钮」禁用**（实测 `buttonDisabledDuringConflict:false`）；② 点「刷新草稿」后草稿确实按新 revision 重取、按钮可用、再次发布 201 成功（可恢复路径正常），但**冲突面板不消失**（实测 `conflictPanelStillVisible:true`，本地 failure 状态未随 refetch 清除）。窄屏发布成功证明不影响主流程；建议 RD 在 PublishPanel 里按 refetch 清除 failure 并在冲突未解决时禁用按钮（与 UI-008 的共用组件行为对齐）。
3. **N3 · 校准页步骤面板「引用部件」显示内部 id**：`引用部件：part-2e063fda1795`（`qa19-2-stale-block.png`）；同一数据在发布版阅读器显示部件名称（`ReleaseReaderPage` 用 `parts.find(...)?.name`，见 `08-release-reader.png`）。建议对齐文案（不阻断发布）。
4. **N4（源码复核，未做浏览器实测）· 修订表单的字段级错误关联**：`EditForm` 无本地校验；服务端 422（如超长）经 `describeFieldIssues` 落到页面级提示，未做 `aria-describedby` 字段级关联（UI-065 的共享实现未覆盖该新表单）。属 a11y 细化项。
5. **N5（沿用）· 证据目录共享互相覆盖**：本轮全量 e2e 重写 `artifacts/web-mvp/t19-rd/screenshots/**`、`t16-*/t17-*` 截图与 `t09-rd/` 日志（截图内容为同一 spec 重新生成，非丢失）。建议后续卡用独立子目录。

## 非代码知识与限制（提炼；跨卡复用条目已写入 `llmdoc/decisions.md`）

- 可切换字节的模型 CDN 是在浏览器链路复现“重新生成 → stale”的最小设施；T17 `LocalFixture` 不支持。
- 画布点击前必须把 canvas 滚进视口（断言/点击会滚动页面）；窄屏抽屉覆盖层会拦截主栏点击（先 Esc）；`:focus-visible` 焦点环需键盘模态（`Shift+Tab`+`Tab`）才可断言。
- `modelReview` 是整体状态提交（两字段必填）；`userConfirmed:true + loaded:false` 在无既有 loaded 时 422。
- stale 热点不产生 `hotspotNotMatchingModel`，它是经 `hotspotMissing` 阻断发布；“冒充 confirmed/candidate”需篡改注入（RD 用例 6）。
- raycast 排除热点标记的可判定做法：标记屏幕半径 >45px 后在 18px 选择半径外点击，断言拾取点仍在包围盒内且贴面。

## 未覆盖边界（不冒充通过）

1. **真实 Provider（Tripo/说明书 AI）与真实费用**：全部本机 fixture（零外网零付费）；T23 授权后另验。
2. `hotspotNotMatchingModel`（confirmed/candidate 冒充）与 `stepPartReferenceMissing` 的负例需要篡改 DB 注入：由 RD `publishing.rs` 用例 6 覆盖（我复跑通过），QA 未独立注入。
3. 真并发（两个 publish 请求同时到达）：QA 复现的是“过期 If-Match → 412”与“同键重放 / 同键不同 body 409”；同键并发唯一键兜底由 RD 用例 5 覆盖。
4. 性能（AC-061）、浏览器矩阵（Chrome/Edge/Firefox，AC-063）、窄屏真机与 Safari：属 T21/T22。
5. 导出与备份恢复（AC-058/AC-009/AC-010）：T20。
6. QA 自建 fixture 不含真实供应商协议差异（由 T12/T13 用例覆盖）；本轮未复跑 AC-062 的 10 次切换资源趋势（T18 已验收）。
7. 全项目：T20–T23 未交付，**MVP 尚未完成**；本 PASS 仅覆盖 T19 切片。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 22 | 2026-09-12 | slice | T18 | 2 | FAIL（BUG-007） | 本文件第 22 节 |
| 23 | 2026-09-12 | slice | T18（复验） | 2 | PASS | 本文件第 23 节 |
| 24 | 2026-09-12/13 | slice | T19 | 2（ui_revision 2） | **PASS**（AC-052–AC-057 全绿；0 缺陷；N1–N5 观察） | 本节 |

- **交接给协调者**：(a) `qa_result: PASS`、`qa_round: 24`；(b) `qa_history` 追加 `{round: 24, scope: slice, task_ids: [T19], prd_revision: 2, result: PASS}`；(c) T19 进入 `accepted_tasks`（accepted_ac_ids：AC-052、AC-053、AC-054、AC-055、AC-056、AC-057、卡内边界；UI-042/046–056/059）；(d) `open_defects` 维持空；(e) cameraPose 交 PM 做合同留痕（结论见上；非阻断）；(f) N1/N2 交 RD 决定修复时机（建议随下次触碰 `KnowledgeReviewPanel`/`PublishPanel` 的卡；若 PM 要求立即修复则作为独立小修派发）；(g) 本回合新增 QA 测试 1 个（`apps/web/tests/e2e/qa-t19-independent.spec.ts`，7 用例）并保留。
- **交接给 RD**：无必修项（0 缺陷）。可选（P3）：N1（去掉文案里的 `**`）、N2（PublishPanel 冲突后禁用发布按钮并在 refetch 后清除冲突提示）、N3（步骤面板引用部件显示名称）。
- **交接给 PM**：裁定 hotspots 级 `cameraPose` 的合同留痕（建议按“步骤级视角为 MVP 语义、热点的 `cameraPose` 从合同字段清单移除或标注为未实现”处理；如需热点级字段，走新修订 + 新 AC）。
- **全项目状态提醒**：T01–T19 已验收；T20–T23 未交付，**MVP 尚未完成**；完整发布仍需 T21 全量回归、T22 冷目录 smoke（多平台）、T23 授权后的真实 Provider 验证。
- **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「T19 验收知识（QA 回合 24）」（真实链路 stale 的可切换 CDN、画布滚动/抽屉覆盖层/键盘模态焦点三陷阱、modelReview 整体提交、stale 与 hotspotNotMatchingModel 的区别、raycast 排除的判定做法）。`llmdoc/validation-release.md` 的命令合同**无变化**（未发现需修订的验收命令或阈值）。本报告即验收依据、缺陷与未覆盖边界的正式记录。
- **QA 产出与证据**：`artifacts/web-mvp/t19-qa/`——`qa-file-hashes.txt`（源文件 sha256）、`qa-publishing-1.log`、`qa-publishing-stab-1..5.log`、`qa-cargo-test-workspace.log`、`qa-xtask-check.log`、`qa-dist-smoke.log`、`qa-typecheck.log`、`qa-lint.log`、`qa-vitest.log`、`qa-t19-spec-final3.log`、`qa-e2e-full-final.log`、`qa19-1-pick.json`、`qa19-2-stale.json`、`qa19-3-publish.json`、`qa19-4-narrow.json`、`qa19-5-review.json`、`qa19-6-steps.json`、`qa19-7-reader.json`、`screenshots/{qa19-1-pick-and-raycast,qa19-2-stale-block,qa19-2-rebound,qa19-4-narrow,qa19-4-narrow-publish,qa19-6-steps,qa19-7-reader}.png`。测试代码：`apps/web/tests/e2e/qa-t19-independent.spec.ts`（7 用例）。

---

# 回合 25 · T20（导出、备份、升级与恢复）

结果：**PASS**（仅本切片 T20；全项目尚未完成；1 个 P3 缺陷 OPEN，需 PM 解释，见 BUG-008）· 回合：25 · PRD 修订：2（ui_revision 2）· 范围：切片

**QA 独立性声明**：不采信 RD 结论。`implementation.md` §T20 只作线索；AC-009/AC-010/AC-058 的每一项都由 QA 用 **dist release 二进制**（`b8776b11…`）在自己构造的 data-dir 上重跑，损坏／非空／持锁／路径穿越／符号链接／新 schema 等负例均为 QA 自己构造。RD 的 `cargo test --test backup_restore` 由 QA 全量复跑，但不作为唯一证据。

## 环境与交付版本

- 工作树：`313849611d190e93b65a84cd13dba1b3ff784477` + 未提交工作树（RD T20 改动 + 既有 T19 尾巴；QA 本回合**未改任何生产代码、未改既有测试**，只新增 `artifacts/web-mvp/t20-qa/` 下的验收脚本与日志）。关键源文件/产物 sha256：`artifacts/web-mvp/t20-qa/qa-file-hashes.txt`。
- 工具链：rustc/cargo 1.98.1（2026-09-01）、Node v26.0.0、npm 11.12.1、Playwright（Chromium，全量 86 用例）；macOS Darwin 25.6.0 arm64；系统工具 `sqlite3 3.51.0`、`unzip`、`python3 3.14.2`、`shasum`（用于独立于 Rust 代码的校验）。
- 二进制：`cargo xtask dist --target aarch64-apple-darwin` **QA 自己重建两次**，两次 sha256 **完全一致** = `b8776b115a01e2a90090a92c1351e59117c18f3ef09dfbdf64e0642851d8be2a`（25 044 976 B），与 RD 记录一致 → 当前树可复现；`smoke-bootstrap` exit 0（7×`[检查]`）。
- 数据：QA 工作 data-dir = **QA 自己把 RD 样例备份 `restore` 到 `/tmp/em-t20-qa/data`**，再用真实 HTTP 在其中新建 1 个物品；另用手工构造的 **v1（仅 0001 迁移，checksum 用脚本按 sqlx 规则写入）** data-dir 验证升级门禁。备份/导出产物全部落在 `/tmp/em-t20-qa/`（已清理）。
- **fixture 与真实 Provider 分开列**：本回合 **100% 本机 fixture／零真实外网、零付费**。QA 自查：dist 二进制运行时 provider 未配置（日志 `provider_not_configured`）；我的 serve 日志中出现的 URL 主机只有 `127.0.0.1:18080/18081`；`cargo test --workspace` 与全量 e2e 日志中真实供应商域名 `openapi.tripo3d.ai` 出现 **0 次**。真实 Provider 属 T23。

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/t20-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test backup_restore`（RD 用例复跑，含在 #2 的 workspace 运行内） | exit 0，**7 passed / 0 failed / 1 ignored**（手工演练造数）；无 skip |
| 2 | `cargo test --workspace` | exit 0，**511 passed / 0 failed / 4 ignored**（25 个测试目标 / 33 条 `test result` 行；另有 1 条 doc-test 行 0 用例）（`qa-cargo-test-workspace.log`），与 RD 记录一致 |
| 3 | `cargo xtask check` | exit 0，**7/7 [通过]**（fmt/clippy/单测/前端 lint/typecheck/vitest 130/合同检查）（`qa-xtask-check.log`） |
| 4 | `cargo xtask contracts --check`（随 #3） | 两份 `[一致]`；工作树中 `contracts/openapi.json`、`generated.ts` 哈希在检查前后**不变**（`8bd161e4…`、`c62903ca…`）→ 不修改工作树 |
| 5 | `cargo xtask dist --target aarch64-apple-darwin` ×2 + `cargo xtask smoke-bootstrap --binary <abs>` | 两次 exit 0 且 **同哈希 `b8776b11…`**；smoke exit 0（7×`[检查]`，含 `/api/unknown` JSON 404 与 SPA 路由）（`qa-xtask-dist-1/2.log`、`qa-smoke-bootstrap.log`） |
| 6 | `npm --prefix apps/web run test:e2e`（**全量**） | exit 0，**86 passed / 0 failed / 0 skipped（7.0 m）**（`qa-playwright-e2e.log`）；未复现 RD 记录的 QA 堆趋势用例波动 |
| 7 | QA 手工演练（dist 二进制；`qa-phase1/2/3/3b/4/5/6/7/8/9*.log` + 同名脚本可复跑） | 全部场景得到期望退出码与产物（40+ 个命令级场景，详见下各节与退出码表） |

## AC 验收矩阵（逐子句；证据 = QA 自己的命令输出）

| AC | 子句 → 结论 | QA 独立证据（`artifacts/web-mvp/t20-qa/`） |
| --- | --- | --- |
| **AC-009** | 运行中持锁 → 拒绝并说明需先停服 → **PASS**；停服后备份成功、产物含一致快照 + 全部被引用 blob + manifest + sha256 → **PASS**；已存在的输出路径不被覆盖 → **PASS** | `qa-phase1.log`：真实 `serve` 子进程持锁时 `backup` **exit 5**，stderr 明写“备份要求先停止服务（data-dir 排他锁被占用）…请停止服务后重新执行 backup（运行中备份无法保证一致快照）”，**未创建输出目录**；SIGTERM 停服后 `backup` exit 0，`blobsCopied: 11`＝源库 `blobs` 行数（11）、`missingBlobs: 0`、`sessionsRemoved: 1`；快照 sha256 == `manifest.database.sha256`；无 `-wal/-shm`；`shasum -a 256 -c SHA256SUMS` 逐行 OK（快照+11 blob+manifest）；**源 data-dir（主文件+全部 blob）备份前后指纹完全相同**；同路径再备份 **exit 4** 且已有 `manifest.json` 逐字节不变；`--out` 在 data-dir 内 → **exit 4** 且不创建 |
| **AC-010** | 非空目标被拒绝 → **PASS**；损坏 blob/hash 不符失败并保留现场 → **PASS**；恢复后同一 release/PDF/GLB 可读 → **PASS**；备份不含密钥/绝对路径/会话 → **PASS**；**“备份不含临时云端 URL” 字面不满足 → 见 BUG-008（P3，需 PM 解释）** | `qa-phase2.log`（QA 自造负例，全部 **目标目录未创建**、备份目录树指纹前后不变）：非空目标 4；目标是文件 4；blob 损坏 7；blob 被换成符号链接 7；缺 blob 文件 7；manifest 路径穿越 `../../etc/passwd` 7；manifest 绝对路径 7；manifest 未知字段 7；manifest 短 sha256 7（**未 panic/未 101**）；未知格式版本 7；快照带 `-wal` 边车 7；备份 schema v99 → 4；备份目录不存在 → 4。`qa-phase3.log`：恢复目录 `restore` 与快照 sha256 一致；**恢复后旧会话 cookie → 401**（会话未复活）；同口令可登录（口令哈希保留）；release `manifestSha256` 与发布时一致；PDF `sha256=e18cf61a…`＋`file` = `PDF document, version 1.4, 2 pages`；GLB `sha256=a9f884c2…`＋magic `glTF`、声明长度 2912 == 实际；恢复后的库含 **kill -9 前只有 WAL 才有的物品**；备份内 0 处绝对路径（DB dump 扫描） |
| **AC-058** | 下载自包含包（原件、GLB、manifest、哈希、来源、相对资产清单）→ **PASS**；只含该 release 有权资产 → **PASS**；不含绝对路径/密钥/会话/临时云端 URL → **PASS**；不代表可双击运行网站（文档说明）→ **PASS** | `qa-phase3.log` + `qa-export-sample.zip`：HTTP 200 + `application/zip` + `content-disposition: attachment; filename="release-<id>.zip"`；条目恰为 `manifest.json` / `release/manifest.json` / `assets/model/<sha>.glb` / `assets/document/<sha>.pdf`；冻结 manifest 字节 sha256 == 发布时；导出清单 `schemaVersion=manual_release_export_v1`、`item`、`release`、`knowledge`、`review`、`files[{path,role,assetId,sha256,size,mime,source}]`（2 条，`source` = `tripo` / `itemUpload`）、`notes`（含“不承诺可直接双击运行网站；当前版本不提供导入接口”）；**系统 `unzip -t` `No errors detected` + python `zipfile.testzip()` = None**；整包字节扫描：会话 token / 管理员口令哈希 / canary 假密钥 / `cdn.example.invalid` / fixture 本机 URL `127.0.0.1:60643` / `Bearer ` / `em_session=` / data-dir 绝对路径 / `/Users/` **全部 0 命中**；**其它 blob（页图/页文字/照片）字节 0 命中**（11 个源 blob 里只出现 model/document/manifest 三个）；无 cookie → 401、未知 release → 404；导出后 `tmp/` 无残留 |
| 卡内项 · 备份不得只复制运行中 WAL 主文件 | **PASS（三项独立证据）** | ① `qa-phase1.log`：停服后 `backup` 前把主文件复制到别处（负对照）→ 该副本 `items = 1` 且**不含** kill -9 前提交的新物品；而真实备份快照 `items = 2` 且**含**该物品 → 快照确实包含 WAL 中已提交事务，不是主文件复制。② 源库主文件 sha256 在备份前后相同，且 `-wal`（32 992 B）在 kill -9 后仍存在。③ `restore` 拒绝带 `-wal` 边车的“假快照”（7） |
| 卡内项 · 路径防穿越与符号链接拒绝 | **PASS** | `qa-phase2.log` C/D/F/F2：`../../etc/passwd`、`/etc/passwd`、blob 换成符号链接 → 均 7 且目标不创建；`blobs/` 内路径只由服务端生成（`blobs/<sha[0..2]>/<file_name>`，file_name 取自 manifest 且内容按 sha256 逐个校验） |
| 卡内项 · schema 门禁回归 | **PASS** | `qa-phase4/5/9`：手工 v1 data-dir → `check` 报“待迁移（库 v1 → 程序 v7）”+ 升级提示；`backup`（不解释 schema，manifest 记 `schemaVersion: 1`，源库仍 v1）→ `restore` 到新目录（仍 v1）→ `serve` 打印“数据库 schema 已自动升级 v1 → v7…程序回滚不等于数据库回滚”，数据保留（`QA 升级前数据|LEGACY-1|rev5`）。注入 v99：`check`/`serve`/`init`/`restore` **全部 exit 4** 且库文件哈希不变；`backup` 允许（manifest 记 99）——与 ADR-031 注 1 的“灾备窗口”一致 |
| 卡内项 · 退出码更新（7/4/5 + 与 ADR-011 留痕一致） | **PASS** | 见下节实测表；ADR-011 注 4（就地标注指向 ADR-031）→ ADR-031 注 7 → `implementation.md` T02-4 的 T20 事实更新块 → §T20-5 表 → 代码 `ExitCode::Integrity = 7` + `backup_error` 映射 + `config/error.rs` 模块文档表，**五处口径一致**，QA 逐条实测无冲突 |
| 回归（T01–T19 证据链） | **PASS** | `cargo test --workspace` 511/0/4i（#2）、`cargo xtask check` 7/7（#3）、合同两份一致且不改树（#4）、dist×2 同哈希 + smoke-bootstrap（#5）、全量 e2e 86/86（#6） |

## 退出码实测表（QA：dist 二进制逐条运行，`qa-phase1/2/6.log`）

| 码 | 文档含义（T20 后） | QA 触发现场 | 实测 |
| --- | --- | --- | --- |
| 0 | 成功 | 停服后 backup；restore 到不存在/空目录 | 0 ✅（stdout 含“备份完成/恢复完成”） |
| 1 | 运行时错误 | `backup --out /dev/null/backup-x`（父路径不可创建） | 1 ✅ |
| 2 | 用法错误（行为不变） | 缺 `--out`、缺 `--from`、未知子命令 | 2 ✅ |
| 3 | 配置错误 | 配置文件含 `bogus_key` | 3 ✅（`unknown field`） |
| 4 | data-dir／路径错误 | 源 data-dir 不存在；`--out` 已存在；恢复目标非空/是文件；备份与 data-dir 嵌套；备份来源不存在；备份的 schema 比程序新（v99）；`check`/`serve`/`init` 遇 v99 | 4 ✅（8 个场景） |
| 5 | 排他锁冲突 | **服务运行中 backup**（真实子进程持锁） | 5 ✅（消息含“停止服务”） |
| 6 | 安全拒绝（行为不变） | `serve --listen 0.0.0.0:18084` 无 TLS/代理 | 6 ✅ |
| 7 | **备份/恢复完整性校验失败**（原“未实现”） | blob 损坏/缺失/符号链接、快照损坏、快照带边车、manifest 缺失/非法/未知字段/未知版本/不安全路径 | 7 ✅（10 个场景，**无 panic、无 101**） |

## UI-060 裁定（RD 报告“网页导出按钮未实现”）

- **事实核对**：`apps/web/src/features/manual/{ReleaseListPage,ReleaseReaderPage}.tsx` 中**没有任何导出入口**（grep 无导出相关 UI 代码）；端点与类型已就绪（`contracts/openapi.json` 有 `/api/v1/releases/{releaseId}/export`（200 `application/zip` → TS `string`），`generated.ts` 有 `export_release`）。
- **判定 1（是否属 T20 范围）**：T20 卡允许范围 = server 侧（export/backup/restore/migration guard）+ xtask + `tests/backup_restore.rs`，**不含 web 功能代码**；RD 未越界改动，属**如实申报**而非隐瞒。
- **判定 2（是否阻断必选 AC）**：**不阻断**。AC-058 的测试方式是 `cargo test -p everything-manual --test backup_restore`（导出部分），其 Then 子句的可观察项全部在 HTTP/包内容层面，QA 已用真实二进制 + curl + 系统 `unzip` 逐条实测通过；“不代表可双击运行网站（文档说明）”也由导出清单 `notes` 与 OpenAPI 描述承载。
- **判定 3（项目影响）**：PRD 修订 2 §6.2 的 **UI-060 仍是 REQ-037/AC-058 的欠交付 UI 项**，且 §7.2 的 S4 门禁写着“向导→任务→校准→发布→**导出全链路**”。故：**登记为对后续卡（T21/T22 或专派前端小卡）的要求**，理由 = (a) T20 文件范围不含 web，(b) 导出端点/生成类型/包内容已验收充分，前端只欠“下载 + 进行中/失败重试 + 常驻说明 + 键盘可达”的 UI-060 交互，(c) 若 T21/T22 的浏览器矩阵不覆盖该入口，则 REQ-037 的 UI 部分将随 MVP 一起缺交付。**T20 不因此判 FAIL**；请协调者在 T21 前把该 UI 项写入卡或由 PM 修订 PRD（QA 不改规范文档）。

## 缺陷

### BUG-008 · 备份快照内含供应商响应的临时云端 URL（AC-010“备份内容不含……临时云端 URL”字面不满足；需 PM 解释）

- 严重度／状态：**P3 / OPEN（需 PM 解释；若 PM 判为严格口径则升级为 P1 并转 RD 修复 + 复验）**
- 对应 REQ / UI / AC：REQ-005 / **AC-010**（“导出与备份内容不含密钥、绝对路径、会话、临时云端 URL”）；与 **T13 沿用至今的 P3（签名 URL 在 `job_stages.usage_json` 长期留存）同源**（`state.yaml` carry_over 已列此项待 T23 收敛）。
- 环境与输入：dist `b8776b11…`；data-dir = QA 由 RD 样例备份 `restore` 得到 + 运行期新建 1 物品（kill -9 后备份）；备份 `/tmp/em-t20-qa/my-backup`。
- 复现步骤（QA 实跑）：
  1. `everything-manual backup --data-dir <data> --out /tmp/em-t20-qa/my-backup`（exit 0）；
  2. `sqlite3 my-backup/database/manual.sqlite3 .dump | grep -c "cdn.example.invalid"` → **1**；
  3. `sqlite3 ... "SELECT stage_kind, usage_json FROM job_stages WHERE usage_json LIKE '%example.invalid%'"` → `tripo_poll` 的观察 JSON 中含 `"renderedImageUrl":"https://cdn.example.invalid/preview.png"`（production 中为供应商签名临时 URL）；同一 JSON 还含 fixture 本机地址 `http://127.0.0.1:60643/model.glb`。
- 期望与实际：期望按 AC-010 字面 = 备份内容不含临时云端 URL；实际 = 备份快照忠实复制了 DB 中 `job_stages.usage_json` 记录的供应商响应 URL（**1 行**）。
- 影响与反证：① **密钥 0 命中**（canary 扫描）、**会话 0 行**、**绝对路径 0 命中**（DB dump 扫描）、导出包全项干净；② 备份目录 0700/文件 0600、`notes` 明写“备份含用户原始资料，按部署者数据保护要求管理”；③ 该 URL 在**运行中 data-dir 里本来就长期存在**（T13 已记录并在案），备份并未新增暴露面；④ 若在快照里脱敏 `usage_json`，会破坏“恢复后继续查询在途远端任务”的依据（该字段是恢复/对账的输入），属产品取舍。
- 证据路径：`artifacts/web-mvp/t20-qa/qa-phase8-urls.log`、`work-notes/backup-temp-url-row.txt`、`work-notes/qa-backup-manifest.json`、`qa-phase7-misc.log` F 段、`qa-phase3.log` 第 9 步（导出包同项 0 命中，形成对照）。
- 回归范围：若采用“快照脱敏”，需重跑 `--test backup_restore`、`jobs_recovery`（恢复语义）、本报告 `qa-phase1/3` 脚本，并评估对在途任务恢复的影响。
- RD 修复摘要引用：无（未派修）。
- QA 复验结果与日期：待 PM 解释后处理。**QA 未自行接受该偏离**：本项按“需要产品解释时列证据交 PM”处理。

本回合 **0 个 P0/P1/P2 缺陷**；新增 1 个 P3（BUG-008，OPEN，待 PM 解释）。

## 非阻断观察（P3/P4；供 RD/PM/T21/T22）

1. **OB-1 · 文档中的退出码 5“恢复目标被占用”在当前实现里不可达**：恢复目标一旦有 `lock` 文件就不再是“空目录”，`restore` 会先以 **exit 4（非空）** 拒绝（QA 实测：用 python 持 flock 的空目录仍得 4）。实现上 fail-safe（仍然拒绝），但 §T20-5 与 `error.rs` 文档表把“恢复目标被占用 → 5”写成可发生场景，建议后续卡注明或改文案。
2. **OB-2 · 错误文案拼接瑕疵**：`备份中的 blob的 sha256 与 manifest 不符` / `备份中的 blob是符号链接`（label + 谓语直接拼接，缺空格/助词）。语义可读，建议触碰该文件时改为“备份中的 blob 的 sha256…”。
3. **OB-3 · 备份/恢复无空间预检**（RD 已记录 §T20-10 第 2 条）：磁盘写满走 exit 1 并保留现场。沿用，非阻断。
4. **OB-4 · GLB 经内容端点仍为 `application/octet-stream`**（RD 已记录 §T20-10 第 4 条；T06 白名单未含 `model/gltf-binary`）：属 T06 已验收行为，交 PM/T18 决定是否改；本卡未动 `http/assets.rs`（QA 复核 diff：无该文件改动）。
5. **OB-5（沿用 N5）· 证据目录共享**：本轮全量 e2e 再次重写 `artifacts/web-mvp/t09-rd/e2e-server.log` 与 `t16-*/t17-*/t18-*/t19-*` 截图/日志（内容为同一 spec 重新生成）；建议后续卡改用独立子目录。
6. **OB-6 · 环境里有一个孤儿 `serve` 测试进程（非本回合启动，QA 未清理）**：PID 14182、PPID 1、启动于 **09-13 00:18:10**（早于 QA 到达），命令为 `target/debug/everything-manual serve --data-dir <临时目录>/restored-data`，而该临时 data-dir **已被删除**。QA 本回合启动的进程已全部结束（`pgrep -f 'serve --data-dir /tmp/em-t20-qa'` 为空）；该孤儿进程不属于本回合、为避免误杀未处理。提示：测试 Harness 的现场清理在“测试被中断”情形可能漏杀子进程，T22 `smoke` 的“只结束自己启动的进程”门禁值得加断言（无证据表明正常通过的用例会漏杀）。

## 非代码知识与限制（提炼；跨卡复用条目已追加到 `llmdoc/decisions.md`）

- **“恢复目录 sha == 备份快照 sha”的比较必须在首次启动服务之前做**（QA 首次踩坑：先起过一次 serve 再比对，读到 `6c4629…` ≠ manifest `7d61a8…`）。原因：备份快照把 journal 模式归一为 `DELETE`，恢复后首次 `serve` 按合同切回 WAL/FULL → 主文件头被改写，字节必然不同（数据不变）。这是**预期行为**（ADR-031 注 2 / §T20-10 第 5 条），不是恢复错误；验收脚本应按“先比对字节，再起服务”排序（`qa-phase3b-restore.log`）。
- **重启后仍有 `-wal` 的数据库无法用只读 URI 打开**（`sqlite3 file:…?mode=ro` → `unable to open database file (14)`，因为只读连接不能建 `-shm`）；做“主文件不含 WAL 数据”的负对照时要用可写打开或 `-shm` 已存在。QA 的 `qa-phase1.log` 第 6 步因此先失败、随后用可写打开取到结论（主文件副本 `items=1`，缺 WAL-only 物品）。
- **kill -9 是制造“WAL 中已提交事务”的最小手段**：正常 SIGTERM 停机时 SQLite 会 checkpoint 并删除 `-wal`；`kill -9` 后 `-wal`（本机 32 992 B）保留，此时“复制主文件”的负对照能稳定复现出丢数据，而 `VACUUM INTO` 快照含全部已提交事务。这是验收“不得只复制运行中 WAL 主文件”最直接的可执行判据。
- **手工构造 v1（旧 schema）data-dir 可行**：用 `sqlite3` 执行 `migrations/0001_core_schema.sql`，再按 sqlx 规则写 `_sqlx_migrations` 行（`checksum = sha384(迁移文件字节)`、`description` 取文件名去 `.sql`、`success=1`），程序即可正常识别为 v1 → 用于验证“check 待迁移提示 / backup 不迁移源库 / restore 后 serve 自动迁移”。
- **macOS 自带 bash 3.2 的坑**（RD 亦记录）：`"$var（"` 会把全角括号并入变量名（`unbound variable`）；QA 脚本本轮实际踩到一次。脚本里变量后紧跟中文一律写 `${var}`。
- **验收脚本用 `set -u` 时必须逐行检查**：一处未加花括号会让整段脚本在中途退出（本轮 phase3 第一次运行即如此），造成“命令没跑却说通过”的风险；QA 以重跑后的完整日志为准。

## 未覆盖边界（不冒充通过）

1. **真实 Provider / 真实费用**：全部本机 fixture，零外网零付费；T23 授权后另验。
2. **网页端导出入口（UI-060）**：未实现（裁定见上，交后续卡）；AC-058 未按浏览器 UI 验收。
3. **备份/恢复的空间预检、磁盘满路径**：未测（RD 已登记为后续增强）。
4. **跨平台**：只测 aarch64-apple-darwin（AC-064 属 T22）；`x86_64-unknown-linux-musl` 未跑。
5. **ZIP 导入**：合同明确不开放，QA 未测（也无接口）。
6. **备份的“跨程序版本回滚”实操**：用同一二进制演练升级（v1→v7）；用旧二进制读新 schema 只验证了“拒绝打开”（exit 4），未做真实旧版本二进制回滚（当前只有一个版本可用）。
7. **全项目**：T21–T23 未交付，**MVP 尚未完成**；本 PASS 仅覆盖 T20 切片。
8. QA 未独立注入“快照 `_sqlx_migrations` 版本与 manifest 记录不一致”的伪造场景（实测门禁读 DB 而非 manifest，见 `qa-phase2.log` I 段）。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 23 | 2026-09-12 | slice | T18（复验） | 2 | PASS | 本文件第 23 节 |
| 24 | 2026-09-12/13 | slice | T19 | 2（ui_revision 2） | PASS | 本文件第 24 节 |
| 25 | 2026-09-13 | slice | T20 | 2（ui_revision 2） | **PASS**（AC-009/AC-010/AC-058 与全部卡内项通过；BUG-008 P3 OPEN 需 PM 解释） | 本节 |

- **交接给协调者**：(a) `qa_result: PASS`、`qa_round: 25`；(b) `qa_history` 追加 `{round: 25, scope: slice, task_ids: [T20], prd_revision: 2, result: PASS}`；(c) T20 进入 `accepted_tasks`（accepted_ac_ids：AC-009、AC-010、AC-058、卡内项〔WAL 快照、路径防穿越/符号链接、schema 门禁、退出码 7/4/5〕）；(d) `open_defects` 记 **BUG-008 (P3, OPEN, 需 PM 解释)**；(e) UI-060 建议写进 T21/T22 卡或专派前端小卡；(f) `carry_over` 建议新增：BUG-008（P3 待 PM 解释）、UI-060（网页导出入口，交 T21/T22 或专派前端卡）、OB-1（退出码 5 不可达的文档订正）、OB-2（错误文案拼接）、OB-6（孤儿测试进程提示）。
- **交接给 RD**：无必修项。可选（P3/P4）：OB-1（§T20-5/`error.rs` 文档里“恢复目标被占用 → 5”不可达）、OB-2（`blob 的` 文案拼接）。
- **交接给 PM**：裁定 **BUG-008**——(a) 若认定“备份是 DB 的忠实副本，供应商响应 URL 属用户数据组成部分、备份机制未新增暴露面” → 请做合同/AC 留痕（可并入 T13 既有 P3 的 T23 收敛项）；(b) 若认定 AC-010 的备份子句为严格口径（备份内不得出现任何临时云端 URL 字串） → 该子句不通过，请派卡做快照脱敏（并评估对在途任务恢复的影响），QA 将复验并把 T20 判为 FAIL。**QA 不自行接受该偏离。**
- **全项目状态提醒**：T01–T20 已验收；T21–T23 未交付，**MVP 尚未完成**；完整发布仍需 T21 全量回归与安全矩阵、T22 多平台冷启动 smoke、T23 授权后的真实 Provider 验证。
- **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「T20 验收知识（QA 回合 25）」（恢复后首次启动会改变快照字节的比较顺序、WAL 只读打开失败、kill -9 制造 WAL-only 事务、手工构造 v1 data-dir、bash 3.2 变量拼接、BUG-008 的解读点）。`llmdoc/validation-release.md` 的命令合同**无变化**（未发现需修订的验收命令或阈值）。本报告即验收依据、缺陷与未覆盖边界的正式记录。
- **QA 产出与证据**：`artifacts/web-mvp/t20-qa/`——`qa-file-hashes.txt`、`qa-phase1-lock-and-wal.sh`+`qa-phase1.log`、`qa-phase2-restore-negatives.sh`+`qa-phase2.log`、`qa-phase3-restore-serve-export.sh`+`qa-phase3.log`、`qa-phase3b-restore.log`、`qa-phase4-legacy-schema.log`、`qa-phase5-newer-schema.log`、`qa-phase6-exit-codes.log`、`qa-phase7-misc.log`、`qa-phase8-urls.log`、`qa-phase9-final-checks.log`、`qa-cargo-test-workspace.log`、`qa-xtask-check.log`、`qa-xtask-dist-1/2.log`、`qa-smoke-bootstrap.log`、`qa-playwright-e2e.log`、`qa-export-sample.zip`（QA 自己导出的样例包）、`work-notes/`（备份 manifest、SHA256SUMS、快照行数、BUG-008 命中行）。**临时目录 `/tmp/em-t20-qa/` 已清理；本回合启动的进程已全部结束（环境里一个 00:18 起的孤儿测试 `serve` 进程非本回合所启，见 OB-6）。**

---

# 回合 26 · BUG-008 复验（新数据／历史数据脱敏、恢复语义、导出兜底）与 T18-4 偶发裁定

结果：**BUG-008 → CLOSED；T20 切片 PASS 不受影响；本轮整体 FAIL**（复验通过原缺陷，但**新增 BUG-009（P2，OPEN）**：模型下载的失败路径把完整签名 URL 写进 `job_stages.last_error`，并经任务详情 API 与 `model_download_failed` 日志输出；另有 **BUG-010（P4，OPEN，非阻断）**：备份文本兜底右向吞字）。回合：26 · PRD 修订：2（ui_revision 2）· 范围：缺陷复验（BUG-008）+ 回归

**QA 独立性声明**：不采信 RD 结论、不采信 RD 脚本输出。本回合全部结论来自 QA 现场执行：dist 二进制 QA 自己重建（哈希与 RD 记录一致）、**新数据链路**用 dist 真跑（QA 自建本机 fixture，含重启后的 task_id 重查）、**历史库**由 QA 自己构造（归档样例 `restore` 回来后**再自注入** JSON/非 JSON 文本 URL 与用户出处链接）、**读取侧透传**由 QA 自己注入后在真实 HTTP 上观察、**导出 fail-closed 与误伤**由 QA 自己造 manifest 触发。RD 的 `cargo test` 用例由 QA 全量复跑，但不作为唯一证据。

## 环境与交付版本

- 工作树：`313849611d190e93b65a84cd13dba1b3ff784477` + 未提交工作树（RD BUG-008 修复 + 既有改动）。**QA 未改任何生产代码**；新增 `crates/server/tests/qa_t20_bug008_independent.rs`（BUG-009 验收/回归测试）与 `artifacts/web-mvp/bug008-qa/**`（脚本+证据），并**只改自己写的** `apps/web/tests/e2e/qa-t18-independent.spec.ts`（T18-4 堆采样口径，理由见下）。
- 工具链：rustc/cargo 1.98.1、Node v26.0.0、Playwright（Chromium，全量 86 用例）；macOS Darwin 25.6.0 arm64；系统 `sqlite3 3.51.0`、`python3 3.14.2`、`shasum`。
- 二进制：`cargo xtask dist --target aarch64-apple-darwin` **QA 自己重建两次，两次 sha256 一致** = `688a57b98c879e65e97aa0479102e0bffb55d379c14430fa8f485d8828c8476a`（25 063 520 B），与 RD §T20-13 记录一致（`qa-xtask-dist.log`、`qa-xtask-dist-2.log`、`qa-dist-sha256*.txt`）。
- 数据：**历史库 = `artifacts/web-mvp/t20-rd/sample-backup/`（修复前二进制产生的归档样例）`restore` 回来后由 QA 自注入新历史行**；新数据 = QA 自建 data-dir + 真实 dist `serve` + 真实 HTTP。全部备份/导出产物落在 mktemp 目录（已清理）。
- **fixture 与真实 Provider 分开列**：本回合 **100% 本机 fixture（`127.0.0.1`）、零付费**；真实 Provider 属 T23。**例外如实披露**：`qa-r26-newdata-requery.sh` **首版**把 `manual_ai` 指向默认公共端点，一次 `manual_extract` 批次请求被发出（`manual_extract_sent`，日志时间 18:25:53Z；未使用真实密钥、无付费）；QA 随即修正脚本（`base_url` 指向本机 fixture）并**重跑为纯回环**——阶段 5b 断言：serve 日志里非回环主机数 = 0。该外发不影响任何 BUG-008 判据（Tripo 链与全部断言均为回环，前后扫描均 0 命中）。

## QA 本轮的 URL 判定规则（自行定义；避免把裸主机名当 URL）

| 判据 | 含义 | 命中即视为"临时/签名 URL 泄露" |
| --- | --- | --- |
| `://` | URL 形态（scheme 分隔符） | 是（**唯一例外**：部署自己的监听横幅 / `provider baseUrl` 回显 / `generation_snapshots.provider_config` 属配置，不是供应商临时地址） |
| `sign=`（及任何签名查询串） | 能力凭据 | 是 |
| URL path（`/qa-r26/model.glb`、`/qa-r26-hist/preview.png`、`/qa-r26-crafted/…`） | 供应商产物路径 | 是 |
| 本轮唯一 canary（`qa-r26-canary-9f3c`、`qa-r26-hist-9c4d`、`qa26-canary-transport-signature-7d31`） | 本次签名的整串 | 是 |
| 裸 host（`cdn.example.invalid`） | 任务卡允许保留的最小诊断信息 | **否**（RD 提醒成立：裸主机名不是 URL） |
| `{"redacted":true,"host":…,"sha256":16 位}` / 文本标签 | 允许保留的不可逆摘要 | **否** |

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/bug008-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo xtask dist --target aarch64-apple-darwin`（QA 重建 **×2**） | 两次 exit 0 且**同哈希** `688a57b9…`（25 063 520 B），与 RD 记录一致（`qa-xtask-dist.log`、`qa-xtask-dist-2.log`、`qa-dist-sha256*.txt`） |
| 2 | `cargo test --workspace` | exit 0，**524 passed / 0 failed / 6 ignored**（含 QA 新增 1 通过 + 2 个 BUG-009 复现用例 `#[ignore]`）；`qa-cargo-test-workspace.log` |
| 3 | `cargo test -p everything-manual --test backup_restore` / `--test tripo_contract` / `--test pipeline` / `--test model_assets` / `--test jobs_recovery` | 9/0/1i、18/0/0、13/0/0、23/0/0、25/0/0（与 RD 记录一致；`qa-cargo-test-*.log`） |
| 4 | `cargo xtask check`（收尾复跑） | **7/7 [通过]**（fmt/clippy/单测/前端 lint/typecheck/vitest/合同）；首轮曾因 QA 新文件 import 顺序触发 `fmt --check` 失败（QA 自身产物、非产品问题），修正后复跑通过（`qa-xtask-check-final.log`；首轮 `qa-xtask-check.log`） |
| 5 | `cargo xtask contracts --check` | 两份 `[一致]`；生成物哈希前后不变（`e39c4c30…`、`600cb7df…`）→ 不改工作树（`qa-xtask-contracts-check.log`） |
| 6 | `cargo xtask smoke-bootstrap --binary <dist>`（首轮 + 收尾） | 两次 exit 0，各 7×`[检查]`（`qa-xtask-smoke-bootstrap.log`、`qa-xtask-smoke-bootstrap-final.log`） |
| 7 | `npm --prefix apps/web run test:e2e`（全量，**两次**） | 86 passed / 0 failed / 0 skipped（7.0 m）×2（`qa-e2e-full-r26.log` 为改口径前、`qa-e2e-full-r26-final.log` 为改口径后） |
| 8 | `bash qa-r26-newdata-requery.sh <dist>`（QA 自建剧本） | exit 0，**20 项通过 / 0 失败**（`qa-r26-newdata.log`） |
| 9 | `bash qa-r26-historical-backup.sh <dist>`（QA 自建剧本） | exit 0，**30 项通过 / 0 失败**（`qa-r26-historical.log`） |
| 10 | `bash qa-r26-export-userurl.sh <dist>`（导出误伤） | exit 0，PASS（`qa-r26-export-userurl.log`） |
| 11 | `bash qa-r26-text-swallow.sh <dist>`（BUG-010 复现） | 复现"已过期"被吞（`qa-r26-text-swallow.log`） |
| 12 | `bash qa-r26-lasterror-exposure.sh <dist>`（读取侧透传观察） | 观察：`lastError` 原样含签名 URL；同一响应 `usage` 已脱敏（`work-historical/qa26-lasterror-observation.txt`） |
| 13 | `cargo test -p everything-manual --test qa_t20_bug008_independent`（QA 新测试） | 1 passed / 0 failed / 2 ignored（**2 个 ignore 即 BUG-009；用 `-- --ignored --nocapture` 可复现 RED**） |

## BUG-008 复验矩阵（逐项；证据 = QA 自己的命令输出）

| 复验项（任务书 1–5） | 结论 | QA 独立证据 |
| --- | --- | --- |
| 1. 新数据不含供应商临时/签名 URL（`job_stages` / `provider_attempts` 全部文本列 × 全部判据） | **PASS** | `work-newdata/scan-phase1.txt`（第一进程）+ `qa-r26-newdata.log` 阶段 5 版块（重启后第二进程）：**逐表逐列 0 命中**（含 `last_error`、`needs_input_json`）；`tripo_poll.usage_json` = `remoteTaskId` + `modelUrl/renderedImageUrl` 两个摘要（`host=cdn.example.invalid` + 16 位 sha256）+ `billing`（`creditMinor=3000`） |
| 2a. 历史库备份快照不含临时 URL（QA 自注入 3 行 URL，含非 JSON 文本列） | **PASS** | `qa-r26-historical.log`：`tempUrlsRedacted: 4`；快照 0 命中；非 JSON 列被替换为文本标签（`（临时供应商地址已脱敏：host=…；sha256=…）`） |
| 2b. 恢复后数据仍可用 | **PASS** | 12 张表行数逐表一致（items/jobs/job_stages/provider_attempts/assets/blobs/pages/documents/preparations/photos/manual_releases/model_revisions）；恢复库 0 命中；`restore` exit 0；serve 后登录 200、任务详情 200、导出 200；PDF/GLB blob sha256 与恢复前一致 |
| 2c. 源库不被就地改写（无 UPDATE） | **PASS** | `before.txt == after.txt`（库文件 sha256 + 全量 `.dump` sha256 + 11 个 blob sha256 逐行相同）；源库仍含注入的 3 行 URL（未迁移） |
| 2d. 不过度清洗（用户内容不误伤） | **PASS** | 快照与恢复库的 `documents.source_url`（用户出处链接 `https://user.example.invalid/spec?sig=qa-r26-user-link-3ad9`）**原样保留** |
| 3. 恢复语义不受影响（按 task_id 重查、不新增付费 POST） | **PASS** | ① 新数据剧本：SIGKILL 重启后 fixture `GET /v3/tasks/` 2→3，`POST /v3/generation/multiview-to-model` 恒为 **1**；② 历史库剧本 8b：恢复库重启后 fixture `poll=2`、`submit=0`；③ 回归：`--test model_assets`（含 RD 新增 `restarted_download_requeries_by_task_id_and_never_repurchases`）与 `qa_t13_independent.rs` 的 `qa_refreshed_link_is_revalidated_and_never_repurchases`（QA 回合 15 用例，未改）全绿 |
| 4a. 任务详情 DTO 不回显完整签名 URL（usage） | **PASS** | 新数据 + 恢复库两次实测：`://`/`sign=`/path/canary 各 **0**；`usage.modelUrl` 为摘要对象。**但** `lastError` 字段另有通路 → 见 BUG-009 |
| 4b. 导出包干净 | **PASS** | 恢复库导出：200 + 4 条目（`manifest.json`、`release/manifest.json`、model GLB、document PDF）；5 个判据（含用户链接 canary、`cdn.example.invalid`）全 0 命中 |
| 5. 兜底策略（备份=过滤 / 导出=失败） | **评估见下节**：方向正确、与 AC-009/AC-010/AC-058 相容；发现 1 处文本误伤（BUG-010）与 1 处残留口径问题（OB-9） | `qa-r26-text-swallow.log`、`work-historical/qa26-lasterror-observation.txt` |
| 导出 fail-closed 真触发 | **PASS** | QA 把 URL 注入"生成字段"来源（`assets[].source`）→ 导出 **500**、无 ZIP（响应体是错误 JSON，无 `PK` magic）、服务端日志 `export_manifest_url_forbidden`、响应体 0 命中（`qa-r26-historical.log` 第 9 步） |
| 导出不误伤用户内容 | **PASS** | QA 把用户链接注入冻结 manifest 的 `knowledge` 子树 → 导出仍 **200**，链接随包保留（`qa-r26-export-userurl.log`） |

**BUG-008 结论：CLOSED。** 回合 25 的复现形态（`job_stages.usage_json` 里的供应商临时 URL 进入备份快照）在**新数据**与**历史数据**两条路径上都不再成立，并保留了恢复所需的 `task_id`/状态/计费/摘要；源库不被改写；导出与展示（`usage`）干净。3+3 条独立证据来自 QA 自建剧本，不依赖 RD 脚本。

## 缺陷

### BUG-008 · 备份快照内含供应商响应的临时云端 URL（回合 25 原文见上）

- 严重度／状态：P3 → **CLOSED**（2026-09-13 · 回合 26 复验通过）
- 对应 REQ / UI / AC：REQ-005 / AC-010
- 复验方式：见上"BUG-008 复验矩阵"（新数据/历史库/源库不变/恢复可查/展示与导出）；判据与脚本可复跑：`artifacts/web-mvp/bug008-qa/qa-r26-newdata-requery.sh`、`qa-r26-historical-backup.sh`
- 遗留（不属本缺陷，另立）：**BUG-009**（失败路径的签名 URL 落库/出网/进日志）——同源问题的另一条通路，见下。

### BUG-009 · 模型下载的传输类失败把**完整签名 URL** 写进 `job_stages.last_error`，并经任务详情 API 与日志输出（新数据，非历史遗留）

- 严重度／状态：**P2 / OPEN（阻断本轮 PASS；交 RD 修复后由 QA 复验）**
- 对应 REQ / UI / AC：contracts §1（"不向用户输出……完整供应商签名 URL"）；REQ-044 / **AC-066**（"日志……无……签名 URL 查询串"——`model_download_failed` 日志行同源）；REQ-043 / AC-065；与 BUG-008 同一"临时地址不得出网"的裁定方向相反（本轮为**新增数据**，不受"历史数据不迁移"豁免）。
- 环境与输入：dist `688a57b9…`；测试构建（`allow_local_fixture`）用于在**不触外网**的前提下制造传输失败；生产等价路径 = 模型 CDN 的 TCP 连接失败/超时（`DownloadPolicy::default()`：connect 10s、request 600s，150 MiB 下载在慢速/CDN 抖动下都会走到）。
- 复现步骤（QA 实跑，两种失败形态）：
  1. 见 `crates/server/tests/qa_t20_bug008_independent.rs`：`cargo test -p everything-manual --test qa_t20_bug008_independent -- --ignored --nocapture`
  2. 连接被拒：`TcpListener::bind("127.0.0.1:0")` 后立即释放端口 → `ModelDownloader::download("http://127.0.0.1:<port>/qa-r26/model.glb?sign=<canary>")` → 实得消息（端口每次不同）：`模型下载传输失败（下载可安全重试；不产生供应商费用）：连接失败：error sending request for url (http://127.0.0.1:65076/qa-r26/model.glb?sign=qa26-canary-transport-signature-7d31)`
  3. 超时：本机 TCP 收下请求后不响应（1s 超时）→ 实得消息：`……超时：error sending request for url (http://127.0.0.1:65077/qa-r26/model.glb?sign=qa26-canary-transport-signature-7d31&expires=9999999999)`（原文见 `qa-t20-test.log`；本次运行 exit 101 / 0 passed / 2 failed）
  4. 读取侧（真实二进制观察）：`bash artifacts/web-mvp/bug008-qa/qa-r26-lasterror-exposure.sh <dist>` → `GET /jobs/{id}` **HTTP 200 响应体里 `lastError` 原样含该签名 URL**（同一响应里 `usage` 已被脱敏为摘要，形成对照）
- 期望与实际：期望（按 ADR-032 第 2 条与 contracts §1）任何失败路径都不得把完整签名 URL 落库/出网/进日志；实际 = `reqwest` 的错误 Display 会在请求错误后追加 ` for url (<完整 URL，含查询串>)`（reqwest 0.13.5 `src/error.rs:299-302`），而该文本被原样拼接进 `DownloadError::Transport{detail}`，之后**没有任何脱敏**地流到：`handlers.rs` 的 `download_failure_outcome` → `StageOutcome::Retryable{Failed}` → `job_stages.last_error`（`repo/job_stages.rs:428`）→ ①任务详情 DTO `lastError`（`http/jobs.rs`：只有 `usage` 走 `redact_urls`，`last_error` 直传）；②`model_download_failed` 日志（`handlers.rs:1377-1386`，`detail = %error.log_summary()`，与消息同串）。
- 影响与反证：① **本轮新数据**受影响（不是"历史遗留"）；② 备份快照仍被 BUG-008 的过滤兜住（快照扫描 0 命中），所以 AC-010 的备份子句不因此失败；③ 泄露面 = 已认证管理员的任务详情 API 与服务器日志；链接是有时效的 CDN 能力（可下载已付费模型）；④ 另两种传输形态（body 半关闭截断）**不含** URL——只测该形态会误判为"无泄露"。
- 证据路径：`crates/server/tests/qa_t20_bug008_independent.rs`（2 个 `#[ignore]` 断言 + 1 个通过的半关闭对照）、`artifacts/web-mvp/bug008-qa/qa-t20-test.log`（RED 输出原文）、`work-historical/qa26-lasterror-observation.txt`。
- 回归范围：`--test model_assets`（下载失败分类）、`--test pipeline`、`--test backup_restore`（快照过滤仍应兜住）、`--test config_cli`、全量 e2e（任务详情 UI 不出现 `://`）；建议同时补一条"失败路径不含签名"的 RD 单测。
- RD 修复摘要引用：无（本轮新开）。
- QA 复验结果与日期：待 RD 修复后复验（期望：把 `classify_transport` 的 detail 收敛为不含 URL 的文本，例如只留 `kind + reqwest 错误 kind`，或对 detail 走 `redaction`/`redact_url_query`；并把 `http::jobs` 的 `last_error` 也纳入读取侧脱敏；两条都做更稳）。

### BUG-010 · 备份"非 JSON 文本列"兜底脱敏右向吞字（静默丢字，可复现）

- 严重度／状态：**P4 / OPEN（非阻断；理由：只影响快照副本里供应商事实表的自由文本，源库与恢复判据不受影响）**
- 对应 REQ / UI / AC：AC-010（备份内容语义）、`implementation.md` §T20-13 第 5 条"非 JSON 文本级兜底"
- 环境与输入：dist `688a57b9…`；`bash artifacts/web-mvp/bug008-qa/qa-r26-text-swallow.sh <dist>`
- 复现步骤：`restore` 归档样例 → `UPDATE provider_attempts SET last_error='链接 https://cdn.example.invalid/x.glb已过期，请重试'` → `backup` → 快照内该列被替换为 `链接 （临时供应商地址已脱敏：host=cdn.example.invalid；sha256=…），请重试` → **"已过期"三字消失**（URL 后紧邻中文，中间无分隔符）。对照样例 `…（…）后重试`、`…；以及 …` 正常保留（因为 `）`/`；` 在停字符集里）。
- 期望与实际：期望只替换 URL 本身；实际 `redaction::redact_urls_in_text` 的右向扫描（`crates/server/src/redaction.rs:110-180`）只把空白/引号/括号/中文标点当停字符，**不含汉字与 ASCII 字母**，故 URL 后紧邻的非 URL 字符被并入 URL 段后一起丢弃。
- 影响：备份快照的自由文本（`job_stages.last_error`/`provider_attempts.last_error`）可能丢字；源库不动、结构化事实（task_id/状态/计费）不受影响。当前程序自产消息多在 URL 后紧跟分隔符，实际暴露低；若未来有字段把用户可见文本放这两张表，影响会升级（届时按 P3 处理）。
- 证据路径：`artifacts/web-mvp/bug008-qa/qa-r26-text-swallow.log`
- 回归范围：`--test backup_restore`（新增/更新的历史脱敏用例）、`--lib` 的 `redaction` 单测。
- RD 修复摘要引用：无（本轮新开）。建议修法：右边界按 RFC 3986 允许字符集（`A-Za-z0-9-._~:/?#[]@!$&'()*+,;=%`）消费，其余字符一律结束 URL 段。

## T18-4 偶发裁定：**测试口径脆弱（非产品泄漏趋势）**；QA 已改口径、阈值不变

- 事实（QA 自己收集的数据集，不采信单一结论）：
  - **旧口径**（每次采样新建/关闭 CDP 会话、单点首末比、未强制 GC）：仓库内可查 **26 次**运行的首末比分布 **0.94–1.43**；加上 RD 回合 26 那次失败（`24 467 040 → 38 894 004` = **1.59**，`artifacts/web-mvp/t20-rd-fix/playwright-e2e.log:305-316`；协调者转述的 1.527 与原始样本不符，以原始样本为准）= 27 次观测、**0.94–1.59**；QA 本轮旧口径全量一次 = **1.38**。阈值 1.5 **落在观测分布之内** → 结构性偶发。
  - 失败那次运行里，**同一用例的强断言全部通过**：每轮 `modelsAlive=1`、几何/材质/贴图 `alive=1`、每轮真实 `gl.deleteBuffer/deleteTexture > 0`、`contexts ≤ 13`、`contextsLost ≥ 10`、卸载后 `disposed == created` —— 受管资源无泄漏趋势。
  - **20 轮探针**（QA 临时把轮数改 20 后跑一次并回退）：保留堆 23.24 → 27.93 MB，中位数比 1.16；**增量前 10 轮 +3.0 MB、后 10 轮 +1.6 MB（减速/收敛）**，不是线性泄漏。
- 判定：**阈值口径脆弱**（噪声来源 = 未回收垃圾 + 每次采样新建 CDP 会话的自身开销），**没有产品泄漏证据**。
- QA 采取的动作（只改自己写的用例，阈值不变）：`apps/web/tests/e2e/qa-t18-independent.spec.ts` —— 采样改为**复用同一个 CDP 会话**并在采样前**强制 GC**（`HeapProfiler.collectGarbage`，不可用时如实记录并继续），判据改为**保留堆前 3 次 / 后 3 次中位数比**，仍 `< 1.5`；证据 JSON 同时保留原始与保留堆两组样本。新口径实测：隔离 3 次 = **1.09 / 1.09 / 1.09**，改后全量 86/86 一次 = **1.10**；原始样本比 0.98–1.11（同时消除了旧口径的端点噪声）。
- 后续观察：若未来某轮"保留堆中位数比 ≥ 1.3 且受管资源账本保持平坦"，按产品问题升级调查（当前无此证据）。

## 兜底策略评估（备份=过滤 / 导出=失败）：方向正确，2 处需要收口

- **备份=过滤（不失败）**：正确且必要。AC-009 要求"停服后备份成功"，若历史库里的 URL 能让备份失败，用户将失去唯一灾备手段；实测 `backup` 对历史库 exit 0 并**主动报告**处数（`tempUrlsRedacted: 4`）+ manifest `notes` 写明——不是静默改数据。过滤只作用于供应商事实表（按合同只存系统事实），**用户内容实测未被清洗**（`documents.source_url` 保留）。过滤发生在 manifest 哈希之前（快照 sha256 == `manifest.database.sha256`、`SHA256SUMS` 全 OK）→ 不会产生"哈希对不上"的静默损坏。
- **导出=失败（fail-closed）**：正确。导出内容由白名单/生成字段构成，出现 URL 只能说明代码缺陷；实测注入后 500 + 日志 `export_manifest_url_forbidden` + **无 ZIP 产出**，且**不误伤用户内容**（knowledge 里的用户链接仍 200 且随包保留）。与 AC-058 相容（导出≠灾备；失败不阻断恢复）。
- **需要收口的两点**：① **BUG-010**（文本兜底右向吞字 = 过滤路径的误伤，已单列）；② **OB-9**（`redact_url_query` 形态的错误文本会保留 `scheme://host/path?[redacted]`，在"严格 `://`"口径下会命中——见观察）。另外**过滤作用域仍是"两张表"**：当前代码没有其它地方把供应商 URL 写进别的表（本轮全库扫描 0 命中），但这是"清单式"而非"结构性"保证，建议 T21 的安全矩阵沿用本轮的全库逐表扫描脚本（`work-newdata/scan-phase1.txt` 同款）。

## 非阻断观察（P3/P4；供 RD/PM/T21/T22）

1. **OB-7 · `manual_ai` 未配置时仍会向默认公共端点发请求（QA 脚本首版踩到）**：把 `[providers.manual_ai]` 的 key 配好但 `base_url` 缺省（= `https://api.openai.com/v1`）时，`manual_extract` 会真实外发一次（`manual_extract_sent`），随后以 401/404 失败并按退避重试。**这不是产品缺陷**（部署者自己配了 key），但**验收脚本必须把两个 provider 的 `base_url` 都指向本机**；本轮已在脚本中固化"非回环主机数=0"断言。建议 T21/T23 的脚本沿用该断言。
2. **OB-8 · 备份快照审计后的 `.sqlite3` 直接复制无效（WAL）**：服务运行中只复制主文件会丢掉 `-wal` 里的已提交数据（QA 本轮 `work-newdata/manual.sqlite3` 因此为空表可见）。做现场留证要么 `kill -9` 后一并复制 `-wal`，要么用 `.dump`/查询输出（本轮以 `scan-phase1/2.txt` + 实时查询为准）。
3. **OB-9 · 错误文本的"URL 形态"口径需要 PM/RD 定标**：`TripoError::redacted()` 与 `redact_url_query` 保留 `scheme://host/path?[redacted]`（无签名、无可用参数）。它**不含能力凭据**，但与 ADR-032/任务书里"不得出现 `://`"的字面判据冲突。建议 T21 明确"允许的错误文本形态"（例如允许 provider API 端点 + `?[redacted]`，或统一改为"host + 摘要"），避免后续验收各说各话。
4. **OB-1/OB-2/OB-3/OB-4/OB-5/OB-6（沿用回合 25）**：退出码 5 文档订正；`备份中的 blob 的` 文案；空间预检；GLB `application/octet-stream`；证据目录共享（本回合全量 e2e 仍重写 `t16-*/t17-*/t18-*/t19-*` 与 `t09-rd/playwright-output/.last-run.json`，后者被 Playwright 删除/重建，属既有噪声）；孤儿 `serve` 进程提示。
5. **OB-10 · UI-060（网页导出入口）仍未实现**（回合 25 登记项）：本回合未新增前端代码，导出仍走 HTTP/`curl` 验收；建议随 T21/T22 或专派前端小卡交付。

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实 Provider / 真实费用 / 真实 CDN 域名与签名形态**：全部本机 fixture；T23 授权后另验（含 `GET /tasks/{id}` 频次上限——ADR-032 已登记）。
2. **BUG-009 的日志侧与传输侧未在 dist 二进制上实测**：本机无法让 release 版下载器对一个"允许域 + 公网 IP"的目标产生连接失败/超时（无外网依赖原则），故传输失败消息的**产生**在测试构建验证（同一代码路径），**日志格式（`log_summary`）与 DTO 透传**在真实二进制上验证（注入观察 + 代码路径）。修复后应补一条端到端。
3. **20 轮堆探针**为临时改动（已回退），不是每轮运行的固定检查。
4. **修复前二进制的真实回滚演练、跨平台（musl）、ZIP 导入**：沿用回合 25 的未覆盖清单，未变。
5. **全项目**：T21–T23 未交付，**MVP 尚未完成**；本轮的 CLOSED/FAIL 均只针对 BUG-008 复验范围。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 24 | 2026-09-12/13 | slice | T19 | 2（ui_revision 2） | PASS | 本文件第 24 节 |
| 25 | 2026-09-13 | slice | T20 | 2（ui_revision 2） | PASS（BUG-008 P3 OPEN 需 PM 解释） | 本文件第 25 节 |
| 26 | 2026-09-13 | 缺陷复验 | BUG-008 | 2（ui_revision 2） | **BUG-008 CLOSED；T20 切片 PASS 不受影响；本轮 FAIL（新增 BUG-009 P2 OPEN）** | 本节 |

- **交接给协调者**：(a) `qa_round: 26`、本轮 `qa_result: FAIL`（原因 = BUG-009 未关闭；BUG-008 已 CLOSED）；(b) `qa_history` 追加 `{round: 26, scope: defect-reverify, task_ids: [BUG-008], prd_revision: 2, result: FAIL}`；(c) `open_defects` 更新：**BUG-008 → CLOSED**（保留记录），新增 **BUG-009 (P2, OPEN)**、**BUG-010 (P4, OPEN)**；(d) **T20 仍为 `accepted`（AC-009/AC-010/AC-058 本轮复验仍全部通过）**，BUG-009 不影响 T20 的 AC 结论，但它落在 T13/T17 的下载与展示路径上；(e) `carry_over` 建议新增：BUG-009、BUG-010、OB-7（脚本级外网门禁）、OB-8（WAL 留证）、OB-9（错误文本 URL 口径）、OB-10（UI-060 仍未实现）。
- **交接给 RD**：**必修 BUG-009**（P2）：把 `classify_transport` 的 `detail` 收敛为不含 URL 的文本（或对 detail 走脱敏），并在 `http::jobs` DTO 对 `last_error` 也做读取侧脱敏；修复后把 `crates/server/tests/qa_t20_bug008_independent.rs` 的 2 个 `#[ignore]` 去掉转正。**建议修 BUG-010**（P4）：`redact_urls_in_text` 右边界按 RFC 3986 字符集收敛。
- **交接给 PM**：BUG-008 按"脱敏"裁定已落地并复验通过（无需再解释）；请裁定 **OB-9** 的口径（错误文本里允许出现的 URL 形态），并确认 UI-060 的排期。
- **全项目状态提醒**：T01–T20 已验收；T21–T23 未交付，**MVP 尚未完成**；完整发布仍需 T21 全量回归与安全矩阵（建议把"失败路径的签名 URL"纳入 AC-065/066 的用例集）、T22 多平台冷启动 smoke、T23 授权后的真实 Provider 验证。
- **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「BUG-008 复验验收知识（QA 回合 26）」（失败路径必须单独验、reqwest Display 带 URL、三形态传输失败的最小构造、文本兜底吞字、导出 fail-closed 与误伤双向手法、restore 现场的重查做法、OB-7/OB-8）。`llmdoc/validation-release.md` 命令合同**无变化**（T18-4 的判据口径属 QA 自有测试，已在报告说明）。本报告即验收依据、缺陷与未覆盖边界的正式记录。
- **QA 产出与证据**：`artifacts/web-mvp/bug008-qa/`——`qa-regression-chain.sh`、`qa-r26-final-regression.sh`、`qa-r26-newdata-requery.sh`（+`qa-r26-tripo-fixture.py` 补丁副本）、`qa-r26-historical-backup.sh`、`qa-r26-export-userurl.sh`、`qa-r26-text-swallow.sh`、`qa-r26-lasterror-exposure.sh`、同名 `.log`、`work-newdata/`（扫描输出、fixture 日志、serve 日志）、`work-historical/`（指纹对比、快照扫描、导出扫描、最后错误观察）、`qa-dist-sha256*.txt`、`qa-t18-4-r26-new-metric.log`。**本回合全部临时目录（`/tmp/em-r26-*`）已清理；本回合启动的进程已结束（`pgrep` 复核）。**

# 回合 27 · BUG-009 / BUG-010 复验（失败路径收口、文本边界、备份兜底）

结果：**BUG-009 → CLOSED；BUG-010 → CLOSED；本轮整体 PASS**（两个缺陷复验通过；另新开 **BUG-011（P3，OPEN，非阻断）** 与 **BUG-012（P3，OPEN，非阻断）**——均为回合 26 未覆盖的**相邻路径**，非本轮修复引入，详见缺陷节）。回合：27 · PRD 修订：2（ui_revision 2）· 范围：缺陷复验（BUG-009 / BUG-010）+ 回归

**QA 独立性声明**：不采信 RD 结论，不采信 RD 脚本输出。本回合全部结论来自 QA 现场执行：dist 由 QA 用 `cargo xtask dist` **自己重建**（哈希与 RD 记录一致）；缺陷复现/回归用例由 QA 自写并自跑；dist 级剧本（注入 → 备份 → 恢复 → DTO → 日志 → 零外网门禁）为 QA 自建；RD 新增测试由 QA 全量复跑，仅作对照。

## 环境与交付版本

- 工作树：`313849611d190e93b65a84cd13dba1b3ff784477` + 未提交工作树（RD R28 修复 + 既有改动）。**QA 未改任何生产代码**：只改自己写的两个 QA 测试文件——`crates/server/tests/qa_t13_independent.rs`（新增 4 个回合 27 用例）、`crates/server/tests/qa_t20_bug008_independent.rs`（**只去掉 2 个 `#[ignore]`，断言逐字未改**；见"复现用例转正"节），并新增 `artifacts/web-mvp/bug009-qa/**`。
- 工具链：rustc/cargo 1.98.1、Node v26.0.0、Playwright（Chromium，全量 86 用例）；macOS Darwin 25.6.0 arm64；系统 `sqlite3 3.51.0`、`python3 3.14.2`、`shasum`。
- 二进制：`cargo xtask dist --target aarch64-apple-darwin` **QA 自己重建** = sha256 `15f4f4d33d8aa7de11d56675d6b3ee8c0a897de8602af0897f2541bae8e20edc`（25 103 632 B），与 RD §R28 记录**一致**（`qa-r27-xtask-dist.log`、`qa-dist-sha256-before/after-rebuild.txt`）。
- 数据：dist 级剧本用 `artifacts/web-mvp/t20-rd/sample-backup/`（修复前二进制产生的归档样例）restore 回来**再由 QA 自注入**修复前形态的签名 URL；测试构建用 QA 回合 15 自建 fixture + QA 回合 27 新建的本机 fixture。
- **fixture 与真实 Provider 分开列**：本回合 **100% 本机 fixture（`127.0.0.1`）、零付费、零外网**；dist 剧本带"serve 日志里非回环主机数 = 0"门禁（通过）。真实 Provider / 真实 CDN 属 T23，未验。

## QA 判据（沿用回合 26 判据表，未放宽、未收紧）

| 判据 | 命中即视为泄露 |
| --- | --- |
| `://`（唯一例外：部署自己的监听横幅 / `provider baseUrl` 回显） | 是 |
| `sign=`（及任何签名查询串） | 是 |
| URL path（`/qa-r27/…`） | 是 |
| 每路径唯一 canary 串 | 是 |
| 裸 host（`cdn.qa-r27.invalid`） | 否（最小诊断） |
| `（临时供应商地址已脱敏：host=…；sha256=…）` 摘要标签 / `{"redacted":true,…}` 对象 | 否（允许保留的不可逆摘要） |
| task_id、状态码、计数、计费、结论文案 | 否（必须保留） |

## 命令与原始结果（QA 现场执行；日志在 `artifacts/web-mvp/bug009-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test qa_t20_bug008_independent -- --ignored --nocapture`（回合 26 复现用例，**修复前 RED**） | exit 0，**2 passed / 0 failed**；消息 = `模型下载传输失败（下载可安全重试；不产生供应商费用）：连接失败：error sending request`（无 `://`、无签名）→ `qa-r27-repro-ignored.log` |
| 2 | `cargo test -p everything-manual --test qa_t20_bug008_independent`（**转正后默认**） | exit 0，**3 passed / 0 failed / 0 ignored**（30.0s，含半关闭对照） |
| 3 | `cargo test -p everything-manual --test qa_t13_independent qa_bug`（QA 回合 27 新用例） | exit 0，**3 passed / 1 ignored**（ignored = BUG-012 RED 用例）→ `qa-r27-newdata-chain.log` |
| 4 | `cargo test -p everything-manual --test qa_t13_independent qa_bug012 -- --ignored --nocapture`（BUG-012 复现） | **exit 101 / 1 failed**：`提供方文本里的签名落库：[("job_stages","usage_json",1)]` → `qa-r27-bug012-red.log` |
| 5 | `cargo test --workspace` | exit 0，**539 passed / 0 failed / 5 ignored**（35 条 test result；基线 524 → RD R28 534 → QA 本轮 +5）→ `qa-r27-cargo-test-workspace.log` |
| 6 | `cargo xtask check` | exit 0，**7/7 [通过]**（fmt/clippy/单测/前端 lint/typecheck/vitest/合同）→ `qa-r27-xtask-check.log` |
| 7 | `cargo xtask contracts --check` | exit 0，两份 `[一致]`；生成物哈希前后不变 → `qa-r27-contracts-check.log`、`qa-contracts-hashes-*.txt` |
| 8 | `cargo xtask dist --target aarch64-apple-darwin`（QA 重建） | exit 0，sha256 `15f4f4d3…`（与 RD 记录一致）→ `qa-r27-xtask-dist.log` |
| 9 | `cargo xtask smoke-bootstrap --binary <dist>` | exit 0，7×`[检查]` → `qa-r27-smoke-bootstrap.log` |
| 10 | `npm --prefix apps/web run test:e2e`（全量） | exit 0，**86 passed / 0 failed / 0 skipped（7.2 m）** → `qa-r27-e2e-full.log` |
| 11 | `bash qa-r27-redaction-dist.sh <dist>`（QA 自建；35 项断言；含备份、DTO、导出、日志与零外网门禁） | exit 0，**35 通过 / 0 失败** → `qa-r27-redaction-dist.log` |
| 12 | `bash qa-r27-bug011-evidence.sh <dist>`（BUG-011 现场） | 再现：源库 `needs_input_json` 完整 → 快照为摘要对象 → 恢复库 `needsInput` 条数 **0** → `qa-r27-bug011-evidence.log` |
| 13 | `--test redaction_persistence` / `--lib redaction` / `--lib http::jobs` / `--test model_assets` / `--test manual_ai_contract` / `--test tripo_contract` / `--test pipeline` / `--test backup_restore` / `--test jobs_recovery`（RD 用例复跑对照） | 4 / 7 / 2 / 24 / 19 / 18 / 13 / 9+1i / 25 passed，0 failed |

## BUG-009 复验矩阵（4 条独立路径；每条用 QA 自己的 canary；路径号 ≈ 任务书建议）

| 路径 | 现场构造（QA 自建） | 实测 | 结论 |
| --- | --- | --- | --- |
| ① **传输错误**（连接被拒 + 黑障超时）· 消息侧 | `qa_t20_bug008_independent.rs`：死端口 / 收下请求不响应（1s 超时） | 消息 = `…：连接失败：error sending request` / `…：超时：error sending request`；无 `://`、无 `qa26-canary-transport-signature-7d31` | **PASS** |
| ② **传输错误**经**真实处理器**落库 + DTO + 重试用尽 | `qa_bug009_transport_failure_never_persists_signed_url`：死端口作模型 CDN，跑真执行器 | `model_download.last_error`（retry_wait 与 failed 两形态）无 `://`/canary；**全库逐表逐列扫描 0 命中**；任务详情 DTO 0 命中；付费提交 1 次 | **PASS** |
| ③ **HTTP 报错**（403 过期 → 续签 → 500）+ 恢复语义 | `qa_bug009_http_error_path_after_refresh_never_persists_signed_url`：两条链接各带 canary | `last_error` = `模型 CDN 返回服务器错误（HTTP 500）：下载可安全重试（不产生费用）`；两条 canary 全库 0 命中；`GET /v3/tasks/` ≥2 次、`POST …/multiview-to-model` 恒 1 次；CDN 请求无 Authorization | **PASS** |
| ④ **提供方文本**（说明书 AI refusal 内嵌签名）· 展示侧 | `qa_bug009_provider_text_url_never_shows_in_job_detail`：refusal 原文含 `?sign=` | 任务详情响应体 0 命中；`needsInput[].message` 保留摘要标签与结论文案；`needs_input_json` 写入侧已脱敏 | **PASS**（**落库侧未收口 → BUG-012**） |
| ⑤ **备份快照 + 历史行现场 + 导出 + 日志**（dist 二进制） | `qa-r27-redaction-dist.sh`：restore 归档样例 → 注入 stage/attempt/needs 三处 5 个 canary（含 BUG-010 边界形态）→ backup → serve → DTO → 导出发布包 | 快照 0 命中（8 判据 × 全列）；快照 db sha256 == `manifest.database.sha256`、`SHA256SUMS` 全 OK；源库 dump sha256 前后一致（未就地迁移）；DTO 8 判据 0 命中；导出 200 + ZIP magic + 包内 6 判据 0 命中；serve 日志 canary/`sign=`/`cdn.qa-r27.invalid` **0 命中**、`://` 只来自监听横幅/baseUrl、非回环主机数 = 0 | **PASS** |

## BUG-010 复验矩阵（URL 右边界）

| 用例 | 实测（快照 + DTO 两处） | 结论 |
| --- | --- | --- |
| `…?sign=x已过期，请重试`（回合 26 原始形态） | `…（临时供应商地址已脱敏：host=…；sha256=…）已过期，请重试` — **"已过期，请重试" 全保留** | **PASS** |
| `见 '…?sign=…' 后重试`（`'` 边界） | `'` 两侧都保留 → `见 '（…）' 后重试` | **PASS** |
| URL 后**换行 + 中文标点**（`…?sign=…：以及结束`） | `：以及结束` 保留（换行未吞） | **PASS** |
| URL 后**全角逗号**（`…?sign=…，结束`） | `，结束` 保留 | **PASS** |
| 无查询串 URL 后被 `）`/引号/句号包围；`#frag`；百分号编码 | RD 单测（`text_redaction_keeps_adjacent_non_url_text`）QA 复跑通过 | **PASS** |
| **已知限制**：URL 后**紧邻 ASCII 字母**（`…?sign=xexpired`） | 按 URL 语法仍并入 URL 段（`is_url_tail_char` 含字母）——ADR-033 已如实记录；QA 核对：当前**不存在**产品文案把 ASCII 文本直接贴在 URL 后的形态（传输错误已 `without_url`；其余文案 URL 后均为分隔符） | 记录，不判缺陷 |

## 覆盖性核对结论（任务书第 2 点：统一入口是否真的收口）

**持久化（写侧）**：`job_stages.last_error` 的**全部**写入点（`advance` 的 RetryWait/NeedsInput/SubmissionUnknown/Failed/Defer 变体 + `reset_for_retry`/`requeue_succeeded_dependents`/`apply_reconcile_resolution`）与 `provider_attempts.last_error` 的三个写入函数（`mark_unknown`/`mark_failed`/`set_last_error`）均已调用 `redact_text_urls`；`needs_input_json` 在 `advance(NeedsInput)` 与 `apply_reconcile_resolution` 均已脱敏。QA 用 `grep` 枚举了全部 `INSERT/UPDATE job_stages|provider_attempts` 语句：**不存在**仓储层之外的直接 SQL 写入口。

**对外（读取侧）**：任务详情 stage `lastError`、attempt `lastError`、`needs_input[].message` 均走统一入口（覆盖历史行）；`usage` 走 `redact_urls_in_json`；**未发现第三个展示点**（`/jobs` 列表 DTO 不含 lastError/usage；`releases` DTO 无相关字段）。日志侧：`model_download_failed` 的 `detail` = `log_summary()`（已干净），错误类别、`errorCode`、`jobId/stageId` 与 **`host` 字段**保留。

**未收口点（本回合新发现，两条）**：
1. **`job_stages.usage_json` 不在任何写侧脱敏范围**——`manual_extract` 把提供方文本（`failure_of` 的摘要 → `errorSummary`）原样写进结果事实 → **BUG-012**（`usage_json` 是 JSON 列，`advance`/`set_result_fact` 都不脱敏；R28 只收了 `last_error`/`needs_input_json`）。
2. **备份 JSON 列的兜底语义与文本列不同**——`redact_urls_in_json` 的判据是"整串含 `://` → 整串替换为摘要对象"，对 `needs_input_json` 的**句子型 message** 属过度替换 → **BUG-011**（句子丢失 + 恢复后 `needsInput` 静默为空）。**ADR-033 §2 的"④ 备份兜底沿用同一函数"只对非 JSON 列成立**。
   > 另：`envelope_error`/`incomplete_reason`/`refusal` 同为提供方文本，走同一 `errorSummary` 写法（BUG-012 的同类入口）。

**reqwest 文本策略**：全仓 `reqwest::Error` 只有 3 个分类点（`assets::glb::download::classify_transport`、`providers::tripo::client::classify_transport_error`、`providers::manual_ai::client::classify_transport_error`），**均已** `without_url()` + 统一入口；`format!("{error}")` 直拼 reqwest 错误的写法已无残留。Tripo/ManualAI 的 `redact_text`/`business_summary` 收敛到统一入口（供应商 `message`/`suggestion` 也走同一函数）。

## 诊断能力与恢复语义（未退化）

- **保留**：稳定错误码（`download_transport`/`download_http_status`…）、错误类别（连接失败/超时/响应体读取中断）、HTTP 状态码（`HTTP 500`）、"下载可安全重试；链接过期按 task_id 重查（不重新购买）"结论文案、任务 ID、host（`model_download_failed` 的 `host` 字段 + 摘要标签 `host=…`）、不可逆 sha256 摘要（16 位）。
- **"链接过期 → 按 task_id 重查"回归**：路径 ③ 实测——403 后 `GET /v3/tasks/{id}` 增加、付费 `POST` 恒为 1；`--test model_assets`（含 `restarted_download_requeries_by_task_id_and_never_repurchases`）与 `--test qa_t13_independent` 的既有重查用例全绿。
- **口径说明（如实记录）**：`without_url()` 路径的传输失败消息**正文里没有 host**（URL 已被整体去掉，没有可做摘要的片段）；host 只在日志事件字段里。与 ADR-033 "保留目标 host（日志字段 + 摘要标签）"一致，但**日志字段为静态核对**（dist 无法制造下载失败，见未覆盖边界）。

## ADR-033 口径核对（与 QA 回合 25/26 判据一致性）

| ADR-033 条目 | 实现核对 | 与 QA 判据关系 |
| --- | --- | --- |
| 需脱敏 = 文本中 `scheme://…`（无论有无查询串） | `is_url_like` = 含 `://`；`redact_urls_in_text` 以 `://` 为锚点整段替换 | **一致**（QA 回合 26 判据表把 `://` 列为泄露；ADR-033 取严格侧，正是 QA 建议的方向） |
| 裸 host 不算、允许保留 | 摘要标签/日志字段保留 host；DTO 里 `host=cdn.qa-r27.invalid`、`"host":"127.0.0.1"` 实测仍在 | **一致** |
| 唯一例外 = 部署者自有 baseUrl 回显（`redact_url_query`） | `redact_url_query` 全仓只被 config/commands 与 providers/mod 的 baseUrl 回显调用；dist serve 日志 `://` 仅来自监听横幅/baseUrl（实测计数相等） | **一致**（QA 例外清单同款） |
| 统一入口 + 四处收口 | 写侧/读取侧/备份文本列/错误构造均已收口；**但** `usage_json`（BUG-012）与备份 JSON 列（BUG-011）未覆盖 | **部分不成立**，见缺陷 |
| `'` 不消费（有意偏离 RFC） | 实测 `'` 两侧保留 | **接受**（自由文本里更像引号；无产品形态受影响） |
| 已知限制：URL 后紧邻 ASCII 字母 | 与实现一致 | **接受**（记录不隐藏；产品无此形态） |

**结论：ADR-033 的口径与 QA 回合 25/26 的判据无冲突，QA 接受严格侧（错误文本不留任何 `scheme://…`）。** 需要 PM/RD 注意的只是上表"四处收口"的**范围声明**：`usage_json` 与备份 JSON 列不在其中（BUG-011/BUG-012）。

## 复现用例转正（`qa_t20_bug008_independent.rs`）

- **RD 未改该文件**（QA 判据）：文件 mtime `9月13 02:39` **早于** RD 本轮第一个产物（`repro-before-qa-t20-ignored.log` 02:55）；RD 修复前后日志里的 panic 行号（`qa_t20_bug008_independent.rs:192:5`）与本文件断言行一致；用例名、canary、消息形态与回合 26 报告完全一致 → **未改动**。
- **QA 决定转正**：去掉 2 个 `#[ignore]`（`qa26_connect_refused_message_must_not_leak_signed_url`、`qa26_timeout_message_must_not_leak_signed_url`），**断言逐字未改**，只更新注释说明"回合 26 RED → 回合 27 转正"。理由：修复后长期回归价值高（其他形态无法替代：只测半关闭会误判"无泄露"）；代价 = 默认套件 +30s（黑障线程 join）。
- 转正后默认入口：3 passed / 0 failed / 0 ignored（命令 2）。

## 缺陷

### BUG-009 · 模型下载的传输类失败把完整签名 URL 写进 `job_stages.last_error`，并经任务详情 API 与日志输出（回合 26 原文见上）

- 严重度／状态：P2 → **CLOSED**（2026-09-13 · 回合 27 复验通过）
- 对应 REQ / UI / AC：contracts §1；REQ-043/REQ-044、AC-065/AC-066
- 复验方式：4 条独立路径（传输错误消息、真实处理器落库+DTO+重试用尽、HTTP 报错+续签、提供方文本展示侧）+ dist 级历史行/备份/日志；判据见"QA 判据"表，脚本可复跑：`qa-r27-redaction-dist.sh`、`qa_t20_bug008_independent.rs`、`qa_t13_independent.rs::qa_bug009_*`
- 修复核对：`classify_transport` = `without_url()` + `redact_text_urls`；读取侧 `lastError`/`needs_input` 兜底；写侧仓储全变体收口；日志 `detail` 同源。**未发现该缺陷原形态的残留**。
- 遗留（不属本缺陷，另立）：**BUG-012**（`usage_json` 写侧未收口）、**BUG-011**（备份 JSON 列过度替换）。

### BUG-010 · 备份"非 JSON 文本列"兜底脱敏右向吞字（回合 26 原文见上）

- 严重度／状态：P4 → **CLOSED**（2026-09-13 · 回合 27 复验通过）
- 对应 REQ / UI / AC：AC-010（备份内容语义）、`implementation.md` §T20-13 第 5 条
- 复验方式：dist 注入 4 个边界形态 → 快照与 DTO 两处逐片段核对（原始形态 `?sign=x已过期，请重试` 全保留；`'`/换行/全角标点/句号均不吞）；RD 单测复跑通过；脚本 `qa-r27-redaction-dist.sh` 第 4/6 节
- 遗留：**JSON 列**的对应问题（整串对象替换）不属本缺陷，另立 **BUG-011**。

### BUG-011 · 备份快照把 `needs_input_json` 的整条 message 替换为摘要对象 → 恢复后该阶段 `needsInput` 静默为空（句子丢失）

- 严重度／状态：**P3 / OPEN（非阻断；理由见下）**
- 对应 REQ / UI / AC：AC-010（备份内容语义）、`implementation.md` §T20-13 第 5 条兜底；REQ-024/AC-036（needs_input 必须"列出可行动缺项"）在**恢复产物**上的语义
- 环境与输入：dist `15f4f4d3…`；`bash artifacts/web-mvp/bug009-qa/qa-r27-bug011-evidence.sh <dist>`
- 复现步骤：`restore` 归档样例 → 注入**产品可达**形态：`job_stages.status='needs_input'`，`needs_input_json = [{"code":"download_insecure_scheme","message":"模型下载必须使用 HTTPS（实际 http://cdn.qa27.invalid/m.glb）：拒绝下载"}]`（`DownloadError::InsecureScheme` 的 message 含 `scheme://`）→ `backup` → `restore` 新目录 → `serve` → `GET /jobs/{id}`
- 期望与实际：期望快照只去掉 URL 片段、保留句子与 JSON 形状；实际快照 = `[{"code":"download_insecure_scheme","message":{"redacted":true,"sha256":"d551df…"}}]`（**整条 message 变对象**），恢复库 `stage.status='needs_input'` 但 **`needsInput = []`**（`stage_dto` 反序列化失败被 `.ok()` + `unwrap_or_default()` 静默吞掉，`http/jobs.rs:772-782`）。影响：**灾备恢复后管理员看不到任何缺项**（连 code 都不显示）；同一阶段里**无 URL 的其它条目也一起消失**（整列解析失败）。
- 证据路径：`artifacts/web-mvp/bug009-qa/qa-r27-bug011-evidence.sh` + `.log`、`qa-r27-redaction-dist.log`（第 3/6 节）
- 根因：`crates/server/src/backup/create.rs`（`redact_temp_urls_in_column`）对**可解析为 JSON 的列**走 `redact_urls_in_json`，而该函数判据是"字符串**含** `://` → 整串替换为摘要对象"（为 `usage_json` 的纯 URL 值设计）；`needs_input_json` 是句子型 JSON。
- 回归范围：`--test backup_restore`、`--lib redaction`、`qa-r27-redaction-dist.sh`、`qa-r27-bug011-evidence.sh`；建议同时覆盖 `usage_json` 里出现"句子含 URL"的未来形态。
- 建议修法（任选，QA 不指定实现）：① 备份对 `needs_input_json` 走**文本级**脱敏（`redact_urls_in_text`，保留句子、JSON 仍合法）；或 ② `redact_urls_in_json` 增加"字符串含 URL 且非纯 URL → 只替换 URL 段"的形态；③ 顺手让 `stage_dto` 的反序列化失败**如实记录**（不静默空列表）。
- 严重度理由（为何非阻断）：源库与快照行仍在（非不可逆）、只影响恢复产物、触发需缺项消息含 `://`（当前产品可达形态仅 `download_insecure_scheme`）、无安全/费用影响；但**灾备路径的静默信息丢失**应尽快修。

### BUG-012 · 提供方文本里的签名 URL 落库到 `job_stages.usage_json`（写侧未收口）

- 严重度／状态：**P3 / OPEN（非阻断）**
- 对应 REQ / UI / AC：ADR-032 第 1 条（临时 URL "永不落库"）；contracts §1；AC-066 同源
- 环境与输入：测试构建（`cargo test`，同一生产代码路径）；本机 fixture 返回 refusal 文本 `…下载参考 https://cdn.qa27.invalid/p.png?sign=qa27-manual-canary-5e42 也失败`
- 复现步骤：`cargo test -p everything-manual --test qa_t13_independent qa_bug012 -- --ignored --nocapture`（RED，exit 101）
- 期望与实际：期望 canary 不落库到任何表/列；实际 `job_stages.usage_json` **1 处命中**（`errorSummary` 字段）。展示/备份/导出侧仍被兜住（DTO `usage` 与备份 JSON 列整串替换、导出白名单不含该字段）→ **暴露面 = data-dir 库文件本身**（URL 是有时效的 CDN 能力）。
- 证据路径：`artifacts/web-mvp/bug009-qa/qa-r27-bug012-red.log`、`crates/server/tests/qa_t13_independent.rs::qa_bug012_provider_text_signed_url_must_not_be_persisted_into_usage_json`（`#[ignore]`，RED）
- 根因：`providers/manual_ai/handlers.rs:752-772`（`failure_of` → `BatchExtractionResult::not_produced(error_summary)`）与 `:871`（`errorSummary` 写进 usage）使用**未脱敏**的提供方文本；`storage/repo/job_stages.rs` 的 `advance(Succeeded)`/`set_result_fact` **不对 `usage_json` 脱敏**（R28 只覆盖 `last_error`/`needs_input_json`）。
- 回归范围：`--test manual_ai_contract`、`--test pipeline`、`--test qa_t13_independent`（修好后转正上条 `#[ignore]` 用例）。
- 建议修法：结果事实写入前对 usage（或至少提供方文本字段）走统一入口；并补"提供方文本含 URL 不落库"的用例。
- 严重度理由（为何非阻断）：无 API/备份/导出外泄（三层均兜住）、无付费影响；但违反 ADR-032 的"永不落库"明文。

## 非阻断观察（P3/P4；供 RD/PM/T21/T22）

1. **OB-11 · 提供方文本的同类入口**（建议与 BUG-012 一并收口）：`manual_ai` 的 `envelope_error`（`响应信封不可解析…{detail}`）与 `incomplete_reason`、`refusal` 都经同一 `errorSummary` 写法进入 `usage_json`；修 BUG-012 时按"提供方文本一律先脱敏"处理更稳。
2. **OB-12 · `is_url_like` 的语义分叉**（BUG-011 的根因说明）：JSON 路径是"整串含 `://` → 整串替换"，文本路径是"URL 片段 → 标签"。两套语义共存容易被后续字段误用；建议在 `redaction.rs` 模块文档里显式写明两条规则的适用面（本次未改代码，仅登记）。
3. **OB-1/OB-2/OB-3/OB-4/OB-5/OB-6（沿用回合 25/26）**：退出码 5 文档订正；`备份中的 blob 的` 文案；空间预检；GLB `application/octet-stream`；证据目录共享（本回合全量 e2e 仍重写 `t16-*/t17-*/t18-*/t19-*` 与 `t09-rd/playwright-output/`，属既有噪声）；孤儿 `serve` 进程提示（本回合复核：环境里仍有 **00:18:10 起的孤儿 `serve`**——`--data-dir …/em-auth-t20-restore-e2e-cli-14140-…/restored-data`，**非本回合所启**，QA 未杀它）。
4. **OB-10 · UI-060（网页导出入口）仍未实现**（沿用回合 25）：本回合未新增前端代码；建议随 T21/T22 或专派前端小卡交付。
5. **OB-7（脚本级外网门禁）/ OB-8（WAL 留证）** 已在本回合脚本中固化（dist 剧本"非回环主机数 = 0"；留证用 `.dump`）。

## 未覆盖边界（本回合记录，不冒充通过）

1. **真实 Provider / 真实费用 / 真实 CDN 域名与签名形态**：全部本机 fixture；T23 授权后另验。
2. **dist（release）二进制上的"传输失败产生侧"不可测**（`allow_local_fixture` 只在测试构建生效）：产生侧在测试构建验证（同一代码路径）；**日志事件 `model_download_failed` 的 `host` 字段为静态核对**，未在运行时抓取（与 RD §R28-7 第 1 条一致）。
3. **BUG-011 的"同阶段其它无 URL 条目一起消失"**：由整列解析失败推出，未逐条构造多条目现场（单条目现场已实测）。
4. **`usage_json` 里"句子含 URL"的其它未来形态**：只验证了当前产品可达的 manual_ai 路径。
5. **修复前二进制的真实回滚演练、跨平台（musl）、ZIP 导入**：沿用回合 25 的未覆盖清单，未变。
6. **全项目**：T21–T23 未交付，**MVP 尚未完成**；本轮的 CLOSED/新缺陷均只针对本次复验范围。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 25 | 2026-09-13 | slice | T20 | 2（ui_revision 2） | PASS（BUG-008 P3 OPEN 需 PM 解释） | 本文件第 25 节 |
| 26 | 2026-09-13 | 缺陷复验 | BUG-008 | 2（ui_revision 2） | BUG-008 CLOSED；本轮 FAIL（新增 BUG-009 P2 OPEN） | 本文件第 26 节 |
| 27 | 2026-09-13 | 缺陷复验 | BUG-009 / BUG-010 | 2（ui_revision 2） | **BUG-009 CLOSED、BUG-010 CLOSED；本轮 PASS（新增 2 个 P3 OPEN，非阻断）** | 本节 |

- **交接给协调者**：(a) `qa_round: 27`、本轮 `qa_result: PASS`（BUG-009/BUG-010 复验通过；新缺陷为 P3 非阻断）；(b) `qa_history` 追加 `{round: 27, scope: defect-reverify, task_ids: [BUG-009, BUG-010], prd_revision: 2, result: PASS}`；(c) `open_defects` 更新：**BUG-009 → CLOSED**、**BUG-010 → CLOSED**（保留记录），新增 **BUG-011 (P3, OPEN)**、**BUG-012 (P3, OPEN)**；(d) T20/T13 的既有 AC 结论不受影响（BUG-011 在备份兜底子句的**过度删除**侧、BUG-012 在"永不落库"侧，均需 RD 修复后由 QA 复验）；(e) `carry_over` 新增：BUG-011、BUG-012、OB-11、OB-12（OB-1…OB-10 沿用）。
- **交接给 RD（建议一轮修复）**：**BUG-012**（写侧）：结果事实（`usage_json`）写入前对提供方文本走统一脱敏入口（`manual_ai::handlers::failure_of`/`errorSummary` 与 `job_stages` 写入点二选一，建议前者 + 后者兜底）；**BUG-011**（备份）：`needs_input_json` 的兜底改为文本级替换（或 JSON 替换保留句子），并让 `stage_dto` 反序列化失败如实记录。修好后把 `qa_bug012_*` 的 `#[ignore]` 转正（由 QA 复验后决定）。**不得修改 QA 测试文件本体**。
- **交接给 PM**：ADR-033 口径 QA **接受**（严格侧，无分歧）；若 PM 认为 BUG-011/BUG-012 应升级为阻断或有其它口径（例如允许 `usage_json` 留 URL），请裁定并留痕。
- **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「失败路径脱敏复验验收知识（QA 回合 27）」（JSON 列与文本列的兜底语义分叉、`usage_json` 不在收口清单、`without_url` 后 host 只在日志字段、needs_input 整列解析失败会静默清空、e2e/脚本的口径）；`llmdoc/validation-release.md` 命令合同**无变化**。本报告即验收依据、缺陷与未覆盖边界的正式记录。
- **QA 产出与证据**：`artifacts/web-mvp/bug009-qa/`——`qa-r27-redaction-dist.sh`+`.log`、`qa-r27-bug011-evidence.sh`+`.log`、`qa-r27-repro-ignored.log`、`qa-r27-newdata-chain.log`、`qa-r27-bug012-red.log`、`qa-r27-cargo-test-workspace.log`、`qa-r27-xtask-check.log`、`qa-r27-contracts-check.log`、`qa-contracts-hashes-*.txt`、`qa-r27-xtask-dist.log`、`qa-dist-sha256-*.txt`、`qa-r27-smoke-bootstrap.log`、`qa-r27-e2e-full.log`、`work-dist/`（快照扫描、DTO 视图、serve 日志、快照 dump）。**本回合临时目录（`/tmp/em-r27-*`、`/tmp/qa27-probe*`）已清理；本回合启动的进程已结束（`pgrep` 复核；环境里 00:18:10 起的孤儿 `serve` 非本回合所启）。**

# 回合 28 · BUG-011 / BUG-012 复验（JSON 列结构感知、写侧收口）与结构性保障审计

结果：**BUG-011 → CLOSED；BUG-012 → CLOSED；本轮整体 PASS**（两项 P3 复验通过；RD 声称的"穷举式收口"四项结构性保障逐一**实测**成立；新增 2 条 P4 观察（OB-13/OB-14，非缺陷、非阻断））。回合：28 · PRD 修订：2（ui_revision 2）· 范围：缺陷复验 + 结构性保障审计 + 全量回归

## 环境与交付版本

- 工作树：HEAD `313849611d190e93b65a84cd13dba1b3ff784477` + 未提交工作树（RD R29 修复 + 既有改动）。**QA 未改任何生产代码**：只改自己的 QA 测试文件 `crates/server/tests/qa_t13_independent.rs`（**转正** 1 个 `#[ignore]` 用例——断言逐字未改，只更新注释；**新增** 1 个独立 canary 用例），并新增 `artifacts/web-mvp/bug011-qa/**`。构造性验证的探针文件与"新增列"均在取证后恢复/仅存在于临时目录（`git status` 复核：`crates/` 下无 QA 探针残留，见下）。
- dist：`cargo xtask dist --target aarch64-apple-darwin`（QA 自行重建，04:26）→ sha256 **`f68d14874936b74f70aaa99b0eb8462396e933344a4495653ed239ba8838ac02`**（25 112 080 B）。**与 RD 构建逐位相同**（可复现构建；`qa-r28-xtask-dist.log`）。
- 数据：dist 剧本 = `artifacts/web-mvp/t20-rd/sample-backup/` restore 后由 QA 自注入（canary：`QA28-SENT-CANARY-5a1c`、`QA28-PROBE-CANARY-7b3e`、`QA28-BLOB-CANARY-3d21`）；测试构建 = 本机 fixture（canary `qa28-usage-canary-c0de`）。全程 127.0.0.1、零外网、零付费。
- 未验证：真实 Provider/CDN 形态（T23）；dist 上无法制造传输失败/提供方文本现场（fixture 仅测试构建，产生侧在测试构建验证）——同回合 26/27。

## 判据（沿用回合 26/27 表，未放宽、未收紧）

| 项 | 判据 |
| --- | --- |
| 泄露判据（DB 列/快照/DTO/导出/日志） | `://`、`sign=`、签名 canary、`/qa-r2x/` 路径 **0 命中** |
| 允许保留 | 裸 host、摘要标签 `（临时供应商地址已脱敏：host=…；sha256=…）`、task_id/responseId、状态码、计数、结论文案 |
| JSON 列形态（ADR-034） | 句子型字符串 → 只替换 URL 片段且**保持字符串**；纯 URL 值按列契约（`needs_input_json`→标签字符串；其余事实 JSON→摘要对象）；脱敏后必须仍是合法 JSON |
| 例外 | provider `baseUrl` 回显（serve 横幅/日志，`redact_url_query`）；用户内容（`sourceUrl`/知识 JSON）按合同保留 |
| 既有语义不回退 | needs_input 可读可行动、usage 事实（id/计费/摘要）保留、按 task_id 重查与错误类别保留 |

## BUG-011 复验 · **CLOSED**

- 复验命令与结果：
  1. `bash artifacts/web-mvp/bug009-qa/qa-r27-bug011-evidence.sh <QA 重建 dist>` → exit 0，**恢复库 needsInput 条数 = 1**，快照 message 为字符串且句子保留（`qa-r28-bug011-r27script-final.log`）——回合 27 的原始 RED 现场转绿。
  2. **QA 自建句子保真现场**（`qa-r28-bug011-sentence.sh`，182 项检查全通过）：注入 4 条 message（①句中夹 URL ②无 URL 纯文本 ③句首 URL + 尾部文字 ④**整串 URL**）→ backup → restore 新目录 → GET /jobs/{id}。实测（`snapshot-items.json` / `job-detail.json` 逐字）：
     - ① `模型下载必须使用 HTTPS（实际 （临时供应商地址已脱敏：host=cdn.qa28.invalid；sha256=…））：拒绝下载，请检查来源后重试`——**仅 URL 片段变标签，句子前后逐字保留**；
     - ② 无 URL 条目**逐字未动**（回合 27"同列其它条目一起消失"的未覆盖边界本轮已补测）；
     - ③ 尾部"已过期，请按 task_id 重查（不重新购买）"保留；④ 整串 URL 只变标签字符串（**未变对象**）；
     - 快照 `needs_input_json` **JSON 合法**（parse 通过）、4 条齐全；恢复后 DTO 与快照**逐字一致**且 `status=needs_input`；全库/快照/详情 0 命中判据。
  3. **不静默**：故意注入损坏的 `needs_input_json`（对象而非列表）→ HTTP 仍 200、needsInput 按空列表展示、serve 日志出现 `job_detail_needs_input_parse_failed`（真实记录，不再静默吞掉）。
- 结论依据：①③④ 三种句子形态、多条目、JSON 合法性、恢复链路与静默告警均实测通过 → **CLOSED**。回合 27 未覆盖边界 3（多条目）与本缺陷已闭环。

## BUG-012 复验 · **CLOSED**

- 复验命令与结果：
  1. **转正**：`crates/server/tests/qa_t13_independent.rs::qa_bug012_provider_text_signed_url_must_not_be_persisted_into_usage_json` 去掉 `#[ignore]`（**理由**：修复后其长期回归价值高——全库 canary 扫描覆盖"提供方文本 → 结果事实"整条链路；断言逐字未改，只更新注释）→ 默认套件运行 **ok**（`qa-r28-bug012-promoted.log`）。
  2. **QA 独立 canary 用例**（新增 `qa_r28_usage_json_new_writes_never_contain_url_scheme`，canary `qa28-usage-canary-c0de`）：refusal 真链路 → 全库扫描 0 命中；`usage_json` 整列无 `://`、无签名；`errorSummary` **仍是字符串**、尾部"也失败"保留、含摘要标签；事实字段保留（`outcome=refusal`、`errorCode=manual_ai_refusal`、`responseId`、token 计数、`diagnosticSha256`）→ ok（同日志）。
  3. dist 级：RD 自查脚本（字节相同副本）29/29 通过，含 `usage.errorSummary` 字符串 + 摘要标签 + restore 后 DTO 形态（`qa-r28-rdverify.log`）。
- 结论依据：写侧（产生侧 + 仓储侧双层）+ 读取侧 + 备份侧 + 日志四层实测无签名落库/出网 → **CLOSED**。OB-11（提供方文本同类入口）经代码核对与实测确认已随本修复处置（`failure_of` 统一包装覆盖全部失败分支 + schema 校验 `detail_text()`）。

## 结构性保障审计（本回合重点；回答"明天新增一个供应商错误文本字段会不会漏"）

> RD 四项主张逐一实测。**两个守卫的负向验证**都用"临时投放探针文件 → 期望失败并打印违规点 → 删除恢复"完成，未改任何生产代码（`qa-r28-guard-negative.log` / `qa-r28-guard-restored.log`）。

1. **①事实表 SQL 只在 `storage/repo`（`redaction_surface::supplier_fact_table_writes_live_only_in_repo_layer`）——判别力成立**：临时在 `src/` 投放含 `UPDATE provider_attempts SET last_error = ?` 的文件 → 测试 **FAILED** 并打印 `src/qa_r28_probe_outside.rs:3：…`；删除后恢复 2 passed。RD 的 A 命令等价 grep 复核：输出为空。
2. **②仓储写入函数必脱敏或登记豁免（`write_functions_either_redact_or_are_exempt`）——判别力成立、豁免未过度放宽**：同法投放一个未脱敏的写函数（repo 目录内）→ **FAILED** 并打印 `…/storage/repo/qa_r28_probe_repo.rs::qa_r28_probe_repo_write`；删除后恢复。**抽查 10/10 豁免函数**逐个人工核对函数体：`insert`（page_set/input_hash 属本地计算，三敏感列为 NULL）、`claim_next`/`renew_lease`/`take_over_expired`（状态与租约字段）、`reset_result_fact`（usage_json 置 NULL）、`cancel_unsubmitted_for_job`（状态与租约）、`create_intent`（request_hash 本地；last_error NULL）、`mark_submitting`、`record_remote_task_id`/`record_sync_response`（opaque id，ADR-032 §2 恢复判据）——**理由与函数体一一相符，未发现过度放宽**。非豁免写函数抽查：`advance`（全部文本分支均经统一入口）、`set_result_fact`、`reset_for_retry`/`requeue_succeeded_dependents`/`apply_reconcile_resolution`（reason/note 先脱敏，needs_input_json 由脱敏后 note 构造）。
3. **③备份"表 × 全列"自动兜底（新列免改代码）——实测成立**：不改任何代码，在 dist 现场对源库 `ALTER TABLE job_stages ADD COLUMN` 临时新增 3 个 TEXT 列（非 JSON 裸 URL / JSON 句子列 / JSON 整串 URL 列）→ `backup` → 快照中三列**全部被脱敏**（0 命中 `://`/canary；裸 URL 列变标签、JSON 句子列片段替换且类型保持、整串 URL 列变摘要对象）。列清单来自 `PRAGMA table_info`，因此**新增列自动覆盖**属实。
4. **④全库 canary + dist 脚本——复跑并接受覆盖范围**：本回合共 5 个 dist 级现场全部通过——QA 新脚本 182 项（`://`/canary/句子保真/JSON 合法性/新列兜底）、回合 27 现场脚本（needsInput=1）、回合 27 QA 三层脚本 35 项、RD 自查脚本 29 项、blob 边界脚本。覆盖范围 = DB 全表全列 × 快照 × DTO × 导出包 × serve 日志 × 非回环主机门禁；**QA 接受该范围**（与回合 26/27 判据表一致）。
   - **表外边界如实性独立复核**：ADR-034 §6 声明"原始响应诊断 blob（及其备份副本）逐字节保留提供方原文"。QA 在 dist 上注入同形 blob（内容寻址、含签名 URL）→ `backup` **原样复制**且保持 canary（`qa-r28-blob-boundary.log`）→ 声明**属实**；该形态属**已登记边界**（不在现有 AC/判据口径内），非本轮缺陷。

**结论（"新增字段会不会漏"）**：在 `job_stages`/`provider_attempts` 内**新增任何 TEXT/JSON 列**——"URL 不泄露"维度**不会漏**（备份兜底按表×全列自动覆盖，已实测；新写入函数/新文件被守卫②强制脱敏或登记）；但两处需**显式接线**且已有残余登记：
   - (a) 新增"**值必须是字符串**"的新契约列（如又一个 needs_input 类字段）：脱敏形态按**列名**选择（默认 SummaryObject），整串 URL 值会变对象——需在 `json_string_mode_for_column` 登记一行（**OB-13，P4**）；
   - (b) 新增**第 3 张**供应商事实表：备份兜底的表清单是固定两表，需同步加一行（**OB-13 同族**）；
   - (c) 静态守卫为**函数级**：在"已经脱敏的既有函数"里新增一个字段写入不会被守卫拦下，靠行为 canary + 评审清单兜底（**OB-14，P4**）。
   以上均只影响未来新字段、且都属"需要一行显式接线"，不影响现有列与已验收行为；判定 **P4 非阻断**（建议随 T21/T22 或下个切片把"新字段三件套：登记形态/加表清单/加 canary"写进实现任务卡）。

## 既有行为不回退（实测）

- **needs_input 语义**：句子、`code`、可行动结论（"该批不产生正式知识…请人工复核后对该批重新授权重算"/"请按 task_id 重查（不重新购买）"）全部保留；恢复产物不再静默为空（见 BUG-011 节）。
- **usage 事实**：`remoteTaskId`/`responseId`/`billing.creditMinor`/token 计数/`diagnosticSha256` 保留（本回合新用例断言 + RD 自查脚本快照/DTO 双查）。
- **诊断能力**：错误类别（`errorCode`/`outcome`、下载 `连接失败`/`传输失败` 类别）、状态码（403 过期链路）、按 task_id 重查（`qa_bug009_http_error_path_after_refresh_never_persists_signed_url` ok，付费恒 1 次）均未退化。
- **脱敏强度未弱化**：纯 URL 值仍为摘要对象（`usage.modelUrl` 既有断言不变）；`qa_bug012` 转正后全套断言逐字执行。

## 回归（全部 QA 亲跑，2026-09-13；日志在 `artifacts/web-mvp/bug011-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | `cargo test -p everything-manual --test qa_t13_independent -- qa_bug012 qa_r28` | exit 0，**2 passed**（转正 + 新用例）→ `qa-r28-bug012-promoted.log` |
| 2 | `cargo test -p everything-manual --test redaction_surface`（负向探针在场） | **exit 101，2 failed** 且打印违规文件/行号 → `qa-r28-guard-negative.log`；删除探针后 exit 0，2 passed → `qa-r28-guard-restored.log` |
| 3 | `cargo test --workspace` | exit 0，**552 passed / 0 failed / 4 ignored**（基线 550/5：+转正 1、+新增 1，ignored 5→4）→ `qa-r28-cargo-test-workspace.log` |
| 4 | `cargo xtask check` | exit 0，**7/7 [通过]**（fmt/clippy/workspace 测试/前端 lint/typecheck/vitest/合同）→ `qa-r28-xtask-check.log` |
| 5 | `cargo xtask contracts --check` | exit 0，两份 `[一致]` → `qa-r28-contracts-check.log` |
| 6 | `cargo xtask dist --target aarch64-apple-darwin` | exit 0，sha256 `f68d1487…`（与 RD 构建逐位相同）→ `qa-r28-xtask-dist.log` |
| 7 | `cargo xtask smoke-bootstrap --binary <dist>` | exit 0，7×`[检查]` 通过 → `qa-r28-smoke-bootstrap.log` |
| 8 | `bash qa-r28-bug011-sentence.sh <dist>`（QA 新脚本） | exit 0，**182 通过 / 0 失败** → `qa-r28-bug011-sentence.log` |
| 9 | `bash artifacts/web-mvp/bug009-qa/qa-r27-bug011-evidence.sh <dist>` | exit 0，恢复库 needsInput = 1 → `qa-r28-bug011-r27script-final.log` |
| 10 | `bash qa-r28-rd-verify-redaction-dist.sh <dist>`（RD 脚本**字节相同副本**，`b7c285c1…`） | exit 0，**29 通过 / 0 失败** → `qa-r28-rdverify.log` |
| 11 | `bash artifacts/web-mvp/bug009-qa/qa-r27-redaction-dist.sh <dist>`（回合 27 现场回归） | exit 0，**35 通过 / 0 失败** → `qa-r28-r27redaction.log` |
| 12 | `bash qa-r28-blob-boundary.sh <dist>`（表外边界复核） | exit 0，3 通过 → `qa-r28-blob-boundary.log` |
| 13 | `npm --prefix apps/web run test:e2e`（全量） | exit 0，**86 passed / 0 failed / 0 skipped（7.0m）** → `qa-r28-e2e-full.log` |

## 非阻断观察（P4；供 T21/T22 与后续字段扩展）

1. **OB-13 · 新增列/表的"显式接线"缺口**：JSON 列的字符串脱敏形态按**列名**选择（`json_string_mode_for_column` 默认 SummaryObject）；新增"字符串契约列"需登记一行，否则整串 URL 值会变对象（未来按类型反序列化的读取侧会重现 BUG-011 类风险）；备份表清单固定两表，新增事实表需加一行。**当前无产品影响**（无此类新列），非缺陷。
2. **OB-14 · 静态守卫颗粒度**：守卫①为字面 SQL 模式（跨行写/动态表名可绕过；`AssertSqlSafe` 动态 SQL 点应纳入人工审计）、守卫②为函数级（既有脱敏函数内新增字段不触发）。缓解 = 行为 canary（`redaction_persistence`/`qa_t13`/dist 脚本）+ 评审清单；本回合已实测两个守卫判别力（会失败、会定位）。
3. **OB-11/OB-12 → 已随 R29 处置**（QA 本回合复核成立）：提供方文本同类入口已收口；`is_url_like` 两套语义已在 `redaction.rs` 模块文档与 ADR-034 显式定标。建议协调者按 CLOSED 记录。
4. **OB-1…OB-6、OB-10 沿用**（回合 25/26/27 原文；OB-7/OB-8 脚本门禁继续生效；本回合全量 e2e 与脚本仍会重写 `t16-*/t17-*/t18-*/t19-*` 证据目录，属既有噪声；环境中 00:18 起的孤儿 `serve` 仍在运行、非本回合所启，QA 未杀）。

## 未覆盖边界（本回合记录，不冒充通过）

1. 真实 Provider / 真实费用 / 真实 CDN 域名与签名形态：全部本机 fixture；T23 授权后另验。
2. dist（release）二进制上"传输失败/提供方文本产生侧"不可测（`allow_local_fixture` 仅测试构建）：产生侧在测试构建验证（同一代码路径）；真实诊断 blob 的内容形态未在真实链路复核（T23）。
3. blob 表外边界（诊断原文及备份副本）为**已登记边界**（ADR-034 §6），非本轮判据范围；本轮只复核了"备份原样复制"的行为面。
4. 新增列/表的显式接线（OB-13）与守卫颗粒度（OB-14）：属未来风险，尚无对应代码可验。
5. 修复前二进制的真实回滚演练、跨平台（musl）、ZIP 导入：沿用回合 25 清单，未变。
6. 全项目：T21–T23 未交付，**MVP 尚未完成**；本轮的 CLOSED 只针对本次复验范围。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 26 | 2026-09-13 | 缺陷复验 | BUG-008 | 2（ui_revision 2） | BUG-008 CLOSED；本轮 FAIL（新增 BUG-009 P2 OPEN） | 本文件第 26 节 |
| 27 | 2026-09-13 | 缺陷复验 | BUG-009 / BUG-010 | 2（ui_revision 2） | BUG-009/010 CLOSED；本轮 PASS（新增 BUG-011/012 P3） | 本文件第 27 节 |
| 28 | 2026-09-13 | 缺陷复验 + 结构性保障审计 | BUG-011 / BUG-012 | 2（ui_revision 2） | **BUG-011 CLOSED、BUG-012 CLOSED；本轮 PASS**（新增 2 条 P4 观察 OB-13/OB-14） | 本节 |

- **交接给协调者**：(a) `qa_round: 28`、本轮 `qa_result: PASS`；(b) `qa_history` 追加 `{round: 28, scope: defect-reverify, task_ids: [BUG-011, BUG-012], prd_revision: 2, result: PASS}`；(c) `open_defects` 更新：**BUG-011 → CLOSED**、**BUG-012 → CLOSED**（保留记录）；**注意 `state.yaml` 里 BUG-008/009/010 仍标 `fixed_pending_reverify`，与本报告 26/27 回合的 CLOSED 结论不一致，请一并订正**；(d) `carry_over`：新增 OB-13、OB-14；OB-11、OB-12 可标 CLOSED；OB-1…OB-6、OB-10 沿用；(e) QA 测试文件转正 1 例（`qa_bug012_*`）+ 新增 1 例（`qa_r28_usage_json_*`），已在报告中说明理由。
- **交接给 RD**：本回合无必须修复项。可选建议（非阻断）：把"新字段三件套"（`json_string_mode_for_column` 登记形态 / 新增事实表加备份表清单 / 加 canary 用例）写进后续任务卡；如后续重排 `redaction_surface` 守卫，请保留其负向可验证性（本回合已验证的判别力是验收基线）。
- **交接给 PM**：无新增需裁定项；ADR-033/ADR-034 口径 QA 均已接受；OB-13/OB-14 为 P4 风险提示（无当前 AC 偏离），供 T21/T22 排期参考。
- **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「BUG-011/BUG-012 复验验收知识（QA 回合 28）」（结构守卫负向验证手法、"新列免改代码"实测手法与两处显式接线缺口、blob 表外边界的独立复核手法、守卫颗粒度）；`llmdoc/validation-release.md` 命令合同**无变化**。本报告即验收依据、缺陷与未覆盖边界的正式记录。
- **QA 产出与证据**：`artifacts/web-mvp/bug011-qa/`——`qa-r28-bug011-sentence.sh`+`.log`（182 项）、`qa-r28-bug011-r27script-final.log`、`qa-r28-bug012-promoted.log`、`qa-r28-guard-negative.log`/`qa-r28-guard-restored.log`、`qa-r28-rd-verify-redaction-dist.sh`（字节相同副本）+`qa-r28-rdverify.log`、`qa-r28-r27redaction.log`、`qa-r28-blob-boundary.sh`+`.log`、`qa-r28-xtask-dist.log`、`qa-r28-cargo-test-workspace.log`、`qa-r28-xtask-check.log`、`qa-r28-contracts-check.log`、`qa-r28-smoke-bootstrap.log`、`qa-r28-e2e-full.log`、`rd-dist-snapshot/`、`snapshot-check.txt`/`snapshot-items.json`/`job-detail.json`/`job-detail2.json`/`serve*.log`/`source-after.sql`。**本回合临时目录（`/tmp/em-r28-*`）已清理；本回合启动的进程已结束（pgrep 复核）；`crates/` 下无探针残留（`git status` 复核）。**

# 回合 29 · T21（产品回归、安全与故障矩阵；QA 独立执行）

结果：**PASS**（T21 切片；**不代表产品全量 PASS**——T22 正式包/平台、T23 真实 Provider 未交付）· 回合：29 · PRD 修订：2（ui_revision 2）· 范围：切片（T21）· PRD 核对：RD/QA 依据的 PRD 修订 = 当前修订（2），一致。

## 环境与交付版本

- 工作树：HEAD `313849611d190e93b65a84cd13dba1b3ff784477` + 未提交工作树（含 T01–T20 与 R29 修复）。源文件清单（307 个文件 sha256）与总摘要：`artifacts/web-mvp/t21-qa/r29-source-manifest.txt`（`3ada9f34…`）。
- 平台／工具链：macOS `Darwin 25.6.0 arm64`（Apple M1，8 核，16 GiB）；cargo 1.98.1；rustc 1.98.1；Node v26.0.0；npm 11.x；Playwright 1.60.0；浏览器：Playwright Chromium **148.0.7778.96**（headless，e2e）、系统 Chrome **152.0.7977.84**（headed，性能）。
- dist 二进制（QA 未重建，核对为当前源）：`dist/aarch64-apple-darwin/everything-manual`，sha256 **`f68d14874936b74f70aaa99b0eb8462396e933344a4495653ed239ba8838ac02`**（25 112 080 B，2026-09-13 04:26 构建；`crates/ apps/web migrations` 下无更新于该时刻的生产源变更，仅 T21 新增测试文件较新 → 二进制代表当前源）。
- 数据与边界：全部本机 `127.0.0.1` fixture（T05）+ 临时 data-dir；**零真实外网、零付费调用**（T23 才允许）。真实 Provider／真实 CDN 签名形态未验（沿用 T23）。
- QA 本回合改动（均为**测试与证据**，未改生产代码）：新增 `crates/server/tests/qa_t21_independent.rs`（4 用例）、新增 `apps/web/tests/e2e/qa-t21-independent.spec.ts`（2 用例）、转正 2 个 `#[ignore]`（`qa_t10_independent`/`qa_t11_independent`，断言逐字未改）、新增 `artifacts/web-mvp/t21-qa/**`（日志/脚本/清单）。

## §3 关键测试矩阵逐行覆盖（行 → 执行方式 → 结果 → 未覆盖项）

| §3 行 | 执行方式（真实命令/用例） | 结果 | 未覆盖项与承接卡 |
| --- | --- | --- | --- |
| 会话：登录、刷新、注销、授权读资产 | `--test auth_api`（`unauthenticated_protected_routes_return_401…`、`logout_revokes_session_and_expired_session_is_401`、`/auth/session` no-store、`insert_session` 过期会话）+ e2e `import-flow`/`qa-t16` 真实登录 | PASS | — |
| 会话：CSRF／Origin／过期／错误密码限速／跨资产归属 | `--test auth_api`（`mutating_requests_require_csrf_and_same_site_origin`、`wrong_password_is_401_and_repeated_failures_are_429`、`rate_limit_window_expires…`、`https_declared_origin…`）+ `--test assets`（`unauthorized_and_unknown_asset_access_is_404…`） | PASS | — |
| 上传：PDF、JPEG/PNG、重复 hash | `--test assets`（`uploads_pdf_png_jpeg_and_deduplicates_by_sha256`、`upload_over_one_mib…`） | PASS | — |
| 上传：假类型、超限、像素炸弹、路径穿越、断流、磁盘满 | `--test assets`（`forged_types…`、`oversized_uploads…`、`decoding_failures_pixel_bombs…`、`path_traversal_filenames…`、`insufficient_disk_space…`、`item_total_limit…`）+ **新增** `qa21_upload_stream_abort_mid_body_leaves_no_half_asset`（真实 hyper + 裸 TCP 中途断开，正对照：先见 tmp 再归零） | PASS | 写流中途 ENOSPC（磁盘满发生在写入过程中）沿用 T06 已登记非阻断项 |
| PDF：文字／扫描／旋转／非拉丁字体、1-based 出处 | e2e `pdf-preparation.spec.ts`（4 用例）+ `qa-t09-independent`（viewport/白底/1-based） | PASS | — |
| PDF：加密、超 100 页、worker 缺失、中断续传、CMaps／WASM 离线 | e2e `pdf-preparation`（`加密…`、`101 页…`、`断线续传…`）+ `qa-t09-independent`（离线全资源本机、续传用新标签页）+ **新增** `qa-t21-independent.spec.ts`（worker 阻断 → 终态拒绝、零页记录零 preparation 零费用；对照用例证明阻断非空转） | PASS | 真实 JPX/ICC 触发 WASM 代码路径仍无样例（T09 已登记；未新增） |
| 费用：输入冻结、价表版本、实际结算 | `--test generation_requests`（`snapshot_freezes_photo_ids_hashes_and_provider_config_without_secrets`、`price_version_change_invalidates_the_quote`、`ledger_settlement_release_and_unknown_keep_reservations_intact`） | PASS | — |
| 费用：过期报价、重放 20 次、并发预算、unknown 不释放 | 同上（`expired_quote_is_rejected…`、`same_key_same_body_replayed_twenty_times…`、`concurrent_submissions_never_create_duplicate_jobs_or_reservations`、`ledger_settlement…unknown`） | PASS | — |
| Tripo：multipart+view-key、taskid、查询、模型 | `--test tripo_contract`（`upload_sends_multipart_with_file_field…`、`submit_body_matches_frozen_parameters_without_v2_fields`、`fixture_end_to_end_reaches_poll_success_with_billing_settled`） | PASS | 真实 API 协议（T23） |
| Tripo：200 业务失败、429、5xx、接受 POST 后断连、未知状态、缺输出 | 同上（`http_200_with_nonzero_code…`、`http_statuses_are_classified…`、`paid_post_disconnect_is_never_resent`、`unknown_remote_status_is_kept_verbatim…`、`success_without_model_is_not_success…`） | PASS | — |
| 说明书 AI：文本／页图、schema、页覆盖、来源 | `--test manual_ai_contract`（`extract_request_uses_responses_text_format_with_strict_schema_and_image_data_url`、`batch_result_marks_scan_page_evidence…`、`seven_pages_split_into_batches…`） | PASS | 真实模型效果（T23） |
| 说明书 AI：拒答、截断、伪造页、漏批、同步结果未保存、资料内恶意指令 | 同上（`refusal_produces_no_official_knowledge…`、`incomplete_truncated_malformed…`、`evidence_referencing_pages_outside…`、`batches_outside_the_frozen_plan…`、`batch_without_persisted_response…`、`page_text_instructions_cannot_change_budget_or_trigger_actions`） | PASS | — |
| 下载：哈希一致本地 GLB | `--test model_assets`（`download_streams_to_blob_without_forwarding_credentials`、`end_to_end_downloads_validates_and_creates_immutable_revision`） | PASS | — |
| 下载：过期 URL 再查、DNS 重绑定／私网跳转、bearer 泄露、截断／超限 | 同上（`expired_link_is_refreshed_by_requerying_the_known_task`、`dns_rebinding_to_private_address_is_rejected`、`redirects_are_validated_hop_by_hop`、`download_streams_to_blob_without_forwarding_credentials`、`truncated_glb_enters_needs_input…`、`size_limit_and_disk_full…`） | PASS | 真实 CDN 域名/签名形态（T23） |
| 执行器：并发领取、恢复、按分支重试 | `--test jobs_recovery`（`concurrent_claims_yield_exactly_one_winner_with_new_epoch`、`expired_worker_receipt_is_saved_but_cannot_advance_or_unlock`、`failed_branch_does_not_block_independent_branch…`）+ `--test pipeline`（按分支 retry） | PASS | — |
| 执行器：SIGKILL、租约过期晚到 ID、重复推进、unknown 不盲重购 | `--test jobs_recovery`（`sigkill_during_paid_post_makes_submission_unknown_without_repurchasing`、`sigkill_during_poll_resumes_with_known_remote_task_and_never_resubmits`、`remote_task_id_conflict_is_recorded…`、`lease_renewal_keeps_stage_alive…`） | PASS | — |
| 3D：旋转缩放、拾取、视角保存 | e2e `viewer.spec.ts`（OrbitControls/键盘等价）+ `manual-review.spec.ts` T19-1（拾取绑定、拖动不建点）+ `qa-t19` QA19-6（视角保存/回到/清除） | PASS | 移动端不支持校准已有禁用解释（AC-060） |
| 3D：根坐标变换、拖动误点、模型外链、WebGL 失效／恢复、资源释放 | `npm run test -- --run src/features/viewer/coordinates.test.ts` + `viewer.spec.ts`（context lost→restored、卸载/换模型资源释放、AC-051 世界坐标一致）+ `--test model_assets`（`external_uris_and_unsupported_features_are_rejected`）+ `qa-t18-independent` QA-T18-3/4（真实 GL 计数、10 次切换趋势） | PASS | 连续切换 10 次的内存趋势为 CDP 堆采样 + 资源账本（非长时压测） |
| 知识／发布：部件→步骤→原文、版本可读 | e2e `manual-review.spec.ts` T19-2/T19-3 + `qa-t19` QA19-7 + `--test publishing` | PASS | — |
| 知识／发布：stale 热点、未确认知识、412 并发、改 draft 不改 release | `--test publishing`（`regenerated_model_marks_previous_bindings_stale_and_rejects_old_sha`、`publish_reports_each_invariant_violation…`、`publish_is_transactional_idempotent_and_immutable`、`publish_rejects_tampered_references…`） | PASS | — |
| 备份：停服快照→新目录恢复 | `--test backup_restore`（`backup_requires_stopped_server_and_produces_consistent_snapshot`、`restore_into_new_directory_reproduces_readable_release_pdf_and_glb`） | PASS | — |
| 备份：活跃锁、损坏 hash、缺 blob、新 schema、非空目录拒绝 | 同上（退出码实测 `backup_requires_stopped…`、`backup_fails_on_corrupted_source_blob…`、`restore_verifies_backup_and_rejects_damage…`(c/d/e)、`restore_rejects_backup_with_newer_schema`、非空目标 → 4） | PASS | "缺 blob（文件被删除）"与"损坏 blob"走同一 hash 校验分支，未单独构造删除形态 |
| 包装：单 binary 启动、API、前端路由刷新 | `cargo xtask dist`（既有 T20/T01 证据线）+ QA 现场 `smoke-bootstrap`（7 项含 `/`、`/assets/*`、`/library/some-item`、`/api/unknown` JSON404、缺失静态资源 404）；`otool -L` 仅系统框架（Security/CoreFoundation/libiconv/libSystem） | 部分 PASS（仅 T01 级） | **正式包 smoke（AC-002 全项）、无 dist/源码/Node/Python 环境、离线 vendor、平台动态依赖清单 = T22** |
| （§3 末）崩溃注入五个断点 | 见下节 | PASS | 断点 1–3 的"资产引用/不可变版本"不涉及该阶段产出，映射见下节 |
| （§3 末）说明书批次中断 | 见下节（新增 `qa21_three_batch_interruption…`） | PASS | — |
| 命令合同 | `cargo xtask check`（7/7）、`cargo test --workspace`、`npm run test -- --run`、`npm run test:e2e`（88 passed / 0 skipped） | PASS | `test-live`（T23）；`smoke`/`dist` 全项（T22） |

## 五个崩溃断点与批次中断（逐断点实测）

判据来源：validation-release §3 末"每个断点重启后核对任务数、远端请求数、费用预留、资产引用与不可变版本"。

| # | 断点 | 实测证据（用例；重启方式） | 核对结果 |
| --- | --- | --- | --- |
| 1 | 付费 POST 发出前 | `jobs_recovery::failpoint_breakpoints_recover…` 断点①（intent 已落库、未标 submitting）/断点②（submitting、请求未发）；进程内 panic + 恢复扫描 | 任务数 1、远端 POST 0→1（恢复后恰一次）、attempt 不重复、下游 queued。费用：无第二次购买（`same_key_…` 与 `interrupted_creation_transaction_leaves_no_half_reservation` 覆盖预留事务完整性） |
| 2 | 供应商接受但响应未到 | `tripo_contract::paid_post_disconnect_is_never_resent`（真实 HTTP job + 真实 ledger）、`jobs_recovery::sigkill_during_paid_post…`（**真实子进程 kill -9**） | POST 计数 1→1、attempt=unknown（无远端 ID、不编造）、ledger 预留保留且 `actual` 不填 0、下游 queued |
| 3 | task ID 到达但状态未推进 | `jobs_recovery::failpoint_breakpoints_recover…` 断点③ + `sigkill_during_poll_resumes_with_known_remote_task_and_never_resubmits`（真实 kill -9） | attempt=accepted + remote_task_id 已存；重启后按同一 ID 查询（GET 1 次）、付费 POST 恒 1、按已知事实补推进 |
| 4 | blob rename 后 DB 事务前 | **新增** `qa21_blob_renamed_before_metadata_commit_is_safe_after_restart`（构造 rename 已完成现场 + 重开数据库/AppState 并执行 `serve` 启动扫描例程）+ **dist 级** `qa-r29-blob-breakpoint-dist.sh`（正式二进制 **kill -9 → 注入现场 → 同 data-dir 重启**） | 两道实测均全项通过：被引用资产逐字节可读（sha256 一致）；jobs／provider_attempts／cost_ledger／model_revisions 行数不变；孤儿与 tmp 进 `quarantine/`（只移动不删除，2 文件）；同一内容重传 201 且 blob 行不重复 |
| 5 | draft 已提交但 HTTP 响应未返回 | **新增** `qa21_draft_committed_before_response_is_never_duplicated_on_restart`（`assemble_draft` 同一入口落草稿 → 阶段 running+租约过期 → 恢复扫描 + 真实 `PipelineHandlers` 重跑） | 草稿恒 1 份、同 id、revision 不递增；`provider_attempts=0`、`cost_ledger=0`、`manual_releases=0`（不自动发布）；job 状态如实（部分草稿 → needs_input，不假成功） |
| 批次 | 第 1 批完成、第 2 批响应未知、第 3 批未开始 | **新增** `qa21_three_batch_interruption_never_reruns_completed_batch`（11 页 → 3 批；批 0 真实成功；批 1 命中 `manual_after_request_before_response`；恢复扫描 + 新执行器） | **第 1 批不重跑、第 2 批不重发**（两批 per-stage attempt 恒为 1）；第 2 批 `submission_unknown` 且有可读原因；merge 保持 queued（不产生正式知识）；job=submission_unknown；**实测形态 = 恢复后第 3 批在已确认预算内继续（POST 总数 3，批 3 attempt=1）**，§3 允许"继续或暂停"两种形态；三批身份独立（stage id + 页集合 `[1..5]/[6..10]/[11]` + 各自 attempt） |

## 命令合同（QA 现场执行；日志在 `artifacts/web-mvp/t21-qa/`）

| # | 命令 | 结果（原始输出摘要） |
| --- | --- | --- |
| 1 | `cargo xtask check` | exit 0，`7/7 [通过]`（fmt / clippy -D warnings / workspace 测试 / 前端 lint / typecheck / vitest / 合同检查）→ `r29-xtask-check-final.log` |
| 2 | `cargo test --workspace` | exit 0，**558 passed / 0 failed / 2 ignored**（含本回合 +4 与转正 +2；唯一 2 条 ignore = T20 手工演练造数、items.rs 文档示例，均非必选）→ `r29-xtask-check-final.log`（check 内含同命令）与 `r29-cargo-test-workspace.log`（加测试前 556/4） |
| 3 | `npm --prefix apps/web run test -- --run` | exit 0，**16 files / 130 tests passed** → `r29-vitest.log` |
| 4 | `npm --prefix apps/web run test:e2e` | exit 0，**88 passed / 0 failed / 0 skipped（7.0m）** → `r29-e2e-full.log`（含本回合新增 2 例；首跑 87/1 的失败为本 QA 新用例自身缺陷，已修，见下节） |
| 5 | `npm --prefix apps/web run test:e2e -- qa-t21-independent.spec.ts` | exit 0，2 passed（worker 缺失形态 = **rejected**：可行动文案 + 零页记录/零 preparation/零费用）→ `r29-e2e-qat21.log` |
| 6 | `cargo test -p everything-manual --test qa_t21_independent` | exit 0，**4 passed** → `r29-qa21-tests.log` |
| 7 | `cargo xtask contracts --check` | exit 0，两份 `[一致]`（在 check 内）→ `r29-xtask-check-final.log` |
| 8 | `cargo xtask smoke-bootstrap --binary <dist>` | exit 0，7×`[检查]` 通过 → `r29-smoke-bootstrap.log` |
| 9 | `bash qa-r29-blob-breakpoint-dist.sh <dist>`（QA 新增） | exit 0，10/10 `[OK]` → `r29-blob-breakpoint-dist.log` |
| 10 | `bash qa-r29-metadata-api-p95.sh <dist> 19180 300`（QA 新增） | exit 0；items 列表 p50 0.6 ms／**p95 0.6 ms**／max 1.0 ms（300 样本）；jobs 100 样本 p95 0.6 ms；ready p95 0.4 ms → `r29-metadata-api-p95.log` + 原始样本 `metadata-*-times.txt` |
| 11 | `node artifacts/web-mvp/t18-rd/measure-viewer-perf.mjs --seconds 20`（QA 复跑 RD 脚本，真实 Chrome headed + 真实 GPU） | exit 0；**p95 帧耗时 17.7 ms**（p50 16.7 / p99 18.4 / max 18.7 ms，1 203 样本，100 352 三角面/1 贴图，Chrome 152 + Apple M1 ANGLE Metal，`pass=true`，console 错误 0）→ `r29-viewer-perf.log`/`r29-viewer-perf.json` |

## QA 新增与转正的测试（均为测试侧；无生产代码改动）

1. **`crates/server/tests/qa_t21_independent.rs`（新增 4 例）**：上传断流；断点四（构造现场 + 启动扫描例程）；断点五（草稿已提交后恢复）；批次 3 批中断。设计要点写入 `llmdoc/decisions.md`（见交接）。
2. **`apps/web/tests/e2e/qa-t21-independent.spec.ts`（新增 2 例）**：worker 缺失（阻断真正的 worker 脚本 URL；**注意不能阻断 `pdf.worker.min.mjs?url` 模块请求**——否则 PreparePage 模块图整体加载失败，属测试缺陷而非产品缺陷；本轮共踩两次测试自身缺陷（① 宽 glob 阻断模块；② 拒绝态下无界等待 `prepare-status`），均已在代码注释与非代码知识中留鉴）+ 对照用例。
3. **转正 2 个 `#[ignore]`**：`qa_t10_independent::qa_total_wait_budget_must_trigger_on_real_poll_shape`（BUG-003）与 `qa_t11_independent::qa_defect_get_estimate_reflects_confirmation_and_consumption`（BUG-004）。两只缺陷均已 CLOSED，但用例仍被静默跳过；本回合先以 `--ignored` 实测**两者通过**，再去掉 ignore（断言逐字未改，只更新注释）。默认套件因此从 556/4 变为 **558/2**。
4. **QA 自查的首跑失败（已修正，非产品缺陷）**：① e2e 首跑 `87 passed / 1 failed`——失败用例是 QA 本轮新写用例本身（宽 glob 阻断 worker 模块 → 页面模块图失败；以及 `prepare-status` 在拒绝态不存在导致 `textContent` 等待到测试超时）。修正后全量 e2e 复跑 **88/0/0**。② 两次脚本自身的 `$VAR（` 中文括号解析问题（bash 把全角括号并入变量名）已改为 `${VAR}`。
5. `test.fixme`/`test.skip`/`#[ignore]` 复核：e2e 与 vitest **无任何**活动 skip/fixme；Rust 套件仅剩 2 条 ignore（非必选，见上）。

## CI / 浏览器矩阵 / 性能基线评估

- **CI**：仓库**无** CI 配置（`.github/workflows` 不存在）。核对合同：validation-release.md 仅在"Playwright 浏览器可在开发／CI 安装"处提及 CI，§8 完成条件不含 CI；implementation-plan.md T22 的允许范围明确含"CI release矩阵"。**结论：CI 不是当前 AC 的必选门禁**（本地 `cargo xtask check` + 全量 e2e 是当前唯一全量入口），但 T22 应交付 CI（release 矩阵）；建议协调者把"新增 CI：check + e2e + dist 矩阵"作为 T22 卡面明确项登记。**登记为对 T22 的明确事项，非本切片缺陷。**
- **浏览器矩阵（validation-release §4 / AC-063）**：e2e 仅配置 **chromium**（Playwright Chromium 148.0.7778.96，headless）；本机**未安装 Firefox / Edge**，Playwright 也未安装 firefox/webkit 浏览器 → **Firefox 与 Edge 的"当前支持版本通过"未覆盖**（本机 Chrome 152.0.7977.84 仅用于性能测量，未跑 e2e）。按 T22（同机多浏览器 + 发布报告写精确版本）/T23 承接；**不得据此宣称 AC-063 通过**。Safari 未列入支持（无实机测试，符合合同）。
- **性能基线（AC-061）**：① 阅读器旋转实测 **p95 17.7 ms ≤ 33 ms**（1 203 样本；T18 原测 18.7 ms，两次数值同量级）（QA 复跑 RD 脚本，真实 Chrome headed + 真实 GPU，Apple M1，100 352 面）；② 本地元数据 API（无供应商等待）**p95 0.6 ms ≤ 200 ms**（QA 新脚本，300 样本）。两项均记录设备/浏览器/GPU/样本。**缺口**：① 未做 ≥5 分钟连续旋转（本轮 20 s 采样，T18 亦为 20 s）——"连续旋转 ≥5 分钟"的长时间稳定性仍无实测，建议 T22/T23 或人工复核补；② 模型加载耗时未记录（合同要求"单独记录，不承诺瞬开"）；③ 性能结论仅本机 Apple M1（目标设备为 PM 待确认项）。

## 缺陷

**本回合未新增缺陷**（BUG-001–012 全部 CLOSED；本轮未发现违反当前必选 AC 的问题）。以下为修掉的 QA 侧测试缺陷（不进入缺陷编号）：见上节第 4 条。

### 非阻断观察（P4；供 T22/T23 与 PM）

1. **OB-15 · worker 缺失的错误归类偏"文件可疑"**：worker 脚本不可加载时，客户端把异常归为 `invalid`，文案为"该文件无法作为 PDF 解析：请确认上传的是完整、未损坏的 PDF 原件"。对"部署缺资源"这类基础设施故障，该文案会把排查方向指向用户文件。行为本身合规（明确失败、可行动、零副作用），仅建议 T22/T23 或后续小卡区分"解析失败"与"运行环境不可用"。
2. **OB-16 · 合同措辞张力（需 PM 留痕）**：contracts §5 表格"付费创建结果未知 → 暂停该分支后续购买" vs validation-release §3"第 3 批按已确认预算继续或暂停并明确显示"。实测行为 = **继续**（在已确认预算内，且 §3 明确允许）。建议 PM 在合同留痕澄清"分支暂停"是否仅指"同阶段后续购买"，以及"并明确显示"的 UI 义务是否只针对暂停形态。
3. **OB-17 · 2 条陈旧 `#[ignore]`（已由 QA 转正）**：见上；建议后续把"缺陷 CLOSED 时同步转正其复现用例"写入卡面完成定义，避免静默跳过累积。
4. **OB-1…OB-14 沿用**（回合 25–28 原文/`state.yaml` carry_over）；`state.yaml` 中 BUG-008/009/010 的 `fixed_pending_reverify` 仍与报告结论不一致（回合 28 已提请订正，本轮再次提示）。
5. **环境噪声**：本机仍有 2026-09-12 遗留的孤儿 `serve` 进程（PID 14182，T20 测试目录，非本回合所启）与既有 e2e 证据重写噪声；本回合新起的进程与临时目录均已清理。

## 未覆盖边界（不冒充通过）

1. **T22（正式包）**：AC-002 全项 smoke（真实备份 restore + 生产 embedded-ui + 无源码/Node/Python 环境启动）、平台动态依赖清单与签名声明、多平台 `dist/smoke`。
2. **T23（真实链路）**：真实 Tripo 与说明书 AI 协议、真实余额/账单、真实 CDN 域名与签名 URL 形态、真实模型效果与预算；`cargo xtask test-live` 参数校验。
3. **Firefox / Edge**（本机无该浏览器）；Safari 明确未支持。
4. **性能**：≥5 分钟连续旋转稳定性、模型加载耗时、PM 具名目标设备；移动端实机布局与"不支持校准"的人工复核。
5. **UI-060（REQ-037/AC-058 的网页导出入口）仍未实现**：grep 确认 `ReleaseListPage`/`ReleaseReaderPage` 无任何导出 UI（端点/生成类型已就绪）；T21 允许范围不含 web 功能代码（tests/CI/validation），故本轮**登记为对 T22/专派前端小卡的明确要求**（回合 25 已登记，未经 T20 PASS 掩盖）。
6. 多实例/多用户、自动备份定时器、E01–E04 增强项（非 MVP）。
7. **全项目：T21–T23 未全部交付 → MVP 未完成**；本 PASS 仅覆盖 T21 切片与上述矩阵。

## 非代码知识与限制（提炼）

- **否定型用例必须有正对照**（断流用例：先证明 tmp 出现 → 再证明归零），否则"零副作用"断言可能只是请求根本没进 handler 的空转通过。
- **重启语义的两级证据**：进程内"重开数据库 + 新 AppState/Router + 执行 serve 启动例程"与 dist 二进制"真实 kill -9 → 同 data-dir 重启"；后者是本轮对断点四的最终证据（10/10）。
- **per-stage attempt 计数**是批次级"不重跑/不重发"的唯一可靠归因（全局 fixture 调用计数无法区分批次）；跨卡复用手法已写入 decisions。
- **Playwright 阻断 worker 的正确姿势**：不可阻断 `pdf.worker.min.mjs?url` 模块请求（会连带打断页面模块图）；只阻断真正的 worker 脚本 URL（predicate 过滤 `?url`）。
- **bash + 全角标点**：`$VAR（` 会被解析为变量名的一部分 → 统一用 `${VAR}`（本轮两次踩坑，供后续脚本复用）。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 27 | 2026-09-13 | 缺陷复验 | BUG-009 / BUG-010 | 2 | PASS | 本文件第 27 节 |
| 28 | 2026-09-13 | 缺陷复验 + 结构性保障审计 | BUG-011 / BUG-012 | 2 | PASS | 本文件第 28 节 |
| 29 | 2026-09-13 | 切片验收（回归/安全/故障矩阵） | T21 | 2（ui_revision 2） | **PASS**（无新增缺陷；P4 观察 OB-15…OB-17） | 本节 |

- **交接给协调者**：(a) `qa_round: 29`、`qa_result: PASS`、`current_tasks: [T21]` 可移入 `accepted_tasks`（`{task_id: T21, prd_revision: 2, qa_round: 29, accepted_ac_ids: [AC-019/AC-065 安全矩阵行, AC-063 部分（仅 Chromium）, AC-061 部分, 卡内必测项], status: accepted}`）；(b) `qa_history` 追加 `{round: 29, scope: slice, task_ids: [T21], prd_revision: 2, result: PASS}`；(c) `open_defects` 保持空；`closed_defects` 不变；(d) `carry_over` 建议新增：**T22 必办**（CI、正式包 smoke、Firefox/Edge、平台依赖）、**PM 留痕**（OB-16 合同措辞、UI-060 前端小卡排期）、**性能补测**（≥5 min 旋转、模型加载耗时）、OB-15；并**再次订正** `state.yaml` 中 BUG-008/009/010 的状态字段。
- **交接给 RD**：本回合无必须修复项（未发现产品缺陷）。建议（非阻断）：① 若后续实现 UI-060 导出入口，复用既有 `export_release` 类型并补"进行中/失败重试"；② worker 不可用的文案区分（OB-15）可并入该前端小卡；③ 请勿在未复验前自行改动 `qa_t21_independent.rs`/`qa-t21-independent.spec.ts` 的断言（转正与新增由 QA 负责）。
- **交接给 PM**：OB-16（contracts §5 与 validation-release §3 的措辞张力 + "明确显示"的 UI 义务）；UI-060 排期裁定（T22 或专派小卡）；AC-061 的"具名测试机/目标设备"确认（当前仅本机 Apple M1）；性能长期项（≥5 分钟旋转、模型加载耗时）是否纳入 T22/T23。
- **llmdoc 更新清单**：`llmdoc/decisions.md` 追加「T21 验收知识（QA 回合 29）」（否定型用例正对照、断点四/五的现场构造与重启核对五件事、per-stage attempt 归因、Playwright worker 阻断陷阱、bash 全角标点陷阱、CI/浏览器/性能缺口结论）；`llmdoc/validation-release.md` 命令合同**语义无变化**（仅按需在执行记录小节留痕；本轮不动合同文本）。本报告即验收依据、缺陷与未覆盖边界的正式记录。
- **QA 产出与证据**：`artifacts/web-mvp/t21-qa/`——`r29-xtask-check-final.log`、`r29-cargo-test-workspace.log`、`r29-vitest.log`、`r29-e2e-full.log`（88/0/0）、`r29-e2e-qat21.log`、`r29-qa21-tests.log`、`r29-blob-breakpoint-dist.log`、`qa-r29-blob-breakpoint-dist.sh`、`r29-metadata-api-p95.log`、`qa-r29-metadata-api-p95.sh`、`metadata-*-times.txt`、`r29-viewer-perf.log`/`r29-viewer-perf.json`、`r29-smoke-bootstrap.log`、`r29-clippy-pretest.log`、`r29-source-manifest.txt`、`r29-ignored-qa-t10.log`。**本回合临时目录（`/tmp/em-r29-*`）已清理；本回合启动的进程已结束（pgrep 复核）；`git status` 复核无预期外改动（QA 仅新增测试文件与 artifacts）。**

---

# 回合 30 · T22（多平台单二进制发布）Linux 半边验收（QA 独立执行）

结果：**PASS**（T22 切片 **Linux 半边**：AC-064 Linux / AC-002 / AC-059 Linux 三条必选全过）· 回合：30 · PRD 修订：2（ui_revision 2，与 RD 依据一致）· 范围：切片（T22）· **不代表产品全量 PASS**（T23 真实 Provider、AC-063 Firefox/Edge、物理 x86_64 硬件、签名/公证仍未验）· **附带 1 个未关闭缺陷 BUG-013（P3，跨卡，不阻断本切片；阻断全量发布门禁）**

## 环境与交付版本

- 工作树：HEAD `39e202d` + 未提交改动（`scripts/*`、`xtask/src/dist.rs`、`.github/workflows/ci.yml`、`docs/operations.md`、llmdoc）；**`crates/**`、`apps/web/**`、`contracts/**`、PRD 未改**（`git status` 复核）。RD 依据 PRD 修订 = 当前修订 = 2。
- 宿主：macOS `Darwin 25.6.0 arm64`（Apple M1）。容器：Docker Desktop 4.68.0 / engine 29.3.1，`--platform linux/amd64`，内核 `6.12.76-linuxkit x86_64`，rustc/cargo 1.98.1，容器内 Node v22.22.2。容器内 `/proc/cpuinfo` = `VirtualApple @ 2.50GHz` → **x86_64 用户态由 Rosetta 翻译执行（宿主是 arm64，不是物理 x86_64）**。
- 交付产物 `artifacts/web-mvp/t22-rd/linux/dist-x86_64-unknown-linux-musl/`（5 件，白名单通过）：binary sha256 **`77b38c91327c4d5c697c1fccb48054a827034ee088167cb38ad3249a3d338246`**、**28 236 728 B**。QA 在**宿主**与**容器内**各独立复算一次 → 与 `SHA256SUMS`/`build-info.json` 一致；并**独立重跑 `dist --check-reproducible`**（`cargo clean -p` 后重链接）得同一哈希（见下表）。
- 样例备份：T20 tracked 目录**缺** `database/manual.sqlite3`（`.gitignore` 的 `*.sqlite3`；`git ls-files artifacts/web-mvp/t20-rd/sample-backup` 无 DB 条目）→ 本轮 smoke 用容器内**重建件**（同源生成器 + 正式二进制 `backup`）。
- 数据与边界：全部本机 loopback；离线轮在 `docker run --network none` 命名空间内；**零真实外网依赖、零付费调用**（T23 才允许）。

## AC 验收矩阵（本回合必选）

| AC | 期望 | 实际（QA 亲测） | 判定 | 命令／证据 |
| --- | --- | --- | --- | --- |
| **AC-064**（Linux 半边） | 单文件 + SHA256 + licenses + 动态依赖清单；冷环境启动通过 smoke；未跑平台不贴标签 | `static-pie linked`、`ldd: statically linked`、`DT_NEEDED=0`；隔离扫描（QA 自算）0 命中；licenses v2（rust 205 / web 40）；smoke 7 步通过；`--check-reproducible` 同哈希 | **PASS**（+ 下方"原生"限定） | 见下表 #1/#2/#5；`artifacts/web-mvp/t22-qa/t22-qa-verify-static.log`、`qa-r30-reproducible-dist-smoke.log` |
| **AC-002** | 冷目录 + restore 合法样例备份 + 首页/嵌套路由/本地资源/JSON 404 + 停服重启持久 + 不连 fixture | smoke 7 步全过（31 条 `[检查]`）；`/api/unknown → 404 application/json` 与未知静态资源 `404 text/plain` 由 `smoke-bootstrap` 在同一发布二进制上逐字断言。样例备份为 **T20 同源重建件**（tracked 目录缺 DB，见下"质疑 3"） | **PASS** | 下表 #3/#4/#8 |
| **AC-059**（Linux 侧） | 断外网仍可读 3D/部件/步骤/原文/PDF，`/health/ready` 不因云端不可达失败 | `--network none` 整条 smoke 7 步全过（同一命名空间内先自证无默认路由/DNS 失败/`http_code=000`）；`providersConfigured=false`、`409 PROVIDER_NOT_CONFIGURED`、`ready=ready` | **PASS** | 下表 #3/#4；`artifacts/web-mvp/t22-qa/qa-r30-wrapper-offline/`、`t22-qa-offline-smoke.log` |
| AC-063（浏览器矩阵） | Chrome/Edge/Firefox 当前版本 | 仅 Chromium（本机无 Firefox/Edge） | **NOT_RUN**（本机无环境；沿用 T21，非本轮范围） | 回合 29 记录 |
| T23（真实 Provider） | 真实 Tripo/说明书 AI 授权链路 | 未开始 | **NOT_RUN** | 需用户凭据与预算 |

**"原生冷启动"的明确判定（本轮要求写死）**：构建与运行**都发生在容器内的 Linux 环境**（不是 macOS→Linux 交叉编译）；`crossCompiled: true` 只表达 triple 差（`x86_64-unknown-linux-gnu` 构建机 → `x86_64-unknown-linux-musl` 目标），**不表达跨 OS**。产物是真实 x86_64 Linux ELF，在**真实 Linux 内核**上以 x86_64 ABI 冷启动并跑完整 smoke。因此 AC-064 的"原生冷启动"在**"目标 OS + 目标 ABI + 无源码/工具链依赖的冷目录"意义上成立**（§6 的立意是"不得以交叉编译退出码 0 替代运行证据"——本轮有真实运行证据，且 QA 独立重跑复现）；在**"物理 x86_64 CPU 原生执行"意义上不成立**（Rosetta 指令翻译）。该环境由**用户明确授权**（`state.yaml user_decisions` 2026-09-13），非 RD 单方面选择；残留风险已由 RD 声明（ADR-036 §2、implementation T22-13.7、validation §9），**判定为已声明的非阻断限制**。需要物理硬件证据时用同一参数化脚本在真机重跑即可。

**性能类结论核对**：RD 全部记录中**未引用**本环境的任何产品性能结论（仅有冷/热构建耗时 9 min/100 s，且明确标注"只作参考"）；QA 逐处 grep 确认。本环境不得用于 AC-061 类结论。

## 五处重点质疑的独立结论

### 1. `xtask/src/dist.rs` 动态依赖判据放宽 —— **不构成弱化证据**（逐行核对）

改动三处：① 采集门从 `target == host` 改为 `same_arch_and_os(host, target)`；② JSON 增加 `host`/`target`/`samePlatformAsBuild`；③ `readelf -d` 由"节选 20 行"改为**全文 + `DT_NEEDED` 计数**；失败原因的 `reason` 由 `cross-compiled` 改为 `cross-platform`。

- 采集到的 `file`/`ldd`/`readelf -d` 是**产物自身属性**（不依赖宿主 CPU），且该产物**就在同一环境被 smoke 真实运行**——不是拿编译成功当运行证据。
- **未制造"原生"假象**：`crossCompiled: true` 与 `samePlatformAsBuild: false` 均保留真值；控制台仍打印"跨构建"。
- **反例边界仍拒绝采集**（逐分支核对 13 行解析函数）：arm64→x86_64（含 macOS Intel 行）、macOS→Linux、Linux→Windows 均为"未采集"。解析启发式（≥4 段取 `parts[len-2]` 为 OS）对项目目标 triple 全对；`thumbv7em-none-eabihf` 一类异形 triple 会误解析，但本项目不会构建它们（且产物级证据仍由 smoke 兜底）。
- **macOS 侧结论不受影响**：host==target 时新判据恒真 → 与原路径等价（代码级证明）。macOS 产物已被用户要求清理，本轮未重测 macOS（不在本轮范围）。
- QA **不采信 `build-info.json` 自述**，独立在容器里自算：禁用模式（仓库根 / `apps/web/dist` / `node_modules` / `$HOME`）**全 0 命中**、`/build/home` 559（与自述一致）、`/Users/` 0、`node_modules` 字面量 0。
- **P4 边界（非缺陷）**：Linux 分支不做"仅系统库"硬失败（macOS 分支会 `bail`），结论依赖 `staticLinked` + `DT_NEEDED` 全文；对静态 musl 无实际影响。

### 2. `crossCompiled: true` 与"原生冷启动" —— 见上"明确判定"

### 3. 样例备份是重建件 —— **smoke 没有变成自证**（逐 blob 核实）

- tracked 备份缺 DB 的两条独立证据：`git ls-files` 无 DB 条目；工作树里连 `database/` 目录都不存在（macOS 侧的 T20 备份同样缺，见"交接"）。
- 重建路径用**仓库自己的 T20 生成器**（`cargo test … prepare_rehearsal_datadir`）+ **正式二进制 `backup`**，不是用 smoke 自己的产物回灌；宿主脚本在仓库备份不完整时**拒绝**用它覆盖构建目录里的完整备份（分支逐一核对）。
- QA 逐文件对比重建件 vs tracked：**各 11 个 blob**；**8 个逐字节相同**（含 GLB 2912 B = `a9f884c2…`、PDF 1216 B = `e18cf61a…`）；**3 个仅时间戳/UUID 字段不同**（大小相同：1962 / 2030 / 5181 B，差异字段为 `assetId`/`documentId`/`preparationId`）→ RD 描述与事实一致。
- 断言未退化：smoke 读**磁盘上的备份 manifest** 取 blob sha 集合 → 要求恢复出的 release `manifestSha256` ∈ 该集合 → 再比对**实际下载字节**的 sha256 与 manifest 声明一致；`restore` 自身重算 manifest/快照/11 个 blob 的 sha256 + 外键与引用。三方交叉核对仍在。
- **P4 记录**：身份断言的输入从"版本控制里那份 T20 备份"变为"同源新生成的备份"。因 DB 从未入库，任何 checkout 都只能如此。建议二选一（非阻断）：把样例 DB 以豁免/压缩形式入库，或把重建步骤固定为唯一入口（RD 已在 `docs/operations.md` 与 CI 落地后者）。

### 4. `cargo test --workspace` Linux 必现失败 —— 复现并定性；**不阻断本切片**，开 **BUG-013**

- **独立复现 3/3**：`cargo test -p everything-manual --test backup_restore -- --exact legacy_schema_backup_restores_and_migrates_automatically` → panic 于 `backup_restore.rs:2390`，`left: -1 / right: 0`，stderr 空，约 0.85 s。**QA 独立全量枚举**：容器内 `cargo test --workspace --no-fail-fast` = 37 个目标 / **557 passed / 1 failed / 2 ignored**，唯一失败目标即 `backup_restore`、唯一失败用例即此例（RD 记录同口径）。
- **QA 独立定性实验**（用**发布二进制**直接做，不经测试框架）：`listening on` 行出现后立即 `SIGTERM` → 退出码 **143**（3/3）；延迟 **≥100 ms** → **0**（3/3）；延迟 1/5/20/50 ms → 143（3/3）。→ **窗口≈50–100 ms**，窗口内 SIGTERM 走默认动作杀进程。
- 该用例的实质断言（v1→v7 自动迁移、数据保留、备份校验）**全部通过**，唯一失败项是"退出码 0"。
- 项目内已有同一判定：`crates/server/tests/storage.rs:1817-1819` 写明"`listening on` 在 SIGTERM 处理器注册前打印…**不是产品缺陷**"，并用 300 ms settle 规避；`backup_restore.rs`/`config_cli.rs` 未 settle（后者用例此前已有 HTTP 交互，故未命中）。
- **定性**：**测试侧启动竞态为主**（读到 listening 行后零等待即发信号）+ **产品侧 50–100 ms 窄窗口**（`listening on` 打印先于 `shutdown_signal()` 被首次 poll）。数据安全影响可忽略：窗口内无写入在途，SQLite 锁由内核回收，执行器租约 120 s 到期恢复（T21 已用 SIGKILL 覆盖更坏情形）。**稳态 SIGTERM 优雅退出 QA 实测正常**（裸容器：退出码 0 + `收到终止信号，服务已停止` + 排他锁释放）。
- **是否阻断本切片**：T22 三条必选 AC 全过，本缺陷**不违反任何 T22 必选 AC**（AC-002 的停服重启步骤用 SIGKILL 完成，比"优雅"更强，故不受影响；AC-064/AC-059 与本缺陷无关）→ 按 `collaboration.md` §4"违反当前必选验收项的问题才阻断当前切片 PASS"，**不阻断 T22 切片**。**但它阻断全量发布门禁**（validation §8"0 个未关闭验收缺陷"），并使 Linux 侧 `cargo xtask check` 红灯（本轮实测），因而 CI 的 `check` job（`ubuntu-latest`）**很可能**同样红灯（`dist-musl` 依赖 `check`，命中则连坐）——该判断**基于容器内在 Rosetta 下必现的实测**，**未在真实 x86_64 Linux 上验证**；CI 尚未推送触发，故目前未暴露。→ 开 **BUG-013（P3 / OPEN）**，归属由协调者/PM 裁定（T20/T21 卡或 T22 后小卡）。

### 5. 两个 shell 脚本改动 —— **没有跳过、短路或弱化 smoke 的任何一步**（逐项核对）

- `step()` 用 `${PIPESTATUS[0]}` 取**真实退出码**（不再是 `| tee` 后可能被吞的状态），失败经 `set -e` 传播；宿主脚本在失败路径**先回收日志再非零退出**（`RUN_RC` 逐层传出）——日志回收不掩盖失败。
- **增强**（非弱化）：`rustup toolchain install` 去掉 `2>/dev/null || true`（原来失败会被吞）；`rustup target add` 变成受检步骤；离线分支要求 `cargo/rustc/file/readelf` 存在，缺失即 `exit 1`；断网自证在**同一命名空间**内先做（路由表/DNS/HTTP 三重），随后整条 smoke 才运行；`readelf` 由节选改为全文 + DT_NEEDED 计数。
- 挂载点/缓存改动是**构建环境修复**（`/src/everything-manual`），**未动 dist 的隔离扫描子串语义**（QA 复核禁用模式与 hits=0 均不变）。
- `--exclude '*.log'`：QA 核对仓库**未跟踪任何非 `artifacts/` 的 `.log`**（`git ls-files '*.log'` 为空）→ 不会漏拷源码，只是避免 `--delete` 清掉上一轮日志。
- **P4 隐患（非阻断，记录待改）**：① `readelf -d … | grep -c "(NEEDED)" || true` 在 `readelf` 本身失败时会打印 `0` 且返回成功（权威结论来自 `dist.rs` 的 `file` 判断与全文输出，`file` 失败是 fatal）→ 建议不要在工具失败时 `|| true`；② 失败路径仍会 `rsync` 回收 `dist/`，可能回收**上一轮的旧产物**（退出码已非零、日志可见，但建议失败路径记录产物 sha256 以免误读）；③ `--keep` 两个分支行为相同（都保留 `BUILD_DIR`），仅打印不同，名不副实。

## QA 亲跑命令与结果（本回合；日志在 `artifacts/web-mvp/t22-qa/`）

| # | 命令 | 结果 |
| --- | --- | --- |
| 1 | 宿主 `shasum -a 256` + 容器内 `sha256sum`/`stat` | `77b38c91…` / 28 236 728 B，与 `SHA256SUMS`、`build-info.json` 一致 |
| 2 | 容器内 `file` / `ldd` / `readelf -d` + **QA 自算**禁用模式计数 | `ELF 64-bit LSB pie executable, x86-64 … static-pie linked`；`statically linked`；`DT_NEEDED=0`；禁用模式 0/0/0/0、`/Users/` 0、`node_modules` 字面量 0 → `t22-qa-verify-static.log` |
| 3 | `docker run --network none … container-linux-musl.sh --skip-dist --offline-only`（QA 直接调用容器脚本） | exit 0；31 条 `[检查]` **与 RD 原始离线轮归一化后逐条一致**（diff 为空）→ `t22-qa-offline-smoke.log` |
| 4 | `bash scripts/linux-musl.sh --offline-only --arch amd64`（官方宿主入口） | exit 0，证据回收成功；QA 已把 RD 原始证据按快照**逐字节还原** → `qa-r30-wrapper-offline/`（含 `wrapper-console.log`） |
| 5 | 容器内 `cargo xtask dist --target x86_64-unknown-linux-musl --check-reproducible`（QA 独立重跑，含 `cargo clean -p` 重链接） | exit 0；**两次独立构建同哈希 `77b38c91…`**；QA 轮 `build-info.json` 与交付件**除 `builtAt` 外逐字段一致** → `qa-r30-reproducible-dist-smoke.log`、`qa-r30-build-info.json` |
| 6 | `cargo test -p everything-manual --test backup_restore -- --exact legacy_schema_backup_restores_and_migrates_automatically`（×3） | **3/3 FAILED**（`left -1`，0.85 s）→ `t22-qa-failing-test.log` |
| 7 | 发布二进制 SIGTERM 窗口扫描（0/1/5/20/50/100/200/300 ms ×3） | 0–50 ms → 143（3/3）；100–300 ms → 0（3/3）→ `t22-qa-sigterm-window.log`、`t22-qa-sigterm-window-scan.log` |
| 8 | 容器内 `cargo xtask smoke-bootstrap --binary <release>` | exit 0；7/7，含 `GET /api/unknown -> 404 application/json`、未知静态资源 `404 text/plain` → `t22-qa-smoke-bootstrap.log` |
| 9 | **QA 新增**：`qa-r30-coldstart-bare-container.sh`（裸 `alpine:3.20`，无 node/python/cargo/git、无源码，只挂一个二进制） | exit 0；`init`（含**权限过宽的密码文件被 fail-closed 拒绝**）、`serve` 冷启动、内嵌 `/`+JS+CSS+PDF cmaps、`health/live|ready`、`/api/unknown` 404、SPA 回退 200、未知静态资源 404；SIGTERM 优雅停服 → `t22-qa-bare-coldstart.log`。**这是 §7 第 7 步"无 Node/Python/源码环境"在 Linux 上的直接证据**（不再是推断） |
| 10 | 容器内 `cargo test --workspace --no-fail-fast`（QA 独立重跑） | **37 个测试目标**：**557 passed / 1 failed / 2 ignored**；唯一失败目标＝`backup_restore`（9 passed / 1 failed / 1 ignored），唯一失败用例＝`legacy_schema_backup_restores_and_migrates_automatically` → 与 RD 枚举**完全同口径** → `t22-qa-workspace-test.log`（macOS 基线 558/0/2，`crates/**` 本轮未改） |

## 缺陷

### BUG-013 · Linux 上 `serve` 启动窗口内 SIGTERM 不优雅退出，导致 `backup_restore.rs` 一例在 Linux 必现失败

- 严重度／状态：**P3 / OPEN**（跨卡缺陷；**不阻断 T22 切片**，**阻断全量发布门禁**与 Linux 侧 `cargo xtask check`／CI `check` job）
- 对应 REQ / UI / AC：validation-release **§5 服务终止合同**（"服务终止先停止领取、完成必要短写入并退出"）；**非** T22 任一必选 AC 的违反（AC-002/AC-059/AC-064 均已独立通过）
- 环境与输入：容器 `linux/amd64`（Rosetta，内核 6.12.76-linuxkit）；发布二进制 `77b38c91…`；容器内 rustc/cargo 1.98.1
- 复现步骤：① 容器内 `cargo test -p everything-manual --test backup_restore -- --exact legacy_schema_backup_restores_and_migrates_automatically`（3/3 失败）；② 用发布二进制 `serve`，读到 `listening on http://…` 行后**立即** `kill -TERM` → 退出码 143；延迟 ≥100 ms 再发 → 退出码 0
- 期望与实际：期望 `serve` 收到 SIGTERM 后优雅退出（退出码 0）；实际在启动后约 50–100 ms 窗口内被信号默认动作杀死（`status.code()=None` → 测试记 `-1`）
- 证据路径：`artifacts/web-mvp/t22-qa/t22-qa-failing-test.log`、`t22-qa-sigterm-window.log`、`t22-qa-sigterm-window-scan.log`、`artifacts/web-mvp/t22-rd/linux/regression/xtask-check.log`、`cargo-test-workspace-nff.log`
- 回归范围：`crates/server/src/config/commands.rs::run_serve`（`listening on` 打印与 `shutdown_signal()` 首次 poll 的次序）；`crates/server/tests/{backup_restore,config_cli}.rs` 的 `ServeProcess::terminate`（与 `storage.rs:1817` 的 settle 口径对齐）
- 修复建议（RD/PM 择一）：① **测试侧** settle（与 `storage.rs` 同口径，最小改动、不改产品行为）；② **产品侧** 把 SIGTERM/SIGINT 处理器的注册提前到打印 `listening on` 之前（更彻底，但任何产品代码改动都会使已交付的 macOS/Linux T22 证据需按 T22 口径重跑）
- RD 修复摘要引用：待 RD 认领；QA 复验结果与日期：待修复后复验

### 非阻断观察（P4；供协调者/RD/PM）

1. **OB-18 · "原生"措辞精度**：`docs/operations.md` §1/§2 与 `scripts/linux-musl.sh` 头注释写"原生 Linux x86_64 构建与运行"，未在同一句标注 Rosetta 翻译（ADR-036 / implementation T22-13.7 / validation §9 已标注）。建议文档补一句限定，避免读者误认为物理 x86_64 硬件证据。
2. **OB-19 · 样例备份身份**：见"质疑 3"的 P4；建议入库或以重建为唯一入口（后者已落地）。
3. **OB-20 · `readelf … || true` 可掩盖工具失败**；**OB-21 · 失败路径回收的 `dist/` 可能是上一轮产物**；**OB-22 · `--keep` 名不副实**：见"质疑 5"的 P4。
4. **OB-23 · T22 smoke 未断言"未知 /api 的 JSON 404"与"未知静态资源非 HTML200"**：这两条是 AC-002 的字面判据，实际由 `smoke-bootstrap`（同一发布二进制）覆盖。建议把这两条断言并入 `xtask smoke`，使 AC-002 的判据由单一入口闭合（当前证据链成立，仅属口径收敛）。
5. **OB-15…OB-17 沿用**（回合 25/28/29）。
6. **artifact 目录不可重定向（P4）**：`scripts/linux-musl.sh` 的 `ARTIFACT_DIR` 硬编码为 `artifacts/web-mvp/t22-rd/linux`，QA 复跑会覆盖 RD 同名证据；本轮 QA 以"先快照、跑完还原"处理（已逐字节还原校验）。建议支持 `EM_LINUX_ARTIFACT_DIR` 覆盖。

## 未覆盖边界（不冒充通过）

1. **物理 x86_64 硬件**：本轮证据来自 arm64 宿主上的 Rosetta 翻译执行；物理机行为与任何性能结论未验（脚本已参数化，可真机重跑）。
2. **T23 真实 Provider**：真实 Tripo / 说明书 AI 协议、余额账单、真实 CDN 签名 URL、真实模型效果与预算 —— 未验（需用户凭据与预算）。
3. **AC-063 浏览器矩阵**：本机无 Firefox/Edge；Safari 未列入支持。
4. **签名与公证**：Linux/macOS 均未做（需用户账号）；分发说明已写在 `build-info.json.deployment`/`signature`。
5. **`--check-reproducible` 的独立性边界**：`cargo clean -p` + 重链接（依赖不重编），与 macOS 侧同口径（ADR-035 §4）；不能覆盖依赖编译层面的非确定性。
6. **Windows / Intel macOS**：未构建未运行（未支持，符合 §6"未跑平台不贴标签"）。
7. **全项目**：T23 未交付 → MVP 未完成；本 PASS 只覆盖 T22 切片（且其中 macOS 半边的 QA 结论仍待协调者按流程确认/回溯）。

## 非代码知识与限制（提炼）

- **"原生运行"要拆成三层说**：目标 OS 内核 / 目标 ABI 用户态 / 物理 CPU。本轮只满足前两层；把三层写清楚比写"原生"更有用（且能被后续真机证据自然替换）。
- **启动协议行先于信号处理器注册**是本项目已知时序（`storage.rs:1817`）；凡是"读到协议行即发信号"的测试都必须先 settle，否则在慢环境（容器/Rosetta）必然偶发到必现失败。**新增此类测试时按 `storage.rs` 的写法做 settle**。
- **重建型 fixture 的验收问法**：不要问"是不是自证"，要分三步问——① 生成器是否独立于被测路径；② 差异是否只落在非语义字段（时间戳/UUID）；③ 断言是否仍跨三方（备份清单 ↔ 恢复结果 ↔ 实际字节）。三条都满足即等价，但"与版本控制里那份逐字节相同"这一条会丢，应显式登记。
- **证书/信任与静态链接**：静态 musl 不含系统 CA，需要 rustls 自带根；本轮 smoke 走 loopback HTTP，**真实 HTTPS 未被本轮证据覆盖**（§6 要求"在冷环境验证真实 HTTPS"属于真实链路范畴，归 T23 与运维声明）。
- **容器挂载点命名**：dist 的隔离扫描把仓库根当**子串**搜，短名挂载点（`/build`、`/work`）会与 remap 目标或依赖 panic 路径自撞——这是**构建环境问题**，正解是换深路径而不是放宽扫描（ADR-036 已固化）。
- **复跑取证的纪律**：官方入口的 `ARTIFACT_DIR` 硬编码，QA 复跑前必须先快照、复跑后按快照还原并做 `diff -r` 校验（本轮已做，RD 证据逐字节一致）。

## 回合历史与交接

| 回合 | 日期 | 范围 | 任务 | PRD 修订 | 结果 | 报告 |
| --- | --- | --- | --- | --- | --- | --- |
| 27 | 2026-09-13 | 缺陷复验 | BUG-009 / BUG-010 | 2 | PASS | 本文件第 27 节 |
| 28 | 2026-09-13 | 缺陷复验 + 结构性保障审计 | BUG-011 / BUG-012 | 2 | PASS | 本文件第 28 节 |
| 29 | 2026-09-13 | 切片验收（回归/安全/故障矩阵） | T21 | 2（ui_revision 2） | PASS | 本文件第 29 节 |
| 30 | 2026-09-13 | 切片验收（多平台发布 · Linux 半边） | T22 | 2（ui_revision 2） | **PASS**（+ BUG-013 未关闭，跨卡 P3） | 本节 |

- **交接给协调者**：(a) 本轮 `result: PASS`；`accepted_ac_ids` 建议：`["AC-064（Linux 半边；物理 x86_64 硬件为已声明限制）", "AC-002", "AC-059（Linux 侧）"]`，`task_id: T22`、`qa_round: 30`、`prd_revision: 2`；**是否把 T22 整卡移入 `accepted_tasks` 由协调者决定**：本轮只派发了 Linux 半边，macOS 半边的 QA 结论（AC-064 macOS）尚未由 QA 回合单独出具（现有为 RD 自证证据 + 本报告顺带复核的代码级结论）。(b) `qa_history` 追加 `{round: 30, scope: slice, task_ids: [T22], prd_revision: 2, result: PASS}`。(c) **`open_defects` 需新增 `BUG-013`（P3）**，并请裁定归属：T20/T21 缺陷单、或 T22 后小卡、或测试侧硬化小卡；修复路径见 BUG-013 条目（任一产品代码改动都会触发两平台 T22 证据重跑）。(d) **口径提示（需协调者确认）**：按 `collaboration.md` §4"违反当前必选验收项才阻断切片 PASS"，BUG-013 不阻断 T22 切片；若协调者的口径是"任何未关闭缺陷都不放行切片"，则本轮应改判 **FAIL** 并把 BUG-013 交 RD——QA 已把两种口径的事实与证据备齐（本轮建议按前者，理由是缺陷不落在 T22 允许范围与 AC 映射内，且产品数据路径无影响）。(e) `carry_over` 建议新增：**T22 重跑前置条件**（`artifacts/web-mvp/t20-rd/sample-backup` 现缺 `database/manual.sqlite3`，macOS 侧复跑需先按 `docs/operations.md` §1 手工重建）、OB-18…OB-23。
- **交接给 RD**：必须修 **BUG-013**（路径二选一，见条目）。非阻断建议：OB-20/OB-21/OB-22（脚本失败路径与 `--keep` 语义）、OB-23（把未知路径断言并入 `xtask smoke`）、OB-19（样例备份入库或固化重建为唯一入口）、`EM_LINUX_ARTIFACT_DIR` 可覆盖。**请勿在 QA 复验前修改断言口径或阈值**（`backup_restore.rs` 若走测试侧 settle 路线，改动需在复验时逐字核对只加等待、不动断言）。
- **交接给 PM**：OB-18（文档措辞精度，是否需要在发布说明中声明"Linux 证据取自 x86_64 容器（Rosetta）"）；物理 x86_64 真机证据是否列入发布要求；AC-063（Firefox/Edge）与真实 HTTPS 的承接位置。
- **llmdoc 更新清单（本回合实际写入）**：① 本报告（回合 30：AC 矩阵、五处质疑的独立结论、BUG-013、OB-18…OB-23、未覆盖边界、非代码知识）；② `llmdoc/decisions.md` 追加「T22 Linux 半边验收知识（QA 回合 30）」——"原生运行"三层表述、启动协议行竞态（含 BUG-013 与 settle 纪律）、重建型 fixture 验收三步问法、复跑取证纪律、挂载点命名与静态链接等跨卡复用结论；③ `llmdoc/validation-release.md` **§9 追加一行 QA 回合 30 执行记录**（§1–§8 命令合同语义**未改动**）。未修改任何 AC 判据与阈值。
- **QA 产出与证据**：`artifacts/web-mvp/t22-qa/`——`qa-r30-coldstart-bare-container.sh`（QA 新增脚本）、`t22-qa-verify-static.log`、`t22-qa-offline-smoke.log`、`qa-r30-wrapper-offline/`（`offline-smoke-console.log`、`network-none-probe.log`、`offline-steps.log`、`offline-host-steps.log`、`wrapper-console.log`）、`t22-qa-failing-test.log`、`t22-qa-sigterm-window.log`、`t22-qa-sigterm-window-scan.log`、`t22-qa-smoke-bootstrap.log`、`t22-qa-bare-coldstart.log`、`qa-r30-reproducible-dist-smoke.log`、`qa-r30-build-info.json`、`t22-qa-workspace-test.log`。**本轮未改动任何生产代码、测试断言、PRD 或 `state.yaml`；RD 的 Linux 证据目录已按快照逐字节还原（`diff -r` 通过）；本轮启动的容器/进程已结束（`docker ps -a`、`pgrep` 复核为空）。**
