# VS-05 独立 QA 报告（qa_round6）

日期：2026-10-05；执行者：Codex 独立 QA。PRD/UI 修订 1。最终结论：**AC-VS-001～012全部PASS，VS-05独立验收完成。46项去重独立用例通过（45项实现字节映射＋1项正式内嵌HTTPS Chrome）；新源全量工程、141普通E2E、native/Linux正式运行及最终包装修订2的交付审计全部闭环。** 本文件是 VS-05 独立报告，不改写 Claude 的回合1～3报告，VS-04 首次FAIL与返工复验保留在 [vs04-qa-report.md](/Users/qsyj/Code/rust/everything-manual/llmdoc/requirements/ui-feedback-3-6/vs04-qa-report.md)。

## 1. 有效源码与运行身份

- 有效发行源码为 `var/ui-feedback-3-6-vs05-qa/release-source-final`，591文件，inventory SHA256 `bb88c81f209e3b6dbac1dba1b9a2f9f65ed5c2d69396bcb1c9dd30624cc07a49`。执行独立QA时的旧发行冻结 `release-source` inventory为 `37d4236ba18b6c7bf12801f08e1e3e9fea3d1d50e2f38b66d232ab5e6dba573c`，保留原证据；此前 `final-product-source` / `final-product-source-v2` 是更早中间冻结，不作为最终发行版本。
- 独立前端/fixture执行快照为 `final-source`，330文件，冻结时间 `2026-10-04T17:46:59.794678+00:00`，执行时与37d423旧发行manifest逐文件核对0差异。原执行后冻结副本及当时工作树330文件0漂移。证据 [旧源绑定](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/release-source-binding.json)、[原执行复查](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/freeze-validation.json)。
- bb88c81f新冻结后独立只读映射再次验证两份manifest inventory、新snapshot全部591文件SHA以及已执行final-source330文件0漂移；431个前端src/public、后端crates、migrations/contracts、fixture和锁文件/运行配置逐文件不变。差分精确只有Dockerfile的validation worker预算、xtask smoke子进程检查、qa-t09/qa-t19/release-download三个E2E前置/旧notice期望。故接受原45项独立用例、72AFTER、52装饰/尺寸矩阵于新发行实现，不声称在bb88重新跑过这些相同页面。新工程门禁必须亲跑，不能继承旧构建结果。[源码前移证明](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/release-source-forward-binding.json)。
- 行为验收后端为当前源码的 `everything-manual-final-fixture`，SHA256 `a34384eb9e990ea066a2b93a681eac01b7782cbb4aea6150c07ee8e96d684618`，带 `job-failpoints` 供本机故障注入。它不作为生产交付二进制。后端源绑定 `final-fixture-source.json` 218文件。
- 同fixture BEFORE/AFTER截图只使用原baseline二进制 `9ecbc00b05058265865397695c2c4dc37cc64be6678a0c3ee0a500c737868b1a` 与原授权合成数据库副本，避免页面事实变化影响视觉比较；该旧binary不作为最终业务合同证明。AFTER前端是上述有效最终快照。
- 常规视觉/行为浏览器 Chrome `154.0.8037.93`、模型 SwiftShader。真实缩放专用 Chromium persistent extension context实际版本 `148.0.7778.96`，通过 `chrome.tabs.setZoom(2)` 设置原生浏览器缩放，并验证实际viewport/DPR变化；不把720px+DSF2的旧“等效缩放”用例计为本轮真实缩放。
- 所有provider/model/CDN均为自有回环fixture；独立监听15405～15408及18405/18408或自管随机端口，不使用生产服务。trace/video/自动失败截图关闭；显式截图遮罩password输入，密钥用例 `EM_AS_QA_NO_SCREENSHOTS=1`。没有付费或外部模型调用。

## 2. 最终源码上的实际执行

