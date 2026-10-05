# 视觉流实施记录（ui-feedback-3-6）

依据：[视觉风格优化 PRD](visual-style-prd.md) **修订 1**、[VS-01 设计基线](design/vs-01/README.md)（UI 修订 1，2026-10-04 冻结）、[实施规划 §6.2](../../implementation-plan.md#62-视觉风格任务)。验收结论由独立 QA 记录，本文件不改需求范围、不写通过结论。

---

## VS-02 · 主题、外壳与资料库（2026-10-04，RD）

状态：实施与自测完成，待 QA 独立验收。依据 PRD 修订号：`prd_revision=1`，`ui_revision=1`（设计冻结基线 `design/vs-01/`）。

### 1. 范围

按任务卡 **VS-02**：全局主题、共享组件（按钮/输入/状态牌/骨架/提示）、应用外壳与导航、登录页、资料库与物品概览。允许前端样式/结构和定向测试；未改后端、路由与数据逻辑、生成与发布规则；未改 `features/standalone/**`、`import/jobs/settings` 自有页面（其全局外观随共享主题变化，见 §7 回归说明）。

明确未做（属 VS-03/VS-04）：阅读器/复核/原件面板的专属样式迁移；`standalone/styles.ts` 迁移；import/jobs/settings 页面逐区对照；版本列表补齐。

### 2. 修改与新增文件

| 文件 | 类型 | 内容 |
| --- | --- | --- |
| `apps/web/src/theme.css` | 修改 | 内联 tokens.css 的 `:root` 与两个媒体查询；建立兼容别名；重建基础组件层（按钮五态、输入、卡片/面板、导航、资料库/详情/登录样式）。 |
| `apps/web/src/features/library/LibraryPage.tsx` | 修改 | 移除营销 hero 区（对齐样页 L-S2 构图）；档案行缺字段「未提供型号」；状态牌变体类；失败态提示与游标提示类名。 |
| `apps/web/src/features/library/ItemOverviewPage.tsx` | 修改 | 规格栏补充 型号/品牌/变体（缺失显示「未提供」）；状态牌变体类。 |
| `apps/web/src/components/Icon.tsx` | 修改 | `ManualArtwork` 取色改为 token（`var(--color-*)`），不再使用旧绿色板与硬编码十六进制。 |
| `llmdoc/requirements/ui-feedback-3-6/visual-implementation.md` | 新增 | 本记录。 |

未修改：`apps/web/package.json`、`apps/web/package-lock.json`（无新增依赖）、`crates/**`、`migrations/**`、`contracts/**`、仓库中用户已有无关未提交改动、`features/standalone/**`。

### 3. tokens 落地方式

- `theme.css` 的 `:root` 内联 `design/vs-01/tokens.css` 全部变量（颜色/字体/层级/布局/形状/操作反馈），并附两个媒体查询：`max-width:767px` 页标题 24/32、`prefers-reduced-motion` 时长归零。
- 兼容别名（README §8 第 1 步）：`--bg → --color-paper`、`--surface → --color-surface`、`--text → --color-ink`、`--muted → --color-ink-muted`、`--border → --color-line`、`--accent → --color-accent`、`--danger → --color-danger`、`--ok → --color-success`；另补 `--surface-soft`、`--color-border`、`--color-text-muted`、`--color-surface-muted` 适配旧功能层引用；`--radius → --radius-panel`（2px），控件在基础层显式取 `--radius-control`（4px）。
- `styles.css` 未改动：其 `:root` 旧值与暗色媒体查询被 `theme.css`（后加载、同特异性）覆盖，功能层通过别名自动继承新 token。**该文件内的旧 `:root` 已不生效**，后续若有人依它取值会出错（见 §7 风险）。
- README §4 覆盖清单逐项核对：颜色/字体/间距/层级/圆角/阴影/焦点/时长均在实现中出现（焦点环 `--focus-ring-*`、过渡 `--duration-control`）；定义但暂无实现实例的 token：`--layout-grid-columns-*`、`--layout-margin-mobile`、`--duration-quick/-panel`、`--focus-ring-color-inverse`、`--color-grid-line`/`--grid-size`（留 VS-03 阅读器画布）、`--color-warning` 于本片仅用于状态牌变体/冲突提示。

### 4. AC-VS-012 基线（改前）与改后构建

改前基线在**任何代码修改之前**、`apps/web` 工作树干净时采集（2026-10-04，Node v26.0.0 / npm 11.12.1）：

```sh
npm --prefix apps/web run build        # exit 0
JS=$(ls apps/web/dist/assets/index-*.js); CSS=$(ls apps/web/dist/assets/index-*.css)
cat "$JS" "$CSS" | gzip -9 | wc -c     # → 135368
```

| 项 | 改前（基线） | 改后 | 差值 |
| --- | --- | --- | --- |
| 主入口 JS | `index-PvaA1NtB.js` 412,060 B | `index-Cw1auV5E.js` 412,726 B | +666 B |
| 主入口 CSS | `index-BViv6ey9.css` 55,425 B | `index-HnZaNl2F.css` 64,729 B | +9,304 B |
| **JS+CSS gzip -9 合计** | **135,368 B** | **136,397 B** | **+1,029 B（约 1.0 KiB，≤10 KiB 上限）** |
| 第三方 UI/字体依赖 | 7 个运行依赖（react、react-dom、react-router、@tanstack/react-query、@react-three/fiber、three、pdfjs-dist），无 UI 套件/字体包 | 同左，`git diff -- apps/web/package.json apps/web/package-lock.json` 为空 | 无新增 |
| 资源请求 | 构建 CSS 无 `url()`；dist 内字体仅 pdfjs vendor `standard_fonts`（PDF 渲染既有资产） | 同左 | 无新增 |

改后哈希：JS `f25cf75dba3e22fff5589de683bdfec4ba95a3191ea4be2156e844cacc4da022`，CSS `d6d1c93293b5d36603eeee39d5807c2bc618568a303afba0adee615531c9e216`。改前哈希：JS `eea038ed5149577437186ad00e34283e456ea9d6c6f7e6a9fbfcfd17e6a174af`，CSS `8a2db500071c260c272c77052487e2f67b9a7bb4c79e7f0ab1b5d348592fac63`。

> VS-03/04 开工前须各跑一次本节程序采集新的“改前”基线（README §8 末段）；VS-05 以同一公式核对总增量。

### 5. 实际执行的验证

| 命令（`apps/web` 下） | 结果 |
| --- | --- |
| `npm run typecheck` | exit 0，无输出错误 |
| `npm run lint` | exit 0（`eslint . --max-warnings=0`） |
| `npm run test` | 46 个测试文件 / **311 个用例全部通过**（最终一轮，含 LibraryPage/AppShell/登录/PageLayout 等全部既有用例；无新增失败） |
| `npm run build` | exit 0；体积见 §4 |
| `PLAYWRIGHT_BROWSERS_PATH=<repo>/var/playwright-browsers npx playwright test -c var/vs02-smoke/playwright.config.ts` | **6 个 Chrome 冒烟用例全部通过**（Chrome 154 stable；无后端、无付费依赖：`vite preview` 静态构建 + 路由级 fixture）。用例与截图见 §6 |
| `EM_E2E_SERVER_BINARY=<repo>/target/aarch64-apple-darwin/release/everything-manual PLAYWRIGHT_BROWSERS_PATH=<repo>/var/playwright-browsers npx playwright test library-row-layout.spec.ts`（既有 e2e 配置 + 真实后端，临时 data-dir，无付费调用） | **2 个用例通过**（RD-ROW-1 桌面 1024–1920、RD-ROW-2 窄屏 <768）。实测几何：名称列宽 1024→1920px 为 273/187/237/281/375/462px（阈值 ≥120），主操作 90×44，无溢出；几何 JSON 见 `artifacts/web-mvp/t16-rd-fix/library-row-geometry.json`。说明：本轮首次（Vite dev 冷启动）运行 RD-ROW-1 曾失败一次、疑为 CSS 注入时序，其后连续 5 次（含清空 `node_modules/.vite` 冷缓存）全部通过；未保留失败现场（被复跑覆盖），建议 VS-05 同样本复跑确认。该 harness 按其设计覆写了 `artifacts/web-mvp/t16-rd-fix/`（几何 JSON、截图 01/02/03）与 `artifacts/web-mvp/t09-rd/e2e-server.log`，为本次运行的真实证据，未还原。 |

冒烟脚本（测试夹具，不入库）：`apps/web/var/vs02-smoke/`（`playwright.config.ts`、`vs02-smoke.pw.ts`，位于 gitignore 的 `var/`）；截图存 `var/ui-feedback-3-6-vs02/screenshots/`（gitignore）：`vs02-library-1440.png`、`vs02-library-1440-full.png`、`vs02-library-375.png`、`vs02-library-768.png`、`vs02-library-1024.png`、`vs02-login-1440.png`、`vs02-detail-1440.png`、`vs02-library-empty-1440.png`、`vs02-library-failure-1440.png`。

冒烟实测断言（计算样式，非截图目测）：

- 正文 14px、辅助/编号 12/18px、页标题 28/36px（375px 下 24/32px）；
- 页面底 `rgb(244,241,232)`、墨色 `rgb(35,38,32)`；输入边界 `rgb(125,129,118)`（`--color-line-control`）；主操作实底 `rgb(166,63,33)` 白字；
- 面板/列表圆角 2px 且 `box-shadow: none`；按钮/输入 4px；状态牌 2px 矩形；
- 首个 Tab 目标是「跳到主要内容」，焦点环实测 `3px / offset 3px / rgb(166,63,33) / solid` 且 `:focus-visible` 命中；
- 侧栏选中项实测 左边线 `rgb(166,63,33)` + 底 `rgb(238,226,207)` + `font-weight 600` + `aria-current="page"`。**更正（2026-10-04，BUG-VS02-001 修复回合）**：该 `aria-current` 主张当时仅对 `/` 成立；`/items/{id}`、`/items/new` 下三项视觉选中在、语义缺失（QA 回合 1 判缺陷）。根因与修复见文末「BUG-VS02-001 修复记录」；
- 登录禁用按钮实测软底 `rgb(237,234,224)` + 文字 `rgb(98,102,93)` + `opacity: 1`（不整体降透明度）；
- 375/768/1024/1440 四档：`document.scrollWidth ≤ innerWidth`（无页面级横向滚动）；375 下移动底栏各项高度 ≥44px、档案行退化为单列；190px 中档侧栏内容不溢出（品牌英文小标可省略号截断，字号不低于 12px）；
- 空库显示「还没有物品」+ 新建入口；请求失败显示错误面板与「重试」，面板内不出现「0 件」字样（保留上次结果逻辑由既有单测覆盖）。

### 6. AC 映射（本片覆盖部分）

| AC | 实现处 | 证据 |
| --- | --- | --- |
| AC-VS-001 变量统一、无发光/渐变/多层阴影、圆角/编号/网格符合规格 | `theme.css`（token 块 + 面板 2px/控件 4px/唯一 `--shadow-overlay`）；`Icon.tsx` 装饰改用 token；仓库内规范装饰 SVG | 冒烟计算样式断言（§5）；`grep` 显示 `theme.css` 之外无新增裸十六进制色；网格 token 定义未铺在任何正文背后（唯一网格在 `viewer-panel`，留 VS-03 迁移） |
| AC-VS-002 字号与对比度 | 层级 token（14/22 正文、12/18 辅助、28/36 标题）；输入边界 `--color-line-control` | 计算样式实测（§5）；对比度为设计期静态值（README §2），实测留 VS-05 |
| AC-VS-003 页标题/导航一致/路由可刷新 | `AppShell` 侧栏三重选中 + 顶栏页名；`LibraryPage`/`ItemOverviewPage` 标题结构 | 冒烟直接刷新 `/`、`/items/item-3` 均 200 渲染；导航项名称/图标未变；`aria-current` 实测 |
| AC-VS-004 档案行/空库/无结果/失败/分页/缺字段 | `LibraryPage`（行结构、`未提供型号`、状态牌、空态、错误面板、分页文案保持真实语义）；`ItemOverviewPage` 规格栏「未提供」 | 冒烟 3 个用例 + 既有 `LibraryPage.test.tsx`（搜索/分页/失败保留/游标 422）全绿 |
| AC-VS-009 键盘与焦点 | 全局 `:focus-visible` 3px/偏移 3px；skip link；既有 Drawer/PageLayout 键盘合同未动 | 冒烟首个 Tab + 焦点环实测；既有 shell/Drawer 测试全绿；减少动效：tokens 归零 + 既有 `prefers-reduced-motion` 规则（本片未在浏览器中开启该模式实操，留 VS-05） |
| AC-VS-010 响应式与触控 | 四档断点样式；375 移动底栏 ≥44px；按钮/输入控制高度 token；列表窄屏堆叠 | 四档冒烟（无横向滚动 + 375 触控高度）；真实 200% 缩放未执行（留 VS-05） |

其余 AC（005/006/007/008/011/012 全量）不在本片覆盖范围：005/006/011 属 VS-03，007/008 属 VS-04，012 的整体核对与截图矩阵属 VS-05。AC-VS-012 的改前/改后单项已在 §4 记录。

### 7. 偏差、风险与待办

1. **移除资料库 hero 区**（原绿色大图块与营销文案）。理由：PRD §6.2 资料库构图为「标题/实际计数 → 搜索 → 有序档案行」，样页 L-S2 无 hero；原 hero 的链接与「新建物品」重复。若 UI 认为需保留，请更新基线并回派。
2. **顶栏任务入口文案未改**：`design/vs-01/component-states.md` §1 写「N 进行中」/「任务状态暂不可用」，实现仍为「进行中任务 N 个」/「任务数暂不可用」。理由：`JobActivityLink.test.tsx` 与 `qa-pc05c-browser.spec.ts` 以现有文案断言；语义要求（计数有界、失败不用 0 冒充）已满足。若需对齐文案，应由 VS-04 连带更新测试。
3. `.status-label` 全站重排为矩形编号牌（2px、12px、中性色 + `--success/--warning/--danger/--done/--unknown/--failed/--archived` 变体），同时命中 settings/jobs/manual 既有用法；这些页面不在本片视觉对照范围（VS-04/05 复核）。`GenerationHistoryPage` 的 `status-label--{queue|local|provider|manual}` 变体尚未配色（VS-04）。
4. `styles.css` 的旧 `:root` 与暗色媒体查询保留但已被 `theme.css` 覆盖（同特异性、后加载）；暗色模式随 `color-scheme: light` 关闭，符合 PRD 固定纸面方向。后续改 `styles.css` 变量前必须确认该覆盖关系。
5. 遗留裸十六进制色（按 §8 顺序留后续片）：阅读器/原件/下载（VS-03）：`#e7eedd/#bfccad/#edf0e5/#dfe5d6/#fffefa/#527449/#2939230a/#829a70/#993c25`；设置/任务（VS-04）：`#64746a/#eef3e9/#765326/#fbf1df/#c9d4c6/#fffef9/#edf0e6/#f6f6f3/#fff` 等。VS-03 另需迁 `standalone/styles.ts`（缺口 4）。
6. 中档（768–1279）侧栏宽 190px 时品牌英文小标以省略号截断（保持 ≥12px，不因宽度下调字号）。
7. 未执行项（留 VS-05）：对比度逐对实测、375/768/1024/1440 全页面矩阵、真实 200% 缩放、键盘全流程实操（含减少动效模式）、旧/新截图成对比对、gzip 总增量核对；既有 e2e 全量套件未跑（其覆盖面含阅读器/任务等本片外页面），本片只跑了资料库行的 RD 几何守卫与定向 fixture 冒烟。
8. 观察（非本片引入，未改代码）：`JobActivityLink` 读取 `query.data?.data.active`，当响应缺少 `data` 字段时会抛渲染错误（冒烟夹具失误时复现）；正常契约（`{data:{active}}`）下无影响。若 VS-04 顺带加固，请以正式缺陷流程记录。

### 8. 给 VS-03 / VS-04 的衔接

- **通用**：所有新样式必须引用 `theme.css` 中的 token（本片已确立：面板 `--radius-panel`、控件 `--radius-control`、牌 `--radius-plate`、焦点 `--focus-ring-*`、正文/辅助字号变量）；禁止再引入裸色值，迁移后请从 §7.5 清单划掉。
- **VS-03（阅读器/复核/独立 HTML）**：`reader-page`/`viewer-panel`/`original-panel`/`release-download` 仍是旧值与硬编码色；`viewer-panel__stage` 网格请改用 `--color-grid-line` + `--grid-size`（24px，仅模型空白区）；`standalone/styles.ts` 用同一 token 集；`--focus-ring-color-inverse` 供深色浮层。
- **VS-04（表单/任务/设置）**：按钮/输入/状态牌基础层已统一，页面内 `provider-settings`、`preparation-*`、`arrange-*`、`wizard-*` 的硬编码色与旧圆角按样页 F-S1～F-S10 替换；`status-label--queue/local/provider/manual` 待配色；顶栏任务入口文案若对齐 component-states §1，需同步更新 `JobActivityLink.test.tsx` 与 `qa-pc05c-browser.spec.ts`。
- **测试基座**：`apps/web/var/vs02-smoke/` 的 Playwright fixture（无后端、路由 mock）可直接复制扩展页面矩阵；改前基线务必按 §4 程序重新采集。

### 9. llmdoc 更新与新增非代码知识

本回合更新：**新增本文件**（`llmdoc/requirements/ui-feedback-3-6/visual-implementation.md`）。未修改 PRD、tokens、design 基线、`state.yaml`、`decisions.md`。

新增非代码知识（本文件 §3/§4/§7 收录，供后续写入决策记录）：

- 兼容别名层方案：旧功能层变量（`--bg` 等）在 `theme.css` 指向新 token，避免逐条改写 `styles.css`；`--radius → --radius-panel` + 控件显式 4px 是区分「面板 2px / 控件 4px」的关键。
- 禁用态实现组合：软底 + 辅助字色 + `opacity: 1`（需显式覆盖旧 `opacity` 降透明规则，否则旧值仍生效）。
- AC-012 口径确认：`index-*.js + index-*.css` 拼接后 `gzip -9`；VS-02 实测 +1,029 B，说明 token 化与页面重排的样式增量远小于 10 KiB 预算。
- 资料库构图取舍：hero 区移除依据 PRD §6.2 与样页 L-S2（登记为偏差，等待 UI/QA 复核）。
- 中档侧栏 190px 的品牌小标截断策略（保 12px 字号优先于完整展示英文眉题）。

---

## BUG-VS02-001 修复记录（2026-10-04，RD；依据 `visual-qa-report.md` 回合 1）

状态：**已修复，待 QA 复验（回合 2）**。本记录不代替验收；缺陷是否关闭由 QA 判定。

### 1. 根因

- 侧栏「资料库」原为 `NavLink to="/" end`：`className` 回调以 `isActive || location.pathname.startsWith("/items")` 补 `active` 类；但 react-router v7 对 `aria-current` 是**内部门控**——`node_modules/react-router/dist/development/chunk-BV7QT456.mjs:10714`：`let ariaCurrent = isActive ? ariaCurrentProp : undefined;`，其中 `isActive` 仅按 `to`（`/` + `end`）与当前路径匹配。
- 于是 `/items/*` 下「视觉选中」与「语义标记」来自两个不同条件：视觉在、`aria-current` 缺（BUG-VS02-001）。
- **兼容性陷阱（对 VS-03/04/05 有效）**：外部显式给 NavLink 传 `aria-current` **无法覆盖**（该 prop 被解构后仅在 NavLink 自身 `isActive` 为真时透出），QA 报告修复建议中「直接传参」的写法单独使用不生效。表达「跨路由族选中」只能改用 `Link` + 自算布尔，或重组匹配目标。

### 2. 修复点（`apps/web/src/features/shell/AppShell.tsx`）

- 第 12 行：移除不再使用的 `NavLink` 导入（修复后仓库生产代码无 NavLink 用法）。
- 第 34–44 行：新增单一选中判定，三个导航项统一为「路由族」布尔：`inRouteFamily(base) = pathname === base || pathname.startsWith(base + "/")`；`libraryActive = pathname === "/" || inRouteFamily("/items")`；`jobsActive = inRouteFamily("/jobs")`；`settingsActive = inRouteFamily("/settings")`。
- 第 74–78 行：三个导航项改为 `<Link>`，`className={xActive ? "active" : undefined}` 与 `aria-current={xActive ? "page" : undefined}` 出自**同一布尔**。
- 有意的小幅收紧：旧 `startsWith("/items")` 会把 `/itemsfoo`（落入 `*` NotFound）也算资料库选中；新判定要求 `/items` 或 `/items/` 前缀。五个验收路由行为不变。
- 未改：路由表、DOM 结构（仍为 `nav.sidebar-nav > a`，文案/图标/href 不变）、CSS（`.sidebar-nav a.active` 三重态原样命中）、无新增依赖。

### 3. 五路由 aria 矩阵（修复前 → 修复后）

修复前（QA 回合 1 `var/ui-feedback-3-6-vs02-qa/aria-matrix.log`；RD 用本修复脚本复现同样失败于 `/items/{id}`）：

| 路由 | 选中项 | 3px 边线/选中底/加粗 | aria-current |
| --- | --- | --- | --- |
| `/` | 资料库 | 有 | page |
| `/items/{id}` | 资料库 | 有 | **null（缺陷）** |
| `/items/new` | 资料库 | 有 | **null（缺陷）** |
| `/jobs` | 任务中心 | 有 | page |
| `/settings` | 设置 | 有 | page |

修复后（RD 自验 `var/ui-feedback-3-6-vs02-fix/aria-matrix-fix.log`，真实后端 + Chrome 154）：五路由均**恰一个** `.sidebar-nav a.active`，其 `aria-current="page"`、`border-left 3px rgb(166,63,33)`、底 `rgb(238,226,207)`、`font-weight 600`；未选中两项 `aria-current` 均缺失。

| 路由 | 选中项 | aria-current | 备注 |
| --- | --- | --- | --- |
| `/` | 资料库 | page | 与修复前一致 |
| `/items/{id}` | 资料库 | page | 缺陷修复点 |
| `/items/new` | 资料库 | page | 缺陷修复点 |
| `/jobs` | 任务中心 | page | 与修复前一致 |
| `/settings` | 设置 | page | 与修复前一致 |

### 4. 回归测试与判别力

- `apps/web/src/features/shell/shell.test.tsx` 新增 1 例「侧栏导航选中语义（UI-VS-001 / AC-VS-003，BUG-VS02-001 回归）」：五路由逐一断言「恰一个 `.active`、文字与 href 正确、`aria-current="page"`、其余两项无 `aria-current`」。
- 判别力验证：临时把 `AppShell.tsx` 换回 HEAD 版本后，该用例在 `/items/new` 精确失败（`expected 'page', received null`）、其余 19 例通过；修复版全绿。

### 5. 实际命令与结果（`apps/web` 下；日志在 `var/ui-feedback-3-6-vs02-fix/`）

| 命令 | 结果 |
| --- | --- |
| `npm run typecheck` | 修复后首次（并行写入流改动落盘前）**exit 0**；最终复核 **exit 2**，6 个 TS 错误**全部**位于并行流的 `features/viewer/interactive.ts`、`features/viewer/useInteractive.ts`（见下「环境事实」），本修复文件零错误 |
| `npm run lint`（eslint . --max-warnings=0） | exit 0 |
| `npm run test` | **46 文件 / 312 用例全部通过**，exit 0（较回合 1 的 311 例 +1，即本缺陷回归用例；`vitest-postfix.log`） |
| 真实浏览器自验（`run-real.sh`：release 二进制 `66bece17…`（同 QA）+ 临时 data-dir + Vite dev 当前工作树；供应商 `base_url=127.0.0.1:1`，无付费调用） | **1 用例通过**，五路由矩阵见 §3（`real-fixed.log`、`aria-matrix-fix.log`、截图 `screenshots/fix-nav-*.png`） |

- 自验脚本（gitignore 的 `var/`，`*.pw.ts` 命名避免被 vitest 收集）：`apps/web/var/ui-feedback-3-6-vs02-fix/`（`vs02-fix-aria.pw.ts`、`playwright.fix.config.ts`、`run-real.sh`）；端口 API 18086 / Web 15186，与 QA 套件（18085/15185）及其证据目录隔离。
- 修复前复现留档：`var/ui-feedback-3-6-vs02-fix/pre-fix/`（单测失败日志、浏览器失败日志与 `playwright-output-buggy/` 现场）。

### 6. 环境事实（供 QA 回合 2 归因，非本修复范围）

- 本回合验证期间（2026-10-04 23:14:44 起）另一并行写入流在 `apps/web/src/features/viewer/` 持续修改（`interactive.ts`、`useInteractive.ts`、`InteractionPanel.tsx` 及新增 `part-inspection.ts`、`PartsExplorer.tsx`、`parts-explorer.css`）。
- 因此**最终时点**的 `npm run typecheck` exit 2 与 `npm run test` 无关联：vitest 312 全过；TS 错误全部在该流文件。QA 复验若仍见同源错误，应归属该并行流，不计入 VS-02 缺陷。

### 7. llmdoc 更新

- 本文件：更正 §5 旧主张（上方）+ 新增本记录。未修改 PRD、design 基线、QA 报告、`state.yaml`、`decisions.md`。
- 新增非代码知识：react-router v7 `NavLink` 的 `aria-current` 内部门控行为与「跨路由族选中」实现方式（§1、§2），以及 QA 建议中直接传参写法不生效的更正；供 VS-03/04/05 的前端导航/标签语义实现参考。

---

## VS-03 · 阅读器、复核与独立 HTML（2026-10-05，RD）

状态：实施与自测完成，待 QA 独立验收。依据 PRD 修订号：`prd_revision=1`，`ui_revision=1`（设计冻结基线 `design/vs-01/`，含 `reader-specimen.html` R-S1～R-S7 与 `component-states.md` §6/§9/§15）。

### 1. 范围与文件清单

按任务卡 **VS-03**：viewer/manual/原件与 standalone viewer 展示层，包括新导出独立 HTML。在用户并行流（viewer-interactive：`PartsExplorer`/`part-inspection`/分件全屏观察等，2026-10-04 23:16 起持续）之上实施，**未回退**其任何功能行为。

| 文件 | 类型 | 内容 |
| --- | --- | --- |
| `apps/web/src/features/viewer/hotspot-markers.ts` | 新增 | 冻结标记语法（见 §2）：`describeHotspotMarker`、`paintHotspotMarker`、`hotspotMarkerSpriteSize`、`HOTSPOT_MARKER_COLORS` |
| `apps/web/src/features/viewer/hotspot-markers.test.ts` | 新增 | 语法纯函数与画布绘制的判别力测试（实心/虚线+?/选中外环/未知状态不冒充候选/尺寸等比） |
| `apps/web/src/features/viewer/ViewerStage.tsx` | 修改 | `ViewerHotspotView.status` 透传；标记改为画布贴图 Sprite（随模型尺寸等比、`depthTest:false`）；桥 `anchors` 增加 `status` |
| `apps/web/src/features/viewer/bridge.ts` | 修改 | `ViewerAnchorProjection` 增加 `status`（只读投影，供 QA 观察候选/已确认映射） |
| `apps/web/src/features/viewer/parts-explorer.css` | 修改 | 分件观察面板 token 化（2px 面板/4px 控件/aux 字号；并行流后续在其上新增全屏观察规则，未回退） |
| `apps/web/src/features/viewer/interactive.ts` | 修改 | `PartAnimator.highlight` 默认取色改为 `--color-accent`（3D 选中与列表选中一致）；无逻辑变更 |
| `apps/web/src/features/manual/review-state.ts` | 修改 | `usableHotspots` 透传 `status`（candidate/confirmed）；`summarizePartHotspots` 增加 `candidate` 计数 |
| `apps/web/src/features/manual/review-state.test.ts` | 修改 | 覆盖候选统计与状态透传 |
| `apps/web/src/features/manual/CalibrationWorkspace.tsx` | 修改 | 复核工作区根加 `page reader-page`；热点状态牌区分「候选热点 N（待复核）」（警示软底）/「热点已确认 N」（成功软底）；选中项加「已选」牌；候选条文案改为「虚线空心环标记」（不再说"橙色"） |
| `apps/web/src/features/manual/ReleaseReaderPage.tsx` | 修改 | 状态牌变体（热点=success/无热点=warning/仅文本=中性）；选中项「已选」牌；无热点提示改「暂无定位」；热点 `status` 透传 3D；步骤「引用部件」chip 加 `aria-current` |
| `apps/web/src/features/manual/ReleaseReaderPage.test.tsx` | 修改 | 新增状态牌/已选/暂无定位断言；fixture 增加无热点部件 |
| `apps/web/src/theme.css` | 修改 | 阅读器/复核/原件/版本列表/下载面板 token 化（替换旧绿色系 `#edf0e5/#dfe5d6/#527449/#e7eedd/#bfccad/#fffefa/#2939230a/#829a70/#993c25` 与 10–11px 字号）；阅读器宽屏三栏 `minmax(220px,240px)｜1fr｜minmax(300px,340px)`；模型容器 24px 低对比网格（`--color-grid-line`/`--grid-size`）；候选条/交互面板/分件 chips 矩形化；窄屏触控名单补 `.step-parts__chip/.interaction-chip/.parts-explorer__node` |
| `apps/web/src/styles.css` | 修改 | 仅两处残留裸色（`.interaction-chip--related`、`.candidate-bar`）改引用 token |
| `apps/web/src/features/standalone/styles.ts` | 重写 | 离线内联样式 = tokens 取值（纸面/墨色/工业橙、字号层级、2/4px 圆角、3px 焦点环、140ms 且减少动效归零）；类名与 DOM 兼容不变 |
| `apps/web/src/features/standalone/build-html.ts` | 修改 | 页头增加「版本信息」`details`（`#em-release`，由脚本以 textContent 填充发布版本/发布时间） |
| `apps/web/src/features/standalone/build-html.test.ts` | 修改 | 新增「含可展开版本信息且无外链资源」断言 |
| `apps/web/src/features/standalone/viewer/main.ts` | 修改 | 热点/选中色改为 accent/accent-active；场景底色改纸面；部件状态牌「无热点」；填充版本信息（与并行流的相机/尺寸改进共存） |

自验脚本（gitignore 的 `var/`，`*.pw.ts` 命名避免被 vitest 收集）：`apps/web/var/ui-feedback-3-6-vs03/{vs03-reader.pw.ts, vs03-helpers.ts, playwright.vs03.config.ts, run-vs03.sh}`。证据：`var/ui-feedback-3-6-vs03/`（`run-final.log`、截图 13 张、`reader-linkage.json`/`export-verification.json`/`responsive-matrix.json`）。

### 2. 候选/已确认热点的视觉语法落地（AC-VS-005 的冻结语法）

- **3D 标记**（`hotspot-markers.ts` → `ViewerStage`，画布贴图 Sprite，始终正对相机、尺寸随模型等比）：已确认＝实心 accent 圆；候选＝surface 实底 + accent **虚线空心环 + 问号**；选中＝再加 3px 焦点环（纸色隔离带）。`depthTest:false` + `renderOrder:10` 使标记不被模型表面切碎（与离线阅读器一致）。
- **不伪造编号**：产品数据没有"热点编号"实体，因此 3D 圆环内不画数字（样页的「实心编号圆」在无编号数据时退化为实心圆）；编号/状态的文字载体是列表状态牌——「热点 N」（N=该部件真实热点数）、「候选热点 N（待复核）」、「无热点」、「仅文本条目」。
- **列表/步骤/引用部件选中**：`aria-current` + 底 `--color-selected` + accent 边框 + 「已选」文字牌（三重表达，不只靠颜色）。
- **无热点**：模型区不画任何标记（调用方过滤 stale/unbound/不匹配锚点），文字提示「暂无定位」；不画装饰连线。
- 复核工作区候选条明确「虚线空心环标记，与已确认的实心圆明显区分」，并保留「确认全部候选热点」的真实 PATCH 路径（确认后候选→已确认，3D 数据与列表牌同步变化）。

### 3. 降级实现（AC-VS-006）

沿用既有状态机（`fetching/noModel/unavailable/lost/restoring/error`，`ViewerPanel.tsx` 未改动逻辑）；本片只做展示层 token 化与验证：模型容器内网格与状态文案、错误面板「重试加载/改用文字阅读」、`role=status` 状态行。浏览器实测（真实后端 + 注入）：WebGL 关闭（`getContext("webgl*")` 返回 null）时显示真实原因、不挂载画布、部件/步骤/原件全部可读、焦点交回文字替代入口；模型 500 时显示可读原因 + 重试/文字入口，不出现"已生成成功"假状态。

### 4. 独立 HTML 迁移方式（新导出 vs 历史不动）

- **新导出**：`styles.ts` 内联 tokens 取值（无网络字体/图片/CDN，`--duration-*` 减少动效归零）；页头可展开「版本信息」（发布版本 + 发布时间，textContent 写入）；部件状态牌「热点 N/无热点」；热点与选中色 accent/accent-active、场景底色纸面；工具条/文本/面板尺寸与应用阅读器一致。
- **历史不动**：`build-html.ts` 只影响此后新生成的 HTML；本次未触碰任何既有导出文件与发布资产（哈希证据见 §5）。旧导出文件继续使用其自带的内联样式（自包含）。
- 断网实测：`file://` + `offline:true` + 阻断一切 http(s) → 状态行/部件/步骤/规格/版本信息/联动全部可用，**0 个 http(s) 请求、0 个页面错误**。

### 5. 命令与结果（实际执行）

工作目录 `apps/web`（Node v26.0.0 / npm 11.12.1；Playwright 1.60.0 + 系统 Chrome 154，`PLAYWRIGHT_BROWSERS_PATH=<repo>/var/playwright-browsers`）。

| 命令 | 结果 |
| --- | --- |
| `npm run typecheck` | exit 0（无输出） |
| `npm run lint`（eslint . --max-warnings=0） | exit 0 |
| `npm run test` | **50 文件 / 334 用例全部通过**，exit 0（VS-02 后基线 48/322：本片 +1 文件 `hotspot-markers.test.ts`（5 例）与 `ReleaseReaderPage.test.tsx`/`build-html.test.ts` 各 +1 例 → 49/329；其后用户并行流新增 `PartsFullscreen.test.tsx`（+1 文件/+5 例）→ 50/334，全部通过） |
| `npm run build` | exit 0；主入口 `index-DuQWmqNL.js` 413,006 B + `index-BjimTgB9.css` 68,598 B，`cat|gzip -9` = **136,727 B**（与 VS-02 后 136,397 B 相比 **+330 B**，≤10 KiB 上限，为 VS-04/VS-05 留余量）；哈希 JS `a6c50093…`、CSS `882f35b1…`。**口径说明**：VS-02 测量点之后并行流还改过 viewer/standalone（interactive/useInteractive/PartsExplorer/分件全屏等），故 +330 B 是"VS-02 测量时点→现在"的上界，包含并行流增量；VS-05 以同一公式复测（本片两次测量 136,744→136,727，中间只有并行流的 parts-explorer 全屏规则） |
| `sh var/ui-feedback-3-6-vs03/run-vs03.sh`（测试构建后端 `cargo build -p everything-manual --features job-failpoints` + 本机 LocalFixture 供应商 + 临时 data-dir + Vite dev；假密钥、零付费） | **10/10 通过**（`run-final.log`，1.4m）：AC-VS-005 阅读器联动 / 候选×已确认 / 旧版本无热点；AC-VS-006 WebGL 不可用 / 模型 500；AC-VS-009 键盘与减少动效；AC-VS-010 四断点；AC-VS-011 新导出断网打开 + 历史不回写；发布后草稿不被阅读器读取；原件阅读与版本列表 token 核对 |
| VS-02 冒烟回归（`var/vs02-smoke`，6 用例，静态构建 + fixture；无后端） | **6/6 通过**（tokens/资料库/登录/详情/断点/触控未回退） |
| 阅读器发布回归（既有 QA 套件 `tests/e2e/release-review-reading.spec.ts` + `playwright.release-review-reading.config.ts`，`EM_E2E_SERVER_BINARY=target/debug/everything-manual`） | **1/1 通过**（`regression-release-review-reading.log`，48.6s）：UI 修订 → 发布 A/B → 各自文本/出处/ID/字节保持、阅读器不读草稿 |

浏览器实测要点（原始证据见 §6 索引）：

- 阅读器（1440）：三个已确认热点 → 桥 `anchors` 三条 `status=confirmed`；部件状态牌「热点 1」/「仅文本条目」；点选 → `aria-current` + 「已选」+ 提示「已在 3D 中定位部件…」；步骤前进/后退与引用部件 chip 选中一致；出处打开第 1 页并「返回出处」焦点回到触发按钮；全程 0 外部请求、fixture 供应商计数不变（300ms 内亦无新增）。
- 复核（1440）：候选部件牌「候选热点 1（待复核）」警示软底、已确认牌「热点已确认 1」成功软底；候选条含「虚线空心环标记」且不再出现「橙色」；桥 `anchors` = `[candidate, confirmed, confirmed]`；「确认全部候选热点」→ 真实 PATCH 200 → 全部 `confirmed`、候选条消失。截图 `review-1440-candidate-vs-confirmed.png` 与人工裁剪 `evidence-candidate-vs-confirmed-markers.png` 可见 **实心橙圆 ×2 与 白底虚线环+问号 ×1 并存**。
- 四断点：`scrollWidth == innerWidth`（1440/1024/768/375，`responsive-matrix.json`）；375 下「复位视角」「部件」「步骤与原文」「部件行」≥44×44；正文无 ellipsis/截断。
- 独立 HTML（AC-VS-011）：新导出 `vs03-export.html`（sha256 `9e8016fc…`，667 KB）断网打开全部可用；导出前后发布 manifest 哈希与全部冻结资产哈希一致（`manifestSha256Stable: true`）；仓库内 3 个既有导出 HTML 哈希逐一未变（`export-verification.json`）；HTML 内除 three.js 源引用（`https://jcgt.org/...`）与 XML 命名空间（`http://www.w3.org/1999/xhtml`）两个字面量外无 http(s)，无 `<link>`/外链 `src`/`@import`/`url(`。

### 6. AC 覆盖映射与证据索引

| AC | 实现处 | 证据 |
| --- | --- | --- |
| AC-VS-005 热点/步骤/出处联动、模型与列表选中一致、候选×已确认可区分、原件返回上下文 | §2 全部实现点；`hotspot-markers.ts`（语法）、列表状态牌、「已选」语义 | `run-final.log` 用例 1-3；`reader-linkage.json`；截图 `reader-1440-part-selected.png`、`review-1440-{candidate-vs-confirmed,after-confirm}.png`、`evidence-candidate-vs-confirmed-markers.png`、`reader-1440-older-release-no-hotspots.png`；单测 `hotspot-markers.test.ts`、`ReleaseReaderPage.test.tsx` |
| AC-VS-006 WebGL 关闭/模型失败：真实原因 + 恢复入口，文字可用，无假成功 | 既有状态机 + token 化展示层 | `run-final.log` 用例 4-5；截图 `reader-1440-webgl-unavailable.png`、`reader-1440-model-error.png` |
| AC-VS-008（阅读器/复核显式操作部分）候选确认、解绑/重绑/回归的显式动作样式与文案 | 候选条/状态牌/「已选」/stale 区块 token 化；确认动作仍为显式 PATCH | `run-final.log` 用例 2（真实 PATCH 200）；既有 `qa-pc02b-browser.spec.ts`/`qa-t19-independent.spec.ts` 语义未改（未重跑，QA 可定向回归）；表单/发布问题明细等属 VS-04 |
| AC-VS-009 键盘与减少动效 | skip link 首 Tab；中部标签方向键；窄屏单抽屉 Esc + 焦点回归；`--duration-*` 归零 | `run-final.log` 用例 6；截图 `reader-375-drawer-reduced-motion.png` |
| AC-VS-010 四断点无横向滚动、触控 ≥44、正文不截断、宽屏三栏 | theme.css 阅读器布局 + token 触控名单 | `run-final.log` 用例 7；`responsive-matrix.json`；截图 `reader-{1440,1024,768,375}.png`、`document-reader-375.png` |
| AC-VS-011 新导出断网一致、无新增网络请求、旧发布资产/HTML 不变 | §4；`styles.ts`/`build-html.ts`/`viewer/main.ts` | `run-final.log` 用例 8；`export-verification.json`；`standalone-offline-1440.png`；单测 `build-html.test.ts` 新增断言 |

### 7. 偏差、风险与给 VS-04/VS-05 的衔接

1. **3D 标记无数字**（样页「实心编号圆」→ 实心圆）：数据没有"热点编号"实体，画数字即伪造编号；文字载体为列表状态牌「热点 N」（N=真实热点数）。若 UI/PM 要求数字，需要先定义真实编号来源（退回 PM/UI 更新基线）。
2. **阅读器宽屏列宽**：样页参考 264/340，实现取 240/340，原因是应用外壳常驻 232px 侧栏，1280 视口下需给中央模型留出可用宽度；中档（768–1279）保持标签、窄屏保持单抽屉（由 PageLayout 承担，未改）。
3. **导入/任务/设置与版本列表的表单类状态**（发布问题明细、未保存保护之外的版本行态）未抢做，属 VS-04；本片只改了版本列表的展示层 token。
4. 并行流仍在改 `PartsExplorer`/`parts-explorer.css`（分件全屏观察）；本片对其只做 token 迁移，其后续规则若引入旧值，归属该流。
5. 本片浏览器证据为 Chrome 154 + SwiftShader（软件渲染）；真实 GPU、200% 缩放实操、对比度逐对实测留 VS-05。
6. 未重跑的既有套件：`npm run test:e2e` 全量（含 `qa-pc*.spec.ts`）与 `interaction-a-qa.spec.ts`（键盘/抽屉/字号阈值同源断言已由本片 VS-03 套件覆盖同等主张）留 QA 定向回归；阅读器 DOM/文案保持兼容（元素 id/文案/testid 未删改，仅新增状态牌与状态类），`release-review-reading.spec.ts` 已复跑通过（§5）。

### 8. llmdoc 更新与新增非代码知识

本回合更新：**本文件新增 VS-03 小节**。未修改 PRD、tokens/design 基线、QA 报告、`state.yaml`、`decisions.md`、`validation-release.md`。

新增非代码知识（供 QA/VS-04/VS-05 复用）：

1. **热点标记的冻结语法与"无编号不伪造"**：候选＝虚线空心环＋问号、已确认＝实心圆、选中＝额外 3px 外环；3D 标记是画布贴图 Sprite（随模型等比、`depthTest:false`），桥 `anchors()` 增加 `status` 作为可观察证据。
2. **测试陷阱（WebGL 画布）**：`viewer-harness.canvasHasPixels()` 只适用于 2D canvas；WebGL canvas 上 `getContext("2d")` 返回 null，永远为 false，改用"帧数持续增长 + 截图"作为渲染证据。
3. **测试陷阱（形态变体注入）**：`route.fetch()` 会命中页面源站（Vite 代理指向保留端口时 ECONNREFUSED），要注入"真实响应的变体"必须在 route 处理器内用 APIRequestContext 从真实后端取响应再 `fulfill`。
4. **测试陷阱（断点切换的布局重渲染）**：`setViewportSize` 跨 768/1280 后 React 会重挂面板；紧接着的 `boundingBox()` 可能瞬时 null、点击可能被"断点切换后自动打开的抽屉"遮挡。做法：几何读取用 `expect.poll`，抽屉先 Esc 关闭再点击（QA 的 interaction-a 套件同源经验）。
5. **服务端实体 id 是内容派生的**（`part-<hash>`/`step-<hash>`）：e2e 造数在 PATCH/发布前必须按名称/标题回读真实 id，不能预写供应商局部 id。
6. **发布不变量在读者侧的推论**：合法发布版本里每个非「仅文本条目」部件必有 confirmed 热点，因此阅读器的「无热点」只出现在旧 manifest（无 hotspots）路径或被拦截注入的形态；测试里用"删除 hotspots 字段"复现旧版本形态。
7. 独立 HTML 的字面量豁免：three.js 打包内含 `https://jcgt.org/published/0007/04/01/`（论文引用）与 `http://www.w3.org/1999/xhtml`（命名空间）两个字符串常量，不发起请求；"无外链"验收应检查标签属性/`url(`/`@import`/运行时请求，而非全部字面量。

## VS-04 · 表单、任务、设置与发布列表（2026-10-05，RD 自检完成，待独立 QA）

依据视觉 PRD 修订 1 / UI 修订 1；接续 Claude 已通过的 VS-03。协调由 Codex 主会话负责，本节仅为实现和自检记录，不代表 AC-VS-007/008/009/010 已验收通过。

### 实现范围

- 导入四页新增同语法眉题；说明书、照片、准备、确认使用 960px 同宽主表单。五步导航以真实步骤顺序编号，当前步保留 `aria-current="step"`，服务端状态在标题下完整显示。状态字从 10.5px 调整为 token 12/18px；禁用步骤保留原因、不降透明度。
- 上传/视图排列：文件选择按钮 44px；排列卡片、槽位、拖拽选中、元信息迁移到纸面、墨色、选中和 2/4px 圆角 tokens。拖拽仍复用原始函数，减少动效时过渡时长为 0。备注允许换行；照片和 PDF 字节不加滤镜。
- 准备与预算确认：准备记录和进度分区 2px 面板，单选选中同时有边线、底色与原生圆点；记录 ID 为 12/18px 等宽辅助字，费用“已消耗”和保守上界辅助标签为 12/18px。准备中断、补缺页、封存、云端发送范围、预算单位、确认/报价/生成门禁和未知提交恢复函数均未改写。
- 任务：桌面任务行独立状态列、手机纵向排列；阶段按已有数组顺序编号，状态文本保持原 `stageStatusLabel`，展示变体复用 `jobStatusMeta`。未决预留与对账使用警示语义，成功/失败有独立状态牌。恢复按钮不新增付费调用。生成结果的已选部件同时有边线、底色、`aria-pressed` 和“已选”文字，候选热点牌为警示态；未改 viewer 和模型数据。
- 设置：供应商卡片、输入边界/错误/焦点/禁用、当前配置规格栏与恢复/重启说明迁移到 tokens。待重启为警示，“当前运行配置已生效”保持中性并保留“连接未验证”。保存按钮改用现有统一主按钮类，增加与 `aria-describedby` 关联的常驻禁用原因。密钥输入、保存合同、保护与不缓存逻辑保持不变。共用离开确认弹窗改为 2px 面板与单层浮层阴影。
- 发布列表：已发布牌采用成功语义；列表读取失败保持页标题与返回入口，增加显式“重新读取发布版本”按钮（原查询的只读 `refetch`，不触发生成或发布）。资料包下载、不可变版本信息和历史资产不变。
- 主按钮按 AC-VS-010 统一为至少 44×44px（设计 token 桌面默认控件仍为 40px，次要紧凑按钮保持既有允许值）；手机表单/任务控制及恢复入口补齐触控规格。未增加依赖、网络字体或图像。

生产代码共 11 个文件：`theme.css`、`styles.css`、导入 `DocumentStepPage`/`ViewsStepPage`/`PreparePage`/`ConfirmStepPage`、任务 `JobDetailPage`/`JobStageList`/`GenerationResultPage`、设置 `ProviderSettingsForm`、版本 `ReleaseListPage`（均在 `apps/web/src/`）。本轮独立于已有大量用户/Claude 改动；与 VS-03 冻结源的逐文件差分及 SHA-256 见 `var/ui-feedback-3-6-vs04-rd/{vs04.diff,source-hashes.json}`。未改 state、PRD/design、后端、viewer/standalone、部署文件，未提交/推送。

### 实际执行的自检

工作目录仓库根；Node v26.0.0 / npm 11.12.1；未执行 `npm ci`。日志在 `var/ui-feedback-3-6-vs04-rd/`。

| 命令 | 结果 |
| --- | --- |
| `npm --prefix apps/web run typecheck` | exit 0，`typecheck.log` |
| `npm --prefix apps/web run lint` | exit 0，0 warning，`lint.log` |
| `npm --prefix apps/web run test` | exit 0，50 文件 / 336 用例全部通过，`vitest.log` |
| `npm --prefix apps/web run build` | exit 0，`build.log`；无新增依赖，已有 viewer/PDF 大 chunk 提示仍存在 |
| `git diff --check`（本轮产品文件） | exit 0 |
| `cat dist/assets/index-*.js dist/assets/index-*.css \| gzip -9 \| wc -c`（目录 `apps/web`） | **137,985 B**；相对原改版前 135,368 B **+2,617 B**，在 +10 KiB 限额内。主入口 `index-M0ECeW8g.js` / `index-COiBTOGV.css`，确切字节与 SHA-256 在 `build-summary.json`；最终 VS-05 仍需对当时版本复测 |

本轮未用自测冒充独立浏览器 QA，未执行真实供应商收费请求。独立 QA 需从当前冻结源验证：上传失败、准备中断/412、过期报价、确认未完成、提交未知、任务部分成功、设置未保存/冲突/密钥保护、版本列表与下载错误、键盘/窄屏/真实 200% 缩放及实际对比度。VS-03 的登记偏差和最终完整发布检查仍由 VS-05 处理。

### 新增非代码知识与交接

1. `ProviderSettingsForm` 原用 `button--primary`/`button--secondary`，全站主题只定义 `button-primary`/`button-secondary`，导致保存配置未采用统一主操作视觉。本轮改类名，没有修改保存请求或门禁。
2. `.75rem` 在本主题根 14px 下为 10.5px，`.85rem` 为 11.9px；辅助信息应直接引用 12px token。实际问题位于步骤状态、准备记录 ID、费用附注，不应通过全站提高根字号掩盖。
3. 待重启/结果未知属于警示而非成功；“当前配置已生效”不等于已验证供应商连通，所以设置状态保留中性与原“连接未验证”提示。
4. 代码写入于上述自检后停止，交给主会话安排独立 QA；后续失败修复由主会话重新派发，避免与 VS-05 同时写入。

### QA 回合 4 返工 1 · 窄屏控件尺寸（2026-10-05）

独立 QA 在冻结源发现 375px 下准备/确认原件下拉为 335×41px、复核“刷新草稿数据”为 72×18px，未达到 44px 触控高度。返工只改 `theme.css` 的 `<768px` 媒体规则：输入/选择/多行输入最小 44×44px，复核页链接式按钮最小 44×44px 并垂直居中；≥768px 的紧凑值保留。没有修改任何业务请求、状态、字段、viewer/standalone。

实际自检：typecheck、lint、build、diff-check 全 exit 0。Chrome 154.0.8037.93 隔离 CSS 几何脚本（不是独立 QA 或真实路由验证）在 375/767px 测得两原生下拉均 44px 高、刷新按钮 44px 高；768/1440px 原生下拉仍 41px 高，桌面紧凑值未变。证据在 `var/ui-feedback-3-6-vs04-rd/fix1-geometry.{mjs,json,log}`；返工差分/完整源 SHA-256 清单在 `vs04fix1.diff`、`vs04fix1-source-hashes.json`。最新主入口 gzip 为 **137,992 B**，较改版前 +2,624 B。未重复运行不涉及 CSS 的 Vitest（此前 50/336 通过），真实页面复验由 QA 从新冻结源执行。返工自检完成后再次停止产品代码写入。
