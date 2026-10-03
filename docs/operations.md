# 运维说明（安装、启动、升级、回滚、备份）

面向部署「万物说明书」单二进制的管理员。命令合同与验收标准见
[llmdoc/validation-release.md](../llmdoc/validation-release.md)；本文件只讲**怎么用**。

## 1. 取得产物与校验

发布产物在构建机 `dist/<target>/` 下（由 `cargo xtask dist --target <target>` 生成）：

| 文件 | 说明 |
| --- | --- |
| `everything-manual` | 单文件可执行程序（内含 Rust 服务、React 构建产物、PDF.js 运行资源、迁移、bundled SQLite） |
| `SHA256SUMS` | 二进制 sha256（`shasum -a 256 -c SHA256SUMS` 可直接校验） |
| `licenses.json` | 第三方许可证清单（Rust 依赖按发布依赖集、前端按 production 条目） |
| `build-info.json` | 版本、target、features、工具链、git 状态、签名声明、隔离扫描、动态依赖摘要 |
| `dynamic-dependencies.txt` | 原生构建时的 `otool -L` / `file`+`ldd` 原始输出 |

```sh
shasum -a 256 -c SHA256SUMS     # macOS
sha256sum -c SHA256SUMS         # Linux
```

**本轮平台支持标签**（未跑平台不得贴支持标签，validation-release §6）：

| 平台 | 状态 | 证据 |
| --- | --- | --- |
| macOS Apple Silicon（`aarch64-apple-darwin`） | 已构建并自证（原生构建 + 可复现哈希 + 冷目录 smoke 7 步 + 断网复检）；**尚未经独立 QA 验收** | `cargo xtask dist --target aarch64-apple-darwin`（本地 `dist/` 已按用户要求清理，可由该命令重建，哈希可复现） |
| Linux x86_64 静态（`x86_64-unknown-linux-musl`） | 已构建并自证（Linux 容器内原生 `dist` + 原生 `smoke` 7 步 + `--network none` 整条 smoke + 静态链接与 0 动态依赖）；**尚未经独立 QA 验收** | `scripts/linux-musl.sh --arch amd64`、`scripts/linux-musl.sh --offline-only`；证据 `artifacts/web-mvp/t22-rd/linux/`，摘要见 [implementation §T22](../llmdoc/requirements/web-mvp/implementation.md) |
| Windows / Intel macOS | **未支持**（未构建、未运行） | — |

按 `validation-release.md` §6：**未跑平台不得贴支持标签**。

Linux 侧复现（Apple Silicon 宿主上跑 x86_64 容器，Rosetta 执行）需要 Docker：

```sh
scripts/linux-musl.sh --arch amd64          # 复制工作树 → 容器内原生 dist + file/ldd + smoke 7 步
scripts/linux-musl.sh --offline-only        # docker run --network none 跑整条 smoke（离线读取证据）
scripts/linux-musl.sh --prepare-sample-backup   # 样例备份缺 DB 时重新生成（见下）
```

**样例备份的已知坑**：`artifacts/web-mvp/t20-rd/sample-backup/database/manual.sqlite3`
被 `.gitignore` 的 `*.sqlite3` 排除，因此**全新 clone 里没有它**，`cargo xtask smoke`
的第 2 步会报"备份快照数据库缺失"（这是环境缺文件，不是产品缺陷）。用
`scripts/linux-musl.sh --prepare-sample-backup`（容器内）或按 T20 的手工演练说明用
`cargo test … prepare_rehearsal_datadir` + `backup` 重新生成一份自洽备份。

macOS 二进制**未做代码签名与公证**（需要用户账号授权）：在其它机器首次运行需在
「系统设置 → 隐私与安全性」按 Gatekeeper 提示放行，或由所有者用自有证书签名。

### 本机预览反复弹出钥匙串授权

