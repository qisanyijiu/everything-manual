# 万物说明书（EverythingManual）

输入物品的**名称、型号、说明书 PDF 与多视图照片**，自动生成 3D 模型与**带原文出处**的部件/步骤知识；
人工校准热点后发布不可变版本，在浏览器里以「3D + 部件 + 步骤 + 原文页」联动的方式阅读。

- **单管理员、自托管**：数据保存在独立 `data-dir`，不依赖外部数据库与消息中间件。
- **开发前后端分离，发布合为一个可执行文件**：Rust 服务同源提供 `/api/v1/*` 与 React SPA，
  内嵌静态资源、PDF.js 运行资源、数据库迁移与 bundled SQLite；运行期不需要 Node / Python / PDF 程序。
- **外部依赖只有两类 HTTP API**：Tripo（3D 生成）与说明书 AI（知识提取），密钥只留在服务端。

## 当前状态（2026-09-13）

| 阶段 | 状态 |
| --- | --- |
| T01–T21（骨架 → 认证/资产/资料 → 生成闭环 → 校准/发布/导出备份 → 回归矩阵） | 已实现，**经 QA 独立验收 PASS**（回合 1–29，缺陷 BUG-001~012 全部关闭） |
| T22（多平台单二进制发布） | macOS 平台已构建并自证；**Linux 平台未验证**（本机 Docker 引擎故障，环境阻塞）；尚未经 QA 验收 |
| T23（授权真实 Provider 链路与最终 QA） | 未开始，需要用户提供凭据与一次生成预算 |

完整的实现概览、未完成项与证据索引见
[llmdoc/requirements/web-mvp/progress-summary.md](llmdoc/requirements/web-mvp/progress-summary.md)；
验收细节见 [qa-report.md](llmdoc/requirements/web-mvp/qa-report.md)。

> 说明：当前交付物在 fixture（本机假 Provider）环境下完成端到端验证，**尚未对真实 Tripo / 说明书 AI
> 服务做过付费验证**；未配置的真实链路不声称通过。

## 技术栈

| 层 | 选型 |
| --- | --- |
| 前端 | React 19 + TypeScript（strict）+ Vite、React Router、TanStack Query、Three.js / React Three Fiber 9、pdfjs-dist |
| 后端 | Rust（edition 2024，`rust-toolchain.toml` 固定 1.98.1）、Axum + Tokio + tower-http、SQLx + bundled SQLite |
| 外部 | Tripo v3（模型生成）、OpenAI Responses 形态的说明书 AI（知识提取） |
| 工程 | Cargo workspace + `xtask`、npm lockfile、Playwright（e2e）、OpenAPI → TS 生成类型（禁止手抄 DTO） |

## 快速开始

前置：Rust（rustup 会按 `rust-toolchain.toml` 自动安装固定工具链）、Node ≥ 22.12 与 npm、C 编译器
（bundled SQLite）；首次构建需要网络拉取依赖与 npm 包。

```sh
# 1) 前端依赖
npm --prefix apps/web ci

# 2) 初始化 data-dir（管理员密码交互输入，不回显；无人值守用 --password-file <0600 文件>）
cargo run -p everything-manual -- init  --data-dir ./var/dev

# 3) 启动服务（默认仅监听 127.0.0.1:8080）
cargo run -p everything-manual -- serve --data-dir ./var/dev
# 浏览器打开 http://127.0.0.1:8080
```

前端热更新开发（Vite `127.0.0.1:5173`，`/api` 代理到 `8080`）：

```sh
cargo run -p everything-manual -- serve --data-dir ./var/dev   # 一个终端
npm --prefix apps/web run dev                                   # 另一个终端
```

## 工程命令

根 `.cargo/config.toml` 定义了 `cargo xtask` 别名：

| 命令 | 作用 |
| --- | --- |
| `cargo xtask contracts` / `--check` | 从 Rust DTO 生成 `contracts/openapi.json` 与前端类型；`--check` 检测漂移且不修改工作树 |
| `cargo xtask check` | 本地全量检查：`fmt --check`、`clippy -D warnings`、Rust 测试、前端 lint/typecheck/test、合同漂移 |
| `cargo xtask dist --target <triple>` | 构建单二进制，产出 `SHA256SUMS` / `licenses.json` / `build-info.json` / 动态依赖清单 |
| `cargo xtask smoke --binary <绝对路径>` | 发布包 7 步检查：冷目录 → 样例备份 restore → 登录/资源/路由 → Range/HEAD/未配置拒绝 → 断网读取 → 重启持久化 → backup→restore 复读 |
| `cargo xtask smoke-bootstrap --binary <路径>` | 最小检查（内嵌页面/静态资源/health），用于早期验证 |

