# VS-04 独立 QA 报告（qa_round4 / qa_round5）

日期：2026-10-05；执行者：Codex 独立 QA。首次切片 **FAIL**；返工复验 **PASS**（52矩阵/真实缩放/键盘、11近邻、1显式部分成功）。本报告不代表 VS-05 全 AC 最终验收或生产发布构建已通过。

## 1. 源码与运行绑定

- qa_round4：`var/ui-feedback-3-6-vs05-qa/qa4-source`，冻结 330 文件，时间 `2026-10-04T17:19:08.614668+00:00`。
- qa_round5：`var/ui-feedback-3-6-vs05-qa/qa5-source`，冻结 330 文件，时间 `2026-10-04T17:30:52.994916+00:00`；返工仅 `theme.css` 两条窄屏规则，文件 SHA256 `ea63ed9ef2e2468cb6e0ecb12102aab75403d2a792713c6ecbb0311eb5e682d9`。
- 两轮后端均为当前源码构建的 `everything-manual-final-fixture` 副本，SHA256 `a34384eb9e990ea066a2b93a681eac01b7782cbb4aea6150c07ee8e96d684618`，`job-failpoints` 仅为本机验收 fixture；没有以历史 fixture 或生产二进制冒充本轮行为证据。
- 常规视觉/行为 Chrome `154.0.8037.93`；模型使用 SwiftShader。真缩放专用 Chromium persistent extension context实际版本 `148.0.7778.96`（qa4/qa5原始native-zoom.json均已记录），采用MV3 `chrome.tabs.setZoom(2)`，并验证 zoom、DPR、innerWidth 与 outerWidth；此处明确两类浏览器运行身份，不改变原测量结论。
- 前端端口 `15406/15407`，后端 `18406` 或自管随机回环端口；全部 provider/model CDN 指向本机合成 fixture，外网浏览器请求被禁止；无付费调用。
- 自有 Vite cache、临时数据库与证据目录；不改生产源码、规划 state 或历史 Claude QA 报告。trace/video/自动失败截图关闭；设置截图仅为空密钥表单或 masked password 输入。含密钥近邻套件额外 `EM_AS_QA_NO_SCREENSHOTS=1`。
- 文件级 SHA 与冻结复查：`source-hashes-qa4.json` / `source-hashes-qa5.json` / `freeze-validation.json` / `qa5/qa-binding.json`。冻结副本 0 文件漂移。

## 2. qa_round4 首次结果：FAIL

实际执行 32 个受影响业务测试，全数 PASS；实际四宽度 13 组页面 × 375/768/1024/1440 = 52 个组合，截图与 computed CSS 留档。发现两项违反窄屏 44×44px 条件的缺陷，未将其记为 PASS：

| ID | 真实路由与操作 | 实测首次结果 | 要求 / 结果 |
|---|---|---|---|
| BUG-VS04-001 | 准备页、费用确认页，原件选择 `.field__input` | 375px 下 **335×41px** | 窄屏 ≥44×44；FAIL |
| BUG-VS04-002 | 复核页“刷新草稿数据” `.link-button` | 375px 下 **72×18px** | 窄屏 ≥44×44；FAIL |

原始证据：`qa4/visual-initial/`；稳定状态重测：`qa4/visual/four-widths.json`、`four-width-issues.json`、`qa4-visual-settled.log`。实际稳定状态只有 3 条异常，对应以上 2 个 ID。主操作无其它不足 44px、页面级 overflow 或小于 12px 的文字异常。

首轮测量脚本的非产品误报已明确保留：透明 sidebar-account 边框不属于必要绘制边界；不可点击的 disabled 控件不计触控目标；原文按钮由 loading 转 enabled 时须等真实 PDF canvas 加载并静置 350ms，不能将过渡色当稳态。方法修订见 `qa4/measurement-method-revision.md`。阈值没有降低。

## 3. qa_round5 返工复验

两处样式返工后重新冻结，真实路由 52 个组合复验 **0 异常**。DOM rect：

| ID | 375px 返工后 | 1440px 原有紧凑次操作 | 关闭条件 |
|---|---|---|---|
| BUG-VS04-001 | prepare / confirm **335×44px** | 下拉 960×41px | 窄屏达到44；PASS |
| BUG-VS04-002 | review **72×44px** | 链接式次操作72×18px | 窄屏达到44；PASS |

证据：`qa5/visual/four-widths.json`、`four-width-issues.json`（空数组）、52 张 PNG、`qa5/visual-results.json` 前三项，以及 `qa5-visual.log`。

受影响近邻回归 **11/11 PASS**，详见 `qa5/results.json` / `qa5-neighbors.log`：

- AS-QA-05：真实配置密钥清除屏蔽部署、单家恢复、未配置 API 拒绝生成。
- import-flow 窄屏键盘；IA-QA-02 四尺寸确认；IA-QA-03 延迟/取消/失败、低预算、同键显式重试。
- PC3-004 中止等待 in-flight PUT / 缺页续传 / 显式封存；BUG-PC3-001 原生 radio 方向键与焦点。
- PDF prepare 逐页进度、取消、离开确认、单 canvas 约束。
- T19-8 复核键盘、减少动效、WebGL 上下文丢失恢复。
- PC2B-007/008 真实 PATCH 412 / 发布412 / 已知与未知422 / 输入保留 / 显式重读与不可变发布。

补充 `VS04-BEH-04` **1/1 PASS**（`qa5/supplemental/visual-results.json` / `qa5-partial-success.log` / `qa5/supplemental/visual/partial-success.json`）：真实本机 fixture 使模型下载/校验成功、知识抽取拒答进入缺项；UI总状态等待人工补齐，模型阶段已完成且无重试，知识阶段缺项且有服务端允许的重试，样式/Tab 操作不增加 `{upload:2,submit:1,task:1,manual:1,cdn:1}` 计数。

