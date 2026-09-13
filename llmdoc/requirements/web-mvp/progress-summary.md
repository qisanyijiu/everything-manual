# web-mvp 进度与实现总结（暂停交接）

状态：**暂停中**，2026-09-13 由主协调会话整理（第二次暂停）；暂停点＝ T22 待收尾（Linux 半边已 QA PASS，macOS 半边证据待重跑；另 1 个未关闭缺陷 BUG-013）；T23 未开始。
依据：`state.yaml`、`qa-report.md`（回合 1–30）、`implementation.md`（§T01–§T22，含 §T22-13）、`llmdoc/decisions.md`（ADR-001~036）、`artifacts/web-mvp/**`（含 `t22-rd/linux/`、`t22-qa/`）。
本文件是**交接快照**，不替代 state.yaml（协调状态）与 qa-report（验收证据）；细节数字以各报告与实测为准。

## 1. 一句话状态

**T01–T21 二十一个切片经 QA 独立验收 PASS；T22 的 Linux 半边已跑通并经 QA 回合 30 独立验收 PASS（AC-064 Linux / AC-002 / AC-059 Linux 三条必选全过），但 T22 整卡记 `needs_retest`：macOS 半边证据相对当前代码已过期待复跑，另有 1 个未关闭缺陷 BUG-013（P3，不阻断切片、阻断发布门禁）；T23（真实 Provider 链路）未开始。** 产品在 fixture 环境下已具备可运行的端到端闭环：建物品 → 上传资料 → 浏览器 PDF 准备 → 报价与确认 → 后台双分支生成 → 草稿复核与热点校准 → 发布不可变版本 → 阅读/导出/备份恢复。

Linux x86_64-unknown-linux-musl 已取得**真实运行证据**（此前受 Docker 环境阻塞）：`static-pie linked`、`ldd: statically linked`、`DT_NEEDED=0`，二进制 sha256 `77b38c91327c4d5c697c1fccb48054a827034ee088167cb38ad3249a3d338246`（28 236 728 B），两次独立构建同哈希，`smoke` 7 步与 `--network none` 离线 smoke 均全过。限定：执行环境是 Rosetta 翻译的 x86_64（宿主 arm64），故「原生」在目标 OS＋目标 ABI＋冷目录意义上成立，在物理 x86_64 CPU 意义上不成立；性能类结论零引用。

## 2. 切片进度

| 卡 | 内容 | 状态 | QA 回合 |
| --- | --- | --- | --- |
| T00 | PM 明细 PRD + UI 交互（PRD 修订 2 / ui_revision 2） | 完成 | — |
| T01 | 工作区、合同生成、最小单二进制 | accepted | 1 |
| T02 | CLI / 配置 / 日志 / data-dir 排他锁 | accepted | 2 |
| T03 | SQLite 迁移与 Repository（schema v7 起点 v1） | accepted | 3 |
| T04 | 认证、会话、CSRF/Origin、限速、统一错误、If-Match | accepted | 4 |
| T05 | 无费用 HTTP fixture 设施 + 原创样例资产 | accepted | 5 |
| T06 | 流式上传与安全资产服务（Range/HEAD/ETag/去重/孤儿隔离） | accepted | 6 |
| T07 | 物品、说明书绑定、照片与视图、乐观锁 | accepted | 7 |
| T08 | React 应用框架（路由、typed fetch、Query、401/412 恢复） | accepted | 8 |
| T09 | 浏览器 PDF 准备与续传（PDF.js 同版本本地资源） | accepted | 9 |
| T10 | 持久任务执行器（DAG、租约/epoch、退避、SIGKILL 恢复、failpoint） | accepted | 10→11（修复后） |
| T11 | 报价快照、云端告知确认、冻结输入、幂等建单、费用分列 | accepted | 12→13（修复后） |
| T12 | Tripo v3 适配器（对 fixture 的字节级协议验证） | accepted | 14 |
| T13 | 模型下载、GLB 校验、不可变 model revision、SSRF 防护 | accepted | 15 |
| T14 | 说明书 AI 适配与证据校验（批次、引用校验、注入防护） | accepted | 16 |
| T15 | 双分支组装草稿、cancel/retry/reconcile、审计 | accepted | 17 |
| T16 | 资料库与新建向导 UI（五步向导 + 视图排列 + 报价确认） | accepted | 18→19（修复后） |
| T17 | 任务中心与可行动错误（轮询、unknown 对账、恢复入口） | accepted | 21 |
| T18 | GLB 阅读器与资源恢复（坐标层、context lost/restore、资源释放） | accepted | 22→23（修复后） |
| T19 | 热点校准、步骤联动、知识确认、发布不变量 | accepted | 24 |
| T20 | 导出、备份、恢复、迁移门禁（backup/restore CLI 落地） | accepted | 25（+26–28 脱敏修复轮） |
| T21 | 产品回归、安全与故障矩阵（§3 矩阵全行 + 五个崩溃断点） | accepted | 29 |
| T22 | 多平台单二进制发布 | **部分**：Linux 半边 accepted（PASS）；macOS 半边证据待复跑 → 整卡 `needs_retest` | 30 |
| T23 | 授权真实链路与最终 QA | 未开始 | — |

