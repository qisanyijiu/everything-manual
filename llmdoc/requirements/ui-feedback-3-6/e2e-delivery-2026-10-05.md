# 完整 E2E 与交付复验 · QA7

2026-10-05 用户新增要求：完成 E2E、修复发现的问题并保证可交付。**QA7 full 验收 PASS，本轮发现的问题已闭环，新版发行包可按部署前提交付。** 该记录和新发行包接续 QA6；[此前交付记录](production-delivery-2026-10-05.md)及其 bb88 来源包保留为历史。视觉 PRD/UI 修订 1 不变，社区/激励仍按用户决定取消本轮实施。

## 修复与复验结论

本轮唯一生产代码修复是阅读器复核操作被共享样式覆盖为 40px，恢复要求的 44px 触控高度。PC02B 原用例及普通阅读器回归通过，新 Mac/Linux 正式产物包含该修复。

其他失败经复现归属于测试方法、夹具或运行工具，分别修正并保留原失败：专项配置恢复用例发现；热点重绑定按实际 canvas 投影点击；聚合计数与已授权完成监听器分别记账；背景任务响应由真实关闭页面事件释放；原生 200% 缩放和 Firefox GC 改用实际引擎接口；模型身份等待实际资产匹配；切宽等待已声明布局状态。PC04 授权夹具从 350ms 调为 5 秒、响应延迟 6 秒，真实证明请求发生于到期前、返回发生于到期后；原业务断言保留。性能夹具修正为精确 100000 面及一致正面绕序，两个既有小模型字节未改。完整最终 lint/typecheck 通过。

Edge 的完整运行原始结果是 140 PASS / 1 FAIL：1024px 切宽后媒体查询已经改变，DOM 仍短暂保留旧宽布局，旧固定 100ms 等待导致失败。改为等待实际布局状态后，原 ROW-1/ROW-2 在 Chrome、Edge、Firefox 共 6/6 通过。最终各引擎 141 个普通用例均有当前接受证据；**没有声称 Edge 再跑过一份完整 141 全绿报告**。其他测试方法改动也由原受影响用例补验，未提高产品阈值、删除失败或把重跑计为新用例。

正式 HTTPS 的首次 Firefox 检查在版本门禁即失败（实际 155 不能写成 156）；第二次真实 156 登录成功后，测试清空已卸载密码框发生竞态。最终捕获同一真实控件，只接受实际脱离 DOM 的清理情况，保留登录及全部后续业务断言。新三引擎完整重跑通过；官方 157 先前完整通过的流程、输入、来源、服务和方法未变，按逐文件 SHA 明确接受原证据。

## 已执行门禁

