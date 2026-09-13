/**
 * e2e 共享辅助：真实 API 造数据（APIRequestContext）+ 通过真实登录页登录。
 *
 * 造数据走公开 HTTP 合同（不是直接改数据库）：e2e 因此同时覆盖了 T06/T07 的
 * 上传与绑定路由；浏览器侧只做 T09 的准备流程本身。
 */

import fs from "node:fs";
import path from "node:path";

import { expect, type APIRequestContext, type Page } from "@playwright/test";

import { REPO_ROOT, fixturePath, readRuntime, type E2eRuntime } from "./runtime";

export interface SeededDocument {
  readonly itemId: string;
  readonly documentId: string;
  readonly sourceAssetId: string;
  readonly sourceSha256: string;
}

export function runtime(): E2eRuntime {
  return readRuntime();
}

/** 登录后端并返回 CSRF token（cookie 由 APIRequestContext 自己保存）。 */
export async function apiLogin(
  request: APIRequestContext,
  base: string,
  password: string,
): Promise<string> {
  const response = await request.post(`${base}/api/v1/auth/login`, { data: { password } });
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: { csrfToken: string } };
  return body.data.csrfToken;
}

async function apiFetch<T>(
  request: APIRequestContext,
  options: {
    method: "POST" | "GET" | "PATCH" | "PUT";
    url: string;
    csrf?: string;
    data?: unknown;
    /** `If-Match`（来自上一 GET 的 ETag，原样回传）。 */
    ifMatch?: string | null;
    multipart?: Record<string, unknown>;
  },
): Promise<T> {
  const headers: Record<string, string> = {};
  if (options.csrf !== undefined) {
    headers["x-csrf-token"] = options.csrf;
  }
  if (options.ifMatch !== undefined && options.ifMatch !== null) {
    headers["if-match"] = options.ifMatch;
  }
  const response = await request.fetch(options.url, {
    method: options.method,
    headers,
    data: options.data,
    multipart: options.multipart as never,
  });
  const text = await response.text();
  expect(response.ok(), `${options.method} ${options.url} → ${response.status()} ${text}`).toBe(true);
  return text === "" ? (undefined as T) : (JSON.parse(text) as T);
}

/** 新建物品 + 上传 PDF 原件 + 绑定 document（等价于 T16 向导前两步的 API 形态）。 */
export async function seedItemWithDocument(
  request: APIRequestContext,
  base: string,
  password: string,
  fixtureName: string,
  name: string,
): Promise<SeededDocument> {
  const csrf = await apiLogin(request, base, password);
  const item = await apiFetch<{ data: { id: string } }>(request, {
    method: "POST",
    url: `${base}/api/v1/items`,
    csrf,
    data: { name, model: `T09-${fixtureName}`, brand: "Fixture" },
  });
  const itemId = item.data.id;

  const asset = await apiFetch<{ data: { id: string; sha256: string } }>(request, {
    method: "POST",
    url: `${base}/api/v1/items/${itemId}/assets`,
    csrf,
    multipart: {
      purpose: "document",
      file: {
        name: fixtureName,
        mimeType: "application/pdf",
        buffer: fs.readFileSync(fixturePath(fixtureName)),
      },
    },
  });

  const document = await apiFetch<{ data: { id: string; sourceSha256: string } }>(request, {
    method: "POST",
    url: `${base}/api/v1/items/${itemId}/documents`,
    csrf,
    data: { sourceAssetId: asset.data.id, title: `${name} 说明书` },
  });

  return {
    itemId,
    documentId: document.data.id,
    sourceAssetId: asset.data.id,
    sourceSha256: document.data.sourceSha256,
  };
}

/** 通过真实登录页登录（不使用注入 cookie 的捷径）。 */
export async function loginViaUi(page: Page, base: string, password: string): Promise<void> {
  await page.goto(`${base}/login`);
  await page.getByLabel("密码").fill(password);
  await page.getByRole("button", { name: "登录" }).click();
  await expect(
    page.getByRole("heading", { name: "资料库", exact: true }),
  ).toBeVisible();
}

