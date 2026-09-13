# 万物说明书 · 项目知识入口

当前产品路线：React + Rust、前后端分离开发、单二进制自托管发布。2026-09-11 起替代旧 macOS 原生实现路线。截至 2026-09-13：T01–T21 已实现并经 QA 验收 PASS，T22 的 macOS 平台已交付、Linux 平台待环境恢复，T23 未开始；当前进度与实现概览见 [web-mvp 进度与实现总结](requirements/web-mvp/progress-summary.md)。

## 阅读顺序

1. [协作流程](collaboration.md)：PM → UI → RD → QA 的输入、输出和返工规则。
2. [技术架构](architecture.md)：固定选型、部署边界、资料处理和 3D 方案。
3. [接口与数据合同](contracts.md)：命名、REST、版本、任务、费用和存储约束。
4. [实施任务卡](implementation-plan.md)：小步编码顺序、依赖、允许文件、验收条件。
5. [验收与发布](validation-release.md)：要实现的命令合同、测试矩阵和单二进制检查。
6. [决策记录](decisions.md)：代码不能表达的约束、取舍和变更原因。
7. [当前初始需求](requirements/web-mvp/request.md)：由 PM 在开始工作时扩展为明细 PRD。
8. [当前进度与实现总结](requirements/web-mvp/progress-summary.md)：切片账、已实现内容、缺陷闭环、未完成项与证据索引（暂停交接快照）。

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
