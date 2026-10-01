# api-settings QA 验收报告

结果：**PASS** · 回合：1 · request / PRD / UI 修订：**1 / 1 / 1** · 范围：**AS-01 全量，REQ-AS-001–008、AC-AS-001–015、UI-AS-001–006**。

日期：2026-09-22（Asia/Shanghai）。15 项必选 AC 均有独立实际验证依据；未关闭产品缺陷 **0**，未执行必选 AC **0**。此结论仅覆盖网页 API 配置，不改变 interaction-a、web-mvp T22/T23 或真实供应商验证状态。

## 环境与交付版本

- macOS 26.6.2 / 25G83，Apple Silicon；Node 26.0.0、Cargo 1.98.1；Playwright 使用 Google Chrome for Testing **148.0.7778.96**。[环境记录](../../../artifacts/api-settings/qa/environment.txt)。
- 验收未提交工作树；[生产源码 SHA-256](../../../artifacts/api-settings/qa/source-sha256-round1.txt)及[结束时冻结核对](../../../artifacts/api-settings/qa/source-freeze-check.txt)一致。QA 未修改生产代码、PRD 或 state。RD 实施记录作为输入，未以其自测结果代替验收。
- Rust 测试构建：`cargo build --offline -p everything-manual --features job-failpoints`；[构建日志](../../../artifacts/api-settings/qa/backend-build-round1.log)、[测试二进制 SHA-256](../../../artifacts/api-settings/qa/test-binary-sha256.txt)。此构建仅为本机模型下载夹具开放测试入口，不是正式发行包。
- 独立 Vite 端口 **15186**，每例独立随机端口 Rust 服务、本机供应商 A/B 与临时 data-dir；服务环境不继承 EM_* 部署覆盖。用户的 8080/5173 与预览数据未被使用或修改。
- 浏览器和 API 场景实际完成创建物品、上传样例 PDF/照片、准备、报价确认、建单及 worker 处理。只使用本地原创 fixture，无真实供应商请求或费用。A/B Authorization 与模型仅在内存比较，证据不保存密钥值。
- Rust 独立集成测试使用真实 SQLite 和完整 HTTP 路由中间件；终态残留场景用仓储/SQL 构造状态，不假装这些异常状态由浏览器自然产生。AS-QA-08 的延迟、字段错误、500 和首次读取错误为明确的浏览器响应注入；认证、文件失败、冲突与重启另有真实服务端验证。

## 实际执行与证据

| 检查 | QA 实际结果 | 证据 |
| --- | --- | --- |
| `cargo test --offline -p everything-manual --test api_settings_qa` | **7/7 PASS**，无 ignored | [Rust 原始输出](../../../artifacts/api-settings/qa/rust-round1.log)；[独立用例](../../../crates/server/tests/api_settings_qa.rs) |
| 独立 Playwright 完整 9 场景首跑 | **8 PASS / 1 测试等待超时**；详情见下文，不计产品缺陷 | [完整原始输出](../../../artifacts/api-settings/qa/e2e-round1.log) |
| 修正取消导航等待后仅复跑 AS-QA-02 | **1/1 PASS，4.9s**；9 个不同场景最终都有通过结果，未重复全量 | [定向输出](../../../artifacts/api-settings/qa/e2e-rerun-02.log)、[原生提醒结果](../../../artifacts/api-settings/qa/02-unsaved-native-dialog.json) |
| 设置表单与确认页组件回归 | **2 文件 / 12 测试 PASS** | [组件测试输出](../../../artifacts/api-settings/qa/frontend-unit-round1.log) |
| `cargo test --offline -p everything-manual --test generation_requests api_settings_` | **2/2 PASS**；独立亲自复跑已有代次/竞争测试，包含 worker 领取旧快照后转 needs_input、attempt 为 0 | [worker 与代次回归](../../../artifacts/api-settings/qa/worker-epoch-round1.log) |
| `npm --prefix apps/web run build` / `run lint` | 均 exit 0；build 含 TypeScript 检查，保留既有大分包提示 | [构建](../../../artifacts/api-settings/qa/frontend-build-round1.log)、[Lint](../../../artifacts/api-settings/qa/frontend-lint-round1.log) |
| `CARGO_NET_OFFLINE=true cargo xtask contracts --check` | PASS；OpenAPI / 生成 TS 与 Rust DTO 一致 | [合同检查](../../../artifacts/api-settings/qa/contracts-round1.log) |
| 最终 QA 配置的用例发现 / 定向 ESLint | 9 场景可发现，Lint exit 0；仅取证配置变化，未重跑生产行为 | [发现输出](../../../artifacts/api-settings/qa/final-test-discovery.log) |

