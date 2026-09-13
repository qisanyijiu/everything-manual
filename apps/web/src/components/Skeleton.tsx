/**
 * 加载骨架（PRD §6.1.1 加载态）：
 * - 文字标签用 `role="status"` 朗读，装饰条 `aria-hidden`；
 * - `prefers-reduced-motion: reduce` 下动画由 CSS 关闭（styles.css）；
 * - 骨架不夺取焦点（不使用 tabindex）。
 */

export function Skeleton({ label = "正在加载…", rows = 3 }: { label?: string; rows?: number }) {
  return (
    <div className="skeleton">
      <p className="skeleton__label" role="status">
        {label}
      </p>
      <div className="skeleton__bars" aria-hidden="true">
        {Array.from({ length: rows }, (_, index) => (
          <div className="skeleton__bar" key={index} />
        ))}
      </div>
    </div>
  );
}

/** 全屏骨架：启动恢复会话（UI-002）时使用，避免先闪登录页。 */
export function FullScreenSkeleton({ label }: { label: string }) {
  return (
    <div className="full-screen-skeleton">
      <Skeleton label={label} rows={4} />
    </div>
  );
}
