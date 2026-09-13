/**
 * T08「React 应用框架与基础交互」测试（真实组件 + 真实路由 + 真实 Query，只替换 fetch）。
 *
 * 覆盖（PRD 修订 2 / ui_revision 2）：
 * - UI-001：登录表单（`type=password`、`autocomplete=current-password`、空密码不可提交、
 *   401/429/403 文案、错误摘要 `role="alert"` 并聚焦）；
 * - UI-002：启动 `GET /auth/session` 恢复会话（恢复中是全屏骨架，不先闪登录页）、
 *   任意 401 → 丢弃本地状态跳 `/login?next=<站内相对路径>`、`next` 只接受同源相对路径；
 * - UI-005：资料库空态与列表错误态；
 * - UI-008：412 显示 `currentRevision` 与「刷新后重试」，不自动覆盖、不丢表单内容；
 * - T08 卡：fetch 封装注入 `X-CSRF-Token` 与 `If-Match`（ETag 原样回传）、token 不进 DOM；
 * - §6.1.1：每路由错误边界显示可读文案 + requestId + 返回资料库，不显示堆栈；
 * - §6.1.1/§6.1.5：三档断点布局、抽屉焦点陷阱、Esc 归还焦点；
 * - 路由骨架：未实现路由显示明确的「尚未实现」，不请求业务数据。
 */

import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Drawer } from "./Drawer";
import { safeNextPath } from "../../lib/next-path";
import { errorResponse, jsonResponse, renderApp, setViewportWidth } from "../../test/render";

const SESSION = {
  data: {
    admin: { id: "01993000-0000-7000-8000-000000000001" },
    csrfToken: "csrf-token-abc",
    expiresAt: "2026-09-19T00:00:00Z",
  },
};

/** 诊断请求 ID：所有桩响应共用，避免并行请求的到达顺序影响断言。 */
const REQUEST_ID = "01993000-0000-7000-8000-0000000000ff";

const ITEM = {
  id: "item-1",
  name: "相机 A",
  model: "X100V",
  brand: "Fujifilm",
  variant: null,
  revision: 3,
  createdAt: "2026-09-11T00:00:00Z",
  updatedAt: "2026-09-11T01:00:00Z",
  archivedAt: null,
};

const FORBIDDEN_WORDING = [
  "已自动校准",
  "自动发布",
  "总进度 100%",
  "已证明页图来自原 PDF",
  "供应商账户硬封顶",
  "零费用",
  "重试不会重复收费",
  "离线可用",
];

type FetchHandler = (url: string, init: RequestInit) => Response | undefined;

function stubFetch(handler: FetchHandler) {
  const mock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const response = handler(url, init ?? {});
    if (response === undefined) {
      throw new Error(`未预期的请求：${url}`);
    }
    return response;
  });
  vi.stubGlobal("fetch", mock);
  return mock;
}

const method = (init: RequestInit): string => init.method ?? "GET";
const headersOf = (init: RequestInit): Record<string, string> =>
  (init.headers ?? {}) as Record<string, string>;

/** 默认桩：会话恢复成功 + 资料库空列表。 */
function stubSignedInItems(items: unknown[] = []): ReturnType<typeof stubFetch> {
  return stubFetch((url) => {
    if (url === "/api/v1/auth/session") {
      return jsonResponse(SESSION, { requestId: "req-session" });
    }
    if (url.startsWith("/api/v1/items?")) {
      return jsonResponse({ data: items, nextCursor: null });
    }
    if (url === "/api/v1/items") {
      return jsonResponse({ data: items, nextCursor: null });
    }
    return undefined;
  });
}

beforeEach(() => {
  setViewportWidth(1024);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("会话恢复（UI-002）", () => {
  it("恢复中显示全屏骨架而不是登录页，成功后就地显示资料库空态", async () => {
    stubSignedInItems();

    renderApp({ route: "/" });

    // 恢复中：骨架存在，登录表单不存在（不先闪登录页）。
    expect(screen.getByRole("status").textContent).toContain("正在恢复会话");
    expect(screen.queryByLabelText("密码")).toBeNull();

    expect(await screen.findByRole("heading", { name: "资料库" })).toBeTruthy();
    expect(await screen.findByText("还没有物品")).toBeTruthy();
    expect(screen.getAllByRole("link", { name: "新建物品" }).length).toBeGreaterThan(0);
  });

  it("资料库加载失败保留可读错误与重试入口", async () => {
    stubFetch((url) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION);
      }
      if (url.startsWith("/api/v1/items")) {
        return errorResponse(500, "INTERNAL", "服务器内部错误");
      }
      return undefined;
    });

    renderApp({ route: "/" });

    expect(await screen.findByText("加载失败")).toBeTruthy();
    expect(screen.getByText("服务器内部错误")).toBeTruthy();
    expect(screen.getByRole("button", { name: "重试" })).toBeTruthy();
  });
});

