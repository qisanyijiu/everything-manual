---
description: 启动或恢复 PM→UI→RD→QA 协作流程，QA 失败交回 RD 修复复验。
argument-hint: "<需求描述或已有 work-id，例如 web-mvp>"
---

你是本项目主协调会话。用户输入：$ARGUMENTS

1. 读取 `CLAUDE.md`、`llmdoc/README.md`、`llmdoc/collaboration.md`。
2. 如果参数对应 `llmdoc/requirements/<work-id>/`，读取并恢复该工作，不创建重复需求；否则将用户需求保存为新的 request.md，以日期和短名称生成 work-id，并复制 state 模板。没有参数时恢复唯一未完成工作；有多个则请用户明确目标，不能猜测。
3. 每次只按下面的 phase 派发表选择下一角色；返回后检查产物、更新 state，再重新派发。不得恢复时无条件重跑 PM/UI，`done` 不再自动修改。
4. PM 产出详细 PRD，UI 补同一 PRD 并检查 REQ→UI→AC 覆盖；UI 的 NEEDS_PM 或 RD 的 NEEDS_PRD_CHANGE 都退回 pm_pending，保留当前任务和缺陷，PM 修订后重新经过 UI。
5. RD 每次收到具体 task IDs、PRD 修订号、允许修改范围、必测 AC、交付路径；首次选择依赖满足的有限切片，恢复或返工继续 current_tasks，不抢做下一切片。需求已经授权时直接推进，不逐阶段重新询问。
6. RD_READY 后交 QA。切片 PASS 记录 task+PRD 修订+QA 报告绑定，再进入下一切片；FAIL 带缺陷交 RD，修复后回 QA。换切片、修复、PRD 变化均清空当前 qa_result，过期 PASS 不可复用。最终另做当前 PRD 的全量验收。
7. 真实阻塞时记录原因、证据、已尝试方案和唯一所需外部动作；同一问题反复出现则要求新的诊断证据，不能盲目重复、伪造 PASS 或静默结束。
8. 每阶段检查 llmdoc 是否记录新增非代码知识。仅主协调者更新 state.yaml 的阶段／修订／回合／切片／阻塞／验收状态；子 agent 返回建议。
9. 在主会话用 Agent 工具调度四个项目角色；不运行嵌套 `claude -p`，不依赖实验性 Agent Teams 或角色间直接消息。若角色未加载，提示重新启动本项目 Claude Code；不得假装已经调用角色。
10. 当前授权范围完成且 QA PASS 后，报告产物、验收证据和剩余限制。产品构建与本地 smoke 属于编码任务；推送、收费真实生成和公网部署需要已有明确授权。

| 当前 phase | 下一步 |
| --- | --- |
| pm_pending | 调用 pm，准备或修订当前 PRD |
| pm_ready | 调用 ui，在同一 PRD 补交互 |
| ui_ready | 选择／恢复 current_tasks，转 rd_running 后调用 rd |
| rd_running | 调用 rd 继续 current_tasks |
| rd_fixing | 调用 rd 修 current_tasks 的 open_defects |
| qa_running | 调用 qa 验当前交付；qa_scope=full 时做全量验收 |
| blocked | 检查解除条件；未解除只报告所需动作，解除后转 resume_phase 再派发 |
| done | 只报告已完成产物与验收；新需求另建工作或经用户明确要求重新打开 |
