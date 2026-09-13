/**
 * 前端 HTTP 封装：类型全部来自 `cargo xtask contracts` 生成的机器合同（禁止手抄 DTO）。
 *
 * 约定（llmdoc/contracts.md §1）：
 * - 单项响应 `{ data }`、列表 `{ data, nextCursor }`、错误统一
 *   `error.code/message/details/requestId`；
 * - 修改请求必须携带 `X-CSRF-Token`（T04 ADR-013 第 1 条）；
 * - 可编辑聚合根带整数 revision，GET 返回 `ETag: "r<n>"`，PATCH 必须回传 `If-Match`，
 *   缺 428、冲突 412（`details.currentRevision`）。
 *
 * 安全：CSRF token 只保存在模块作用域（内存），不写 localStorage/sessionStorage、
 * 不写 DOM、不写日志；会话凭据在 HttpOnly cookie 中，前端 JS 读不到。
 */

import type { components } from "./generated";

export type ApiErrorBody = components["schemas"]["ApiErrorBody"];
export type ApiErrorResponse = components["schemas"]["ApiErrorResponse"];

/** 服务端要求的 CSRF 头名（`crates/server/src/http/auth/mod.rs` 的 `CSRF_HEADER`）。 */
export const CSRF_HEADER = "x-csrf-token";

/** 合同错误结构对应的异常；`code` 为 null 表示响应不符合合同（如代理/网关错误页）。 */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string | null;
  readonly requestId: string | null;
  readonly details: unknown;

  constructor(
    status: number,
    code: string | null,
    message: string,
    requestId: string | null,
    details: unknown = null,
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.requestId = requestId;
    this.details = details;
  }
}

/** 单个资源的响应：解包后的 `data` 与响应头 ETag（原样回传，不自行解析 `"r7"`）。 */
export interface ApiResource<T> {
  readonly data: T;
  readonly etag: string | null;
}

export function isApiError(value: unknown): value is ApiError {
  return value instanceof ApiError;
}

/** 服务端返回的错误体是否符合合同结构。 */
export function isApiErrorResponse(value: unknown): value is ApiErrorResponse {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const error = (value as { error?: unknown }).error;
  if (typeof error !== "object" || error === null) {
    return false;
  }
  const { code, message, requestId } = error as {
    code?: unknown;
    message?: unknown;
    requestId?: unknown;
  };
  return typeof code === "string" && typeof message === "string" && typeof requestId === "string";
}

/** 把任意异常转成界面可读的文案；网络层失败与合同错误分开表达。 */
export function describeError(error: unknown): {
  message: string;
  requestId: string | null;
  code: string | null;
  status: number | null;
} {
  if (isApiError(error)) {
    return {
      message: error.message,
      requestId: error.requestId,
      code: error.code,
      status: error.status,
    };
  }
  if (error instanceof TypeError) {
    return {
      message: "无法连接服务：网络请求失败，请确认后端进程正在运行",
      requestId: null,
      code: null,
      status: null,
    };
  }
  if (error instanceof Error) {
    return { message: error.message, requestId: null, code: null, status: null };
  }
  return { message: String(error), requestId: null, code: null, status: null };
}

// ---------------------------------------------------------------------------
// 会话级状态（模块作用域，不落 DOM）
// ---------------------------------------------------------------------------

/** CSRF token：登录/会话恢复响应中取得，所有修改请求自动注入。 */
let csrfToken: string | null = null;

export function setCsrfToken(token: string | null): void {
  csrfToken = token;
}

/**
 * 最近一次响应头 `x-request-id`。
 * 错误边界在「渲染异常先于 API 错误被捕获」时用它展示可追踪的诊断 ID；
 * 只保存服务端生成的诊断 ID，不含任何凭据。
 */
let mostRecentRequestId: string | null = null;

export function lastRequestId(): string | null {
  return mostRecentRequestId;
}

type UnauthorizedListener = () => void;

const unauthorizedListeners = new Set<UnauthorizedListener>();

