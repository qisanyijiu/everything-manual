import type { ReactNode } from "react";

/** 空态（PRD §6.2 UI-005 等）：标题 + 说明 + 主操作入口。 */
export function EmptyState({
  title,
  description,
  action,
}: {
  title: string;
  description?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <h2 className="empty-state__title">{title}</h2>
      {description !== undefined && <p className="empty-state__description">{description}</p>}
      {action !== undefined && <div className="empty-state__action">{action}</div>}
    </div>
  );
}

/** 区块级「暂无内容」说明（不占用 h1/h2 主标题槽位时的轻量版本）。 */
export function EmptyNote({ children }: { children: ReactNode }) {
  return <p className="empty-note">{children}</p>;
}
