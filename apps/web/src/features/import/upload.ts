/**
 * 资产上传（`POST /items/{id}/assets`，multipart：`purpose` + `file`）。
 *
 * 为什么用 XMLHttpRequest 而不是 fetch：UI-009 要求**按字节显示上传进度**并可取消；
 * fetch 没有上传进度事件，XHR 的 `upload.onprogress` 是唯一可观察的真实进度源
 * （不伪造假进度条）。其余请求仍统一走 `src/api/client.ts`。
 *
 * 错误形态（T06 / ADR-016 惯例，QA 按 `details.reason` 断言）：
 * - 415 类型不支持；413 分两类——输入超限 / 物品累计上限（`itemTotalLimit`）与
 *   磁盘预留不足（`insufficientStorage`，含 `requiredBytes`/`availableBytes`，A-14/D-2）；
 * - 422 内容校验失败（像素炸弹、解码失败等）；
 * - 403 `CSRF_REJECTED` 视为 token 轮换：刷新一次后自动重试（与既有行为一致）。
 *
 * CSRF token 只存在模块作用域（不写 DOM/localStorage，安全约定见 ADR-013/018）。
 */

import { fetchSession, API_PREFIX } from "../../api/endpoints";
import type { components } from "../../api/generated";

export type AssetDto = components["schemas"]["AssetDto"];

/** 上传用途（合同取值：document / photo / pageImage / pageText）。 */
export type AssetPurpose = "document" | "photo" | "pageText" | "pageImage";

export interface UploadProgress {
  /** 已发送字节（真实计数；不按时间估算）。 */
  readonly loaded: number;
  /** 文件总字节。 */
  readonly total: number;
}

export interface UploadOptions {
  readonly onProgress?: (progress: UploadProgress) => void;
  /** 取消上传（用户点「取消」或离开流程）。 */
  readonly signal?: AbortSignal;
}

/** 上传失败：保留 HTTP 状态、合同错误码与 `details`，供界面给出可行动文案。 */
export class AssetUploadError extends Error {
  readonly status: number | null;
  readonly code: string | null;
  readonly details: unknown;
  readonly requestId: string | null;
  readonly aborted: boolean;

  constructor(init: {
    message: string;
    status: number | null;
    code?: string | null;
    details?: unknown;
    requestId?: string | null;
    aborted?: boolean;
  }) {
    super(init.message);
    this.name = "AssetUploadError";
    this.status = init.status;
    this.code = init.code ?? null;
    this.details = init.details ?? null;
    this.requestId = init.requestId ?? null;
    this.aborted = init.aborted ?? false;
  }
}

export function isAssetUploadError(value: unknown): value is AssetUploadError {
  return value instanceof AssetUploadError;
}

let cachedCsrfToken: string | null = null;

/**
 * 取当前会话的 CSRF token（写请求需要）。
 *
 * 与 `src/api/client.ts` 保持的是同一会话的派生值；这里单独缓存只为 multipart 请求
 * （XHR 不走 client.ts 的注入路径）。
 */
export async function multipartCsrfToken(refresh = false): Promise<string> {
  if (!refresh && cachedCsrfToken !== null) {
    return cachedCsrfToken;
  }
  const session = await fetchSession();
  cachedCsrfToken = session.data.csrfToken;
  return cachedCsrfToken;
}

/** 测试用：清空缓存的 CSRF token。 */
export function resetMultipartCsrfTokenCache(): void {
  cachedCsrfToken = null;
}

interface RawUploadResponse {
  readonly status: number;
  readonly body: unknown;
}

function parseJsonText(text: string): unknown {
  if (text === "") {
    return null;
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return null;
  }
}

function errorParts(body: unknown): {
  code: string | null;
  message: string | null;
  details: unknown;
  requestId: string | null;
} {
  if (typeof body !== "object" || body === null) {
    return { code: null, message: null, details: null, requestId: null };
  }
  const error = (body as { error?: unknown }).error;
  if (typeof error !== "object" || error === null) {
    return { code: null, message: null, details: null, requestId: null };
  }
  const { code, message, details, requestId } = error as {
    code?: unknown;
    message?: unknown;
    details?: unknown;
    requestId?: unknown;
  };
  return {
    code: typeof code === "string" ? code : null,
    message: typeof message === "string" ? message : null,
    details: details ?? null,
    requestId: typeof requestId === "string" ? requestId : null,
  };
}

function sendUpload(
  itemId: string,
  purpose: AssetPurpose,
  file: Blob,
  filename: string,
  csrfToken: string,
  options: UploadOptions,
): Promise<RawUploadResponse> {
  return new Promise<RawUploadResponse>((resolve, reject) => {
    const form = new FormData();
    form.append("purpose", purpose);
    form.append("file", file, filename);

    const xhr = new XMLHttpRequest();
    xhr.open("POST", `${API_PREFIX}/items/${encodeURIComponent(itemId)}/assets`);
    xhr.withCredentials = true;
    xhr.setRequestHeader("x-csrf-token", csrfToken);
    xhr.setRequestHeader("accept", "application/json");

    const onAbort = (): void => {
      xhr.abort();
    };
    const cleanup = (): void => {
      options.signal?.removeEventListener("abort", onAbort);
    };

    xhr.upload.onprogress = (event: ProgressEvent): void => {
      options.onProgress?.({ loaded: event.loaded, total: file.size });
    };
    xhr.onload = (): void => {
      cleanup();
      resolve({ status: xhr.status, body: parseJsonText(xhr.responseText) });
    };
    xhr.onerror = (): void => {
      cleanup();
      reject(
        new AssetUploadError({
          message: "无法连接服务：上传请求失败，请确认后端进程正在运行",
          status: null,
        }),
      );
    };
    xhr.onabort = (): void => {
      cleanup();
      reject(new AssetUploadError({ message: "上传已取消", status: null, aborted: true }));
    };

    if (options.signal !== undefined) {
      if (options.signal.aborted) {
        reject(new AssetUploadError({ message: "上传已取消", status: null, aborted: true }));
        return;
      }
      options.signal.addEventListener("abort", onAbort, { once: true });
    }

    xhr.send(form);
  });
}

/**
 * 上传一个资产并返回 `AssetDto`。
 *
 * 失败时抛 `AssetUploadError`（带 status/code/details），界面据此给出可行动文案；
 * 取消不产生错误卡片（调用方按 `aborted` 区分）。
 */
export async function uploadAsset(
  itemId: string,
  purpose: AssetPurpose,
  file: Blob,
  filename: string,
  options: UploadOptions = {},
): Promise<AssetDto> {
  let token = await multipartCsrfToken();
  let response = await sendUpload(itemId, purpose, file, filename, token, options);

  // 会话轮换：403 CSRF_REJECTED 时刷新 token 重试一次（其余 403 直接失败）。
  if (response.status === 403) {
    const parts = errorParts(response.body);
    if (parts.code === "CSRF_REJECTED") {
      token = await multipartCsrfToken(true);
      response = await sendUpload(itemId, purpose, file, filename, token, options);
    }
  }

  const parts = errorParts(response.body);
  if (response.status < 200 || response.status >= 300) {
    throw new AssetUploadError({
      message: parts.message ?? `上传失败（HTTP ${response.status}）`,
      status: response.status,
      code: parts.code,
      details: parts.details,
      requestId: parts.requestId,
    });
  }
  const data = (response.body as { data?: AssetDto } | null)?.data;
  if (data === undefined) {
    throw new AssetUploadError({
      message: "上传响应不符合合同（缺少 data）",
      status: response.status,
    });
  }
  return data;
}
