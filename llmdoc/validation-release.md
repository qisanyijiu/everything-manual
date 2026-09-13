# 验收、运行与单二进制发布规范

版本：1.0 · 2026-09-11 · 状态：计划，产品代码／脚本／测试尚未创建。本文件中的命令是工程需实现的合同，不是本轮运行结果。

## 1. 验收层级与证据

1. 静态：格式、Rust clippy、TS strict、OpenAPI／生成类型无漂移、依赖锁、配置与文档链接。
2. 单元：状态转换、费用decimal、引用校验、坐标变换、输入限制。
3. 集成：真实SQLite、临时data-dir、真实HTTP适配器对本地fixture、文件／迁移／恢复，不只mock repository。
4. 浏览器：真实前端+Rust+本地fixture，从建物品到发布；以 DOM／network／canvas 行为和人工观察验证，不只截图“看起来像”。
5. 发布：移动到无源码目录启动单个可执行文件；本地完整资源、持久化、平台依赖与灾备。
6. 真实API：明确授权后选真实资料验证Tripo和说明书AI协议、费用与产出；与fixture报告分开。

每个结果绑定 work-id、PRD revision、task IDs、AC IDs、测试目标、执行时间／平台／浏览器、binary hash或代码tree快照、日志／截图路径。尚无 Git 提交时记源文件hash清单，不编造commit。退出码0但跑了0测试不算PASS；缺环境、skip必选项、无法观察WebGL或真实服务未验要明确记录。

QA 的 PASS 只覆盖派发范围；全量PASS必须是当前PRD修订的full记录。FAIL回RD修复后由QA关闭缺陷，RD不能自行签发。外部授权缺失导致BLOCKED，不是让RD修一个假错误，也不能无限调用相同失败API。

## 2. 工程命令合同（T01起逐步实现）

根 `.cargo/config.toml` 定义 `xtask = "run --package xtask --"`。脚本不得隐式调用付费API、拉私有资料、推送Git或公网发布。

| 命令 | 必须执行／返回 |
| --- | --- |
| `npm --prefix apps/web ci` | 按package-lock安装构建依赖；不替换锁文件 |
| `npm --prefix apps/web run dev` | Vite5173，代理/api到8080 |
| `cargo run -p everything-manual -- serve --data-dir ./var/dev` | 开发后端；管理员先init；不要求embedded-ui |
| `npm --prefix apps/web run typecheck` | tsc --noEmit；Vite build不能代替它 |
| `npm --prefix apps/web run lint` | 前端lint，警告策略统一；不能无脚本却声称通过 |
| `npm --prefix apps/web run test -- --run` | Vitest非watch模式，缺测试非零 |
| `npm --prefix apps/web run test:e2e -- <spec>` | Playwright；自动管理测试后端／fixture和临时目录，失败保留证据 |
| `cargo test -p everything-manual --test <target>` | 命名集成测试，避免空filter被当成功 |
| `cargo xtask contracts` | 从Rust DTO导出OpenAPI，再生成TS；稳定排序且确定性 |
| `cargo xtask contracts --check` | 临时生成并比较，差异非零；不修改工作树 |
| `cargo xtask check` | fmt --check、clippy --workspace --all-targets -- -D warnings、单元／集成、前端lint/typecheck/test、合同检查；不能短路隐藏失败 |
| `cargo xtask dist --target <triple>` | 校验工具链→npm ci/typecheck/test/build→内嵌资源Rust release --locked；产出binary、SHA256、licenses与build-info |
| `cargo xtask smoke-bootstrap --binary <absolute-path>` | T01最小检查：拷贝binary到新临时目录，仅验内嵌页面／静态资源／health；不代表产品验收 |
| `cargo xtask smoke --binary <absolute-path>` | T22正式包检查：新临时目录、从合法备份恢复新data-dir，验证生产静态资源／API／持久化并清理测试进程；不放行本机Provider fixture |
| `cargo xtask test-live --case <case-id> --budget-file <path>` | 仅显式人工入口，确认可用预算／配置和授权范围；无授权／超预算非零；日志脱敏 |

测试依赖：Playwright浏览器可在开发／CI安装；它不是发布运行依赖。Rust tests使用临时本地目录，不接触真实data-dir。测试fixture URL准入通过仅测试构建开关+显式配置，不允许release因`TRIPO_API_KEY`缺失自动走fake。测试构建可以启embedded-ui验证同样的资源路径，但必须标记测试构建，不能作为正式发布包；其hash／生成E2E证据与正式binary分开记录。

