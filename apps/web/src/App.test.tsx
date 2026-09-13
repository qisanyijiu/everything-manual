/**
 * 应用级冒烟（真实 BrowserRouter + 真实 Provider）：
 * - 未登录直达受保护路由 → 跳转 `/login?next=<站内相对路径>`（浏览器地址栏真的变化）；
 * - 已登录时资料库渲染真实的 `{ data, nextCursor }` 列表行。
 *
 * T01 的最小 health 页面已被 T08 的完整路由表取代；本文件保留为应用装配层的冒烟测试。
 */

import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";

const SESSION = {
  data: {
    admin: { id: "01993000-0000-7000-8000-000000000001" },
    csrfToken: "csrf-token-abc",
    expiresAt: "2026-09-19T00:00:00Z",
  },
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
  window.history.pushState({}, "", "/");
});

describe("<App /> 装配与路由", () => {
  it("未登录直达 /settings 时跳转登录页，并在 URL 保留 next", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        jsonResponse(
          { error: { code: "UNAUTHORIZED", message: "需要登录", details: null, requestId: "r1" } },
          401,
        ),
      ),
    );
    window.history.pushState({}, "", "/settings");

    render(<App />);

    expect(await screen.findByLabelText("密码")).toBeTruthy();
    expect(window.location.pathname).toBe("/login");
    expect(window.location.search).toBe("?next=%2Fsettings");
  });

  it("已登录时资料库渲染服务端返回的行", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url === "/api/v1/auth/session") {
          return jsonResponse(SESSION);
        }
        if (url.startsWith("/api/v1/items")) {
          return jsonResponse({
            data: [
              {
                id: "item-1",
                name: "相机 A",
                model: "X100V",
                brand: "Fujifilm",
                variant: null,
                revision: 2,
                createdAt: "2026-09-11T00:00:00Z",
                updatedAt: "2026-09-11T09:30:00Z",
                archivedAt: null,
              },
            ],
            nextCursor: null,
          });
        }
        throw new Error(`未预期的请求：${url}`);
      }),
    );
    window.history.pushState({}, "", "/");

    render(<App />);

    expect(await screen.findByRole("link", { name: "相机 A" })).toBeTruthy();
    expect(screen.getByText("X100V · Fujifilm")).toBeTruthy();
    expect(screen.getByText("使用中")).toBeTruthy();
    expect(screen.getByRole("navigation", { name: "主导航" })).toBeTruthy();
  });
});
