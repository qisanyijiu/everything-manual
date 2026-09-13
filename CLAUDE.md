# 万物说明书 · Claude Code 项目协作约定

## 当前目标

React + TypeScript 前端，Rust 后端；开发时通过 HTTP API 分离，发布时前端静态资源、服务端和 SQLite 引擎合入一个二进制。用户数据在独立 data-dir。完整约束从 `llmdoc/README.md` 开始读取。

当前仓库处于技术方案和协作配置阶段，尚无新网站实现。`docs/everything-manual-implementation-plan.md` 是旧 macOS 方案，只作历史背景；不得继续实现 SwiftUI、SceneKit 或强制 USDZ 转换。无需先恢复旧 Swift 源码。

## 默认协作流程

收到明确的开发需求后，主会话充当协调者，按 `llmdoc/collaboration.md` 执行：

`PM → UI → RD → QA →（FAIL → RD → QA，直到 PASS）`

- 使用项目子 agent `pm`、`ui`、`rd`、`qa`。由主会话顺序调用 Agent 工具；不依赖子 agent 互相创建或直接发送消息。
- PM 先读需求，编写明细 PRD；UI 把交互设计补进同一 PRD；RD 依据冻结的 PRD 编码；QA 独立验收。
- 主会话维护 `llmdoc/requirements/<work-id>/state.yaml` 并路由交接。角色只返回结果和建议状态，不抢写协调状态。
- 不跳过 PM/UI，不以 RD 自测代替 QA，不以模板、mock 演示或未执行的命令声称通过。
- 本项目每次只安排一个生产代码写入者。PM/UI/QA 的研究可在不争写文件的情况下进行，主交付顺序不变。
- 已获授权的需求不在每阶段重复请求用户确认。明确假设可继续；真实业务歧义、缺失权限或外部依赖才报告 BLOCKED。
- 用户仅要求方案、文档、配置或解释时，交付该范围，不自动开始产品编码。`/team <需求或工作编号>` 是启动／恢复上述开发流程的明确入口。

## llmdoc 是子 agent 的持久上下文

每个角色开工先读 `llmdoc/README.md`、相关技术文档、当前工作目录和 `llmdoc/decisions.md`。

代码无法充分表达的内容，必须提炼重点记录到 `llmdoc/`：需求背景、范围取舍、交互意图、架构决策、外部 API 限制、失败原因、风险、验收结论与未解决问题。记录“结论、原因、影响、证据、状态、日期”，并链接相关代码／测试／原始资料。

不复制大段源码、聊天、日志或可自动生成的类型定义；不保存密钥与私人令牌。长日志／截图放 `artifacts/<work-id>/`，llmdoc 只写摘要和链接。角色完成时必须报告 llmdoc 更新；确实没有新增信息时明确写“无新增非代码知识”。

## 实现硬约束

- 以用户最新要求为最高业务依据；再按当前 PRD、`llmdoc/architecture.md`、`llmdoc/contracts.md` 与决策记录实现。冲突必须显式解决。
- React 19 + R3F 9；Rust Axum + Tokio + SQLite。GLB 为网页运行资产。
- Rust 直接调用 Tripo 与说明书 AI HTTP API，密钥留在服务端。生产运行不要求 Node/Python/Redis/外部数据库/PDF 可执行程序。
- 默认测试禁止真实收费 API；fixture 与真实 Provider 显式隔离。未配置服务返回未配置状态，不能假成功。
- 任务提交未知不能自动重复创建付费任务；重生成资产不能静默复用旧热点。
- 文档中的未来命令先按任务卡实现，再运行；不得把不存在的脚本写成已通过。
- 不修改用户已有无关改动，不强制重置仓库，不自动提交／推送／公网发布。正常本地构建与测试属于编码任务范围。

## 快速入口

- 总索引：`llmdoc/README.md`
- 团队流程：`llmdoc/collaboration.md`
- 技术架构：`llmdoc/architecture.md`
- 接口和数据合同：`llmdoc/contracts.md`
- 编码任务卡：`llmdoc/implementation-plan.md`
- 验收和发布：`llmdoc/validation-release.md`
- 初始需求：`llmdoc/requirements/web-mvp/request.md`

在 Claude Code 中运行 `/team web-mvp` 开始 PM 阶段；新需求运行 `/team <需求描述>`；恢复时传已有工作编号，不创建重复工作。
