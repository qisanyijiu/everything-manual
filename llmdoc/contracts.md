# 接口、数据和状态合同

版本：1.0 · 2026-09-11 · 状态：待实现的规范，不是现有 API。技术选型与取舍见 [架构](architecture.md)，任务映射见 [实施路径](implementation-plan.md)。T01 创建 Rust DTO 和 OpenAPI，之后本文主要保留语义与不变量，不手动维护第二套生成类型。

## 1. 统一约定

- URL 前缀 `/api/v1`；JSON 为 camelCase，Rust／SQL 为 snake_case；UTF-8。
- 应用实体 ID 由服务器生成 UUIDv7 字符串。供应商 ID 是 opaque string，不验证成 UUID，不截断／去掉前缀。
- 时间 UTC RFC3339；页码 `pageNumber` 从 **1** 开始。旧 macOS 文档中的 0-based 页索引不适用新 API；接 PDF.js 时统一检查，不重复加一。
- 金额不使用浮点运算：Tripo `creditMinor` 是 1/100 credit，USD `usdMicros` 是 1/1,000,000 USD；供应商小数字面量用精确 decimal 解析再转换。配置价格快照并保留原始已脱敏计费字段。
- 单项响应 `{ "data": ... }`；列表 `{ "data": [], "nextCursor": null }`，默认 20／最多 100 条，游标包含稳定排序键及 ID；不返回裸数组。所有 API 时间、枚举和 nullable 字段在 OpenAPI 中声明。
- 错误统一 `error.code/message/details/requestId`；不向用户输出堆栈、SQL、密钥或完整供应商签名 URL。
- 可编辑聚合根带整数 `revision`，GET 返回 `ETag: "r7"`。PATCH、确认、发布、重试／取消等修改要求 `If-Match`，并在数据库事务内校验；缺少返回 428，冲突返回 412。不能读出 revision 后无条件更新。
- 创建资源的成功 201；后台入队 202；无内容删除／注销 204。401 未登录，403 无权限／CSRF，404 不存在，409 业务／幂等冲突，413 超限，415 类型不支持，422 内容校验失败，429 限速，503 服务未就绪。
- 413 同时用于"请求体积超限"与"磁盘预留空间不足"两种情形；后者在 `details.reason` 标记 `insufficientStorage` 并给出所需／可用字节，不新增 507（决策见 PRD 修订 2 A-14 / ADR-017，2026-09-12 由协调者补记）。

错误示例：

```json
{
  "error": {
    "code": "REVISION_CONFLICT",
    "message": "该草稿已更新，请刷新后重试",
    "details": { "currentRevision": 8 },
    "requestId": "01993000-0000-7000-8000-000000000001"
  }
}
```

## 2. 数据对象与数据库约束

所有表的建表／索引由 `migrations/` 追加维护。以下列核心字段，通用 id/created_at/updated_at 略；不是允许漏建外键和唯一键的概念图。

