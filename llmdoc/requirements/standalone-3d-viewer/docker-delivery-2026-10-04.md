# Docker、Linux与统一资料交付 · 2026-10-04

本轮完成Linux镜像、Compose部署、完整Linux工程检查、正式发行包七步检查及统一资料入口验证。
统一资料库包含8个物品，Nikon F3HP在本机与Linux容器中均可读取原始发布版本及3D交互。
实测范围是ARM虚拟机经Rosetta运行的Linux AMD64 ABI；本轮付费请求0。

本轮基于本地 `master` 的工作树变更。镜像构建上下文不带 `.git` 或私有资料，
发行包 `build-info.json` 如实记录 `git.commit=null`、`git.dirty=true`；本记录不将该产物归为已提交的干净版本。
既有合并、macOS发行包和真实Nikon生成的历史事实保留在
[原交付验收](delivery-2026-10-04.md)，不以本轮结果改写当时尚未验证Linux的状态。

## 统一入口与资料

项目根目录统一使用：

```sh
bash scripts/start-project.sh           # macOS默认本机程序；Linux默认Docker
bash scripts/start-project.sh --docker  # 同一份资料改用Docker
bash scripts/start-project.sh --status
bash scripts/start-project.sh --stop
```

浏览器地址为 `http://127.0.0.1:8080/`；当前本机登录密码为 `12345678`。
所有日常资料使用 `var/preview/data`，旧 `scripts/start-wiiu-preview.sh` 转发到同一入口。
资料与私有配置不会进入Git，全新克隆会建立空资料库并在首次启动要求设置口令。

| 统一资料 | 数量 |
| --- | ---: |
| 物品 | 8 |
| 说明书 / 已提取页 | 7 / 166 |
| 资产记录 / 内容blob | 368 / 361 |
| 草稿 / 不可变发布版本 | 3 / 2 |

8个物品包括Nikon F3HP、Nintendo Wii U、IKEA TILLREDA/BILLY/LACK、Roland TR-808及既有交互/备份演示。
说明书原件、模型、供应商任务与账务记录按原ID保留；合并不启动worker、不购买任务，旧会话不沿用。
历史快照与合成压力数据集中归档，不混入日常预览列表。

38个旧data-dir已可逆移动到 `var/preview/archive/originals`，旧路径保留相对软链接。
迁移前后核对SQL内容、文件SHA及目录inode；补充的3份旧备份与1份预检数据也已移动到统一目录。
没有删除原始资料。迁移、预检、备份、合法回归样例与验证证据均在 `var/preview/` 下。
最终检查确认项目 `var/preview` 外没有物理应用数据库；SQLite完整性与外键均通过，
361个存储blob的大小/SHA全量一致，供应商请求记录42条和账务记录10条没有增加。

macOS本机启动复用已授权的冻结程序和原钥匙串密文，未替换该二进制或导出主密钥。
Docker使用 `private/docker-master.key` 和独立 `private/docker-config`；历史资料读取不需要旧钥匙串，
继续生成时需在Docker设置页重新录入API配置。主密钥与API配置不包含在应用备份或导出中。

## 镜像与Linux发行程序

实测宿主为macOS Apple Silicon；独立虚拟机为Ubuntu26.04 ARM，Docker Engine29.8.2、Compose5.1.3。
`linux/amd64` 容器经Rosetta执行，发行目标为 `x86_64-unknown-linux-musl`。
这是Linux AMD64 ABI的构建和运行验证；未宣称物理x86服务器或原生Linux ARM64已验证。
原Docker Desktop数据没有被重置、清空或替换。

Docker多阶段构建只在构建阶段使用Rust和Node；运行镜像包含单个应用程序及CA证书、引导工具。
Compose的 `prepare` 在无网络容器中读取0600秘密文件、注入tmpfs并降权，
`manual` 和 `proxy` 以非root运行，只读根文件系统，持久资料挂载在独立目录。
入口默认仅向主机loopback发布8080端口。

| 产物 | 实测值 |
| --- | --- |
| Linux发行目录 | `dist/x86_64-unknown-linux-musl/` |
| 二进制大小 | 31,736,608字节 |
| 二进制SHA256 | `640fe94aceab095f3f651aa544ee6edabb19083a1c08acff750a12c10cd54614` |
| 静态链接 | `static-pie`，`DT_NEEDED=0`；运行期不需要Node/Python/外部SQLite/PDF程序 |
| 已验证应用镜像 | `everything-manual:local`，`sha256:c00bed32fad72b2a1b190974c035748761915bde1a2590713186b5fc09e83c16` |
| 全量检查镜像 | `everything-manual:linux-validation`，index `sha256:6033a3abb8a13ec8827bdd0810ef832d64a608937d5d9fea8a0b3b2e8cbc6a97` |
| 代理镜像 | `nginx:1.28-alpine`，digest `sha256:a8b39bd9cf0f83869a2162827a0caf6137ddf759d50a171451b335cecc87d236` |

构建上下文通过 `.dockerignore` 与显式源文件COPY排除数据、密钥、历史日志、Git目录、宿主构件和node_modules。
发行包附带SHA清单、许可证清单、构建信息和动态依赖原始证据；二进制内构建环境路径扫描0命中。
启动、密钥、维护与恢复步骤见 [Docker部署](../../../docs/docker.md)。