describe("登录页（UI-001）", () => {
  it("密码为空时不可提交；密码框具备 password/current-password 属性", async () => {
    stubFetch((url) => {
      if (url === "/api/v1/auth/session") {
        return errorResponse(401, "UNAUTHORIZED", "需要登录");
      }
      return undefined;
    });

    renderApp({ route: "/login" });

    const password = (await screen.findByLabelText("密码")) as HTMLInputElement;
    expect(password.type).toBe("password");
    expect(password.getAttribute("autocomplete")).toBe("current-password");
    expect((screen.getByRole("button", { name: "登录" }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(password, { target: { value: "secret" } });
    expect((screen.getByRole("button", { name: "登录" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("401 时显示「密码不正确」，聚焦错误摘要并与字段关联", async () => {
    stubFetch((url, init) => {
      if (url === "/api/v1/auth/session") {
        return errorResponse(401, "UNAUTHORIZED", "需要登录");
      }
      if (url === "/api/v1/auth/login" && method(init) === "POST") {
        return errorResponse(401, "INVALID_CREDENTIALS", "密码不正确");
      }
      return undefined;
    });

    renderApp({ route: "/login" });

    const password = (await screen.findByLabelText("密码")) as HTMLInputElement;
    fireEvent.change(password, { target: { value: "wrong-password" } });
    fireEvent.click(screen.getByRole("button", { name: "登录" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("密码不正确");
    expect(document.activeElement).toBe(alert);
    // 字段级关联：输入通过 aria-describedby 指向同一条错误文案。
    const describedBy = password.getAttribute("aria-describedby");
    expect(describedBy).toBe("field-password-error");
    expect(document.getElementById("field-password-error")?.textContent).toContain("密码不正确");
  });

  it("429 与 403 使用限速/刷新文案", async () => {
    stubFetch((url, init) => {
      if (url === "/api/v1/auth/session") {
        return errorResponse(401, "UNAUTHORIZED", "需要登录");
      }
      if (url === "/api/v1/auth/login" && method(init) === "POST") {
        return errorResponse(429, "RATE_LIMITED", "尝试过于频繁");
      }
      return undefined;
    });

    const first = renderApp({ route: "/login" });
    const password = (await screen.findByLabelText("密码")) as HTMLInputElement;
    fireEvent.change(password, { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: "登录" }));
    expect((await screen.findByRole("alert")).textContent).toContain("尝试过于频繁，请稍后再试");
    first.unmount();

    stubFetch((url, init) => {
      if (url === "/api/v1/auth/session") {
        return errorResponse(401, "UNAUTHORIZED", "需要登录");
      }
      if (url === "/api/v1/auth/login" && method(init) === "POST") {
        return errorResponse(403, "CSRF_REJECTED", "CSRF 校验失败");
      }
      return undefined;
    });

    renderApp({ route: "/login" });
    const secondPassword = (await screen.findByLabelText("密码")) as HTMLInputElement;
    fireEvent.change(secondPassword, { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: "登录" }));
    expect((await screen.findByRole("alert")).textContent).toContain("请刷新页面后重试");
  });

  it("登录成功后进入资料库，顶栏出现登出", async () => {
    stubFetch((url, init) => {
      if (url === "/api/v1/auth/session") {
        return errorResponse(401, "UNAUTHORIZED", "需要登录");
      }
      if (url === "/api/v1/auth/login" && method(init) === "POST") {
        return jsonResponse(SESSION);
      }
      if (url.startsWith("/api/v1/items")) {
        return jsonResponse({ data: [], nextCursor: null });
      }
      return undefined;
    });

    renderApp({ route: "/login" });

    const password = (await screen.findByLabelText("密码")) as HTMLInputElement;
    fireEvent.change(password, { target: { value: "correct-password" } });
    fireEvent.click(screen.getByRole("button", { name: "登录" }));

    expect(await screen.findByRole("heading", { name: "资料库" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "登出" })).toBeTruthy();
  });

  it("登出调用 /auth/logout 并回到登录页", async () => {
    const fetchMock = stubFetch((url, init) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION);
      }
      if (url.startsWith("/api/v1/items")) {
        return jsonResponse({ data: [], nextCursor: null });
      }
      if (url === "/api/v1/auth/logout" && method(init) === "POST") {
        return new Response(null, { status: 204 });
      }
      return undefined;
    });

    renderApp({ route: "/" });
    await screen.findByRole("heading", { name: "资料库" });

    fireEvent.click(screen.getByRole("button", { name: "登出" }));

    expect(await screen.findByLabelText("密码")).toBeTruthy();
    const logoutCall = fetchMock.mock.calls.find(
      ([url, init]) => String(url) === "/api/v1/auth/logout" && method(init ?? {}) === "POST",
    );
    expect(logoutCall).toBeTruthy();
    expect(headersOf(logoutCall?.[1] ?? {})["x-csrf-token"]).toBe("csrf-token-abc");
  });
});

describe("401 跳转与 next 安全性（UI-002）", () => {
  it("会话探测 401 时跳登录页并保留 next", async () => {
    stubFetch((url) => {
      if (url === "/api/v1/auth/session") {
        return errorResponse(401, "UNAUTHORIZED", "需要登录");
      }
      return undefined;
    });

    renderApp({ route: "/settings" });

    expect(await screen.findByLabelText("密码")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain("登录已过期");
    expect(screen.getByText("/settings")).toBeTruthy();
  });

  it("业务请求 401 时丢弃本地状态跳登录页（已在登录页不重复跳转）", async () => {
    stubFetch((url) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION);
      }
      if (url.startsWith("/api/v1/items")) {
        return errorResponse(401, "UNAUTHORIZED", "需要登录");
      }
      return undefined;
    });

    renderApp({ route: "/" });

    expect(await screen.findByLabelText("密码")).toBeTruthy();
    // 已经在登录页：不再重复触发跳转（不出现循环请求）。
    expect(screen.getAllByRole("button", { name: "登录" })).toHaveLength(1);
  });

  it("非相对或跨站的 next 一律回落 /", async () => {
    expect(safeNextPath("/items/item-1")).toBe("/items/item-1");
    expect(safeNextPath("/items/item-1/edit?x=1")).toBe("/items/item-1/edit?x=1");
    expect(safeNextPath("https://evil.example/steal")).toBe("/");
    expect(safeNextPath("//evil.example/steal")).toBe("/");
    expect(safeNextPath("/\\evil.example")).toBe("/");
    expect(safeNextPath("/a\\b")).toBe("/");
    expect(safeNextPath("javascript:alert(1)")).toBe("/");
    expect(safeNextPath("items")).toBe("/");
    expect(safeNextPath(null)).toBe("/");
    expect(safeNextPath("")).toBe("/");
    expect(safeNextPath("/a\nb")).toBe("/");
  });

  it("跨站 next 参数不会出现在登录页的返回提示中", async () => {
    stubFetch((url) => {
      if (url === "/api/v1/auth/session") {
        return errorResponse(401, "UNAUTHORIZED", "需要登录");
      }
      return undefined;
    });

    renderApp({ route: "/login?next=https%3A%2F%2Fevil.example%2Fsteal" });

    await screen.findByLabelText("密码");
    expect(screen.queryByText(/evil\.example/)).toBeNull();
  });
});

describe("CSRF 与 If-Match 注入（T08 卡）", () => {
  it("编辑物品提交携带 X-CSRF-Token 与 If-Match（ETag 原样回传），且 token 不进 DOM", async () => {
    const fetchMock = stubFetch((url, init) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION);
      }
      if (url === "/api/v1/items/item-1" && method(init) === "GET") {
        return jsonResponse({ data: ITEM }, { etag: '"r3"' });
      }
      if (url === "/api/v1/items/item-1" && method(init) === "PATCH") {
        return jsonResponse({ data: { ...ITEM, revision: 4 } }, { etag: '"r4"' });
      }
      if (url === "/api/v1/items/item-1/documents" || url === "/api/v1/items/item-1/photos") {
        return jsonResponse({ data: [], nextCursor: null });
      }
      if (url.startsWith("/api/v1/items")) {
        return jsonResponse({ data: [], nextCursor: null });
      }
      return undefined;
    });

    renderApp({ route: "/items/item-1/edit" });
    const name = (await screen.findByLabelText(/名称/)) as HTMLInputElement;
    fireEvent.change(name, { target: { value: "相机 A（改名）" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(fetchMock.mock.calls.some(([, init]) => method(init ?? {}) === "PATCH")).toBe(true);
    });

    const patchCall = fetchMock.mock.calls.find(([, init]) => method(init ?? {}) === "PATCH");
    const patchHeaders = headersOf(patchCall?.[1] ?? {});
    expect(patchHeaders["x-csrf-token"]).toBe("csrf-token-abc");
    expect(patchHeaders["if-match"]).toBe('"r3"');
    expect(JSON.parse(String(patchCall?.[1]?.body))).toMatchObject({ name: "相机 A（改名）" });

    // GET 不携带 CSRF（只有修改请求注入）。
    const getCall = fetchMock.mock.calls.find(
      ([url, init]) => String(url) === "/api/v1/items/item-1" && method(init ?? {}) === "GET",
    );
    expect(headersOf(getCall?.[1] ?? {})["x-csrf-token"]).toBeUndefined();

    // 凭据不进 DOM。
    expect(document.body.innerHTML).not.toContain("csrf-token-abc");
  });
});

describe("412 并发冲突恢复（UI-008）", () => {
  it("显示 currentRevision 与刷新后重试，保留输入；刷新后用新 ETag 重新提交", async () => {
    // 服务端版本在冲突后前进到 r7（模拟"其他操作已更新"）。
    let serverRevision = 3;
    const fetchMock = stubFetch((url, init) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION);
      }
      if (url === "/api/v1/items/item-1" && method(init) === "GET") {
        return jsonResponse(
          { data: { ...ITEM, revision: serverRevision } },
          { etag: `"r${serverRevision}"` },
        );
      }
      if (url === "/api/v1/items/item-1" && method(init) === "PATCH") {
        const ifMatch = headersOf(init)["if-match"];
        if (ifMatch === '"r3"') {
          return errorResponse(412, "REVISION_CONFLICT", "该内容已被其他操作更新", {
            currentRevision: 7,
          });
        }
        return jsonResponse({ data: { ...ITEM, revision: 8 } }, { etag: '"r8"' });
      }
      if (url === "/api/v1/items/item-1/documents" || url === "/api/v1/items/item-1/photos") {
        return jsonResponse({ data: [], nextCursor: null });
      }
      return undefined;
    });

    renderApp({ route: "/items/item-1/edit" });
    const name = (await screen.findByLabelText(/名称/)) as HTMLInputElement;
    fireEvent.change(name, { target: { value: "保留的输入" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("该内容已被其他操作更新（当前 r7）");
    // 不丢表单内容 + 刷新前禁用提交。
    expect((screen.getByLabelText(/名称/) as HTMLInputElement).value).toBe("保留的输入");
    expect((screen.getByRole("button", { name: "保存" }) as HTMLButtonElement).disabled).toBe(true);
    const refreshButton = screen.getByRole("button", { name: "刷新后重试" });

    // 刷新：重新读取服务端最新版本（r7）。
    serverRevision = 7;
    fireEvent.click(refreshButton);
    expect(await screen.findByText(/已刷新到服务端最新版本（r7）/)).toBeTruthy();
    expect((screen.getByLabelText(/名称/) as HTMLInputElement).value).toBe("保留的输入");

    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => {
      const patchCalls = fetchMock.mock.calls.filter(([, init]) => method(init ?? {}) === "PATCH");
      expect(patchCalls).toHaveLength(2);
    });
    const patchCalls = fetchMock.mock.calls.filter(([, init]) => method(init ?? {}) === "PATCH");
    expect(headersOf(patchCalls[1]?.[1] ?? {})["if-match"]).toBe('"r7"');
  });
});

describe("错误边界（§6.1.1）", () => {
  it("渲染异常显示可读文案 + requestId + 返回资料库，不显示堆栈", async () => {
    // 合同违约载荷（documents 缺 sourceSha256）触发渲染异常，验证真实路由上的边界。
    vi.spyOn(console, "error").mockImplementation(() => {});
    stubFetch((url) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION);
      }
      if (url === "/api/v1/items/item-1") {
        return jsonResponse({ data: ITEM }, { etag: '"r3"', requestId: REQUEST_ID });
      }
      if (url === "/api/v1/items/item-1/documents") {
        return jsonResponse(
          {
            data: [
              {
                id: "doc-1",
                itemId: "item-1",
                sourceAssetId: "asset-1",
                title: "说明书",
                createdAt: "2026-09-11T00:00:00Z",
                updatedAt: "2026-09-11T00:00:00Z",
                sourceUrl: null,
                // 合同违约：sourceSha256 必须存在（触发渲染异常，验证真实路由上的边界）。
                sourceSha256: null,
              },
            ],
            nextCursor: null,
          },
          { requestId: REQUEST_ID },
        );
      }
      if (url === "/api/v1/items/item-1/photos") {
        return jsonResponse({ data: [], nextCursor: null }, { requestId: REQUEST_ID });
      }
      return undefined;
    });

    renderApp({ route: "/items/item-1" });

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("页面出现异常");
    expect(alert.textContent).toContain("页面渲染时发生错误");
    expect(alert.textContent).toContain(REQUEST_ID);
    expect(alert.textContent).toContain("返回资料库");
    // 不显示堆栈/源码位置。
    expect(alert.textContent).not.toContain(".tsx");
    expect(alert.textContent).not.toContain(".js:");
  });
});

