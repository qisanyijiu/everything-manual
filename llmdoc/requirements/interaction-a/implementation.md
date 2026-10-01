# interaction-a · IA-01 实现交接

日期：2026-09-19；依据：[PRD revision 1 / UI revision 1](prd.md)；RD 状态：RD_READY，待独立 QA。只交付原体验方案的切片 A，不代表十项方案全部实现；不修改 web-mvp 的 T22/T23 状态。

## 1. 实际行为与文件

| 文件 | 改动及原因 | REQ / AC |
| --- | --- | --- |
| [ItemFormPage](../../../apps/web/src/features/library/ItemFormPage.tsx) | 创建成功直接到新物品的说明书上传路由；原有编辑保存仍回概览。创建等待、失败留存沿用现有请求状态与字段错误 | REQ-001；AC-001/002 |
| [ConfirmStepPage](../../../apps/web/src/features/import/ConfirmStepPage.tsx) | 资料/缺项 → 报价与预算 → 发送范围 → 明确确认 → 生成/受理统一进入单列主内容，四个断点都无需打开侧栏。保留完整费用单位、上界、模型、资料范围、价格日期、有效期、预算语义及恢复入口 | REQ-002/003；AC-003/004/005 |
| 同上 | 生成同时检查当前勾选、当前报价确认成功及非保存中；取消立即禁用，重新勾选重新等待；换报价立即清除确认并用请求代次忽略旧响应。确认保存失败回退，提交中锁定确认；增加同步提交守卫，保留同页面失败重试的幂等键、已有任务锁定及受理结果 | REQ-004；AC-006/007/008 |
| [PageLayout](../../../apps/web/src/features/shell/PageLayout.tsx) | 中屏标签左右循环、Home/End 同步激活和焦点。只有活动标签可 Tab 进入；有可聚焦内容时直接进入子控件，无控件时面板本身为 Tab 落点。单面板与手机 Drawer 继续走原有路径 | REQ-005；AC-009/010 |
| [theme.css](../../../apps/web/src/theme.css) | 确认卡片上限 960px；两项预算桌面分列、手机单列；正文 14px、必读辅助说明及操作标签至少 12px。后置规则覆盖原有阅读器/手机的高优先级小字号和小按钮，指定手机点击区至少 44×44px，主操作保持正常文档流并为底栏留空间 | REQ-006；AC-011/012 |

测试改动：新增 [创建路由组件回归](../../../apps/web/src/features/library/ItemFormPage.test.tsx)、[确认与幂等组件回归](../../../apps/web/src/features/import/ConfirmStepPage.test.tsx)、[面板键盘组件回归](../../../apps/web/src/features/shell/PageLayout.test.tsx)。更新 [import-flow](../../../apps/web/tests/e2e/import-flow.spec.ts) 与 [qa-t16-independent](../../../apps/web/tests/e2e/qa-t16-independent.spec.ts) 中创建后概览和确认侧栏的旧布局假设；原有费用、请求计数、上传、错误与焦点断言保留。QA 自己新增的 interaction-a-qa 测试由 QA 维护，RD 未修改。

## 2. 已执行验证

以下为真实执行结果，不替独立 QA 签发 PASS。命令均从仓库根目录执行。

| 命令 / 检查 | 实际结果 | 覆盖和边界 |
| --- | --- | --- |
| `npm --prefix apps/web run build` | exit 0 | 包含 `tsc --noEmit` 与 Vite 生产构建；既有大 chunk 提醒保留，不是错误 |
| `npm --prefix apps/web run lint` | exit 0 | 0 warnings |
| `npm --prefix apps/web run test` | exit 0，20 文件、144 测试通过 | 基线 130 + RD 新增 12 + QA 并行新增键盘用例 2；最终运行包含网络异常拒绝和创建中再次点击断言 |
| `npm --prefix apps/web run test:e2e -- import-flow.spec.ts qa-t16-independent.spec.ts` | 首轮实际启动后 17/18 通过，exit 1 | 唯一失败为缺准备指针时按钮旁的通用缺项文案没有具体“准备”说明；其余创建→上传→准备→确认→受理、请求去重、费用校验、四尺寸确认主流程均通过 |
| `npm --prefix apps/web run test:e2e -- qa-t16-independent.spec.ts -g 'QA-11'` | 修正后 1/1 通过，exit 0 | 按钮旁现在展示第一条真实缺项消息，原有无准备指针/不请求报价/不伪造状态断言通过；未声称最终整套 18 条同轮重跑 |
| `git diff --check` | exit 0 | 无空白错误 |
| 主会话 in-app 375px 检查 | 创建成功直达上传、表单标签 computed 14px、无页面横向溢出 | 主会话实测并交接给 RD；不是独立 QA 结果 |

