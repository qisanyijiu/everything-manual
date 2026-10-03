# 万物说明书 · 项目知识入口

当前产品路线：React + Rust、前后端分离开发、单二进制自托管发布。2026-09-11 起替代旧 macOS 原生实现路线。2026-10-02 正在按用户要求继续补齐 PRD，最新范围和协调状态见 [prd-completion](requirements/prd-completion/prd.md) 与 [state](requirements/prd-completion/state.yaml)。已完成的交互 A、API 网页配置和密钥加密见下方索引；本轮发布包下载、原件阅读、知识复核闭环、准备/建单恢复、模型栏密钥防护与受控验证命令均已通过各自独立切片验收（QA1–9），当前实施资料库搜索、保存/离开保护和任务体验，随后执行当前版本的完整发布门禁。官方说明书原件与本地准备结果保存在忽略的运行目录中。

原 web-mvp 的 T01–T21 历史验收已通过；T22 尚未完整关闭，BUG-013 的测试侧修复仍缺 Linux 复验，T23 真实供应商验收待正确配置与明确预算。九月两平台构建、哈希与测试计数属于历史版本，不能作为十月新增代码的发布证明；历史细节见 [进度快照](requirements/web-mvp/progress-summary.md) 和 [implementation §R30](requirements/web-mvp/implementation.md)。

## 阅读顺序

1. [协作流程](collaboration.md)：PM → UI → RD → QA 的输入、输出和返工规则。
2. [技术架构](architecture.md)：固定选型、部署边界、资料处理和 3D 方案。
3. [接口与数据合同](contracts.md)：命名、REST、版本、任务、费用和存储约束。
4. [实施任务卡](implementation-plan.md)：小步编码顺序、依赖、允许文件、验收条件。
5. [验收与发布](validation-release.md)：要实现的命令合同、测试矩阵和单二进制检查。
6. [决策记录](decisions.md)：代码不能表达的约束、取舍和变更原因。
7. [当前初始需求](requirements/web-mvp/request.md)：由 PM 在开始工作时扩展为明细 PRD。
8. [当前进度与实现总结](requirements/web-mvp/progress-summary.md)：切片账、已实现内容、缺陷闭环、未完成项与证据索引（暂停交接快照）。
9. [交互体验改进方案](../docs/interaction-experience-improvement-plan.md)：体验评审、优先级、实施切片与验收标准；[切片 A](requirements/interaction-a/prd.md) 于 2026-09-20 完成并通过 [独立 QA](requirements/interaction-a/qa-report.md)，B/C/D 待实施。
10. [网页 API 配置](../docs/api-settings.md)：Tripo 与说明书 AI 的地址、模型、密钥设置已实现，保存后重启服务生效；2026-09-22 全部 15 项 AC 通过 [独立 QA](requirements/api-settings/qa-report.md)。该请求替代旧版仅只读、禁止网页输入密钥的范围限制，未改变 T22/T23 的验收状态。
11. [API 密钥加密存储](requirements/encrypted-secrets/prd.md)：2026-10-02 已交付并更新本地预览；网页与部署密钥文件认证加密、独立主密钥、安全迁移及供应商回显保护，[全部 12 AC 独立验收通过](requirements/encrypted-secrets/qa-report.md)，状态见 [协调记录](requirements/encrypted-secrets/state.yaml)。

12. [受控本地生成验证](test-live.md)：具名案例与受限预算文件的 `test-live` 命令，复用冻结任务、幂等与账本；本机fixture交付不代表AC-042/T23真实供应商验收完成。

## llmdoc 记录规则

只保存代码无法充分表达、值得后续子 agent 知道的重点。每条记录应能回答：结论是什么、为什么、影响哪里、证据在哪里、是否已实现／已验证、何时更新。

- 全局架构／接口设计放上述文档；跨需求决策放 decisions。
- 单次工作放 `requirements/<work-id>/`：request、PRD（含 UI）、implementation、qa-report、state。
- 可执行代码／迁移／OpenAPI／生成类型形成后，以对应实现文件为机器合同；llmdoc 记录设计原因与链接，不复制整份生成物。实现与需求不符不能以“代码为准”绕过修正流程。
- 大日志与截图放 `artifacts/<work-id>/`；这里记录摘要、相对路径和结论。
- 不记录密钥、令牌、浏览器 cookie 或整段私人资料。外部 API 文档保存链接、日期和影响结论。
- 过时记录标为 superseded，并指向替代决策；不要让旧结论和新结论同时处于 accepted。
- PM/UI/RD/QA 每次交接均列出更新过的 llmdoc；没有新增时明确说明。

模板见 [PRD](templates/prd.md)、[QA 报告](templates/qa-report.md)、[协调状态](templates/state.yaml)。状态表示本项目流程进度，不是供应商 AI 任务状态。

本轮配置与文档的实际验证范围见 [设置检查记录](setup-validation.md)，不要将其当作网站产品验收。