/** 订阅「任意 API 返回 401」事件（UI-002：丢弃本地状态并跳登录）。返回取消订阅函数。 */
export function onUnauthorized(listener: UnauthorizedListener): () => void {
  unauthorizedListeners.add(listener);
  return () => {
    unauthorizedListeners.delete(listener);
  };
}

function notifyUnauthorized(): void {
  for (const listener of [...unauthorizedListeners]) {
    listener();
  }
}

// ---------------------------------------------------------------------------
// 请求核心
// ---------------------------------------------------------------------------

export type QueryParams = Record<string, string | number | boolean | undefined | null>;

export interface RequestOptions {
  method?: "GET" | "POST" | "PATCH" | "PUT" | "DELETE";
  body?: unknown;
  query?: QueryParams;
  /** `If-Match` 的值：GET 响应头 `ETag` 原样回传（含引号），不自行拼接。 */
  ifMatch?: string | null;
  /**
   * 附加请求头（如建单的 `Idempotency-Key`）。
   * 只在没有同名固定头时生效：不能借此覆盖 CSRF／If-Match 的注入逻辑。
   */
  headers?: Record<string, string>;
  cache?: RequestCache;
  /** 取消信号（页面卸载/切换模型时中止进行中的请求）。 */
  signal?: AbortSignal;
  /**
   * 401 时是否广播全局会话失效（默认 true）。
   * 登录页与会话探测自身传 false：它们自己处理未登录状态，避免跳转循环。
   */
  handleUnauthorized?: boolean;
  /**
   * 例外状态码：这些状态不抛错，按正常载荷解析。
   * 目前只有 `/health/ready` 的 503（未就绪仍是 `{ data }` 结构，ADR-013 第 9 条）。
   */
  tolerateStatuses?: readonly number[];
}

export interface JsonResponse<T> {
  readonly body: T;
  readonly etag: string | null;
  readonly status: number;
  /** 原始响应头（T19：发布重放的 `x-idempotent-replay` 等标记）。 */
  readonly headers: Headers;
}

function buildUrl(path: string, query?: QueryParams): string {
  if (!query) {
    return path;
  }
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null || value === "") {
      continue;
    }
    params.set(key, String(value));
  }
  const queryString = params.toString();
  return queryString === "" ? path : `${path}?${queryString}`;
}

function parseJson(text: string): unknown {
  if (text === "") {
    return null;
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return null;
  }
}

function toApiError(status: number, body: unknown, headerRequestId: string | null): ApiError {
  if (isApiErrorResponse(body)) {
    return new ApiError(
      status,
      body.error.code,
      body.error.message,
      body.error.requestId ?? headerRequestId,
      body.error.details ?? null,
    );
  }
  return new ApiError(
    status,
    null,
    `服务返回了非合同错误响应（HTTP ${status}）`,
    headerRequestId,
    null,
  );
}

/** 原始请求：返回完整响应体（用于列表等自带包装的响应）。 */
async function requestJson<T>(
  path: string,
  options: RequestOptions = {},
): Promise<JsonResponse<T>> {
  const method = options.method ?? "GET";
  const headers: Record<string, string> = { accept: "application/json" };
  const init: RequestInit = { method, headers, credentials: "same-origin" };

  if (options.cache !== undefined) {
    init.cache = options.cache;
  }
  if (options.body !== undefined) {
    headers["content-type"] = "application/json";
    init.body = JSON.stringify(options.body);
  }
  if (method !== "GET" && csrfToken !== null) {
    headers[CSRF_HEADER] = csrfToken;
  }
  if (options.ifMatch) {
    headers["if-match"] = options.ifMatch;
  }
  if (options.headers !== undefined) {
    for (const [name, value] of Object.entries(options.headers)) {
      const key = name.toLowerCase();
      if (!(key in headers)) {
        headers[key] = value;
      }
    }
  }

  const response = await fetch(buildUrl(path, options.query), init);
  const headerRequestId = response.headers.get("x-request-id");
  if (headerRequestId !== null && headerRequestId !== "") {
    mostRecentRequestId = headerRequestId;
  }

  const body = parseJson(await response.text());
  const tolerated = options.tolerateStatuses?.includes(response.status) ?? false;
  if (!response.ok && !tolerated) {
    const error = toApiError(response.status, body, headerRequestId);
    if (error.status === 401 && options.handleUnauthorized !== false) {
      notifyUnauthorized();
    }
    throw error;
  }
  return {
    body: body as T,
    etag: response.headers.get("etag"),
    status: response.status,
    headers: response.headers,
  };
}

