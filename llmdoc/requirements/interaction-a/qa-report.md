# interaction-a QA 验收报告

结果：**PASS** · 回合：2 · PRD 修订：1 · UI 修订：1 · 范围：IA-01 全部 12 AC（原体验方案切片 A）

日期：2026-09-20；实际执行时间跨 2026-09-19 23:56 至 2026-09-20 00:06（Asia/Shanghai）。首轮发现 IA-BUG-001 并交 RD 修复；第 2 轮独立复验通过并 **CLOSED**，12 AC 全部通过、未关闭缺陷 0。保留首轮 FAIL 原因与证据。本报告不改变 web-mvp T22/T23 状态，也不表示其他体验切片已完成。

## 环境与交付版本

- macOS 26.6.2 / build 25G83，Apple Silicon；Chromium 148.0.7778.96（Playwright 缓存 1223）。源码工作树验收，未编造提交号；[最终源码 SHA-256 清单](../../../artifacts/interaction-a/qa/source-sha256-round2.txt)，另保留[首轮清单](../../../artifacts/interaction-a/qa/source-sha256-round1.txt)。
- 独立前端 `127.0.0.1:15182`、Rust 测试后端 `127.0.0.1:18182`、专有临时 data-dir；没有使用用户预览数据。后端构建 `CARGO_NET_OFFLINE=true`；两个供应商地址均为 `127.0.0.1:1`，假密钥由原测试设施注入。浏览器阻断非本机请求，实际无外部请求。
- 创建、编辑、PDF/照片上传造数、准备、报价、确认及任务均使用真实 Rust HTTP 与 SQLite；延迟、断线、配置未就绪和错误响应明确由浏览器测试注入。阅读器 manifest/GLB 是合成布局夹具，PDF 原件和字节来自真实后端；不能据此宣称重新验过 T19 发布语义或真实供应商质量。
- QA 独立新增 [7 个浏览器场景](../../../apps/web/tests/e2e/interaction-a-qa.spec.ts)和 [2 个组件边界用例](../../../apps/web/src/features/shell/interaction-a-qa.test.tsx)，亲自运行生产构建、Lint、全量前端单元与浏览器检查。RD 自测记录只作为交接输入。

## 执行结果与证据

| 检查 | 亲自执行结果 | 证据 |
| --- | --- | --- |
| 第 2 轮 `npm --prefix apps/web run test` | **21 文件，147/147 通过**，包含修复后的 Drawer 三条回归 | [unit-round2.log](../../../artifacts/interaction-a/qa/unit-round2.log) |
| 第 2 轮 `npm --prefix apps/web run lint` 与 `run build` | 均 exit 0；Lint 0 warnings，build 包含 typecheck | [lint-round2.log](../../../artifacts/interaction-a/qa/lint-round2.log)、[build-round2.log](../../../artifacts/interaction-a/qa/build-round2.log) |
| 第 2 轮独立完整 7 场景 | **7/7 通过，28.6s**；包含缺陷原始断言、summary 的 Space/Enter、完整 PDF 文字层测量、真实 200% 缩放与遮挡取证 | [e2e-round2.log](../../../artifacts/interaction-a/qa/e2e-round2.log) |
| `npm --prefix apps/web run test` | 20 文件，144/144 通过 | [unit.log](../../../artifacts/interaction-a/qa/unit.log) |
| `npm --prefix apps/web run lint` | exit 0，0 warnings | [lint.log](../../../artifacts/interaction-a/qa/lint.log) |
| `npm --prefix apps/web run build` | exit 0，包含 tsc 与 Vite；保留既有大 chunk 提醒 | [build.log](../../../artifacts/interaction-a/qa/build.log) |
| 独立 E2E 首轮 7 场景 | 5 通过、2 测试侧错误；保存状态选择器匹配两处、隔离浏览器继承 deviceScaleFactor 与 null viewport 冲突 | [e2e-round1.log](../../../artifacts/interaction-a/qa/e2e-round1.log) |
| 修正测试侧错误后定向 IA-QA-03、06 | 2/2 通过；未修改产品代码或放宽业务断言 | [e2e-rerun.log](../../../artifacts/interaction-a/qa/e2e-rerun.log) |
| 加强 IA-QA-05：等待真实 PDF 文字层并核对 Tab 路径 | 0/1，复现 IA-BUG-001；此前只测抽屉内循环，遗漏了原生 summary 控件 | [e2e-focus.log](../../../artifacts/interaction-a/qa/e2e-focus.log) |
| 实际浏览器 200% 缩放 | `chrome.tabs.setZoom(2)`；getZoom=2，outerWidth 始终 1440，innerWidth 1440→720，DPR 1→2；表单/确认键盘路径、元素边界与无遮挡命中通过 | [缩放数值及元素几何](../../../artifacts/interaction-a/qa/06-browser-zoom-evidence.json)、[原生视口确认页](../../../artifacts/interaction-a/qa/06-confirm-browser-zoom-200-viewport.png)、[原生视口创建页](../../../artifacts/interaction-a/qa/06-create-browser-zoom-200-viewport.png) |