浏览器命令在 `apps/web`：`npx playwright test --config playwright.api-settings-qa.config.ts`；定向追加 `-g AS-QA-02 --output ../../artifacts/api-settings/qa/playwright-rerun-02`。专用配置不运行历史 globalSetup，未覆盖 web-mvp 或 interaction-a 的截图/日志。

## AC 验收矩阵

下表 AS-QA 编号对应 [独立浏览器用例](../../../apps/web/tests/e2e/api-settings-qa.spec.ts)；Rust 用例对应上表 7/7 输出。全部判断基于本轮实际执行及所列静态合同核对。

| AC ID | 期望与实际可观察结果 | 结果 | 证据 |
| --- | --- | --- | --- |
| AC-AS-001 | 运行/保存地址、模型、来源与密钥存在状态可读；输入不回显密钥；读取失败没有可保存的猜测字段，重试恢复 | **PASS** | AS-QA-01/08；Rust `http_auth…`、`legacy_deployment…`；375/1440 截图 |
| AC-AS-002 | 真实 HTTP 匿名 GET/PUT 401，缺失/错误 CSRF、错误 Origin 403；拒绝不改配置；GET no-store；响应不包含密钥、秘密来源路径或旧部署 URL 的 userinfo | **PASS** | AS-QA-04；Rust `http_auth…`、`legacy_deployment…`、`validation_matrix…` |
| AC-AS-003 | 浏览器整体保存两家并产生新修订；刷新保留；读取/保存期间 jobs、quotes、attempts、ledger 数量不变，A/B 请求均为 0 | **PASS** | AS-QA-03/04；[重启链路数据](../../../artifacts/api-settings/qa/03-restart-worker.json) |
| AC-AS-004 | userinfo/query/fragment、非 loopback HTTP、非法协议、控制字符、长度及非法/空密钥均拒绝；422 不回显输入，修订/文件/运行值不变；合法本机地址可保存 | **PASS** | Rust `validation_matrix…` 参数化；AS-QA-04/09；结构枚举错误亦验证安全固定消息 |
| AC-AS-005 | 保存中按钮/字段/恢复冻结，强制重复 submit 仍 1 请求；422 焦点与关联错误、500 编辑保留；真实目录冲突写失败无半更新；真实旧修订 409 不覆盖；显式确认重读才丢弃编辑 | **PASS** | AS-QA-04/08；Rust `write_failure…`；[重读后截图](../../../artifacts/api-settings/qa/08-conflict-explicit-reload.png) |
| AC-AS-006 | keep 保留密钥/来源；replace 重载后内存相等；clear 重启后无密钥且不回退；空替换拒绝；切换模式及成功后清空新密钥 | **PASS** | Rust `three_key_actions…`；AS-QA-03/05/09 |
| AC-AS-007 | 明确环境模型覆盖 TOML；网页 B 值覆盖环境；清除不回退；只恢复 Tripo 后它采用部署值，说明书 AI 网页覆盖保持；无网页文件启动使用原部署值 | **PASS** | AS-QA-01/05/09；Rust `three_key_actions…`；来源为“部署配置（含默认值）”，未按相同值猜测 default |
| AC-AS-008 | 保存后 active 不变、saved 更新、pending=true；真实停止/同目录重启后 active=saved、pending=false；私有文件 0600；破坏文件后真实服务拒绝启动，未静默回退 | **PASS** | AS-QA-03/05；Rust `three_key_actions…`；[待重启](../../../artifacts/api-settings/qa/03-saved-pending.png)、[重启生效](../../../artifacts/api-settings/qa/03-restart-effective.png) |
| AC-AS-009 | 网页写入 B 后重启并显式确认测试任务：B 接到 Tripo 4 次请求、说明书 AI 1 次，两个模型与 Authorization 匹配；A 接到 0 次；任务 succeeded；本机模拟付费提交仅 1 次 | **PASS** | AS-QA-03；[匹配布尔值与计数](../../../artifacts/api-settings/qa/03-restart-worker.json) |
| AC-AS-010 | queued/running/waiting_provider/retry_wait/needs_input/submission_unknown 全部阻止变更；cancelled 遗留 intent/submitting/unknown 及仍执行的供应商 stage 也阻止；无变化提交不触发切换，不代取消任务 | **PASS** | Rust `all_nonterminal_jobs…`；AS-QA-07；独立复跑代次回归中的活动任务拒绝 |
| AC-AS-011 | 保存与真实建单竞争仅一方受理（实测保存 200、建单 422）；保存与 retry 竞争亦只一方受理；pending 拒绝新报价/确认/建单/恢复，资料与健康仍可读 | **PASS** | AS-QA-06/07；Rust `save_retry_race…`；[并发结果](../../../artifacts/api-settings/qa/07-create-save-race.json) |
| AC-AS-012 | 已确认但未消费旧报价重启后 422 changed；旧 failed retry 在恢复原值后仍 changed 并要求重新报价；已受理建单同键重放返回原 ID、业务数量/调用不增；worker 旧快照不执行供应商动作 | **PASS** | AS-QA-03/06；Rust `save_retry_race…`；2/2 worker/代次回归 |
| AC-AS-013 | 站内离开单一对话框，继续编辑默认焦点，Esc/取消保留模型与新密钥；真实 beforeunload 取消后页面仍可编辑，接受刷新后丢弃；重入不恢复密钥；URL/localStorage/sessionStorage/DOM attributes 无密钥；仅 pending 离开不误报 | **PASS** | AS-QA-02 定向通过、AS-QA-09；[原生提醒结果](../../../artifacts/api-settings/qa/02-unsaved-native-dialog.json) |
| AC-AS-014 | 375/1440 宽度等于文档滚动宽度；输入 DOM 字号≥14，labels 可关联；主保存/恢复按钮≥44×44、radio 标签高≥44；Tab 地址→模型、字段错误焦点、手机清除/恢复/保存均实际可操作；CSS 核对 labels=14、辅助=12 | **PASS** | AS-QA-01/08/09；[布局测量](../../../artifacts/api-settings/qa/01-layout.json)、两尺寸图和手机操作图 |
| AC-AS-015 | 配置完整只表述基础条件齐备/仍需报价校验；pending 表述生成暂停；目录外模型允许保存但真实报价 409 PRICE_CATALOG_MISSING；B 停止后健康/资料可读、页面连接未验证；缺密钥/模型不冒充可生成 | **PASS** | AS-QA-05/06；[目录外模型且供应商停止](../../../artifacts/api-settings/qa/06-unknown-model-offline-readiness.png) |

