/**
 * HTTP 封装测试（T08）：合同错误解析、CSRF/If-Match 注入、401 广播、
 * `{ data }` 解包与 ETag 透传、`/health/ready` 的 503 容忍（ADR-013 第 9 条）。
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError, describeError, onUnauthorized, setCsrfToken } from "./client";
import { fetchHealthLive, fetchReadiness, listItems, patchItem } from "./endpoints";

function jsonResponse(
  body: unknown,
  options: { status?: number; etag?: string; requestId?: string } = {},
): Response {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (options.etag !== undefined) {
    headers.etag = options.etag;
  }
  if (options.requestId !== undefined) {
    headers["x-request-id"] = options.requestId;
  }
  return new Response(JSON.stringify(body), { status: options.status ?? 200, headers });
}

function stub(handler: (url: string, init: RequestInit) => Response) {
  const mock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) =>
    handler(String(input), init ?? {}),
  );
  vi.stubGlobal("fetch", mock);
  return mock;
}

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
});

describe("请求封装（类型来自生成合同）", () => {
  it("GET health/live 解包 data 并带上 JSON Accept", async () => {
    const fetchMock = stub(() => jsonResponse({ data: { status: "ok" } }));

    await expect(fetchHealthLive()).resolves.toEqual({ data: { status: "ok" }, etag: null });

    const call = fetchMock.mock.calls[0];
    expect(call?.[0]).toBe("/api/v1/health/live");
    expect((call?.[1] as RequestInit).headers).toMatchObject({ accept: "application/json" });
  });

  it("CSRF 只注入修改请求，并携带 If-Match（原样回传 ETag）", async () => {
    const fetchMock = stub((_url, init) =>
      init.method === "PATCH"
        ? jsonResponse({ data: { id: "item-1" } }, { etag: '"r8"' })
        : jsonResponse({ data: [], nextCursor: null }),
    );
    setCsrfToken("csrf-token-abc");

    await listItems({});
    await patchItem("item-1", { name: "新名称" }, '"r7"');

    const listInit = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect((listInit.headers as Record<string, string>)["x-csrf-token"]).toBeUndefined();

    const patchInit = fetchMock.mock.calls[1]?.[1] as RequestInit;
    expect((patchInit.headers as Record<string, string>)["x-csrf-token"]).toBe("csrf-token-abc");
    expect((patchInit.headers as Record<string, string>)["if-match"]).toBe('"r7"');
    expect(JSON.parse(String(patchInit.body))).toEqual({ name: "新名称" });
    expect(patchInit.credentials).toBe("same-origin");
  });

  it("没有 CSRF token 时不伪造该头（登录前）", async () => {
    const fetchMock = stub(() => jsonResponse({ data: { id: "item-1" } }));

    await patchItem("item-1", { name: "x" }, '"r1"');

    const init = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect((init.headers as Record<string, string>)["x-csrf-token"]).toBeUndefined();
  });

  it("非 2xx 抛出带 code/message/details/requestId 的 ApiError", async () => {
    stub(() =>
      jsonResponse(
        {
          error: {
            code: "REVISION_CONFLICT",
            message: "该内容已被其他操作更新",
            details: { currentRevision: 7 },
            requestId: "01993000-0000-7000-8000-000000000001",
          },
        },
        { status: 412, requestId: "01993000-0000-7000-8000-000000000001" },
      ),
    );

    const error: unknown = await patchItem("item-1", {}, '"r3"').catch((reason: unknown) => reason);
    expect(error).toBeInstanceOf(ApiError);
    const apiError = error as ApiError;
    expect(apiError.status).toBe(412);
    expect(apiError.code).toBe("REVISION_CONFLICT");
    expect(apiError.message).toBe("该内容已被其他操作更新");
    expect(apiError.details).toEqual({ currentRevision: 7 });
    expect(apiError.requestId).toBe("01993000-0000-7000-8000-000000000001");
  });

  it("错误体不符合合同时仍抛 ApiError 且 code 为 null", async () => {
    stub(() => new Response("<html>gateway error</html>", { status: 502 }));

    const error: unknown = await fetchHealthLive().catch((reason: unknown) => reason);
    expect(error).toBeInstanceOf(ApiError);
    const apiError = error as ApiError;
    expect(apiError.status).toBe(502);
    expect(apiError.code).toBeNull();
    expect(apiError.message).toContain("502");
  });

  it("401 广播会话失效；会话探测自身不广播", async () => {
    const unauthorized = vi.fn();
    const unsubscribe = onUnauthorized(unauthorized);
    stub(() =>
      jsonResponse({ error: { code: "UNAUTHORIZED", message: "需要登录", details: null, requestId: "r" } }, { status: 401 }),
    );

    await listItems({}).catch(() => undefined);
    expect(unauthorized).toHaveBeenCalledTimes(1);

    unsubscribe();
  });

  it("health/ready 的 503 按 { data } 解析（不是错误信封）", async () => {
    stub(() =>
      jsonResponse(
        {
          data: {
            status: "not_ready",
            checks: [{ name: "database", status: "fail" }],
          },
        },
        { status: 503 },
      ),
    );

    await expect(fetchReadiness()).resolves.toEqual({
      status: "not_ready",
      checks: [{ name: "database", status: "fail" }],
    });
  });

  it("网络失败与业务失败在文案上可区分", () => {
    expect(describeError(new TypeError("Failed to fetch")).message).toContain("无法连接服务");
    expect(describeError(new ApiError(422, "VALIDATION_FAILED", "字段校验失败", "r1")).message).toBe(
      "字段校验失败",
    );
  });

  it("retry 策略不自动重放（QueryClient 默认值）", async () => {
    const { createAppQueryClient } = await import("../App");
    const client = createAppQueryClient();
    const defaults = client.getDefaultOptions();
    expect(defaults.queries?.retry).toBe(false);
    expect(defaults.mutations?.retry).toBe(false);
    client.clear();
  });
});
