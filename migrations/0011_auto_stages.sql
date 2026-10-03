-- no-transaction
-- 0011_auto_stages —— `job_stages.stage_kind` 增加 `tripo_segment` 和 `auto_bind`（ADR-045）。
--
-- 为什么：生成完成后自动拆分模型为可交互分件（Tripo mesh/segment），再自动绑定热点与
-- 推导动作/姿势，用户无需手动操作。新增两个流水线阶段：`tripo_segment` 依赖
-- `assemble_draft`，`auto_bind` 依赖 `tripo_segment`。
--
-- 重建表步骤与 0007/0009 相同（SQLite 不能原地修改 CHECK 约束）。
-- 必须包含 0005_job_execution 追加的 poll_count / last_error / needs_input_json 列。
-- 迁移只追加，不修改历史文件（ADR-009）。

PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE job_stages_new (
    id              TEXT PRIMARY KEY,
    job_id          TEXT NOT NULL REFERENCES jobs (id) ON DELETE RESTRICT,
    stage_kind      TEXT NOT NULL CHECK (stage_kind IN (
        'freeze_inputs', 'manual_extract', 'manual_merge', 'tripo_upload',
        'tripo_submit', 'tripo_poll', 'model_download', 'model_validate',
        'assemble_draft', 'tripo_segment', 'auto_bind'
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
    poll_count      INTEGER NOT NULL DEFAULT 0 CHECK (poll_count >= 0),
    last_error      TEXT,
    needs_input_json TEXT CHECK (needs_input_json IS NULL OR json_valid(needs_input_json)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (job_id, stage_kind, batch_index),
    CHECK (stage_kind = 'manual_extract' OR batch_index = 0)
);

INSERT INTO job_stages_new (id, job_id, stage_kind, batch_index, page_set, input_hash,
    result_asset_id, usage_json, status, lease_owner, lease_epoch, lease_until,
    next_run_at, attempt_count, poll_count, last_error, needs_input_json, created_at, updated_at)
SELECT id, job_id, stage_kind, batch_index, page_set, input_hash,
    result_asset_id, usage_json, status, lease_owner, lease_epoch, lease_until,
    next_run_at, attempt_count, poll_count, last_error, needs_input_json, created_at, updated_at
FROM job_stages;

DROP TABLE job_stages;

ALTER TABLE job_stages_new RENAME TO job_stages;

CREATE INDEX job_stages_job ON job_stages (job_id);
CREATE INDEX job_stages_due ON job_stages (status, next_run_at);

COMMIT;

PRAGMA foreign_keys = ON;