| 套件 | 最终已接受的独立用例 | 实际结果与证据 |
|---|---:|---|
| 原VS-02/03独立QA复用 | 27 | 原运行25PASS/2QA方法FAIL；方法复验关闭2项，全部27最终PASS。没有改变业务断言；[原结果](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/reused-qa/results.json)、[方法复验](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/reused-qa/method-rerun/results.json) |
| 真实未保存modal近邻 | 1 | 单层冻结overlay、2px圆角、opacity1、默认“继续处理”焦点、Escape保留输入；同方法复验结果 |
| 自有尺寸/对比/真实缩放/键盘/部分成功/阶段编号语义 | 5 | 原运行4PASS/1QA空白字符FAIL；完整语义规范空白后复验1PASS。[原结果](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/visual-results.json)、[语义复验](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/semantic-rerun/visual-results.json) |
| 最终关键恢复回归 | 11 | 11PASS/0FAIL/0skip/0retry，a343后端及最终源码。[结果](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/critical-recovery/results.json)、[日志](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final-critical-recovery.log) |
| 同fixture AFTER截图 | 1个采集用例/72组合 | 18组×4宽度全部captured，0unavailable；它是截图证据，不将72组合冒充72业务测试 |
| 正式Linux内嵌HTTPS Chrome | 1 | 本QA亲跑真实Chrome154，1PASS/0FAIL/0skip/0retry，唯一POST登录；JS/CSS/PDF/GLB哈希及实际模型/PDF/375断点通过；[公共摘要](/Users/qsyj/Code/rust/everything-manual/var/final-production-browser/browser-summary.json) |
| 全站可见装饰扫描 | 1 | 13组×4宽度共52组合，0违规；精确允许冻结模型区24px/6%网格与单层浮层，面板2px且无shadow，body14/22。[结果](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/decorations-rerun/visual-results.json)、[实际元素记录](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/decorations-rerun/visual/all-visible-decorations.json) |

上述46为去重后的最终已接受独立用例：45项在37源执行并通过严格字节映射接受于bb88，1项直接在bb88正式生产内嵌bundle上执行。不计重跑次数、不计qa4业务32、qa5近邻11，也不把父会话执行的普通E2E记为本人执行。qa4/qa5背景结果可用于差分影响分析，但最终critical11是本次实际重跑。

关键恢复11包括：AS-QA-02未保存导航/刷新及密钥不缓存、AS-QA-08读取/重复提交/字段错误/写失败/冲突重读；上传超限/一次失败重试；预算上下界/报价过期显式重报；任务needs_input本地重试不重复下载购买；PC3-004准备中止等待in-flight PUT和缺页续传；PC2B三项真实PATCH412/发布412/已知及未知422；PC3B两项未知提交及未消耗transport同key同body显式重试。

首次失败的原因、原现场与调整均保留：[measurement-method-revisions.md](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/measurement-method-revisions.md)。装饰旧探针扫描未显示的DIALOG并拒绝一切阴影，与冻结合同允许的单层浮层不符；仅改为实际可见元素、仅允许确切单层DIALOG overlay，同时新增真实打开未保存modal的近邻。阶段说明首次断言仅因JSX在“状态与 结果”之间引入空白失败，规范空白后仍硬断言完整意思、实际01编号及“不代表执行先后”。新增装饰探针第29组合首次失败为QA期待单层background-size，而浏览器正确序列化两层为`24px 24px, 24px 24px`；改成精确两层尺寸后52组合全部PASS，旧失败保留在`final/decorations/`。未改产品源码或降低阈值。

## 3. AC-VS-001～012 验收矩阵