另有独立缺陷修复轮：回合 10/12/18/22/26 为 FAIL，修复后 11/13/19/23/27–28 复验 PASS。

## 3. 已实现内容

### 3.1 代码规模（2026-09-13 实测）

- Rust：`crates/` + `xtask/` 共 179 个 `.rs`，约 98.5k 行。
- 前端：`apps/web/src/` 共 89 个 `.ts/.tsx`，约 20.8k 行。
- 迁移：7 个（`migrations/0001…0007`，只追加）。
- API：31 条路由（`contracts/openapi.json`，由 Rust DTO 生成，前端类型由它生成，禁止手抄）。

### 3.2 后端（`crates/server/src/`）

| 模块 | 职责 |
| --- | --- |
| `config/` | CLI（init/serve/check/backup/restore）、配置优先级、日志脱敏、data-dir 排他锁、退出码 0/1/2/3/4/5/6/7 |
| `storage/` | SQLx/SQLite（WAL、foreign_keys、busy_timeout=5s、synchronous=FULL、池上限 4）、迁移与 schema 门禁、repository、`tx.rs` 统一 `begin_write`（BEGIN IMMEDIATE） |
| `http/` | 路由、DTO、统一错误（`error.code/message/details/requestId`）、ETag/If-Match（428/412）、分页、静态资源与 SPA 分离 |
| `assets/` | 流式上传（magic/像素/体积校验）、blob 去重与原子落盘、Range/HEAD/ETag、孤儿隔离、GLB 结构校验、SSRF 安全下载 |
| `jobs/` | 阶段 DAG、租约/epoch、退避、限并发、恢复、attempt 三态、failpoint（仅测试构建）、pipeline 组装与 control（cancel/retry/reconcile） |
| `providers/tripo` | v3 上传/多视图生成/查询/状态归一化/计费；付费 POST 无通用自动重试 |
| `providers/manual_ai` | Responses 形态请求、≤5 页批次、严格 JSON Schema、refusal/incomplete 处理、引用校验、注入防护 |
| `generation/` | 报价快照、价格目录版本、冻结输入、预留/结算、幂等建单 |
| `drafts/` `releases/` | 草稿聚合与受限 PATCH、发布事务与不变量、manifest 冻结 |
| `backup/` | release 导出（白名单 + 无密钥/绝对路径）、停服快照（VACUUM INTO，含 WAL 与全部引用 blob）、restore 校验与新空目录 |
| `redaction.rs` | 统一临时 URL 脱敏（写入/读取/备份/导出同源） |

### 3.3 前端（`apps/web/src/features/`）

`shell`（布局/导航/通知/错误边界）、`auth`（登录与会话恢复）、`library`（资料库、物品表单）、`import`（五步向导、上传与视图排列、准备、报价与确认）、`jobs`（任务中心、费用分列、对账）、`viewer`（R3F 懒加载阅读器、坐标层 asset-root、WebGL 恢复、资源账本）、`manual`（校准工作区、知识确认/修订、发布、版本）、`settings`（配置状态只读）。断点：≥1280 三栏 / 768–1279 主栏+侧栏 / <768 单栏+抽屉。

### 3.4 工程命令（`cargo xtask`，根 `.cargo/config.toml` 别名）

`contracts` / `contracts --check`（生成与漂移检查）、`check`（fmt+clippy+测试+前端 lint/typecheck/test+合同）、`dist --target <triple> [--check-reproducible]`（产出 binary+SHA256+licenses+build-info+动态依赖清单）、`smoke --binary`（正式包 7 步）、`smoke-bootstrap --binary`（T01 最小检查）。

