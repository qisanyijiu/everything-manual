# ES-01 实施记录

- RD：2026-10-01；work-id `encrypted-secrets`，PRD revision 2（§3.4 原子发布澄清），UI revision 2。
- 状态：RD_READY，生产代码冻结等待独立 QA；本文不代替 QA 结论。无真实供应商请求、费用、真实用户凭据读取；未修改运行中的预览服务与数据目录。

## 实现

`config/encrypted_secrets.rs` 使用 ring 0.17.14 AES-256-GCM、32 字节主钥、OS 随机 12 字节 nonce。私有 Envelope 版本、算法、用途经 AAD 绑定，认证 tag 并入 ciphertext；外部文件与 overlay、两家 provider 不能互换。密钥上限 4096 字符、受限文件上限 256 KiB，覆盖 UTF-8/hex 膨胀。未知格式、认证失败、错主钥、截断、符号链接、非普通文件、不安全权限、超限均拒绝。

主钥选择：显式 `EM_SECRETS_MASTER_KEY` 恰好 64 个 hex 字符，非法值不回退；未注入时 macOS 原生默认 Keychain，service=`org.everything-manual.secrets.v1`、account=`master`，SecKeychainAddGenericPassword create-only，duplicate 读并发赢家，经典本机条目不云同步。其他平台无主钥时允许不需要持久秘密的启动/读取，首次秘密保存失败。无明文主钥文件、无环境注入复制到磁盘。读取只 load；同进程已经使用的主钥丢失/替换后不重新生成，clear/restore 也不能越过原主钥检查覆盖已存密文。

`ProviderConfigStore` 增加独立 Disk DTO。新 replace 只保存密文；keep 不复制环境 API key。serve 在 data-dir 锁后调用 `load_from(..., true)` 完整验证、加密、自验证并原子迁移旧 overlay；保持 revision、有效值与 AS 报价/任务代次。check 使用只读加载，旧明文只给迁移提示。AppState 直接构造仍无隐式 Keychain IO；注入边界为 `Secrets::fixed/unavailable/with_source/native` 和 `ProviderConfigStore::with_secrets/load_from`。

`encrypt-api-key --provider tripo|manual-ai --input <受限明文文件> --output <不存在的新文件>` 转换外部旧文件；原文件不修改、不删除，目标 0600，原子 hard-link 发布且拒绝覆盖。`api_key_file` 两家共用密文 reader，正常 serve/check 拒绝明文。无密钥命令行值参数。建议输出 `.api-key.enc`，已加入 Git 忽略，临时文件同样忽略。

文件发布前先 open/sync 父目录、写密文临时文件并 fsync，再 rename/link。提交前失败保留原内容和 revision；提交后目录同步失败保持已发布状态并输出固定安全告警，不能报告未保存。测试专用 `encrypted_secrets::test_faults::fail_directory_sync(path, after_commit)` 由既有 dev-only `job-failpoints` feature 提供精确路径 RAII 注入；没有生产环境开关，普通生产构建不含注入分支。

两家 client 在解析/返回/诊断之前检测已知 API key 的直接与 JSON 解码反射，敏感成功响应返回安全协议错误；4xx、429、5xx、Tripo 非零业务 code 分类保留且敏感消息丢弃。ManualAI 携钥 body 不会到达诊断资产。部署 URL 与 client 构造统一拒绝 userinfo、错误不复述 URL。SecretString、网页写入 DTO 中 apiKey、主钥、加解密/文件/响应临时缓冲在可控边界 zeroize。

UI 只增补两条加密说明和文档提示，沿用密码输入、dirty保护、失败编辑保留、no-store、不回显和重启生效语义。architecture/contracts/ADR-039/config.example 已同步；用户指南与 README/operations 由主会话维护。

## 实际验证

证据仅在 `artifacts/encrypted-secrets/rd/`。普通测试使用假密钥与显式 DI，不接触默认用户 Keychain。

| 命令/范围 | 实际结果 | 证据 |
| --- | --- | --- |
| `cargo check -p everything-manual --offline` | 两次通过 | 早期工具结果；后续 build/clippy 覆盖最终源码 |
| `cargo test -p everything-manual --offline --test encrypted_secrets_rd --test api_settings_rd` | ES 9/9 + AS 4/4 PASS | encrypted-tests.log |
| `cargo test -p everything-manual --offline --lib` | 179/179 PASS | rust-unit.log |
| config_cli / generation_requests / jobs_recovery / manual_ai_contract / tripo_contract / api_settings_rd | 12 + 27 + 25 + 20 + 18 + 4 PASS | rust-regression.log |
| `cargo xtask contracts --check` | OpenAPI/生成 TS 无漂移 | contracts.log |
| 前端 `npm run lint`、`npm run build`、`npm test` | lint/build PASS，152/152 unit PASS | 工具构建输出、web-unit.log |
| `cargo fmt --all -- --check` | PASS | fmt.log |
| `cargo clippy -p everything-manual --offline --lib --bin everything-manual -- -D warnings` | PASS | clippy.log |
| `git diff --check` | PASS | diff-check.log |
| `cargo build -p everything-manual --offline --features embedded-ui`（无 job-failpoints） | PASS，2026-10-01 23:56 CST 完成 | build.log |

首轮 localhost 回归被沙箱监听限制阻断，获工具授权后完整隔离复跑通过；不是忽略失败。新增失钥完整保存测试曾命中“同值 replace 是 no-op”的错误测试预期，修正为实际 keep/clear/restore 变更后 9/9 通过。初轮 clippy 的四条格式/迭代建议已处理，完整重跑通过。cargo 输出包含宿主旧 `.cargo/config` 命名弃用提示；前端有既有大 chunk 提示，无新增构建错误。