| AC | 最终实际验证 | 当前结论 |
|---|---|---|
| 001 | 资料库/登录/详情actual-visible装饰扫描与tokens；阅读器/独立HTML实际样式；全部72截图；最终13组×4宽度全站可见装饰/面板形状扫描52组合0违规 | PASS |
| 002 | 52组合实际绘制文字/边界对比测量，五页实际focus环；VS02独立复算；body14/22与标题desktop28/36、mobile24/32；状态牌有真实文案 | PASS；所测最低文字12px/对比4.5878，边界3.2842，焦点5.5432 |
| 003 | 登录→资料库→详情→返回、导航名称/aria-current同源、直接刷新详情/settings/jobs；任务/设置近邻；72矩阵页标题/位置/返回入口 | PASS |
| 004 | 最终重新跑空库、缺字段、长型号、归档、无封面本地图标、搜索无结果、游标分页、请求失败保留筛选、摘要/任务入口失败、骨架/reduced-motion | PASS；API拒绝空白model，缺model是明确标识的防御fixture |
| 005 | 真实发布fixture，部件/热点/步骤/出处联动，bridge实际选中验证，canvas候选白底虚线问号/confirmed实心橙标记，真实PATCH确认候选，打开原件并回上下文 | PASS；编号只来自实际实体，不伪造热点序号 |
| 006 | 真实WebGL关闭、模型500后重试、原件500后重读、长文；选中notice现在“已选择部件…及关联热点”，WebGL失败仍有文字/PDF，不宣称已在3D定位 | PASS |
| 007 | 最终critical上传/准备取消续传/报价过期/未知提交/needs_input；四尺寸confirm与qa4定向确认未完成门禁；最终新增partial fixture模型成功、manual拒答缺项，UI真实状态与允许重试一致 | PASS；改变尺寸/Tab不增加provider计数 |
| 008 | 最终critical未保存/密钥不缓存与不回显/配置冲突/412/422/显式发布；真实候选PATCH及零加载自动写；qa5未配置/单家恢复近邻；费用及发送范围连续显示 | PASS；密钥输入不进入截图trace |
| 009 | 最终五页纯Tab首目标skip-link→Enter main，实际3px/3px焦点；reduced transitions0s；原VS03标签Arrow/Home/End、移动drawer Esc/恢复焦点、热点列表替代；VS02导航键盘 | PASS；抽屉关闭按钮成为可用焦点后才发Esc，未使用事件挂载前自动化抢拍 |
| 010 | 最终52组合与72全页矩阵，375/768/1024/1440，page overflow0，主操作≥44×44；五页真实原生200%，outer保持1440、inner1440→720、DPR1→2，滚动后主操作中心hit自己 | PASS于实测宽度/浏览器；旧“等效缩放”未计入 |
| 011 | 同发布重新导出HTML断网0HTTP/0error、model/文字/步骤可用；颜色/文字/标题/面板应用与HTML对比；manifest稳定、2发布asset字节不变；四份原历史HTMLhash前后不变 | PASS；独立部件按钮44高比应用桌面次操作40高更大，font14一致，不虚报所有几何逐像素相等 |
| 012 | 新bb88 host全check、普通完整141/141 E2E及native v3正式重现/前端build通过；72全页证据；gzip138016 vs135368，+2648≤10240；package/lock无新增依赖。Linux正式cold/network-none、HTTPS216checks、实际Chrome内嵌bundle及R2最终35成员/34SHA/save-load/外层包/独立总审核全部通过 | PASS；最终R2 SHA47d38665…3e64，按部署前提交付，未声称已公网部署 |

## 4. 实测数据与截图覆盖

[measurement-summary.json](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/measurement-summary.json) 汇总52组合：0尺寸/对比/字体异常，page overflow最大0，正常/必读辅助样本最小12px，实际文字最小4.587820:1，实际已绘制必要边界最小3.284202:1，主操作最小62×44px，移动可点击受测控件最小44×44px。禁用且不可操作的元素不计可点击目标，alpha0的未绘制边框不当作必要边界；正常/错误/警示状态用文案表达，不单靠色彩。

真实200%与键盘记录：[native-zoom.json](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/visual/native-zoom.json)、[keyboard-reduced-motion.json](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/visual/keyboard-reduced-motion.json)。五页为新建表单、费用确认、任务详情、应用阅读器、设置；focus实测solid3px/offset3px/`#A63F21`，对纸面5.543223:1；减少动效下实际按钮transition全0s。

