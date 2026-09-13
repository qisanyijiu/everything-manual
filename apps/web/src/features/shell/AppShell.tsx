/**
 * 应用外壳（PRD §6.1.1 顶栏 + 每路由错误边界）：
 * - 顶栏：产品名、当前物品名·型号（无当前物品时隐藏）、任务中心、设置、登出；
 *   顶栏不承载业务提交按钮；
 * - 每个路由挂错误边界（渲染异常显示可读文案 + requestId + 返回资料库，不显示堆栈）。
 *
 * 任务中心徽标（进行中任务计数）需要 `/jobs` 服务端接口（T15/T17），尚未实现前不显示
 * 任何计数，避免用假数字冒充。
 */

import { Link, Outlet, useLocation, useMatch, useNavigate } from "react-router";
import { useQuery } from "@tanstack/react-query";

import { describeError } from "../../api/client";
import { RouteErrorBoundary } from "../../components/ErrorBoundary";
import { useNotify } from "../../components/notifications";
import { getItem } from "../../api/endpoints";
import { itemKeys } from "../library/items";
import { useLogout } from "../auth/session";

export function AppShell() {
  const location = useLocation();
  const navigate = useNavigate();
  const notify = useNotify();
  const logoutMutation = useLogout();
  const exactItemMatch = useMatch("/items/:itemId");
  const nestedItemMatch = useMatch("/items/:itemId/*");
  const itemId = exactItemMatch?.params.itemId ?? nestedItemMatch?.params.itemId;

  const itemQuery = useQuery({
    queryKey: itemKeys.detail(itemId ?? ""),
    queryFn: () => getItem(itemId ?? ""),
    enabled: itemId !== undefined && itemId !== "",
    retry: false,
  });

  async function handleLogout() {
    try {
      await logoutMutation.mutateAsync();
    } catch (error) {
      const info = describeError(error);
      notify(`登出请求失败：${info.message}`, { kind: "alert", requestId: info.requestId });
    } finally {
      navigate("/login", { replace: true });
    }
  }

  return (
    <div className="app-shell">
      <header className="top-bar">
        <div className="top-bar__left">
          <Link to="/" className="top-bar__brand">
            万物说明书
          </Link>
          {itemQuery.data !== undefined && (
            <p className="top-bar__context">
              {itemQuery.data.data.name}
              {itemQuery.data.data.model !== "" && ` · ${itemQuery.data.data.model}`}
            </p>
          )}
        </div>
        <nav className="top-bar__nav" aria-label="主导航">
          <Link to="/">资料库</Link>
          <Link to="/jobs">任务中心</Link>
          <Link to="/settings">设置</Link>
          <button type="button" onClick={() => void handleLogout()} disabled={logoutMutation.isPending}>
            {logoutMutation.isPending ? "登出中…" : "登出"}
          </button>
        </nav>
      </header>
      {/* tabIndex=-1：作为程序化焦点目标（会话恢复后焦点落回页面主体），不进入 Tab 顺序。 */}
      <main className="app-shell__content" id="main" tabIndex={-1}>
        <RouteErrorBoundary key={location.pathname}>
          <Outlet />
        </RouteErrorBoundary>
      </main>
    </div>
  );
}
