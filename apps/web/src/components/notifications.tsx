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

export type NoticeKind = "status" | "alert";

export interface Notice {
  readonly id: number;
  readonly kind: NoticeKind;
  readonly message: string;
  readonly requestId: string | null;
}

export interface NotifyOptions {
  kind?: NoticeKind;
  requestId?: string | null;
}

interface NotificationContextValue {
  notify: (message: string, options?: NotifyOptions) => void;
}

const NotificationContext = createContext<NotificationContextValue | null>(null);

/** 成功提示自动消失的时间（失败提示保留到用户关闭）。 */
const AUTO_DISMISS_MS = 6000;

export function NotificationProvider({ children }: { children: ReactNode }) {
  const [notices, setNotices] = useState<Notice[]>([]);
  const nextId = useRef(1);
  useNoticesBelowTopBar(notices.length > 0);

  const dismiss = useCallback((id: number) => {
    setNotices((list) => list.filter((notice) => notice.id !== id));
  }, []);

  const notify = useCallback((message: string, options: NotifyOptions = {}) => {
    const id = nextId.current;
    nextId.current += 1;
    setNotices((list) => [
      ...list,
      { id, kind: options.kind ?? "status", message, requestId: options.requestId ?? null },
    ]);
  }, []);

  const value = useMemo(() => ({ notify }), [notify]);

  return (
    <NotificationContext.Provider value={value}>
      {children}
      <div className="notices" aria-label="全局通知">
        {notices.map((notice) => (
          <NoticeItem key={notice.id} notice={notice} onDismiss={dismiss} />
        ))}
      </div>
    </NotificationContext.Provider>
  );
}

/**
 * 让通知条固定出现在**顶栏之下**（BUG-001-r8）：
 * 通知条曾盖住顶栏，在其 6 秒可见期内吞掉对主导航的指针点击（键盘路径不受影响）。
 * 修复由两部分组成：
 * 1. CSS（`styles.css`）：`.notices` 容器 `pointer-events: none` + `.notice`
 *    `pointer-events: auto`，通知内容之外的区域不再拦截指针；
 * 2. 本 hook：通知出现时测量**真实**顶栏高度并写入 `--notices-top`
 *    （顶栏会随物品上下文与窄屏换行变化，固定像素值会漂移）。
 * 不做任何布局反馈（该变量只用于通知条定位），因此不会引发 resize 循环。
 */
function useNoticesBelowTopBar(active: boolean): void {
  useEffect(() => {
    if (!active) {
      return;
    }
    const update = (): void => {
      const topBar = document.querySelector(".top-bar");
      if (topBar === null) {
        return;
      }
      const height = Math.ceil(topBar.getBoundingClientRect().height);
      document.documentElement.style.setProperty("--notices-top", `${height}px`);
    };
    update();
    window.addEventListener("resize", update);
    // 顶栏高度还会因路由切换而变化（物品上下文出现/消失、窄屏换行、导航换行），
    // 而这些变化不一定触发 window resize：通知可见期间按固定间隔重新测量。
    // 只写定位变量（无布局反馈），可见期最长 6 秒、间隔 100ms，开销可忽略。
    const timer = window.setInterval(update, 100);
    return () => {
      window.removeEventListener("resize", update);
      window.clearInterval(timer);
    };
  }, [active]);
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
    if (notice.kind !== "status") {
      return;
    }
    const timer = setTimeout(() => onDismiss(notice.id), AUTO_DISMISS_MS);
    return () => clearTimeout(timer);
  }, [notice.id, notice.kind, onDismiss]);

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
