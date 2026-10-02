# 对照 PRD 继续完成产品

- 日期：2026-10-02
- 用户请求：「重启后，继续自己去网上找说明书构建后续功能，直到符合PRD预期」。
- 目标：恢复本地服务，以公开官方说明书验证真实资料入口，继续完成原 web-mvp PRD 的实际缺口及已接受的交互改进，逐片实现和独立验收；不把本机 fixture 结果冒称真实供应商或全平台发布通过。
- 既有成果：interaction-a、api-settings、encrypted-secrets 已 full PASS，保留关闭状态与历史证据。原 web-mvp T01–T21 已验收，T22/23 仍未完整关闭；本轮按当前源码重新判断，不套用九月旧产物哈希。
- 样本：可自行寻找并下载公开官方说明书到 Git 忽略目录；记录来源、型号、页数、哈希与本机测试用途，不将第三方 PDF 当作拥有再分发许可的仓库 fixture。
- 执行：一位 RD 写生产代码，PM/UI 冻结每个切片要求，QA 独立验收；协调者维护此状态和本地预览。完成可独立推进的开发后，再列明确实依赖外部条件的项目。
- 保密：不输出配置秘密、不在项目文档和证据中复制密钥。初始发现的模型栏误填与密钥缺失已由用户在网页修正；2026-10-02 12:53 UTC 只读复核两供应商均已配置、无待重启/模型误填问题。价格目录仍未配置，真实计费验收仍待准确价目表及一次样本的明确预算。
- 外部动作：未授权提交、推送、部署公网、购买额度或无预算的付费验证；真实链路应先准备具名样本与可核对的请求/预算计划，再取得所需预算范围。
- 环境实测：本地预览已按请求重启，登录和设置读取通过；无活跃/未决供应商任务。初始数据卷可用约 4.1 GiB，清理本项目可重建的 `target/debug/incremental` 后约 9.44 GiB；未清理用户数据。一次非破坏性 `docker desktop start --timeout 30` 返回启动中，但随后 `docker version` 仍报 `Docker Desktop is unable to start`，Linux 验证暂缺运行环境；不反复重启或重置其数据。
- Docker 后续只读诊断（当前启动日志 UTC 2026-10-01 17:43）：VM 成功启动后，dockerd 读取镜像引用存储 JSON 遇到 NUL 字符并退出码 1，随后 VM 关闭。`overlay2` 引用索引损坏是直接错误，不以旧 ENOSPC 日志推断本次原因；未挂载或修改 `Docker.raw`。约 8 GiB 余量不足以安全保全磁盘镜像后恢复，未发现可保证无损的官方修复流程。证据 `var/prd-completion/docker-readonly-diagnosis.json`，保留为 Linux 验证环境阻塞。

## 官方样本与本机实测（2026-10-02）

三份官方数字原件已下载到忽略目录 `var/manual-samples/`；来源、SHA-256、页数和逐页文字量见该目录的 `manifest.json`，未将 PDF 或页图提交为仓库 fixture。