该新增用例的首次执行曾因 **QA脚本属性错误** 超时，完整保留在 `qa5/visual-results.json` 第四项及 `qa5-visual.log`：脚本误用 `s.kind`，既有合同实际为 `JobStageView.stageKind`。首次最后真实响应已经为 `model_validate=succeeded/manual_extract=needs_input`；没有产品失败。仅修正QA属性与既有显示标签定位（“缺项（等待人工）”），不改产品、不降低业务断言，仅重跑该用例于独立supplemental目录。原首次错误没有隐藏或覆盖。

## 4. AC-VS-007～010 的实际验收覆盖

| AC | 本轮真实执行与证据 | 结论与边界 |
|---|---|---|
| AC-VS-007 | 上传超限/一次失败→重试、准备中断、报价过期显式重报、确认前禁用与旧确认隔离、未知提交恢复同job/同key、任务缺项与失败；`qa4/results.json`；prepare取消在 qa5补验 | PASS；qa5显式部分成功补充确认 model_validate=succeeded / manual_extract=needs_input，状态及恢复动作准确 |
| AC-VS-008 | settings未保存导航/刷新、密钥仅内存/不回显不缓存；真实412/422与发布显式确认；UI未配置/报价加载失败；`AS-QA-02/08/09`、`T19-4/5/7`、`PC2B-007/008`、`IA-QA-07`；qa5 AS-QA-05 | PASS于这些实际用例；本报告不替代生产密钥迁移全合同套件 |
| AC-VS-009 | IA-QA-05 标签Arrow/Home/End、抽屉Tab/Esc/焦点恢复/热点列表替代；纯Tab skip-link→Enter到main；五页真实focus环与减少动效；T19-8 | PASS；`keyboard-reduced-motion.json` 记录3px环、3pxoffset、5.543:1与0s过渡 |
| AC-VS-010 | 13组页面52组合；全部页面级横向overflow≤1；受测可点击窄屏控件/主操作≥44；实际Chrome浏览器200%五页 | qa4 FAIL；qa5所测矩阵/zoom PASS。非VS04页全18组最终矩阵留待VS05 |

qa4业务32/32涵盖文件：`api-settings-qa`、`import-flow`、`interaction-a-qa`、`job-recovery`、`manual-review`、`preparation-discovery`、`review-tasks`、`workflow-recovery`。没有宣称 Playwright 默认忽略的 qa-pc 独立套件已执行。

## 5. 浏览器实测

13组：新建表单、上传原件、照片、准备、费用确认、任务列表、任务详情、结果、生成历史、复核、版本列表、应用阅读器、设置。

正常/辅助文本实测范围最低字号 **12px**；已采集稳态文本最低对比 **4.5878:1**（辅助字 `#62665D` / 选中底 `#EEE2CF`）；输入必要边界 `#7D8176` / 页面为 **3.5258:1**。样式层未检查之外的动态状态不能据此宣称全部合规；错误/冲突用例的实际行为由定向套件覆盖。

| 实测颜色 | 背景 | 对比 |
|---|---|---|
| 辅助 `#62665D` | 页面 `#F4F1E8` | 5.1968:1 |
| 辅助 `#62665D` | 内容 `#FFFEF9` | 5.8125:1 |
| 白主操作 | `#A63F21` | 6.2605:1 |
| 成功 `#2F6248` | `#E9EFE6` | 6.0624:1 |
| 警示 `#805D12` | `#F4E8D2` | 4.9594:1 |
| 错误 `#A12F2F` | 内容 `#FFFEF9` | 7.0186:1 |
| 焦点 `#A63F21` | 页面 `#F4F1E8` | 5.5432:1 |

真实200%五页：新建表单、确认、任务详情、应用阅读器、设置。`chrome.tabs.getZoom=2`，innerWidth **1440→720**，outerWidth **1440→1440**，DPR **1→2**；逐页无横向滚动，滚动后的主操作中心命中自身，固定导航没有遮挡。证据 `native-zoom.json` 与五张masked PNG。

四宽度样式访问前后 provider 计数保持 `{upload:2, submit:1, task:1, manual:1, cdn:1}`；浏览器外网请求0、pageerror0。详见 `four-width-runtime.json`。

## 6. 重跑与后续范围

从项目根目录执行自有 `freeze-source.py qa5` 只允许一次冻结；随后：

```sh
EM_VS05_QA_STAGE=qa5 apps/web/node_modules/.bin/playwright test -c var/ui-feedback-3-6-vs05-qa/visual.playwright.config.ts
```

完整32业务入口为 `run-qa4.sh`（stage可选择qa4/final）；qa5近邻的实际命令保留于独立执行日志与本轮报告。不在生产工作树跑构建，不复用线上监听服务。

VS-05仍须以父agent最终冻结版本验收全部 AC-VS-001～012，关闭已知正文行高/独立HTML h1/不可用3D提示偏差，执行18组×4宽度同fixture AFTER、新导出离线文件、构建gzip与生产embedded-ui/no-job-failpoints smoke。VS04矩阵不能替代这些最终证据。

## 7. VS-05 新发现的人工评审缺陷

在qa5实际截图的人工评审中追加发现 **BUG-VS05-001**：`job-detail-1440.png` 阶段说明宣称“按执行顺序列出”，但当前记录01组装草稿、02冻结输入，实际任务存在并行DAG分支，编号是列表定位序号。该矛盾未被旧行为/geometry套件发现。主会话已接受并修订说明文字，v1最终freeze作废；将在v2最终source的真实fixture UI中验证说明与编号，不新增执行排序功能、不回改首次截图。该项属于VS-05整体验收，不将现有VS-04切片结果充作全AC最终PASS。