完整同fixture截图18组：登录、资料库、新建物品、详情、原件上传、照片、准备、费用确认、任务列表、详情、结果、生成历史、复核、版本列表、原件、应用阅读器、设置、新导出独立阅读器。每组375/768/1024/1440，共72 BEFORE +72 AFTER。BEFORE是VS-03已完成时点，供观察VS-04/VS-05变化，不冒称整个视觉改版前的旧UI截图。实际视觉审查包括375原件/PDF、照片、reader、standalone、confirm，768资料库，1440资料库/reader/job/settings；此前人工图审发现阶段错误说明，已形成BUG-VS05-001。

[comparison-index.html](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/comparison-index.html) 可按页面/宽度筛选前后图；[comparison-pairs.json](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/comparison-pairs.json) 保存72配对的PNG hash/尺寸/样式，采集元数据与fixture身份见 [AFTER摘要](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/after/derived-summary.json)。72均已采集，0不可用、0页面横向overflow，fixture及发布fingerprint与BEFORE一致。两次截图浏览器provider调用计数均0，AFTER唯一写请求是登录POST；断网独立HTML0HTTP。

独立HTML h1实际375为24/32，768/1024/1440为28/36，body所有宽度14/22；面板2px、按钮4px、部件按钮44高。PDF页、照片、模型材质没有滤镜。原历史4份`*-3d.html`前后SHA逐一相同：[历史复查](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/historical-exports-post-final.json)。

## 5. 缺陷与偏差闭环

| ID | 初次事实 | 最终复验 |
|---|---|---|
| BUG-VS02-001 | /items子路由资料库只有视觉选中缺aria-current | 最终重新跑导航aria矩阵，已关闭 |
| BUG-VS04-001 | 375 prepare/confirm原件select高41 | qa5及最终实际335×44，已关闭；原FAIL保留 |
| BUG-VS04-002 | 375 review刷新72×18 | qa5及最终72×44，已关闭；原FAIL保留 |
| BUG-VS05-001 | 阶段编号与“按执行顺序”提示矛盾，真实DAG记录assemble在freeze前 | 最终完整提示明确“不代表执行先后”，实际列表01assemble事实，已关闭；没有要求新增执行排序 |
| N-VS03-1/2 | 独立h1旧26/34、应用body22.4而HTML22 | 最终h1逐宽对齐28/36与24/32、body22；最终样式对比布尔true，已关闭 |
| N-VS03-3 | WebGL不可用仍称“已在3D定位” | 最终关闭WebGL真实UI与notice改为选择实体/关联热点，不冒称渲染成功，已关闭 |
| N-VS03-4 | HTML热点状态中性tag，应用success软底 | 有文字且软底为冻结合同可选，不属阻断；保留观察 |
| N-VS03-5 | 自动化在drawer监听挂载前发Esc | 在close-button实际focused后验证Esc/焦点恢复PASS；未以超早事件要求新业务行为 |

## 6. 工程与生产交付门禁

新bb88完整工程检查已独立读取 [release-final-check.log](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/checks/release-final-check.log) 与 [summary](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/checks/release-final-check-summary.json)：`VITEST_MAX_WORKERS=1 cargo xtask check`实际PASS，Rust672PASS/10ignored（不计ignored为通过），Vitest50文件336PASS，fmt/clippy/typecheck/lint/contracts全部exit0；591主源复查无漂移。这是新源亲跑，旧37check/gzip日志仍保留且不覆盖。

新bb88正式native构建的 [frontend-build-summary.json](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/native-run-final-v3/frontend-build-summary.json) 明确285前端源逐SHA匹配bb88、typecheck/Vitest/Vite/dist-reproducible全部exit0。主入口JS413743B + CSS79160B，同配置gzip-9为138016B，对改版原基线135368B为+2648B≤10KiB。JS SHA `fad81211a4dfe694b5903f74014d8eae540f16a211efa04be9862d71056adc0c`，CSS SHA `ebb47eeaa43a7ce77c07a9db7de2b0afc7371c7245a7f0f350d9264bc377d022`。新dist两资产逐字节与37相同；package.json/lock BEFORE、37、bb88 hash相同，无新增第三方UI/字体依赖。

