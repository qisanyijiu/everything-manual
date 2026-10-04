/**
 * 全局通知条（PRD §6.1.1 / §6.3.3 U-07）：
 * - 成功用 `role="status"`（不打断朗读），失败用 `role="alert"`；
 * - 通知条只做摘要，失败信息必须同时保留在对应区块（调用方负责内联错误）；
 * - 不承载唯一错误信息，也不放凭据：requestId 是可展示的诊断 ID。
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Link } from "react-router";

export type NoticeKind = "status" | "alert";

/** 通知里的跳转动作（站内路由；例如"生成完成 → 查看结果"）。 */
export interface NoticeAction {
  readonly label: string;
  readonly to: string;
}

export interface Notice {
  readonly id: number;
  readonly kind: NoticeKind;
  readonly message: string;
  readonly requestId: string | null;
  readonly action: NoticeAction | null;
  /** true = 不自动消失（需要用户处理的结果提示，例如生成完成）。 */
  readonly sticky: boolean;
}

export interface NotifyOptions {
  kind?: NoticeKind;
  requestId?: string | null;
  action?: NoticeAction | null;
  sticky?: boolean;
}

interface NotificationContextValue {
  notify: (message: string, options?: NotifyOptions) => void;
  notices: Notice[];
  dismiss: (id: number) => void;
  dismissAll: () => void;
}

const NotificationContext = createContext<NotificationContextValue | null>(null);

/** 成功提示自动消失的时间（失败提示保留到用户关闭）。 */
const AUTO_DISMISS_MS = 6000;

export function NotificationProvider({ children }: { children: ReactNode }) {
  const [notices, setNotices] = useState<Notice[]>([]);
  const nextId = useRef(1);

  const dismiss = useCallback((id: number) => {
    setNotices((list) => list.filter((notice) => notice.id !== id));
  }, []);
  const dismissAll = useCallback(() => setNotices([]), []);

  const notify = useCallback((message: string, options: NotifyOptions = {}) => {
    const id = nextId.current;
    nextId.current += 1;
    setNotices((list) => [
      ...list,
      {
        id,
        kind: options.kind ?? "status",
        message,
        requestId: options.requestId ?? null,
        action: options.action ?? null,
        sticky: options.sticky ?? false,
      },
    ]);
  }, []);

  const value = useMemo(() => ({ notify, notices, dismiss, dismissAll }), [notify, notices, dismiss, dismissAll]);

  return (
    <NotificationContext.Provider value={value}>
      {children}
    </NotificationContext.Provider>
  );
}

/** Render below the shell header in normal flow, so notices never cover page controls or dialogs. */
export function NotificationRegion() {
  const context = useContext(NotificationContext);
  if (context === null || context.notices.length === 0) return null;
  return (
    <section className="notices" aria-label="全局通知">
      {context.notices.length > 1 && (
        <div className="notices__toolbar">
          <span>{context.notices.length} 条通知</span>
          <button type="button" onClick={context.dismissAll}>关闭全部通知</button>
        </div>
      )}
      <div className="notices__list">
        {context.notices.map((notice) => (
          <NoticeItem key={notice.id} notice={notice} onDismiss={context.dismiss} />
        ))}
      </div>
    </section>
  );
}

export function useNotify(): (message: string, options?: NotifyOptions) => void {
  const context = useContext(NotificationContext);
  if (context === null) {
    throw new Error("useNotify 必须在 NotificationProvider 内使用");
  }
  return context.notify;
}

function NoticeItem({
  notice,
  onDismiss,
}: {
  notice: Notice;
  onDismiss: (id: number) => void;
}) {
  useEffect(() => {
    if (notice.kind !== "status" || notice.sticky) {
      return;
    }
    const timer = setTimeout(() => onDismiss(notice.id), AUTO_DISMISS_MS);
    return () => clearTimeout(timer);
  }, [notice.id, notice.kind, notice.sticky, onDismiss]);

  return (
    <div className={`notice notice--${notice.kind}`}>
      <p role={notice.kind === "status" ? "status" : "alert"} className="notice__message">
        {notice.message}
        {notice.requestId !== null && (
          <>
            {"（请求 ID："}
            <code>{notice.requestId}</code>
            {"）"}
          </>
        )}
      </p>
      {notice.action !== null && (
        <Link className="notice__action" to={notice.action.to} onClick={() => onDismiss(notice.id)}>
          {notice.action.label}
        </Link>
      )}
      <button
        type="button"
        className="notice__dismiss"
        onClick={() => onDismiss(notice.id)}
        aria-label={`关闭通知：${notice.message}`}
      >
        关闭
      </button>
    </div>
  );
}
