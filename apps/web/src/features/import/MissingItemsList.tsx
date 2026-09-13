/**
 * 生成前缺项列表（PRD §6.2 UI-013 / UI-023）。
 *
 * 缺项**常驻可见**（不是一次性提示）：每条带「去修复」链接与稳定 `code`
 * （与服务端 estimate 的 `details.items[].code` 同一词表）。
 * 生成按钮在缺项未解决前保持禁用并说明缺什么（本组件与其 `aria-describedby` 关联）。
 */

import { Link } from "react-router";

import type { MissingItem } from "./views";

export function MissingItemsList({
  title,
  gaps,
  emptyText,
  testId = "generation-gaps",
}: {
  readonly title: string;
  readonly gaps: readonly MissingItem[];
  readonly emptyText: string;
  readonly testId?: string;
}) {
  if (gaps.length === 0) {
    return (
      <p className="missing-list missing-list--ok" data-testid={testId} role="status">
        {emptyText}
      </p>
    );
  }
  return (
    <div className="missing-list" data-testid={testId}>
      <h2 className="missing-list__title">{title}</h2>
      <ul className="missing-list__items" role="alert">
        {gaps.map((gap) => (
          <li key={gap.code}>
            <span className="missing-list__message">{gap.message}</span>
            {gap.actionHref !== null && gap.actionLabel !== null && (
              <Link to={gap.actionHref}>{gap.actionLabel}</Link>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