/** 解包 `{ data }` 的单资源请求。 */
async function requestData<T>(
  path: string,
  options: RequestOptions = {},
): Promise<ApiResource<T>> {
  const { body, etag } = await requestJson<{ data: T }>(path, options);
  return { data: body.data, etag };
}

/** 二进制响应（资产内容：GLB、原 PDF）。 */
export interface ByteResponse {
  readonly bytes: ArrayBuffer;
  readonly contentType: string | null;
  /** 服务端给出的 `content-length`（缺失时为 null：进度只显示"已接收"）。 */
  readonly contentLength: number | null;
}

export interface ByteRequestOptions extends RequestOptions {
  /** 下载进度回调（字节数；只在浏览器能给出流时逐块触发）。 */
  readonly onProgress?: (receivedBytes: number, totalBytes: number | null) => void;
}

/**
 * 取字节的请求（`/assets/{id}/content` 这类二进制端点）。
 *
 * 与 JSON 请求共享同一套语义：CSRF 注入规则、401 广播、合同错误解析（错误体
 * 仍是 `{ error: { code, message, details, requestId } }`）。返回 `ArrayBuffer`
 * 而不是解析 JSON —— 资产内容不是 JSON。
 */
async function requestBytes(path: string, options: ByteRequestOptions = {}): Promise<ByteResponse> {
  const headers: Record<string, string> = { accept: "*/*" };
  const init: RequestInit = { method: options.method ?? "GET", headers, credentials: "same-origin" };
  if (options.cache !== undefined) {
    init.cache = options.cache;
  }
  if (options.ifMatch) {
    headers["if-match"] = options.ifMatch;
  }
  if (options.headers !== undefined) {
    for (const [name, value] of Object.entries(options.headers)) {
      const key = name.toLowerCase();
      if (!(key in headers)) {
        headers[key] = value;
      }
    }
  }
  if (options.signal !== undefined) {
    init.signal = options.signal;
  }

  const response = await fetch(buildUrl(path, options.query), init);
  const headerRequestId = response.headers.get("x-request-id");
  if (headerRequestId !== null && headerRequestId !== "") {
    mostRecentRequestId = headerRequestId;
  }
  if (!response.ok) {
    const error = toApiError(response.status, parseJson(await response.text()), headerRequestId);
    if (error.status === 401 && options.handleUnauthorized !== false) {
      notifyUnauthorized();
    }
    throw error;
  }

  const contentType = response.headers.get("content-type");
  const declared = response.headers.get("content-length");
  const contentLength = declared === null ? null : Number(declared);
  const total =
    contentLength !== null && Number.isFinite(contentLength) && contentLength >= 0
      ? contentLength
      : null;
  const body = response.body;
  if (body === null || options.onProgress === undefined) {
    const bytes = await response.arrayBuffer();
    options.onProgress?.(bytes.byteLength, total ?? bytes.byteLength);
    return { bytes, contentType, contentLength: total };
  }
  // 有流且需要进度：逐块读取（模型可能是几十 MB，整块 await 会没有中间反馈）。
  const reader = body.getReader();
  const chunks: Uint8Array[] = [];
  let received = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    if (value !== undefined) {
      chunks.push(value);
      received += value.byteLength;
      options.onProgress(received, total);
    }
  }
  const merged = new Uint8Array(received);
  let offset = 0;
  for (const chunk of chunks) {
    merged.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return { bytes: merged.buffer, contentType, contentLength: total };
}

export { requestBytes, requestData, requestJson };
