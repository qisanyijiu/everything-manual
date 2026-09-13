/**
 * 类型化端点封装：请求/响应类型全部从 `generated.ts` 的 `operations` 派生，
 * 不手抄 DTO、不手写路径字符串以外的东西。路径与合同（contracts.md §3）一致。
 */

import type { components, operations } from "./generated";
import {
  requestBytes,
  requestData,
  requestJson,
  type ApiResource,
  type ByteResponse,
  type QueryParams,
} from "./client";

/** 合同前缀（contracts.md §1：URL 前缀 `/api/v1`）。 */
export const API_PREFIX = "/api/v1";

type JsonBody<R> = R extends { content: { "application/json": infer C } } ? C : never;

type OkStatus<Op extends keyof operations> = 200 extends keyof operations[Op]["responses"]
  ? 200
  : 201 extends keyof operations[Op]["responses"]
    ? 201
    : never;

type OkBody<Op extends keyof operations> = JsonBody<operations[Op]["responses"][OkStatus<Op>]>;

type OkData<Op extends keyof operations> = OkBody<Op> extends { data: infer D } ? D : never;

type JsonRequest<Op extends keyof operations> = operations[Op] extends {
  requestBody: { content: { "application/json": infer C } };
}
  ? C
  : never;

// --- 合同类型别名（供页面与测试引用，全部来自生成文件） ---
export type SessionData = OkData<"session">;
export type LoginRequest = JsonRequest<"login">;
export type ItemDto = OkData<"get_item">;
export type ItemCreateRequest = JsonRequest<"create_item">;
export type ItemPatchRequest = JsonRequest<"patch_item">;
export type ItemList = OkBody<"list_items">;
export type DocumentList = OkBody<"list_documents">;
export type PhotoList = OkBody<"list_photos">;
export type SettingsStatusData = OkData<"status">;
export type LivenessData = OkData<"live">;
export type ReadinessData = OkData<"ready">;
export type ReadinessCheck = components["schemas"]["ReadinessCheck"];
export type DocumentDto = components["schemas"]["DocumentDto"];
export type DocumentCreateRequest = JsonRequest<"create_document">;
export type PhotoDto = components["schemas"]["PhotoDto"];
export type PhotoCreateRequest = JsonRequest<"create_photo">;
export type PhotoPatchRequest = JsonRequest<"patch_photo">;
export type QuoteDto = OkData<"create_estimate">;
export type EstimateRequest = JsonRequest<"create_estimate">;
export type ConfirmationDto = OkData<"confirm_estimate">;
/** 建单响应只有 202（不是 200/201），因此直接取 schema 而不是 `OkData`。 */
export type JobDto = components["schemas"]["JobDto"];
export type JobCreateRequest = JsonRequest<"create_job">;
export type BudgetLimitsDto = components["schemas"]["BudgetLimitsDto"];

// --- auth ---

/**
 * `GET /auth/session`：刷新页面后恢复会话。
 * `cache: "no-store"` 与响应头一致（AC-003）；401 由会话恢复流程自己处理，不广播。
 */
export function fetchSession(): Promise<ApiResource<SessionData>> {
  return requestData<SessionData>(`${API_PREFIX}/auth/session`, {
    cache: "no-store",
    handleUnauthorized: false,
  });
}

/** `POST /auth/login`：成功返回会话数据（会话 token 只经 Set-Cookie 下发）。 */
export function login(body: LoginRequest): Promise<ApiResource<SessionData>> {
  return requestData<SessionData>(`${API_PREFIX}/auth/login`, {
    method: "POST",
    body,
    handleUnauthorized: false,
  });
}

/** `POST /auth/logout`：204，撤销会话并清 cookie。 */
export async function logout(): Promise<void> {
  await requestJson<null>(`${API_PREFIX}/auth/logout`, { method: "POST" });
}

// --- health 与设置 ---

export function fetchHealthLive(): Promise<ApiResource<LivenessData>> {
  return requestData<LivenessData>(`${API_PREFIX}/health/live`);
}

/**
 * `GET /health/ready`：就绪时 200，**未就绪时 503 仍是 `{ data }` 结构**
 * （ADR-013 第 9 条），因此这里把 503 当正常载荷解析，其它非 2xx 走统一错误。
 */
export async function fetchReadiness(): Promise<ReadinessData> {
  const { data } = await requestData<ReadinessData>(`${API_PREFIX}/health/ready`, {
    tolerateStatuses: [503],
  });
  return data;
}

export function fetchSettingsStatus(): Promise<ApiResource<SettingsStatusData>> {
  return requestData<SettingsStatusData>(`${API_PREFIX}/settings/status`);
}

// --- items ---

export interface ItemPageRequest {
  limit?: number;
  cursor?: string;
  archived?: boolean;
}

