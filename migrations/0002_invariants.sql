-- 0002_invariants —— contracts.md §2/§4/§5 中必须以 SQL 层强制、无法由普通
-- 唯一键/外键表达的不变量。
--
-- 为什么用触发器（RD 取舍，见 decisions.md ADR-012）：
--   * 「输入不可变」「发布不可变」「远端 ID 只允许 null→值或同值」是合同里的
--     "必须保证"，应用层校验会被后来的代码路径绕过；触发器把不变量钉在 schema 上，
--     且可以像唯一键一样被测试直接验证。
--   * 触发器只禁止明确的错误写入，不替应用做业务规则（业务校验仍属各服务层）。
-- 迁移只追加，不修改 0001（ADR-009）。

-- 1) generation_snapshots 输入不可变：编辑原物品/资料不改变已开始任务
--    （contracts.md §2「输入不可变」、§4、REQ-017/AC-027）。
CREATE TRIGGER generation_snapshots_immutable
BEFORE UPDATE ON generation_snapshots
BEGIN
    SELECT RAISE(ABORT, 'generation_snapshots 是冻结输入，不允许修改');
END;

-- 2) manual_releases 是不可变完整快照（contracts.md §2/§7、REQ-035/AC-055）：
--    发布后内容（含 manifest 引用与 draft_revision）不允许改写或删除。
CREATE TRIGGER manual_releases_immutable
BEFORE UPDATE ON manual_releases
BEGIN
    SELECT RAISE(ABORT, 'manual_releases 是已发布快照，不允许修改');
END;

CREATE TRIGGER manual_releases_no_delete
BEFORE DELETE ON manual_releases
BEGIN
    SELECT RAISE(ABORT, 'manual_releases 是已发布快照，不允许删除（归档用 draft/物品字段）');
END;

-- 3) provider_attempts.remote_task_id 只允许 null→值或同值，不覆盖不同值、
--    也不允许把已知远端 ID 清空（contracts.md §2/§5：冲突记录并停机告警，
--    过期租约也允许把空 ID 补成返回值）。
CREATE TRIGGER provider_attempts_remote_task_id_monotonic
BEFORE UPDATE OF remote_task_id ON provider_attempts
WHEN OLD.remote_task_id IS NOT NULL
     AND (NEW.remote_task_id IS NULL OR NEW.remote_task_id <> OLD.remote_task_id)
BEGIN
    SELECT RAISE(ABORT, 'provider_attempts.remote_task_id 已存在：只允许 null→值或同值，不允许覆盖或清空');
END;

-- 4) 同一阶段只允许一个未对账 attempt（contracts.md §5 提交窗口第 1 条）：
--    intent/submitting/unknown 都是"未对账"；accepted/failed 已定性，允许后续重试
--    再建新 attempt。部分唯一索引把该约束钉在 SQL 上，而不是靠调用顺序。
CREATE UNIQUE INDEX provider_attempts_unresolved_stage
    ON provider_attempts (stage_id)
    WHERE submit_state IN ('intent', 'submitting', 'unknown');
