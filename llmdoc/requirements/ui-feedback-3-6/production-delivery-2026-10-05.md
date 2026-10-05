# VS-05 生产交付记录

> 本页为历史 QA6/bb88 来源验收与包记录，已由用户后续 E2E-01 工作接续。当前来源、新包和最终状态以[完整 E2E 与交付复验](e2e-delivery-2026-10-05.md)及[state](state.yaml)为准；下文旧哈希与签署保留，不代表本轮修复后版本。

日期：2026-10-05。最终状态：**VS-01～05全部接受，QA6全部12AC PASS，正式运行及包装修订2独立交付审计PASS；可按目标机前置与部署说明上线。** VS-03由Claude QA3接受；VS-04由Codex完成并经独立QA5接受；VS-05由Codex实施并独立验收。现有规划/state已更新为完成；社区CI-00～06沿用用户取消决定，保留PRD。本轮未执行公网部署。

## 实现与协作结果

视觉 PRD/UI 修订 1 保持冻结。VS-04 完成上传、任务、设置及版本列表的视觉与状态表达，两处移动触控尺寸经返工关闭。VS-05 统一正文行高和独立 HTML 标题/主按钮，修复 WebGL 降级后的选择提示及阶段编号语义；保留真实错误、费用、发布冻结数据和用户并行的阅读/分件能力。smoke 实际启用 macOS OS 沙箱，并正确识别无子进程。生产配置采用内嵌 UI 镜像、HTTPS、Secure Cookie、非 root/只读服务及独立私有凭据。

再次核对 Claude 日志，最后业务回复为 2026-10-05 00:39:46（Asia/Shanghai）的 VS-03 QA3 PASS，随后 00:39:53 用户中断；没有 VS-04 实际派发。Codex 按用户授权接手后续交付。共享分工见 [交接记录](codex-handoff-2026-10-05.md)；电脑工具拒绝访问 iTerm2，不能声称终端消息已送达或 Claude 已确认接收。

## 来源与平台

唯一最终来源为 `var/ui-feedback-3-6-vs05-qa/release-source-final/`，591 个文件，inventory SHA256：

`bb88c81f209e3b6dbac1dba1b9a2f9f65ed5c2d69396bcb1c9dd30624cc07a49`

清单为同级 `release-source-final-manifest.json`。它包含用户/Claude/Codex 已授权的未提交改动；基准 commit `3be4d3b655eea1a142d60dba1b71c585a4da4908` 不能单独代表本次版本。原37来源及原失败现场保留；最终工程与正式产物均亲跑绑定 bb88，431 个不变实现/fixture/锁/合同的独立视觉证据另有逐字节前移证明。

已实际验证 macOS ARM64 原生正式 binary 和 Linux AMD64/musl 正式镜像。Linux 在 ARM64 VM 的 AMD64 仿真环境执行；不宣称物理 x86 或 Linux ARM64 已通过。正式构建仅 embedded-ui，没有 job-failpoints；QA fixture binary 不在交付包。

## 实际门禁结果

| 门禁 | 结果 | 原始证据目录（均在 `var/`） |
|---|---|---|
| 完整工程检查 | PASS：672 Rust/10 ignored；50 文件336前端；fmt/clippy/lint/typecheck/合同一致性 | `ui-feedback-3-6-vs05-qa/checks/release-final-check-summary.json` |
| 普通 Chrome 回归 | PASS：25 文件141/141，0失败/跳过/重跑 | `ui-feedback-3-6-vs05-qa/root-regression-final/` |
| 独立视觉与业务 | 12AC全部PASS，46个去重接受用例（45严格实现映射+1正式HTTPS Chrome）；72组AFTER、52组全站扫描；真实200%、键盘、减少动效、历史HTML哈希 | [QA6](vs05-qa-report.md)及 `ui-feedback-3-6-vs05-qa/final/` |
| Mac 正式发行 | PASS：两次独立重链接SHA一致；冷目录七步及全流程OS沙箱七步；真实EPERM/回环、无子进程、备份恢复 | `ui-feedback-3-6-vs05-qa/native-run-final-v3/` |
| Linux validation/dist | PASS：671 Rust/8 ignored及336前端；正式dist阶段默认pool亦336通过 | `ui-feedback-3-6-vs05-qa/linux-release-final-20261005/logs/` |
| Linux 正式冷启动 | PASS：静态musl binary、源/Node/Python-free镜像，network-none完整七步、重启与备份恢复 | `ui-feedback-3-6-vs05-qa/linux-release-final-20261005/cold-summary.json` |
| Docker 六阶段 | PASS：9份HTTP报告，661断言；恢复、加密配置、资产SHA和秘密清理 | `ui-feedback-3-6-vs05-qa/linux-release-final-20261005/docker-verification/summary.json` |
| 严格 HTTPS | PASS：216检查；HTTP origin/损坏PEM负例，严格TLS、Secure Cookie、CSRF/Origin/XFF限速、权限、重启资产SHA | `ui-feedback-3-6-vs05-qa/linux-release-final-20261005/production-https/summary.json` |
| 正式 HTTPS Chrome | PASS：1个独立用例；真实JS/CSS/PDF/GLB SHA、模型/部件/步骤/出处、375抽屉/Esc/0overflow | `final-production-browser/browser-summary.json` |
| 镜像归档导入、35文件包与独立审计 | PASS（包装修订2）：23blob/15层、35成员/34校验、导入引用、非root目标机探针和外层逐字节一致 | [独立交付审计](/Users/qsyj/Code/rust/everything-manual/var/final-delivery-audit/final-delivery-audit.md) |

