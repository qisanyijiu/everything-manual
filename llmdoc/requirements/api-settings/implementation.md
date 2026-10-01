# AS-01 实施记录

日期：2026-09-21；依据 request_revision 1、PRD revision 1、UI revision 1。RD 实现完成，自检通过后冻结交独立 QA；本文不代替 QA 验收，不改变 state 或 T22/T23 状态。

## 实现与范围

- `config/provider_overrides.rs`：两家供应商整体私有覆盖，revision CAS，keep/replace/clear/restore，URL/模型/Authorization 格式检查；Unix 0600 临时文件同步并原子替换，拒绝符号链接/损坏或不安全读取。无覆盖文件不改变部署行为。keep 不复制部署密钥，clear 不回退，恢复只撤销目标供应商；空覆盖仍保留新代次。
- `config/commands.rs`、`http/state.rs`：serve 排他锁后读取覆盖，再向 HTTP 与 worker 装配同一运行 Settings；check 验证已保存覆盖。Settings 结构未增加字段，旧直接构造不做隐式文件 IO。测试或嵌入者应显式 `ProviderConfigStore::load(&mut settings)` 后 `AppState::new(...).with_provider_config(store)`。
- `http/settings.rs`、`http/dto/settings.rs`、OpenAPI/生成 TS：新增 GET/PUT `/api/v1/settings/providers`，旧 `/settings/status` 原字段仍代表运行基础配置，新增 providerConfigPending。成功读取/写响应 no-store，密钥只报存在状态与类别。含秘密请求的 JSON 结构错误固定消息，不引用原字符串；旧部署 URL 展示剥离 userinfo/query/fragment。不存在连通性探测或保存时上游请求。
- `generation/estimate.rs`、`generation/jobs.rs`、`http/estimates.rs`、`http/jobs.rs`、`jobs/control.rs`：共享配置读写锁覆盖保存与接受动作；pending 拒绝新报价/确认/生成/恢复。配置代次进入报价指纹与不可变生成快照，历史缺省为 deployment。旧报价/失败任务不能跨配置使用，同键同 body 幂等重放先读取原结果。任务详情的 retry 原因同源呈现。
- `jobs/executor.rs`：生产 worker 执行前也校验当前共享 store 与 snapshot 代次，覆盖恢复库缺失私有覆盖时的自动执行边界。锁顺序为配置锁后数据库连接，避免连接池耗尽互等。
- `SettingsPage.tsx`、新增 `ProviderSettingsForm.tsx`、端点与主题样式：双卡、运行/保存/编辑状态、密钥三操作、恢复与取消恢复、整体保存、字段错误、修订冲突重读、失败保留输入、页面局部站内/浏览器离开保护。密钥只赋 password 实时 value 属性，不赋 HTML value attribute；成功或退出替换/恢复时清空。确认页新增 pending/代次失效的原因与设置/重新报价入口，保留 interaction-a 的确认状态守卫。

## HTTP 与复验入口

`PUT /api/v1/settings/providers` body：`{ revision, tripo, manualAi }`。每家 `{ action: "update", baseUrl, model, keyAction: "keep"|"replace"|"clear", apiKey?: string }` 或 `{ action: "restore" }`。响应 `{data:{revision,pending,active:{tripo,manualAi},saved:{tripo,manualAi}}}`；每家包含 baseUrl/model/keyConfigured 与三个来源类别。revision 是非秘密、不透明 UUID（初始无文件为 deployment）。

- 冲突：409 REVISION_CONFLICT，details.reason=providerConfigConflict。
- 门禁：422 VALIDATION_FAILED，reason=providerConfigBusy / providerConfigPending / providerConfigChanged。
- 字段：422 details.fields；持久化：安全固定 500，不暴露路径或输入。
- 故障注入无需生产 failpoint：在隔离 data-dir 下把 `provider-overrides.json` 建成目录，可验证持久化失败及表单保留。损坏文件或符号链接验证启动拒绝；不要操作用户预览数据目录。

## 自检结果

以下均为实际命令结果；未使用真实供应商或付费请求，未重启用户 8080/5173，未提交 Git。