浏览器命令统一为 `CARGO_NET_OFFLINE=true EM_E2E_WORK_DIR=/private/tmp/everything-manual-interaction-a-qa EM_E2E_API_PORT=18182 EM_E2E_WEB_PORT=15182 npm --prefix apps/web run test:e2e -- interaction-a-qa.spec.ts`，追加表中的 `-g` 场景和独立 `--output` 目录。首次沙箱监听 `EPERM` 后按授权流程重新执行成功；不能把该环境尝试计为产品失败。实际缩放使用隔离临时浏览器扩展，不修改用户浏览器、viewport 或 CSS zoom 来冒充浏览器缩放。

## AC 验收矩阵

| AC ID | 期望 | 实际 | 结果 | 独立证据 |
| --- | --- | --- | --- | --- |
| AC-001 | 创建一次后直达该物品上传路由 | 真实 201 后进入 `/items/{id}/import/document`，看到原件上传；无额外创建 | PASS | IA-QA-01；[375px 创建](../../../artifacts/interaction-a/qa/01-create-375.png) |
| AC-002 | 创建等待/失败留存、编辑保存回概览 | 等待时禁用并阻止重复 submit；422 保留输入；编辑真实保存回概览 | PASS | IA-QA-01；144 单元中的创建回归 |
| AC-003 | 四尺寸主流程连续且无重复确认/横溢出 | 375/768/1024/1440 均无侧栏入口，DOM 顺序为报价→发送范围→确认→生成，一套 checkbox，无横溢出 | PASS | IA-QA-02；[尺寸数据](../../../artifacts/interaction-a/qa/02-layout-measurements.json)、`02-confirm-{width}.png` |
| AC-004 | 缺项/加载/配置/失败/报价的真实主内容 | 无准备及配置不可用有修复入口，无假金额；加载可见；错误有重试并可恢复；credits/USD、范围、页类型及模型可直接读到 | PASS | IA-QA-02/07；[配置未就绪](../../../artifacts/interaction-a/qa/07-config-not-ready.png)、[报价错误](../../../artifacts/interaction-a/qa/07-quote-error.png) |
| AC-005 | 错误金额、低预算、过期及新报价约束 | 错误及低预算禁用并解释；到期只在主动点击后重报；新报价不勾选；输入变化有返回资料入口 | PASS | IA-QA-03/04；独立执行的 ConfirmStepPage 单元 |
| AC-006 | 确认等待禁用、失败回退、成功才放行 | 实际延迟期间禁用且显示保存状态；失败回退；成功后放行 | PASS | IA-QA-03 定向通过日志 |
| AC-007 | 取消立即禁用、重勾重等、响应按报价隔离 | 三次确认请求覆盖成功/取消/重等失败/重试；旧确认在新报价后到达未污染页面 | PASS | IA-QA-03/04；[旧响应截图](../../../artifacts/interaction-a/qa/04-stale-confirm-response.png) |
| AC-008 | 提交去重、同页面显式重试、已有任务/受理入口 | 网络失败无自动重发，主动重试复用 key，真实后端只有 1 个 job；既有任务锁定及受理文案经单元复验 | PASS | IA-QA-03；[任务受理](../../../artifacts/interaction-a/qa/03-confirm-accepted.png)；ConfirmStepPage 单元 |
| AC-009 | 标签方向/Home/End/循环、ARIA 与 Tab | 1024px 真实浏览器全部通过；Tab 进入当前面板。纯文本落点与单面板由独立组件用例覆盖 | PASS | IA-QA-05 首轮；[中屏键盘](../../../artifacts/interaction-a/qa/05-reader-keyboard-mid.png)；interaction-a-qa 组件 2/2 |
| AC-010 | 单面板、鼠标及完整抽屉焦点路径 | 第 2 轮 summary 已可由下一页 Tab 到达，Space 折叠/Enter 展开，首尾 Tab/Shift+Tab、Esc 和触发点恢复全部通过；首轮失败见缺陷历史 | **PASS** | IA-QA-05 第 2 轮；IA-BUG-001 CLOSED；Drawer 单元 |
| AC-011 | 关键正文14、辅助12、触控44 | 375px 创建/确认/步骤正文、辅助说明、按钮矩形达到阈值；真实两页 PDF 和非空 summary/pre 加载完成，计算字号 ≥12 | PASS | IA-QA-01/02/05；[手机真实 PDF 文字层](../../../artifacts/interaction-a/qa/05-reader-pdf-text-375.png) |
| AC-012 | 手机和真实200%可读可操作、不被底栏挡住 | 创建及确认真实缩放、375px 纵向滚动与键盘到输入/确认/生成通过；关键内容左右边界在视口内，输入和主按钮中心命中自身，原生视口图确认未被底栏遮住 | PASS | IA-QA-02/06；真实缩放数值/几何及原生截图；不作全站无障碍认证 |