父会话旧37源全量普通E2E实际141例/25文件首次完成136PASS/3FAIL/2串行未执行，证据 `root-regression/results.json` / `run.log` 保留。三项分别为qa-t09顶栏elementFromPoint、旧notice期望字符串、导出pending切换宽度瞬时overflow；三个QA前置/旧文案已修订，新bb88完整141/141实际通过，旧失败记录仍保留。旧37源Linux前端Vitest默认worker下334/336；单worker诊断失败两文件15例及完整336例全PASS，完整validation也已exit0，证据 `linux-final-20261005-workers1/execution-state.json`。没有放宽断言或timeout，新bb88 Docker将该validation预算正式固定为1并须重新完整验证。默认testIgnore不包含的独立PC配置由父会话单列验证，不虚称普通141覆盖所有ignored文件。

新bb88的三个E2E修订已逐行审阅：qa-t09先滚动通知进入viewport再测中心hit，去掉吞click错误并断言具体通知按钮消失；qa-t19用完整真实部件名精确匹配“已选择部件…及关联热点”，仍保留模型anchor联动断言；release-download等待窄/中/宽实际React断点DOM再测几何。均没有删除业务断言或延长断言deadline。Docker仅validation增加worker1，runtime/release未改；xtask将真实子进程或pgrep异常作为验收失败，仅exit1且空stdout/stderr认定无匹配，不再假跳过。新完整141与native v3、Linux及最终交付实际结果见下，已全部闭环。

新bb88普通全量已独立读取 [results.json](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/root-regression-final/results.json) / [summary.json](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/root-regression-final/summary.json)：25文件141/141PASS，0FAIL/skip/retry/flaky/errors，830.6s，source/fixture绑定bb88/a343且591文件0漂移。旧3FAIL与2串行未跑均在新完整suite通过；旧现场保留。普通套件仍明确排除`qa-pc*.spec.ts`，不将独立PC工作流谎称为这141的一部分。本QA未亲跑该141，仅独立核对peer实际产物。

正式native/Linux/embedded-ui/no-job-failpoints/smoke/backup生产包由父会话及production_readiness agent独立验证。本QA已读取 [native v3报告](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/native-run-final-v3/native-verification-report.md) 与status，明确绑定bb88最终源：macOS arm64仅embedded-ui binary SHA `adcad48ac9bd4e95908569ffa2c400a928b0a856cc86f408ed52b3b0998f7a99`，新源亲跑两次重链接hash一致，冷目录完整七步与全checker OS sandbox七步均exit0；TEST-NET真实EPERM，回环读取成功。修订后的pgrep helper本轮真实执行且无skip，不以v2旧补采替代；正式备份及归档checksum通过。v2已由owner标为SUPERSEDED，原报告/原PASS状态仍保留。该结论来自peer证据，未计为本QA亲跑。

Linux新bb88正式validation已671Rust+336前端PASS；runtime dist默认pool又336PASS（并发数未打印，不能将它宣称worker1）。已独立读取 `linux-release-final-20261005/execution-state.json`、`cold-summary.json`与真正执行日志：已读取正式release/build-info，features仅`embedded-ui`、musl静态链接，不带`job-failpoints`；runtime正式binary SHA `e260dccb690a234d05163fa241ba00fd123cbafc7fe05f1b24c5fdc68feadc6f`，应用image ID `sha256:9719e16794ba06ff53da5d94d9df2f1ad10c623e10a576305757e19033ef2710`。`--network none`冷目录完整七步PASS，Docker六阶段exit0/661 HTTP断言，正式HTTPS严格证书与hostname验证216checks PASS。两项环境初次失败保留在execution-state：platform manifest ID不可直接run的exit125已改用本Engine实际可寻址的image/index引用，未重建/重测；默认/tmp noexec下冷测试exit1，改明确QA-only exec tmpfs后完整七步亲跑，不改变生产tmpfs的noexec安全属性。Linux内层macOS sandbox提示skip不作为断网证据，真正整条checker network-none才是证据。

