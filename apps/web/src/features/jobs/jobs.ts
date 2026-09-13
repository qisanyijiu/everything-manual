/**
 * 任务中心查询与动作（TanStack Query；PRD REQ-031 / UI-029–UI-039）。
 *
 * 轮询语义（UI-029 / AC-049）：
 * - **可见页约 2 秒、页面不可见约 15 秒、终态停止**；
 * - `refetchIntervalInBackground: true` 是刻意设置：react-query 默认在"窗口失焦"时
 *   完全暂停 interval，那样"后台 15 秒"就退化成"后台不轮询"；这里的降频由
 *   [`usePollingInterval`] 依据 `document.visibilityState` 自己决定；
 * - 网络错误**不是业务失败**（UI-033）：请求失败时保留最后一次数据，
 *   由页面显示"网络连接异常，正在自动重试（本地状态未变）"。
 */

import { useEffect, useState } from "react";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  cancelJob,
  getJob,
  listJobs,
  reconcileJob,
  retryJob,
  type CancelResultDto,
  type JobDetailDto,
  type JobSummaryDto,
  type ReconcileRequestDto,
  type ReconcileResultDto,
  type RetryResultDto,
} from "../../api/endpoints";
import { isTerminalJobStatus } from "./status";

/** 可见页轮询间隔（PRD §6.2 UI-029："约 2 秒"）。 */
export const JOBS_POLL_VISIBLE_MS = 2000;
/** 页面不可见时的轮询间隔（UI-029："约 15 秒"）。 */
export const JOBS_POLL_HIDDEN_MS = 15000;

export const JOBS_PAGE_SIZE = 20;

export const jobKeys = {
  root: ["jobs"] as const,
  /** 列表按起始游标与物品过滤分区（两者都由 URL 承载，§6.1.1）。 */
  list: (startCursor: string | null, itemId: string | null = null) =>
    ["jobs", "list", startCursor, itemId] as const,
  listRoot: ["jobs", "list"] as const,
  detail: (jobId: string) => ["jobs", "detail", jobId] as const,
};

/** 文档可见性（`document.visibilityState`）；SSR/无 document 时按可见处理。 */
export function useDocumentVisible(): boolean {
  const [visible, setVisible] = useState(() =>
    typeof document === "undefined" ? true : document.visibilityState !== "hidden",
  );
  useEffect(() => {
    if (typeof document === "undefined") {
      return;
    }
    const update = (): void => setVisible(document.visibilityState !== "hidden");
    update();
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  return visible;
}

/** 可见 2 秒 / 不可见 15 秒（UI-029；AC-049 的观察点）。 */
export function usePollingInterval(): number {
  return useDocumentVisible() ? JOBS_POLL_VISIBLE_MS : JOBS_POLL_HIDDEN_MS;
}

/**
 * 任务列表：游标分页（`startCursor` 由 URL 承载；U-03 排序由服务端给定）。
 *
 * 轮询条件：已加载的行里存在非终态任务时按当前间隔轮询；全部终态 → 停止。
 * 尚未拿到任何数据（首次失败）时继续轮询：网络恢复后自动回到服务端实际状态，
 * 不需要用户手动刷新。
 */
export function useJobList(startCursor: string | null, itemId: string | null = null) {
  const interval = usePollingInterval();
  return useInfiniteQuery({
    queryKey: jobKeys.list(startCursor, itemId),
    queryFn: ({ pageParam }) =>
      listJobs({
        itemId: itemId ?? undefined,
        cursor: pageParam === null ? undefined : pageParam,
        limit: JOBS_PAGE_SIZE,
      }),
    initialPageParam: startCursor,
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    retry: false,
    refetchIntervalInBackground: true,
    refetchInterval: (query) => {
      if (query.state.data === undefined) {
        return interval;
      }
      const jobs: JobSummaryDto[] = query.state.data.pages.flatMap((page) => page.jobs);
      return jobs.some((job) => !isTerminalJobStatus(job.status)) ? interval : false;
    },
  });
}

/** 任务详情：终态停止轮询；网络错误保留最后一次数据（UI-033）。 */
export function useJobDetail(jobId: string | null) {
  const interval = usePollingInterval();
  return useQuery({
    queryKey: jobKeys.detail(jobId ?? ""),
    queryFn: () => getJob(jobId ?? ""),
    enabled: jobId !== null && jobId !== "",
    retry: false,
    refetchIntervalInBackground: true,
    refetchInterval: (query) => {
      const resource = query.state.data;
      if (resource === undefined) {
        return interval;
      }
      return isTerminalJobStatus(resource.data.status) ? false : interval;
    },
  });
}

export interface JobMutationTarget {
  readonly jobId: string;
  /** 任务详情 GET 的 `ETag` 原样回传（缺 428、过期 412）。 */
  readonly ifMatch: string;
  /** 重试的幂等键：一次用户操作生成并复用（重放不产生第二个 attempt）。 */
  readonly idempotencyKey?: string;
}

export function useCancelJob() {
  const queryClient = useQueryClient();
  return useMutation<CancelResultDto, unknown, JobMutationTarget>({
    mutationFn: ({ jobId, ifMatch }) => cancelJob(jobId, ifMatch),
    onSuccess: (_result, variables) => {
      void queryClient.invalidateQueries({ queryKey: jobKeys.root });
      void queryClient.invalidateQueries({ queryKey: jobKeys.detail(variables.jobId) });
    },
  });
}

export interface RetryVariables extends JobMutationTarget {
  readonly stageId: string;
  readonly idempotencyKey: string;
}

export function useRetryJob() {
  const queryClient = useQueryClient();
  return useMutation<RetryResultDto, unknown, RetryVariables>({
    mutationFn: ({ jobId, stageId, ifMatch, idempotencyKey }) =>
      retryJob(jobId, stageId, ifMatch, idempotencyKey),
    onSuccess: (_result, variables) => {
      void queryClient.invalidateQueries({ queryKey: jobKeys.detail(variables.jobId) });
      void queryClient.invalidateQueries({ queryKey: jobKeys.listRoot });
    },
  });
}

export interface ReconcileVariables extends JobMutationTarget {
  readonly request: ReconcileRequestDto;
}

export function useReconcileJob() {
  const queryClient = useQueryClient();
  return useMutation<ReconcileResultDto, unknown, ReconcileVariables>({
    mutationFn: ({ jobId, request, ifMatch }) => reconcileJob(jobId, request, ifMatch),
    onSuccess: (_result, variables) => {
      void queryClient.invalidateQueries({ queryKey: jobKeys.detail(variables.jobId) });
      void queryClient.invalidateQueries({ queryKey: jobKeys.listRoot });
    },
  });
}

/** 详情查询的响应类型别名（页面与测试引用）。 */
export type JobDetailResource = { readonly data: JobDetailDto; readonly etag: string | null };

/**
 * 412 冲突的 `details.currentRevision`（UI-008）；缺失时为 null（不编造版本号）。
 * 取消/重试/对账共用：刷新提示必须给出服务端的当前版本。
 */
export function readCurrentRevision(details: unknown): number | null {
  if (typeof details !== "object" || details === null) {
    return null;
  }
  const value = (details as { currentRevision?: unknown }).currentRevision;
  return typeof value === "number" ? value : null;
}