第一次 E2E 尝试受沙箱本机监听限制失败（`listen EPERM 127.0.0.1:15173`），按权限流程升级后真实隔离服务器与 Chromium 已成功启动并完成以上测试。E2E 使用专有临时 data-dir、本地 fixture 配置和公开 HTTP；未调用真实收费供应商。

原 T16 测试硬编码历史截图路径，本次生成物由协调者归档到 [本批 RD 证据目录](../../../artifacts/interaction-a/rd)，再恢复测试前干净的历史产物，避免覆盖之前的验收证据。未执行的独立 QA 字体/触控完整测量、手机阅读器及真实浏览器 200% 缩放，以后续 [QA 报告](qa-report.md) 为准，RD 不用改 viewport 代替真实缩放。

## 3. 复验路径与 AC 映射

- AC-001/002：新建填名称/型号，等待响应期间按钮禁用且不提前导航；成功直接到 `import/document`；失败保留输入并展示错误；已有物品编辑保存仍回概览。组件测试 3 条及原向导 E2E 覆盖。
- AC-003/004：同一确认 URL 在 375/768/1024/1440px 保持报价、发送范围、确认、生成的纵向顺序且无面板触发器；缺项和服务未配置不伪造金额。既有 E2E 的 QA-9 已更新并实际通过，独立 QA 再测完整字段/字号与异常状态。
- AC-005–008：延迟确认、取消再勾选、确认失败、过期只手动重报、旧确认晚返回、两种预算独立校验、同页面网络失败显式重试复用 key、已有任务锁定、受理后只给详情入口。组件测试与原 T16 实际请求计数共同覆盖；不是跨刷新恢复或真实远端计费验证。
- AC-009/010：中屏打开多面板，左右/Home/End 后核对焦点、ARIA、tabIndex、内容；有控件面板不增加额外停靠点，无控件面板可聚焦；单面板不多加标签；手机从资料库摘要验证 Esc 归还焦点。新增组件测试及既有 shell/窄屏 E2E 通过，真实阅读器由独立 QA 复验。
- AC-011/012：用真实含步骤/部件/PDF 的阅读器检查最终计算字号和实际操作矩形，检查手机、200% 缩放的换行、焦点与底栏遮挡。RD 样式已实现、创建页与确认尺寸检查已执行，完整测量与实际缩放结果待独立 QA。

## 4. 非代码知识与限制

| 结论、原因与影响 | 证据 | 状态 / 日期 |
| --- | --- | --- |
| 勾选是当前页面的操作意愿，服务端确认是既存记录；取消只阻止当前页面生成，不撤销审计。新报价和重勾选均不得复用旧页面成功状态 | ConfirmStepPage 的确认守卫与 7 条状态回归；PRD UI-003 | implemented、RD verified，2026-09-19 |
| 报价重取期间仍可能收到旧确认响应；开始新报价时增加代次并清空状态，旧响应的成功/失败/finally 都不能更新新报价的确认状态。只解决同页面竞争，不增加浏览器持久化或跨刷新恢复承诺 | 旧报价延迟响应组件回归 | implemented、RD verified，2026-09-19 |
| 两类预算低于上界必须分别判定。原逻辑用共享 budgetBelow 会让未低于上界的另一输入也显示警告；本次拆开并保留整体禁止条件 | 独立预算组件回归 | implemented、RD verified，2026-09-19 |
| 大小阈值必须以后置且足够具体的样式覆盖，并测 computed style。仅修改通用 button 不能覆盖现有阅读器与手机规则 | theme.css 后置交互层；PRD UI-005 | implemented，完整浏览器阈值待 QA，2026-09-19 |
| 更新历史验收测试是因为路由和布局的产品要求改变，不能删除原费用/幂等/请求计数/焦点保证 | import-flow、qa-t16-independent 的有限差异及实际结果 | RD verified，2026-09-19 |