最终普通嵌入前端二进制：`target/debug/everything-manual`，构建参数只有 `embedded-ui`，未启用 `job-failpoints`。后续 cargo test 可能重新生成同路径测试用途二进制，更新预览前应由主会话正常构建确认。

## AC 对照与交接

| AC | RD证据/交付 | 独立 QA 需覆盖 |
| --- | --- | --- |
| 001 | 随机 nonce、相同密钥不同密文、两家 4096 中文字符网页保存→重载、不含明文 | 浏览器与实际进程消费 |
| 002 | 错主钥、用途交换、format/version/algorithm/nonce/ciphertext 篡改拒绝 | 独立恶意输入矩阵 |
| 003 | 原生 create-only/duplicate API、独立 native 构造/精确 delete | QA 已报告 native 预检跨两子进程 + 定向清理 1/1；正式证据归 QA |
| 004 | unavailable、Some→None 禁止 create、已有保存后 keep/clear/restore 丢钥失败；无秘密直接构造回归 | native 不可用与实际进程矩阵 |
| 005 | 完整旧 overlay 迁移保持 revision/有效值/ensure_revision/pending | 带实际旧业务快照的迁移 |
| 006 | check 只读、坏 JSON/无主钥迁移原文件不变；明确提交点与测试 fault guard | 前后 fsync 故障独立注入 |
| 007 | 外部 plaintext 拒绝、CLI 转换、新 reader/跨 provider 拒绝、源未改、target 0600/no overwrite | 两家 serve/check 使用新文件 |
| 008 | 文件 symlink、权限、非文件、错误主钥/输入固定错误；不覆盖 | 完整恶意路径/超限/写失败矩阵 |
| 009 | 原 AS keep/clear/restore 环境 key 不落文件继续通过 | 独立扫描 |
| 010 | 原 AS + 报价27 + 恢复25回归；配置代次保持；13 RD持久化相关通过 | 本机双供应商加密保存后真实重启 Authorization 消费 |
| 011 | direct/JSON 解码检测 + URL userinfo安全拒绝，两家合同20/18通过 | 真实反射 fixture 与 HTTP/log/DB/诊断/备份/browser全域扫描 |
| 012 | 最少 UI 提示、技术合同/架构/决策更新 | 浏览器 375/键盘/失败编辑保留与主会话运维指南 |

限制：未连接任何真实收费供应商；RD 未代跑 QA Keychain 专用测试或双供应商重启浏览器套件；不宣称任意编码反射检测、历史明文清除、磁盘块擦除、全部资产加密或全平台发行验证。主钥遗失无法恢复密文；普通备份仍不携带私有 overlay 或系统主钥。API key 环境注入仅内存使用。


## BUG-ES-001 修复（2026-10-02，待独立 QA 复验）

首轮 QA 判定 P1 / AC011：完整 JSON 中的已知密钥可以识别，但同样的完整或混合 Unicode 转义放入损坏的外层 JSON 时，旧检测把解析失败当作未命中。隔离 RD 负例实测 Tripo HTTP400 错误摘要与 ManualAI HTTP200 RawResponse 都保留可还原密钥。`malformed-reflection-repro.log` 只记录布尔结论，1/1 表示漏洞复现成功，不是验收通过；该探针现已改为正式回归，移除 ignore。

最小修复：检查函数更名 `response_requires_discard`，只对成功解析的完整 JSON 作解码检查，解析失败直接判为不可保留；不新增任意编码扫描。两家固定诊断为“供应商响应包含敏感凭据或无法安全解析，内容已丢弃”。Tripo 4xx 保持 Business/HTTP状态；429/503和可解析非零业务code分类不变。ManualAI 2xx 不安全 body 在 RawResponse 前返回 Unexpected，通过原有处理器保持 unknown/无自动重购，既不产生结果资产也不写原始诊断。

正式 RD 回归覆盖两种转义、有效无敏感2xx成功、有效4xx/业务code、429/503对照；原 `manual_ai_contract` 中要求非JSON HTML落原始诊断的旧断言已调整为 SubmissionUnknown、attempt unknown、无结果资产，并连续3次tick验证只请求一次。外层 JSON 合法但模型正文格式错误的既有验收保持不变。生产修改仅安全检测及两家client的变量/固定诊断文案。

修复后实际结果：

| 命令/范围 | 结果 | 证据 |
| --- | --- | --- |
| `cargo test -p everything-manual --offline --test encrypted_secrets_rd --test tripo_contract --test manual_ai_contract --lib` | ES **10/10**、Tripo **18/18**、ManualAI **20/20**、lib **179/179** PASS | `artifacts/encrypted-secrets/rd/bug-es-001-regression.log` |
| `cargo fmt --all -- --check` | PASS | `bug-es-001-fmt.log` |
| `cargo clippy -p everything-manual --offline --lib --bin everything-manual -- -D warnings` | PASS | `bug-es-001-clippy.log` |
| `git diff --check` | PASS | `bug-es-001-diff-check.log` |

**RD_READY，修复生产代码再次冻结，BUG-ES-001 待独立 QA 复验关闭。** 未重跑完全无关前端；无 DTO/HTTP 路由变更。首轮证据保留，未以修复后测试覆盖漏洞证据；预览仍未操作，主会话将在 QA 后重新正常构建。