`dist` 可以使用构建机的Node和C编译器；`build.rs` 只声明内嵌输入／变更依赖，禁止偷偷安装包和执行npm。server普通测试可不启embedded-ui，但发布和smoke必须启用，避免“后端测过、前端没打进去”。新迁移或vendor变动必须触发重编译。

## 3. 关键测试矩阵

| 领域 | 正常必测 | 失败／恢复必测 | 主要卡 |
| --- | --- | --- | --- |
| 会话 | 登录、刷新、注销、授权读资产 | CSRF／Origin／过期／错误密码限速／跨资产归属 | T04/T06 |
| 上传 | PDF、JPEG/PNG、重复hash | 假类型、超限、像素炸弹、路径穿越、断流、磁盘满 | T06 |
| PDF | 文字／扫描／旋转／非拉丁字体、1-based出处 | 加密、超100页、worker缺失、中断续传、CMaps／WASM离线 | T09 |
| 费用 | 输入冻结、价表版本、实际结算 | 过期报价、重放20次、并发预算、unknown不释放 | T11 |
| Tripo | multipart+view-key、taskid、查询、模型 | 200业务失败、429、5xx、接受POST后断连、未知状态、缺输出 | T12 |
| 说明书AI | 文本／页图、schema、页覆盖、来源 | 拒答、截断、伪造页、漏批、同步结果未保存、资料内恶意指令 | T14 |
| 下载 | 哈希一致本地GLB | 过期URL再查、DNS重绑定／私网跳转、bearer泄露、截断／超限 | T13 |
| 执行器 | 并发领取、恢复、按分支重试 | SIGKILL、租约过期晚到ID、重复推进、unknown不盲重购 | T10/T15 |
| 3D | 旋转缩放、拾取、视角保存 | 根坐标变换、拖动误点、模型外链、WebGL失效／恢复、资源释放 | T18/T19 |
| 知识／发布 | 部件→步骤→原文、版本可读 | stale热点、未确认知识、412并发、改draft不改release | T19 |
| 备份 | 停服快照→新目录恢复 | 活跃锁、损坏hash、缺blob、新schema、非空目录拒绝 | T20 |
| 包装 | 单binary启动、API、前端路由刷新 | 无dist源码NodePython、离线vendor、Range／404、平台动态依赖 | T22 |

崩溃注入最少五个断点：付费POST发出前；供应商接受但响应未到；task ID到达但状态未推进；blob rename后DB事务前；draft已提交但HTTP响应未返回。每个断点重启后核对任务数、远端请求数、费用预留、资产引用与不可变版本。

说明书批次另测：第1批完成、第2批响应未知、第3批未开始时中断；恢复不能重跑第1批，第2批待授权处理，第3批按已确认预算继续或暂停并明确显示。两批并发必须有独立持久身份。

## 4. UI／性能初始验收基线

这些是待PM在PRD确认、QA在具名设备验证的目标，不是已测性能：

- 桌面完整编辑，≥1280px三栏阅读；768px以下转抽屉／单栏，保留读说明书和步骤；若移动端不支持校准必须明确禁用提示，不能坏掉无解释。
- 键盘可达所有非3D核心操作，有可见focus、标签和错误关联；部件列表提供3D热点的文字替代；支持减少动效。
- 默认100k三角面、≤4K贴图预算下，目标桌面连续旋转p95帧耗时≤33ms；模型加载目标在指定本地网络／样例大小下单独记录，不承诺所有150MiB模型瞬开。
- 本地无供应商等待的普通元数据API，具名测试机p95目标≤200ms；PDF准备逐页、有进度可取消，100页不同时铺满100张canvas。
- 3D/PDF不在首屏库列表强制加载；连续切换10次模型后检查资源数／内存趋势，不能只测一次成功。
- 浏览器默认验Chrome/Edge/Firefox当前支持版本，发布报告写精确版本；Safari需单独实机测试才列支持。Headless WebGL结果不替代目标用户设备视觉／交互复核。

PM可在编码前按目标用户设备调整目标，但必须记录原因和当前修订。出现不达标后不能由RD私自降低阈值来使测试转绿。