| 范围 | 结果与实际边界 | 证据 |
|---|---|---|
| 普通 E2E | Chrome154.0.8037.93 完整 141/141；0 跳过/重试/错误 | [原始结果](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/ordinary-final/results.json) |
| PC01–04 | 42/42 | [专项报告](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/pc01-04/README.md) |
| PC05–06 | 25 浏览器用例 + 官方 Firefox157 原生窗口恢复 1；26/26 | [专项报告](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/pc05-06/final-report.md) |
| 不同产品 E2E 用例 | **209/209**；当前发现 41 spec 文件，独立按文件/标题去重；兼容性及重跑不增加数量 | [独立覆盖核对](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/platform-gates/final-delivery-audit/e2e-209-independent-report.json) |
| 三引擎兼容性 | 每引擎 141 当前接受；Firefox156.0 完整 141/141；Edge154.0.4258.53 原 140/1 + 受影响原用例复验；Chrome 补验新方法 | [矩阵账本](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/browser-matrix/matrix-coverage-ledger.json) |
| Linux Rust / CLI | 默认 671/0/8 ignored；另显式执行 PC04 六项 6/6，合计 677 不同通过；两项既有可选演练/文档仍 ignored | [默认与 BUG013](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/platform-gates/linux-independent-report.json)、[CLI 六项](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/platform-gates/pc04-linux-independent-report.json) |
| 工程门禁 | 新 Linux validation：671 Rust、336 前端、fmt/clippy/lint/typecheck/合同全部通过；正式 dist 默认 pool 亦 336/336；最后 QA 文件修订后 lint/typecheck exit0 | [构建记录](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/linux-source/var/release/logs/linux-validation-build.log)、[最终 lint](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/browser-matrix/lint-row-final.log)、[最终 typecheck](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/browser-matrix/typecheck-row-final.log) |
| Mac 正式发行 | 两次独立链接一致；完整七步冷启动与 OS 断网沙箱下完整七步均通过，7 文件归档核对 | [执行状态](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/native-run-final/status.json) |
| Linux 正式运行 | 静态 musl、正式 embedded-ui、无 failpoints/源码/Node/Python；network-none、非 root、只读完整七步通过 | [冷启动记录](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/linux-source/var/release/cold-summary.json) |
| Docker / 严格 HTTPS | 六阶段 9 HTTP 报告、661 检查；严格 TLS 216 检查全部通过 | [Docker](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/linux-source/var/release/docker-verification/summary.json)、[HTTPS](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/linux-source/var/release/production-https/summary.json) |
| 正式 HTTPS 浏览器 | Chrome154、Edge154、PW Firefox156、官方 Firefox157 全部完整流程通过；实际登录/冻结 GLB 与 PDF 字节/模型/部件/步骤/出处墨迹与焦点/375 抽屉/退出；4 项单独记账 | [正式浏览器报告](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/production-browser/final-report.md) |
| 正式包 GPU 300 秒 | Apple M1 真 Metal GPU、Chrome154、100000 面、持续 300.034 秒，帧间隔 p95 18.5ms≤33；150 实际元数据 API p95 6.941ms≤200；0 外呼/页面错误/事实变化 | [性能独立报告](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/platform-gates/final-delivery-audit/gpu-independent-report.md) |
| 部署包与归档 | 35 成员 / 34 SHA；实际 save/load、Config/RootFS 身份和 UID10001/101 离线探针通过；外层解压逐字节一致 | [归档核对](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/linux-source/var/release/revision-2/outer-archive-verification.json) |

所有真实付费调用为 0。Docker 的一次 provider 请求是自有 TLS fixture，严格 HTTPS/正式浏览器为 0；真实供应商 T23 未执行。自有浏览器、端口及 TLS 栈已清理，未管理用户预览或真实资料。

BUG013 的 ADR-037 既定测试侧 300ms settle 已在 Linux 精确复验 3/3，通过结果补齐此前缺失证据；**没有修改或宣称消除生产启动期注册信号的窄窗口**，不把该三次复跑增加到 677 的不同用例数。原 Linux PC04 五过一败、第一次六过但产物绑定失败、各浏览器/工具中间失败均保留。

## 当前来源与正式产物

规范来源为 [592 文件清单](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/final-source-r2-manifest.json)，inventory SHA256 `73e224e76dc8e0b59c47de966c0ed1863d60ea8bcaee3d0b522a84ec1bcdecb1`；基准 commit `4a8b22fdeffa8737b02aa2eb9a421dc5c003b76e` 本身不代表含授权未提交修复的交付版本。

Mac 编译来源 `54ab89fd81f395c7f55747235ab6f8a450443f1d13a51287c9e5a0c4606e31d9`，Linux 编译来源 `a124e25c1a3cb5954067c0ad3996a85438245caf19c66bcd17e2d71d6575cdca`。相对规范来源分别只有 11/9 个测试文件差异，生产运行代码差异为 0；全源逐字节前移与受影响测试证据见 [来源映射](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/runtime-source-forward-map.json)，独立审核不把测试修订写成全源零差异。Mac/Linux 入口 JS/CSS 逐字节一致，HTTPS 实际请求也已核对这些字节。

视觉 12AC 接续另有[独立视觉来源审核](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/platform-gates/final-delivery-audit/visual-forward-independent.json)：旧 431 实现/fixture/锁/合同文件实际 429 个不变，差异精确为新增 44px 样式和仅测试侧授权夹具。触控影响由本轮原用例复验关闭；旧 72 组截图、52 组装饰扫描没有重新执行，不能写成本轮新跑。新入口按冻结的先拼接主JS再CSS、gzip -9（mtime=0）口径实际 138,066 B，相对原基线 135,368 B 增长 2,698 B≤10,240 B，当前体积门禁通过。