| 命令 | 实际结果 |
| --- | --- |
| `cargo test --offline -p everything-manual --lib config::provider_overrides` | PASS，2/2：URL、密钥格式与 Debug 脱敏 |
| `cargo test --offline -p everything-manual --test api_settings_rd` | PASS，4/4：认证/CSRF/结构脱敏、keep/clear/restore/重载/权限/CAS、格式及磁盘失败原子性、损坏/符号链接/部署 URL 脱敏 |
| `cargo test --offline -p everything-manual --test generation_requests --test api_settings_rd --test auth_api --test jobs_recovery` | api_settings_rd 4/4、auth_api 13/13、generation_requests 27/27；jobs_recovery 初次沙箱仅17/25，8项因禁止绑定127.0.0.1测试端口失败，不计PASS |
| `cargo test --offline -p everything-manual --test jobs_recovery`（允许本机fixture端口复跑） | PASS，25/25；无真实上游 |
| `cargo test --offline -p everything-manual --test generation_requests api_settings_` | PASS，2/2；最后增加 worker 旧快照阻断断言后定向复跑通过 |
| `npm test -- --run src/features/settings/ProviderSettingsForm.test.tsx` | PASS，5/5：读取失败、密钥DOM属性/422焦点与留存、防重/成功、恢复生命周期、冲突丢弃重读 |
| `npm test`（apps/web） | PASS，22文件152/152，包含 interaction-a 的既有回归 |
| `npm run lint` | PASS，退出0 |
| `npm run build` | PASS，tsc --noEmit与Vite均通过；保留既有大型PDF/3D分包提示 |
| `cargo xtask contracts --check` | PASS，OpenAPI与生成TS无漂移 |
| `cargo build --offline -p everything-manual --features embedded-ui` | PASS，退出0；不启用job-failpoints，产物未替换用户运行进程 |
| 本批Rust文件定向rustfmt，随后 `cargo fmt --all -- --check` | PASS，退出0；未手改QA独立测试 |
| `git diff --check` | PASS |

构建、格式与合同自检完成，结果 **RD_READY**，生产代码冻结等待独立QA。未重复运行历史 web-mvp E2E，无历史截图或日志覆写；长证据如新增只允许 `artifacts/api-settings/rd/`。

## AC 映射与独立 QA 待验

| AC | RD 已实现/验证；仍由 QA 独立验收 |
| --- | --- |
| 001–002 | 双状态读取、来源、失败重试、认证/CSRF/no-store/无秘密响应；RD API/单元通过 |
| 003–005 | 整体保存、字段校验、失败留存、CAS/防重、原子文件；RD API/表单单元通过 |
| 006–008 | keep不复制、replace私有保存、clear屏蔽、restore单家撤销、重载及0600/失败关闭；RD模块和路由验证，真实进程重启待QA |
| 009 | HTTP/worker同配置启动装配已完成；按协调者安排，本机A/B fixture真实保存→重启→模型/Authorization匹配由随后独立QA统一执行，RD未宣称已跑该进程链 |
| 010–012 | 活动/未决门禁、保存与建单并发、pending、旧报价/旧failed retry、幂等及worker恢复代次；RD真实SQLite回归通过，更多终态残留/并发排列交QA |
| 013–014 | 页面局部离开dialog及beforeunload、临时密钥不缓存、手机单列44px目标/键盘/错误焦点；RD单元验证核心行为，真实375/1440布局与浏览器后退/刷新由QA验证 |
| 015 | 保存不探测，基础配置齐备仍需报价校验、pending明确暂停，未知模型仍走现有价格目录拒绝；实际fixture不可达/未知模型重启场景交QA |

## 技术知识、限制和交接

新增非代码知识已同步 `llmdoc/architecture.md`、`llmdoc/contracts.md`、`llmdoc/decisions.md` ADR-038，并调整 `config.example.toml`。Settings 没有字段级部署来源记录，所以界面准确标“部署配置（含默认值）”，不通过值相同推断默认来源。网页覆盖不包含价格目录、TLS、监听、目录或下载策略。

网页路径保证任务/代次切换；部署者离线改环境变量、TOML 或私有文件的意图不在此保证范围。备份/导出仍不带私有配置或密钥，迁移需重新配置，带旧代次的活动快照不得自动改发新服务。保留输入失败重试仅限当前已认证页面，不跨登录或持久化恢复。未验证真实收费服务、任意兼容协议、连接/余额/模型真实性；不新增网页重启或“测试连接”。

保留全部 interaction-a 和用户先前暂存/未暂存改动；本批不修改 PRD/state，QA独立测试文件由QA维护。文档指南和 README/llmdoc索引由协调者维护，RD未争写。

## 协调者本地预览交付检查（2026-09-22）

已将 RD 构建的普通 `embedded-ui` 二进制复制到 `var/preview-api-settings-bin/everything-manual`（未启用 `job-failpoints`），正常停止旧预览后，使用原 `var/preview-20260919` 数据目录与 `127.0.0.1:8080` 启动。原 Vite `127.0.0.1:5173` 继续代理新服务，未覆盖发行产物或写入测试供应商配置。

`/api/v1/health/ready` 的进程、数据目录、数据库和迁移检查均为 ok。真实内置浏览器已打开 `/settings`，确认原登录保留、Tripo / 说明书 AI 两张卡与缺项状态可读，并目视检查首屏。实际配置保存、重启及 worker 消费由独立 QA 在隔离数据目录验证，验收结论见本批 `qa-report.md`。

该本地预览的再次启动命令（先正常停止当前同目录服务）：

```sh
./var/preview-api-settings-bin/everything-manual serve --data-dir ./var/preview-20260919 --listen 127.0.0.1:8080
```