describe("断点与抽屉（§6.1.1 / §6.1.5）", () => {
  it("wide 直接并排显示侧栏，mid 需展开，narrow 用抽屉", async () => {
    setViewportWidth(1400);
    stubSignedInItems();
    const first = renderApp({ route: "/" });
    await screen.findByRole("heading", { name: "资料库" });
    expect(screen.getByRole("complementary", { name: "资料库摘要" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /显示.*摘要/ })).toBeNull();
    first.unmount();

    setViewportWidth(900);
    stubSignedInItems();
    const second = renderApp({ route: "/" });
    await screen.findByRole("heading", { name: "资料库" });
    expect(screen.queryByRole("complementary", { name: "资料库摘要" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "显示资料库摘要" }));
    expect(screen.getByRole("complementary", { name: "资料库摘要" })).toBeTruthy();
    second.unmount();

    setViewportWidth(600);
    stubSignedInItems();
    renderApp({ route: "/" });
    await screen.findByRole("heading", { name: "资料库" });
    const trigger = screen.getByRole("button", { name: "资料库摘要" });
    fireEvent.click(trigger);
    const dialog = screen.getByRole("dialog", { name: "资料库摘要" });
    expect(within(dialog).getByText("已加载")).toBeTruthy();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
    expect(document.activeElement).toBe(trigger);
  });

  it("抽屉焦点陷阱：Tab 在面板内循环，Esc 关闭", async () => {
    const onClose = vi.fn();
    render(
      <Drawer open onClose={onClose} title="测试抽屉">
        <a href="#first">第一个</a>
        <button type="button">第二个</button>
      </Drawer>,
    );

    const close = screen.getByRole("button", { name: "关闭" });
    const second = screen.getByRole("button", { name: "第二个" });
    // 打开时焦点进入抽屉的第一个可聚焦元素。
    expect(document.activeElement).toBe(close);

    // Tab 在最后一个元素上回绕到第一个；Shift+Tab 在第一个元素上回绕到最后一个。
    second.focus();
    fireEvent.keyDown(document, { key: "Tab" });
    expect(document.activeElement).toBe(close);

    fireEvent.keyDown(document, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(second);

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("路由骨架与文案边界", () => {
  it("版本列表已交付：显示真实空态而不是占位页，且只请求它需要的数据", async () => {
    // 事实更新（T19 交付回合，2026-09-12）：T17/T18/T19 之后，PRD §6.1.2 的主路由
    // 全部有真实实现，没有占位页可指。本用例改写为守住"真实页面 + 精确的数据请求"：
    // 版本列表读取发布版本并显示空态，不再出现"该页面尚未实现"。
    const fetchMock = stubFetch((url) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION);
      }
      if (url === "/api/v1/items/item-1/releases") {
        return jsonResponse({ data: [], nextCursor: null });
      }
      return undefined;
    });

    renderApp({ route: "/items/item-1/releases" });

    expect(await screen.findByText(/还没有发布版本/)).toBeTruthy();
    expect(screen.queryByText("该页面尚未实现。")).toBeNull();
    // 页面只请求自己需要的数据；`/api/v1/items/item-1` 是 AppShell 顶栏的物品上下文
    // （§6.1.1 的既有行为，T08 起如此）。
    const urls = fetchMock.mock.calls.map(([url]) => String(url));
    expect(urls.filter((url) => url !== "/api/v1/items/item-1")).toEqual([
      "/api/v1/auth/session",
      "/api/v1/items/item-1/releases",
    ]);
  });

  it("向导占位页保留五步步骤条且当前步 aria-current", async () => {
    stubFetch((url) => {
      if (url === "/api/v1/auth/session") {
        return jsonResponse(SESSION);
      }
      return undefined;
    });

    renderApp({ route: "/items/item-1/import/prepare" });

    const steps = await screen.findByRole("list", { name: "新建向导步骤" });
    expect(within(steps).getAllByRole("listitem")).toHaveLength(5);
    expect(within(steps).getByText("准备").getAttribute("aria-current")).toBe("step");
  });

  it("关键页面不出现 PRD §6.3.2 的禁用措辞", async () => {
    stubSignedInItems();
    const result = renderApp({ route: "/" });
    await screen.findByRole("heading", { name: "资料库" });
    const html = document.body.innerHTML;
    for (const phrase of FORBIDDEN_WORDING) {
      expect(html).not.toContain(phrase);
    }
    result.unmount();
  });
});
