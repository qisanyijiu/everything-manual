import { useMemoryEdit } from "../shell/work-protection";
/**
 * 校准工作区的写入 hook（T19）：所有修改都走 `PATCH /items/{id}/drafts/{draftId}`，
 * 带 `If-Match`（来自 GET 的 ETag），成功后显式 GET 回读草稿，完成后才更新缓存及报告成功。
 *
 * 错误语义（UI-008/UI-056）：
 * - 412：显示「该内容已被其他操作更新（当前 rN）」+ 刷新入口，不自动覆盖、不丢弃输入；
 * - 422：字段级明细按服务端 `details.fields` 原样展示（不翻译、不掩盖）；
 * - 网络错误与业务失败分开表达（`describeError`）。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { describeError, isApiError } from "../../api/client";
import { getDraft, patchDraft, type CameraPoseDto, type DraftPatchRequest } from "../../api/endpoints";

export interface DraftMutations {
  readonly lastError: string | null;
  readonly conflictRevision: number | null;
  /** 本会话内 3D 模型是否真的成功加载过（只有它才能声明 loaded；UI-052）。 */
  readonly modelReady: boolean;
  readonly pending: boolean;
  readonly needsRead: boolean;
  setModelReady: (ready: boolean) => void;
  createHotspot: (ifMatch: string, body: DraftPatchRequest) => Promise<boolean>;
  rebindHotspot: (ifMatch: string, body: DraftPatchRequest) => Promise<boolean>;
  updateEntities: (ifMatch: string, body: DraftPatchRequest) => Promise<boolean>;
  updateModelReview: (ifMatch: string, body: DraftPatchRequest) => Promise<boolean>;
  savePose: (ifMatch: string, stepId: string, pose: CameraPoseDto) => Promise<boolean>;
  clearPose: (ifMatch: string, stepId: string) => Promise<boolean>;
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

export function useDraftMutations(itemId: string, draftId: string, modelIdentity: string): DraftMutations {
  const queryClient = useQueryClient();
  const [failure, setFailure] = useMemoryEdit<{ lastError: string | null; conflictRevision: number | null; needsRead: boolean }>(`draft-write:${itemId}/${draftId}`, { lastError: null, conflictRevision: null, needsRead: false });
  const { lastError, conflictRevision, needsRead } = failure;
  const [loadedIdentity, setLoadedIdentity] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const lock = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const submit = useCallback(async (body: DraftPatchRequest, ifMatch: string): Promise<boolean> => {
    if (lock.current || needsRead) return false;
    lock.current = true;
    setPending(true);
    setFailure({ lastError: null, conflictRevision: null, needsRead: true });
    let written = false;
    try {
      await patchDraft(itemId, draftId, body, ifMatch);
      written = true;
      if (!mounted.current) return false;
      const resource = await getDraft(itemId, draftId);
      if (!mounted.current) return false;
      queryClient.setQueryData(["draft", itemId, draftId], resource);
      setFailure({ lastError: null, conflictRevision: null, needsRead: false });
      return true;
    } catch (error) {
      const revision = currentRevisionFrom(error);
      setFailure({ conflictRevision: revision,
        lastError: revision !== null ? null : written ? `修改已提交，但读取结果失败；请核对最新版本。${describeError(error).message}` : describeFieldIssues(error),
        needsRead: written || (isApiError(error) && (error.status === 412 || error.status >= 500)) || !isApiError(error) });
      return false;
    } finally {
      lock.current = false;
      if (mounted.current) setPending(false);
    }
  }, [itemId, draftId, queryClient, needsRead, setFailure]);

  return {
    lastError,
    conflictRevision,
    pending, needsRead,
    modelReady: loadedIdentity !== null && loadedIdentity === modelIdentity,
    setModelReady: (ready) => setLoadedIdentity(ready ? modelIdentity : null),
    // 成功提示由页面按动作给出；这里只提交请求体（去掉未使用的 status）。
    createHotspot: (ifMatch, body) => submit(withoutStatus(body), ifMatch),
    rebindHotspot: (ifMatch, body) => submit(withoutStatus(body), ifMatch),
    updateEntities: (ifMatch, body) => submit(withoutStatus(body), ifMatch),
    updateModelReview: (ifMatch, body) => submit(withoutStatus(body), ifMatch),
    savePose: (ifMatch, stepId, pose) => submit(toStepPoseBody(stepId, pose), ifMatch),
    clearPose: (ifMatch, stepId) => submit({ clearStepPoses: [stepId] }, ifMatch),
    clearError: () => {
      setFailure({ lastError: null, conflictRevision: null, needsRead: false });
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
