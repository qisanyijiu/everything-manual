# VS-05 实现与发行收口

日期：2026-10-05。负责人：Codex当前会话。依据视觉PRD/UI修订1；VS-03由Claude独立QA3接受，VS-04经QA4 FAIL、移动尺寸返工、QA5 PASS接受；VS-05已由独立QA6签署全部12AC PASS。整体验收见[QA6](vs05-qa-report.md)，正式产物/部署前提见[生产交付](production-delivery-2026-10-05.md)。现有规划和state已标完成，源码冻结bb88。

## 展示修复

- `theme.css` 正文行高使用冻结的 22px token，消除应用 22.4px 与独立 HTML 22px 的差异。
- `standalone/styles.ts` 新导出标题使用 28/36px，<768px 使用 24/32px；正文复用 14/22px token，部件主按钮至少 44×44。没有改动已有 HTML 文件或发布资产。
- `ReleaseReaderPage.tsx` 点选部件统一提示“已选择部件…及关联热点”；不论 WebGL 是否可用都如实说明选择，不再暗示模型已定位。模型/列表联动、热点/步骤、出处与用户并行新增分件能力保留。
- 人工图审发现 `JobStageList.tsx` 原“执行顺序”说明与实际阶段记录不符。登记 BUG-VS05-001，改为编号便于定位、不代表执行先后；保留后端记录顺序，没有新增排序或串行执行规则。
- N-VS03 的中性离线热点牌保留：冻结组件说明允许软底可选，牌上真实热点数量/无热点文字持续可辨。断点抽屉 Esc 复验先等待可聚焦关闭控件，避免在 React 断点重挂前发送不具代表性的瞬时事件。

## 工程与生产配置修复

发布审查发现 `xtask smoke` 步骤 5 原先仅写出 profile，仍直接启动 binary，历史“沙箱断网”日志不足以证明实际限制。已通过 `Service::start_sandboxed` 真正执行 `/usr/bin/sandbox-exec -f profile binary serve`，其余进程/环境清空/清理机制沿用统一入口。新增 macOS 真 OS 回归，确认本机监听可连接，文档保留地址被 EPERM 拒绝；实际执行 1/1 PASS。Linux 离线证据仍由外层 `--network none` 提供。

`.dockerignore` 在源码允许清单之后排除所有 `var/`、`artifacts/`，避免 QA JSON/截图进入中间构建上下文。新增 `docker/production/` 七文件：image-only Compose、强制 HTTPS/Secure Cookie 的应用配置、相邻 TLS 代理、降权秘密准备器、环境模板、部署/备份/恢复/回滚说明、独立 HTTPS 验证器。配置要求实际 origin 和操作者私有证书；运行不需要源码/Node/Python。新增文件已静态复核，实际运行另记发行报告。

## 唯一有效来源

最终冻结来源为 `var/ui-feedback-3-6-vs05-qa/release-source-final/`；591 个产品/构建/部署文件，inventory SHA256：

`bb88c81f209e3b6dbac1dba1b9a2f9f65ed5c2d69396bcb1c9dd30624cc07a49`

逐文件清单为同级 `release-source-final-manifest.json`。这是包含用户/Claude/Codex 已授权未提交改动的来源快照，Git 基准 commit 单独记录，不能把基准 commit 当成最终发行版本。LLMD/证据报告不作为产品源，后续可追加真实结果。前一冻结37d4236及早期 `final-product-source*` 保留为中间证据，不能把其检查结论写成新源亲跑结果。

相对37仅五文件变化：三个普通E2E修订关闭通知测量的viewport前置、阅读器精确提示、切宽后的真实React断点等待；正式Dockerfile validation显式设置一个Vitest worker；smoke正确识别pgrep空stdout/stderr且exit1为没有子进程，发现实际children或工具执行错误时报错。未改变超时、业务断言、UI/后端运行实现、合同、fixture或锁。五文件delta在 `checks/final-source-delta.json`；独立QA将431不变实现/fixture/锁/合同精确映射，原45项/72截图/52装饰证据不覆盖。