| 表／对象 | 核心字段 | 必须保证 |
| --- | --- | --- |
| admins / sessions | password_hash；session_token_hash、expires_at、csrf_hash | 无默认密码；会话明文只返回 cookie，不落库／日志 |
| items | name、brand、model、variant、revision、archived_at | 名称／型号必填；同品牌型号不强制唯一，允许不同配置 |
| blobs / assets | sha256、size、mime、storage_state；blob_id、item_id、purpose、original_name | sha256 唯一内容存储；所有访问先验证 asset 归属；文件名不是路径 |
| documents | item_id、source_asset_id、source_sha256、title、source_url | URL 只作出处，不触发任意服务端下载 |
| preparations / pages | document_id、source_sha256、state、page_count；preparation_id、page_number、text_asset_id、image_asset_id | 页号唯一且完成时连续 1..N；ready 后不可修改 |
| photos | item_id、asset_id、view、revision | view 为 front/left/back/right/detail；detail 不直接进入多视图 API |
| generation_snapshots | item_revision、preparation_id、photo_ids+hashes、provider_config、prompt_version、price_version、budgets | 输入不可变；不保存 API key；编辑原物品不改变已开始任务 |
| jobs / job_stages | item_id、snapshot_id、status、revision；stage_kind、batch_index、page_set、input_hash、result_asset_id、usage_json、status、lease_owner、lease_epoch、lease_until、next_run_at、attempt_count | 每 job+stage_kind+batch_index 唯一；非批处理 batch_index=0；状态推进条件包含当前租约 epoch |
| provider_attempts | job_id、stage_id、request_hash、submit_state、remote_task_id、response_id、started_at、last_error | 先存 intent 再请求；远端 ID 只允许 null→值或同值，不覆盖不同值；同步 response_id 不等于可轮询任务 |
| idempotency_records | admin_id、method、route、key、body_hash、resource_id、response_status | 范围内唯一；同 key 不同 body 返回 409 |
| cost_ledger | snapshot_id、attempt_id、provider、currency、reserved、actual、state、price_version | 预留／结算／释放在事务内且幂等；unknown 保留预留 |
| model_revisions | item_id、asset_id、sha256、provider_attempt_id、bounds、validation_state | 模型字节不可变；校验通过才可进入阅读器 |
| manual_drafts | item_id、snapshot_id、model_revision_id、revision、status、knowledge_json、review_json | 聚合更新需要 If-Match；草稿不等于已发布版本 |
| manual_releases | item_id、draft_id、draft_revision、model_revision_id、manifest_asset_id | 不可变完整快照；发布后不引用会变的 draft 内容 |
| audit_events | entity_type/id、action、actor、result、metadata | 只存必要摘要；记录费用确认、人工事实修改、发布和重试 |

初版可把部件／步骤／热点作为版本化 draft 的 JSON 聚合存储，使用 Rust 类型完整校验；无需一开始拆几十张表。文本检索先按物品名称／型号，全文索引后续需求再做。

知识聚合的约束：

| 对象 | 最小信息与规则 |
| --- | --- |
| Part | id、name、description、evidence[]、reviewStatus；ID 由应用分配，AI 返回的局部标识先做映射 |
| Step | id、title、orderedActions[]、partIds[]、evidence[]、safetyNotes[]；顺序与依赖须可读，不凭外观编操作 |
| Evidence | documentId、preparationId、pageNumber、quote、bbox=null；页必须来自本次输入，扫描识别文字标识 derived |
| Hotspot | id、partId、anchor=null、status、cameraPose=null；anchor 非空时含 modelRevisionId、modelSha256、positionLocal；全部数值有限，禁止 NaN/Infinity |
| CameraPose | positionLocal、targetLocal、upLocal、fov；均相对同一 asset-root，fov 合理范围，视角不是机械动作 |
| Review | 实体级 confirmed/needs_review；人工修改标注 userEdited 和出处；modelReview=null 或 {modelRevisionId, modelSha256, loaded, userConfirmed, checkedAt} |

`bbox` 如有，为旋转后的页图上 `[x,y,width,height]`，0..1 归一化、左上原点，并保存页图尺寸／旋转。无法可靠提供时为 null，仍可跳到页；不得为了满足 schema 捏造框。

热点状态：`unbound → candidate → confirmed`；人工直接拾取可 `unbound → confirmed`。unbound 时 anchor=null，不能用 [0,0,0] 占位；candidate/confirmed 要求非空 anchor。模型 revision 不同必须进入 `stale` 并重新绑定，旧 anchor 可以保留用于解释但不得当有效热点显示。API 拒绝 confirmed 热点与当前模型 sha 不符。MVP 自动生成知识但不承诺自动 candidate。

`modelReview` 通过草稿 PATCH 写入：实际模型首帧成功后才能标 loaded，用户明确确认后才能标 userConfirmed；checkedAt 由服务器赋值。它是已认证用户的复核声明，不是服务端可证明的 GPU 测试。发布校验 loaded/userConfirmed 均 true 且 revision/hash 匹配，换模型清空该记录。不能由后台 CPU 校验自动替用户确认。

