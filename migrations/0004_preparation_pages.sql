-- 0004_preparation_pages —— 浏览器 PDF 准备（T09 / REQ-014、REQ-015）所需的两个附加列。
--
-- 为什么需要迁移而不是改 0001：迁移只追加（ADR-009）；0001 的 preparations/pages
-- 已含合同核心列（document_id、source_sha256、state、page_count、page_number、
-- text_asset_id、image_asset_id），本迁移只补"页图坐标参照"与"客户端派生"标记。
--
-- 1) preparations.client_derived：REQ-015 要求封存后标记页资产由浏览器（PDF.js）
--    派生上传（contracts.md §2）。默认 0（历史/未封存记录不冒充已派生），
--    complete 成功时在同一事务写入 1。
--
-- 2) pages.viewport_json：contracts.md §3 的 PUT 页请求包含 `viewport`，
--    架构 §5.1 规定"旋转后 viewport 左上角"为页图坐标原点。保存渲染时实际使用的
--    `{width, height, rotation}` 让后续知识 bbox 归一化有确定参照（contracts.md §2
--    「bbox 为旋转后的页图上 [x,y,w,h]，并保存页图尺寸／旋转」）。
--    允许 NULL：未上传/未渲染的页没有 viewport；不写 [0,0] 之类占位。
--
-- 迁移只追加，不修改历史文件（ADR-009）。

ALTER TABLE preparations
    ADD COLUMN client_derived INTEGER NOT NULL DEFAULT 0
        CHECK (client_derived IN (0, 1));

ALTER TABLE pages
    ADD COLUMN viewport_json TEXT
        CHECK (viewport_json IS NULL OR json_valid(viewport_json));

-- 注：页号上界（≤100 页，PRD §5.3）不能由 SQLite 的 ALTER TABLE ADD COLUMN 新增的
-- 列级 CHECK 表达（无法引用 page_number 之外的既有列），由服务端在 PUT 与 complete
-- 的事务内显式校验（422 + details.reason=pageLimitExceeded），并有 Rust 集成测试守住。
-- 索引不需要新增：pages 的复合主键 (preparation_id, page_number) 已覆盖页集合查询。
