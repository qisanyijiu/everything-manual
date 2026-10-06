# 万物说明书（EverythingManual）

输入物品的**名称、型号、说明书 PDF 与多视图照片**，自动生成 3D 模型与**带原文出处**的部件/步骤知识；
人工校准热点后发布不可变版本，在浏览器里以「3D + 部件 + 步骤 + 原文页」联动的方式阅读。

- **单管理员、自托管**：数据保存在独立 `data-dir`，不依赖外部数据库与消息中间件。
- **开发前后端分离，发布合为一个可执行文件**：Rust 服务同源提供 `/api/v1/*` 与 React SPA，
  内嵌静态资源、PDF.js 运行资源、数据库迁移与 bundled SQLite；运行期不需要 Node / Python / PDF 程序。
- **外部依赖只有两类 HTTP API**：Tripo（3D 生成）与说明书 AI（知识提取），密钥只留在服务端。

## 当前状态（2026-10-04）

| 阶段 | 状态 |
| --- | --- |
| 核心流程与独立 3D 阅读器 | 已合并到本地 `master`；既有macOS验收Rust669项、前端311项、Chrome140项通过 |
| Nikon F3HP 真实供应商流程 | 47页说明书导入、整机/分件生成、人工知识校对、发布、ZIP与离线HTML导出、重启复读通过；实际消耗70 Tripo credits |
| macOS Apple Silicon 单二进制 | 构建、可复现哈希、冷目录七步检查及断网读取通过；未签名/公证 |
| Docker / Linux AMD64 ABI | 新76分件镜像、前端336项测试与生产Chrome验收通过；既有Rust671项、Compose647项及断网七步记录保留；ARM虚拟机内经Rosetta运行 |
| 统一预览资料库 | 8个物品、7份说明书/166页、370条资产、363个blob、3份草稿及3个发布版本；原资料与发布保留 |
| Nikon 76 个编号分件 | 本地重新划分几何边界，全部76件可选择、高亮、展开/复原；草稿、新发布及断网离线HTML的Chrome逐件矩阵验证通过，本轮0 credits |

**演示**：[CyberDog 2 机器狗 3D 交互说明书](demo/cyberdog2/README.md)——克隆后双击 `demo/cyberdog2/cyberdog2-3d.html`
即可离线体验部件热点、步骤与站立/坐下/握手等姿势，无需启动服务。

当前合并与真实样本证据见 [2026-10-04交付验收](llmdoc/requirements/standalone-3d-viewer/delivery-2026-10-04.md)，
Docker部署与本轮Linux状态见 [Docker部署](docs/docker.md) 和
[Docker、Linux与统一资料交付记录](llmdoc/requirements/standalone-3d-viewer/docker-delivery-2026-10-04.md)。
[历史进度](llmdoc/requirements/web-mvp/progress-summary.md)及 [早期QA报告](llmdoc/requirements/web-mvp/qa-report.md)
保留各轮事实，不代表最新交付状态。

## 技术栈

| 层 | 选型 |
| --- | --- |
| 前端 | React 19 + TypeScript（strict）+ Vite、React Router、TanStack Query、Three.js / React Three Fiber 9、pdfjs-dist |
| 后端 | Rust（edition 2024，`rust-toolchain.toml` 固定 1.98.1）、Axum + Tokio + tower-http、SQLx + bundled SQLite |
| 外部 | Tripo v3（模型生成）、OpenAI Responses 形态的说明书 AI（知识提取） |
| 工程 | Cargo workspace + `xtask`、npm lockfile、Playwright（e2e）、OpenAPI → TS 生成类型（禁止手抄 DTO） |

## 快速开始

在本机项目根目录运行一条命令，启动并在Chrome打开统一资料库：

```sh
bash scripts/start-project.sh
```