本QA亲跑正式HTTPS stack上的实际Chrome `154.0.8037.93`，1PASS/0FAIL/0skip/0retry，6.7s：[browser-summary.json](/Users/qsyj/Code/rust/everything-manual/var/final-production-browser/browser-summary.json)、[results.json](/Users/qsyj/Code/rust/everything-manual/var/final-production-browser/results.json)。新context仅接受该QA临时证书，前置Python已经严格验证证书链及hostname，系统信任未改。真实登录cookie为Secure/HttpOnly/SameSiteStrict且JS不可读；实际资料列表→详情→发布列表→冻结发布版；真正HTTPS主JS/CSS SHA与正式构建匹配，GLB/PDF完整响应及浏览器实际请求SHA同时匹配manifest。模型实际12triangles/2objects/1texture、frames>0/context ok；选择部件关联1个anchor、实际步骤1/1与出处PDF canvas 290×411/2098墨迹像素。375实际narrow DOM、overflow0、部件drawer选择/Escape及焦点恢复、原文可见。实际桌面与移动PNG经本QA人工图审通过。0未捕获错误/外部请求/生成或保存POST，唯一写请求为登录，0paid/provider调用，无route.fulfill、无响应替换。

正式浏览器两次首次QA失败均保留：第一regex标题同时匹配资料库h1/摘要h2，改精确h1；第二桌面“返回出处”保留的steps panel在缩宽后成为真实modal，QA点击modal背景部件按钮而超时，改正常关闭既有drawer再继续。原现场在 `var/final-production-browser/first-attempt` / `second-attempt`，完整修订在 [method-revisions.md](/Users/qsyj/Code/rust/everything-manual/var/final-production-browser/method-revisions.md)。产品源码、业务断言和180秒case deadline未改；新增30秒action deadline用于更早显示具体action失败。没有把这两项QA前置失败虚称产品修复。
本机a343测试fixture及Vite页面不能替代正式包。部署配置、发布资产、迁移与回滚由生产交付报告说明；正式交付归档save/load、文件允许清单及独立交付审核现已闭环；本报告签署范围为具备所列前提的交付物验收，未声称实际公网部署。

## 7. 可复跑入口与实际限制

不可覆盖现有快照；如产品源变化须新freeze并重新绑定manifest。自有验证入口（项目根目录）：

```sh
EM_VS05_QA_STAGE=final apps/web/node_modules/.bin/playwright test -c var/ui-feedback-3-6-vs05-qa/visual.playwright.config.ts
apps/web/node_modules/.bin/playwright test -c var/ui-feedback-3-6-vs05-qa/final-reuse.playwright.config.ts
EM_VS05_QA_STAGE=final EM_VS05_QA_RUN_TAG=critical-recovery EM_AS_QA_NO_SCREENSHOTS=1 apps/web/node_modules/.bin/playwright test -c var/ui-feedback-3-6-vs05-qa/qa4.playwright.config.ts import-flow.spec.ts workflow-recovery.spec.ts job-recovery.spec.ts api-settings-qa.spec.ts review-tasks.spec.ts preparation-discovery.spec.ts --grep '上传失败重试|预算变动|PC3B-008|needs_input：|AS-QA-02|AS-QA-08|PC2B-007|PC3-004'
```

真实200%使用Chromium扩展，普通视觉使用本机Chrome；未声称独立QA在Safari/Firefox/Edge或真实移动硬件全测。Chrome/SwiftShader覆盖逻辑和降级，本轮不构成所有GPU/浏览器认证。长文、空缺字段及供应商故障部分是明确的合成/拦截fixture，实际成功发布、文件/PDF/模型与412/422使用真实本机后端。5位用户五秒识别研究尚未开展，PRD已说明其为研究目标而非工程门禁。密钥截图/trace始终关闭或masked；原失败现场不含真实生产密钥。