| 样本与官方来源 | 页数 | 用途 |
| --- | --- | --- |
| [IKEA BILLY AA-2289108-3](https://www.ikea.com/us/en/assembly_instructions/billy-bookcase-white__AA-2289108-3-100.pdf) | 16 | 配件编号、图示组装、稀疏文字；准确商品 SKU 未核实 |
| [IKEA LACK AA-2544914-1](https://www.ikea.com/th/en/assembly_instructions/lack-side-table-white__AA-2544914-1-100.pdf) | 8 | 小型完整资料准备；准确商品 SKU 未核实 |
| [IKEA TILLREDA 404.934.20](https://www.ikea.com/us/en/manuals/tillreda-microwave-oven-white__AA-2274353-2-1.pdf) | 32 | 部件标注、五步操作、规格及合法空白页 |

LACK 已通过真实 HTTP 导入本地预览，并由现有 PDF.js worker 完成 8 页准备。物品 `01a0f894-0433-72ec-8693-9b65e02b9b38`，文档 `01a0f894-046d-75ec-b4a0-d953164c26e0`，准备记录 `01a0f894-f88b-77cd-9321-d8233c460a04`；状态 ready，8 份文字与 8 份 JPEG（加原件共 17 份资产）字节 SHA-256 与 ETag 一致。样本没有生成任务，浏览器外网请求为 0。证据在忽略目录 `var/manual-samples/lack-local-evidence.json`。这只验证本机资料路径，不是 LLM 提取、Tripo 质量或 T23 真实链路通过。

现有原文组件另经独立挂载实测：8 页实际绘制、文字与准备资产一致、首尾翻页禁用和返回页正确；这不是待补产品入口的验收。第 4 页存在 `U+FFFD` 替换字符，不能宣称抽取完全准确。三份样本均为数字原件，仍不能替代原始扫描样本。

原始扫描候选已补充：[Roland TR-808 官方手册](https://cdn.roland.com/assets/media/pdf/TR-808_OM.pdf)，42 页、4,130,116 字节、未加密，SHA-256 `bf5e15408c3aee59fd43135ee51834daaeec6cd54a7fb118984727d15e22e7fb`。逐页无文字/字体资源，主体为 1-bit CCITT 整页图，并经面板/操作/规格页视觉核验；原始字节未转换。原件及来源记录为 `var/manual-samples/roland-tr-808-original-om.pdf`、`scan-candidate.json`；本机准备实测另记，不以结构核验冒充解析质量通过。

TR-808 真实 PDF.js 准备也已完成：物品 `01a0f8ad-ff39-7323-be86-6154e68ace95`，文档 `01a0f8ae-008e-7482-acd5-157e8d8c59b5`，准备 `01a0f8ae-07e7-7452-afce-9f5e2d142204`。42 页按顺序上传，ready/clientDerived=true，无缺页；42 份 JPEG（共 11,662,990 字节）解码、viewport 尺寸及 SHA-256/ETag 均通过；全部 textAssetId=null，未为扫描页伪造文本。样本 jobs 前后为 0、浏览器外网/敏感接口请求/pageerror 为 0。证据 `var/manual-samples/tr808-local-evidence.json`；真实产品阅读入口待 PC-02，未调用 LLM/Tripo。

浏览器环境已补充：项目忽略目录内 Playwright Firefox 150.0.2 与微软官方 Edge Stable 154.0.4258.48 均能启动并显示本地登录页；Edge 包官方 SHA-256、微软签名和 Apple 公证已核验，仅展开到本地，未运行系统安装器。路径与版本见 `var/playwright-browsers/environment.json`。这只是环境就绪，不等于 AC-063 全量回归通过，Firefox 为 Playwright 测试发行版。当前 Chrome 也可用。

版本边界：2026-10-02 读取 Mozilla 官方版本元数据为 Firefox Stable 157.0、ESR 140.17.0esr；不能把上述 150.0.2 结果称为当前稳定版全验收。另行在忽略目录准备的 Playwright 1.63.0 官方稳定测试工具仍配 Firefox 155.0，next 元数据为 156.0；未更改项目 npm 依赖。

当前 Firefox 环境随后补齐：用 `@puppeteer/browsers@3.2.3` 将 Mozilla 官方 Firefox 157.0 安装到忽略目录 `var/bidi-browsers/`，代码签名校验通过，使用独立 `puppeteer-core@25.12.0` 的 WebDriver BiDi 成功启动并显示本地登录页；后续需在冻结交付上执行实际回归，不能以此单页 smoke 宣称 AC-063 PASS。本机 Google Chrome 实际版本为 154.0.8037.59；完整路径统一记录于 `var/playwright-browsers/environment.json`。

BILLY 已另建为仅原件的本地样本：物品 `01a0f8cb-1621-70a3-8dea-27cb2f8ec4b5`、文档 `01a0f8cb-16ab-7252-9884-c0a1e97bc9ee`，真实上传/绑定/下载字节哈希匹配官方原件，任务数为 0；没有调用准备、报价或供应商接口。用于 PC-02A 从无准备记录的物品直接阅读 PDF；入口尚待实现和验收。证据 `var/manual-samples/billy-original-evidence.json`。

PC-06 独立 QA round 2 的 10 项 AC 已通过。使用冻结前端和普通 `embedded-ui` 构建（无 job-failpoints）更新预览并重启，构建 SHA-256 `a5ad525c2042e2936aea4772560d7d2547ae97a126529c65a806141edd94e649`，已通过 cold smoke-bootstrap；这不是最终发行验收。重启前只有一条既有 succeeded 历史任务，无非终态任务或 unknown 提交；没有删除历史资料。重启后 health、登录、设置与三份样本访问通过，模型问题字段 active/saved 都已遮蔽，pending=false；专用密钥字段仍未配置，generation=false，未自动移动或改写用户误填值。证据 `var/prd-completion/pc06-normal-build/{build.json,preview-health.json}`。

真实多视图候选已补齐到 Wii U：任天堂[官方操作手册](https://csassets.nintendo.com/noaext/image/private/t_KA_PDF/WiiUP_ENG_FINAL.pdf)，34 个 PDF 实际页、4,791,814 字节、未加密，SHA-256 `cf0164606089f2da06a6edb9104d4ba6d884ace2ac64c4bda5d05940cb690294`。主机部件图位于第 7 实际页（印刷页 8–9）。[Brittany McCrigler / iFixit 拆解指南第 2 步](https://www.ifixit.com/Teardown/Nintendo+Wii+U+Teardown/11796)的白色主机正、背面实拍，经视觉核对按钮与端口布局一致；不猜未核验的硬件编号、序列号或容量。照片保留原作者彩色端口标记及双手，属于后续生成质量的已知输入限制，未修图。来源、原始 CDN 字节与哈希保存在 `var/manual-samples/wii-u/`；遵循 iFixit [CC BY-NC-SA 3.0 许可说明](https://www.ifixit.com/Info/Licensing)，仅本机非商业资料测试、不用于模型训练、不加入仓库 fixture，尚未向供应商发送。本地上传/准备结果另记，不能以来源核对代替 T23 通过。

Wii U 随后完成真实本机导入：物品 `01a0f8e4-8162-7217-9010-bab19107c292`、文档 `01a0f8e4-82c7-74b0-a968-bde72bbc68dd`、准备记录 `01a0f8e7-8b7f-7405-8d80-73ce10714e1a`。真实 PDF.js 渲染并依序上传 34 页，ready/clientDerived=true、无缺页；34 份 JPEG 共 8,296,914 字节逐一解码/核对尺寸及 SHA-256/ETag，34 份文本共 95,103 字符，无替换字符。正背照片原 CDN 字节上传/绑定/回读哈希一致。该物品 jobs、snapshots、provider_attempts、ledger 数量与预留/实际费用前后全部 0；外网请求、敏感请求与 pageerror 均 0。证据 `var/manual-samples/wii-u/local-preparation-evidence.json`，脚本 `import-prepare-local.mjs` 可按现有 ID 恢复，未报价或生成。

PC-02A 已交付 RD_READY 并冻结 114 个前端文件（`var/prd-completion/web-snapshots/pc02a-rd-ready/source-hashes.json`），独立 QA round 3 验收四项 AC。协调者允许下一独立卡 PC-03A 的唯一 RD 与冻结测试并行，以减少等待；QA 使用复制后端和冻结前端，不从正在编辑的源码签发旧卡结果。

PC-02A 独立 QA round 3 随后四项 AC 全 PASS（6 浏览器场景、30 冻结组件测试）。另外对真实预览 BILLY/TR-808/Wii U 在 Chrome 154.0.8037.59、375/1440 六组产品入口补证：概览→原件指定页→翻页→返回原资料行焦点均通过；无数据库准备/资产/任务/费用变化，BILLY仍无preparation。证据 `var/manual-samples/pc02a-product-reading/{evidence,visual-review}.json`。窄屏扫描或双页 spread 的细字较小，只有页图正确可达结论，没有专用放大、OCR或舒适细读的额外承诺。

AC-061 性能 seed 已经由协调者在隔离本机假供应商上完成真实 HTTP 生成/复核/发布造数，30份合成物品、一个恰好100000面/16×16纹理的模型；`var/prd-completion/performance/seed-100k-003/seed-result.json`。初始生成器100352面被生产100000面预算正确拒绝，改的是忽略目录中的私有250×200网格生成器，没有放宽产品预算。五分钟真实GPU测量及最终版本性能验收仍未执行。

正式包冒烟输入已准备：从停止的合成 seed 独立复制，仅在复制库删除29条无资产的合成列表占位物品，使既有 smoke 的首物品规则命中发布样本；原性能库与预览不变。以普通 PC-06 中间构建生成合法脱敏备份，在冷目录执行完整七步 smoke（认证/嵌入资产/Range/断外网读取/重启持久/再次备份恢复）全部通过。`var/prd-completion/macos-smoke-input/{provenance,pc06-reference-smoke}.json` 记录身份和指纹。此项证明测试输入与工具链可用，最终 release 构建仍必须重跑，不能据此关闭 AC-064。

为 PC-03A 迁移后实测准备了 TILLREDA 旧格式中断记录：物品 `01a0f8f9-c6a0-74bf-937f-96475449bf45`、文档 `01a0f8f9-c722-706a-a5c2-8808cb277a51`、preparation `01a0f8f9-d3d6-753f-895c-14e11450eda5`。完整32页官方原件真实上传；冻结PC02A浏览器以实际PDF.js处理并保存1～3页后点击既有取消，状态preparing、数据库总页数仍未知；页图回读SHA/ETag通过。脚本只在第3页成功后暂缓后续PUT以稳定停止边界，不伪造成功内容；前两次拦截方式失败留存于脚本记录，复用同一物品与已保存页。jobs/attempts/ledger仍0，未报价/封存。`var/manual-samples/tillreda-partial-evidence.json`。下一步须在新后端迁移后用全新浏览器发现并只补4～32页，此前不能计为新恢复功能通过。

PC-03A A2 于 round5 独立验收通过（8 场接口/浏览器、24 项组件回归）；round4 的键盘焦点缺陷与失败证据保留，BUG-PC3-001 已由 QA 关闭。普通 `embedded-ui` 构建 SHA-256 `102ab3587c81ddc9cd1c23dca237bd2dad54fa03fa09c04a594b9b794f83d41f`，内嵌 A2 冻结的 115 份前端源码，未混入正在开发的 PC-02B。最小冷启动检查通过；停止旧服务后，完整复制并逐文件核对 154 份本地数据文件，私有回滚副本在忽略目录 `var/prd-completion/pc03a-a2-normal-build/preview-before-schema8`。启动后 schema7→8，12 张业务表原列内容和配置字节均保持不变，8080/5173 健康。本项不是最终发行验收。

迁移后的真实恢复实测已完成：Chrome 154.0.8037.93 的新 context 仅复用正常认证 cookie，没有本地准备指针或查询缓存，直接从服务端发现 TILLREDA 的旧 3 页记录。点击继续后真实 PDF.js 只 PUT 第 4～32 页，原 1～3 页记录及页图 SHA 不变；32 页齐全后仍为 preparing，明确点击封存才变 ready。另用新 context 逐一打开 TILLREDA/LACK/TR-808/Wii U，4 份 ready 均只读取记录，无准备写入、无 PDF/页资产读取。全库 preparations 仍 5、页数 90→119、资产 157→214（第31页空白无文字资产）；历史 jobs/snapshots/attempts/ledger/drafts/releases 计数不变，没有新增付费任务。浏览器外网/敏感接口请求与 pageerror 均为 0。证据与截图 `var/manual-samples/pc03a-preview/evidence.json`；真实官方文件仍仅在忽略目录，未调用供应商。PC-02B 审核引导继续独立开发。

后续内容验收另有小范围人工参考：`var/manual-samples/t23-reference-baseline/reference-baseline.md`，对照 Wii U 物理第7页与两张原始实物照片记录9个可见部件，TR-808扫描物理第4/24页人工识读3项。页面渲染、原件与照片哈希在同目录；明确不能由照片证明序列号、容量、缺少的侧面几何或操作功能。这不是生成结果或 T23 PASS。

性能工具短前置发现旧合成地形面绕序与法线反向，已只修忽略目录的私有生成器，并用真实本机适配器流程建立新 seed004；未修改生产面数限制或材质。新模型 SHA `48c260c0ae14f0b70e0b17be373d67ba6091e8eaa7545d8da04489eedd0378a4`，100000面均正向、无退化、16×16贴图、单面材质，初始画面已目视可见。`performance/preflight-pc06-006` 真实Chrome154.0.8037.93 / M1 Metal GPU前置通过且进程清理完成；明确没有运行预热或五分钟采样，300秒/33ms/200ms验收阈值未改。最终验收调度清单在 `var/prd-completion/final-validation-plan.md`，不以中间构建证据关闭最终门禁。

PC-02B 审核待办于 round6 独立验收通过：6项接口/浏览器、26项组件；受影响 PC-02A 原文阅读在 B 冻结版完整6场复验通过。5173 现固定运行 `pc02b-rd-ready` 的118文件快照（manifest SHA `cdd4561700e236365e4d59f6c14814ebd9ed7f6a6aec5fd4192afbfba47e3145`），避免未完成的下一卡接口影响用户预览；8080 仍为普通A2/schema8二进制，本卡无后端变更，原配置和官方样本未动。服务身份见 `var/prd-completion/preview-current.json`。PC-03B 开发继续；最终普通发行包仍需包含全部最后前端重新构建。

PC-03B 于 round7 发现 BUG-PC3-002（正常发布后摘要误推荐复核）；唯一RD以同事务发布审计精确识别发布后版本，独立 round8 真实9场/41项Rust/最终22项组件通过，缺陷由QA关闭。组件首轮21/22与定向失败保留，最终只修测试helper等待控件可操作，不改产品或验收断言。122文件B2生产前端与普通后端已更新到5173/8080；普通binary SHA `02029867d46471cf81351e1978db9332a50deafb96b1c8f64ea021011f21f64d`，无job-failpoints。升级前完整43,969,516字节/213文件副本已校验，12张业务表逐行摘要和配置字节保持不变。详情在 `var/prd-completion/pc03b-b2-normal-build/` 与 `preview-current.json`。

本轮普通候选冷启动发现xtask错误选取字母排序首个JS预加载文件，导致遗漏真正模块入口的PDF worker。已单独修复为检查全部直接JS/CSS与全部HTML JS根的引用图，缺失chunk仍失败；3项工具回归通过。root用复制xtask与普通B2、v2合成备份独立执行7步冷启动/离线读取/重启/备份恢复全部通过（5.58秒）。仍为中间预览证据，最终源码发行包须再构建验收。

当前进入PC-04受控预算验证命令开发，之后继续PC-05A/B/C。2026-10-02再次仅读取配置状态确认Tripo与ManualAI均未就绪、ManualAI模型字段仍suspectedCredential；没有读取或输出该字段值、移动密钥或真实外呼。配置及明确具名预算齐备前，不将fixture链路作为真实T23通过。

### 2026-10-02 续办：PC04独立验收通过

受控预算验证命令 PC04 已通过独立 QA round9 的7项AC（8组CLI/localhost验收，119个不同复制Rust回归）。真实密钥回显的fixture误用被正确判为未知并保留预留；修正成功fixture、保留原负例后的两组复核通过。原失败与所有边界保存在qa-report round9，不冒充一次全绿或真实供应商通过。普通embedded-ui候选 SHA `cf43e525ebbd5e01f700d28f886663a848e4c989bc6e60d368d91871fe7cdbf4` 已隔离复制并通过7步冷启动/断网/备份恢复，未部署。预览仍是已经验证的PC03B B2。

继续按PRD2实施PC05A搜索/发布版入口，然后PC05B保存与离开保护、PC05C任务/登录体验及最终全量门禁。协作工具无法恢复旧RD任务（thread limit），现存ui_pc任务已明确接任唯一生产RD；这只是人员调度，未改变产品或验收范围，QA仍独立。真实provider/model/价格/明确案例预算与Linux环境未就绪的事实保持，不标done。

### 2026-10-02 续办：PC05A通过并更新预览

搜索、分页范围恢复与最新发布版入口在独立QA11通过；QA10发现的快速清除后退导致输入框与URL不一致问题 BUG-PC5-001 已修复并由QA关闭。原六组在真实Chrome154.0.8037.93一次全过，冻结组件11项通过；原21项Rust证据按不变源码与二进制哈希复核复用，不计为新执行。失败历史保留。

普通embedded-ui A2构建 SHA `a38eaf45bb73ca792c733cff54b71cf6b8910c27febb07784afb76605a6ea62c` 通过七步冷启动测试。更新前完整备份213文件/43,970,504字节；12张业务表逐行摘要及加密配置字节更新后不变，schema仍8。8080后台与5173前端现在都对应A2冻结版（125文件，manifest SHA `b5cfb55ce19d5ae5d19eda7849b4f9b50aaae2b11a4929341eef9743a67b4646`）。真实预览只读验证搜索“Wii U”由7件正确筛为1件，并关闭临时验证标签页；用户原标签页保留。证据在 `var/prd-completion/pc05a-a2-normal-build/` 和 `preview-current.json`。

PC05B保存/离开保护继续开发，独立QA并行准备测试，尚未签发B或最终全量PASS。预览不运行B未完成源码，未调用真实供应商。

### 2026-10-02 用户补齐配置后的重启

用户确认已填写后，协调者核对无运行中任务或未决提交并重启当前A2后台。07:41 UTC只读脱敏状态：pending=false，两家供应商均configured=true，模型问题均null，密钥来源均为web；后台与前端代理健康。未输出地址、模型或密钥值，未向供应商发送请求。此前“密钥缺失/模型疑似误填”已解除。

当前priceCatalog.configured=false，generation=false；连接有效性、实际计费及真实T23仍未验证，不能以配置就绪冒充真实生成成功。价格目录须对应实际供应商/模型计费，不能直接把示例价格作为真实上限。RD与QA已恢复PC05B剩余检查，继续采用隔离合成资料，不使用刚保存的真实凭据。

### PC05B 预览更新（2026-10-02）

QA12 独立通过后，5173 已切到冻结 B3 前端（127文件，cb4419fc）。新普通 embedded B3 的用户实例启动等待 macOS 原生钥匙串授权，未监听8080；协调者停止该等待进程并恢复已获授权的普通 A2 后台 a38eaf45。两版211个 Rust 后台源文件完全一致，B3仅前端变化；没有修改钥匙串ACL或导出主密钥。当前后台会话68501、前端74488，ready通过，12张业务表与加密配置字节和更新前备份一致，新API设置仍生效。刚才系统授权提示暂不需要处理。B3正式包冷smoke结果与用户实例钥匙串授权状态分开记录，不将混合预览说成B3 embedded运行通过。

PC05C正在独立开发/验收准备；真实T23仍等待用户网关价目表与具体case预算授权，未发起供应商请求。
