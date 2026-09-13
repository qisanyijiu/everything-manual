/**
 * 应用入口：Provider 组合与路由表（PRD §6.1.2）。
 *
 * 路由实现状态（T19 后）：
 * - 完整实现：`/login`、`/`（资料库）、`/items/new`、`/items/:itemId`、`/items/:itemId/edit`、
 *   向导第 2–5 步（`import/document`、`import/views`、`import/prepare`、`import/confirm`）、
 *   `/jobs`（任务中心）、`/jobs/:jobId`（任务详情）、`/settings`；
 * - **校准工作区**：`/items/:itemId/drafts/:draftId/review`（T18 只读承载 → T19：
 *   热点校准、步骤视角、知识确认与修订、发布）；
 * - **发布与阅读器**：`/items/:itemId/releases[/:releaseId]`（T19：版本列表与不可变
 *   manifest 阅读器，四方联动）；导出/备份属 T20（未实现）。
 */

import { Suspense, lazy, useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter, Route, Routes } from "react-router";

import { RouteErrorBoundary } from "./components/ErrorBoundary";
import { NotificationProvider } from "./components/notifications";
import { LoginPage } from "./features/auth/LoginPage";
import { ConfirmStepPage } from "./features/import/ConfirmStepPage";
import { DocumentStepPage } from "./features/import/DocumentStepPage";
import { ViewsStepPage } from "./features/import/ViewsStepPage";
import { JobDetailPage } from "./features/jobs/JobDetailPage";
import { JobsListPage } from "./features/jobs/JobsListPage";
import { ItemFormPage } from "./features/library/ItemFormPage";
import { ItemOverviewPage } from "./features/library/ItemOverviewPage";
import { LibraryPage } from "./features/library/LibraryPage";
import { AppShell } from "./features/shell/AppShell";
import { NotFoundPage } from "./features/shell/NotFoundPage";
import { RequireSession } from "./features/shell/RequireSession";
import { Skeleton } from "./components/Skeleton";
import { SessionExpiryWatcher } from "./features/shell/SessionExpiryWatcher";
import { SettingsPage } from "./features/settings/SettingsPage";

/**
 * PDF 准备页懒加载（PRD §5.5：「3D/PDF 不在首屏库列表强制加载」）：
 * pdfjs-dist 主包与 worker 只在进入准备页时下载，资料库首屏不背这部分体积。
 */
const PreparePage = lazy(() =>
  import("./features/import/PreparePage").then((module) => ({ default: module.PreparePage })),
);

/**
 * 阅读/校准工作区懒加载（T18/T19；PRD §5.5 同一约束的 3D 侧）：
 * 该页内部再对 3D（three）与原文（pdfjs）分别 `lazy()`，因此资料库首屏既不含
 * three 也不含 PDF.js；进页面后 3D 模块也只在确实要渲染模型时下载。
 */
const ReviewWorkspacePage = lazy(() =>
  import("./features/viewer/ReviewWorkspacePage").then((module) => ({
    default: module.ReviewWorkspacePage,
  })),
);

/** 发布版本列表与阅读器（T19）：与校准页同一懒加载边界（含 3D/PDF 子模块）。 */
const ReleaseListPage = lazy(() =>
  import("./features/manual/ReleaseListPage").then((module) => ({
    default: module.ReleaseListPage,
  })),
);
const ReleaseReaderPage = lazy(() =>
  import("./features/manual/ReleaseReaderPage").then((module) => ({
    default: module.ReleaseReaderPage,
  })),
);

/**
 * Query 默认值（PRD UI-002「不自动重放已失败请求」）：
 * 不做自动重试，也不因窗口聚焦重新请求；网络错误由页面显式「重试」触发。
 */
export function createAppQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnWindowFocus: false,
        staleTime: 30_000,
      },
      mutations: {
        retry: false,
      },
    },
  });
}

export function App() {
  const [queryClient] = useState(createAppQueryClient);
  return (
    <QueryClientProvider client={queryClient}>
      <NotificationProvider>
        <BrowserRouter>
          <AppRoutes />
        </BrowserRouter>
      </NotificationProvider>
    </QueryClientProvider>
  );
}

export function AppRoutes() {
  return (
    <>
      <SessionExpiryWatcher />
      <RouteErrorBoundary>
        <Routes>
          <Route path="/login" element={<LoginPage />} />
          <Route
            element={
              <RequireSession>
                <AppShell />
              </RequireSession>
            }
          >
            <Route path="/" element={<LibraryPage />} />
            <Route path="/items/new" element={<ItemFormPage mode="create" />} />
            <Route path="/items/:itemId" element={<ItemOverviewPage />} />
            <Route path="/items/:itemId/edit" element={<ItemFormPage mode="edit" />} />
            <Route
              path="/items/:itemId/import/document"
              element={<DocumentStepPage />}
            />
            <Route path="/items/:itemId/import/views" element={<ViewsStepPage />} />
            <Route
              path="/items/:itemId/import/prepare"
              element={
                <Suspense fallback={<Skeleton label="正在加载 PDF 准备模块…" rows={3} />}>
                  <PreparePage />
                </Suspense>
              }
            />
            <Route path="/items/:itemId/import/confirm" element={<ConfirmStepPage />} />
            <Route path="/jobs" element={<JobsListPage />} />
            <Route path="/jobs/:jobId" element={<JobDetailPage />} />
            <Route
              path="/items/:itemId/drafts/:draftId/review"
              element={
                <Suspense fallback={<Skeleton label="正在加载阅读器模块…" rows={4} />}>
                  <ReviewWorkspacePage />
                </Suspense>
              }
            />
            <Route
              path="/items/:itemId/releases"
              element={
                <Suspense fallback={<Skeleton label="正在加载版本列表…" rows={3} />}>
                  <ReleaseListPage />
                </Suspense>
              }
            />
            <Route
              path="/items/:itemId/releases/:releaseId"
              element={
                <Suspense fallback={<Skeleton label="正在加载阅读器模块…" rows={4} />}>
                  <ReleaseReaderPage />
                </Suspense>
              }
            />
            <Route path="/settings" element={<SettingsPage />} />
            <Route path="*" element={<NotFoundPage />} />
          </Route>
        </Routes>
      </RouteErrorBoundary>
    </>
  );
}