未增加依赖、后端 API、DTO、OpenAPI、持久化、真实收费请求或发布操作。跨标签页准备恢复、真实步骤完成状态、复核待办定位、出处自动唤起、搜索与导出继续按原方案后续切片处理。初始用户暂存文档改动保留，未提交 Git。

初始交接没有来自本批独立 QA 的 defect ID。RD 自测发现的缺项文案回归已修正并定向通过，不能登记为 QA 已关闭缺陷；后续 QA 缺陷修复见下节。

本轮 RD 更新的 llmdoc：`llmdoc/requirements/interaction-a/implementation.md`。新增知识包括确认状态语义、报价异步响应隔离、预算独立提示、样式验证边界和历史证据保留；没有通用架构/合同变化，不修改全局 decisions。

## 5. QA 回合 1 修复：IA-BUG-001

日期：2026-09-20；依据 PRD revision 1 / UI revision 1、[QA 报告](qa-report.md) 与 [实际失败日志](../../../artifacts/interaction-a/qa/e2e-focus.log)。RD 状态：已修复、RD_READY，待 QA 复验；不由 RD 标记 CLOSED。

- **缺陷与复现**：IA-BUG-001，P2，AC-010/UI-004、UI-005。375px 阅读器打开「步骤与原文」，等待 PDF 和文字层，聚焦「下一页」后按 Tab，焦点直接回「关闭」，跳过随后原生 `summary`。
- **根因**：[Drawer](../../../apps/web/src/features/shell/Drawer.tsx) 焦点陷阱只识别链接、按钮和表单控件，没有包含原生 `summary`，于是错误地把 PDF 下一页视为最后一个停靠点。只检查 `tabIndex !== -1` 还会误纳其他负值、显式 tabIndex 的禁用按钮及隐藏子树。
- **修复范围**：仅改 Drawer 的停靠点发现逻辑，加入 `summary`；每次 Tab 动态过滤禁用（含 disabled fieldset）、负 tabIndex、hidden/inert、display:none 容器、visibility:hidden/collapse 和关闭 details 中除 summary 外的内容。保留正常浏览器 Tab 顺序，只在真实首尾回绕；Esc 与关闭后还焦行为不变。未改 PRD、state 和 QA 独立测试。
- **有意义的回归**：[Drawer.test.tsx](../../../apps/web/src/features/shell/Drawer.test.tsx) 新增 3 条：下一页→summary 的原生 Tab 不被取消；末端与关闭按钮双向循环且隐藏/禁用元素不成为落点；details 展开/折叠时内部按钮动态进入/离开顺序。jsdom 不模拟原生 Tab 移焦，因此第一条验证事件未被拦截，真实移焦由 QA 独立浏览器用例复验。
- **实际命令**：`npm --prefix apps/web run test -- src/features/shell/Drawer.test.tsx src/features/shell/shell.test.tsx src/features/shell/PageLayout.test.tsx src/features/shell/interaction-a-qa.test.tsx`，exit 0，4 文件 26 测试通过；`npm --prefix apps/web run build`（含 typecheck）exit 0；`npm --prefix apps/web run lint` exit 0；`git diff --check` exit 0。此回合没有重跑全套单元或把此前 144 数字冒充修复后的全套结果。

新增非代码知识：焦点陷阱必须包括浏览器原生折叠入口，并以当前可见、可用的停靠点计算边界；异步加载的 PDF 内容和 details 开合使固定初始列表不足。影响仅为抽屉键盘顺序，状态为 implemented / RD verified，2026-09-20，证据为上述代码、单元和 QA 失败日志。llmdoc 更新路径仍为本文件。
