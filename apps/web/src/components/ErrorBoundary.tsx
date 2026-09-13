/**
 * 路由级错误边界（PRD §6.1.1）：
 * 渲染异常显示可读文案 + requestId（响应含有时）+「返回资料库」，**不显示堆栈**。
 */

import { Component, type ReactNode } from "react";
import { Link } from "react-router";

import { isApiError, lastRequestId } from "../api/client";

interface BoundaryProps {
  children: ReactNode;
}

interface BoundaryState {
  error: unknown;
}

export class RouteErrorBoundary extends Component<BoundaryProps, BoundaryState> {
  override state: BoundaryState = { error: null };

  static getDerivedStateFromError(error: unknown): BoundaryState {
    return { error };
  }

  private readonly reset = () => {
    this.setState({ error: null });
  };

  override render(): ReactNode {
    if (this.state.error !== null) {
      return <ErrorFallback error={this.state.error} onRetry={this.reset} />;
    }
    return this.props.children;
  }
}

export function ErrorFallback({
  error,
  onRetry,
}: {
  error: unknown;
  onRetry?: () => void;
}) {
  // 合同错误：显示服务端给用户的可读文案；其余（渲染异常等）：只给通用文案，
  // 不把内部异常消息/堆栈暴露到界面。
  const message = isApiError(error) ? error.message : "页面渲染时发生错误，请重试或返回资料库。";
  const requestId = (isApiError(error) ? error.requestId : null) ?? lastRequestId();
  return (
    <section className="error-panel" role="alert" aria-labelledby="error-fallback-title">
      <h1 id="error-fallback-title">页面出现异常</h1>
      <p className="error-panel__message">{message}</p>
      {requestId !== null && (
        <p className="error-panel__request">
          诊断请求 ID：<code>{requestId}</code>
        </p>
      )}
      <p className="error-panel__hint">
        你可以返回资料库继续其他操作；若问题重复出现，请把上面的请求 ID 提供给管理员。
      </p>
      <div className="error-panel__actions">
        {onRetry !== undefined && (
          <button type="button" onClick={onRetry}>
            重试
          </button>
        )}
        <Link to="/">返回资料库</Link>
      </div>
    </section>
  );
}