### 3.5 测试与验收资产

- Rust 集成测试目标 28 个（含 QA 独立用例 `qa_t*.rs`、`redaction_*`）。
- Playwright e2e：14 个 spec（`apps/web/tests/e2e/`），含真实链路用例（不拦截草稿/模型字节）。
- 最近全量结果：Rust workspace 558 passed / 0 failed / 2 ignored；e2e 88 passed / 0 failed / 0 skipped（T21）；Vitest 约 130 项。
- fixture：`crates/test-support`（仅 dev-dependency）+ `tests/fixtures/`（原创 GLB/文字 PDF/扫描 PDF/图片与脱敏响应，含许可记录）。
- 验收证据：`artifacts/web-mvp/`（77 个目录、约 74 MB：各卡原始日志、截图、可复跑脚本）。

### 3.6 发布产物（macOS，2026-09-13）

`dist/aarch64-apple-darwin/everything-manual`，25,112,080 B，sha256 `ab693cc3c881a6ee355fd969d24a419c6221b010aeb20a296c19b6553d198333`（三次构建一致）；同目录含 `SHA256SUMS`、`licenses.json`（203 Rust + 40 前端包）、`build-info.json`、`dynamic-dependencies.txt`（仅 4 个系统库）。签名与公证未做。

## 4. 缺陷闭环（BUG-001~012 已关闭；BUG-013 未关闭）

| 缺陷 | 级别 | 主题 | 结局 |
| --- | --- | --- | --- |
| BUG-001 | P2 | limiter 单测真实时钟依赖（时序 flake） | T06 轮修复（可注入时钟），回合 6 关闭 |
| BUG-002 | P3 | 通知条覆盖顶栏吞点击 | T09 轮修复，回合 9 关闭 |
| BUG-003 | P2 | 30 分钟等待预算锚点错误（真实轮询路径不可达 needs_input） | 修复后回合 11 关闭 |
| BUG-004 | P2 | 报价回读未合并确认/消费状态 | 修复后回合 13 关闭 |
| BUG-005 | P2 | 资料库行 1280/1366px 身份列 0 宽、行高 358px | 修复后回合 19 关闭 |
| BUG-006 | P2 | 并发写 deferred 事务读→写升级致 SQLITE_BUSY（400 并发 237×500） | 统一 `begin_write`，回合 20 关闭 |
| BUG-007 | P2 | 手动重建后状态不恢复、8s 误报上下文不可用 | 修复后回合 23 关闭 |
| BUG-008 | P3 | 备份快照残留供应商签名 URL | 脱敏，回合 26 关闭 |
| BUG-009 | P2 | 传输错误文本带签名 URL 进 lastError/详情/日志 | 统一入口 + `without_url()`，回合 27 关闭 |
| BUG-010 | P4 | 脱敏扫描吞掉紧邻 URL 的字符 | 回合 27 关闭 |
| BUG-011 | P3 | 备份对 JSON 列整串替换致 needsInput 静默为空 | JSON 感知脱敏，回合 28 关闭 |
| BUG-012 | P3 | `usage_json` 写侧未脱敏 | 回合 28 关闭 |
| **BUG-013** | **P3** | **`serve` 启动窗口（≈50–100 ms）内 SIGTERM 不优雅退出** | **OPEN（回合 30 开）** |

**BUG-013 详情**（跨卡，T22 回合 30 发现）：`run_serve` 打印 `listening on …` 之后才把 `shutdown_signal()` 交给 `with_graceful_shutdown`，tokio 的 SIGTERM 处理器在被 poll 时才注册；窗口内 SIGTERM 走默认动作杀进程（期望退出码 0，实得 -1）。表现为 Linux 容器内 `cargo test --workspace` **1 例必现失败**：`backup_restore.rs::legacy_schema_backup_restores_and_migrates_automatically`（全量枚举 37 目标 557 passed / 1 failed / 2 ignored）。

