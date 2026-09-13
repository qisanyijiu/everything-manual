-- 0006_generation_requests —— 报价快照（T11：REQ-020/021/022）。
--
-- 为什么需要 quotes 表（RD 取舍，见 implementation.md §T11 与 ADR-021）：
--   * `POST /items/{id}/jobs` 的输入是 `quoteId`，服务端必须能在**不接受前端费用数值**
--     的前提下重新校验"引用、报价未过期、输入未变、预算足够"；把报价落库是唯一
--     可回读、可审计的做法（contracts.md §4「每份 quote 绑定……」）；
--   * 快照列（photo_ids/photo_hashes/input_hash/provider_config/price_version/预算）
--     与 `generation_snapshots` 同源，冻结后不可修改（触发器）。
--
-- 确认与消费是**一次性**事实：
--   * `confirmed_at`/`confirmation_json`：REQ-021 的"云端发送前告知与同意"，
--     只允许 null → 值（重复确认由服务端按幂等返回原记录）；确认动作另写 audit_events；
--   * `consumed_at`/`consumed_job_id`：一份报价只能创建一份任务（"重生成总是新快照 +
--     新预算确认"），第二个 job 必须重新报价；消费标记同样只允许 null → 值。
--
-- 迁移只追加，不修改 0001–0005（ADR-009）。

CREATE TABLE quotes (
    id                  TEXT PRIMARY KEY,
    item_id             TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    preparation_id      TEXT NOT NULL REFERENCES preparations (id) ON DELETE RESTRICT,
    -- 多视图照片的冻结引用（photoId 与内容 sha256 一一对应；T07 QA 前置约束：
    -- photos 行可变，只存 id 会漏掉"同 id 换资产"）。
    photo_ids           TEXT NOT NULL CHECK (json_valid(photo_ids)),
    photo_hashes        TEXT NOT NULL CHECK (json_valid(photo_hashes)),
    input_hash          TEXT NOT NULL CHECK (length(input_hash) = 64),
    model_preset        TEXT NOT NULL,
    -- 非密钥的供应商配置快照（模型名/质量参数/prompt 版本）；不含 API key。
    provider_config     TEXT NOT NULL CHECK (json_valid(provider_config)),
    price_version       TEXT NOT NULL,
    price_snapshot_date TEXT NOT NULL,
    page_count          INTEGER NOT NULL CHECK (page_count >= 1),
    -- 最大输出 token（说明书 AI 的预算口径之一；批次数 × 每批上限）。
    max_output_tokens   INTEGER NOT NULL CHECK (max_output_tokens >= 0),
    -- 完整报价载荷（分列金额、保守上界、发送范围）；服务端回读时不依赖前端。
    quote_json          TEXT NOT NULL CHECK (json_valid(quote_json)),
    expires_at          INTEGER NOT NULL,
    confirmed_at        INTEGER,
    confirmation_json   TEXT CHECK (confirmation_json IS NULL OR json_valid(confirmation_json)),
    consumed_at         INTEGER,
    consumed_job_id     TEXT REFERENCES jobs (id) ON DELETE RESTRICT,
    created_at          INTEGER NOT NULL,
    CHECK (consumed_at IS NULL OR consumed_job_id IS NOT NULL),
    CHECK (confirmed_at IS NULL OR confirmation_json IS NOT NULL)
);

CREATE INDEX quotes_item ON quotes (item_id, created_at DESC);
CREATE INDEX quotes_expires ON quotes (expires_at);

-- 1) 报价内容（含金额与到期时间）是冻结快照：不允许任何 UPDATE 改写。
CREATE TRIGGER quotes_snapshot_immutable
BEFORE UPDATE OF item_id, preparation_id, photo_ids, photo_hashes, input_hash, model_preset,
                 provider_config, price_version, price_snapshot_date, page_count,
                 max_output_tokens, quote_json, expires_at, created_at ON quotes
BEGIN
    SELECT RAISE(ABORT, 'quotes 是报价快照，不允许修改');
END;

-- 2) 确认只允许 null → 值（重复确认走幂等读回，不覆盖首次确认时间与范围）。
CREATE TRIGGER quotes_confirmation_frozen
BEFORE UPDATE OF confirmed_at ON quotes
WHEN OLD.confirmed_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'quotes 的确认已记录：不允许再次修改或清除');
END;

-- 3) 消费标记只允许 null → 值（一份报价只能创建一份任务）。
CREATE TRIGGER quotes_consumption_frozen
BEFORE UPDATE OF consumed_at ON quotes
WHEN OLD.consumed_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'quotes 已被任务消费：不允许改写');
END;

-- 4) 同一快照 + 供应商至多一笔**进行中**预留（reserved）：并发建单/重试不允许重复占用
--    同一份预算。已结算/已释放/未决（unknown）的记录不在该部分索引内——重试可以在
--    旧记录定性后重新预留，对账决定（T15）不受影响。
CREATE UNIQUE INDEX cost_ledger_active_reservation
    ON cost_ledger (snapshot_id, provider) WHERE state = 'reserved';
