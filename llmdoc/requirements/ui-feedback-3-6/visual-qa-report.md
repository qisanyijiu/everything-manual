# ui-feedback-3-6 · 视觉流 QA 验收报告

结果：**FAIL**（1 个未关闭验收缺陷 BUG-VS02-001；其余本回合必选 AC 通过）· 回合：1 · PRD 修订：1（UI 修订 1）· 范围：VS-02 切片（AC-VS-001/002/003/004/009/010 的外壳/登录/资料库/物品详情对应部分）
日期：2026-10-04。依据：`visual-style-prd.md` 修订 1、`design/vs-01/`（tokens.css、component-states.md、样页）、RD 交付 `visual-implementation.md`。QA 不修改生产代码；缺陷交 RD 修复后由 QA 复验（回合 2）。

> 回合 2 更新（2026-10-04）：BUG-VS02-001 复验 **CLOSED**，本切片最近结论 **PASS**（prd_revision=1 / ui_revision=1）。复验证据与影响判定见「回合 2 复验（BUG-VS02-001 + AC-VS-003 复核）」一节；回合 1 内容原样保留。
>
> 回合 3 更新（2026-10-05）：VS-03 切片验收 **PASS**（0 个未关闭缺陷；2 项非阻断偏差登记 N-VS03-1/N-VS03-2）。结论、逐 AC 证据与限制见「回合 3（VS-03 切片验收）」一节；回合 1/2 内容原样保留。

## 环境与交付版本

- 代码版本：git `3be4d3b`（master）+ RD 未提交的 VS-02 前端改动（`apps/web/src/theme.css`、`features/library/LibraryPage.tsx`、`features/library/ItemOverviewPage.tsx`、`components/Icon.tsx`，`git diff --stat` 共 4 个前端文件）。
- 后端：release 二进制 `target/aarch64-apple-darwin/release/everything-manual`（2026-10-04 19:41 构建，sha256 `66bece17d5aec5a462e8177a95fe422377d15c6710c04f6b0f33ef0793c34d12`）。VS-02 未改 Rust，二进制仅提供真实 API 与临时 data-dir；供应商配置 `base_url=http://127.0.0.1:1` + 套件专用假密钥，**无任何真实收费调用**。
- 前端：Vite dev（`npm run dev`，`/api` 代理到测试后端）从当前工作树加载；Playwright 1.60.0，系统 Chrome **154.0.8037.93**（channel `chrome`），`PLAYWRIGHT_BROWSERS_PATH=<repo>/var/playwright-browsers`；Node v26.0.0 / npm 11.12.1。
- 数据隔离：临时 data-dir `var/ui-feedback-3-6-vs02-qa/work/data`（自动 init，干净库 + 测试种子物品）；未接触用户资料库。
- fixture 与真实后端分开：真实套件 `playwright.qa.config.ts`（真实二进制 + 临时 data-dir）；fixture 套件 `playwright.qa-fixture.config.ts`（无后端，`/api` 代理指向保留端口 127.0.0.1:1，全部路由级 mock + 兜底 404）。
- 未验证项见「未覆盖边界与限制」；真实 Provider 不属本切片（无付费调用）。

## 必执行命令结果（apps/web 下；原始日志 `var/ui-feedback-3-6-vs02-qa/checks.log`）

| 命令 | 结果 |
| --- | --- |
| `npm run typecheck` | exit 0 |
| `npm run lint`（eslint . --max-warnings=0） | exit 0 |
| `npm run test` | **46 文件 / 311 用例全部通过**，exit 0（与 RD 声称一致；QA 脚本已用 `*.pw.ts` 命名避免被 vitest 收集，见 §陷阱） |
| `npm run build` | exit 0 |
| Playwright 真实套件（QA 自编 11 用例） | **9 通过 / 2 失败**（2 个失败同为 BUG-VS02-001）`real-suite-final.log` |
| Playwright fixture 套件（QA 自编 8 用例） | **8 通过** `fixture-suite-final.log` |
| 冷启动复跑 RD-ROW 守卫 ×3（每次清 `node_modules/.vite`） | **3×（2 用例）全过**，未复现 RD 披露的一次性失败 `cold-start/cold-{1,2,3}.log` |

### AC-VS-012 体积独立复算（口径=design/vs-01/README §2）

- 改后：`index-Cw1auV5E.js` 412,726 B + `index-HnZaNl2F.css` 64,729 B，`cat|gzip -9` = **136,397 B**（与 RD 声称一致；sha256 JS `f25cf75d…`、CSS `d6d1c932…`，同 RD）。
- 改前基线独立重建：把 HEAD 六个关键文件（theme.css / LibraryPage / ItemOverviewPage / Icon / package.json / package-lock）逐一哈希核验与 HEAD 一致的干净副本（`/tmp/em-vs02-baseline`，非仓库工作树）构建 → `index-PvaA1NtB.js` 412,060 B + `index-BViv6ey9.css` 55,425 B，gzip = **135,368 B**（同 RD 基线；哈希 `eea038ed…`/`8a2db500…`）。
- **增量 +1,029 B ≤ 10 KiB**；`git diff -- apps/web/package.json apps/web/package-lock.json` 为空（无新增依赖）；构建 CSS 无 `url(`；dist 字体仅 pdfjs vendor `standard_fonts`（既有 PDF 资产）。

## AC 验收矩阵（本回合范围）