| 平台 | 交付包 | binary SHA256 | 包 SHA256 |
|---|---|---|---|
| macOS ARM64 | [新版原生包](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/native-run-final/everything-manual-macos-arm64-vs05.tar.gz) | `88fa0cfb9665b23f1dddb5b825b6308b8a02dcb1ffc14a786acd8c0054c756d4` | `d3217375780a41c1b28db8ede994732b6bb5f5b2a4778dd48b9cf089d1d3b45c` |
| Linux AMD64/musl | [新版生产部署包·修订2](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/linux-source/var/release/revision-2/everything-manual-linux-amd64-production-vs05.tar.gz) | `c04e7f2b4b6671caeab17e542706c6397213517f2f31cb67b6180469ff60e0e9` | `2e479cd7651c327d2d8750d61e5c09dd854791fffe6c67b3de906a3fdd9e4fe2` |

Linux 包 54,914,547 B，[SHA 校验文件](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/linux-source/var/release/revision-2/everything-manual-linux-amd64-production-vs05.tar.gz.sha256)；images.tar SHA `35c1c8610fce640ee4c108932e6c8df2b2ffb9377f21209656fa9e210d5fcacb`。正式 HTTPS 构建索引引用 df1e…465e 对应平台 6c4a…fda2；归档导入后默认可运行应用引用为 `sha256:6c4a53da9288ee3df07faa8c3620bd4c9ea5e906c70e16b0502080e06777fda2`，代理 `sha256:0dcc88822d45581e65ae329f8be769762bf628d3b2bb7d2a077d4aa5c98b30e3`，实际二进制与配置/层一致，地址模式差异不是重建。

解包先读 `DEPLOYMENT-PREREQUISITES.md` 并执行完整目标机导入门禁，再按包内 README 准备实际 HTTPS origin/证书、私有首次口令和主密钥，填写该机验证后的不可变镜像引用并启动。目标机前置已在 Engine29.8.2/containerd、CLI29.5.2、Compose5.1.3 实际执行；不能把 API 版本或本机 raw ID 当任意 store 的通用通过证明。运行无需源码、Rust、Node、Python或联网拉取；包不含私钥、数据库、真实/夹具资料或 QA checker 镜像。升级/迁移先备份，回滚按包内 README 恢复迁移前备份。

## 验收边界和接续

Linux AMD64 ABI 在 Apple Silicon VM/Rosetta 执行，没有物理 x86 性能、Linux ARM64、Windows或 Intel macOS 证明；Mac 包未签名/公证。完整 Firefox 矩阵用 Playwright patched156，官方157的原生恢复和正式 HTTPS 全流程分别验收；未声称 Safari 覆盖。GPU 测量为实际渲染帧间隔，纹理为16×16，不是 GPU kernel 耗时或4K纹理压力验证，也不是其他设备/浏览器的性能保证。5 秒识别用户研究仍未实施。

依赖锁未修改，此前记录的开发工具间接 brace-expansion high 告警仍保留；生产依赖 audit 0 告警属于既有检查，没有伪称本轮重新取得当前漏洞库结果。本轮没有真实付费生成或公网部署，管理员仍需提供实际域名、证书与私有材料。

原 QA6 签署、失败现场与中间包保留。独立门禁全部通过后，E2E-01 已勾选并追加 QA7 full 记录；后续源变更必须重新冻结并验受影响门禁，社区任务不得自动恢复。

独立总验收：[报告](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/platform-gates/final-delivery-audit/final-delivery-audit.md)、[机器签署](/Users/qsyj/Code/rust/everything-manual/var/e2e-delivery-20261005/platform-gates/final-delivery-audit/signature.json)。签署核对最终文档、原始报告、当前来源及发行包 SHA；历史 QA6 签署字节保留。
