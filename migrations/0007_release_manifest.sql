-- no-transaction
-- 0007_release_manifest —— `assets.purpose` 增加 `release_manifest`
-- （T19 / REQ-035；contracts.md §2 `manual_releases.manifest_asset_id`、§3 publish）。
--
-- 为什么：发布事务把不可变 manifest 作为**内容寻址资产**落盘（与上传资产同一套
-- sha256 去重、Range/ETag 服务与归属校验），`manual_releases.manifest_asset_id`
-- NOT NULL REFERENCES assets(id) 要求 assets 行存在。manifest 既不是 document
-- 也不是 model，必须有自己的用途值，否则 T20 导出/扫描按 purpose 分流时会把它
-- 误当说明书原件。
--
-- 为什么需要重建表：SQLite 不能在原地修改 CHECK 约束（0001 的 purpose 枚举）。
-- 重建流程按 SQLite 官方推荐步骤（安全检查清单）：
--   PRAGMA foreign_keys=OFF（事务外）→ 建新表 → 拷数据 → 删旧表 → 重命名 →
--   重建索引 → COMMIT → PRAGMA foreign_keys=ON。
-- 因此本迁移文件**刻意使用 `-- no-transaction`**（sqlx 0.9 支持：文件首行不含
-- 该标记时每条迁移会被包进事务，而 PRAGMA foreign_keys 在事务内是 no-op）。
-- 与 `storage/migrations.rs` 的说明一致：迁移原子性单位是单条迁移；本迁移只做
-- "重建一张表 + 拷行"，失败重跑前可用 `PRAGMA foreign_key_check` 核对引用。
--
-- 迁移只追加，不修改历史文件（ADR-009）。

PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE assets_new (
    id            TEXT PRIMARY KEY,
    blob_id       TEXT NOT NULL REFERENCES blobs (sha256) ON DELETE RESTRICT,
    item_id       TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    purpose       TEXT NOT NULL
        CHECK (purpose IN ('document', 'photo', 'page_image', 'page_text', 'model', 'release_manifest')),
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