## 已完成验证

`scripts/verify-docker.sh` 使用隔离数据、临时口令和临时测试key，运行六阶段、九份HTTP检查报告，
合计647项断言通过。没有读取真实供应商key，没有调用付费生成API。

| 验证 | 结果 |
| --- | --- |
| 空目录初始化 | 管理员登录、内嵌UI/PDF.js、上传、网页API配置加密及重启读取通过 |
| 合法样例恢复 | 冻结发布、PDF、GLB与重启持久化通过 |
| 停服备份 / 新空卷恢复 | 发布清单和资产SHA一致；不包含API配置或主密钥 |
| 无网络读取 | `--network none`中读取26项内嵌资源及冻结发布；2份PDF/GLB资产SHA一致，供应商请求0 |
| HTTPS与信任边界 | Secure cookie/Origin校验通过；Rust供应商客户端发出1次真实证书校验HTTPS GET，目标 `https://example.com/models` |
| 秘密与运行权限 | 加密配置、0600运行秘密、非root应用/代理、只读根文件系统及证据/备份/镜像环境无测试秘密通过 |

HTTPS探测使用临时测试key，只验证证书与传输，计费请求0；本轮没有新增真实Nikon生成账务。
应用仍不内置TLS监听，公网部署仍须按运维合同配置受信TLS反向代理。

Chrome按用户要求使用系统Chrome154.0.8037.93，浏览器在macOS宿主运行，分别连接本机服务与Linux容器服务。
本机程序与Linux容器都读取同一统一资料库，
均验证8个物品及两个发布版本，五份发布资产SHA与原件相同。
Nikon发布清单SHA保持 `472eee72adb6183a0c7345d1aa0e0ceecc9ceba399065b03d5b3597628e5aae1`，
141个部件、75个步骤、116项参数及原模型版本保持一致；3D动作切换改变画布，复原后画布内容SHA与初始相同。
Chrome复核只有登录与本地读取，没有外网请求、供应商调用、发布或生成重试。

统一启动脚本已实际完成本机启动、重复启动、停止、重启和再次重复启动；
两次重复启动均保留同一进程，停止后可正常重新取得资料锁并启动。

## 最终工程检查

| 检查 | 结果 |
| --- | --- |
| Linux `cargo xtask check` | UID10001执行，Rust671通过、0失败、8项按默认忽略；前端46文件/311项通过；fmt、Clippy（warnings为错误）、lint、typecheck及合同漂移均通过 |
| Linux正式七步 `xtask smoke` | 由已编译QA检查程序执行，七步全部通过，退出码0；生产二进制SHA与上表一致，整个检查在 `--network none` 中运行 |

完整检查的权限测试需要普通用户语义。初轮构建进程为root，能够绕过0500目录限制，导致一项密钥QA失败；
验证阶段改为UID10001重新跑完整检查，11项密钥QA及其余测试均通过，没有跳过失败用例。

正式七步检查以UID501/GID20运行，只读根文件系统、全部capabilities移除、`no-new-privileges`，
只挂载只读发行目录、合法样例备份和独立QA检查程序；没有源码或Rust/Node/Python运行环境，
被测应用子进程清空环境，从冷目录启动。Linux没有macOS的 `sandbox-exec`，
第5步由整个容器的 `--network none` 提供实际断网边界，重启和二次恢复后的发布/模型/PDF哈希均一致。
最初摘要程序期待不存在的第3/4步标题而误报，原始应用退出码0、七步完成行均已核实；
只修正摘要解析，保留原解析失败记录，没有为此重复测试。

macOS上一轮669项Rust、311项前端、140项Chrome及真实Nikon70credits完整流程属于原交付报告，
不以这些数字代替本轮Linux工程检查。

## 证据索引

下列私有证据保留在本机、被Git忽略；仓库文档只记录结果与相对路径，不复制密钥或说明书内容。

| 证据 | 路径 |
| --- | --- |
| 合并报告 | `var/preview/migration-apply-20261004/report.json` |
| 38份旧目录移动与兼容链接 | `var/preview/archive/originals-apply-20261004.json` |
| 补充备份/预检原件移动 | `var/preview/archive/supplemental-originals-20261004.json` |
| 归档脚本安全验证 | `var/preview/originals-script-verification.json` |
| Docker六阶段647项汇总 | `var/preview/validation/docker-final-v4/summary.json` |
| 无网络运行报告 | `var/preview/validation/docker-final-v4/offline-http-verification.json` |
| 本机Chrome读取报告 | `var/preview/validation/chrome-native/restart-browser-verification.json` |
| Linux容器Chrome读取报告 | `var/preview/browser-check/restart-browser-verification.json` |
| 统一脚本本机启停 | `var/preview/validation/launcher-native.json` |
| 最终资料完整性/存储blob/原配置与程序保持 | `var/preview/validation/final-data-verification.json` |
| Linux完整工程检查 | `var/preview/validation/linux-check/summary.json`、`build.log` |
| Linux正式七步检查 | `var/preview/validation/linux-cold/summary.json`、`smoke.log` |
| 七步摘要解析修正追溯 | `var/preview/validation/linux-cold/summary-parser-failure.json` |
| Linux发行包元数据与依赖 | `dist/x86_64-unknown-linux-musl/build-info.json`、`dynamic-dependencies.txt`、`SHA256SUMS` |
