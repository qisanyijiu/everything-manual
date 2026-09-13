-- 0005_job_execution —— T10 持久任务执行器的追加 schema（迁移只追加，ADR-009）。
--
-- 为什么需要这些列/表（取舍见 decisions.md ADR-020）：
--   1) `job_stage_deps`：阶段 DAG 的**依赖边**。合同的解锁条件是"依赖阶段全部
--      succeeded"，执行器领取时必须用 SQL 判定（两个 worker 竞争同一行也只能有一个
--      成功领取）；把边落库而不是在内存里跑 for 循环，崩溃重启后判定依旧成立。
--   2) `job_stages.poll_count`：远端轮询节奏 3 秒起逐步到 15 秒（contracts.md §5），
--      需要持久计数才能在重启后继续同一条节奏，而不是每次重启又从 3 秒开始。
--   3) `job_stages.last_error` / `needs_input_json`：临时失败原因与"可行动缺项"
--      必须能展示给用户（REQ-024：needs_input 列出可行动缺项），不能只留在日志里。
--
-- 不修改 0001–0004（历史迁移不可改，ADR-009/ADR-012 注 10）。

CREATE TABLE job_stage_deps (
    stage_id            TEXT NOT NULL REFERENCES job_stages (id) ON DELETE RESTRICT,
    depends_on_stage_id TEXT NOT NULL REFERENCES job_stages (id) ON DELETE RESTRICT,
    created_at          INTEGER NOT NULL,
    PRIMARY KEY (stage_id, depends_on_stage_id),
    -- 自环不是合法依赖（DAG 无环；跨阶段成环由建单方保证，见 T11/T15 交接）。
    CHECK (stage_id <> depends_on_stage_id)
);

CREATE INDEX job_stage_deps_depends_on ON job_stage_deps (depends_on_stage_id);

ALTER TABLE job_stages ADD COLUMN poll_count INTEGER NOT NULL DEFAULT 0
    CHECK (poll_count >= 0);
ALTER TABLE job_stages ADD COLUMN last_error TEXT;
ALTER TABLE job_stages ADD COLUMN needs_input_json TEXT
    CHECK (needs_input_json IS NULL OR json_valid(needs_input_json));