## 3. REST 路由清单

全部受会话保护，只有存活／就绪探针和登录例外。GET 会话未登录返回 401；健康探针不输出配置细节。JSON 请求限制默认 1 MiB，页文字另设更高但有限上限。

| 方法与路径（省略前缀） | 输入／返回 | 关键行为 |
| --- | --- | --- |
| GET /health/live、/health/ready | 最小状态 | ready 检查 DB/迁移/数据目录，不依赖云端可达 |
| POST /auth/login | password → cookie、CSRF token | Origin 校验、限速，不记录 body |
| GET /auth/session | admin、csrfToken | 供刷新页面恢复；Cache-Control: no-store |
| POST /auth/logout | 204 | 撤销会话、清 cookie |
| GET /settings/status | providersConfigured、limits、capabilities | 不返回密钥或完整配置；首版配置在服务端 |
| GET/POST /items | 查询／名称品牌型号配置 → item | 创建 201；服务端校验长度与空白 |
| GET/PATCH /items/{id} | item／变更 → item | PATCH If-Match；归档不物理删除被引用资产 |
| POST /items/{id}/assets | multipart file+purpose → asset | 流式上传；purpose 指定 document/photo/pageImage/pageText |
| GET/HEAD /assets/{id}/content | 字节 | 授权、ETag、Range；不暴露磁盘路径 |
| POST /items/{id}/documents | sourceAssetId、title、sourceUrl → document | 校验 PDF 类型、所属物品 |
| POST/PATCH /items/{id}/photos[/{photoId}] | assetId、view → photo | 修改 If-Match；同一快照每视图最多一张 |
| POST /documents/{id}/preparations | sourceSha256 → preparation | 相同原件和准备参数可返回现有未完成记录 |
| GET /preparations/{id} | 页状态、缺页、revision | 用于断线续传；不假定 IndexedDB 是事实来源 |
| PUT /preparations/{id}/pages/{pageNumber} | textAssetId、imageAssetId、viewport → page | 内容哈希相同幂等，不同内容覆盖需 If-Match；ready 禁止写 |
| POST /preparations/{id}/complete | pageCount → ready preparation | If-Match；事务校验全部连续页、归属、哈希；不收费 |
| POST /items/{id}/estimates | preparationId、photoIds、modelPreset → quote | 只计算计划不调用生成；返回配置价格与到期时间 |
| GET /items/{id}/estimates/{quoteId} | → quote | 读取既有报价（确认页回显用）；过期状态如实返回，不自动续期（T11 新增，2026-09-12 由协调者补记） |
| POST /items/{id}/estimates/{quoteId}/confirm | → 确认记录 | 记录"已获用户对发送内容与预算的确认"并写 audit_events；未确认的建单请求被拒（T11 新增，同上） |
| POST /items/{id}/jobs | quoteId、输入 IDs、limits → job | Idempotency-Key；quote 未过期且输入未变；冻结并预留，202 |
| GET /jobs[/{id}] | 列表／阶段、费用、错误、revision | 未知状态可读，前端轮询，不需要 WebSocket |
| POST /jobs/{id}/cancel | If-Match → job | 取消后续本地阶段；不声称已取消远端付费操作 |
| POST /jobs/{id}/retry | If-Match、stage、费用确认 → job | Idempotency-Key；仅可重試阶段；未知提交不得从此盲重试 |
| POST /jobs/{id}/reconcile | If-Match、action、remoteTaskId?、acknowledgeDuplicateRisk? | 处理 unknown，细则见下文；保留审计 |
| GET /items/{id}/drafts/{draftId} | 版本化知识、模型、复核状态 | 返回 ETag |
| PATCH /items/{id}/drafts/{draftId} | 受限字段变更 → draft | If-Match，校验 Part/Step/Evidence/Hotspot 引用；不能修改供应商事实快照 |
| POST /items/{id}/drafts/{draftId}/publish | If-Match → release | 幂等键；发布不变量全满足才 201，否则 422 明细 |
| GET /items/{id}/releases[/{releaseId}] | 版本列表／完整 manifest | 已发布内容不可变，model URL 指本地 asset |
| GET /releases/{releaseId}/export | 下载自包含包 | 原件／GLB／manifest／哈希；只导出有权资产，无密钥 |