Mac 与 Linux 的入口 JS/CSS 逐字节相同。拼接 gzip -9 为138,016 B，较改版原基线增加2,648 B，低于10 KiB门禁。Rust ignored 未计为通过，普通套件默认排除 `qa-pc*.spec.ts`，不声称141覆盖这些文件。独立45与正式HTTPS新1例分别记账，不把截图组合或重跑次数冒充业务用例。

所有运行使用隔离样例和本机 fixture，未修改用户正在使用的资料库。Docker 六阶段 `providerRequests=1` 为自有 TLS fixture，`fixtureOnly=true/paidRequests=0`；严格HTTPS及正式Chrome `providerRequests/paidRequests=0/0`。没有真实付费生成请求。

## 正式产物

| 平台 | binary SHA256 | 包及状态 |
|---|---|---|
| macOS ARM64 | `adcad48ac9bd4e95908569ffa2c400a928b0a856cc86f408ed52b3b0998f7a99` | [正式原生包](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/native-run-final-v3/everything-manual-macos-arm64-vs05.tar.gz)；7文件独立审计PASS |
| Linux AMD64/musl | `e260dccb690a234d05163fa241ba00fd123cbafc7fe05f1b24c5fdc68feadc6f` | [生产部署包·包装修订2](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/linux-release-final-20261005/revision-2/everything-manual-linux-amd64-production-vs05.tar.gz)；35文件独立审计PASS |

Mac包 SHA256：`31d16183c67c3ec43d4c56981d3542a47526ddd0916d4f0da1f09145feac8657`。

Linux最终包 54,908,769 B，SHA256：`47d38665d38ea06de0f3f46ff705744f86433e5538e9e42d2d40606c6aa13e64`；[校验文件](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/linux-release-final-20261005/revision-2/everything-manual-linux-amd64-production-vs05.tar.gz.sha256)。内层images.tar 43,482,624 B，SHA `39deede12dd808ea742123c705bc77a14bd09b3580679e8d0621b13b9455f6ee`；最终manifest SHA `7c95f5a7b548a3da112595818b9d7a5e5f0b78b35a6e91c96316f8871a76fb1b`，根SHA256SUMS SHA `9cec4bd1abd49ddc14271a2b0b10c46320a3e1f506561fa816c23aa6a47199bc`。

本机导入后实际 app 引用 `sha256:30824107d02d7cc1f3bf337eda42c9eb16ed32c9e18d183a46333d61cf2caa60`，proxy `sha256:0dcc88822d45581e65ae329f8be769762bf628d3b2bb7d2a077d4aa5c98b30e3`；归档、Config、层与实际无拉取执行均已核对。正式HTTPS/Chrome执行时的本机index引用为9719…2710，同一app平台manifest/binary；导入后地址模式变化已记录，不能把地址差异当重建或把未能执行的config ID冒充运行引用。

运行镜像不需要源码、Rust、Node或Python；随包Python HTTPS工具仅用于独立QA，明确不是部署依赖。解包先读包内 `DEPLOYMENT-PREREQUISITES.md`，完成目标机实际前置后按README初始化。

## 部署、升级和回滚

