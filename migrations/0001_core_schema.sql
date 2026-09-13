-- 0001_core_schema —— contracts.md §2「数据对象与数据库约束」的核心表。
--
-- 约定（本文件是持久 schema 的机器来源，ADR-009；语义解释在 llmdoc/contracts.md §2）：
--   * 实体 ID：TEXT，服务器生成 UUIDv7 字符串（contracts.md §1）；
--   * 时间：INTEGER，Unix epoch 毫秒（UTC）。API 层转换为 RFC3339（contracts.md §1）；
--     统一整数避免文本时间格式漂移破坏租约/过期比较（见 decisions.md ADR-012）；
--   * 枚举：TEXT + CHECK；SQL 值用 snake_case，JSON 线上值用 camelCase（contracts.md §1）；
--   * JSON 文本列用 json_valid() 兜底（bundled SQLite 内置 JSON 函数）；
--   * 金额：INTEGER 最小单位（Tripo creditMinor=1/100 credit、USD usdMicros=1/1e6），
--     禁止浮点（contracts.md §1）；
--   * 本迁移只建表/键/约束/索引；触发器（不可变快照、远端 ID 单调、未对账 attempt 唯一）
--     在 0002_invariants.sql。迁移只追加，不修改历史文件（ADR-009）。

-- ---------------------------------------------------------------------------
-- 认证（表结构按合同建齐；登录/会话逻辑属 T04）
-- ---------------------------------------------------------------------------

CREATE TABLE admins (
    id            TEXT PRIMARY KEY,
    -- 只存 Argon2 哈希；无默认值、无默认密码（contracts.md §2）。
    password_hash TEXT NOT NULL CHECK (length(password_hash) > 0),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
    id                 TEXT PRIMARY KEY,
    admin_id           TEXT NOT NULL REFERENCES admins (id) ON DELETE CASCADE,
    -- 会话明文只返回 cookie，不落库/日志：这里只存哈希（contracts.md §2）。
    session_token_hash TEXT NOT NULL UNIQUE CHECK (length(session_token_hash) > 0),
    csrf_hash          TEXT NOT NULL CHECK (length(csrf_hash) > 0),
    created_at         INTEGER NOT NULL,
    expires_at         INTEGER NOT NULL,
    revoked_at         INTEGER,
    CHECK (expires_at > created_at)
);

CREATE INDEX sessions_admin ON sessions (admin_id);
CREATE INDEX sessions_expires ON sessions (expires_at);

-- ---------------------------------------------------------------------------
-- 物品与原始资料
-- ---------------------------------------------------------------------------

CREATE TABLE items (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL CHECK (length(trim(name)) > 0),
    brand       TEXT,
    model       TEXT NOT NULL CHECK (length(trim(model)) > 0),
    variant     TEXT,
    revision    INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    archived_at INTEGER,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
    -- 同品牌型号不强制唯一：允许“同型号不同配置”并存（contracts.md §2、REQ-010）。
);

CREATE INDEX items_created ON items (created_at DESC);

-- 内容寻址的二进制存储：sha256 唯一内容存储（一个内容一行 blob）。
-- 文件字节在 `<data-dir>/blobs/<sha256 前两位>/<sha256>`（architecture.md §6）。
CREATE TABLE blobs (
    sha256        TEXT PRIMARY KEY
        CHECK (length(sha256) = 64 AND sha256 GLOB '[0-9a-f]*' AND NOT sha256 GLOB '*[^0-9a-f]*'),
    size          INTEGER NOT NULL CHECK (size >= 0),
    mime          TEXT NOT NULL,
    storage_state TEXT NOT NULL DEFAULT 'stored'
        CHECK (storage_state IN ('stored', 'quarantined', 'missing')),
    created_at    INTEGER NOT NULL
);

-- 资产引用 blob（去重）并归属到物品；所有访问先验证 asset 归属（contracts.md §2）。
-- purpose 取值来自路由合同（contracts.md §3）加模型资产（model_revisions.asset_id）；
-- SQL 值统一 snake_case，线上 JSON 由 Rust 枚举序列化为 camelCase（contracts.md §1，
-- 即 API 里的 pageImage/pageText 对应库里的 page_image/page_text）。
CREATE TABLE assets (
    id            TEXT PRIMARY KEY,
    blob_id       TEXT NOT NULL REFERENCES blobs (sha256) ON DELETE RESTRICT,
    item_id       TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    purpose       TEXT NOT NULL
        CHECK (purpose IN ('document', 'photo', 'page_image', 'page_text', 'model')),
    -- 原文件名只作元数据，不参与路径拼接（contracts.md §2、REQ-011）。
    original_name TEXT,
    created_at    INTEGER NOT NULL
);

