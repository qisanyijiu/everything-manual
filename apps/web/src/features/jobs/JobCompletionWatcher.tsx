/**
 * 全局「生成完成」提示（挂在 AppShell，任何页面都生效）。
 *
 * - 轮询最近一页任务（与任务中心同一套可见/不可见间隔）；没有进行中的任务时停止轮询，
 *   直到有页面让任务查询失效（确认生成后任务中心/详情会刷新同一个 `jobs` 根键）；
 * - 只对**本次会话里亲眼见过处于非终态**的任务提示：首次加载时已经结束的历史任务不弹；
 * - 成功：常驻提示 + 「查看生成结果」直达结果页；失败/取消：alert 提示 + 「查看任务」；
 * - 同时更新页签标题（后台标签页也能看到），回到前台后恢复。
 */

import { useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";

import { listJobs, type JobSummaryDto } from "../../api/endpoints";
import { useNotify } from "../../components/notifications";
import { isTerminalJobStatus } from "./status";
import { jobKeys, usePollingInterval } from "./jobs";

const WATCH_PAGE_SIZE = 20;

function itemLabel(job: JobSummaryDto): string {
  return job.itemModel !== "" ? `${job.itemName}（${job.itemModel}）` : job.itemName;
}

export function JobCompletionWatcher() {
  const notify = useNotify();
  const interval = usePollingInterval();
  /** 本会话见过的非终态任务（只对它们的结束弹提示）。 */
  const pending = useRef(new Set<string>());
  const baseTitle = useRef<string | null>(null);

  const query = useQuery({
    queryKey: [...jobKeys.root, "watch"],
    queryFn: () => listJobs({ limit: WATCH_PAGE_SIZE }),
    retry: false,
    refetchIntervalInBackground: true,
    refetchInterval: (state) => {
      const jobs = state.state.data?.jobs;
      if (jobs === undefined) {
        return interval;
      }
      return jobs.some((job) => !isTerminalJobStatus(job.status)) || pending.current.size > 0
        ? interval
        : false;
    },
  });

  useEffect(() => {
    const jobs = query.data?.jobs;
    if (jobs === undefined) {
      return;
    }
    for (const job of jobs) {
      if (!isTerminalJobStatus(job.status)) {
        pending.current.add(job.id);
        continue;
      }
      if (!pending.current.delete(job.id)) {
        continue;
      }
      if (job.status === "succeeded") {
        notify(`「${itemLabel(job)}」生成完成：3D 模型与说明书草稿已就绪。`, {
          sticky: true,
          action: { label: "查看生成结果", to: `/jobs/${job.id}/result` },
        });
        if (document.visibilityState === "hidden") {
          baseTitle.current ??= document.title;
          document.title = `✓ 生成完成 · ${baseTitle.current}`;
        }
      } else {
        notify(`「${itemLabel(job)}」生成${job.status === "cancelled" ? "已取消" : "失败"}。`, {
          kind: "alert",
          action: { label: "查看任务", to: `/jobs/${job.id}` },
        });
      }
    }
  }, [query.data, notify]);

  useEffect(() => {
    const restore = (): void => {
      if (document.visibilityState === "visible" && baseTitle.current !== null) {
        document.title = baseTitle.current;
        baseTitle.current = null;
      }
    };
    document.addEventListener("visibilitychange", restore);
    return () => document.removeEventListener("visibilitychange", restore);
  }, []);

  return null;
}