- QA 独立复现 3/3，并用**发布二进制**直接扫窗口：0–50 ms → 退出码 143（3/3），≥100 ms → 0（3/3）；稳态 SIGTERM 优雅退出正常。
- 定性：**测试侧启动竞态为主**（读到 listening 行后零等待即发信号）＋**产品侧 50–100 ms 窄窗口**。数据安全影响可忽略（窗口内无写入在途，SQLite 锁由内核回收，执行器租约 120 s 到期恢复）。
- 项目内已有同源判定：`crates/server/tests/storage.rs:1817-1819` 写明此事「**不是产品缺陷**」并用 300 ms settle 规避；`backup_restore.rs` / `config_cli.rs` 缺该 settle。
- **不阻断 T22 切片**（不违反任何 T22 必选 AC），**阻断发布门禁**（validation §8「0 个未关闭验收缺陷」），并使 Linux 侧 `cargo xtask check` 红灯 → CI `check` job 很可能连坐（未在真机验证，CI 未推送）。
- 修复路径：测试侧 settle（不改变发布二进制）或产品侧提前注册处理器（触发两平台 T22 证据重跑），待 RD 评估择一并写明理由。

脱敏主题经四轮（008→009/010→011/012）后做了**穷举式收口**：仓储层写入审查测试（`tests/redaction_surface.rs`）、备份按表×全列自动兜底、全库 canary 扫描；口径见 ADR-033，结构见 ADR-034。

## 5. 如何运行

```sh
# 开发
npm --prefix apps/web ci
npm --prefix apps/web run dev                       # Vite 5173，代理 /api → 8080
cargo run -p everything-manual -- init  --data-dir ./var/dev     # 交互设置管理员密码
cargo run -p everything-manual -- serve --data-dir ./var/dev

# 验证
cargo test --workspace
cargo xtask check
npm --prefix apps/web run test:e2e                  # 自管测试后端与临时 data-dir

# 发布（macOS）
cargo xtask dist --target aarch64-apple-darwin
cargo xtask smoke --binary "$PWD/dist/aarch64-apple-darwin/everything-manual"

# 备份 / 恢复（须先停服）
./everything-manual backup  --data-dir ./manual-data --out ./backups/<新路径>
./everything-manual restore --from ./backups/<备份> --data-dir ./restored-data
```

## 6. 未完成与已知限制

1. **T22 macOS 半边证据待复跑（未完成）**：`xtask/src/dist.rs` 是在 macOS 证据产出之后才改的（新增 `host`/`target`/`samePlatformAsBuild` 字段；动态依赖判据由 `target == host` 改为 `same_arch_and_os`；`readelf -d` 由节选 20 行改为全文 + `DT_NEEDED` 计数）。QA 逐行核对后判定该放宽**不构成弱化证据**（采集的是产物自身属性，且产物体在同一环境被 smoke 真跑），但已录 macOS 证据相对当前代码**过期**，需在现行代码下复跑 `dist --check-reproducible` 并比对二进制 sha256 是否仍为 `ab693cc3c881a6ee355fd969d24a419c6221b010aeb20a296c19b6553d198333`，据此判定是否需整轮重验。这是 T22 整卡记 `needs_retest` 的原因之一。
2. **BUG-013 未关闭**（详见 §4）：P3，不阻断 T22 切片，阻断发布门禁；并可能使 CI `check` job 红灯。
3. **Linux「原生运行」的限定**：构建与运行都在容器 Linux 内完成（非 macOS 交叉编译），但 x86_64 由 **Rosetta 翻译执行**（宿主 arm64）。「原生」在目标 OS＋目标 ABI＋无源码/工具链冷目录意义上成立，在**物理 x86_64 CPU** 意义上不成立。该环境由用户明确授权。需要物理硬件证据时，用同一参数化脚本在真机重跑即可。**性能类结论不得引用本环境。**
4. **T23 未开始**：需要用户提供 Tripo / 说明书 AI 凭据、真实型号资料与一次生成预算；缺失则记 BLOCKED。
5. **UI-060 网页导出按钮**未实现（端点与生成类型已就绪）；登记为 T21/T22 之后的独立前端小项。
6. **浏览器矩阵**：仅 Playwright Chromium 148；Firefox/Edge 本机未安装，AC-063 未满足（T22/T23 承接）。
7. **热点级 `cameraPose`** 未实现（视角保存落在步骤级 `stepPoses`）：字面偏离 `contracts.md` §2，不影响必选 AC，建议 PM 在合同留痕。
8. UI-032（字面倒计时）、顶栏任务计数徽标未做；签名/公证未做；模型加载耗时与 ≥5 分钟连续旋转未测（旋转 p95 实测 17.7 ms ≤ 33 ms 已达标）。
9. T12/T13 的真实形态收敛项（upload token/计费字段候选、下载允许域）待 T23。
10. **T22 脚本层 P4 隐患**（QA 回合 30 记录，非阻断）：`readelf -d … | grep -c "(NEEDED)" || true` 在工具本身失败时会打印 0 且返回成功；失败路径仍回收 `dist/` 可能带回上一轮旧产物；`--keep` 两分支行为相同、名不副实；`ARTIFACT_DIR` 硬编码。
11. **样例备份身份**（OB-19）：tracked 样例备份目录缺 DB（`.gitignore` 的 `*.sqlite3`），本轮 smoke 改用容器内**同源重建件**；QA 逐 blob 比对 11 个中 8 个逐字节相同、3 个仅时间戳/UUID 不同，判定**不是自证**，但身份断言已从 tracked 件变为重建件，需登记。

