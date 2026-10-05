import { JobActivityLink } from "../jobs/JobActivityLink";
import { useWorkProtection } from "./work-protection";
/**
 * 应用外壳（PRD §6.1.1 顶栏 + 每路由错误边界）：
 * - 顶栏：产品名、当前物品名·型号（无当前物品时隐藏）、任务中心、设置、登出；
 *   顶栏不承载业务提交按钮；
 * - 每个路由挂错误边界（渲染异常显示可读文案 + requestId + 返回资料库，不显示堆栈）。
 *
 * 任务计数使用全库有界汇总，失败不以0替代；本地倒计时不增加请求。
 */

import { Link, Outlet, useLocation, useMatch, useNavigate } from "react-router";
import { useQuery } from "@tanstack/react-query";

import { describeError } from "../../api/client";
import { RouteErrorBoundary } from "../../components/ErrorBoundary";
import { NotificationRegion, useNotify } from "../../components/notifications";
import { getItem } from "../../api/endpoints";
import { itemKeys } from "../library/items";
import { useLogout } from "../auth/session";
import { Icon } from "../../components/Icon";
import { JobCompletionWatcher } from "../jobs/JobCompletionWatcher";

export function AppShell() {
  const { request, bypass, memory } = useWorkProtection();
  const location = useLocation();
  const navigate = useNavigate();
  const notify = useNotify();
  const logoutMutation = useLogout();
  const exactItemMatch = useMatch("/items/:itemId");
  const nestedItemMatch = useMatch("/items/:itemId/*");
  const itemId = exactItemMatch?.params.itemId ?? nestedItemMatch?.params.itemId;

  /**
   * 侧栏选中判定（component-states.md §1「选中」冻结态）：视觉类名与 `aria-current="page"`
   * 必须由**同一布尔**驱动。不用 `NavLink` 的原因：react-router v7 的 `aria-current` 由
   * NavLink 自身 `isActive` 门控（传入值仅在内部 isActive 为真时生效），无法表达「资料库在
   * 物品相关路由保持选中」（BUG-VS02-001 根因）；这里按路由表统一判定三个导航项。
   */
  const pathname = location.pathname;
  const inRouteFamily = (base: string): boolean => pathname === base || pathname.startsWith(`${base}/`);
  const libraryActive = pathname === "/" || inRouteFamily("/items");
  const jobsActive = inRouteFamily("/jobs");
  const settingsActive = inRouteFamily("/settings");

  const itemQuery = useQuery({
    queryKey: itemKeys.detail(itemId ?? ""),
    queryFn: () => getItem(itemId ?? ""),
    enabled: itemId !== undefined && itemId !== "" && itemId !== "new",
    retry: false,
  });

  async function handleLogout() {
    try {
      await logoutMutation.mutateAsync();
    } catch (error) {
      const info = describeError(error);
      notify(`登出请求失败：${info.message}`, { kind: "alert", requestId: info.requestId });
    } finally {
      memory.clear();
      bypass(() => navigate("/login", { replace: true }));
    }
  }

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main">跳到主要内容</a>
      <aside className="app-sidebar" aria-label="应用导航">
        <Link to="/" className="sidebar-brand" aria-label="万物说明书">
          <span className="brand-symbol"><Icon name="book" size={25} /></span>
          <span>万物说明书<small>EVERYTHING MANUAL</small></span>
        </Link>
        <p className="sidebar-label">我的工作空间</p>
        <nav className="sidebar-nav" aria-label="主导航">
          <Link to="/" className={libraryActive ? "active" : undefined} aria-current={libraryActive ? "page" : undefined}><Icon name="grid" /><span>资料库</span></Link>
          <Link to="/jobs" className={jobsActive ? "active" : undefined} aria-current={jobsActive ? "page" : undefined}><Icon name="activity" /><span>任务中心</span></Link>
          <Link to="/settings" className={settingsActive ? "active" : undefined} aria-current={settingsActive ? "page" : undefined}><Icon name="settings" /><span>设置</span></Link>
        </nav>
        <div className="sidebar-bottom">
          <div className="sidebar-note"><Icon name="shield" size={22} /><p>你的物品，你的资料。<small>自托管 · 本地保存</small></p></div>
          <button className="sidebar-account" type="button" aria-label={logoutMutation.isPending ? "登出中…" : "登出"} onClick={() => request(() => { void handleLogout(); })} disabled={logoutMutation.isPending}>
            <span className="account-avatar">我</span><span>管理员<small>{logoutMutation.isPending ? "登出中…" : "登出"}</small></span><Icon name="logout" size={17} />
          </button>
        </div>
      </aside>
      <header className="top-bar">
        <div className="top-bar__left">
          <Link to="/" className="top-bar__brand">工作空间</Link>
          <Icon name="chevron" size={13} />
          <span className="top-bar__page">{location.pathname.startsWith("/jobs") ? "任务中心" : location.pathname.startsWith("/settings") ? "设置" : "资料库"}</span>
          {itemQuery.data !== undefined && (
            <p className="top-bar__context">
              {itemQuery.data.data.name}
              {itemQuery.data.data.model !== "" && ` · ${itemQuery.data.data.model}`}
            </p>
          )}
        </div>
        <div className="top-bar__utilities"><JobActivityLink /><button className="mobile-logout" type="button" aria-label="退出登录" onClick={() => request(() => { void handleLogout(); })} disabled={logoutMutation.isPending}><Icon name="logout" size={17} /></button></div>
      </header>
      <NotificationRegion />
      {/* tabIndex=-1：作为程序化焦点目标（会话恢复后焦点落回页面主体），不进入 Tab 顺序。 */}
      <main className="app-shell__content" id="main" tabIndex={-1}>
        <RouteErrorBoundary key={location.pathname}>
          <Outlet />
        </RouteErrorBoundary>
      </main>
      <JobCompletionWatcher />
    </div>
  );
}
