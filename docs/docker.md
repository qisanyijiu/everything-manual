# Docker部署与统一启动

本文以项目根目录为工作目录。Docker镜像、Compose与统一资料读取已完成本轮实际验证；
结果、平台边界及证据索引见 [Docker/Linux交付记录](../llmdoc/requirements/standalone-3d-viewer/docker-delivery-2026-10-04.md)。
macOS及Nikon真实供应商流程的原始验收见 [2026-10-04交付文档](../llmdoc/requirements/standalone-3d-viewer/delivery-2026-10-04.md)。

## 一条命令启动

先安装并启动Docker Engine或Docker Desktop，再运行：

```sh
bash scripts/start-project.sh --docker
```

脚本构建镜像、启动Compose、等待就绪，并在有Chrome的本机打开 `http://127.0.0.1:8080/`。
服务在后台运行，关闭终端不影响服务；首次构建需要网络下载基础镜像和构建依赖。
macOS日常预览可直接用 `bash scripts/start-project.sh`，默认启动本机程序；Linux默认选择Docker。

当前本机统一资料库位于 `var/preview/data`，可浏览8个有效样本，包含Nikon F3HP和Wii U，
登录密码为 `12345678`。数据不进入Git；全新克隆会建立空资料库，脚本在终端要求设置首次登录密码且不回显。
这份密码文件仅用于首次初始化，已有数据库的登录密码由数据库内的口令哈希决定。

统一资料库已合并368条资产、361个内容blob、7份说明书/166页、3份草稿及2个发布版本。
38个旧数据目录已可逆移动到 `var/preview/archive/originals`，旧位置只保留兼容软链接；
数据库、文件哈希及原目录inode均已核对，历史快照与压力测试资料集中在 `var/preview/archive`。
日常阅读只打开统一资料库，归档不是新的运行实例，也不会再次触发供应商生成。

```sh
bash scripts/start-project.sh --status
bash scripts/start-project.sh --stop
bash scripts/start-project.sh --docker --no-open
```

重启：先执行 `--stop`，再执行 `--docker`。切换本机程序与Docker时脚本会先停止另一种方式，
两者读取同一资料库，不能同时运行。`--stop`只停服务，保留资料与配置。

## 路径、端口与密钥

| 主机路径 | 用途 |
| --- | --- |
| `var/preview/data` | SQLite、说明书、模型、冻结发布版本等持久化资料 |
| `var/preview/private/password.txt` | 首次管理员初始化口令，0600 |
| `var/preview/private/docker-master.key` | 32字节随机主密钥的64位十六进制文本，0600；与数据目录分开 |
| `var/preview/private/docker-config` | Docker专用的加密API配置，私有目录0700 |
| `var/preview/logs` | 统一启动日志 |
| `var/preview/archive` | 旧资料原件、历史快照和迁移记录；不会启动worker |
| `var/preview/test-fixtures` | 合法回归样例备份 |
| `var/preview/validation` | 本轮隔离验证资料与证据 |

脚本首次创建Docker主密钥，后续复用已有文件，不打印主密钥，不将其作为命令行参数。
这些文件被Git忽略；不要将整个 `var/preview/private` 或密钥文件加入Git。
容器以文件秘密来源加载主密钥，网页保存的API key继续使用AES-256-GCM加密。
容器的私有API配置目录与 `/data` 分离，因此原macOS钥匙串密文无需导出或覆盖。

macOS本机预览继续使用原先的钥匙串配置与已授权程序；Docker/Linux不能读取该钥匙串。
切换到Docker后，已有说明书和发布版本仍能阅读；需要生成时，在
`http://127.0.0.1:8080/settings` 的「API配置」录入地址、模型和key，再停止并启动服务使其生效。
API key不要发到聊天、放入shell参数或提交到 `.env`。应用备份不包含API私有配置与主密钥。

