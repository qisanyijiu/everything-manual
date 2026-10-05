# 单平台生产部署包

此目录提供生产部署配置。部署时使用同一发行包附带的镜像归档、SHA256 清单、镜像身份和实际验收报告；仓库中的配置本身不代表运行验收通过。此配置不申请域名、证书或云账号，不代表实际公网已经上线。

## 部署

运行机只需 Docker Engine/Compose 和解包/校验工具；不需要源码、Rust、Node、Python或外部数据库。将发行包放到独立目录，校验并导入发行者提供的单平台镜像：

```sh
sha256sum -c SHA256SUMS
docker image load --platform linux/amd64 --input images.tar
cp deploy.env.example deploy.env
chmod 600 deploy.env
```

按发行清单填入不可变应用镜像 ID 和代理 digest，并用 `docker image inspect --platform linux/amd64 <引用>` 核对。`images.tar` 必须包含这两个镜像；配置不会联网拉镜像或尝试现场构建。

管理员自行准备独立私有目录及文件：首次初始化口令（**没有默认生产密码**）、32字节随机主密钥的64位十六进制文本、实际主机的证书完整链、无口令 PEM 私钥。目录0700、口令/主密钥/私钥0600；这些秘密不放入发行包或 Git。已有数据库的登录密码由数据库哈希决定，初始化文件不会替换它。证书应匹配实际浏览器主机名。

在 `deploy.env` 填文件绝对路径及 `EM_PUBLIC_ORIGIN`，必须为实际 HTTPS origin，包含非默认端口，不含路径。默认只发布到 `127.0.0.1:8443`；需要公开入口时，操作者明确设置 `EM_HTTPS_BIND=0.0.0.0`、`EM_HTTPS_PORT=443` 和实际域名。此处使用 Nginx 直接终止 TLS，不再串接会覆盖用户IP的另一层代理。应用只信任同一网络空间的 `127.0.0.1/32`，后端和HTTP健康检查端口不向主机发布。

```sh
dc() { docker compose --env-file deploy.env -f compose.production.yaml -p everything-manual-production "$@"; }
dc config --quiet
dc up -d --force-recreate --no-build --pull never --wait --wait-timeout 180
dc ps
```

使用实际 HTTPS 地址登录；不得使用开发 launcher 启动此包，它会重写公开地址并可能复用旧程序。初次上线应核对证书、Secure登录会话、页面/原件/模型读取，并保存该包及镜像清单。管理员可在设置页配置供应商；API key仍加密保存在私有配置卷，重启后生效。没有密钥时已有资料可读。

## 停服与秘密

```sh
dc stop -t 60
```

停止全部四个服务后，tmpfs运行秘密会清空；主机原秘密文件及数据卷保留。prepare与prepare-tls仅在初始化时读受限主机文件，然后分别降权到10001、101，保持tmpfs挂载。应用/代理为非root、只读根文件系统。不要使用 `down --volumes` 维护生产，它会删除持久数据。

更新TLS证书后保留原证书/私钥用于回退，停止服务并用同一配置执行完整stack的 `up --force-recreate`，让prepare-tls重新复制。每次重新创建manual时必须重新创建proxy，因为proxy共享manual的网络namespace，不能沿用指向旧容器的proxy。主密钥必须持续保留；不允许用新随机密钥替换已有加密配置所需密钥。

## 备份与恢复验证

停服后创建全新备份。以下主机维护目录需由部署管理员创建为0700、UID/GID10001可写；示例 `sudo install` 仅用于管理员准备目录。备份含原件及资料，限制访问。

```sh
dc stop -t 60 proxy manual
sudo install -d -m 0700 -o 10001 -g 10001 /srv/everything-manual/backups
backup_name="$(date +%Y%m%d-%H%M%S)"
dc up -d --no-build --pull never --wait prepare
dc run --rm --no-deps -v /srv/everything-manual/backups:/backup manual \
  backup --data-dir /data --out "/backup/$backup_name"
```

HTTP应用保持停止；prepare/prepare-tls保持挂载，主密钥和TLS秘密不因应用维护清空。只运行无网络秘密引导和维护程序。备份目标不能已存在。恢复检查写入**全新或空**的目录：

```sh
restore_dir="/srv/everything-manual/restore-check-$backup_name"
sudo install -d -m 0700 -o 10001 -g 10001 "$restore_dir"
dc run --rm --no-deps \
  -v /srv/everything-manual/backups:/backup:ro -v "$restore_dir:/restore" manual \
  restore --from "/backup/$backup_name" --data-dir /restore
# 如暂不恢复应用，可以停止全部四个服务清空运行秘密；数据/主机秘密保留。
dc stop -t 60
```

应用恢复会验证hash/数据库引用/外键后写入。备份保留管理员口令哈希，不含会话、供应商配置、主密钥或TLS私钥；这些私有材料须由部署管理员另外保管。要验收恢复目录，停应用后将 `EM_DATA_PATH` 指向这个新目录，继续使用原 `EM_RUNTIME_CONFIG_PATH`（默认同项目manual-config卷）及**同一个**主密钥文件，执行完整stack `up --force-recreate`，核对原发布清单/PDF/GLB SHA。恢复验证通过后才能将其正式作为新的资料库；原数据保留到复核完成。不要因切换data目录而生成新主密钥或新provider配置卷。

## 升级与回滚

先停manual/proxy、做迁移前备份并完成恢复验证；保留旧发行包、镜像归档/引用、公开地址、配置和私有材料。加载新镜像，按新清单更新不可变引用，再执行上面的完整stack `up --force-recreate`。这也确保proxy连接新的manual网络namespace。启动会运行受支持迁移。

回滚必须停止新版本，以旧包/旧镜像从**迁移前备份**恢复到新空数据目录，旧部署的 `EM_DATA_PATH` 指向恢复目录，并继续挂载原provider配置及同一主密钥，再完整重新创建旧stack。只切换旧镜像不能撤销数据库迁移；旧程序会拒绝更新的schema。不要覆盖原数据、删除卷或替换主密钥来绕过错误。

## 独立HTTPS验收（仅QA，不是部署依赖）

验证机需Python3.11+、OpenSSL及Docker/Compose，另准备已完成、无进行中任务的脱敏T20样例备份。脚本只接受本地已有的不可变镜像引用；不构建、不拉镜像，不接触生产数据或真实供应商配置，不调用任何生成API。

```sh
python3 verify-production-https.py \
  --image sha256:APPLICATION_64_HEX_IMAGE_ID \
  --proxy-image nginx@sha256:PROXY_64_HEX_DIGEST \
  --sample-backup /absolute/path/to/completed-t20-sample-backup \
  --out /absolute/path/to/new-https-evidence \
  --port 18443
```

它为127.0.0.1/localhost生成一天有效测试证书，HTTPS客户端仅在当前进程信任该证书，不改系统信任；使用唯一Compose项目/卷和测试密码/主密钥。先验证HTTP origin无法产生就绪TLS入口、损坏PEM证书被nginx拒绝，再检查实际TLS登录、Secure Cookie、CSRF/错误Origin、伪造转发头和限速、内嵌资源/冻结资产SHA、运行权限、镜像身份及重启持久化。仅删除自己创建的项目和卷；`--keep`可保留隔离环境排障。不要发布证据中的 `private/` 子目录。

平台须按实际运行证据声明；默认linux/amd64，不因为支持选择参数就宣称ARM64或物理x86服务器通过。此验收不替代生产域名的真实证书/浏览器验证、整体验收VS-05或完整冷目录/备份恢复测试。
