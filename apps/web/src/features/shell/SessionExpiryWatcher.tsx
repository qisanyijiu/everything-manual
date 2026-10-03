import { useWorkProtection } from "./work-protection";
/**
 * 任意 API 返回 401 时的统一处理（PRD §6.2 UI-002）：
 * 丢弃本地查询状态 → 跳 `/login?next=<站内相对路径>`（`next` 只接受同源相对路径）；
 * 已经在登录页时不重复跳转（避免循环），也不自动重放失败请求。
 *
 * 「登录已过期，请重新登录」的提示由登录页按 router state 渲染，这里不重复通知。
 */

import { useEffect, useRef } from "react";
import { useLocation, useNavigate } from "react-router";
import { useQueryClient } from "@tanstack/react-query";

import { onUnauthorized, setCsrfToken } from "../../api/client";
import { hasVerifiedSession } from "../auth/session";
import { loginHref } from "../../lib/next-path";

export function SessionExpiryWatcher() {
  const { bypass } = useWorkProtection();
  const location = useLocation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const locationRef = useRef(location);

  useEffect(() => {
    locationRef.current = location;
  }, [location]);

  useEffect(() => {
    return onUnauthorized(() => {
      const current = locationRef.current;
      const expired = hasVerifiedSession(queryClient);
      setCsrfToken(null);
      queryClient.removeQueries();
      if (current.pathname === "/login") {
        return;
      }
      bypass(() => navigate(loginHref(current), { replace: true, state: { expired } }));
    });
  }, [navigate, queryClient, bypass]);

  return null;
}
