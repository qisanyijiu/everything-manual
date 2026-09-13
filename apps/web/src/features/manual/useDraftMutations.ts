/**
 * 校准工作区的写入 hook（T19）：所有修改都走 `PATCH /items/{id}/drafts/{draftId}`，
 * 带 `If-Match`（来自 GET 的 ETag），成功后失效草稿查询重新读取服务端事实。
 *
 * 错误语义（UI-008/UI-056）：
 * - 412：显示「该内容已被其他操作更新（当前 rN）」+ 刷新入口，不自动覆盖、不丢弃输入；
 * - 422：字段级明细按服务端 `details.fields` 原样展示（不翻译、不掩盖）；
 * - 网络错误与业务失败分开表达（`describeError`）。
 */

import { useCallback, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { QueryClient } from "@tanstack/react-query";

import { describeError, isApiError } from "../../api/client";
import { patchDraft, type CameraPoseDto, type DraftPatchRequest } from "../../api/endpoints";

export interface DraftMutations {
  readonly lastError: string | null;
  readonly conflictRevision: number | null;
  /** 本会话内 3D 模型是否真的成功加载过（只有它才能声明 loaded；UI-052）。 */
  readonly modelReady: boolean;
  setModelReady: (ready: boolean) => void;
  createHotspot: (ifMatch: string, body: DraftPatchRequest) => void;
  rebindHotspot: (ifMatch: string, body: DraftPatchRequest) => void;
  updateEntities: (ifMatch: string, body: DraftPatchRequest) => void;
  updateModelReview: (ifMatch: string, body: DraftPatchRequest) => void;
  savePose: (ifMatch: string, stepId: string, pose: CameraPoseDto) => void;
  clearPose: (ifMatch: string, stepId: string) => void;
  clearError: () => void;
}

function currentRevisionFrom(error: unknown): number | null {
  if (!isApiError(error) || error.status !== 412) {
    return null;
  }
  const details = error.details;
  if (typeof details === "object" && details !== null) {
    const value = (details as { currentRevision?: unknown }).currentRevision;
    if (typeof value === "number") {
      return value;
    }
  }
  return null;
}

/** 字段级明细文本（422：`details.fields[]`；没有明细时退回 message）。 */
export function describeFieldIssues(error: unknown): string {
  if (isApiError(error) && error.status === 422) {
    const details = error.details;
    if (typeof details === "object" && details !== null) {
      const fields = (details as { fields?: unknown }).fields;
      if (Array.isArray(fields) && fields.length > 0) {
        return fields
          .map((field) => {
            const record = field as { message?: unknown };
            return typeof record.message === "string" ? record.message : null;
          })
          .filter((message): message is string => message !== null)
          .join("；");
      }
    }
  }
  return describeError(error).message;
}

function invalidateDraft(queryClient: QueryClient, itemId: string, draftId: string): void {
  void queryClient.invalidateQueries({ queryKey: ["draft", itemId, draftId] });
}

export function useDraftMutations(itemId: string, draftId: string): DraftMutations {
  const queryClient = useQueryClient();
  const [lastError, setLastError] = useState<string | null>(null);
  const [conflictRevision, setConflictRevision] = useState<number | null>(null);
  const [modelReady, setModelReady] = useState(false);

  const mutation = useMutation({
    mutationFn: (input: { body: DraftPatchRequest; ifMatch: string }) =>
      patchDraft(itemId, draftId, input.body, input.ifMatch),
    onSuccess: () => {
      setLastError(null);
      setConflictRevision(null);
      invalidateDraft(queryClient, itemId, draftId);
    },
    onError: (error: unknown) => {
      const revision = currentRevisionFrom(error);
      if (revision !== null) {
        setConflictRevision(revision);
        setLastError(null);
        return;
      }
      setLastError(describeFieldIssues(error));
    },
  });

  const submit = useCallback(
    (body: DraftPatchRequest, ifMatch: string) => {
      setLastError(null);
      setConflictRevision(null);
      mutation.mutate({ body, ifMatch });
    },
    [mutation],
  );

  return {
    lastError,
    conflictRevision,
    modelReady,
    setModelReady,
    // 成功提示由页面按动作给出；这里只提交请求体（去掉未使用的 status）。
    createHotspot: (ifMatch, body) => submit(withoutStatus(body), ifMatch),
    rebindHotspot: (ifMatch, body) => submit(withoutStatus(body), ifMatch),
    updateEntities: (ifMatch, body) => submit(withoutStatus(body), ifMatch),
    updateModelReview: (ifMatch, body) => submit(withoutStatus(body), ifMatch),
    savePose: (ifMatch, stepId, pose) => submit(toStepPoseBody(stepId, pose), ifMatch),
    clearPose: (ifMatch, stepId) => submit({ clearStepPoses: [stepId] }, ifMatch),
    clearError: () => {
      setLastError(null);
      setConflictRevision(null);
    },
  };
}

/** `CameraPose`（只读元组）→ 线上请求体（可变数组）。 */
function toStepPoseBody(stepId: string, pose: CameraPoseDto): DraftPatchRequest {
  return {
    stepPoses: {
      [stepId]: {
        positionLocal: [...pose.positionLocal],
        targetLocal: [...pose.targetLocal],
        upLocal: [...pose.upLocal],
        fov: pose.fov,
      },
    },
  };
}

/** 去掉未使用的 status 字段，保证请求体只包含本次动作的字段（服务端 422 更精确）。 */
function withoutStatus(body: DraftPatchRequest): DraftPatchRequest {
  const { status: _status, ...rest } = body;
  return rest;
}