## 5. 管理员运行合同

未来CLI示例（需在实现后使用；不含真实密钥）：

```sh
./everything-manual init --data-dir ./manual-data
./everything-manual check --data-dir ./manual-data
./everything-manual serve --data-dir ./manual-data --listen 127.0.0.1:8080
```

`init` 交互输入密码，不在参数／shell history中放密码；无人值守初始化使用受限文件参数并在文档说明权限。API密钥用受限配置文件或环境注入；不把`--api-key 秘密`作为推荐操作。`check`只验证配置／目录／schema，不收费。非loopback监听若没有明确TLS配置或可信反向代理模式应拒绝启动。

建议配置键：`data_dir`、`listen`、`public_origin`、`tls.cert_file/key_file`、`trusted_proxy_cidrs`、`providers.tripo.base_url/model/api_key_env`、`providers.manual_ai.model/api_key_env`、`limits`、`concurrency`、`price_catalog_path`。缺少密钥可以启动浏览已有资料，但生成返回未配置；不能使整站健康失败，也不能生成假模型。

服务终止先停止领取、完成必要短写入并退出；已知远端任务下次恢复查询。取消本地job不保证退款／远端停止。供应商关停或密钥撤销时历史说明书仍本地可读。

### 备份和升级

1. 停止服务并确认data-dir排他锁已释放；在停止状态执行backup。
2. `./everything-manual backup --data-dir ./manual-data --out ./backups/<明确新路径>`：checkpoint／一致快照+所有引用blob+manifest+hash；不覆盖已有备份。
3. 校验备份可恢复；保存旧binary、版本／hash与schema版本。替换binary后启动执行受支持迁移。
4. 若需要回滚，先停新服务，用旧binary配合迁移前备份恢复到新空目录；不能只换旧binary读取新schema。
5. `./everything-manual restore --from ./backups/<备份> --data-dir ./restored-data`：目标必须不存在或为空，验证hash／外键／引用后完成；再用恢复目录启动复核发布版、PDF和GLB。

以上尖括号是需要用户选择的路径，不是可原样复制的shell命令。自动备份／定时器不在本轮范围。备份包含用户原始资料，应按部署者的数据保护要求限制访问；不要把备份上传到未授权第三方。

## 6. 平台发布矩阵

默认建议首轮必选 macOS Apple Silicon（当前开发环境）和 Linux x86_64 自托管；PM在T00明确必选范围。其余按扩展支持验收，绝不以交叉编译退出码0替代运行证据。

| 目标 | 构建建议 | 发布检查 |
| --- | --- | --- |
| aarch64-apple-darwin | macOS原生runner、固定最低系统版本／SDK | otool -L；只允许系统库，不依赖构建机Homebrew路径；签名／公证需要用户账号授权，未做须声明 |
| x86_64-unknown-linux-musl | Linux musl工具链，SQLite与相关C依赖可编译 | file/ldd或readelf检查；在声明的干净Linux运行；TLS信任根按实际rustls配置验证 |
| x86_64-pc-windows-msvc（扩展） | Windows原生runner | 检查DLL依赖／CRT策略、杀软与路径、服务关闭、实际.exe运行 |
| x86_64-apple-darwin（扩展） | 对应macOS runner | 不能用Apple Silicon运行结果代替Intel验证 |

单二进制允许依赖操作系统系统库，不允许临时安装Node/Python/SQLite服务才能运行。Linux静态链接不自动带来可用CA证书；根据选定rustls根信任方案明确环境要求，在冷环境验证真实HTTPS，不能只拿HTTP fixture说TLS通过。

二进制内嵌：web HTML/JS/CSS、字体图标、PDF worker/CMaps/standard fonts/WASM等选择版本所需资源、允许的3D解码器、迁移。不要在启动时到CDN下载缺少资源。Vite import与URL由构建配置解析，不手工拼可能被hash改名的worker路径。

## 7. 冷目录 Smoke 的准确步骤