`[/{...}]` 在表中表示两个明确路由，OpenAPI 中展开，不实际实现方括号 URL。归档通过 PATCH archived 字段；MVP 不暴露永久删除或任意磁盘浏览 API。备份恢复用 CLI，不开放高风险网页管理接口。

## 4. 输入快照与费用合同

生成前必须：名称／型号非空；PDF preparation ready；至少 front 加 left/back/right 之一；图片同物品、每视图唯一；Tripo 与 ManualAi 配置存在；有效价格快照；用户同意将选定资料发给相应供应商。

每份 quote 绑定输入哈希、模型参数、页数、最大 AI 输出 token、价格版本、预计与保守上界、expiresAt。期限默认 10 分钟。`POST jobs` 再验证引用和预算，不相信前端传入费用；允许上限必须覆盖服务器计算的计划上界，否则 422。自动降质量／换模型／增加处理阶段必须重新确认，不能藏在重试里。

计费区分 Tripo credits 与说明书 AI USD；同屏分别展示，不能相加成无单位数字。缺价格配置时不能宣称精确费用，应阻止正式生成并说明配置缺项。对服务商无法严格预估的图像 token 使用文档支持的保守上界；不能算出可靠上界的型号不进入首版支持清单。预算限制保证本应用不主动发起超过授权估算的请求，不冒充供应商账户级硬封顶。

幂等逻辑：调用者一次操作生成并复用 key → 在数据库中按 admin+method+route+key 唯一记录 → 相同 body_hash 返回原 job／release → 不同 body_hash 返回 409。重复点击、连接断开、服务器重启不能产生第二份本地生成单。生成和发布的幂等记录至少保留到相关业务记录删除，不用很短 TTL 造成迟到重放收费。

预留与创建 job 同事务；attempt 成功按实际结算，明确未计费失败释放，未知结果保留预留并等待对账。预估偏差显式显示；不能为消除 unknown 而将实际费用填 0。重生成总是新快照／新预算确认，原发布版不变。

## 5. 任务阶段与崩溃语义

阶段 DAG：

```text
freeze_inputs（入队事务）
  ├─ manual_extract_batches → manual_merge ──────────┐
  └─ tripo_upload → tripo_submit → tripo_poll         │
                   → model_download → model_validate ┤
                                                    ↓
                                              assemble_draft
```

PDF preparation 在这个 DAG 之前；浏览器热点校准／发布在这个 DAG 之后。`succeeded` 的含义是可复核草稿已产出，不是 `published`。

`manual_extract_batches` 是逻辑分支，实际展开为 `(stageKind=manual_extract, batchIndex=0..N-1)` 的持久执行单元。每批固定页集合、inputHash，分别保存结果资产与usage，不用内存for循环代替checkpoint；全部批次成功且页覆盖完整才解锁merge。租约、attempt和“同阶段一个未决提交”都针对具体stage_id，允许两个不同批次并发。

job／stage 状态枚举：`queued/running/waiting_provider/retry_wait/needs_input/submission_unknown/succeeded/failed/cancelled`。工作流 state.yaml 的 `pm_ready` 等不是这些运行时状态，不共用枚举。