export interface PreparationDetail {
  readonly id: string;
  readonly state: string;
  readonly pageCount: number | null;
  readonly clientDerived: boolean;
  readonly revision: number;
  readonly missingPages: number[];
  readonly pages: {
    pageNumber: number;
    textAssetId: string | null;
    imageAssetId: string | null;
    viewport: { width: number; height: number; rotation: number } | null;
  }[];
}

/** `GET /preparations/{id}`（服务端事实来源）。 */
export async function fetchPreparation(
  request: APIRequestContext,
  base: string,
  preparationId: string,
): Promise<PreparationDetail> {
  const response = await request.get(`${base}/api/v1/preparations/${preparationId}`);
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: PreparationDetail };
  return body.data;
}

/** 读取资产字节（授权会话在 APIRequestContext 中）。 */
export async function fetchAsset(
  request: APIRequestContext,
  base: string,
  assetId: string,
): Promise<{ contentType: string; bytes: Buffer }> {
  const response = await request.get(`${base}/api/v1/assets/${assetId}/content`);
  expect(response.status(), await response.text()).toBe(200);
  return {
    contentType: response.headers()["content-type"] ?? "",
    bytes: Buffer.from(await response.body()),
  };
}

/** 保存一张人工可复核的截图（稳定文件名，落在 artifacts/web-mvp/t09-rd/screenshots）。 */
export async function capture(page: Page, name: string): Promise<void> {
  await captureTo("t09-rd", page, name);
}

/** 保存截图到 `artifacts/web-mvp/<subdir>/screenshots/`（T16 等人工作品的证据目录）。 */
export async function captureTo(subdir: string, page: Page, name: string): Promise<void> {
  const dir = path.join(REPO_ROOT, "artifacts", "web-mvp", subdir, "screenshots");
  fs.mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: path.join(dir, `${name}.png`), fullPage: true });
}

// ---------------------------------------------------------------------------
// T16 造数（全部走公开 HTTP 合同）：物品 / document / 照片 / ready preparation
// ---------------------------------------------------------------------------

/** 只建物品（向导从 `/items/new` 开始时的前置状态）。 */
export async function seedItem(
  request: APIRequestContext,
  base: string,
  password: string,
  name: string,
): Promise<string> {
  const csrf = await apiLogin(request, base, password);
  const item = await apiFetch<{ data: { id: string } }>(request, {
    method: "POST",
    url: `${base}/api/v1/items`,
    csrf,
    data: { name, model: `T16-${name}` },
  });
  return item.data.id;
}

const MIME_BY_FIXTURE: Record<string, string> = {
  "sample-photo-front.jpg": "image/jpeg",
  "sample-photo-left.png": "image/png",
  "sample-manual-text.pdf": "application/pdf",
};

/** 上传一份 fixture 资产（`purpose` 决定用途）。 */
export async function uploadFixture(
  request: APIRequestContext,
  base: string,
  csrf: string,
  itemId: string,
  purpose: "document" | "photo" | "pageText" | "pageImage",
  fixtureName: string,
): Promise<{ id: string; sha256: string }> {
  const asset = await apiFetch<{ data: { id: string; sha256: string } }>(request, {
    method: "POST",
    url: `${base}/api/v1/items/${itemId}/assets`,
    csrf,
    multipart: {
      purpose,
      file: {
        name: fixtureName,
        mimeType: MIME_BY_FIXTURE[fixtureName] ?? "application/octet-stream",
        buffer: fs.readFileSync(fixturePath(fixtureName)),
      },
    },
  });
  return asset.data;
}

/** 给物品登记一张已上传 fixtures 的视图照片（front/left/…）。 */
export async function seedPhoto(
  request: APIRequestContext,
  base: string,
  csrf: string,
  itemId: string,
  view: string,
  fixtureName: string,
): Promise<{ photoId: string; assetId: string }> {
  const asset = await uploadFixture(request, base, csrf, itemId, "photo", fixtureName);
  const photo = await apiFetch<{ data: { id: string; assetId: string } }>(request, {
    method: "POST",
    url: `${base}/api/v1/items/${itemId}/photos`,
    csrf,
    data: { assetId: asset.id, view },
  });
  return { photoId: photo.data.id, assetId: photo.data.assetId };
}

