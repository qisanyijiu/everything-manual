# VS-01 设计基线（ui-feedback-3-6 · 视觉流）

日期：2026-10-04。依据：[视觉风格 PRD 修订 1](../visual-style-prd.md)（§3 需求、§4 验收、§6 交互设计、§7 切片）、[request.md](../../request.md)、[实施规划第 6 节](../../../../implementation-plan.md#6-ui-反馈第-3-和第-6-项待办)、[反馈归档](../../../../../artifacts/ui-feedback-20261004/README.md)。

冻结结论：视觉流 **UI 修订 1**（对应 `prd_revision=1`，未变更需求范围）；本目录是 VS-02～VS-05 的设计对照基线。**本卡不代表浏览器验收、对比度实测、构建或 QA 通过**——执行范围声明见第 9 节。

## 1. 交付物索引

| 文件 | 内容 | 主要对应 |
| --- | --- | --- |
| [tokens.css](tokens.css) | 设计变量（PRD §6.1 全部项目；纯 CSS custom properties，无网络字体） | AC-VS-001/002 |
| [library-specimen.html](library-specimen.html) | 资料库与物品概览样页（含空库、无结果、失败、长型号、无封面、分页、窄屏堆叠） | AC-VS-001/002/004 |
| [reader-specimen.html](reader-specimen.html) | 3D 阅读器与复核样页（部件、占位画布、步骤、出处、工具条；无热点、候选/已确认、WebGL 降级、窄屏抽屉） | AC-VS-005/006/010 |
| [forms-specimen.html](forms-specimen.html) | 上传/准备/确认/任务/设置样页（分节表单、按钮与输入全状态、错误摘要、费用与发送范围、未知不显示完成） | AC-VS-007/008/010 |
| [component-states.md](component-states.md) | 15 类组件的全状态、使用场景、可访问性与键盘合同（含 UI-VS 映射） | AC-VS-009/012 |

静态自检（本回合实际执行，方法：在 `design/vs-01/` 内对三个样页做文本检索）：

- 三个样页各以 `<link rel="stylesheet" href="tokens.css">` 引用同一变量文件（3/3）。
- 三个样页无 `<script>`、无 `src=`、无 `url(`、无 `@import`、无 CDN / 网络字体引用；样页样式行引用 tokens 变量计数：library 109 行、reader 97 行、forms 118 行。全文仅有的两处 `https://` 是设置页示例文本与输入值（供应商地址样例），不产生资源加载。
- 样页与 tokens 均为 UTF-8 纯文本；未在浏览器中打开（见第 9 节）。

## 2. 三类样页 → AC-VS-001 / 002 / 012 对照基线（可执行）

### AC-VS-001（同一套设计变量；无发光/装饰渐变/多层重阴影；圆角、编号、网格符合规格）

- 对照物：`tokens.css` 是唯一数值来源；三个样页的全部颜色、字号、圆角、焦点、过渡都经 `var()` 引用（静态计数见上）。
- VS-02+ 执行方式：
  1. 将 tokens 内联进 `apps/web/src/theme.css`（第 8 节给出映射顺序）；实现代码检索引擎不得在 `theme.css` 之外出现裸十六进制颜色（允许注释与数据例外，例外逐条记录）。
  2. 圆角核对：面板 2px（`--radius-panel`）、输入/按钮 4px（`--radius-control`）、编号牌 2px（`--radius-plate`）、圆形仅头像与 3D 热点编号（`--radius-full`）。对照 library L-S3/L-S8、reader R-S4、forms F-S1。
  3. 阴影核对：正文面板零阴影；唯一允许 `--shadow-overlay`（浮层），对照 forms F-S8 对话框。
  4. 网格核对：24px 低对比网格仅出现在模型空白区（reader R-S2 画布）与装饰区；不得铺在密集正文背后。
- 截图对照：1440px 视口下资料库、阅读器、确认页三张实现截图与对应样页人工比对；差异项登记后由 RD 修正或由 UI 更新基线（二者必须走 §9 记录）。

### AC-VS-002（正文 ≥14px、必读辅助 ≥12px；正文对比 ≥4.5:1；控件边界/焦点 ≥3:1）

字号冻结值（tokens）：页标题 28/36（手机 24/32）、章节 18/26、正文 14/22、阅读正文 16/26、辅助与编号 12/18。

设计期**静态计算**的对比度（WCAG 2.x 相对亮度公式，四舍五入到 0.1；**未用浏览器或工具实测**，VS-05 须以计算样式复测）：

| 前景 / 背景 | 计算值 | 要求 |
| --- | --- | --- |
| 墨色 `#232620` / 内容 `#FFFEF9` | 15.2:1 | ≥4.5 |
| 墨色 / 页面 `#F4F1E8` | 13.6:1 | ≥4.5 |
| 辅助字 `#62665D` / 内容 | 5.8:1 | ≥4.5 |
| 辅助字 / 页面 | 5.2:1 | ≥4.5 |
| 辅助字 / 禁用软底 `#EDEAE0` | 5.0:1 | ≥4.5（禁用文字仍可读） |
| 白字 / 主操作 `#A63F21`（默认） | 6.3:1 | ≥4.5 |
| 白字 / 悬停 `#8F3518`、按下 `#7A2C12` | 7.8:1、9.6:1 | ≥4.5 |
| 主操作色 / 内容（链接文字） | 6.2:1 | ≥4.5 |
| 主操作色 / 页面（链接文字） | 5.5:1 | ≥4.5 |
| 成功 `#2F6248` / 内容；/ 软底 `#E9EFE6` | 7.0:1；6.1:1 | ≥4.5 |
| 警示 `#805D12` / 内容；/ 软底 `#F4E8D2` | 6.0:1；5.0:1 | ≥4.5 |
| 错误 `#A12F2F` / 内容；/ 软底 `#F6E3DE` | 7.0:1；5.7:1 | ≥4.5 |
| 控件边界 `#7D8176` / 内容；/ 页面 | 3.9:1；3.5:1 | ≥3（必要控件边界） |
| 焦点环 `#A63F21` / 页面 | 5.5:1 | ≥3（焦点指示） |
| 选中边线 `#A63F21` / 选中底 `#EEE2CF`；墨色 / 选中底 | 4.9:1；12.0:1 | 选中三重表达（边线+底色+文字）各自 ≥3 |

VS-05 执行方式：对实现后的计算样式逐对复测（方法见第 7 节）；上表只作设计值对照，不构成通过。

### AC-VS-012（构建回归；主入口 gzip 增幅 ≤10 KiB；无新增第三方 UI/字体依赖）

- 本项「改前基线」在 VS-02 动工前、以冻结修订的干净构建采集；采集程序：

```sh
npm --prefix apps/web run build
JS=$(ls apps/web/dist/assets/index-*.js); CSS=$(ls apps/web/dist/assets/index-*.css)
cat "$JS" "$CSS" | gzip -9 | wc -c      # 主入口 gzip 合计（字节）；改前/改后必须同一公式
```

- 依赖检查：`git diff -- apps/web/package.json apps/web/package-lock.json` 不得出现新增 UI/字体运行依赖；构建产物不得新增字体或图片网络请求。
- 现成参考：`apps/web/dist/` 存在 2026-10-04 交付检查时的构建产物（可查看入口文件名），但**本回合未测量其体积**，也未执行任何构建命令（原因见第 9 节）。基线数值采集后写入 VS-02/VS-05 实施记录。

## 3. REQ → UI → AC 全量映射复核（8 REQ × 6 UI × 12 AC）

### 3.1 REQ 闭合检查

| REQ | PRD 所列 AC | UI 覆盖 | 结论 |
| --- | --- | --- | --- |
| REQ-VS-001 | AC-001、002 | UI-VS-001 | 闭合 |
| REQ-VS-002 | AC-003、010 | UI-VS-001 | 闭合 |
| REQ-VS-003 | AC-004 | UI-VS-002 | 闭合 |
| REQ-VS-004 | AC-005、006 | UI-VS-003 | 闭合 |
| REQ-VS-005 | AC-007、008 | UI-VS-004 | 闭合 |
| REQ-VS-006 | AC-002、009、010 | UI-VS-001、UI-VS-003 | 闭合 |
| REQ-VS-007 | AC-011 | UI-VS-005 | 闭合 |
| REQ-VS-008 | AC-012 | UI-VS-006 | 闭合 |

### 3.2 UI 闭合检查

| UI | PRD 所列 REQ / AC | 修正 | 结论 |
| --- | --- | --- | --- |
| UI-VS-001 | REQ-001/002/006；AC-001/002/003/009/010 | 无 | 闭合 |
| UI-VS-002 | REQ-003；AC-004 | 无 | 闭合 |
| UI-VS-003 | REQ-004；AC-005/006/009/010 | 补列 REQ-006（AC-009/010 的来源，已改 PRD §6.3，见 §9） | 闭合（修正后） |
| UI-VS-004 | REQ-005；AC-007/008 | 无 | 闭合 |
| UI-VS-005 | REQ-007；AC-011 | 无 | 闭合 |
| UI-VS-006 | REQ-008；AC-012 | 无 | 闭合 |

无孤儿 UI（每个 UI 至少挂一个 REQ 且其 AC 均在 REQ 的 AC 集合内或已补列）。

### 3.3 AC 闭合检查（每项至少一个 UI，修正后）

| AC | 来源 REQ | UI | 基线锚点（样页/文档） |
| --- | --- | --- | --- |
| AC-VS-001 | REQ-001 | UI-VS-001 | tokens.css 全文；library L-S1～L-S9、reader R-S2/R-S4、forms F-S9 |
| AC-VS-002 | REQ-001、006 | UI-VS-001、003 | 本 README 字号/对比表；三样页 token-note 标注 |
| AC-VS-003 | REQ-002 | UI-VS-001 | library L-S1；component-states §1 |
| AC-VS-004 | REQ-003 | UI-VS-002 | library L-S3～L-S7 |
| AC-VS-005 | REQ-004 | UI-VS-003 | reader R-S2/R-S4/R-S6（候选与已确认区分） |
| AC-VS-006 | REQ-004 | UI-VS-003 | reader R-S3/R-S5（WebGL 降级与文字恢复） |
| AC-VS-007 | REQ-005 | UI-VS-004 | forms F-S3/F-S5/F-S6/F-S7 |
| AC-VS-008 | REQ-005 | UI-VS-004 | forms F-S2/F-S5/F-S8/F-S10 |
| AC-VS-009 | REQ-006 | UI-VS-001、003 | component-states §15 键盘合同；reader R-S5 |
| AC-VS-010 | REQ-002、006 | UI-VS-001、003 | library L-S9、reader R-S7、forms F-S10；component-states §2/§9 |
| AC-VS-011 | REQ-007 | UI-VS-005 | reader R-S1/R-S2 语法；standalone 迁移要求见 3.4 缺口 4 |
| AC-VS-012 | REQ-008 | UI-VS-006 | 本 README 第 2 节（AC-012 程序）与 component-states |

### 3.4 缺口与修正记录

1. **已修正（PRD §6.3）**：UI-VS-003 的 REQ 引用由「REQ-004」补为「REQ-004、006」——其 AC-009/010 来自 REQ-006，原表存在引用不完整（不改变任何需求含义）。
2. **已修正（PRD §6.3）**：UI-VS-002 空库引文由「还没有说明书」对齐为实现现状与样页一致的「还没有物品」（仅文案引用修正，不改变 AC-004 含义）。
3. **已修正（PRD §9 记录）**：§6.1 五处数值/说明完善（悬停/按下取值、状态软底、网格线取色、禁用态定义、过渡冻结值、字体栈指引、面板圆角由「0～4px」定为 2px）。
4. **已补充（PRD §6.3 表后）**：指向本目录 component-states.md 的组件状态入口。
5. **实现风险提示（不属映射缺口，移交 VS-03）**：`apps/web/src/features/standalone/styles.ts` 仍使用旧主题（`#f7f8f4`、`#356552`、`#c8643c` 等），与 AC-VS-011「色彩、字号、状态与应用阅读器一致」不符。VS-03 必须以 tokens.css 同步新导出 HTML 的样式；历史已导出 HTML 不回写（PRD §2）。
6. **实现风险提示（移交 VS-02）**：现有 `theme.css` / `styles.css` 的状态与规格不一致点——辅助字号多处 9–11px、面板 12px 圆角、按钮 8px 圆角、多处轻阴影、`:focus-visible` 为 `#be693f`/offset 4px、输入边界 `#e2e6dd` 对内容底远低于 3:1。VS-02 按第 8 节映射替换；眉题（英文小标）保留时须 ≥12/18 等宽，不得低于下限。

## 4. tokens.css 对 PRD §6.1 逐项覆盖清单

| §6.1 项目 | token | 样页实例 |
| --- | --- | --- |
| 纸面与正文 | `--color-paper/-surface/-ink/-ink-muted/-ink-on-accent` | 全部样页 body/面板/文字 |
| 强调与状态 | `--color-accent/-hover/-active/-selected/-success/-warning/-danger/-*-soft` | L-S1 导航、L-S3 状态牌、F-S9 按钮矩阵、R-S4 标记、F-S6 金额 |
| 线条与网格 | `--color-line/-line-control/-grid-line`、`--grid-size`、`--border-width` | 面板 1px 实线、R-S2 画布网格、F-S9 输入边界 |
| 字体 | `--font-sans/-mono`、`--font-weight-*`、`--letter-spacing-label` | 型号/编号等宽（L-S3、R-S2）、眉题 |
| 文字层级 | `--font-size-page-title/-section/-body/-reading/-aux` 及各 `--line-height-*`（含 767px 断点覆盖） | L-S2 标题注记、F-S9、R-S5 原文摘录、F-S1 辅助 |
| 布局 | `--space-*`（8px 制 + 4px 微调）、`--layout-content-max/-measure/-grid-gutter/-nav-width` | 全样页间距；`--layout-grid-gutter` 在 L-S9 12 列示意 |
| 组件形状 | `--radius-panel/-control/-plate/-full`、`--shadow-overlay` | L-S3 封面、F-S9 圆角、F-S8 浮层阴影、R-S4 圆形标记 |
| 操作反馈 | `--control-height/-touch/-compact`、`--focus-ring-*`、`--duration-*/-ease-standard`（含减少动效归零与 `--focus-ring-color-inverse`） | F-S9 全状态、R-S2 紧凑工具条、L-S1 焦点环 |

定义但暂无样页直接实例、供 VS-02 引用的 token：`--layout-grid-columns-desktop/-mobile`、`--layout-margin-mobile`、`--duration-panel`、`--focus-ring-color-inverse`（深色浮层）。其余 token 均有样页使用实例（静态检索确认）。

## 5. §6.3 UI 合同状态覆盖清单（样页章节）

| UI-VS | 要求的状态/行为 | 样页章节 |
| --- | --- | --- |
| UI-VS-001 | 导航选中、按钮五态、禁用原因、骨架按分区 | L-S1、L-S2、L-S7、F-S1、F-S9；component-states §1/2/3/7 |
| UI-VS-002 | 空库、无结果、错误不显示零条、长型号、无封面回退 | L-S3～L-S7 |
| UI-VS-003 | 目录/热点联动、当前项语义、无热点「暂无定位」、候选可区分、文字入口可达 | R-S2、R-S4、R-S5、R-S6 |
| UI-VS-004 | 错误贴近字段+摘要、阶段名称与图标、未知不显示完成、失败重试显式 | F-S1～F-S8 |
| UI-VS-005 | 版本/章节可展开、断网/能力缺失恢复提示、继承阅读器按钮与面板 | R-S1（入口与语法同源）；独立 HTML 迁移与断网实测移交 VS-03/VS-05 |
| UI-VS-006 | 组件说明全状态、页面证据覆盖 §6.2、交付记录格式 | 本 README + component-states.md |

## 6. 冻结的 UI 修订建议

- 建议主会话写入：`streams.visual_style.ui_revision = 1`、`streams.visual_style.status = design_frozen`、`current_tasks` 转入 VS-02（具体字段由主会话决定，本文不写入 state.yaml）。
- `prd_revision` 保持 1：本回合改动均为映射引用修正与数值/说明完善，未新增或修改 REQ/UI/AC 含义（§9 有记录）。
- 若后续实现发现 tokens 数值需再调，按「数值修正」处理并记录；改变需求范围则退回 PM。

## 7. 度量方法（供 VS-05 执行）

1. **对比度**：目标 正文/辅助文字 ≥4.5:1、必要控件边界与非文本指示/焦点 ≥3:1。方法：浏览器 DevTools 读取元素 `color` 与有效背景色的计算样式，按 WCAG 2.x 相对亮度公式计算；逐对填写本 README 表的「实测」列（VS-05 在 QA 报告中另建表，不改本文件）。不得宣称获得外部认证。
2. **字号**：DevTools computed `font-size`/`line-height`：正文 ≥14/22、辅助与编号 ≥12/18、阅读正文 16/26、页标题 28/36（375px 下 24/32）；检出任何信息性 <12px 文本即登记。
3. **触控尺寸**：375 与 768 视口下对全部交互元素读 `getBoundingClientRect()`，目标 ≥44×44 CSS px（主操作、工具条、链接式操作、底栏按钮）。
4. **真实 200% 缩放**：桌面浏览器页面缩放 200%，检查无页面级横向滚动、固定导航不遮挡控件、正文不截断。
5. **截图矩阵**：§6.2 各组页面 × 375/768/1024/1440 全量；改前/改后成对留档到 `artifacts/`（由 VS-05 决定目录），用于 AC-010 与 AC-001 的对照。
6. **体积**：第 2 节 AC-012 程序；改前基线在 VS-02 动工前采集。

## 8. 给 RD 的确定性实现顺序

原则：先 token、再基础组件、后逐页替换；不一次性重写；不改路由、数据、文案语义；每步保持现有测试与类型检查通过。

VS-02（本卡冻结后）：

1. `theme.css`：内联 tokens.css 的 `:root` 与两个媒体查询；建立最小兼容别名（其余用法逐页替换）：`--bg`→`--color-paper`、`--surface`→`--color-surface`、`--text`→`--color-ink`、`--muted`→`--color-ink-muted`、`--border`→`--color-line`、`--accent`→`--color-accent`、`--danger`→`--color-danger`、`--ok`→`--color-success`。
2. 全局基础：body 文字、h1–h3 字号、`:focus-visible` 换 `--focus-ring-*`；按钮三态与禁用规则（第 2 节 + component-states §2）；卡片 12px→`--radius-panel` 并去阴影；输入/选择边界改 `--color-line-control`（原 `--border` 太浅，不满足 ≥3:1）。
3. `AppShell` 与登录：导航选中三重表达（L-S1）、移动底栏 44px、顶栏任务入口状态。
4. 资料库与物品概览：档案行、空态、失败保留上次结果、分页、状态牌、缺字段「未提供」、冲突提示（L-S2～L-S9）。
5. 验证：`npm --prefix apps/web run typecheck`、`npm --prefix apps/web run lint`、受影响组件测试与定向 E2E；1440/375 截图与样页对照；再提交 VS-05 所需证据。

VS-03（依赖 VS-02）：viewer/manual 展示层与热点标记语法（R-S3/R-S4/R-S6）、原文面板（R-S5）；`standalone/styles.ts` 迁移到 tokens（缺口 4）；新导出 HTML 样式同步。VS-04：import/jobs/settings 对照 F-S1～F-S10。每片开始前先跑一次 AC-012 的改前基线并留档。

## 9. 本回合验证范围声明

- 本回合为静态设计复核：产物为文档、变量与静态样页；**未**在浏览器中打开样页或应用，**未**执行对比度实测（仅设计期静态计算），**未**执行 375/768/1024/1440 截图或真实 200% 缩放检查，**未**执行键盘实操，**未**运行构建、类型检查、lint 或体积 gzip 测量（本回合无执行环境，也未把命令写成已通过）。
- 5 秒识别测试**未实施**（PRD §1 的研究目标；不引用任何历史测试数）。
- 本回合不写实现记录、QA 报告或缺陷；不修改 `state.yaml`、`community-incentives-prd.md`、`apps/**`、`crates/**`、`migrations/**`、`contracts/**` 与反馈原件。
- 样页为设计意图的表达，不是可用功能；不得以样页充当浏览器验收。

## 10. 未覆盖项与待实测项

| 项目 | 现状 | 归属 |
| --- | --- | --- |
| 浏览器渲染与截图对照（三样页 → 实现） | 未执行 | VS-02～VS-05 |
| 对比度/字号/触控实测 | 未执行（仅静态设计值） | VS-05 |
| 真实 200% 缩放、四断点截图矩阵 | 未执行 | VS-05 |
| AC-012 改前基线与 gzip 增量 | 未采集 | VS-02 开工前采集；VS-05 核对 |
| 键盘实操与减少动效实测 | 未执行（合同已定义） | VS-05 |
| 独立 HTML 断网打开与样式一致性 | 未执行（迁移要求已写明） | VS-03 实现；VS-05 验收 |
| 5 秒识别研究 | 未实施 | 研究记录（非工程门禁） |

## 11. llmdoc 更新与跨需求知识

本回合更新：

- 新增：`llmdoc/requirements/ui-feedback-3-6/design/vs-01/`（本文件、tokens.css、三个样页、component-states.md）。
- 修改：`llmdoc/requirements/ui-feedback-3-6/visual-style-prd.md`（§6.1 数值完善、§6.3 映射修正与入口、标题状态、§9 冻结记录）。
- 未修改：`state.yaml`（主会话唯一写入）、`decisions.md`（超出本卡写入边界）。

候选跨需求决策（登记草案，建议主会话评估后由后续角色写入 `decisions.md`）：

1. 设计变量单一来源：全站（含独立 HTML 阅读器与未来社区页面）共用 tokens.css 命名；数值变更须同步 PRD、tokens 与验收截图——影响 theme.css、standalone、CI-03 前端；状态：设计冻结；证据：本文件第 4 节。
2. 禁用态不用整体降透明度（软底 + 辅助字色 + 原因常驻），保证禁用文字仍可读并可解释。
3. 状态软底只作辅助，不得单独承载含义；任务「结果未知」永不用成功色或完成文案。
4. 独立 HTML 阅读器与应用共享同一 token 集与热点标记语法，不引入网络字体或新依赖。

新增非代码知识摘要：§6.1 冻结取值与理由（见 visual-style-prd.md §9）；候选/已确认热点的可区分视觉语法；档案行窄屏堆叠规则；AC-012 体积基线的采集口径。
