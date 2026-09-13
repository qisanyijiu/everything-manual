/**
 * 登录页（PRD §6.2 UI-001 / UI-002）：
 * - 单卡片居中、无顶栏业务入口；仅密码一个字段，无默认值、无占位密码；
 * - 密码框 `type=password` + `autocomplete=current-password`；
 * - 登录中按钮禁用并显示「登录中…」；密码为空不提交；
 * - 失败文案：401「密码不正确」、429「尝试过于频繁，请稍后再试」、403「请刷新页面后重试」，
 *   `role="alert"` 并把焦点移到错误摘要；
 * - 成功跳转 `next`（只接受站内相对路径）或 `/`。
 */

import { useEffect, useRef, useState } from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router";

import { describeError, isApiError } from "../../api/client";
import { TextField } from "../../components/form";
import { safeNextPath } from "../../lib/next-path";
import { useLogin } from "./session";

interface LoginError {
  readonly message: string;
  readonly requestId: string | null;
}

function loginErrorMessage(error: unknown): LoginError {
  if (isApiError(error)) {
    if (error.status === 401) {
      return { message: "密码不正确", requestId: error.requestId };
    }
    if (error.status === 429) {
      return { message: "尝试过于频繁，请稍后再试", requestId: error.requestId };
    }
    if (error.status === 403) {
      return { message: "请刷新页面后重试", requestId: error.requestId };
    }
    return { message: `登录失败：${error.message}`, requestId: error.requestId };
  }
  return { message: `登录失败：${describeError(error).message}`, requestId: null };
}

export function LoginPage() {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<LoginError | null>(null);
  const [searchParams] = useSearchParams();
  const location = useLocation();
  const navigate = useNavigate();
  const loginMutation = useLogin();
  const summaryRef = useRef<HTMLDivElement | null>(null);

  const next = safeNextPath(searchParams.get("next"));
  const expired = (location.state as { expired?: boolean } | null)?.expired === true;

  useEffect(() => {
    if (error !== null) {
      summaryRef.current?.focus();
    }
  }, [error]);

  async function onSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (password === "" || loginMutation.isPending) {
      return;
    }
    setError(null);
    try {
      await loginMutation.mutateAsync(password);
      navigate(next, { replace: true });
    } catch (caught) {
      setError(loginErrorMessage(caught));
    }
  }

  const fieldError = error === null ? null : error.message;
  const errorId = "login-error-summary";

  return (
    <div className="login-page">
      <main className="login-card" aria-labelledby="login-title">
        <h1 id="login-title">万物说明书</h1>
        <p className="login-card__subtitle">输入管理员密码以继续。</p>

        {expired && (
          <p className="login-card__notice" role="status">
            登录已过期，请重新登录。
            {next !== "/" && (
              <>
                {" "}
                登录后将返回 <code>{next}</code>。
              </>
            )}
          </p>
        )}

        {error !== null && (
          <div className="form-errors" role="alert" id={errorId} tabIndex={-1} ref={summaryRef}>
            <h2 className="form-errors__title">{error.message}</h2>
            {error.requestId !== null && (
              <p className="form-errors__request">
                诊断请求 ID：<code>{error.requestId}</code>
              </p>
            )}
          </div>
        )}

        <form className="login-form" onSubmit={(event) => void onSubmit(event)} noValidate>
          <TextField
            field="password"
            label="密码"
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={setPassword}
            error={fieldError}
          />
          <button type="submit" className="button-primary" disabled={password === "" || loginMutation.isPending}>
            {loginMutation.isPending ? "登录中…" : "登录"}
          </button>
        </form>

        {error !== null && (
          <p className="login-card__hint">
            若反复失败，请通过 <code>everything-manual init --data-dir &lt;目录&gt;</code> 重置密码。
          </p>
        )}

      </main>
    </div>
  );
}