1. 建新临时目录，只复制release binary；不得让工作目录等于仓库或含apps/web/dist。创建独立临时data-dir。
2. 用正式binary的restore从T20生成的合法、脱敏样例备份恢复到新空data-dir（含测试管理员和样例资料），启动生产embedded-ui模式。不启fixture URL放行，不连接本机Provider fixture；另外用独立空目录验证init。记录服务PID并只结束自己启动的进程。
3. 登录，访问首页、嵌套路由刷新、JS/CSS/字体/PDF所有本地资源；未知/api必须JSON404，未知非导航静态资源不得返回HTML200。
4. 在新物品验证PDF／照片上传与准备；对恢复出的模型／草稿验证加载、校准、编辑与发布，检测Range/HEAD、状态与持久化。正式包无Provider配置时生成必须明确拒绝。完整fixture生成由T21测试构建证明，正式包真实生成由T23授权证明，不能混成一次运行证据。
5. 关闭外部网络仍可读取已有release／PDF／GLB；服务关闭后网站不可用属于部署边界，不冒充离线PWA。
6. 停服／重启确认用户数据仍在。做backup→新目录restore再读同一release，比对manifest/hash。
7. 在没有Node/Python/源码目录的目标运行环境重复上述读取与恢复检查。不能仅修改PATH后就宣称证明了所有动态依赖不存在，应结合平台依赖检查。

## 8. 发布报告与完成条件

QA报告使用 [模板](templates/qa-report.md)，附：binary清单/hash、来源版本、工具链和浏览器、fixture与真实API分别结果、实际费用摘要、性能样例、所有缺陷关闭证据、备份恢复、未支持平台／能力。大日志在artifacts，llmdoc只保留重点与链接。

完整发布必须：当前PRD全部必选AC通过；0个未关闭验收缺陷；真实API在授权范围验证；必选平台实际冷启动；文档与实现一致；无密钥泄露；用户知道如何启动和升级。许可证、第三方资源及模型／说明书来源权利由发布者核查记录，不能因为工具生成就断言权利无风险。

未经授权不自动上传源码、commit/push、发布公网服务、购买API额度或签署账户条款。本轮交付仅文档／协作配置的静态检查，不会产生这些外部动作。

## 9. 执行记录（QA 回合追加；不改动 §1–§8 的命令合同语义）