## 7. 挂起决策与待用户/PM 动作

| 事项 | 归属 | 说明 |
| --- | --- | --- |
| ~~Docker 恢复~~ | ~~用户~~ | **已解除**（2026-09-13）：重启 Docker Desktop 即恢复，Linux 验证已执行并 QA PASS |
| T23 凭据与预算 | 用户 | 无授权则 T23 记 BLOCKED，不伪造真实链路结论 |
| UI-032 / cameraPose / OB-16 措辞张力 | PM | 需在 PRD/合同留痕或裁定，不由 RD 自行扩大 |
| BUG-013 归属与修法（测试侧 settle vs 产品侧提前注册） | RD 评估 → 协调者确认 | 测试侧修不改变发布二进制；产品侧修触发两平台 T22 证据重跑。项目已有 `storage.rs:1817` 同源先例 |
| OB-18「原生」措辞精度、OB-19 样例备份身份、OB-20/21/22 脚本失败路径、OB-23 未知路径断言并入 `xtask smoke` | PM/协调者 | QA 回合 30 记录的非阻断观察，待裁定是否立小卡 |
| 物理 x86_64 硬件证据（如需） | 用户 | 本机为 Rosetta 翻译；需要时用同一参数化脚本在真机重跑 |

## 8. 工作区状态

- **仓库历史已被压缩为单个提交**：HEAD = `39e202d init`（本次会话核验；上一版快照提到的 `3138496` / `cb69afb` 已不存在）。T01–T22 的代码均在该提交内。
- **未提交改动**（本次会话末）：`scripts/linux-musl.sh`、`scripts/container-linux-musl.sh`、`xtask/src/dist.rs`、`.github/workflows/ci.yml`、`docs/operations.md`、`llmdoc/{decisions,validation-release}.md`、`llmdoc/requirements/web-mvp/{implementation,state,progress-summary,qa-report}.md`，以及未跟踪的 `artifacts/web-mvp/t22-rd/linux/`、`artifacts/web-mvp/t22-qa/`。**`crates/**`、`apps/web/**`、`contracts/**`、PRD 未改。未推送。**
- 本轮实测回归（Linux 容器内）：`file`/`ldd` 静态链接通过、`smoke` 7 步通过、`--network none` 离线 smoke 通过、`dist --check-reproducible` 同哈希；`cargo xtask check` 因 BUG-013 红灯（见 §4）。

## 9. 证据索引

- 协调状态与切片账：`llmdoc/requirements/web-mvp/state.yaml`
- 验收报告（回合 1–30，含缺陷全文）：`llmdoc/requirements/web-mvp/qa-report.md`（回合 30 见第 3676 行起）
- 实现记录（§T01–§T22 与各修复轮，Linux 轮见 §T22-13）：`llmdoc/requirements/web-mvp/implementation.md`
- 决策与非代码知识（ADR-001~036 + 各回合验收知识）：`llmdoc/decisions.md`
- Linux 构建证据：`artifacts/web-mvp/t22-rd/linux/`（`build-file-ldd.log`、`smoke-console.log`、`offline/`、`regression/`、`dist-x86_64-unknown-linux-musl/`）
- QA 回合 30 证据（16 件）：`artifacts/web-mvp/t22-qa/`（含裸容器冷启动脚本、可复现轮日志、SIGTERM 窗口实验、离线 smoke、全量测试枚举）
- 原始证据：`artifacts/web-mvp/<卡或回合>/`（日志、截图、可复跑脚本）