| AC ID | 期望（修订 1 文本/冻结合同） | 实际 | 判定 | 证据 |
| --- | --- | --- | --- | --- |
| AC-VS-001 | 同一套设计变量；面板无发光/装饰渐变/多层阴影；圆角、编号、网格符合规格 | 资料库/详情/登录计算样式：面板与列表 2px + `box-shadow:none`；按钮/输入 4px；输入边界 `rgb(125,129,118)`；编号牌 2px + 等宽 12px；全页装饰扫描（background-image / box-shadow）0 违规；无网格铺在正文 | **PASS** | `real-suite-final.log`（AC-VS-001/002 两用例）；截图 `qa-library-1440.png`、`qa-detail-1440.png`、`qa-login-1440.png` vs `library-specimen.html`（`qa-specimen-library-1440.png`） |
| AC-VS-002 | 正文 ≥14px、必读辅助 ≥12px；正文对比 ≥4.5:1，控件边界/焦点 ≥3:1；状态有文字 | 字号：h1 28/36（375px 实测 24/32）、h2 18/26、正文 14（列表名称行高 22.4 ≥22）、辅助/编号 12/18；全页 <12px 扫描为空。对比实测见下表；状态牌/任务入口/错误态均有文字 | **PASS** | 同上 + `smallTextSamples` 全页扫描 |
| AC-VS-003 | 登录→资料库→详情→返回；页标题/当前位置/主操作/返回可辨；导航名称图标一致；路由可直接刷新 | 页面链路、top-bar 页名与上下文、主操作（补齐资料/编辑/归档/生成历史）、返回资料库、导航快照一致性、/items/{id} /settings /jobs 直接刷新均通过；**「当前位置」在 /items/* 路由缺 `aria-current="page"`（仅视觉选中）→ 失败** | **FAIL** | `real-suite-final.log`（AC-VS-003 用例第 178 行失败，34 次轮询 null）；`aria-matrix.log` 全矩阵；`playwright-output/…AC-VS-003…/error-context.md` |
| AC-VS-004 | 有/无封面、长型号、缺字段、空库、搜索无结果、分页、请求失败 | 全部通过：空库「还没有物品」+新建入口；无结果「当前范围没有匹配物品」；失败保留上次结果 + 诊断 ID + 重试（无 0 件/空库文案）；重试保留 `q=` 筛选；游标 422 明确提示+回开头；分页只计已加载（20→total，URL 游标）；缺字段列表「未提供型号」/详情 3 处「未提供」；长型号无截断无溢出；无封面本地立方体图标（`img` 计数 0）；归档范围独立（默认不混入，勾选后「已归档」牌） | **PASS** | `real-suite-final.log`（2 用例）+ `fixture-suite-final.log`（4 用例）+ 截图 `qa-library-mixed/archived/page1/page2/…`、`qa-fixture-empty/failure/cursor-422/missing-model*` |
| AC-VS-009 | 纯键盘与减少动效 | skip link 为首个 Tab 目标（聚焦可见 3px/offset3px/`rgb(166,63,33)`/solid）；Enter 落点 `#main`；Tab 顺序正常；详情「返回资料库」键盘可达；375 抽屉 Enter 打开、Esc 关闭、焦点回触发按钮；`prefers-reduced-motion` 下 `--duration-*` 全 0ms 且按钮 transition 0s | **PASS** | `real-suite-final.log`（AC-VS-009 用例）；`qa-keyboard-skip-link.png`、`qa-keyboard-drawer-375.png`、`qa-fixture-skeleton-reduced-1440.png` |
| AC-VS-010 | 375/768/1024/1440 + 200% 缩放；无横向滚动；主操作 ≥44×44；正文不截断；固定导航不遮挡 | 四档 `scrollWidth == innerWidth`；375：底栏各项 ≥44，新建 116×44、行主操作 90×44、搜索按钮 62×44、输入 335×44，行单列堆叠，滚到底最后一行不被底栏遮挡；768/1024/1440 主操作高 ≥40（按 `--control-height` 桌面档与 component-states §2）；正文 `text-overflow:clip` 且 scroll≤client；200% 等效（720 CSS px + DSF2）无横向滚动、底栏 ≥44、内容不被遮挡 | **PASS** | `real-suite-final.log`（2 用例，含 `[QA 四档几何]` 原始 JSON）；截图 `qa-library-{375,768,1024,1440}.png`、`qa-library-zoom200.png`、`qa-detail-zoom200.png` |

### AC-VS-002 对比度实测（浏览器计算样式 + WCAG 2.x 公式，QA 独立计算，不采信设计期静态表）

| 前景 / 背景 | 实测 | 要求 | 结果 |
| --- | --- | --- | --- |
| 正文墨色 / 页面 | 13.58:1 | ≥4.5 | 通过 |
| 辅助字 / 内容底 | 5.81:1 | ≥4.5 | 通过 |
| 引导字 / 页面 | 5.20:1 | ≥4.5 | 通过 |
| 白字 / 主操作实底 | 6.26:1 | ≥4.5 | 通过 |
| 成功文字 / 软底 | 6.06:1 | ≥4.5 | 通过 |
| 选中文字 / 选中底 | 11.99:1 | ≥4.5 | 通过 |
| 控件边界 / 内容底 | 3.94:1 | ≥3 | 通过 |
| 选中边线 / 选中底 | 4.89:1 | ≥3 | 通过 |
| 登录禁用字 / 禁用软底（`opacity:1`） | 4.88:1 | ≥4.5 | 通过 |
| 焦点环色 / 页面（独立计算） | 5.54:1 | ≥3 | 通过 |

## 缺陷

### BUG-VS02-001 · 侧栏「资料库」在 /items/* 路由保持视觉选中但缺 `aria-current="page"`

- 严重度／状态：**P2 / OPEN**（当前切片必选 AC-VS-003 的「当前位置可辨」非视觉语义不满足，阻断切片 PASS）
- 对应 REQ / UI / AC：REQ-VS-002、REQ-VS-006；UI-VS-001（component-states.md §1「选中」冻结态：`aria-current="page"`；明确「资料库在物品相关路由保持选中」）；AC-VS-003
- 环境与输入：真实后端 + 临时 data-dir；Chrome 154；任意 `/items/{id}` 或 `/items/new`（物品相关路由）
- 复现步骤：登录 → 打开任意物品详情（或访问 `/items/new`）→ 读取 `.sidebar-nav a.active` 属性
- 期望：选中项带 `aria-current="page"`（边线/底色/加粗三重表达 + 语义标记）
- 实际：`aria-current` 为 `null`（三重视觉在，语义缺）；`/`、`/jobs`、`/settings` 均有 `aria-current="page"`。实测矩阵：`/items/01a10770…` 与 `/items/new` → `ariaCurrent: null, borderLeftWidth: 3px, background: rgb(238,226,207), fontWeight: 600`
- 证据路径：`var/ui-feedback-3-6-vs02-qa/real-suite-final.log`（AC-VS-003 用例失败，location 178）、`var/ui-feedback-3-6-vs02-qa/aria-matrix.log`（5 路由矩阵 JSON）、`var/ui-feedback-3-6-vs02-qa/playwright-output/vs02-real.qa.pw.ts-AC-VS-003…/`（error-context.md、test-failed-1.png、trace.zip）、截图 `qa-detail-1440.png`（视觉选中可见）
- 根因摘要（代码级，供 RD 定位）：`AppShell.tsx` 资料库 NavLink 的 `className` 函数以 `isActive || location.pathname.startsWith("/items")` 补 `active` 类；而 react-router v7 的 `aria-current` 仅由 NavLink 自身 `isActive`（`to="/" end`）决定（`node_modules/react-router` NavLink 实现：`ariaCurrent = isActive ? ariaCurrentProp : undefined`），两者不一致。RD `visual-implementation.md §5` 声称「aria-current=page 实测」仅对 `/` 成立，物品相关路由未覆盖。
- 修复建议：为资料库 NavLink 显式传入 `aria-current`（如 `aria-current={isActive || location.pathname.startsWith("/items") ? "page" : undefined}`），或在 className 函数内同步语义。
- 回归范围：修复后复跑 `vs02-real.qa.pw.ts` 的 AC-VS-003 用例与 `vs02-aria.qa.pw.ts` 五路由矩阵；`npm run test`（AppShell 相关用例）；RD 冒烟即可。
- RD 修复摘要引用：RD `visual-implementation.md` §「BUG-VS02-001 修复记录」（`AppShell.tsx` 弃用 NavLink 改 `Link`，新增 `inRouteFamily` 布尔同源驱动三个导航项的 `className` 与 `aria-current`；`shell.test.tsx` 增 5 路由回归用例）
- QA 复验结果与日期：**CLOSED**（2026-10-04，回合 2）——五路由矩阵、AC-VS-003 复核、收紧边界与影响面边界全部通过；回合 1 两处失败断言（`vs02-real.qa.pw.ts` AC-VS-003 用例 L178、`vs02-aria.qa.pw.ts` 探针）同文档原样复跑转绿；证据 `var/ui-feedback-3-6-vs02-qa/{r2-real.log, r2/real-orig-acvs003.log, r2/nav-matrix.json, r2-screenshots/}`

## RD 自披露项判定

1. **hero 区移除（资料库）**：判定**成立、不属缺陷**。PRD §6.2 资料库构图为「标题/实际计数 → 搜索 → 有序档案行」，样页 L-S2 无 hero；实现中 JSX 与 `.library-hero`/`.hero-kicker` 样式均已移除（grep 无残留），`ManualArtwork` 仍用于登录页（PRD §6.2 允许登录短说明+装饰）。实现截图（`qa-library-1440.png`）与样页构图一致。
2. **顶栏任务入口文案（「进行中任务 N 个」vs component-states §1「N 进行中」；失败文案「任务数暂不可用」vs「任务状态暂不可用」）**：判定**语义满足、文案差异按非阻断记录**（见 N-2）。失败不显示 0（fixture 用例实测 `aria-label="任务数暂不可用"`、无「0 个」）。
3. **一次性 Vite 冷启动 e2e 失败**：判定**环境/时序问题，无产品缺陷证据，未复现**。每次 `rm -rf apps/web/node_modules/.vite` 后完整重跑 RD-ROW-1/ROW-2 共 **3 轮全部通过**（含首次冷启动；`cold-start/cold-{1,2,3}.log`）；连同上一回合 QA 复制 spec 的通过记录，未再出现失败。RD 未保留失败现场，无法进一步定根因；保留风险提示：VS-05 全量 e2e 首轮留意同类时序（若再发，需保留 failure 现场）。
4. **`artifacts/web-mvp/t09-rd/e2e-server.log`、`artifacts/web-mvp/t16-rd-fix/` 被 RD e2e 覆写**：判定**新证据有效**。核验方式：
   - `e2e-server.log`（hash `e6d132d1…`，与 QA 快照一致）：为 2026-10-04T14:35Z 的完整成功会话（schema v11 初始化、资产扫描 0 隔离、`manual_ai baseUrl=http://127.0.0.1:1`、正常停止 exit 0），无真实 provider 调用痕迹。
   - `t16-rd-fix` 几何 JSON 内部一致（名称列宽 273/187/237/281/375/462 ≥120；主操作 90×44、历史入口 48×44；无溢出；窄屏单列 227/619）。QA 用**同一 spec 原样重跑 3 次**（`run-t16.sh`），复现值逐一相同（见下）；重跑后证据为新版本（hash `abcf14f7…`/`15a39ef1…`），RD 覆写版已快照留档于 `var/ui-feedback-3-6-vs02-qa/rd-evidence-snapshot/`。QA 自身套件使用独立 evidence 目录，未再覆写 `artifacts/web-mvp/t09-rd/`。

| 宽度 | RD 名称列 → QA 重跑 | RD 主操作 → QA 重跑 | 结尾（无溢出） |
| --- | --- | --- | --- |
| 1024/1280/1366/1440/1600/1920 | 273.3/186.5/237.1/280.6/374.8/461.8 → 逐一相同 | 90×44、历史 48×44 → 相同 | true → true |
| 窄屏 375/767 | 名 227/619，单列 → 相同 | — | true |

## 非阻断建议（独立于必选验收，不计入 PASS/FAIL）

- N-1 资料库搜索提示 `.field__hint` 实测 14px（PC05A 既有规则），token 辅助档为 12px；14px 高于 AC 下限（≥12px）故合规，但偏离 tokens 单一来源。建议 VS-04/05 统一取值，或由 UI 在基线中说明保留 14px。
- N-2 顶栏任务入口文案差异（见上）；若对齐 component-states §1，需同步更新 `JobActivityLink.test.tsx` 与 `qa-pc05c-browser.spec.ts`（RD §7.2 已说明）。
- N-3 中档 768–1279 侧栏 190px 时品牌英文小标省略号截断（保 12px 字号优先）；已复核视觉可接受（`qa-library-1024.png`）。
- N-4 settings/jobs 等页仍留有旧绿色软底与旧圆角（`#eef3e9`、10–16px 等，RD §7.5 清单）；属 VS-04 范围，本回合仅记录不判缺陷。
- N-5 `JobActivityLink` 在响应缺 `data` 字段时渲染抛错（RD §7.8 观察）；本回合复现不到正常契约路径，建议按正式缺陷流程在 VS-04 处理。

## 陷阱与环境发现（已同步 `llmdoc/validation-release.md` §9）

1. **受控 checkbox + 路由 transition 的 Playwright `check()` 竞态**：`显示已归档` 勾选时 `check()` 曾一次报 "Clicking the checkbox did not change its state"，而同期快照显示终态已勾选、归档范围/行/状态牌全部正确 → 是断言时序而非产品缺陷。可靠写法：`click()` + `expect(URL).toMatch(/archived=true/)` + `toBeChecked()`。
2. **fixture 套件必须与真实后端完全隔离**：未 mock 的 `/api` 请求若打到真后端会 401，前端全局「会话失效」会重定向登录页并污染 fixture 结论（回合 1 首次运行实测）。做法：fixture 配置不启动后端（代理指向保留端口）+ `beforeEach` 最低优先级兜底 404。
3. **apps/web 下的 QA 脚本须以 `*.pw.ts` 命名**（不要用 `*.spec.ts`）：`vitest run` 的默认 include 会收集 `*.spec.ts`，导致 `npm run test` 出现额外失败文件（46→49 文件/伪失败 3 个），干扰对 RD 声称 46 文件/311 用例的核对。

## 未覆盖边界与限制

- 不属本回合：AC-VS-005/006/011（VS-03：阅读器/复核/原件/独立 HTML），AC-VS-007/008（VS-04：表单/任务/设置逐区），AC-VS-012 全量与四档截图矩阵、新旧对照（VS-05）。
- 浏览器仅 Chrome 154（本切片口径）；Edge/Firefox/Safari 留给 VS-05 全量按 `validation-release.md §4` 执行。
- 「200% 缩放」采用布局等效条件（CSS 视口 720×450 + deviceScaleFactor 2）而非 Chrome UI 菜单缩放实操；结论覆盖布局滚动/遮挡/触控，不含缩放特有的 UA 行为。
- 对比度为计算样式 + WCAG 2.x 公式自算；未使用外部认证工具，不宣称外部认证。
- 768px 为 `--control-height-touch` 断点边界（<768 为触控档）；本报告对 768 采用桌面档 ≥40px 判定（component-states §2 原文），375 与 200% 缩放采用 ≥44×44。
- 5 秒识别研究未实施（PRD §1 研究目标，非工程门禁）。
- fixture 覆盖的边界（空模型 model=""）为防御状态，真实 API 要求 model 非空白；已在用例内注明。
- RD 冒烟套件（`apps/web/var/vs02-smoke/`）未直接作为验收执行；QA 以上述自编套件独立校验同等主张。

## 回合 2 复验（BUG-VS02-001 + AC-VS-003 复核）

结果：**PASS**（BUG-VS02-001 **CLOSED**）· 回合：2 · 范围：VS-02 切片缺陷复验（prd_revision=1、ui_revision=1，与 `state.yaml` 核对一致）· 日期：2026-10-04。
依据：回合 1 本报告、冻结基线 `design/vs-01/component-states.md` §1、RD「BUG-VS02-001 修复记录」（`visual-implementation.md`）。复验环境与回合 1 同套 harness：release 二进制 `target/aarch64-apple-darwin/release/everything-manual`（sha256 `66bece17…`，与回合 1 一致）；全新临时 data-dir `var/ui-feedback-3-6-vs02-qa/work-r2`；供应商 `base_url=http://127.0.0.1:1` + 套件专用假密钥，**无付费调用**（后端日志 `r2/backend-evidence/e2e-server.log` 仅本机 API 请求）。QA 未修改生产代码、RD 记录与 design 基线。

### 复验方式与独立证据（断言为 QA 自写，不复制 RD 验证）

| 项 | 内容 | 结果 | 证据 |
| --- | --- | --- | --- |
| 五路由矩阵（QA 自写 R2-1） | `/`、`/items/{id}`、`/items/new`、`/jobs`、`/settings`：恰一个视觉选中，且 `aria-current="page"` 与该选中项**同一项**、其余两项无 `aria-current`；三重表达实测 | 5/5 通过 | `r2-real.log`、`r2/nav-matrix.json`、`r2-screenshots/r2-nav*.png` |
| 回合 1 失败探针原样复跑（`vs02-aria.qa.pw.ts`） | 回合 1 两处失败脚本之一；修复前 `/items/{id}`、`/items/new` 读数为 null | 通过（五路由全部 page） | `r2-real.log` L27–69 |
| 回合 1 原始失败用例原样复跑（`vs02-real.qa.pw.ts -g AC-VS-003`，含 L178 断言） | 登录→资料库→详情→返回、页标题/上下文/主操作、（`/items/{id}`、`/settings`、`/jobs`）直接刷新、末尾选中语义 | 1/1 通过 | `r2/real-orig-acvs003.log` |
| AC-VS-003 复核（QA 自写 R2-2，含硬 reload） | 同链路 + `/items/{id}` 硬刷新与重载后语义保持 + 导航身份跨页一致 | 通过 | `r2-real.log`、`r2-walk-*.png` |
| 收紧边界（R2-3） | `/itemsfoo`、`/settingsfoo`、`/jobsfoo`、`/nonexistent` 不落任何路由族（0 选中/0 aria）；NotFound「页面不存在」+「返回资料库」可用；不影响五个验收路由（R2-1 覆盖） | 通过 | `r2-real.log`、`r2-nf-*.png` |
| 影响面边界（R2-4） | skip link 仍为第一个 Tab 目标（3px/offset 3px/rgb(166,63,33)/solid/`:focus-visible`）；导航项仍为 `tabIndex=0` 标准 `<a>` | 通过 | `r2-real.log` |

### 五路由矩阵原始结果（R2-1；Chrome 154）

| 路由 | 视觉选中（恰一） | 选中项 aria-current | 未选中两项 aria-current | 三重表达（边线/底/字重） |
| --- | --- | --- | --- | --- |
| `/` | 资料库 | page | null / null | 3px rgb(166,63,33) / rgb(238,226,207) / 600 |
| `/items/{id}` | 资料库（href=`/`） | page | null / null | 同上 |
| `/items/new` | 资料库（href=`/`） | page | null / null | 同上 |
| `/jobs` | 任务中心 | page | null / null | 同上 |
| `/settings` | 设置 | page | null / null | 同上 |

逐路由 href 与计算样式原始 JSON：`var/ui-feedback-3-6-vs02-qa/r2/nav-matrix.json`。`/items/*` 下选中项 href 为 `/`（资料库入口），与 component-states §1「资料库在物品相关路由保持选中」一致。

### AC-VS-003 复核结论

通过。链路：登录落资料库（标题「资料库」、顶栏页名「资料库」、导航三项名称/顺序/href 快照一致且选中项 aria=page）→ 进入详情（URL `/items/{id}`、标题=物品名、顶栏页名保持「资料库」、`top-bar__context` 含物品名·型号、主操作「返回资料库/编辑/归档」可见、侧栏资料库选中且 aria 一致）→ 返回资料库成功 → `/items/{id}` 硬刷新与 `page.reload()` 后选中语义保持 → `/settings`、`/jobs` 直达刷新页标题正确且选中项语义一致；全程无控制台错误。收紧边界（`/itemsfoo` 等 → NotFound 可用且不选中）不影响五个验收路由。

### 回合 1 已通过 AC 的影响判定

| AC | 是否受修复影响 | 依据 / 复核方式 | 结论 |
| --- | --- | --- | --- |
| AC-VS-001 | 导航选中部分受影响（同文件） | R2-1 五路由重验三重表达计算样式（tokens 冻结值）全部通过；面板/圆角等保留回合 1 证据（`theme.css` mtime 22:31:53 早于修复窗口 23:14，未被触碰） | 不回退（导航部分重验通过）；其余保留 |
| AC-VS-002 | 否 | 修复未改字号/颜色/DOM 文案（diff 仅选中语义；导航文本与图标不变） | 保留回合 1 结论 |
| AC-VS-004 | 否 | `LibraryPage.tsx`/`ItemOverviewPage.tsx` mtime 22:26，修复窗口内未再改 | 保留回合 1 结论 |
| AC-VS-009 | 边界重验（修复直接改导航 DOM 渲染） | R2-4：skip link 首 Tab/焦点环值不变；导航项仍为标准 `<a tabIndex=0>`（NavLink→Link 不改 Tab 语义） | 不回退 |
| AC-VS-010 | 否 | 断点 CSS 与导航类名/结构未变；R2-1 验证的 class/aria 语义与视口无关 | 保留回合 1 结论 |

### 本轮命令结果与归因（原始日志 `var/ui-feedback-3-6-vs02-qa/r2/`）

| 命令（`apps/web` 下） | 结果 | 归因 |
| --- | --- | --- |
| `npm run typecheck` | exit 0（无输出） | 全仓干净；RD 修复回合记录的 viewer TS 错误在此最终时点已不存在（用户并行流已落盘/自解） |
| `npm run lint` | exit 0 | 同 |
| `npm run test`（vitest） | **48 文件 / 322 用例全部通过**，exit 0 | 46/311（回合 1 基线）→ +2 文件/+11 用例：`shell.test.tsx` 19→20（**+1，RD 回归用例**，单独复跑 20/20 通过）；其余 +2 文件/+10 用例属**用户并行流**（`features/viewer/PartsExplorer.test.tsx`、`features/viewer/part-inspection.test.ts` 共 9 用例；`features/standalone/build-html.test.ts` +1 用例），全部通过，无需按失败归因 |
| Playwright 回合 2 真实套件（R2-1～R2-4 + 回合 1 aria 探针） | **5/5 通过**（12.5s） | `r2-real.log` |
| 回合 1 原始失败用例原样复跑（`-g AC-VS-003`） | **1/1 通过**（5.2s） | `r2/real-orig-acvs003.log` |
| `npm run build` | **未执行**（本轮修复仅 TSX 语义与测试；无构建物需求） | — |

### 修复范围核对（未引入范围外改动）

- `git diff` 复核：`AppShell.tsx` 仅 NavLink→Link 与 `inRouteFamily` 布尔同源驱动 `className`/`aria-current`（并移除 NavLink 导入）；`shell.test.tsx` 仅追加 1 个回归 describe/用例。无路由表、CSS、依赖、DOM 结构更改（仍为 `nav.sidebar-nav > a` 三项，文案/图标/href 不变）。
- RD 未触碰 QA 报告与脚本（`visual-qa-report.md` mtime 23:12:06，QA `var/**` 全部 ≤23:11:14，早于修复窗口 23:14+）、未触碰 `design/vs-01/`（22:15–22:19）。`state.yaml`（23:31:43）由协调者更新为回合 2 状态，QA 未写入。回合 1 其余 VS-02 生产文件（theme.css/LibraryPage/ItemOverviewPage/Icon）mtime 22:26–22:31，均早于修复窗口。

### 非阻断观察（回合 2 新增）

- O-R2-1 `/items`（路由族根、无对应页面 → NotFound）在新判定下资料库仍选中（`pathname === base` 分支；与修复前 `startsWith("/items")` 行为一致）。冻结契约只规定「当前页面」与「资料库在物品相关路由保持选中」，未规定 NotFound 的选中态；记录不判缺陷，与 `/itemsfoo` 收紧互不冲突（均不影响五个验收路由）。

### 未覆盖与限制（回合 2）

- `npm run build` 未执行；回合 1 其余 9 个不受影响用例与 fixture 套件未重跑（依据上表影响判定保留，边界项已专项重验）。
- 375px 移动底栏选中态未专项重验（断点 CSS 未改；R2-1 验证的 class/aria 语义与视口无关）。
- VS-03/04/05 范围、真实 Provider（无付费调用）同回合 1 未覆盖；浏览器仅 Chrome 154。

## 回合 3（VS-03 切片验收）

结果：**PASS**（0 个未关闭验收缺陷；2 项非阻断偏差登记：N-VS03-1/N-VS03-2）· 回合：3 · PRD 修订：1（UI 修订 1，与 `state.yaml` 的 `prd_revision=1`/`ui_revision=1` 核对一致）· 范围：VS-03 切片 —— AC-VS-005 / AC-VS-006 / AC-VS-008（片内：阅读器/复核显式操作）/ AC-VS-009 / AC-VS-010 / AC-VS-011 + VS-02 导航回归
日期：2026-10-05。依据：`visual-style-prd.md` 修订 1（§3 REQ-VS-004/005/007、§4 AC 表、§6.2/§6.3、§7 VS-03 行、§9）、冻结基线 `design/vs-01/`（`reader-specimen.html` R-S1～R-S7、`component-states.md` §6/§9/§15、`tokens.css`、README §7 度量口径）、RD 交付 `visual-implementation.md`「VS-03」小节、回合 1/2 本报告、`llmdoc/validation-release.md` §9。基线核对：`visual-style-prd.md`（22:19）与 `design/vs-01/`（22:15–22:19）mtime 均早于 VS-03 实施窗口（23:49–00:06），未被本轮改动。

### 环境与交付版本（QA 全部自建，零付费）

- 代码版本：git `3be4d3b`（master）+ 未提交工作树（VS-03 改动；用户并行流仍在写 `features/viewer|standalone`，见「环境归因」）。QA 未修改任何生产文件、PRD、design 基线、RD 记录与 `state.yaml`。
- 后端：`cargo build -p everything-manual --features job-failpoints`（exit 0；与 RD 自验同一测试二进制，sha256 `9ecbc00b…`，QA 00:08 确认无源码变更）；每用例 `TestBackend` 随机端口 + `mkdtemp` 临时 data-dir，用后删除；供商 `base_url` 指向本机 fixture、假密钥。
- fixture：QA 自写 `QaFixture` 场景（4 部件[3 已确认热点 + 1 仅文本]/2 步骤/1 规格；名称、坐标与文案自定），与 RD 场景独立。
- 前端：Vite dev 端口 15188（与 RD 15187、VS-02 15185/15186 隔离）；Chrome 154（channel chrome）+ SwiftShader；Playwright 1.60.0；Node v26.0.0 / npm 11.12.1。
- 供应商调用观察：用例 1/3/4/5 断言 fixture 计数与基线逐一相等（均为 0 新增）；套件同时把非本机请求一律阻断并记录（用例 1 断言为空）。原件失败、长文、导出、键盘、断点用例未逐一断言计数（其链路只读本地 API），留 VS-05 全量复核。
- 证据目录：`var/ui-feedback-3-6-vs03-qa/`；脚本目录：`apps/web/var/ui-feedback-3-6-vs03-qa/`（`vs03qa-reader.pw.ts`、`vs03qa-helpers.ts`、`playwright.vs03qa.config.ts`、`run.sh`；诊断脚本 `vs03qa-debug.pw.ts` 已 `testIgnore`，不进入验收套件）。

### 命令结果（QA 独立执行）

| 命令 | 结果 |
| --- | --- |
| `npm run typecheck` | exit 0（`qa-typecheck.log`） |
| `npm run lint`（eslint . --max-warnings=0） | exit 0（`qa-lint.log`） |
| `npm run test` | **50 文件 / 336 用例全部通过**，exit 0（`qa-vitest.log`，00:30 时点） |
| `npm run build` + gzip 复算 | exit 0；主入口 `index-D4wrfCvC.js` 413,006 B + `index-BjimTgB9.css` 68,598 B；`cat \| gzip -9` = **136,732 B**（`qa-build.log`） |
| QA 浏览器套件（QA 自编 11 用例，Chrome 154 + SwiftShader） | **11/11 通过**，1.1m，EXIT=0（`run-final.log`） |
| `release-review-reading.spec.ts`（定向重跑） | **1/1 通过**（16.4s；`release-review-reading.log`） |
| `interaction-a-qa.spec.ts`（定向重跑） | **6/7 通过**；IA-QA-06（200% 缩放，`channel: "chromium"` 自带浏览器）因仓库 `var/playwright-browsers` 缺 Playwright 自带 Chromium 而环境失败 → 改用默认浏览器缓存（`~/Library/Caches/ms-playwright` 有 chromium-1223）**单独补跑 1/1 通过**（`interaction-a-qa-zoom.log`）→ 有效 7/7；含 IA-QA-05「阅读器键盘标签/抽屉与手机触控」通过 |
| `delivery-standalone.spec.ts`（离线交互补充，`EM_E2E_EVIDENCE_DIR` 重定向到 QA 目录，未覆写历史交付产物） | **2/2 通过**（48.0s，EXIT=0）；离线证据 `offline-verification.json`：修订事实/安全说明保留、动作点击画布变化 `true`、`httpRequests=0`、`errors=0`（`delivery-standalone.log`） |

### AC 验收矩阵（VS-03 切片）

| AC ID | 期望（修订 1 + 冻结合同） | 实际（QA 独立执行） | 判定 | 证据 |
| --- | --- | --- | --- | --- |
| AC-VS-005 | 有热点/无热点/候选热点的阅读器与复核 fixture；点选部件/步骤/出处：文本与模型选中联动；编号只来自真实实体；装饰不捕获模型点击；候选与已确认可区分；原件可打开并返回上下文 | 阅读器：部件状态牌「热点 1/仅文本条目」来自真实实体（热点 DTO 键集合实测 `{anchor,id,partId,status}`，无编号字段）；桥 `anchors` 3 条 `confirmed`；3D 热点投影点选 → 列表 `aria-current`+「已选」；列表点选 → 提示 + 互斥；步骤上一步/下一步与当前步骤、选中部件、引用 chip 联动；出处打开第 1 页、返回出处焦点回触发按钮；模型容器内 5 个采样点命中元素均为 CANVAS（装饰不拦截）。复核：候选牌「候选热点 1（待复核）」warning、已确认牌「热点已确认 1」success、候选条含「虚线空心环标记」且无「橙色」；桥状态 = `[candidate, confirmed, confirmed]`。标记语法用真实模块画布像素独立验证：confirmed=实心 accent（内部 accent、外圈透明、圆面积 5140 像素）、candidate=surface 内部 + 虚线环（144 采样中 accent 81/间隔 63/通断交替 22）+ 中心问号笔画 346 像素、选中再加外环 72/72 + 纸色隔离带 72/72；未知/stale 状态不进入可显示集合（`usable=false`），不冒充候选 | **PASS** | 用例 1-3；`run-final.log`；`qa-marker-syntax.json`；`qa-review-stage-markers.png`、`qa-review-stage-marker-zoom.png`（目视：实心橙圆 ×2 与白底虚线环+问号 ×1 并存）、`qa-review-parts-plates.png`；截图 `qa-reader-1440-part-selected.png` |
| AC-VS-006 | 关 WebGL、模型加载失败、长文说明、原件加载失败：真实原因 + 可用恢复入口；部件/步骤文字可读；无「已生成成功」假状态；不要求重新付费 | WebGL 关闭（getContext 返回 null）：横幅为真实原因「浏览器 3D 上下文不可用：此环境没有可用的 WebGL…文字与 PDF 阅读不受影响。」、不挂载画布、状态行「请使用文字阅读」、部件/步骤/规格可读、「改用文字阅读」把焦点交回部件面板。模型 500：错误面板含真实 message +「重试加载/改用文字阅读」，撤掉注入后重试**恢复**为已加载；原件仍可读。原件 500：`original-error` 真实原因 +「重新加载原文」，撤掉注入后**恢复**画布；模型与步骤不受影响。长文（500+ 字符无空格）完整换行不截断（1440/375）。全页无「已生成成功/生成成功/重新生成/付费」文案；WebGL/模型降级用例断言供应商计数不变（原件失败与长文用例链路只读本地 API） | **PASS** | 用例 4-7；截图 `qa-reader-1440-webgl-unavailable.png`、`qa-reader-1440-model-error.png`、`qa-reader-1440-document-recovered.png`、`qa-reader-1440-long-description.png`；`qa-webgl-unavailable-texts.json`（原文见 N-VS03-3） |
| AC-VS-008（片内） | 阅读器/复核内的显式操作行为保持（候选确认等真实 PATCH） | 复核页加载后 0 个 PATCH（显式操作才写）；「确认全部候选热点」→ 等待真实 PATCH **200** → 候选牌转「热点已确认 1」、候选条消失、桥全 `confirmed`；服务端草稿热点读取全 `confirmed`；全流程仅此 1 个写请求；供应商计数不变。表单/发布问题明细等属 VS-04 | **PASS**（片内） | 用例 3；`qa-review-1440-candidate-confirmed.png`、`qa-review-1440-after-confirm.png` |
| AC-VS-009 | 纯键盘 + 减少动效：跳转主内容、Tab、标签方向键、抽屉 Esc 与焦点恢复；热点有列表替代；取消非必要过渡 | 首个 Tab = 「跳到主要内容」→ Enter 焦点落 `#main`；中部标签页 `ArrowLeft/Right + Home/End` 切换且 `aria-selected` 同步；部件列表键盘 Enter 选中（`aria-current`+已选）；步骤「下一步」键盘生效；工具条「复位视角」键盘触发不破坏状态；窄屏单抽屉逐次开关 + Esc 关闭 + 焦点归还触发按钮；断点切换后已选面板自动带入抽屉（状态保留）；减少动效下 `--duration-control` 0ms、按钮 `transition-duration` 归零 | **PASS** | 用例 8；`qa-reader-375-drawer-reduced-motion.png` |
| AC-VS-010 | 375/768/1024/1440 无页面级横向滚动；主操作 ≥44×44；正文不截断；工具条固定在模型容器内；§6.2 面板/抽屉规则 | 四档 `scrollWidth == innerWidth`（矩阵 JSON）；1440 三栏 rail 240/main 504/aside 340 且依次不重叠，工具条在面板内且不遮挡画布（四档轮询断言）；1024/768 保留侧栏标签页（3 个 tab、无 dialog）；375 单抽屉、触发按钮/工具条按钮/部件按钮 ≥44×44、描述无 ellipsis/截断；文档阅读页 375 无横向滚动、翻页栏 ≥12px；版本列表行 2px 圆角/无阴影/线色 `rgb(213,209,197)` | **PASS** | 用例 9；`qa-responsive-matrix.json`；截图 `qa-reader-{375,768,1024,1440}.png`、`qa-document-375.png` |
| AC-VS-011 | 新导出独立 HTML 断网打开：色彩/字号/状态与应用阅读器一致；原有模型/步骤/展示动作可用；无新增网络请求；旧发布资产与既有 HTML 不变 | 走真实导出下载新 HTML（sha256 `5347a052…`）→ `file://` + offline + 阻断 http(s)：0 请求、0 错误；状态行/部件（含「热点 1」）/步骤/规格/版本信息（可展开，含 releaseId）/选中态/下一步/复位视角全部可用；**展示动作**（交互层）由 `delivery-standalone` 补充：离线动作点击画布变化 `true`、0 请求 0 错误；标签级外链检查 0（仅 three.js 论文引用与 XML 命名空间两个字面量）；**历史不回写**：QA 自算 3 个既有导出 HTML 哈希（与 RD 记录逐一相同）与发布 manifest+全部冻结资产哈希在导出前后不变。样式对比（计算样式）：纸面/内容/墨色/选中、面板 2px+线色、舞台纸面、aux 12/18、正文 14、部件按钮 14/40、状态牌 12/18、h2 18/26 全部一致；**例外**：离线 h1 26/34 vs 应用页标题 28/36（tokens `--font-size-page-title`）见 N-VS03-1 | **PASS**（附 N-VS03-1 登记） | 用例 10；`qa-export-verification.json`、`qa-standalone-style-compare.json`、`delivery-standalone/offline-verification.json`；`qa-vs03-export.html`、`qa-standalone-offline-1440.png` |
| VS-02 回归 | 回合 1/2 已接受项不回退 | 导航 aria 矩阵 4 路由（`/`、`/items/{id}`、`/jobs`、`/settings`）各恰一个 `.active` + `aria-current=page` + 三重表达（3px `rgb(166,63,33)` / `rgb(238,226,207)` / 600）；skip link 首 Tab + 焦点环 3px/offset 3px/solid；真实用户路径 资料库→物品概览→版本列表→打开阅读器 全程可达；`release-review-reading` 与 `interaction-a-qa`（含阅读器键盘/抽屉/触控）定向重跑通过 | **PASS** | 用例 11；`qa-nav-regression.json`；`release-review-reading.log`、`interaction-a-qa.log` |

### 缺陷

**本回合 0 个**（无 P0–P3 打开缺陷）。

### 非阻断偏差与观察（独立于必选判定）

- **N-VS03-1（唯一量化偏差，待 VS-05/UI 对齐或决例外）**：离线 HTML `h1` 计算字号 **26/34**，应用阅读器页标题 **28/36**（tokens `--font-size-page-title` 桌面 28/36、<768 24/32）。`qa-standalone-style-compare.json` 中 `h1.fontSize/lineHeight = false`，其余 16 项为 true；26px 为迁移前旧值（`git diff styles.ts` 可见），非 token 取值。影响：无可用性/可读性影响（26>24，无障碍合规）；属 token 单一来源的迁移遗漏。建议：VS-05 前把离线 h1 改为 28/36 与 <768 的 24/32；若 UI 要保留文档标题例外，需在基线登记。**QA 判定不阻断 AC-VS-011**（"色彩、字号、状态一致"按内容/状态/系统级字号口径成立，此条单列待决）；若主会话/PM 采用"逐元素字号完全一致"的字面口径，这是唯一需回 RD 的条目。
- **N-VS03-2**：`body` 行高应用计算值 22.4px（1.6 比例，旧基础层）vs 离线 22px（token `--line-height-body`）；名义同为 14/22，差 0.4px，建议 VS-05 顺手对齐（应用侧改 22px 或离线改 1.6）。
- **N-VS03-3**：WebGL 不可用时点选部件仍提示「已在 3D 中定位部件「主机壳体」的热点」，与同页「3D 不可用：请使用文字阅读。」并存（`qa-webgl-unavailable-texts.json` 原文）。非"假成功/重新付费"，属降级态措辞组合瑕疵（P3 级建议：降级时改为"已选择部件「…」"或附带"3D 不可用"限定）。
- **N-VS03-4**：离线「热点 N」状态牌为中性 tag，应用阅读器为 success 软底；`component-states.md` §4 明确软底为可选、文字必须——语义合规，记录备查。
- **N-VS03-5**：断点切换自动打开抽屉后，紧随其后的第一次 Escape 可能落于 Drawer 监听挂载前被吞（子帧级；自动化可见、真实用户不可达）。测试陷阱与做法已记入 `validation-release.md` §9（回合 3 条目 1）。

### RD 自披露偏差判定（逐条）

1. **3D 标记无数字**（样页「实心编号圆」→ 实心圆）：**成立、非缺陷**。REQ-VS-004「标注线/编号只来自现有实体」与 §6.1「不伪造产品规格」优先于 component-states §6 的示意措辞；服务端热点 DTO 实测无编号字段（QA 独立断言键集合 = `{anchor,id,partId,status}`）；模块层 confirmed 标记 `glyph=null`。建议 UI 在下次修订把 component-states §6/样页 R-S4 的「实心编号圆」改为「实心圆（存在真实编号时才画编号）」，属文档对齐。
2. **阅读器宽屏列宽 240/340（样页参考 264/340）**：**合理、非阻断**。QA 实测 1440 三栏 = rail 240 / main 504 / aside 340（`qa-responsive-matrix.json`）；外壳常驻侧栏挤压总宽，中央模型可用宽度优先；PRD §6.2 仅要求「≥1280px 三栏」，样页差异按 README §2「差异登记」处理，RD §7.2 已登记。建议 VS-05 截图对照时把该差异写入对照说明。
3. **gzip +330 B**：**成立（上界口径已披露）**。QA 独立构建两次均 **136,732 B**（JS 413,006 + CSS 68,598）；对 VS-02 后独立基线 136,397 B（回合 1 QA 复算）为 **+335 B**；与 RD +330 B 的 5 B 差归因其测量点之后用户并行流的 viewer 改动；远低于 10 KiB（AC-VS-012 总核对归 VS-05）。
4. **`npm run test:e2e` 全量未重跑**：**可接受，已定向补偿**。QA 重跑 `release-review-reading`（1/1）、`interaction-a-qa`（6/7 + 环境补跑 1/1，含 IA-QA-05 阅读器键盘/抽屉/触控）、`delivery-standalone`（离线交互补充，见上表）。全量 88+ 例仍留 VS-05。

### 环境归因

- 用户并行写入流（viewer/standalone：`PartsExplorer`/`parts-explorer.css`/`PartsFullscreen.test.tsx`，mtime 00:09–00:30）在 QA 期间持续改动；QA 的浏览器证据绑定运行时刻工作树（构建入口 `index-D4wrfCvC.js`，00:30 时点）。`npm run test` 336 例（较 00:08 的 334 例 +2）归因该流 `PartsFullscreen.test.tsx`；typecheck/lint/vitest 在最终时点全部绿。本回合没有失败项需要归因并行流。
- 既有产物影响：运行 `interaction-a-qa` 按其设计覆写了 `artifacts/interaction-a/qa/`（17 张截图，运行前已快照到 `var/ui-feedback-3-6-vs03-qa/pre-regression-snapshot/`）；`artifacts/web-mvp/t09-rd/e2e-server.log` 被 global-setup 重写（运行前快照 hash `e6d132d1…`）。`delivery-standalone` 用 `EM_E2E_EVIDENCE_DIR` 重定向，未触碰 `var/delivery-20261004/`。

### 未覆盖与限制（回合 3）

- 离线**展示动作**（交互层姿势/动作 chips）：QA 场景无交互层；由 `delivery-standalone.spec.ts`（含离线动作 aria-pressed + 画布变化断言）定向补充（见上表）。
- stale 热点的复核 UI「失效（stale）」文本牌未用真实 stale 数据走查；模块级已证 stale 不进入 3D 显示、不冒充 confirmed（`qa-marker-syntax.json`）。
- 真实 200% 缩放（阅读器侧）、对比度逐对复测、Edge/Firefox 矩阵：VS-05（本回合阅读器证据为 Chrome 154 + SwiftShader 软件渲染）。
- 真实 Provider：本切片不适用（所有链路 fixture，计数不变）。
- 供应商计数未在导出/键盘/断点用例逐一断言（见「环境与交付版本」口径说明）。

### 跨任务知识（已更新 `llmdoc/validation-release.md` §9）

1. 断点切换后自动打开抽屉的首次 Escape 监听挂载竞态与「关闭按钮聚焦 = 监听已挂载」判据；2. 断点切换后几何读取要轮询"关系"（含非 null 中间帧实例）；3. Vite dev 页面内动态 import 生产模块做画布像素级视觉语法验证；4. `__EM_VIEWER__.project(local)` 精确点选 3D 热点。以上 4 条证据分别为 `vs03qa-debug.pw.ts`、`qa-responsive-matrix.json`、`qa-marker-syntax.json`、`vs03qa-reader.pw.ts`。



## 回合历史与交接

- 回合 1（2026-10-04，本报告）：范围 VS-02（prd_revision=1、ui_revision=1）。结论 **FAIL**：AC-VS-001/002/004/009/010 通过；AC-VS-003 因 BUG-VS02-001 未通过。命令级检查（typecheck/lint/test/build）、体积增量、真实浏览器走查均已执行；缺陷交 RD 修复后由 QA 复验（回合 2）。
- 交付给 RD 的具体事项：修复 BUG-VS02-001（AppShell 资料库 NavLink 的 `aria-current` 与 `/items/*` 选中逻辑同步）；修复后自跑 `vs02-*` 无关，仅需保证 `npm run test` 与既有 e2e 冒烟不回归，并回报修复摘要。
- 建议主会话 state.yaml 更新（供参考，QA 未写入）：`phase: rd_fixing`（或保持 qa_running 待派 RD 修复），`qa_result: FAIL`，`open_defects: [BUG-VS02-001]`，`qa_history += {round:1, scope:"VS-02 slice", prd_revision:1, result:"FAIL", report:"llmdoc/requirements/ui-feedback-3-6/visual-qa-report.md"}`，`next_action: "RD 修复 BUG-VS02-001 后 QA 回合 2 复验（重点：AC-VS-003 + aria 矩阵）"`。
- 回合 2（2026-10-04，本报告补充；scope=BUG-VS02-001 复验 + AC-VS-003 复核）：结论 **PASS**，BUG-VS02-001 **CLOSED**。五路由矩阵、AC-VS-003 复核、收紧边界（`/itemsfoo` + NotFound 可用）、回合 1 两处失败断言同文档原样复跑全部通过；typecheck/lint/vitest（48/322）全过，测试增量归因见回合 2 小节；回合 1 已通过 AC 按影响判定保留或专项重验（AC-VS-001 导航部分 / AC-VS-009 边界）。
- 回合 2 建议主会话 state.yaml 更新（供参考，QA 未写入）：`qa_round: 2`、`qa_result: PASS`、`open_defects: []`（BUG-VS02-001 CLOSED）、`qa_history += {round:2, scope:"BUG-VS02-001 复验 + AC-VS-003 复核", prd_revision:1, result:"PASS", report:"llmdoc/requirements/ui-feedback-3-6/visual-qa-report.md#回合-2-复验bug-vs02-001--ac-vs-003-复核"}`；可将 VS-02 移入 `accepted_tasks`（accepted_ac_ids = AC-VS-001/002/003/004/009/010 本切片对应部分）；`next_action: "VS-03 派发前核对用户并行流（viewer/standalone）是否仍在写入，避免双写冲突"`。
- 回合 3（2026-10-05，本报告补充；scope=VS-03 切片）：结论 **PASS**，0 个未关闭验收缺陷。QA 自编 11 用例浏览器套件 **11/11**（Chrome 154 + SwiftShader；QA 自写 fixture 与断言、零供应商调用）；AC-VS-005（含候选/已确认标记的画布像素级独立验证与目视截图）、AC-VS-006（WebGL 关闭 / 模型 500 / 原件 500 / 长文，均可恢复且无假成功）、AC-VS-008 片内（真实 PATCH、零自动写）、AC-VS-009（含标签 Home/End、抽屉 Esc+焦点恢复、减少动效归零）、AC-VS-010（四断点、工具条包含关系、触控与不截断）、AC-VS-011（断网 0 请求 0 错误；历史发布资产与既有导出哈希逐一不变）全部通过；回归：typecheck/lint exit 0、vitest 50/336、build gzip 136,732 B（+335 B ≤10 KiB）、`release-review-reading` 1/1、`interaction-a-qa` 6/7 + 环境补跑 1/1（IA-QA-06 自带 Chromium 缺失属环境）、`delivery-standalone` 2/2（含离线交互：动作画布变化、0 请求 0 错误）。2 项非阻断偏差登记（N-VS03-1：离线 h1 26/34 vs 应用 28/36——唯一量化一致项例外；N-VS03-2：行高 22.4/22）。
- 回合 3 建议主会话 state.yaml 更新（供参考，QA 未写入）：`qa_round: 3`、`qa_result: PASS`、`open_defects: []`、`qa_history += {round:3, scope:"VS-03 slice（AC-VS-005/006/008 片内/009/010/011 + VS-02 回归）", task_ids:[VS-03], prd_revision:1, result:"PASS", report:"llmdoc/requirements/ui-feedback-3-6/visual-qa-report.md#回合-3vs-03-切片验收"}`；可将 VS-03 移入 `accepted_tasks`（accepted_ac_ids：AC-VS-005、AC-VS-006、AC-VS-008（片内）、AC-VS-009、AC-VS-010、AC-VS-011；附注 N-VS03-1 待 VS-05/UI 对齐）；`next_action: "VS-04 派发（表单/任务/设置；派发前再核对用户并行流活跃度）；VS-05 收口：N-VS03-1 离线 h1 token 对齐、test:e2e 全量、200% 缩放与对比度复测、AC-VS-012 总核对"`。

## 证据索引

- 报告与日志：`var/ui-feedback-3-6-vs02-qa/{checks.log, real-suite-final.log, fixture-suite-final.log, aria-matrix.log, cold-start/cold-{1,2,3}.log, backend-evidence/e2e-server.log}`
- 回合 2 日志：`var/ui-feedback-3-6-vs02-qa/{r2-real.log, r2/lint-r2.log, r2/typecheck-r2.log, r2/vitest-r2.log, r2/real-orig-acvs003.log, r2/nav-matrix.json, r2/backend-evidence/e2e-server.log}`
- 测试脚本（gitignore）：`apps/web/var/ui-feedback-3-6-vs02-qa/`（`vs02-real.qa.pw.ts`、`vs02-fixture.qa.pw.ts`、`vs02-aria.qa.pw.ts`、`vs02-r2.qa.pw.ts`、`qa-sidebar.qa.pw.ts`、`row-layout.qa.pw.ts`、`qa-helpers.ts`、playwright 配置 ×5、`run-*.sh`）
- 截图：`var/ui-feedback-3-6-vs02-qa/screenshots/`（33 张，含样页对照 `qa-specimen-library-*.png`）；回合 2：`var/ui-feedback-3-6-vs02-qa/r2-screenshots/`（11 张，五路由选中 + 走查 + NotFound）；失败现场：`var/ui-feedback-3-6-vs02-qa/playwright-output*/`
- RD 证据快照与哈希：`var/ui-feedback-3-6-vs02-qa/rd-evidence-snapshot/`；几何复跑：`var/ui-feedback-3-6-vs02-qa/row-geometry/`
- **回合 3（VS-03）**：日志 `var/ui-feedback-3-6-vs03-qa/{run-final.log, run-final.exit, qa-typecheck.log, qa-lint.log, qa-vitest.log, qa-build.log, release-review-reading.log, interaction-a-qa.log, interaction-a-qa-zoom.log, delivery-standalone.log}`；JSON `{qa-marker-syntax.json, qa-responsive-matrix.json, qa-export-verification.json, qa-standalone-style-compare.json, qa-nav-regression.json, qa-webgl-unavailable-texts.json}`；截图 12 张（`qa-reader-1440-part-selected/model-error/webgl-unavailable/document-recovered/long-description.png`、`qa-reader-375.png`、`qa-reader-375-drawer-reduced-motion.png`、`qa-review-1440-candidate-confirmed/after-confirm.png`、`qa-review-stage-markers.png` + 放大 `qa-review-stage-marker-zoom.png`、`qa-review-parts-plates.png`、`qa-document-375.png`、`qa-standalone-offline-1440.png`）；导出样本 `qa-vs03-export.html`；离线交互证据 `delivery-standalone/offline-verification.json`；运行前快照 `pre-regression-snapshot/`；套件脚本 `apps/web/var/ui-feedback-3-6-vs03-qa/`（`vs03qa-reader.pw.ts`、`vs03qa-helpers.ts`、`playwright.vs03qa.config.ts`、`run.sh`、诊断 `vs03qa-debug.pw.ts`）

## llmdoc 更新说明

- 新增本文件 `llmdoc/requirements/ui-feedback-3-6/visual-qa-report.md`（回合 1 全部结论、证据与限制）。
- 追加 `llmdoc/validation-release.md` §9：三条跨任务测试陷阱（checkbox 竞态、fixture 隔离、`*.pw.ts` 命名）。
- 回合 2 追加：本文件「回合 2 复验」小节与缺陷闭环字段；`llmdoc/validation-release.md` §9 追加 react-router v7 `NavLink` 的 `aria-current` 门控陷阱与回合 2 记录（跨任务复用：VS-03/04/05 的导航/标签页语义实现与验收）。
- 回合 3 追加：本文件「回合 3（VS-03 切片验收）」全节（逐 AC 矩阵、非阻断偏差 N-VS03-1/N-VS03-2、RD 自披露偏差四条判定、环境归因与未覆盖项）；`llmdoc/validation-release.md` §9 追加 VS-03 结果摘要与四条跨任务陷阱（抽屉首次 Esc 挂载竞态、断点切换几何"关系轮询"、Vite 页面内 import 生产模块做画布像素验证、`__EM_VIEWER__.project` 精确点选热点）。
- 未修改：PRD、tokens/design 基线、`visual-implementation.md`、`state.yaml`、`apps/web` 生产文件、RD 记录与用户并行流文件。