## 视觉核验

QA 已逐张打开查看本轮全部 7 张 PNG：`01-settings-375`、`01-settings-1440`、`03-saved-pending`、`03-restart-effective`、`06-unknown-model-offline-readiness`、`08-conflict-explicit-reload`、`09-mobile-clear-impact`。

- 两家卡片顺序、运行值与下次启动值、来源类别、清除影响、重启指引均可辨识；桌面字段并排，375px 单列，无正文横向裁切。焦点轮廓清晰；手机按钮已真实点击完成保存与恢复。
- [375px 首屏与整页](../../../artifacts/api-settings/qa/01-settings-375.png)、[1440px](../../../artifacts/api-settings/qa/01-settings-1440.png)、[375px 清除影响](../../../artifacts/api-settings/qa/09-mobile-clear-impact.png)。清除影响截图同时保留前一次空替换校验摘要；后续提交重新校验并成功，截图不是“清除保存失败”的证据。
- fullPage 截图中的固定侧栏/底栏随拍摄时滚动位置出现在长图中段；不能把该位置当成静态文档布局。可操作性依据 DOM 测量、键盘和实际点击，不只依据长截图。

## 缺陷与首轮测试问题

本轮无新增 `BUG-AS-*` 产品缺陷，未关闭验收缺陷为 **0**。