管理员网页登录密码只控制应用会话，不控制 macOS 钥匙串。当前直接由 Cargo 构建的
Mach-O 使用临时（ad-hoc）签名；代码更新会改变其身份，即使始终运行在同一路径，
钥匙串仍可能把新版当作新的访问者。保留同一份已授权的二进制并重启，通常无需再次
确认。不要为消除弹窗而将主密钥改为网页登录密码、写入仓库/数据目录，或开放该
钥匙串条目给所有应用。

长期在本机迭代时，可以由所有者**先准备一个现有的 macOS 代码签名身份**，再将每次
预览构建签成相同的身份和固定标识。仓库提供可选的
[`scripts/macos-sign-local.sh`](../scripts/macos-sign-local.sh)：

若 `security find-identity -v -p codesigning` 显示 0 个可用身份，先按
[Apple 的钥匙串访问指南](https://support.apple.com/guide/keychain-access/create-self-signed-certificates-kyca8916/mac)
在「钥匙串访问 → 证书助理 → 创建证书」建立**仅用于本机预览的代码签名身份**；
私钥留在用户钥匙串，不导出到仓库。创建后重新运行 `find-identity`，确认有可用的
40 位指纹，再签新包。证书、签名标识与首次「始终允许」授权应作为同一套身份持续复用。

```sh
security find-identity -v -p codesigning # 选择已有身份的 40 位 SHA-1 指纹
mkdir -p var/local-signed-bin
scripts/macos-sign-local.sh target/release/everything-manual \
  var/local-signed-bin/everything-manual.next YOUR_40_CHARACTER_IDENTITY_SHA1
# 停服后，原子替换稳定路径上的可执行文件，再重启服务。
```

脚本只签署一个**新路径**，不会创建/导出证书、改动钥匙串条目或替换运行中的程序；
它也不签署、公证 `cargo xtask dist` 的正式发行包。首次改用此身份访问原钥匙串条目时，
macOS 仍可能要求所有者在系统弹窗选择“始终允许”。以后保持同一签名身份与标识，
新版本才可继承这项信任；换证书、换系统用户、条目权限变化或钥匙串锁定都可能再次
要求授权。签名会改变二进制哈希，需对**签名后的文件**重新校验并记录哈希，不能沿用
原构建包的 `SHA256SUMS`。这项本机预览做法不等同于正式发行所需的签名与公证。

## 2. 首次部署

```sh
mkdir -p /srv/manual-data            # 数据目录：独立、可写、随备份保护
./everything-manual init --data-dir /srv/manual-data
# 交互输入管理员密码（不回显）。无人值守用 --password-file <0600 文件>。
./everything-manual check --data-dir /srv/manual-data
./everything-manual serve --data-dir /srv/manual-data --listen 127.0.0.1:8080
```

- 运行期**不需要** Node / Python / PDF 程序 / 外部数据库 / Redis；Linux musl 产物为静态链接。
- 服务启动后向 stdout 打印 `listening on http://<addr>`；日志（结构化 JSON）不包含密钥、
  原始 PDF 内容与签名 URL 查询串。
- 同一 data-dir 只允许一个进程持有排他锁；重复启动会失败而不是并行写库。
- `serve` 默认只监听 `127.0.0.1`。

### API 密钥与主密钥

在「设置 → API 配置」新增或替换的密钥，以及部署 `api_key_file`，均使用 AES-256-GCM 认证加密。macOS 默认使用运行服务的系统用户的钥匙串；Linux 等环境须由部署侧秘密管理机制注入 `EM_SECRETS_MASTER_KEY`（32 字节随机值的 64 位十六进制编码）。macOS 无人值守服务也可显式注入。应用不会将主密钥写入数据目录，也不会自动从项目 `.env` 读取它。

重启须继续使用原主密钥。请将主密钥与密文分开保管，勿放入命令行参数、仓库或同一备份。没有任何持久密钥时，服务仍可启动阅读已有资料；首次保存密钥需要可用的主密钥来源。供应商 `api_key_env` 仍只在内存中使用。

旧版网页明文覆盖由新版 `serve` 在持有数据目录锁、启动 HTTP 与任务执行器前迁移，`check` 只读并提示迁移。旧明文 `api_key_file` 会被拒绝，需先运行 `encrypt-api-key` 输出新的密文文件并更新配置引用；原文件由操作者验证后处理。具体命令、错误恢复与保护范围见 [API 配置指南](api-settings.md#旧密钥迁移)。

## 3. 部署边界（PRD §5.6 / A-15）

- **本机使用**：直接 `serve --listen 127.0.0.1:8080`。
- **对局域网／公网暴露**：放到显式受信的反向代理之后，由代理终止 TLS 并转发到 loopback；
  应用侧配置 `trusted_proxy_cidrs` 并在反向代理模式下显式设置会话 cookie `Secure`。
- **MVP 不包含内置 TLS 监听**：配置 `tls.cert_file/key_file` 时服务**拒绝启动**（fail-closed，
  不会降级为明文）。这是期望语义，不是故障。
- 不要相信任意 `X-Forwarded-*`；只有配置了 `trusted_proxy_cidrs` 才采纳代理头。

## 4. 数据目录

```text
<data-dir>/
  manual.sqlite3    SQLite（WAL）；schema 版本随程序迁移
  blobs/<前2位>/<sha256>   用户原始资料与模型产物（内容寻址）
  tmp/              上传与导出临时区（崩溃残留会在下次启动被隔离）
  quarantine/       孤儿/残留隔离区（只移动不删除，供人工清理）
  logs/             服务日志
  lock              排他锁（flock；进程退出即释放）
  provider-overrides.json  可选网页覆盖；API key 为密文，不含主密钥
```

备份包含用户原始资料，请按部署者的数据保护要求限制访问；不要把备份上传给未授权第三方。

## 5. 备份、升级与回滚

```sh
# 1) 停服（确认服务进程已退出，排他锁已释放）
# 2) 备份到新路径（必须不存在，不覆盖；运行中执行会以退出码 5 拒绝）
./everything-manual backup --data-dir /srv/manual-data --out /srv/backups/2026-09-13
# 3) 校验备份可恢复（可选但推荐：恢复到临时目录后用同一个二进制启动复核）
./everything-manual restore --from /srv/backups/2026-09-13 --data-dir /srv/restore-check
# 4) 替换二进制，启动时自动执行受支持迁移（会打印「升级前请备份」提示）
./everything-manual serve --data-dir /srv/manual-data
```

- 备份是 `VACUUM INTO` 的一致快照（含 WAL 中已提交事务）+ 全部被引用 blob + manifest + sha256；
  备份中不含会话（恢复后需重新登录），保留管理员口令哈希（否则灾备后无法登录）。
- 私有 API 配置和主密钥不包含在应用备份/导出中，恢复后需重新配置。迁往无法访问原钥匙串或原环境注入的环境时，手工复制数据目录无法解密，还须另行恢复原主密钥；同一 macOS 用户的默认钥匙串在多个数据目录间共享主密钥。升级不会清除旧日志、历史备份或文件系统快照中的既有明文。
- 恢复到**不存在或为空**的目录；先全量校验（hash/外键/引用）再写入；校验失败退出码 7 且
  不创建目标目录。
- **回滚 = 停服后用旧二进制 + 迁移前备份恢复到新空目录**；程序回滚不等于数据库回滚，
  旧程序不能打开比它新的 schema（会以退出码 4 拒绝）。

退出码：`0` 成功；`1` 一般错误；`2` 用法错误；`4` 路径/存储/schema 门禁；`5` 备份需先停服；
`7` 备份/恢复完整性校验失败。

## 6. 离线与外部服务

- 外部 AI（Tripo / 说明书 AI）未配置或不可达时：已有物品、部件、步骤、原文页图与本地 PDF
  仍可完整读取；`/health/ready` 不因云端不可达失败；生成与报价返回 409「供应商未配置」，
  **不会**回退到 mock 或假成功。
- 服务停止则网站不可用——这是部署边界，不是离线 PWA 能力。
- 供应商关停或密钥撤销后，历史说明书仍本地可读。

## 7. 许可证与第三方资源

- `licenses.json` 列出进入发布包的全部 Rust 依赖与前端 production 依赖的许可证标识；
  完整文本在 crates.io 源码包（cargo registry 缓存）与 `node_modules/<包>/LICENSE*`。
- 运行资源中 PDF.js 与其编解码依赖（OpenJPEG / JBIG2 / QCMS / Liberation / Foxit）的许可证
  文本随二进制内嵌在 `/vendor/pdfjs/*/LICENSE_*`（HTTP 可读）。
- 项目自身许可证未在仓库声明（`publish = false`，许可证选择属所有者决定）；用户上传的
  资料与生成的模型/说明书内容的权利由发布者核查记录。

## 8. 排障

| 现象 | 处理 |
| --- | --- |
| 启动报「data-dir 被占用」 | 已有进程在运行；先停止它（锁随进程退出释放，`kill -9` 也不会留死锁） |
| 启动报「schema 比程序新」 | 用与数据同代的（或更新的）程序；恢复请用迁移前备份到新目录 |
| `backup` 退出码 5 | 服务仍在运行；停服后重试 |
| `restore` 退出码 7 | 备份损坏或不完整；用 `shasum -a 256 -c SHA256SUMS` 在备份目录内定位 |
| 页面 404 但 API 正常 | 确认访问的是二进制自带的地址（内嵌 UI 随二进制发布，不需要外部 dist 目录） |
| 生成按钮报「供应商未配置」 | 在网页「设置 → API 配置」填写密钥与模型并重启；也可继续使用服务端 `api_key_env` / `api_key_file`。网页覆盖优先，详见 [配置说明](api-settings.md) |
| 保存密钥报主密钥不可用 | 恢复服务运行用户的原系统钥匙串访问后可重试；若修正 `EM_SECRETS_MASTER_KEY` 注入，需重启服务后再保存。页面保留当次编辑，不持久缓存秘密 |
| 启动报密文认证失败 | 恢复原主密钥并检查密文完整性；不要删除密文或改回明文绕过错误 |
| 启动/check 报旧密钥格式 | 网页旧覆盖须通过正常 `serve` 迁移；外部 `api_key_file` 按 [迁移指南](api-settings.md#旧密钥迁移) 生成新密文并更新引用 |

## 9. 构建与 CI

- 本地发布：`cargo xtask dist --target <triple>`（校验工具链 → 前端 `npm ci/typecheck/test/build`
  → Rust `--release --locked --features embedded-ui` → 产出上表 5 个文件）；
  `--check-reproducible` 会清理本包构件重建一次并比对二进制 sha256。
- 正式包验收：`cargo xtask smoke --binary <绝对路径>`（7 步：冷目录 → 样例备份 restore → 登录/
  资源/路由 → Range/HEAD/未配置拒绝 → 断网读取 → 重启持久化 → backup→restore 再读）；
  这是**发布包检查**，与 T01 的 `smoke-bootstrap` 不同。
  样例备份不在仓库里完整存在时，先按 §1 的 `--prepare-sample-backup` 重新生成。
- Linux 侧（Apple Silicon / 任意宿主）：见 §1 的 `scripts/linux-musl.sh` 三条命令；
  容器内是**原生 Linux x86_64 构建与运行**（`file` 结论为 `static-pie linked`、`ldd` 为
  `statically linked`、`DT_NEEDED` 为 0），离线读取证据由 `--network none` 整条 smoke 提供。
- CI：`.github/workflows/ci.yml`（推送到 master / PR 触发）跑 `cargo xtask check`，并在
  `dist-musl` job 中原生构建 Linux musl 产物与 smoke（该 job 会先用 T20 造数用例重建样例备份，
  因为 `*.sqlite3` 不入库）。CI **未**覆盖浏览器矩阵（Firefox/Edge）、macOS 构建与
  真实 Provider（需授权，属 T23）。
