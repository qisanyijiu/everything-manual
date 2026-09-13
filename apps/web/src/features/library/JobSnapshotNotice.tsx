/**
 * 冻结快照常驻提示（PRD §6.2 UI-021 / REQ-017）。
 *
 * 触发：该物品存在**进行中**的任务（`GET /jobs?itemId=` 的非终态状态）。
 * 文案要点（UI-021）：当前任务使用提交时的资料快照；你的修改用于下次生成，
 * 需要重新报价确认；不提供修改使用中快照的入口。
 *
 * 只读查询，不承担任务中心职责（T17）；查询失败时静默不显示（不把只读提示
 * 变成页面错误），但也不伪造"没有进行中任务"的结论以外的内容。
 */

import { useQuery } from "@tanstack/react-query";

import { listJobs } from "../../api/endpoints";
import { formatLocalDateTime } from "../../lib/format";

/** 终态（不再消费快照）。 */
const TERMINAL_STATUSES = new Set(["succeeded", "failed", "cancelled"]);

export function JobSnapshotNotice({ itemId }: { itemId: string }) {
  const jobsQuery = useQuery({
    queryKey: ["jobs", "item", itemId],
    queryFn: () => listJobs({ itemId, limit: 20 }),
    retry: false,
  });

  const active = (jobsQuery.data?.jobs ?? []).filter((job) => !TERMINAL_STATUSES.has(job.status));
  if (active.length === 0) {
    return null;
  }

  return (
    <div className="snapshot-notice" role="note" data-testid="snapshot-notice">
      <h2 className="snapshot-notice__title">进行中任务使用冻结的资料快照</h2>
      <p>
        该物品有 {active.length} 个进行中的任务：它们继续使用提交时冻结的快照
        （最早创建于 {formatLocalDateTime(active[active.length - 1]!.createdAt)}）。
      </p>
      <p>
        你对资料（视图照片、说明书、物品信息）的修改用于<strong>下一次</strong>生成，需要重新报价并确认；
        本版本不提供修改使用中快照的入口。
      </p>
      <p className="snapshot-notice__meta">
        快照时间与输入摘要以服务端为准；任务阶段与恢复入口由任务中心（T17）展示。
      </p>
    </div>
  );
}