## 缺陷

### IA-BUG-001 · PDF 文字层的折叠控件被抽屉 Tab 陷阱跳过

- 严重度／状态：**P2 / CLOSED**（QA 回合 2，2026-09-20）。
- 对应 REQ / UI / AC：REQ-005/006；UI-004、UI-005 的原生折叠控件键盘可达；AC-010。
- 环境与输入：Chromium 148.0.7778.96、375×812；IA-QA-05 的真实两页 PDF + 合成已发布阅读器 manifest，文字层已加载、`summary/pre` 已测 ≥12px。
- 复现：打开阅读器的「步骤与原文」抽屉；等页码显示 `第 1 / 2 页` 且出现「本页文字（PDF 文字层）」；将焦点放在「下一页」按钮；按 Tab。
- 期望：Tab 到后续的原生 `summary`，可按 Enter/Space 展开文字层；最后一个实际可聚焦控件后才回到「关闭」。
- 实际：焦点直接回到「关闭」。`summary` 仍存在且可用鼠标点击，却无法沿抽屉内正常 Tab 路径到达。
- 根因依据：[Drawer.tsx](../../../apps/web/src/features/shell/Drawer.tsx) 的 `FOCUSABLE_SELECTOR` 未包含 `summary`，把「下一页」错误当作最后一个控件。
- 证据：[失败日志](../../../artifacts/interaction-a/qa/e2e-focus.log)；[截图](../../../artifacts/interaction-a/qa/playwright-focus/interaction-a-qa-IA-QA-05--4e827-抽屉与手机触控（AC-009-010-011-012）-chromium/test-failed-1.png)；[trace](../../../artifacts/interaction-a/qa/playwright-focus/interaction-a-qa-IA-QA-05--4e827-抽屉与手机触控（AC-009-010-011-012）-chromium/trace.zip)；独立断言 `interaction-a-qa.spec.ts` 的下一页→Tab→summary。
- 建议回归范围：抽屉原生 summary 的 Tab/Shift+Tab、开/合文字层、首尾循环、Esc 和关闭后归还焦点；现有 Drawer/shell 单元及 IA-QA-05。不要放宽断言或去掉原生折叠。
- RD 修复摘要引用：[implementation §5](implementation.md#5-qa-回合-1-修复ia-bug-001)。加入 summary 并动态过滤禁用/隐藏/关闭 details 内不可用控件，保持原生 Tab 顺序。
- QA 复验：2026-09-20 00:05，第 2 轮 IA-QA-05 真实浏览器通过原失败断言；继续验证 Space 折叠、Enter 展开、Shift+Tab 末端循环、Esc 和还焦；完整 E2E 7/7、147 单元、Lint、build/typecheck 通过。状态由 QA 改为 CLOSED。

## 非代码知识与限制

- 字号、44px 操作区与隐藏面板的断言必须等真实内容加载后测。手机切断点会重新挂载 PDF 面板，先前中屏 `2 页` 成功不能代替手机加载证据。本轮因此加强检查发现 summary 被旧焦点陷阱遗漏。
- 可见的保存中状态会同时出现在确认区和生成禁用原因里，自动测试应精确定位状态文案；这不是重复确认控件。Playwright 自建 persistent context 会继承测试默认 deviceScaleFactor，原生窗口缩放测试需显式清除该选项。
- 本轮只验 UI 改进的 fixture 行为。未执行真实 Provider、全量 T22 打包、浏览器跨刷新建单恢复或跨标签页准备恢复；这些均非本片承诺。浏览器手工触控硬件研究未做，不将 CSS 点击矩形测量称为用户研究。
- 原生浏览器 200% 缩放已执行且有数值证据；未将窄 viewport、CDP pinch zoom 或 CSS zoom 充当浏览器缩放。
- **缩放截图口径**：首轮 `06-*-browser-zoom-200.png` 是 Playwright fullPage 在原生缩放下产生的裁图伪影（宽 720 输出却以 DPR 2 渲染），已标为 superseded，不能据其判断产品裁切。用不指定 clip 的 `Page.captureScreenshot(captureBeyondViewport=false)` 取得 1440px 原生视口图后，人工查看确认主操作和底部导航完整可见；所有主要内容左右界 20–700 均位于 720 CSS px 视口内。最终 QA-06 同时断言聚焦输入/主按钮的 `elementFromPoint` 命中自身、按钮位于可视高度内，避免只凭无横向滚动判断无遮挡。旧截图保留用于解释测试取证差异。

## 回合历史与交接

- 回合 1（2026-09-20）：12 AC 中 11 项通过、AC-010 失败；发现 IA-BUG-001 P2 OPEN。两次测试侧错误已修正并定向 2/2 通过，不计产品缺陷。**交 RD 修复后由 QA 复验**；未修改生产代码、PRD、state 或验收阈值。
- 回合 2（2026-09-20）：同一 PRD/UI revision 1，独立运行完整 7 场景 7/7、全量 147 单元、Lint 与构建/typecheck；12/12 AC PASS，IA-BUG-001 CLOSED，无未关闭缺陷或必选未执行 AC。最终浏览器服务自动退出，`lsof -nP -iTCP:15182 -iTCP:18182 -sTCP:LISTEN` 无监听；临时原生缩放浏览器已关闭。本次全范围仅为 IA-01，不扩张到其他体验切片或 T22/T23。
- 本轮 QA 更新 llmdoc：`llmdoc/requirements/interaction-a/qa-report.md`。新增非代码知识为原生缩放证据、PDF 重挂载测量陷阱、抽屉 summary 的可复现缺陷与验证边界。
