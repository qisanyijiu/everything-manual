# 受控本地生成验证

`cargo xtask test-live` 针对已经初始化、停服的**专用隔离实例**；资料须已上传、照片视图已标记、说明书 preparation 已 ready。它不下载任意来源链接，不准备 PDF，不自动复核知识、确认热点或发布。

本地实例的操作系统访问权是管理边界；命令持有与 `serve` 相同的排他锁，不能对运行中的常用库执行，也不会伪造网页登录会话。供应商凭据继续通过该实例现有加密配置或环境注入，禁止放入命令参数、案例、授权文件和报告。配置文件必须明确位于该实例目录，避免工作目录配置回退。

```sh
cargo xtask test-live --plan --case /绝对路径/案例.json
cargo xtask test-live --case /绝对路径/案例.json --budget-file /绝对路径/授权.json
```

第一条输出「计划预览 · 未执行」。计划复用服务器报价校验/页批次/整数价格计算，临时报价在事务中回滚；不创建任务、预留或供应商请求。计划包含可核对的 `caseHash`、`planHash`、`inputHash`、模型、页批次、实际页资产hash、照片视图、独立 `requiredLimits.creditMinor` 和 `requiredLimits.usdMicros`。不把币种相加，也不把缺失价格估成0。

执行必须另有明确授权；`allowed=false`、缺文件、公开权限、过期、未知字段、计划或资料变化、任一币种上限不足都会拒绝。真实供应商调用还需要会话中的独立用户授权，代理不能自行开启真实收费授权。默认 `check` / `e2e` 不运行真实供应商。

## 案例合同 v1

每层都拒绝未知字段；ID必须为数据库中实际UUID，hash必须是64位小写SHA-256。具名ID是1～100个ASCII字母/数字/下划线/横线，拒绝疑似密钥；型号、模型等不接受凭据或URL。照片2～4张，含front及至少一个侧面，不能有重复ID/视图或detail。以下是需替换实际资料的模板，示例自身不是可执行案例：

```json
{
  "schemaVersion": 1,
  "caseId": "manual-verification-001",
  "mode": "real",
  "instance": {
    "instanceId": "dedicated-validation",
    "dataDir": "/绝对路径/专用实例",
    "configFile": "/绝对路径/专用实例/config.toml"
  },
  "material": {
    "itemId": "替换实际UUID",
    "itemModel": "实际物品型号",
    "documentId": "替换实际UUID",
    "sourceSha256": "替换实际64位hash",
    "preparationId": "替换实际ready UUID",
    "photos": [
      { "id": "替换实际UUID", "sha256": "替换实际64位hash", "view": "front" },
      { "id": "替换实际UUID", "sha256": "替换实际64位hash", "view": "left" }
    ]
  },
  "generation": {
    "modelPreset": "使用现有价格目录的预设",
    "tripo": { "identity": "tripo", "model": "实际配置模型" },
    "manualAi": { "identity": "manual_ai", "model": "实际配置模型" },
    "priceVersion": "实际目录版本"
  },
  "outputDirectory": "/绝对路径/已创建的独立结果目录"
}
```

`identity` 固定标识现有适配器。实际endpoint、配置代次、凭据绑定和价格目录内容被不可逆摘要纳入 `configurationHash` / `planHash`，不输出凭据、地址或秘密路径；同版本价格内容变化也需重新核对。实例绑定包含目录定位与现有管理员身份，不能复制到另一实例复用授权。结果目录必须独立于实例，不能位于实例内部或是实例的父目录。

## 授权合同 v1

普通文件，Unix必须0600或更严格，拒绝软链接、非常规文件和超限文件。两个金额均为非负JSON整数，拒绝小数、字符串、负数及i64溢出。

```json
{
  "schemaVersion": 1,
  "authorizationId": "authorization-001",
  "caseId": "manual-verification-001",
  "caseHash": "复制所核对计划的caseHash",
  "planHash": "复制所核对计划的planHash",
  "allowed": false,
  "expiresAt": "2099-01-01T00:00:00Z",
  "limits": { "creditMinor": 0, "usdMicros": 0 },
  "maxInitialGenerations": 1,
  "retryScopes": []
}
```

示例保持未授权及零上限；到期时间和实际预算由授权人确定，不能把模板当作真实授权。字段值须与实际计划一致。`maxInitialGenerations` 必须为1。空 `retryScopes` 不允许付费阶段的额外安全提交重试。安全查询/取回已接受任务仍复用引擎原有查询策略。