export interface ItemPage {
  readonly items: ItemDto[];
  readonly nextCursor: string | null;
  readonly etag: string | null;
}

export async function listItems(params: ItemPageRequest = {}): Promise<ItemPage> {
  const query: QueryParams = {
    limit: params.limit,
    cursor: params.cursor,
    archived: params.archived,
  };
  const { body, etag } = await requestJson<ItemList>(`${API_PREFIX}/items`, { query });
  return { items: body.data, nextCursor: body.nextCursor ?? null, etag };
}

export function createItem(body: ItemCreateRequest): Promise<ApiResource<ItemDto>> {
  return requestData<ItemDto>(`${API_PREFIX}/items`, { method: "POST", body });
}

// --- documents / photos ---

/** `POST /items/{id}/documents`：绑定已上传的 PDF 原件（校验类型与归属）。 */
export function createDocument(
  itemId: string,
  body: DocumentCreateRequest,
): Promise<ApiResource<DocumentDto>> {
  return requestData<DocumentDto>(`${API_PREFIX}/items/${encodeURIComponent(itemId)}/documents`, {
    method: "POST",
    body,
  });
}

/** `POST /items/{id}/photos`：登记照片并分配视图（同一视图最多一张，冲突 422 `viewOccupied`）。 */
export function createPhoto(
  itemId: string,
  body: PhotoCreateRequest,
): Promise<ApiResource<PhotoDto>> {
  return requestData<PhotoDto>(`${API_PREFIX}/items/${encodeURIComponent(itemId)}/photos`, {
    method: "POST",
    body,
  });
}

/** `PATCH /items/{id}/photos/{photoId}`：替换资产或改选视图；必须带 `If-Match`。 */
export function patchPhoto(
  itemId: string,
  photoId: string,
  body: PhotoPatchRequest,
  ifMatch: string,
): Promise<ApiResource<PhotoDto>> {
  return requestData<PhotoDto>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/photos/${encodeURIComponent(photoId)}`,
    { method: "PATCH", body, ifMatch },
  );
}

// --- 报价、确认与建单（T11 合同） ---

/** `POST /items/{id}/estimates`：只计算计划（不调用生成服务、不写费用记录）。 */
export function createEstimate(
  itemId: string,
  body: EstimateRequest,
): Promise<ApiResource<QuoteDto>> {
  return requestData<QuoteDto>(`${API_PREFIX}/items/${encodeURIComponent(itemId)}/estimates`, {
    method: "POST",
    body,
  });
}

/** `GET /items/{id}/estimates/{quoteId}`：回读既有报价（确认/消费状态以服务端为准）。 */
export function getEstimate(itemId: string, quoteId: string): Promise<ApiResource<QuoteDto>> {
  return requestData<QuoteDto>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/estimates/${encodeURIComponent(quoteId)}`,
  );
}

/** `POST .../estimates/{quoteId}/confirm`：显式确认"将发送给供应商的资料"（写 audit_events）。 */
export function confirmEstimate(
  itemId: string,
  quoteId: string,
): Promise<ApiResource<ConfirmationDto>> {
  return requestData<ConfirmationDto>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/estimates/${encodeURIComponent(quoteId)}/confirm`,
    { method: "POST" },
  );
}

export interface JobCreation {
  readonly job: JobDto;
  /** 202 = 首次建单；同键重放也返回 202（同一 job），由服务端幂等保证。 */
  readonly status: number;
}

/**
 * `POST /items/{id}/jobs`：冻结快照 + 预留费用 + 创建任务（202）。
 *
 * `Idempotency-Key` 由调用方**一次操作生成并复用**：重复点击／断线重试沿用同一个 key，
 * 服务端按 `admin + method + route + key` 去重，不产生第二份生成单（REQ-022）。
 * 请求体没有费用字段：金额一律由服务端从报价快照回读。
 */
export async function createJob(
  itemId: string,
  body: JobCreateRequest,
  idempotencyKey: string,
): Promise<JobCreation> {
  const { body: payload, status } = await requestJson<{ data: JobDto }>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/jobs`,
    { method: "POST", body, headers: { "Idempotency-Key": idempotencyKey } },
  );
  return { job: payload.data, status };
}

export function getItem(itemId: string): Promise<ApiResource<ItemDto>> {
  return requestData<ItemDto>(`${API_PREFIX}/items/${encodeURIComponent(itemId)}`);
}

/** `PATCH /items/{id}`：必须携带 GET 得到的 `ETag`；缺 428、过期 412。 */
export function patchItem(
  itemId: string,
  body: ItemPatchRequest,
  ifMatch: string,
): Promise<ApiResource<ItemDto>> {
  return requestData<ItemDto>(`${API_PREFIX}/items/${encodeURIComponent(itemId)}`, {
    method: "PATCH",
    body,
    ifMatch,
  });
}

