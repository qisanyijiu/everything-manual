# encrypted-secrets QA 验收报告

**结果：PASS · round 2 full · PRD/UI revision 2/2 · ES-01 全部 12 AC · 2026-10-02**

12 AC 全部通过，未关闭缺陷 0。独立 Rust ES 11/11、AS 7/7、真实专用 Keychain 跨进程 1/1；最终浏览器 AS 9 + ES 9 = **18/18 PASS（45.8 秒）**。没有未执行的本批必选 AC。测试服务已退出，15187 无监听，测试 binary 已交还协调者。

## 环境与证据

macOS 26.6.2 arm64、Node 26.0.0、Cargo 1.98.1、Playwright Chromium。每例独立临时 data-dir、随机主密钥与 API canary，仅 localhost 双供应商 fixture，无真实收费调用。QA 未操作用户 8080/5173 预览或数据。正式源码指纹见 [round2 清单](../../../artifacts/encrypted-secrets/qa/source-sha256-round2.txt)；与 round1 相比仅 secret helper 和两家 client 改变，核心加密、Keychain、迁移与文件代码未变，沿用其已执行的 Rust/native 证据。最终扫描确认 round2 源码无后续变化。

以下证据路径均相对 `artifacts/encrypted-secrets/qa/`；[证据目录](../../../artifacts/encrypted-secrets/qa/) 包含完整原始 runner 输出。`rust-round1.log`、`native-round1.log`、`e2e-round2.log` 为实际独立执行；`typecheck-round2.log`、`qa-lint-round2.log`、`backend-build-round2.log` 均通过。

## 12 AC 验收矩阵

| AC | 结果 | 独立证据与实际结果 |
| --- | --- | --- |
| AC-ES-001 | PASS | Rust es_qa_01 + `es01-web-encryption.json`：AEAD 用途认证、随机 nonce、双 provider 密文可重启读取，0600，无明文/原始主钥。 |
| AC-ES-002 | PASS | Rust es_qa_02：nonce/ciphertext/tag/版本/算法/用途篡改、错 key、截断和用途交换拒绝，不回退文本。 |
| AC-ES-003 | PASS | `native-round1.log`：真实唯一 Keychain 条目，两子进程创建/读取，同秘密解密成功；plaintext/master leak=false，精确 cleanup=true。 |
| AC-ES-004 | PASS | Rust es_qa_04、es_qa_07 CLI：严格环境格式、有效主钥往返、缺失/拒绝来源、读取不创建替代主钥；无持久秘密路径可用。系统不可用通过 DI 注入。 |
| AC-ES-005 | PASS | Rust es_qa_05 + `es05-legacy-process-migration.json`：代表 keep/replace/clear/restore 迁移、revision/source/pending 保持；真实 serve 迁移后旧报价仍可消费，两家 auth/model=true。 |
| AC-ES-006 | PASS | Rust es_qa_06/es_qa_10 + ES-QA-05：check 只读并给 serve 指引；提交前失败原字节不变；提交后目录 fsync 故障保留完整密文/一致修订并安全告警。 |
| AC-ES-007 | PASS | Rust es_qa_07 + `es03-cli-worker-reflection-backup-export.json`：两家 CLI 转换、源不变、0600、不覆盖、改文件引用后真实 worker 消费成功。 |
| AC-ES-008 | PASS | Rust es_qa_08/es_qa_10 + `es02-failure-recovery.json`：符号链接/权限/超限/非普通文件/无 key/输出失败拒绝；实际保存失败保留编辑与 revision，显式重试成功。 |
| AC-ES-009 | PASS | Rust es_qa_09 + ES-QA-01：环境 keep 不复制 key 至 overlay 或应用文件；页面准确区分环境内存注入和网页密文保存。 |
| AC-ES-010 | PASS | AS Rust 7/7 + 浏览器 AS 9/9；`as-regression/03-restart-worker.json`、`07-create-save-race.json`：三态/恢复/CAS/旧报价及任务门禁保持；B 真重启 auth/model=true，幂等未重购；fsync 前后状态一致。 |
| AC-ES-011 | PASS | Rust URL userinfo 安全拒绝；ES03/04/06/07 真反射、DB/blob/日志/HTTP/browser/Git/backup/export 扫描无泄漏；`final-evidence-audit.json`：私有文件忽略规则均匹配、tracked 私有文件 0、QA 文本随机 canary 0。BUG-ES-001 复验关闭。 |
| AC-ES-012 | PASS | ES01/02 + AS01/02/08/09：375/1440 无横溢出、键盘标签/焦点、失败编辑和重启反馈；10 张截图逐张检查，密码遮挡。审阅 `docs/api-settings.md`：主钥分离、CLI/迁移、丢钥、历史副本与静态加密边界准确。 |