CREATE INDEX assets_item ON assets (item_id, purpose);
CREATE INDEX assets_blob ON assets (blob_id);

CREATE TABLE documents (
    id              TEXT PRIMARY KEY,
    item_id         TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    source_asset_id TEXT NOT NULL REFERENCES assets (id) ON DELETE RESTRICT,
    source_sha256   TEXT NOT NULL REFERENCES blobs (sha256) ON DELETE RESTRICT,
    title           TEXT NOT NULL,
    -- source_url 只作出处记录，服务端不据此发起抓取（contracts.md §2、REQ-012）。
    source_url      TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX documents_item ON documents (item_id);

CREATE TABLE preparations (
    id            TEXT PRIMARY KEY,
    document_id   TEXT NOT NULL REFERENCES documents (id) ON DELETE RESTRICT,
    source_sha256 TEXT NOT NULL REFERENCES blobs (sha256) ON DELETE RESTRICT,
    state         TEXT NOT NULL DEFAULT 'preparing'
        CHECK (state IN ('preparing', 'ready')),
    page_count    INTEGER CHECK (page_count IS NULL OR page_count >= 0),
    revision      INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    -- ready 必须声明页数；“ready 后不可修改”由仓储层在事务内保证（T09）。
    CHECK (state <> 'ready' OR page_count IS NOT NULL)
);

CREATE INDEX preparations_document ON preparations (document_id, state);

-- 页号唯一且完成时连续 1..N（contracts.md §2）：唯一性由复合主键落到 SQL。
CREATE TABLE pages (
    preparation_id TEXT NOT NULL REFERENCES preparations (id) ON DELETE RESTRICT,
    page_number    INTEGER NOT NULL CHECK (page_number >= 1),
    text_asset_id  TEXT REFERENCES assets (id) ON DELETE RESTRICT,
    image_asset_id TEXT REFERENCES assets (id) ON DELETE RESTRICT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (preparation_id, page_number)
);

CREATE TABLE photos (
    id         TEXT PRIMARY KEY,
    item_id    TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    asset_id   TEXT NOT NULL REFERENCES assets (id) ON DELETE RESTRICT,
    -- 视图方向以物品自身为参照（PRD A-04）；detail 不进入 Tripo 多视图请求。
    view       TEXT NOT NULL CHECK (view IN ('front', 'left', 'back', 'right', 'detail')),
    revision   INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX photos_item ON photos (item_id, view);

-- ---------------------------------------------------------------------------
-- 生成输入快照、任务与费用
-- ---------------------------------------------------------------------------

-- 输入不可变（contracts.md §2/§4）：冻结后由 0002 的触发器拒绝 UPDATE。
CREATE TABLE generation_snapshots (
    id              TEXT PRIMARY KEY,
    item_id         TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    item_revision   INTEGER NOT NULL CHECK (item_revision >= 1),
    preparation_id  TEXT NOT NULL REFERENCES preparations (id) ON DELETE RESTRICT,
    photo_ids       TEXT NOT NULL CHECK (json_valid(photo_ids)),
    photo_hashes    TEXT NOT NULL CHECK (json_valid(photo_hashes)),
    provider_config TEXT NOT NULL CHECK (json_valid(provider_config)),
    prompt_version  TEXT NOT NULL,
    price_version   TEXT NOT NULL,
    budgets         TEXT NOT NULL CHECK (json_valid(budgets)),
    created_at      INTEGER NOT NULL
    -- 不保存 API key：provider_config 只允许非密钥配置（contracts.md §2/REQ-022）。
);

CREATE INDEX generation_snapshots_item ON generation_snapshots (item_id, created_at DESC);

-- 任务状态枚举见 contracts.md §5；SQL 值 snake_case。
CREATE TABLE jobs (
    id          TEXT PRIMARY KEY,
    item_id     TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    snapshot_id TEXT NOT NULL REFERENCES generation_snapshots (id) ON DELETE RESTRICT,
    status      TEXT NOT NULL DEFAULT 'queued' CHECK (status IN (
        'queued', 'running', 'waiting_provider', 'retry_wait', 'needs_input',
        'submission_unknown', 'succeeded', 'failed', 'cancelled'
    )),
    revision    INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX jobs_item ON jobs (item_id, created_at DESC);
CREATE INDEX jobs_status ON jobs (status, updated_at);

-- 阶段是持久执行单元：每 job+stage_kind+batch_index 唯一；
-- 非批处理阶段 batch_index=0；状态推进条件含当前 lease_epoch（contracts.md §2/§5）。
CREATE TABLE job_stages (
    id              TEXT PRIMARY KEY,
    job_id          TEXT NOT NULL REFERENCES jobs (id) ON DELETE RESTRICT,
    stage_kind      TEXT NOT NULL CHECK (stage_kind IN (
        'freeze_inputs', 'manual_extract', 'manual_merge', 'tripo_upload',
        'tripo_submit', 'tripo_poll', 'model_download', 'model_validate',
        'assemble_draft'
    )),
    batch_index     INTEGER NOT NULL DEFAULT 0 CHECK (batch_index >= 0),
    page_set        TEXT CHECK (page_set IS NULL OR json_valid(page_set)),
    input_hash      TEXT NOT NULL,
    result_asset_id TEXT REFERENCES assets (id) ON DELETE RESTRICT,
    usage_json      TEXT CHECK (usage_json IS NULL OR json_valid(usage_json)),
    status          TEXT NOT NULL DEFAULT 'queued' CHECK (status IN (
        'queued', 'running', 'waiting_provider', 'retry_wait', 'needs_input',
        'submission_unknown', 'succeeded', 'failed', 'cancelled'
    )),
    lease_owner     TEXT,
    lease_epoch     INTEGER NOT NULL DEFAULT 0 CHECK (lease_epoch >= 0),
    lease_until     INTEGER,
    next_run_at     INTEGER,
    attempt_count   INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (job_id, stage_kind, batch_index),
    CHECK (stage_kind = 'manual_extract' OR batch_index = 0)
);

CREATE INDEX job_stages_job ON job_stages (job_id);
CREATE INDEX job_stages_due ON job_stages (status, next_run_at);

-- 付费提交的 attempt：先存 intent 再请求；远端 ID 只允许 null→值或同值
-- （overwrite 由 0002 触发器拒绝）；同一阶段只允许一个未对账 attempt
-- （0002 的部分唯一索引 provider_attempts_unresolved_stage，§5）。
CREATE TABLE provider_attempts (
    id             TEXT PRIMARY KEY,
    job_id         TEXT NOT NULL REFERENCES jobs (id) ON DELETE RESTRICT,
    stage_id       TEXT NOT NULL REFERENCES job_stages (id) ON DELETE RESTRICT,
    request_hash   TEXT NOT NULL,
    submit_state   TEXT NOT NULL CHECK (submit_state IN (
        'intent', 'submitting', 'accepted', 'unknown', 'failed'
    )),
    remote_task_id TEXT,
    -- 同步 Manual AI 的 response_id 不等于可轮询任务（contracts.md §2）。
    response_id    TEXT,
    started_at     INTEGER NOT NULL,
    last_error     TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

CREATE INDEX provider_attempts_job ON provider_attempts (job_id);
CREATE INDEX provider_attempts_stage ON provider_attempts (stage_id);

-- 幂等记录：范围内唯一；同 key 不同 body 由服务层返回 409（contracts.md §4）。
CREATE TABLE idempotency_records (
    id              TEXT PRIMARY KEY,
    admin_id        TEXT NOT NULL REFERENCES admins (id) ON DELETE RESTRICT,
    method          TEXT NOT NULL,
    route           TEXT NOT NULL,
    "key"           TEXT NOT NULL,
    body_hash       TEXT NOT NULL,
    resource_id     TEXT,
    response_status INTEGER,
    created_at      INTEGER NOT NULL,
    UNIQUE (admin_id, method, route, "key")
);

CREATE INDEX idempotency_records_resource ON idempotency_records (resource_id);

-- 费用账本：预留/结算/释放在事务内且幂等；unknown 保留预留且 actual 保持 NULL
-- （不得填 0，contracts.md §4/REQ-023）。
CREATE TABLE cost_ledger (
    id            TEXT PRIMARY KEY,
    snapshot_id   TEXT NOT NULL REFERENCES generation_snapshots (id) ON DELETE RESTRICT,
    attempt_id    TEXT REFERENCES provider_attempts (id) ON DELETE RESTRICT,
    provider      TEXT NOT NULL CHECK (provider IN ('tripo', 'manual_ai')),
    currency      TEXT NOT NULL CHECK (currency IN ('credit_minor', 'usd_micros')),
    reserved      INTEGER NOT NULL CHECK (reserved >= 0),
    actual        INTEGER CHECK (actual IS NULL OR actual >= 0),
    state         TEXT NOT NULL CHECK (state IN ('reserved', 'settled', 'released', 'unknown')),
    price_version TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    CHECK (state <> 'settled' OR actual IS NOT NULL),
    CHECK (state <> 'unknown' OR actual IS NULL)
);

CREATE INDEX cost_ledger_snapshot ON cost_ledger (snapshot_id, provider, state);
CREATE INDEX cost_ledger_attempt ON cost_ledger (attempt_id);

-- ---------------------------------------------------------------------------
-- 模型版本、草稿与发布
-- ---------------------------------------------------------------------------

-- 模型字节不可变；校验通过（validated）才可进入阅读器（contracts.md §2、§7）。
CREATE TABLE model_revisions (
    id                  TEXT PRIMARY KEY,
    item_id             TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    asset_id            TEXT NOT NULL REFERENCES assets (id) ON DELETE RESTRICT,
    sha256              TEXT NOT NULL
        CHECK (length(sha256) = 64 AND sha256 GLOB '[0-9a-f]*' AND NOT sha256 GLOB '*[^0-9a-f]*'),
    provider_attempt_id TEXT REFERENCES provider_attempts (id) ON DELETE RESTRICT,
    bounds              TEXT CHECK (bounds IS NULL OR json_valid(bounds)),
    validation_state    TEXT NOT NULL DEFAULT 'pending'
        CHECK (validation_state IN ('pending', 'validated', 'rejected')),
    created_at          INTEGER NOT NULL
);

CREATE INDEX model_revisions_item ON model_revisions (item_id, created_at DESC);

-- 每个快照至多一份草稿：assemble_draft 幂等、重启不重复创建（contracts.md §5、REQ-030）。
-- 部件/步骤/热点作为版本化 JSON 聚合存储（contracts.md §2），聚合更新用 revision CAS。
CREATE TABLE manual_drafts (
    id                TEXT PRIMARY KEY,
    item_id           TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    snapshot_id       TEXT NOT NULL UNIQUE REFERENCES generation_snapshots (id) ON DELETE RESTRICT,
    model_revision_id TEXT REFERENCES model_revisions (id) ON DELETE RESTRICT,
    revision          INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    status            TEXT NOT NULL DEFAULT 'needs_review'
        CHECK (status IN ('needs_review', 'ready')),
    knowledge_json    TEXT NOT NULL CHECK (json_valid(knowledge_json)),
    review_json       TEXT CHECK (review_json IS NULL OR json_valid(review_json)),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

CREATE INDEX manual_drafts_item ON manual_drafts (item_id, created_at DESC);

-- 不可变完整快照（contracts.md §2/§7）：UPDATE 由 0002 触发器拒绝；
-- 发布后不引用会变的 draft 内容（只保存 draft_revision 与 manifest 资产引用）。
CREATE TABLE manual_releases (
    id                TEXT PRIMARY KEY,
    item_id           TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    draft_id          TEXT NOT NULL REFERENCES manual_drafts (id) ON DELETE RESTRICT,
    draft_revision    INTEGER NOT NULL CHECK (draft_revision >= 1),
    model_revision_id TEXT NOT NULL REFERENCES model_revisions (id) ON DELETE RESTRICT,
    manifest_asset_id TEXT NOT NULL REFERENCES assets (id) ON DELETE RESTRICT,
    created_at        INTEGER NOT NULL
);

CREATE INDEX manual_releases_item ON manual_releases (item_id, created_at DESC);
CREATE INDEX manual_releases_draft ON manual_releases (draft_id);

-- ---------------------------------------------------------------------------
-- 审计
-- ---------------------------------------------------------------------------

-- 只存必要摘要（contracts.md §2）：费用确认、人工事实修改、发布与重试等。
CREATE TABLE audit_events (
    id            TEXT PRIMARY KEY,
    entity_type   TEXT NOT NULL,
    entity_id     TEXT NOT NULL,
    -- actor 为管理员 id，或 'system'（服务器动作）。
    actor         TEXT,
    action        TEXT NOT NULL,
    result        TEXT NOT NULL,
    metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json)),
    created_at    INTEGER NOT NULL
);

CREATE INDEX audit_events_entity ON audit_events (entity_type, entity_id, created_at);
CREATE INDEX audit_events_created ON audit_events (created_at);