export interface DocumentPage {
  readonly documents: DocumentList["data"];
  readonly nextCursor: string | null;
  readonly etag: string | null;
}

export async function listDocuments(itemId: string): Promise<DocumentPage> {
  const { body, etag } = await requestJson<DocumentList>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/documents`,
  );
  return { documents: body.data, nextCursor: body.nextCursor ?? null, etag };
}

export interface PhotoPage {
  readonly photos: PhotoList["data"];
  readonly nextCursor: string | null;
  readonly etag: string | null;
}

export async function listPhotos(itemId: string): Promise<PhotoPage> {
  const { body, etag } = await requestJson<PhotoList>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/photos`,
  );
  return { photos: body.data, nextCursor: body.nextCursor ?? null, etag };
}

/** `GET /assets/{id}/content` 的同源 URL（浏览器带 cookie 直接取字节）。 */
export function assetContentUrl(assetId: string): string {
  return `${API_PREFIX}/assets/${encodeURIComponent(assetId)}/content`;
}

/**
 * `GET /assets/{id}/content`：取资产字节（GLB / 原 PDF）。
 * 授权、ETag、Range 语义由服务端保证（contracts §7）；这里只取完整内容。
 */
export function fetchAssetContent(
  assetId: string,
  options: { signal?: AbortSignal; onProgress?: (received: number, total: number | null) => void } = {},
): Promise<ByteResponse> {
  return requestBytes(assetContentUrl(assetId), {
    signal: options.signal,
    onProgress: options.onProgress,
  });
}

// --- 草稿读取与复核写入（T15 读取 + T19 受限字段 PATCH / 发布） ---

export type DraftDto = components["schemas"]["DraftDto"];
export type DraftPatchRequest = components["schemas"]["DraftPatchRequest"];
export type HotspotUpsert = components["schemas"]["HotspotUpsert"];
export type CameraPoseDto = components["schemas"]["CameraPose"];
export type EntityReviewPatchDto = components["schemas"]["EntityReviewPatchDto"];
export type UserEditDto = components["schemas"]["UserEdit"];
export type ReviewStatusDto = components["schemas"]["ReviewStatusDto"];

/** `GET /items/{id}/drafts/{draftId}`（带 ETag）：草稿知识、模型版本引用与缺项。 */
export function getDraft(itemId: string, draftId: string): Promise<ApiResource<DraftDto>> {
  return requestData<DraftDto>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/drafts/${encodeURIComponent(draftId)}`,
  );
}

/**
 * `PATCH /items/{id}/drafts/{draftId}`（If-Match）：复核写入（状态、热点、视角、
 * 实体确认/修订、modelReview）。服务端逐字段校验：非法请求 422（`details.fields`）、
 * revision 过期 412（`details.currentRevision`）；前端不绕过这些校验。
 */
export function patchDraft(
  itemId: string,
  draftId: string,
  body: DraftPatchRequest,
  ifMatch: string,
): Promise<ApiResource<DraftDto>> {
  return requestData<DraftDto>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/drafts/${encodeURIComponent(draftId)}`,
    { method: "PATCH", body, ifMatch },
  );
}

// --- 发布与版本（T19） ---

export type ReleaseDto = components["schemas"]["ReleaseDto"];
export type ReleaseDetailDto = components["schemas"]["ReleaseDetailDto"];
export type PublishIssueDto = components["schemas"]["PublishIssue"];

export interface PublishResult {
  readonly release: ReleaseDto;
  /** 幂等重放（同 key 同 body）：返回既有 release，不新建。 */
  readonly replayed: boolean;
}

/**
 * `POST /items/{id}/drafts/{draftId}/publish`（If-Match + Idempotency-Key）。
 *
 * 发布是**显式动作**（不存在自动发布路径）。不变量不满足 → 422
 * （`details.issues[]`，由调用方展示逐条处理入口）；并发/陈旧 revision → 412。
 * `Idempotency-Key` 由调用方一次操作生成并复用（重放返回同一 release）。
 */
export async function publishDraft(
  itemId: string,
  draftId: string,
  ifMatch: string,
  idempotencyKey: string,
): Promise<PublishResult> {
  const { body, headers } = await requestJson<{ data: ReleaseDto }>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/drafts/${encodeURIComponent(draftId)}/publish`,
    { method: "POST", ifMatch, headers: { "Idempotency-Key": idempotencyKey } },
  );
  return {
    release: body.data,
    replayed: headers.get("x-idempotent-replay") === "true",
  };
}

/** `GET /items/{id}/releases`：已发布版本（发布时间倒序，服务端排序）。 */
export async function listReleases(itemId: string): Promise<ReleaseDto[]> {
  const { body } = await requestJson<{ data: ReleaseDto[]; nextCursor?: string | null }>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/releases`,
  );
  return body.data;
}