## 缺陷

### BUG-ES-001 · 损坏 JSON 内转义密钥反射可进入错误或诊断内容

- 严重度／状态：**P1 / CLOSED（QA round 2 独立复验，2026-10-02）**；对应 AC-ES-011。
- 首轮复现：本机 Tripo HTTP 400 与 Manual AI HTTP 200 返回含当前随机假 key 的全 Unicode 或混合转义，但 JSON 信封截断。完整 JSON 可检测，损坏 JSON 检测失败，Tripo 错误摘要、Manual AI RawResponse 可还原 key。原始复现证据由 RD 执行并明确标为漏洞复现成功：[malformed-reflection-repro.log](../../../artifacts/encrypted-secrets/rd/malformed-reflection-repro.log)。首轮因此 FAIL，未以 RD 自测替代 QA 关闭。
- 修复：无法安全解析的 JSON body 丢弃；固定安全错误，不保留反射原文，维持失败/未知提交分类。详情见 [implementation](implementation.md)。
- QA 关闭依据：`es06-malformed-manual-ai.json`、`es06-malformed-tripo.json`，两家分别执行全 Unicode/混合转义，合计 4 个真实响应；Manual AI 为 submission_unknown、Tripo 为 failed；连续观察超过 3 次执行器唤醒，无自动新增请求。HTTP/日志/DB/blob/备份无直接或 JSON 解码 canary 命中。`es07-safe-diagnostic-control.json`：合法外层 JSON、无秘密但知识正文损坏时，仍为 needs_input，安全诊断 blob 确实保留 1 份。原合法 JSON 直接/转义错误反射 ES04 两家也通过。

## 首轮历史与测试修正

- round 1：Rust ES 11/11、AS 7/7、native 1/1；浏览器 12/15。三个浏览器失败为 QA 前置/断言问题：ES02 只匹配“加密配置”而真实固定错误说明不安全文件；ES03 把仅部件可用的 textOnly 错加到步骤/规格；ES05 同值 keep 是 no-op，造出了非法 legacy revision="deployment"。改为准确错误断言、只给部件 textOnly、先同值 replace 生成合法持久 UUID 后迁移；未修改产品或减少验收条件。
- 定向修正过程保留 `e2e-round1-harness-rerun.log`、`e2e-check-diagnostic.log`。一次编辑命令工作目录写错导致旧用例额外执行，原始记录在 `e2e-round1-command-path-error.log`；不算有效修复复验。最终 round2 全 18 项真实通过，覆盖三项完整路径和新安全回归。
- BUG-ES-001 是独立产品缺陷，首轮 FAIL 保留，交 RD 修复后本轮关闭。没有把测试失败直接冒充产品缺陷，也没有以修测试掩盖安全问题。

## 证据安全、观察与边界

精确随机 API key 与主密钥扫描在每例值仍位于内存时执行，涵盖实际应用输出、日志、DB/blob、备份/导出及 Git index；只写布尔与计数。最终 QA 文本追加直接/JSON 转义 canary 模式审计 0 命中。trace/HAR/video/自动截图关闭，手动截图 mask 密码字段。`PLAYWRIGHT_NO_COPY_PROMPT=1` 阻止页面快照；8 个自动产生的源代码上下文文件经核对均无 Page snapshot/password DOM、无 canary，已删除这些冗余 error-context，保留完整 runner 日志。证据见 `final-evidence-audit.json`；没有把私有 temp 文件拷入 artifacts。

375/1440 截图内容、说明、错误和表单操作可读；长页全页截图中的固定导航显示在拍摄时视口位置，不代表页面横向溢出，DOM scrollWidth 与 viewport 一致。保存失败截图密钥区域为遮挡色。指南审阅与真实 CLI/进程行为相符。

本次真实系统凭据验证仅 macOS 专用唯一服务/账号，未枚举或访问用户默认条目；锁定/拒绝/缺失通过注入边界验证，未锁住真实用户钥匙串。未覆盖 Linux/Windows 原生凭据、真实收费供应商认证、全平台发行或任意编码反射。没有清除历史密钥副本、旧备份或 SSD 历史块；静态加密不抵御已控制系统账号/进程。上述均为 PRD 明示边界，不冒称整个项目完成。

## 交接

本批 **round 2 full PASS，12/12 AC，未关闭缺陷 0**。测试后端、fixture 与 QA Vite 已停止；协调者可正常构建并更新预览。未改 state/PRD/生产代码。新增非代码知识已写入本报告；llmdoc 更新路径仅 `llmdoc/requirements/encrypted-secrets/qa-report.md`。