| 事件 | 状态规则 |
| --- | --- |
| 领取阶段 | queued 或到期 retry_wait → running，原子取得新 leaseEpoch |
| 已提交远端 ID | running → waiting_provider；轮询由 nextRunAt 驱动 |
| 安全临时失败 | → retry_wait，记录原因和次数，超过上限 failed |
| 资料／schema／支持能力不足 | → needs_input，列出可行动缺项，不无休止重试 |
| 付费创建结果未知 | → submission_unknown，暂停该分支后续购买 |
| 阶段产物校验成功 | → succeeded；只解锁依赖全部完成的阶段 |
| 任一必需阶段 failed | 父 job failed，其他已完成成果保留，可针对失败分支重试 |
| 必需阶段含 unknown／needs_input | 父 job 展示对应状态与阻塞分支；独立已提交任务仍需对账 |
| 取消 | 未提交阶段 cancelled；已提交阶段停止新业务推进但保留查询／账务收尾，不保证供应商撤单 |
| 最后组装完成 | 父 job succeeded；draft.status = needs_review |

安全重试默认最多 5 次，退避 2/4/8/16/32 秒加 jitter，尊重 Retry-After 上限并记录。正常远端轮询默认 3 秒起、逐步到 15 秒，与“重试次数”不同；总等待默认 30 分钟后进入 needs_input，保留 task_id，用户恢复仅查询，不重新购买。只有可证明未被接受的错误才允许自动再提交；网络中断、含糊 5xx、超时均不能证明。

提交窗口必须按以下顺序测试：

1. 持久化 provider_attempt intent 和费用预留；同一阶段只允许一个未对账 attempt。
2. 在当前租约下标记 submitting，再发 HTTP POST。
3. 收到 ID 立即持久化 **事实观察**。即使租约刚过期，也允许将该 attempt 的空 remote_task_id 补成返回值；若已有不同 ID，记录冲突并停机告警，不能覆盖。
4. 业务状态推进另用当前 leaseEpoch 条件更新；过期 worker 不能解锁后续阶段。
5. Tripo 启动恢复：intent 尚未标记 submitting 可安全领取；submitting 但无 task ID 一律 unknown；有 task ID 继续查。这个异步查询规则不套到同步 Manual AI。

Manual AI 首版采用同步 Responses：每批完整响应收到后，先持久化响应结果资产，再在同一短事务保存usage、receipt和完成checkpoint。业务推进仍检查租约；过期worker可按attempt保存不可变receipt/result事实但不解锁merge。若已发请求却没有持久化完整响应，进入该批 submission_unknown；即使收到 response_id 也不假定可轮询／重取（可能未启用远端存储）。只对该批做有预算的人工授权重算，已完成批次不重跑。result已持久化而checkpoint未推进时，恢复程序校验已有结果并补推进，不重新付费。

`reconcile` 只允许管理员：`attachRemoteTask` 仅用于 Tripo，附加从供应商账户查到的 ID，并查询验证类型／账号可访问性，用户明确确认与该 attempt 对应；`recordNoTask` 要求填写核查证据；`authorizeReplacement` 要求再次预算确认并明确重复收费风险，创建新 attempt、保留旧 attempt 未决账务。同步 Manual AI 不提供 attachRemoteTask 恢复选项。不能伪造供应商“不存在”的证明。当前官方页面未提供可据以保证创建请求 exactly-once 的合同，因此应用不能自行宣称此保证。

本地 cancel 与远端状态分开，未决账务可比 UI 取消生命周期更长。应用退出时停止领取任务，等待短在途写入／保存 checkpoint，释放锁；下次启动恢复。用 hard kill 验证的不仅是正常 shutdown。

## 6. 外部 Provider 合同

### Tripo

Rust 接口建议分离 `upload_image / submit_multiview / get_task / download_model`；请求和响应 DTO 留在 providers/tripo，领域层只接收归一化结果。上传 multipart 字段 `file`，返回 token；生成 body 示例（token 为示意，不能直接执行）：

```json
{
  "inputs": [{ "front": "TOKEN_FRONT" }, { "back": "TOKEN_BACK" }],
  "model": "v3.1-20260211",
  "texture": true,
  "pbr": true,
  "texture_quality": "standard",
  "geometry_quality": "standard",
  "face_limit": 100000,
  "quad": false,
  "generate_parts": false
}
```