/** `GET /items/{id}/releases/{releaseId}`：不可变 manifest（阅读端消费）。 */
export function getRelease(itemId: string, releaseId: string): Promise<ApiResource<ReleaseDetailDto>> {
  return requestData<ReleaseDetailDto>(
    `${API_PREFIX}/items/${encodeURIComponent(itemId)}/releases/${encodeURIComponent(releaseId)}`,
  );
}

// --- 任务中心（T15 端点；T17 的界面消费） ---

export type JobSummaryDto = components["schemas"]["JobSummaryDto"];
export type JobStageSummaryDto = components["schemas"]["JobStageSummaryDto"];
export type JobStageDto = components["schemas"]["JobStageDto"];
export type JobStageRetryDto = components["schemas"]["JobStageRetryDto"];
export type JobAttemptDto = components["schemas"]["JobAttemptDto"];
export type JobMissingItemDto = components["schemas"]["JobMissingItemDto"];
export type JobDetailDto = components["schemas"]["JobDetailDto"];
export type ReservationDto = components["schemas"]["ReservationDto"];
export type JobMissingItem = components["schemas"]["JobMissingItemDto"];
export type CancelResultDto = components["schemas"]["CancelResultDto"];
export type RetryResultDto = components["schemas"]["RetryResultDto"];
export type ReconcileRequestDto = JsonRequest<"reconcile_job">;
export type ReconcileActionDto = components["schemas"]["ReconcileActionDto"];
export type ReconcileResultDto = components["schemas"]["ReconcileResultDto"];

export interface JobPage {
  readonly jobs: JobSummaryDto[];
  readonly nextCursor: string | null;
}

/**
 * `GET /jobs`（T15 交付）：游标分页；`itemId` 过滤（游标与过滤条件绑定）。
 * 排序由服务端给定（createdAt DESC, id DESC；U-03）——前端不重排。
 */
export async function listJobs(
  params: { itemId?: string; cursor?: string; limit?: number } = {},
): Promise<JobPage> {
  const query: QueryParams = {
    itemId: params.itemId,
    cursor: params.cursor,
    limit: params.limit,
  };
  const { body } = await requestJson<{ data: JobSummaryDto[]; nextCursor?: string | null }>(
    `${API_PREFIX}/jobs`,
    { query },
  );
  return { jobs: body.data, nextCursor: body.nextCursor ?? null };
}

/** `GET /jobs/{id}`：阶段/尝试/费用/缺项；`ETag` 原样回传给 cancel/retry/reconcile。 */
export function getJob(jobId: string): Promise<ApiResource<JobDetailDto>> {
  return requestData<JobDetailDto>(`${API_PREFIX}/jobs/${encodeURIComponent(jobId)}`);
}

/** `POST /jobs/{id}/cancel`（If-Match）：未提交阶段停止推进；不声称已取消远端付费操作。 */
export async function cancelJob(jobId: string, ifMatch: string): Promise<CancelResultDto> {
  const { body } = await requestJson<{ data: CancelResultDto }>(
    `${API_PREFIX}/jobs/${encodeURIComponent(jobId)}/cancel`,
    { method: "POST", ifMatch },
  );
  return body.data;
}

/**
 * `POST /jobs/{id}/retry`（If-Match + `Idempotency-Key`）：只重跑指定阶段。
 *
 * `Idempotency-Key` 由调用方**一次操作生成并复用**（重复点击/断线重试沿用同一个 key，
 * 服务端不产生第二个 attempt）；可重试与否以服务端判定为准（任务详情的
 * `stages[].retry` 与端点同源，界面据此决定是否渲染入口）。
 */
export async function retryJob(
  jobId: string,
  stageId: string,
  ifMatch: string,
  idempotencyKey: string,
): Promise<RetryResultDto> {
  const { body } = await requestJson<{ data: RetryResultDto }>(
    `${API_PREFIX}/jobs/${encodeURIComponent(jobId)}/retry`,
    {
      method: "POST",
      body: { stageId },
      ifMatch,
      headers: { "Idempotency-Key": idempotencyKey },
    },
  );
  return body.data;
}

/** `POST /jobs/{id}/reconcile`（If-Match）：处理 `submission_unknown`（三种动作）。 */
export async function reconcileJob(
  jobId: string,
  request: ReconcileRequestDto,
  ifMatch: string,
): Promise<ReconcileResultDto> {
  const { body } = await requestJson<{ data: ReconcileResultDto }>(
    `${API_PREFIX}/jobs/${encodeURIComponent(jobId)}/reconcile`,
    { method: "POST", body: request, ifMatch },
  );
  return body.data;
}