- **2026-09-13 · QA 回合 29（T21）**：§2 命令合同实测——`cargo xtask check` 7/7 通过；`cargo test --workspace` 558 passed / 0 failed / 2 ignored（两条 ignore 均非必选：T20 手工演练造数、items.rs 文档示例）；`npm --prefix apps/web run test -- --run` 16 files / 130 tests；`npm --prefix apps/web run test:e2e` 88 passed / 0 failed / 0 skipped；`cargo xtask contracts --check` 一致；`cargo xtask smoke-bootstrap` 7 项通过。日志在 `artifacts/web-mvp/t21-qa/`，逐行矩阵与未覆盖边界见 [qa-report 回合 29](requirements/web-mvp/qa-report.md)。
- **尚未执行的合同项**（按卡面归属）：`cargo xtask dist --target` 全项与 `cargo xtask smoke`（AC-002）归 T22；`cargo xtask test-live`（AC-042）归 T23 且需用户授权预算；§3 包装行的"无 dist/源码/Node/Python 环境、离线 vendor、平台动态依赖"归 T22。
- **浏览器矩阵现状**（§4）：e2e 当前仅配置 Playwright Chromium（本机 148.0.7778.96，headless）；本机未安装 Firefox/Edge → AC-063 的"三者全部通过"未满足，归 T22/T23；Safari 未列入支持（符合合同）。
- **CI 现状**：仓库无 `.github/workflows`；合同未把 CI 列为 AC 门禁，本地全量入口是 `cargo xtask check` + 全量 e2e。T22 卡面含"CI release矩阵"，建议随 T22 落地。
- **性能基线**（§4）：阅读器 100k 面旋转 p95 **17.7 ms**（≤33 ms 目标；QA 回合 29 复跑，Chrome 152 + Apple M1 真实 GPU）；本地元数据 API p95 **0.6 ms**（≤200 ms 目标；300 样本）。缺口：≥5 分钟连续旋转、模型加载耗时、PM 具名目标设备尚未记录。
- **2026-09-13 · RD 交付 T22（部分；Linux BLOCKED）**：§2/§6/§7 实测——`cargo xtask dist --target aarch64-apple-darwin`（两次；第二次带 `--check-reproducible`，`cargo clean -p` 后独立重建）sha256 恒为 `ab693cc3…`（25 112 080 B）；`cargo xtask smoke`（§7 的 7 步，含 `sandbox-exec` 断网读取与 backup→restore→再读同 release）通过；`otool -L` 仅 4 个系统库；隔离扫描构建机路径 0 命中；`cargo xtask check` 7/7、`cargo test --workspace` 558/0/2、`smoke-bootstrap` 回归通过；CI（`.github/workflows/ci.yml`）已交付但**未推送未触发**。**Linux x86_64-musl（§6 必选平台之一）未验 = BLOCKED**：宿主 ENOSPC 事件后 Docker Desktop 引擎无法启动（非破坏性恢复动作均无效；Clean/Purge 属破坏性操作待所有者授权），恢复路径见 [implementation §T22-5](requirements/web-mvp/implementation.md) 与 `scripts/linux-musl.sh`。日志：`artifacts/web-mvp/t22-rd/`；决策：ADR-035。
- **2026-09-13 · RD 交付 T22（Linux 半边；原 BLOCKED 已解除）**：§6/§7 实测——容器内（`--platform linux/amd64`，宿主 Apple Silicon + Rosetta）**原生**运行 `cargo xtask dist --target x86_64-unknown-linux-musl`：sha256 `77b38c91…`（28 236 728 B），`--check-reproducible` 独立重链接同哈希；`file` = `static-pie linked`、`ldd` = `statically linked`、`readelf -d` 的 `DT_NEEDED = 0`（无动态依赖）；`isolation.hits = 0`（`remappedHomePathHits = 559`、`nodeModulesLiteralHits = 0`）；`licenses.json` v2 rust 205 / web 40。`cargo xtask smoke`（§7 的 7 步）在容器内通过；Linux 无 `sandbox-exec`，步骤 5 打印跳过提示，其证据改由 `scripts/linux-musl.sh --offline-only`（`docker run --network none` 跑整条 smoke，7 步全过 + 容器内自证无默认路由/DNS 失败）提供 → AC-059 Linux 侧成立。命令入口 `scripts/linux-musl.sh {--arch amd64,--check-reproducible,--offline-only,--prepare-sample-backup,--check}`，分步退出码与耗时见日志。**未决**：Linux 侧 `cargo test --workspace` 有 1 例必现失败（`backup_restore.rs::legacy_schema_backup_restores_and_migrates_automatically`，serve 启动窗口内 SIGTERM 走默认动作，属产品代码范围，本卡未改，见 implementation §T22-13.6）；`gitignore` 的 `*.sqlite3` 使样例备份在全新 checkout 缺 DB，已提供重建入口。日志：`artifacts/web-mvp/t22-rd/linux/**`；决策：ADR-036。
- **2026-09-13 · QA 回合 30（T22 Linux 半边）**：§6/§7 合同项 QA **独立复现**——交付二进制 sha256 `77b38c91…`（28 236 728 B，宿主+容器各算一次）；`file` = `static-pie linked`、`ldd` = `statically linked`、`DT_NEEDED = 0`、**QA 自算**禁用路径 0 命中；容器内**独立重跑** `dist --check-reproducible` 复现同哈希且 `build-info.json` 除 `builtAt` 外逐字段一致；`--network none` 整条 smoke（31 条 `[检查]`）与 `scripts/linux-musl.sh --offline-only` 均通过；`smoke-bootstrap` 7/7；**新增**裸容器（无 Node/Python/源码）冷启动 9 项通过。结论 **PASS**（AC-064 Linux / AC-002 / AC-059 Linux），但 §6 的"原生"须按三层读：目标 OS 内核 + x86_64 ABI 用户态成立，**物理 x86_64 CPU 不成立**（Rosetta），性能类结论不得引用。**新增未关闭缺陷 BUG-013（P3）**：`serve` 启动后约 50–100 ms 窗口内 SIGTERM 走默认动作（实测 143 vs 稳态 0），使 `backup_restore.rs::legacy_schema_backup_restores_and_migrates_automatically` 在 Linux 容器内必现失败（QA 全量枚举 557/1/2，37 目标），**不阻断 T22 切片**但阻断 §8 的"0 个未关闭验收缺陷"与 Linux 侧 `cargo xtask check`/CI `check` job。日志：`artifacts/web-mvp/t22-qa/**`；逐条矩阵、判定理由与未覆盖边界见 [qa-report 回合 30](requirements/web-mvp/qa-report.md)。§1–§8 合同语义未改动。
