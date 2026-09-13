/**
 * 会话与 CSRF 状态（PRD §6.2 UI-001/UI-002；ADR-013）：
 * - `GET /auth/session` 恢复会话（`Cache-Control: no-store`），恢复中不闪登录页；
 * - CSRF token 由服务端下发并只保存在前端内存（`api/client` 模块作用域），
 *   所有修改请求由 fetch 封装自动注入 `X-CSRF-Token`；
 * - 登出清空查询缓存与内存 token，不留下可复用的本地状态。
 *
 * 401 的统一跳转由 [`SessionExpiryWatcher`]（shell 层）处理，避免每个调用点各自为政。
 */

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { setCsrfToken, type ApiResource } from "../../api/client";
import { fetchSession, login, logout, type SessionData } from "../../api/endpoints";

export const SESSION_QUERY_KEY = ["session"] as const;

/** 会话恢复查询：401 交给会话恢复流程处理（不广播全局跳转），且不自动重试。 */
export function useSession() {
  return useQuery({
    queryKey: SESSION_QUERY_KEY,
    queryFn: async (): Promise<ApiResource<SessionData>> => {
      const session = await fetchSession();
      setCsrfToken(session.data.csrfToken);
      return session;
    },
    retry: false,
    staleTime: Infinity,
    gcTime: Infinity,
  });
}

export function useLogin() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (password: string) => login({ password }),
    onSuccess: (session) => {
      setCsrfToken(session.data.csrfToken);
      queryClient.setQueryData(SESSION_QUERY_KEY, session);
      void queryClient.invalidateQueries({
        predicate: (query) => query.queryKey[0] !== SESSION_QUERY_KEY[0],
      });
    },
  });
}

export function useLogout() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => logout(),
    onSettled: () => {
      // 无论服务端登出成功与否，本地凭据与查询缓存都不保留。
      setCsrfToken(null);
      queryClient.removeQueries();
    },
  });
}