请求为 `POST /generation/multiview-to-model`，读取 `code/data.task_id`，非零 code 即业务错误，不能只判断 HTTP 200。GET `/tasks/{task_id}` 保存原始状态与归一化状态；success 必须有可下载模型，否则不组装成功。下载不能复用带 API Authorization 的 client 默认头。供应商信息核对入口：[上传](https://developers.tripo3d.ai/en/docs/files)、[多视图](https://developers.tripo3d.ai/en/docs/generation-multiview-to-model/standard)、[查询](https://developers.tripo3d.ai/en/docs/task-query)。

### Manual AI

`extract_batch(input, limits) -> StructuredBatchResult`：input 包含 itemIdentity、pages（1-based）、schemaVersion、promptVersion；结果包含 parts/steps/specs/evidence/uncertainties，不允许自由执行任何动作。`merge_batches` 首版可在本地做确定性合并，对同名不同事实保留冲突，不额外无限循环调用 AI。

OpenAI 参考请求由代码构造：`model` 来自配置，`input` 包含 input_text 和必要 input_image；`text.format.type=json_schema`、name=manual_extract_v1、strict=true、schema 使用全部 required 和 additionalProperties=false，可选值用 nullable。限制 max_output_tokens，按支持情况关闭远端响应存储；隐私选项不等于供应商完全零留存承诺。

服务器再次校验 JSON schema、字符串长度、总实体数、所有引用页集合和部件引用关系。refusal/incomplete/格式错不产生正式知识；保留短错误摘要及原始响应的受限诊断路径。不能把模型 confidence 当作已经验真的概率。依据：[Responses Structured Outputs](https://developers.openai.com/api/docs/guides/structured-outputs)。

Mock 与真实 Provider 共用领域接口但初始化显式互斥；生产启动禁止无配置时回退 mock。真实 HTTP 适配器必须先用本地 fixture 验证字节协议，不能只测一个永远返回成功的 fake trait。

## 7. 资产和发布不变量

输入限制默认：单图片 20 MiB、原 PDF 50 MiB／100 页、GLB 150 MiB、物品累计 500 MiB。请求解析前限制体积，读流时再次计数，检查文件 magic、图片尺寸／解码像素上限；Content-Type／后缀不可信。磁盘预留空间不足返回明确错误，不半提交资产。

GLB 检查：magic/version/声明长度／chunk 长度／JSON 结构／bufferView-accessor 范围／索引边界／有限坐标／非空几何／资源内嵌；默认三角面 ≤100000、贴图单边 ≤4096，禁止不支持的 required extension。超过预算进入 needs_input，保留原始模型与错误，不静默改坏模型。CPU 结构校验不代表 GPU 可以成功绘制，浏览器需二次验证。

Range 合同：完整 GET 200；合法单区间 206 并正确 Content-Range/Length；不可满足 416；多区间首版可忽略并返回完整 200，不能返回错误拼接。HEAD 无 body；If-None-Match 可 304，If-Range 不匹配返回完整 200。PDF 原件仍可逐页读取，不使用 SPA fallback 响应。

发布事务必须验证：所有必需知识已确认或有明确人工修订记录；引用页存在；选中模型 validated 且浏览器加载复核通过；每个要发布的交互部件至少一个 confirmed 热点且 hash 匹配；步骤引用全部存在；没有 stale/candidate 热点冒充 confirmed。无法绑定的知识可在 PM 允许的“仅文本条目”模式保留并明显标识，不能为了发布自动隐藏必需内容。

导出 manifest 包含 schemaVersion、item、release、知识、相对资产清单、sha256 和来源；不包含绝对路径、密钥、会话、临时云端 URL。首版导出／恢复是数据便携和灾备，不承诺导出包能直接双击运行网站。将来导入时必须另设 zip-slip／解压炸弹验收，当前不顺手开放 ZIP 导入接口。