当前本机已启用源码前端，地址为 `http://127.0.0.1:5173/`，后端为 `http://127.0.0.1:8080/`。
`init` 会在空资料库中自带一份已发布的示例说明书「机器狗」（CyberDog 2：头部/躯干/四肢 6 个部件，
站立、趴下、坐下、点头、作揖 5 个姿势），首次登录即可阅读；不需要时用 `init --no-sample`，
说明见 [内置示例](crates/server/sample/cyberdog2/README.md)。
全新克隆或内嵌前端模式地址为 `http://127.0.0.1:8080/`。当前本机整理后的资料统一放在 `var/preview/data`，
可浏览8个有效样本，包含Nikon F3HP和Wii U；登录密码为 `12345678`。
38个旧数据目录已移至 `var/preview/archive/originals`，旧位置仅保留兼容软链接；项目 `var/preview` 外已无物理应用数据库。
历史快照、合成压力资料和回归样例备份也集中在 `var/preview/` 下。
这些资料与私有配置均被Git忽略，全新克隆会创建空资料库并要求首次设置密码。
脚本在macOS默认用本机发行程序，在Linux默认用Docker；Docker须先安装并启动。

```sh
bash scripts/start-project.sh --docker  # 改用Docker，仍读取同一份资料
bash scripts/start-project.sh --source-ui  # 当前源码前端，复用本机后端并记住偏好
bash scripts/start-project.sh --embedded-ui  # 回到发行程序内嵌页面
bash scripts/start-project.sh --status  # 查看状态
bash scripts/start-project.sh --stop    # 停止服务，保留资料
```

Docker使用独立的文件主密钥，不读取macOS钥匙串中的旧密钥；切换后历史资料可读，
如需继续生成，请在「设置 → API配置」重新录入供应商地址和key，再重启。
首次镜像构建、持久化、备份与恢复见 [Docker部署](docs/docker.md)。

统一资料库已在Chrome验证本机及Linux容器中的8个样本可浏览；原两个发布版本的五份资产SHA保持一致。
Nikon新增76分件版本保留旧语义绑定，其全部编号均可观察展开；这些编号仍表示外观几何区域，不表示真实机械拆解。
统一脚本的本机启动、重复启动、停止与重启已实测通过；源码前端模式保留已获钥匙串授权的后端身份。
这轮资料整理与Chrome复核未发送供应商请求或新增账务记录。

### 从源码开发

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

