/**
 * 会话门禁（PRD §6.2 UI-002）：
 * - 恢复中显示全屏骨架，**不先闪登录页**；
 * - 会话探测 401 → 跳 `/login?next=<站内相对路径>`（只接受同源相对路径）；
 * - 网络等非 401 失败 → 显示可读错误 + 重试，不自动跳转、不重放已失败请求。
 */

import { useEffect, type ReactNode } from "react";
import { useLocation, useNavigate } from "react-router";

import { describeError, isApiError } from "../../api/client";
import { FullScreenSkeleton } from "../../components/Skeleton";
import { loginHref } from "../../lib/next-path";
import { useSession } from "../auth/session";

export function RequireSession({ children }: { children: ReactNode }) {
  const session = useSession();
  const location = useLocation();
  const navigate = useNavigate();
  const unauthorized = session.isError && isApiError(session.error) && session.error.status === 401;

  useEffect(() => {
    if (unauthorized) {
      navigate(loginHref(location), { replace: true, state: { expired: true } });
    }
  }, [unauthorized, location, navigate]);

  if (unauthorized) {
    // 正在跳转；保持骨架，避免闪一下受保护内容或登录表单。
    return <FullScreenSkeleton label="登录已过期，正在跳转登录页…" />;
  }

  if (session.isPending) {
    return <FullScreenSkeleton label="正在恢复会话…" />;
  }

  if (session.isError) {
    const info = describeError(session.error);
    return (
      <div className="full-screen-skeleton">
        <section className="error-panel" role="alert" aria-labelledby="session-error-title">
          <h1 id="session-error-title">无法恢复会话</h1>
          <p className="error-panel__message">{info.message}</p>
          {info.requestId !== null && (
            <p className="error-panel__request">
              诊断请求 ID：<code>{info.requestId}</code>
            </p>
          )}
          <div className="error-panel__actions">
            <button type="button" onClick={() => void session.refetch()}>
              重试
            </button>
          </div>
        </section>
      </div>
    );
  }

  return children;
}
