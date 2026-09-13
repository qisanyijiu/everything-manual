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
| Linux x86_64 静态（`x86_64-unknown-linux-musl`） | **未验证**（构建与运行证据均缺失：本机 Docker 引擎故障，环境阻塞；脚本 `scripts/linux-musl.sh`、CI `dist-musl` job 已就绪但未执行） | — |
| Windows / Intel macOS | **未支持**（未构建、未运行） | — |

按 `validation-release.md` §6：**未跑平台不得贴支持标签**；Linux 平台在取得原生运行证据前不得对外声明支持。

macOS 二进制**未做代码签名与公证**（需要用户账号授权）：在其它机器首次运行需在
「系统设置 → 隐私与安全性」按 Gatekeeper 提示放行，或由所有者用自有证书签名。

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
| 生成按钮报「供应商未配置」 | 在服务端配置密钥（`api_key_env` / `api_key_file`）与模型后重启 |

## 9. 构建与 CI

- 本地发布：`cargo xtask dist --target <triple>`（校验工具链 → 前端 `npm ci/typecheck/test/build`
  → Rust `--release --locked --features embedded-ui` → 产出上表 5 个文件）；
  `--check-reproducible` 会清理本包构件重建一次并比对二进制 sha256。
- 正式包验收：`cargo xtask smoke --binary <绝对路径>`（7 步：冷目录 → 样例备份 restore → 登录/
  资源/路由 → Range/HEAD/未配置拒绝 → 断网读取 → 重启持久化 → backup→restore 再读）；
  这是**发布包检查**，与 T01 的 `smoke-bootstrap` 不同。
- CI：`.github/workflows/ci.yml`（推送到 master / PR 触发）跑 `cargo xtask check`，并在
  `dist-musl` job 中原生构建 Linux musl 产物与 smoke。CI **未**覆盖浏览器矩阵（Firefox/Edge）、
  macOS 构建与真实 Provider（需授权，属 T23）。