源码映射首次尝试曾因QA预设了错误的两个E2E文件名而失败，原结果保留 `final/release-source-forward-binding-initial.json`；该初次结果已证明431实现文件和两棵树无变化。按实际三项旧失败文件名与父冻结delta核对后改精确allowlist，再次映射PASS。不是产品差分、没有扩充可接受产品改动范围。

最终交付已关闭包装可移植性缺口：API≥1.49只满足flag要求，不能保证不同Docker image store可直接使用本机raw引用。包装修订2明确首次部署/换机必须从归档tag用default inspect发现该机不可变Id，精确比较amd64/完整Config/RootFS层，再执行无拉取、无网络、只读、drop-all且app UID10001/proxy UID101的真实SHA/version探针，失败必须停止。证据是原完整导入/Config/层检查与随后两项显式非root探针组合，不冒称重新完整运行未观察到的最终脚本。classic store/其他Engine版本未实测。源码、binary、images.tar、正式浏览器摘要与其他29成员不变，精确只6项包装说明/manifest/checksum/log变化。旧c499包及原先PENDING审计记录保留、明确superseded，不作为发行交付。

本QA亲复算R2目录35允许成员与34校验和，manifest SHA `7c95f5a7b548a3da112595818b9d7a5e5f0b78b35a6e91c96316f8871a76fb1b`、根清单SHA `9cec4bd1abd49ddc14271a2b0b10c46320a3e1f506561fa816c23aa6a47199bc`；随包browser摘要与本人原执行证据逐字节相同。实际外层54,908,769B、SHA `47d38665d38ea06de0f3f46ff705744f86433e5538e9e42d2d40606c6aa13e64`，只读tar解压流逐35常规成员size/SHA与目录相同，无链接、特殊或越界路径。[最终交付物](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/linux-release-final-20261005/revision-2/everything-manual-linux-amd64-production-vs05.tar.gz)、[本人包/浏览器绑定](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/production-package-binding.json)。

独立交付peer已正式签署 [总审计PASS](/Users/qsyj/Code/rust/everything-manual/var/final-delivery-audit/final-delivery-audit.md) 与机器结果；本QA复核其7份阶段JSON SHA全部一致、bb88/最终R2/二进制/实际浏览器绑定一致，没有剩余gate。checksum-only子阶段原PENDING标签是历史先行检查，后续执行/镜像/前置/外层验证关闭全部待项，综合PASS才是最终结果。Docker六阶段有1次明确自有TLS fixture Provider请求/paid0；严格HTTPS及正式Chrome Provider均0，不把fixture请求说成实际供应商验收。最终591主源与发行快照再次逐SHA核对0漂移。

自有QA context均关闭、harness teardown已执行；最终只读lsof15405～15408及18405～18408八个自有端口均空，无广泛kill或用户预览操作。[清理记录](/Users/qsyj/Code/rust/everything-manual/var/ui-feedback-3-6-vs05-qa/final/cleanup-check.json)。唯一正式HTTPS栈/loader/fixture卷由其owner清理并验证，不由本QA操作。

签署：Codex独立QA，2026-10-05T03:39:26.829984+08:00（Asia/Shanghai）。验收结论：AC-VS-001～012 PASS、VS-05完成，交付身份以bb88发行源及R2最终归档为准。部署仍需管理员提供实际域名/合法证书/私有初始口令和主密钥，并在目标机执行所列导入门禁；本轮没有公网部署或真实付费供应商/GPU/跨浏览器验收。Mac为arm64且未签名/公证，Linux AMD64在ARM64 VM/Rosetta环境运行，未声称物理x86或其他平台覆盖。父会话负责将本结论同步已有规划state；本QA未改shared state。
