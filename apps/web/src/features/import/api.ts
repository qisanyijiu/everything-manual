/**
 * 导入（PDF 准备）相关的 API 封装（T09 / REQ-014、REQ-015）。
 *
 * 类型全部来自 `cargo xtask contracts` 生成的机器合同（禁止手抄 DTO，ADR-009）。
 *
 * 为什么要在这里做 multipart 上传而不是复用 `src/api/client.ts`：
 * 该封装只支持 JSON 请求体；页文字/页图资产必须走 `POST /items/{id}/assets` 的
 * multipart（T06 合同）。multipart 实现（含 CSRF 注入与 403 `CSRF_REJECTED`
 * 刷新重试、真实字节进度）统一在 `./upload.ts`，避免多份实现漂移。
 */

import { requestData, requestJson } from "../../api/client";
import { API_PREFIX } from "../../api/endpoints";
import type { components, operations } from "../../api/generated";
import { uploadAsset, type UploadOptions } from "./upload";

// --- 合同类型别名（全部派生自 generated.ts） ---

type JsonRequest<Op extends keyof operations> = operations[Op] extends {
  requestBody: { content: { "application/json": infer C } };
}
  ? C
  : never;

type JsonBody<R> = R extends { content: { "application/json": infer C } } ? C : never;

export type PreparationDto = components["schemas"]["PreparationDto"];
export type PreparationDetailDto = components["schemas"]["PreparationDetailDto"];
export type PreparationPageDto = components["schemas"]["PageDto"];
export type PreparationViewport = components["schemas"]["ViewportDto"];
export type PreparationCreateRequest = JsonRequest<"create_preparation">;
export type PagePutRequest = JsonRequest<"put_page">;
export type PreparationCompleteRequest = JsonRequest<"complete_preparation">;
export type DocumentDto = components["schemas"]["DocumentDto"];
export type AssetDto = components["schemas"]["AssetDto"];
export type PreparationCreateResponse = JsonBody<
  operations["create_preparation"]["responses"][200]
>;
export type PageResponseBody = JsonBody<operations["put_page"]["responses"][200]>;

/** 资产上传用途（页文字/页图；上传路由的取值集合，见 T06）。 */
export type PageAssetPurpose = "pageText" | "pageImage";

// --- 准备记录 ---

export interface PreparationResource {
  readonly preparation: PreparationDto;
  /** 201 = 新建，200 = 复用未完成记录（断线续传）。 */
  readonly created: boolean;
}

/** `POST /documents/{id}/preparations`：创建或复用未完成记录。 */
export async function createOrResumePreparation(
  documentId: string,
  sourceSha256: string,
): Promise<PreparationResource> {
  const body: PreparationCreateRequest = { sourceSha256 };
  const { body: payload, status } = await requestJson<PreparationCreateResponse>(
    `${API_PREFIX}/documents/${encodeURIComponent(documentId)}/preparations`,
    { method: "POST", body },
  );
  return { preparation: payload.data, created: status === 201 };
}

export interface PreparationDetail {
  readonly detail: PreparationDetailDto;
  /** 原样回传的 `ETag`（封存与页覆盖的 If-Match 值）。 */
  readonly etag: string | null;
}

/** `GET /preparations/{id}`：状态 + 已上传页 + 缺页（续传的事实来源）。 */
export async function getPreparation(preparationId: string): Promise<PreparationDetail> {
  const { data, etag } = await requestData<PreparationDetailDto>(
    `${API_PREFIX}/preparations/${encodeURIComponent(preparationId)}`,
  );
  return { detail: data, etag };
}

/**
 * `PUT /preparations/{id}/pages/{n}`：上传单页（1-based 页号）。
 *
 * `ifMatch` 只在覆盖已有页（内容变化）时需要；新页与同内容幂等提交不带该头。
 * 走 `client.ts` 的 JSON 封装：自动注入 CSRF、401 触发全局会话失效处理。
 */
export async function putPreparationPage(
  preparationId: string,
  pageNumber: number,
  body: PagePutRequest,
  ifMatch: string | null = null,
): Promise<PreparationPageDto> {
  const payload = await requestJson<PageResponseBody>(
    `${API_PREFIX}/preparations/${encodeURIComponent(preparationId)}/pages/${pageNumber}`,
    { method: "PUT", body, ifMatch },
  );
  return payload.body.data;
}

/** `POST /preparations/{id}/complete`：封存为 ready（If-Match + 声明页数）。 */
export async function completePreparation(
  preparationId: string,
  pageCount: number,
  ifMatch: string | null,
): Promise<PreparationDto> {
  const body: PreparationCompleteRequest = { pageCount };
  const { data } = await requestData<PreparationDto>(
    `${API_PREFIX}/preparations/${encodeURIComponent(preparationId)}/complete`,
    { method: "POST", body, ifMatch },
  );
  return data;
}

// --- 资产上传（multipart）与原件读取 ---

export type UploadErrorOptions = UploadOptions;

/**
 * `POST /items/{id}/assets`（multipart：`purpose` + `file`）。
 *
 * T16 起统一委托给 `./upload.ts`（XHR + 真实字节进度 + 结构化 `AssetUploadError`），
 * 避免两份 multipart 实现漂移：CSRF 注入、403 `CSRF_REJECTED` 刷新重试与
 * 错误解析只有一处实现。准备流水线（`pdf/prepare.ts`）只依赖"成功返回 AssetDto、
 * 失败抛出带 message 的错误"，签名保持兼容。
 */
export function uploadPageAsset(
  itemId: string,
  purpose: PageAssetPurpose,
  content: Blob,
  filename: string,
  options: UploadErrorOptions = {},
): Promise<AssetDto> {
  return uploadAsset(itemId, purpose, content, filename, options);
}

/** 读取原件内容（PDF 字节）；每次调用都重新读取，不复用可能已被 worker 转移的缓冲。 */
export async function fetchAssetBytes(
  assetId: string,
  signal?: AbortSignal,
): Promise<Uint8Array> {
  const response = await fetch(`${API_PREFIX}/assets/${encodeURIComponent(assetId)}/content`, {
    credentials: "same-origin",
    signal: signal ?? null,
  });
  if (!response.ok) {
    throw new Error(
      response.status === 401
        ? "登录已过期：请重新登录后再开始准备"
        : `读取原 PDF 失败（HTTP ${response.status}）`,
    );
  }
  return new Uint8Array(await response.arrayBuffer());
}