默认只向主机 `127.0.0.1:8080` 发布端口。容器网络内的反向代理配置由Compose负责，
本机入口不需开放局域网端口。公网部署需另行配置受信TLS反向代理，见 [运维边界](operations.md#3-部署边界prd-56--a-15)。

可选择另一个统一资料库和本机端口；重复启动必须使用同一组值：

```sh
EM_PREVIEW_ROOT=/srv/everything-manual EM_PREVIEW_PORT=8081 bash scripts/start-project.sh --docker --no-open
EM_PREVIEW_ROOT=/srv/everything-manual EM_PREVIEW_PORT=8081 bash scripts/start-project.sh --stop
```

`EM_PREVIEW_ROOT`必须是独立目录的绝对路径；登录口令、主密钥及API私有配置位于其 `private/`，
资料位于 `data/`。选择目录时确保部署用户可写，并限制私有目录访问。
使用普通用户运行脚本，保持主机资料和容器UID/GID一致。

## 镜像与Compose

仓库提供 `Dockerfile` 与 `compose.yaml`。统一启动脚本负责设置数据路径、私有配置路径、
主机UID/GID、秘密文件与端口，然后执行Compose，不要求本机安装Rust或Node。
默认镜像平台为 `linux/amd64`；可用 `EM_DOCKER_PLATFORM` 显式选择待构建的平台。
不同平台的实际支持状态以本文最后的验证记录为准。

Compose包含三个服务：`prepare`在无网络容器中完成秘密引导，将主机0600文件复制到tmpfs，
随后降权为应用用户，以内存卷守护进程维持挂载；`manual`以非root运行Rust服务；
`proxy`以非root运行本机反向代理。后两者使用只读根文件系统及独立临时目录。
秘密文件不进入镜像层，主机权限不必改成全员可读。

首次部署不应手工执行 `docker compose up`绕过私有文件初始化；优先使用统一脚本。
如需排查，先确认 `bash scripts/start-project.sh --status` 输出及Docker引擎状态。
数据目录受排他锁保护，重复服务不能并发写入同一数据库。

## 备份、恢复与升级

备份前执行 `bash scripts/start-project.sh --stop`，确认服务已停止。
在项目根目录、同一个终端设置Compose与统一入口一致的路径：

```sh
export EM_DATA_PATH="$PWD/var/preview/data"
export EM_RUNTIME_CONFIG_PATH="$PWD/var/preview/private/docker-config"
export EM_ADMIN_PASSWORD_FILE="$PWD/var/preview/private/password.txt"
export EM_MASTER_KEY_FILE="$PWD/var/preview/private/docker-master.key"
export EM_RUNTIME_UID="$(id -u)"
export EM_RUNTIME_GID="$(id -g)"
export EM_HTTP_PORT=8080
export EM_PUBLIC_ORIGIN=http://127.0.0.1:8080
```

自定义 `EM_PREVIEW_ROOT` 的部署请相应替换上面四个路径。使用独立Lima引擎时，
还需使用该引擎的Docker context或设置它的 `DOCKER_HOST`；本机本轮的socket位置为
`unix://$PWD/var/preview/vm/linux/sock/docker.sock`，统一启动脚本会自动检测它。

全体停止后tmpfs中的运行密钥会清空。先只启动无网络的 `prepare` 重新注入主密钥，
确认它已健康，再创建应用备份；此时HTTP服务保持停止，备份目标不会覆盖已有路径：

```sh
umask 077
mkdir -p var/preview/backups
backup_name="$(date +%Y%m%d-%H%M%S)"
docker compose -f compose.yaml -p everything-manual up -d --wait prepare
docker compose -f compose.yaml -p everything-manual run --rm --no-deps \
  -v "$PWD/var/preview/backups:/backup" manual \
  backup --data-dir /data --out "/backup/$backup_name"
```

恢复检查写入新空目录，先校验SHA、数据库引用与外键，再写入：

```sh
mkdir -p var/preview/restore-check
docker compose -f compose.yaml -p everything-manual run --rm --no-deps \
  -v "$PWD/var/preview/backups:/backup:ro" \
  -v "$PWD/var/preview/restore-check:/restore" manual \
  restore --from "/backup/$backup_name" --data-dir /restore
```

`backup_name`使用上一步真实生成的目录名；`restore-check`必须不存在或为空。
确认恢复结果后再选择它作为新的资料目录，保留原目录直到复核通过。
已有资料库可重新执行 `bash scripts/start-project.sh --docker` 继续使用。
若完成维护后暂不启动服务，可执行 `docker compose -f compose.yaml -p everything-manual stop prepare` 清空内存秘密卷。

应用备份包含数据库与被引用的原件/模型，保留登录口令哈希，不包含会话、API配置或主密钥。
主密钥和加密API配置应由管理员另外保管；恢复后可在网页重新录入供应商配置。
详细数据合同和退出码见 [运维说明](operations.md#5-备份升级与回滚)。

升级顺序为停服、建立应用备份、保留旧镜像版本、更新代码并重新运行统一Docker启动命令。
启动时执行受支持的数据库迁移。回滚需用旧镜像和迁移前备份恢复到新空目录；
只换回旧镜像不能撤销数据库迁移。

## 本轮Linux验证状态（2026-10-04）

实测环境为Ubuntu26.04 ARM虚拟机、Docker Engine29.8.2、Compose5.1.3，
通过Rosetta运行 `linux/amd64` 容器。项目发行目标为 `x86_64-unknown-linux-musl`；
本轮没有验证物理x86服务器或原生Linux ARM64。原Docker Desktop资料未重置或删除。

| 检查 | 结果 |
| --- | --- |
| 项目镜像与Compose | 构建、非root应用、只读根文件系统、健康检查、SIGTERM停服和重新启动通过 |
| 六阶段隔离验证 | 647项HTTP断言通过，覆盖空目录初始化、登录/上传/加密配置、冻结发布读取、重启、备份恢复及TLS边界 |
| 无网络运行 | `--network none`中读取发布、PDF/GLB及26项内嵌资源，2项资产哈希一致；运行期没有Node/Python或源码挂载 |
| 真实HTTPS证书验证 | Rust供应商HTTP客户端向 `https://example.com/models` 发出1次GET，使用临时测试key；证书验证通过，0次付费请求 |
| 统一资料Chrome复核 | 本机与Linux容器均能浏览8个物品；两个发布版本的五份资产SHA一致，Nikon3D展示与复原通过 |
| 统一脚本本机启停 | 启动、重复启动保持同一进程、停止、重启及再次重复启动通过 |
| Linux单二进制 | 31,736,608字节，静态musl，`DT_NEEDED=0`；已导出 `dist/x86_64-unknown-linux-musl/` |
| 完整Linux工程检查 | UID10001执行 `cargo xtask check` 通过：Rust671项，0失败、8项按默认忽略；前端311项；fmt/Clippy/lint/typecheck/合同漂移均通过 |
| Linux正式发行包七步检查 | 整条在 `--network none` 中通过，退出码0；无源码/Rust/Node/Python，涵盖冷目录、恢复/初始化、资源/认证、断网读取、重启及备份恢复 |

发行程序SHA256为 `640fe94aceab095f3f651aa544ee6edabb19083a1c08acff750a12c10cd54614`。
隔离验证汇总在 `var/preview/validation/docker-final-v4/summary.json`，Chrome报告在
`var/preview/browser-check/restart-browser-verification.json` 和
`var/preview/validation/chrome-native/restart-browser-verification.json`；脚本启停报告为
`var/preview/validation/launcher-native.json`。证据保留在本机被Git忽略的目录。
完整工程检查与正式七步检查分别见 `var/preview/validation/linux-check/summary.json` 和
`var/preview/validation/linux-cold/summary.json`，原始日志也保留在相同目录。
上述公网HTTPS请求只验证真实证书与传输，不触发模型生成；公网入口仍须按部署边界配置TLS反向代理。

macOS既有669项Rust、311项前端、140项Chrome及真实Nikon证据仍见
[已完成的交付验收](../llmdoc/requirements/standalone-3d-viewer/delivery-2026-10-04.md)。