## 已实际执行的检查

- 最终 bb88 来源 `VITEST_MAX_WORKERS=1 cargo xtask check` 全通过：672 Rust 测试、10 ignored；50 文件/336 前端单测，fmt/clippy/lint/typecheck/合同一致性全部通过。日志和摘要在 `checks/release-final-check.log`、`checks/release-final-check-summary.json`，主源 591 文件无漂移。
- 最终来源前端正式构建 exit 0；主入口 JS+CSS 拼接 gzip -9 为 **138,016 B**，原改版前为 135,368 B，增量 **2,648 B**，低于 10 KiB 上限。实际文件/产物 SHA 在 `native-run-final-v3/frontend-build-summary.json`。未新增 UI/字体依赖。
- 最终来源 macOS Apple Silicon 正式发行在独立源、缓存与依赖目录完成；两次独立重链接 binary SHA 相同，完整七步及额外全流程真实断网沙箱通过。实际日志两次确认无子进程，不含该检查的跳过。详细原始报告在 `native-run-final-v3/native-verification-report.md`；v2 保留为旧来源证据。
- 最终来源普通浏览器全回归 **141/141 PASS、25 文件、0 失败/跳过/重跑**；默认排除 `qa-pc*.spec.ts`，不会把它们算作已执行。结果在 `root-regression-final/summary.json`；旧来源的 136 PASS、3 FAIL、2 未执行及逐项诊断仍保留。关闭三项 QA 前置/期望后完整重跑，未改变业务断言或超时。
- 最终来源 Linux AMD64/musl validation 全通过：671 Rust 测试、8 ignored、50 文件/336 前端单测及 fmt/clippy/lint/typecheck/合同一致性；正式 dist 构建阶段未设置 worker 环境变量，也实际完成 336/336。静态正式 binary 的运行镜像、`--network none` 完整七步、Docker 六阶段的 9 份 HTTP 报告/661 断言均通过。
- 实际生产HTTPS配置完成 **216项检查**：HTTP origin与损坏证书负例、严格TLS、Secure Cookie、CSRF/Origin、代理限速、容器降权/只读、秘密权限和重启资产SHA。独立正式HTTPS Chrome新增1例实际通过，真实资源SHA、模型/PDF/出处/375抽屉均通过；镜像保存加载及最终包装修订2独立审计PASS。
- 最终Linux包装修订2为35成员/34SHA、23镜像blob/15层，与源/binary/镜像身份一致；外层SHA **47d38665d38ea06de0f3f46ff705744f86433e5538e9e42d2d40606c6aa13e64**。最后包装六文件变化补充目标服务器默认inspect/完整Config/层/非root无网络实际执行前置，纠正根清单需覆盖嵌套SHA；产品实现和运行镜像未改。实际CLI/Engine/containerd范围、目标机操作及旧包装/失败保留见生产交付报告。

## 准确记录的边界

`npm audit` 当日生产依赖为 0 条告警；全量开发工具间接 `brace-expansion` 有 1 条 high，涉及 lint/合同生成工具链。它不在正式运行包的 Node 依赖路径中；不可宣称全量依赖无漏洞，具体版本/路径/官方通告另记发行报告。默认构建只处理冻结可信源。

前一 smoke 子进程复核把 `pgrep` 的 no-matches exit 1 写成“不可用/跳过”，不据旧日志声称已验证；v2另以 `ps` 三次采样确认无子进程。最终 bb88 检查器已修正识别，实际 children 会使 smoke 失败；Mac v3 正式 smoke 已验证该分支，不改变运行产品。Docker 六阶段只有 1 次自有 TLS fixture 请求（providerRequests=1、fixtureOnly=true、paidRequests=0）；生产 HTTPS 的 providerRequests/paidRequests 均为 0，不能将这两份报告合写为所有 provider 请求为 0。

没有真实 GPU/多浏览器性能、物理 x86、Linux ARM64、Windows/Intel macOS、公证签名或实际生产域名证据时不宣称通过；本轮未触发真实付费生成。最终平台/门禁结果以发行报告为准。