唯一支持的可选范围是明确未受理、可证明安全的原阶段重试，`operation` 固定 `safeRetry`，例如：

```json
{
  "stageKind": "manual_extract",
  "batchIndex": 0,
  "stageInputHash": "复制计划stages对应阶段的hash",
  "operation": "safeRetry",
  "maxAdditionalAttempts": 1
}
```

`stageKind` 只接受 `manual_extract` / `tripo_submit`；同一具体阶段不能重复授权，次数1～5，仍受原引擎5次上限和原退避约束。没有 `all`、更换模型、提高质量或强制重购操作。unknown绝不因可选范围或剩余预算而再次购买；需要原有对账流程。已证明未受理的重试不构成额外计费，上界仍是一次已受理生成的两币种上界。

## 恢复与结果

授权校验后先在实例 `live-authorizations/` 受限目录原子保存固定quote/body/key，再调用原建单事务。任务、远端ID、账本仍以既有数据库事实为准。重跑使用同一授权文件和案例；并发进程由实例锁串行化，后到进程提示等待，不另买。报价在建单前过期时停止，不偷偷换quote/key。已受理任务可重放原业务身份并继续查询。过期或绑定变化时不外呼；不要通过删除journal“恢复”。

授权/配置/资料在执行过程中也会在新请求前重读；付费窗口和CDN重定向均有门禁。已发请求的结果仍可保存，不把停止进程等同供应商取消。中断后用同授权重跑；租约未过期时等待原恢复规则，submitting且无可信结果保持unknown预留。命令最多等待30分钟，届时保留任务并提示同授权恢复。

结果子目录为 `<caseId>-<授权ID摘要前16位>`，文件为 `model.glb`、`knowledge.json`、`report.json`（均受限文件）。结构化知识产物保留页码出处；报告只列身份/hash、阶段状态、已保存资产、未完成项及分币种费用。账本actual缺失保持null，不填0。部分失败非零退出，已有资产不删除；未知费用或超额风险不冒充成功。命令结束后仍须在网页人工复核知识、模型、热点，再显式发布。

`AC-042` / `T23` / `realSupplierAcceptance` 始终单列 `NOT_RUN`，不会把fixture或自动生成草稿冒充人工/正式包真实验收。

## 本机fixture验证

案例 `mode=loopbackFixture`；两provider必须是字面 `127.0.0.1` 或 `[::1]` 的HTTP地址，下载名单也只能是同样的字面loopback，开启专用测试配置 `allow_local_fixture=true`。提交客户端不跟随重定向；下载逐跳白名单和地址校验，loopback禁用环境代理。fixture授权不能命中公网或任意域名。

必须显式构建带既有 `job-failpoints` 的测试工具；普通发行构建不会因此开放本机下载：

```sh
CARGO_NET_OFFLINE=true CARGO_INCREMENTAL=0 cargo build -p xtask --features everything-manual/job-failpoints
```

该开关只供隔离fixture，不应用于正式发行。测试通过的输出标题是「本机链路验证通过；真实供应商验收未运行」。具体RD/QA实测命令与证据见本需求的 implementation.md / qa-report.md，本文不替代验收记录。

## 验收入口与隔离

CLI fixture集成用例默认标为ignored，必须先明确构建并指定测试工具，才调用临时实例上的假供应商；两项纯合同单测仍由普通Rust测试执行：

```sh
EM_PC04_XTASK_BINARY=/绝对路径/已冻结xtask cargo test -p everything-manual --test publishing test_live_cases::pc04_cli -- --ignored --test-threads=1
```

默认 `apps/web/playwright.config.ts` 只排除 `**/qa-pc*.spec.ts`：这些是持有独立冻结源码、复制二进制及专属端口的验收套件，必须通过各自配置运行，例如 `npm --prefix apps/web run test:e2e -- --config=playwright.pc04-qa.config.ts`。PC01/PC06/PC02A/PC02B/PC03A/PC03B对应同目录的具名QA配置。原 `qa-t16`、`qa-t18` 等基线仍由默认入口发现；排除默认发现不等于移除验收，最终快照由协调者逐专用配置复验。helper不能静默回退到旧快照或共享target binary。

崩溃恢复时阶段/attempt已unknown而账本仍reserved也是保留预留的有效状态：报告按数据库原事实列出state和actual=null，不擅自改账本或写0。
