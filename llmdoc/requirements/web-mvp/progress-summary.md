# web-mvp 进度与实现总结（暂停交接）

状态：**暂停中**，2026-09-13 由主协调会话整理；暂停点＝ T22（macOS 平台已交付，Linux 平台受环境阻塞）；T23 未开始。
依据：`state.yaml`、`qa-report.md`（回合 1–29）、`implementation.md`（§T01–§T22）、`llmdoc/decisions.md`（ADR-001~035）、`artifacts/web-mvp/**`。
本文件是**交接快照**，不替代 state.yaml（协调状态）与 qa-report（验收证据）；细节数字以各报告与实测为准。

## 1. 一句话状态

**T01–T21 二十一个切片全部经 QA 独立验收 PASS（0 个未关闭缺陷），T22 的 macOS 交付完成并通过自证，Linux x86_64-musl 因 Docker 引擎损坏未验证，T23（真实 Provider 链路）未开始。** 产品在 fixture 环境下已具备可运行的端到端闭环：建物品 → 上传资料 → 浏览器 PDF 准备 → 报价与确认 → 后台双分支生成 → 草稿复核与热点校准 → 发布不可变版本 → 阅读/导出/备份恢复。

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
| T22 | 多平台单二进制发布 | **部分**：macOS 侧完成；Linux 未验证 | 未验收 |
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

## 4. 缺陷闭环（全部已关闭）

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

1. **T22 Linux x86_64-musl（阻塞中）**：macOS 数据卷曾被写满（ENOSPC），Docker Desktop 引擎随后无法启动（VM 引导即失败），本机无法取得该平台的**原生运行证据**。仓库已备 `scripts/linux-musl.sh`、`scripts/container-linux-musl.sh` 与 `.github/workflows/ci.yml`（含 dist-musl job），但**未执行**。按规范"不得以交叉编译退出码 0 替代运行证据、未跑平台不贴支持标签"，Linux 目前不得声称通过。
2. **T23 未开始**：需要用户提供 Tripo / 说明书 AI 凭据、真实型号资料与一次生成预算；缺失则记 BLOCKED。
3. **UI-060 网页导出按钮**未实现（端点与生成类型已就绪）；登记为 T21/T22 之后的独立前端小项。
4. **浏览器矩阵**：仅 Playwright Chromium 148；Firefox/Edge 本机未安装，AC-063 未满足（T22/T23 承接）。
5. **热点级 `cameraPose`** 未实现（视角保存落在步骤级 `stepPoses`）：字面偏离 `contracts.md` §2，不影响必选 AC，建议 PM 在合同留痕。
6. UI-032（字面倒计时）、顶栏任务计数徽标未做；签名/公证未做；模型加载耗时与 ≥5 分钟连续旋转未测（旋转 p95 实测 17.7 ms ≤ 33 ms 已达标）。
7. T12/T13 的真实形态收敛项（upload token/计费字段候选、下载允许域）待 T23。

## 7. 挂起决策与待用户/PM 动作

| 事项 | 归属 | 说明 |
| --- | --- | --- |
| Docker 恢复或改用外部 Linux 机器 | 用户 | 恢复后由 RD 执行 `scripts/linux-musl.sh --arch amd64`（含 `--network none` 离线验证），再交 QA 补 T22 结论 |
| T23 凭据与预算 | 用户 | 无授权则 T23 记 BLOCKED，不伪造真实链路结论 |
| UI-032 / cameraPose / OB-16 措辞张力 | PM | 需在 PRD/合同留痕或裁定，不由 RD 自行扩大 |

## 8. 工作区状态

- 已提交：`3138496`（2026-09-12 23:08，lizhao）覆盖 T01–T19 时期的代码；更早为 `cb69afb`。
- **未提交**：T20–T22 的工作（121 个修改 + 24 个新增，含 `crates/server/src/backup/`、`redaction.rs`、`scripts/`、`.github/`、`docs/operations.md` 等）以及大量 `artifacts/` 证据文件。**未推送**。
- 暂停前最后一次全量回归：`cargo xtask check` 7/7、workspace 测试 0 失败、`xtask dist` 可复现同哈希、`xtask smoke` 7 步通过。

## 9. 证据索引

- 协调状态与切片账：`llmdoc/requirements/web-mvp/state.yaml`
- 验收报告（回合 1–29，含缺陷全文）：`llmdoc/requirements/web-mvp/qa-report.md`
- 实现记录（§T01–§T22 与各修复轮）：`llmdoc/requirements/web-mvp/implementation.md`
- 决策与非代码知识（ADR-001~035 + 各回合验收知识）：`llmdoc/decisions.md`
- 原始证据：`artifacts/web-mvp/<卡或回合>/`（日志、截图、可复跑脚本）