日常预览使用上面的统一启动脚本；原 `scripts/start-wiiu-preview.sh` 现转发到同一入口。
分离的Vite开发地址仅用于前端热更新。

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
真实链路须有明确的供应商、样本与预算授权，不能从默认回归触发；
本轮Nikon实际运行记录见 [交付验收](llmdoc/requirements/standalone-3d-viewer/delivery-2026-10-04.md)。

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
scripts/             统一启动、Linux 容器构建/验证脚本
Dockerfile / compose.yaml  Linux 镜像与本机 Compose 部署
docs/docker.md       Docker 快速启动、持久化、密钥、备份与恢复
docs/operations.md   运维：安装、启动、升级、回滚、备份
llmdoc/              为什么这样做：架构、合同、决策、任务卡、验收与进度
```

## 配置

- 示例：`config.example.toml`（服务/限制/并发/供应商）、`price-catalog.example.toml`（价格快照）。
- 供应商地址、模型和密钥可在「设置 → API 配置」中保存，**重启服务后生效**；操作说明见 [网页 API 配置](docs/api-settings.md)。
- 供应商字段优先级：网页覆盖 > 环境变量（`EM_*` 白名单）> TOML > 默认。其它字段仍遵循 CLI > 环境变量 > TOML > 默认；**未知配置键报错**而不是忽略。
- 网页密钥与部署 `api_key_file` 使用 AES-256-GCM 加密存储，密文文件权限为 0600；主密钥默认保存在仓库与数据目录外的私有文件，也可通过 `EM_SECRETS_MASTER_KEY` 独立注入。旧 macOS 钥匙串密文需显式设置 `EM_SECRETS_BACKEND=keychain` 读取。环境变量 API key 只在内存中使用。旧文件迁移、主密钥保管与验收状态见 [加密存储说明](docs/api-settings.md#密钥加密存储)。
- 已保存密钥不回传前端，私有密钥配置与主密钥不进入导出包和备份；应用在持久化供应商回复前处理已知密钥的直接及 JSON 转义回显。
- **未配置 Provider 时**：站点可正常浏览已有资料，生成/报价返回 409「供应商未配置」，不会回退 mock。

## 部署与安全要点

- 默认监听 `127.0.0.1:8080`；**非 loopback 必须放在受信反向代理之后**（配置 `trusted_proxy_cidrs`
  并显式启用 `Secure` cookie），否则拒绝启动。MVP 不内置 TLS 监听——配置 `tls.*` 即 fail-closed 拒绝启动。
- 备份要求先停服（排他锁）；`restore` 只写入不存在或为空的目录，先校验 hash/外键/引用。
- 回滚 = 停服后用旧二进制 + 迁移前备份恢复到新空目录；旧程序拒绝打开更新的 schema。
- 已发布的说明书是**不可变版本**；修改草稿不影响已发布内容。

签名/公证、Linux平台的当前验证状态见 [运维说明](docs/operations.md) 与 [Docker部署](docs/docker.md)。
本轮浏览器验收按用户要求只覆盖Chrome。

## 文档

| 文档 | 内容 |
| --- | --- |
| [llmdoc/README.md](llmdoc/README.md) | 项目知识总索引（阅读顺序） |
| [Docker部署](docs/docker.md) | 统一脚本启动、Compose、私有密钥、持久化及Linux验证状态 |
| [Docker、Linux与统一资料交付记录](llmdoc/requirements/standalone-3d-viewer/docker-delivery-2026-10-04.md) | 本轮镜像/静态发行程序、隔离验证、资料迁移及Chrome读取证据 |
| [Nikon 76 分件观察](llmdoc/requirements/standalone-3d-viewer/nikon-all-parts-2026-10-04.md) | 新几何分区、在线/离线全部编号交互、Chrome逐件证据与预览方式 |
| [2026-10-04交付验收](llmdoc/requirements/standalone-3d-viewer/delivery-2026-10-04.md) | 本地master合并、Chrome140项、Nikon真实完整流程、macOS发行包证据 |
| [交互体验改进方案](docs/interaction-experience-improvement-plan.md) | 流程连续性、准备恢复、复核与阅读、可访问性及分阶段验收（建议方案） |
| [交互改进 · 切片 A](llmdoc/requirements/interaction-a/prd.md) | 创建跳转、生成确认、键盘操作与可读性已实现；[独立验收通过](llmdoc/requirements/interaction-a/qa-report.md)（2026-09-20） |
| [网页 API 配置](docs/api-settings.md) | Tripo / 说明书 AI 设置、AES-256-GCM 密钥存储、迁移及故障处理；[网页配置验收](llmdoc/requirements/api-settings/state.yaml)、[加密升级验收](llmdoc/requirements/encrypted-secrets/qa-report.md) |
| [progress-summary.md](llmdoc/requirements/web-mvp/progress-summary.md) | 进度与实现总结（切片账、缺陷闭环、未完成项、证据索引） |
| [prd.md](llmdoc/requirements/web-mvp/prd.md) | PRD（需求、验收条件、UI 交互合同） |
| [implementation.md](llmdoc/requirements/web-mvp/implementation.md) | 各卡实现记录与实际命令结果 |
| [qa-report.md](llmdoc/requirements/web-mvp/qa-report.md) | QA 验收报告（回合 1–29，含缺陷全文） |
| [architecture.md](llmdoc/architecture.md) · [contracts.md](llmdoc/contracts.md) | 技术架构 · 接口与数据合同 |
| [validation-release.md](llmdoc/validation-release.md) · [decisions.md](llmdoc/decisions.md) | 验收与发布规范 · 决策记录（ADR） |