## 测试

```sh
cargo test --workspace                                  # 单元 + 集成（默认全部走本机 fixture，零真实外网/付费）
npm --prefix apps/web run test -- --run                  # Vitest
npm --prefix apps/web run test:e2e                       # Playwright（自管测试后端与临时 data-dir）
```

约定：**默认测试禁止调用真实收费 API**；缺配置时系统返回"未配置"而不是假成功；
真实链路只经显式授权入口（`cargo xtask test-live`，属 T23，尚未启用）。

## 目录结构

```text
apps/web/            React SPA（src/features：shell / auth / library / import / jobs / viewer / manual / settings）
crates/core/         领域类型与状态机（不依赖 Axum/SQLx/浏览器）
crates/server/       Axum 服务：http / storage / assets / jobs / providers(tripo, manual_ai) / generation / drafts / releases / backup
crates/test-support/ 本机 HTTP fixture 与样例资产（仅 dev-dependency）
xtask/               工程命令（contracts / check / dist / smoke / smoke-bootstrap）
migrations/          SQL 迁移（只追加，随二进制内嵌）
contracts/           openapi.json（由 Rust DTO 生成）
tests/fixtures/      原创样例资料与脱敏响应（含来源与许可）
scripts/             Linux 容器构建/验证脚本
docs/operations.md   运维：安装、启动、升级、回滚、备份
llmdoc/              为什么这样做：架构、合同、决策、任务卡、验收与进度
```

## 配置

- 示例：`config.example.toml`（服务/限制/并发/供应商）、`price-catalog.example.toml`（价格快照）。
- 优先级：CLI 非密钥项 > 环境变量（`EM_*` 白名单）> TOML > 默认；**未知配置键报错**而不是忽略。
- 密钥只从环境注入或 0600 受限文件读取，不进入前端、日志、导出包与备份 manifest。
- **未配置 Provider 时**：站点可正常浏览已有资料，生成/报价返回 409「供应商未配置」，不会回退 mock。

## 部署与安全要点

- 默认监听 `127.0.0.1:8080`；**非 loopback 必须放在受信反向代理之后**（配置 `trusted_proxy_cidrs`
  并显式启用 `Secure` cookie），否则拒绝启动。MVP 不内置 TLS 监听——配置 `tls.*` 即 fail-closed 拒绝启动。
- 备份要求先停服（排他锁）；`restore` 只写入不存在或为空的目录，先校验 hash/外键/引用。
- 回滚 = 停服后用旧二进制 + 迁移前备份恢复到新空目录；旧程序拒绝打开更新的 schema。
- 已发布的说明书是**不可变版本**；修改草稿不影响已发布内容。

签名/公证、Linux 平台验证、浏览器矩阵（Firefox/Edge）等未完成项与解除条件，
见 [进度总结](llmdoc/requirements/web-mvp/progress-summary.md) 与 [运维说明](docs/operations.md)。

## 文档

| 文档 | 内容 |
| --- | --- |
| [llmdoc/README.md](llmdoc/README.md) | 项目知识总索引（阅读顺序） |
| [progress-summary.md](llmdoc/requirements/web-mvp/progress-summary.md) | 进度与实现总结（切片账、缺陷闭环、未完成项、证据索引） |
| [prd.md](llmdoc/requirements/web-mvp/prd.md) | PRD（需求、验收条件、UI 交互合同） |
| [implementation.md](llmdoc/requirements/web-mvp/implementation.md) | 各卡实现记录与实际命令结果 |
| [qa-report.md](llmdoc/requirements/web-mvp/qa-report.md) | QA 验收报告（回合 1–29，含缺陷全文） |
| [architecture.md](llmdoc/architecture.md) · [contracts.md](llmdoc/contracts.md) | 技术架构 · 接口与数据合同 |
| [validation-release.md](llmdoc/validation-release.md) · [decisions.md](llmdoc/decisions.md) | 验收与发布规范 · 决策记录（ADR） |