/**
 * 用 HTTP 合同造一份 **ready** 的 preparation（第 5 步所需的权威状态）。
 *
 * 页图用 fixtures 的 JPEG（32×32，长边 ≤2000）、页文字用一段自撰文本；
 * 页号 1-based、`complete` 需 `If-Match` + `pageCount`（T09 合同）。
 * 返回 preparationId；调用方负责把会话指针写进浏览器 sessionStorage
 * （向导第 5 步按同一指针查询服务端状态）。
 */
export async function seedReadyPreparation(
  request: APIRequestContext,
  base: string,
  csrf: string,
  seed: { itemId: string; documentId: string; sourceSha256: string },
): Promise<string> {
  const preparation = await apiFetch<{ data: { id: string } }>(request, {
    method: "POST",
    url: `${base}/api/v1/documents/${seed.documentId}/preparations`,
    csrf,
    data: { sourceSha256: seed.sourceSha256 },
  });
  const preparationId = preparation.data.id;

  const image = await uploadFixture(request, base, csrf, seed.itemId, "pageImage", "sample-photo-front.jpg");
  const text = await apiFetch<{ data: { id: string } }>(request, {
    method: "POST",
    url: `${base}/api/v1/items/${seed.itemId}/assets`,
    csrf,
    multipart: {
      purpose: "pageText",
      file: {
        name: "page-0001.txt",
        mimeType: "text/plain",
        buffer: Buffer.from("第 1 页：这是 e2e 造数用的页文字。", "utf8"),
      },
    },
  });

  await apiFetch(request, {
    method: "PUT",
    url: `${base}/api/v1/preparations/${preparationId}/pages/1`,
    csrf,
    data: {
      textAssetId: text.data.id,
      imageAssetId: image.id,
      viewport: { width: 32, height: 32, rotation: 0 },
    },
  });

  // ETag 必须在写入页之后重新读取：PUT 会自增 revision（旧 ETag 会让 complete 412）。
  const detailResponse = await request.get(`${base}/api/v1/preparations/${preparationId}`);
  expect(detailResponse.status(), await detailResponse.text()).toBe(200);
  const etag = detailResponse.headers()["etag"];
  expect(etag, "GET /preparations 必须返回 ETag").toBeTruthy();
  await apiFetch(request, {
    method: "POST",
    url: `${base}/api/v1/preparations/${preparationId}/complete`,
    csrf,
    ifMatch: etag,
    data: { pageCount: 1 },
  });
  return preparationId;
}

/** `GET /items/{id}/photos`（服务端事实）。 */
export async function fetchPhotos(
  request: APIRequestContext,
  base: string,
  itemId: string,
): Promise<{ id: string; view: string; assetId: string; revision: number }[]> {
  const response = await request.get(`${base}/api/v1/items/${itemId}/photos`);
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as {
    data: { id: string; view: string; assetId: string; revision: number }[];
  };
  return body.data;
}

/** `GET /items/{id}/documents`（服务端事实）。 */
export async function fetchDocuments(
  request: APIRequestContext,
  base: string,
  itemId: string,
): Promise<{ id: string; title: string; sourceSha256: string }[]> {
  const response = await request.get(`${base}/api/v1/items/${itemId}/documents`);
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as {
    data: { id: string; title: string; sourceSha256: string }[];
  };
  return body.data;
}

/** `GET /jobs?itemId=…`（建单幂等断言的服务器事实）。 */
export async function fetchJobsForItem(
  request: APIRequestContext,
  base: string,
  itemId: string,
): Promise<{ id: string; status: string }[]> {
  const response = await request.get(`${base}/api/v1/jobs?itemId=${encodeURIComponent(itemId)}`);
  expect(response.status(), await response.text()).toBe(200);
  const body = (await response.json()) as { data: { id: string; status: string }[] };
  return body.data;
}

/** 把准备记录的**会话指针**写进浏览器 sessionStorage（第 5 步读取它查询服务端状态）。 */
export async function setPreparationPointer(
  page: Page,
  itemId: string,
  preparationId: string,
): Promise<void> {
  const key = `em.prepare.${itemId}`;
  await page.evaluate(
    (args) => window.sessionStorage.setItem(args.key, args.value),
    { key, value: preparationId },
  );
}

/** JPEG 魔数（页图必须是白底 JPEG，而不是 PNG/未编码位图）。 */
export function isJpeg(bytes: Buffer): boolean {
  return bytes.length > 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff;
}
