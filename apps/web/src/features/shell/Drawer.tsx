/**
 * 抽屉（PRD §6.1.5 / §6.2 UI-062）：
 * - 覆盖层，同一时刻只开一个（由调用方保证只有一个 Drawer 实例）；
 * - 打开时焦点移入抽屉，Tab/Shift+Tab 陷阱在面板内循环；
 * - Esc 关闭并把焦点归还触发按钮（`returnFocusRef`）。
 */

import { useEffect, useRef, type ReactNode } from "react";

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

export interface DrawerProps {
  readonly open: boolean;
  readonly onClose: () => void;
  readonly title: string;
  readonly children: ReactNode;
  /** 关闭时把焦点归还给它返回的元素（通常是打开抽屉的触发按钮）。 */
  readonly returnFocus?: () => HTMLElement | null;
}

export function Drawer({ open, onClose, title, children, returnFocus }: DrawerProps) {
  const panelRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    const panel = panelRef.current;
    if (panel === null) {
      return;
    }
    const activeBeforeOpen = document.activeElement;

    const focusable = (): HTMLElement[] =>
      Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
        (element) => element.tabIndex !== -1,
      );

    const first = focusable()[0];
    (first ?? panel).focus();

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const items = focusable();
      if (items.length === 0) {
        event.preventDefault();
        panel.focus();
        return;
      }
      const firstItem = items[0];
      const lastItem = items[items.length - 1];
      if (firstItem === undefined || lastItem === undefined) {
        return;
      }
      const active = document.activeElement;
      if (event.shiftKey && (active === firstItem || !panel.contains(active))) {
        event.preventDefault();
        lastItem.focus();
      } else if (!event.shiftKey && (active === lastItem || !panel.contains(active))) {
        event.preventDefault();
        firstItem.focus();
      }
    };

    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      const target =
        returnFocus?.() ??
        (activeBeforeOpen instanceof HTMLElement ? activeBeforeOpen : null);
      target?.focus();
    };
  }, [open, onClose, returnFocus]);

  if (!open) {
    return null;
  }

  return (
    <div className="drawer-overlay">
      <div
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        ref={panelRef}
      >
        <div className="drawer__header">
          <h2 className="drawer__title">{title}</h2>
          <button type="button" className="drawer__close" onClick={onClose}>
            关闭
          </button>
        </div>
        <div className="drawer__body">{children}</div>
      </div>
    </div>
  );
}
