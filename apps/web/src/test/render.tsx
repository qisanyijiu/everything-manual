/**
 * 测试辅助：用真实 Provider 组合（Query + 通知 + 路由）渲染完整路由树，
 * 只替换 `fetch`（网络边界），不替换业务组件——测试覆盖的是真实页面逻辑。
 */

import { render, type RenderResult } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router";

import { AppRoutes, createAppQueryClient } from "../App";
import { NotificationProvider } from "../components/notifications";

export interface RenderAppResult extends RenderResult {
  readonly queryClient: QueryClient;
}

export interface RenderAppOptions {
  readonly route?: string;
  readonly queryClient?: QueryClient;
}

export function renderApp(options: RenderAppOptions = {}): RenderAppResult {
  const queryClient = options.queryClient ?? createAppQueryClient();
  const result = render(
    <QueryClientProvider client={queryClient}>
      <NotificationProvider>
        <MemoryRouter initialEntries={[options.route ?? "/"]}>
          <AppRoutes />
        </MemoryRouter>
      </NotificationProvider>
    </QueryClientProvider>,
  );
  return { ...result, queryClient };
}

/** 设置视口宽度（断点判定用；jsdom 无 matchMedia 时退化为 innerWidth）。 */
export function setViewportWidth(width: number): void {
  window.innerWidth = width;
  window.dispatchEvent(new Event("resize"));
}

/** 构造 JSON 响应（可带 ETag / x-request-id 头）。 */
export function jsonResponse(
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

/** 合同错误响应体（contracts.md §1）。 */
export function errorResponse(
  status: number,
  code: string,
  message: string,
  details: unknown = null,
  requestId = "01993000-0000-7000-8000-0000000000aa",
): Response {
  return jsonResponse(
    { error: { code, message, details, requestId } },
    { status, requestId },
  );
}