**AS-QA-02 首跑超时（测试侧，已修正）**：原生 beforeunload 已触发并被 dismiss，但测试 `await page.reload()` 等待一个本来已取消的导航完成，直到整例 120 秒超时，后续输入断言才收到“page/context/browser closed”。将该导航等待单独限制为 3 秒，保留并执行页面存活、模型/密钥未丢失及后续丢弃断言；额外实际验证接受原生刷新会清空编辑。定向 1/1 通过。未改产品、未放宽业务验收；首跑 8/9 原始日志保留。

冻结前发现的查询列名和旧“生成能力可用”文案已由 RD 修正；本轮通过实际旧 retry/worker 与目录外模型浏览器场景验证最终代码，不以静态修复声明代替结果。

## 非代码知识与限制

- 禁用 trace/HAR/视频/自动截图仍不足以排除 Playwright 的失败 `error-context.md`：本轮首跑超时时其 accessibility snapshot 记录了当次测试假密钥。QA 审计发现唯一 1 处，立即替换为 `[REDACTED_QA_SECRET]`，未把原值写入报告或工具输出；[脱敏记录](../../../artifacts/api-settings/qa/evidence-redaction.json)。命令原始输出未改，只有该诊断上下文已脱敏。
- 已按本机安装的 Playwright `lib/index.js::_takePageSnapshot` 确认，在独立 QA 配置设置 `PLAYWRIGHT_NO_COPY_PROMPT=1` 可阻止后续失败自动落该快照。最终配置已做发现/Lint 检查；此取证开关不改变被测产品行为。后续升级 Playwright 应重新核对该环境变量入口。
- 浏览器测试随机假密钥只保存在内存和隔离服务器的临时私有配置；日志扫描只产出布尔结果。最终 artifacts 扫描假密钥模式命中 **0**，trace/HAR 文件 **0**。没有保留临时 data-dir、数据库、密钥文件或原始服务器日志。[收尾审计](../../../artifacts/api-settings/qa/shutdown-and-evidence-audit.json)。
- 本次复验不是全量历史 E2E、单二进制发行矩阵、备份/迁移全链或真实收费 Provider 认证；未验证任意兼容 API、连通性、余额、模型真实性或真实生成质量。旧网页无密钥文件行为、环境优先、JSON 私有文件重启、失败关闭均在本批范围内实际验证。
- 站内链接、Esc、原生刷新取消/接受已实际验证；关闭窗口使用同一 beforeunload 生命周期，不以此宣称已覆盖所有浏览器/操作系统退出方式或全站无障碍认证。

## 回合历史与交接

- **回合 1 / 2026-09-22 / full / PASS**：独立 Rust 7/7、浏览器不同场景 9/9 最终有通过依据（首跑 8/9 + 1 个测试等待问题定向 1/1）、组件 12/12、额外 worker/代次回归 2/2、build/typecheck/lint/contracts 通过。15 AC PASS，0 必选未执行，0 未关闭产品缺陷。
- 测试服务已停止：15186 无监听，所有 `em-api-settings-qa-*` 临时目录已清理；本机供应商夹具和每例 Rust 子进程由 afterEach 停止。未操作用户 8080/5173。无待清理的历史 serverlog。
- 下一步交协调者：依据本报告更新 `state.yaml` 与用户交付说明。QA 不替协调者更新状态。
- 本轮 QA 更新 llmdoc：**`llmdoc/requirements/api-settings/qa-report.md`**。新增非代码知识为取消原生导航的测试等待语义、Playwright 失败快照取证风险及精确验证边界；未修改其他工作目录的验收结论。
