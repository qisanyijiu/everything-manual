-- 0010_view_candidates —— 说明书 PDF 中解析出的「视图候选图」（ADR-044）。
--
-- 为什么：多视图生成需要 front/left/back/right（+ detail）照片；很多说明书本身就带有产品
-- 多角度图。准备阶段在浏览器内用 pdf.js 拆出候选图（内嵌位图 + 线稿区域裁剪），由视觉模型
-- 给出建议视图与置信度；用户在视图排列页拖拽确定最终排列。候选只是"待选"：不参与生成快照，
-- 选中后才复制为 photos 行（同一资产），删除候选不影响已排列的照片。
--
-- 约束：
--   * asset 必须属于同一物品且 purpose=photo（由 API 校验）；
--   * suggested_view 为空表示"未判断/不像产品视图"；dismissed_at 非空 = 用户已删除（软删除，
--     便于撤销与审计；列表默认不返回）。
-- 迁移只追加，不修改历史文件（ADR-009）。

CREATE TABLE view_candidates (
    id              TEXT PRIMARY KEY,
    item_id         TEXT NOT NULL REFERENCES items (id) ON DELETE RESTRICT,
    asset_id        TEXT NOT NULL REFERENCES assets (id) ON DELETE RESTRICT,
    document_id     TEXT REFERENCES documents (id) ON DELETE RESTRICT,
    page_number     INTEGER CHECK (page_number IS NULL OR page_number >= 1),
    source          TEXT NOT NULL CHECK (source IN ('embedded', 'region', 'upload')),
    suggested_view  TEXT CHECK (suggested_view IS NULL OR suggested_view IN ('front', 'left', 'back', 'right', 'detail')),
    confidence      REAL CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
    note            TEXT CHECK (note IS NULL OR length(note) <= 200),
    dismissed_at    INTEGER,
    created_at      INTEGER NOT NULL
);

CREATE INDEX view_candidates_item ON view_candidates (item_id, dismissed_at, created_at);
CREATE UNIQUE INDEX view_candidates_item_asset ON view_candidates (item_id, asset_id);