Linux 包包含正式 app/proxy 镜像、逐文件SHA、不可变引用清单、六份部署配置/工具及README、先决条件和公开验收证据。实际运行机需要 Linux AMD64 Docker Engine/CLI，协商 API 至少1.49，以及支持 `up --wait --wait-timeout` 的 Compose；本次验证为 CLI29.5.2/API1.54、Engine29.8.2/API1.56、Compose5.1.3。API条件依据 [Docker load](https://docs.docker.com/reference/cli/docker/image/load/) / [inspect](https://docs.docker.com/reference/cli/docker/image/inspect/) 文档及实际帮助/执行记录。

API版本是必要前置，不能据此保证任意镜像存储模式都接受同一 raw ID。本次运行与导入引用的实际验证范围为 Engine29.8.2/containerd `io.containerd.snapshotter.v1`。目标机导入后须按包内补充说明从两个归档tag的默认inspect取得该机不可变ID，核对平台及归档配置/层身份，实际执行 `--pull never --network none --read-only --cap-drop ALL` 的应用binary SHA与代理 `nginx -V` 探针（显式UID10001/101），通过才把验证引用填入 `deploy.env`；不匹配或不兼容应停止。不能退回可变tag运行或把其他存储模式写成本轮已验证。

部署管理员准备实际 HTTPS origin/证书完整链/PEM私钥、私有首次口令文件和32字节主密钥的64位十六进制文本，填 `deploy.env` 的 `EM_PUBLIC_ORIGIN`、`EM_ADMIN_PASSWORD_FILE`、`EM_MASTER_KEY_FILE`、`EM_TLS_CERT_FILE`、`EM_TLS_KEY_FILE` 及镜像引用，按照包内README校验、导入并启动。没有默认生产密码；公开T20测试常量只存在于冻结的QA脚本中，不得用于生产初始化。包不得带私有文件、数据库、blob、fixture或QA checker镜像。

[生产说明](../../../docker/production/README.md)记录停服/秘密清理、迁移前备份、恢复到新空目录、原provider配置与同一主密钥的保留、完整stack重新创建及从迁移前备份回滚。不能仅换回旧镜像撤销数据库迁移。没有提供真实域名/服务器/证书，本轮未执行公网部署；临时测试证书只在验证进程/新浏览器context使用，未改系统信任。

## 限制与保留记录

生产依赖 `npm audit --omit=dev` 为0告警；全量开发工具间接 `brace-expansion` 有1条high，属于构建工具的glob拒绝服务风险，保留原告警，不能称全量依赖零漏洞。正式运行包不携带Node工具链；本次构建仅处理冻结可信源。官方通告：[GHSA-qhr7-859c-m2p7](https://github.com/advisories/GHSA-qhr7-859c-m2p7)。

5秒识别用户研究尚未实施，未捏造用户数据。没有验证物理x86、Linux ARM64、Windows、Intel macOS、真实GPU/多浏览器性能、Mac签名公证或实际生产证书。Chrome使用SwiftShader覆盖功能与降级。旧136/3/2普通回归、334/336 Linux并发失败、镜像身份和tmpfs执行前置失败及HTTPS Chrome两次QA前置失败均保留；修订与最终完整复验可追溯，未删除用例、放宽业务断言或延长原门禁超时。

最终交付包审计与独立QA全部12AC签署后，才同步现有规划和state为完成。

上述签署已于2026-10-05 03:39:26（Asia/Shanghai）完成，[机器签署记录](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/signature.json)与QA6报告哈希一致，12项无缺失证据、无剩余门禁。[Linux最终验收报告](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/linux-release-final-20261005/revision-2/final-linux-acceptance.md)与[独立总审计](/Users/qsyj/Code/rust/everything-manual/var/final-delivery-audit/final-delivery-audit.md)均为PASS。自有QA栈、监听端口和浏览器context已清理，没有管理用户生产服务。根协调者依据实际签署勾选VS-05、清空pending/current并记录accepted与QA6历史。

包装修订1的35文件/镜像层/导入执行已通过，外层 `c499e12b…a0b80e` 也通过内容核对，但目标主机引用前置说明不足，保留为中间包。包装修订2精确六文件变化，仅部署前置、manifest、根清单及三份既有日志追加；其他29成员、最终源码、binary、前端和images.tar逐字节不变。目标机完整身份检查与两项显式非root探针实际通过；嵌套SHA清单漏项经自检修正后全部34项通过。修订2已通过最终独立复验；修订1不是最终交付。
