-- no-transaction
-- 0008_model_parts —— `assets.purpose` 增加 `model_parts`（分件模型，ADR-042）。
--
-- 为什么：交互式说明书需要"可单独高亮/移动的部件"。Tripo `mesh/segment` 产出与草稿
-- 模型**同一坐标系**的多节点 GLB；它不是新的模型版本（热点锚点仍属于草稿模型
-- revision），而是该版本的可选附件，因此需要独立用途值，避免被 T20 导出/启动扫描
-- 误当作模型版本或说明书原件。
--
-- 重建表步骤与 0007 相同（SQLite 不能原地修改 CHECK；`-- no-transaction` 让
-- PRAGMA foreign_keys 生效）。迁移只追加，不修改历史文件（ADR-009）。

PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE assets_new (
    id            TEXT PRIMARY KEY,
    blob_id       TEXT NOT NULL REFERENCES blobs (sha256) ON DELETE RESTRICT,
    item_id       TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    purpose       TEXT NOT NULL
        CHECK (purpose IN ('document', 'photo', 'page_image', 'page_text', 'model', 'release_manifest', 'model_parts')),
    original_name TEXT,
    created_at    INTEGER NOT NULL
);

INSERT INTO assets_new (id, blob_id, item_id, purpose, original_name, created_at)
    SELECT id, blob_id, item_id, purpose, original_name, created_at FROM assets;

DROP TABLE assets;

ALTER TABLE assets_new RENAME TO assets;

CREATE INDEX assets_item ON assets (item_id, purpose);
CREATE INDEX assets_blob ON assets (blob_id);

COMMIT;

PRAGMA foreign_keys = ON;
