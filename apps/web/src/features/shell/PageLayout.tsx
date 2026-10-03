/**
 * 页面布局骨架（PRD §6.1.1 / §6.1.3 / §6.1.4）：
 * - `wide ≥1280px`：主栏 + 左栏（rail，可选 280–320px）+ 右栏（aside，可选 360–420px）并排；
 * - `mid 768–1279px`：主栏 + **单个可折叠侧栏**（多个面板时用标签页切换，不并排第三栏）；
 * - `narrow <768px`：单栏 + 抽屉（每个面板一个触发按钮，同一时刻只开一个抽屉）。
 *
 * 同一 URL 在三种断点下是同一个页面、同一份数据，只有布局与可用操作不同；
 * 布局切换由 CSS 媒体查询与 [`useBreakpoint`] 共同驱动，两者使用同一断点。
 */

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { Drawer } from "./Drawer";
import { useBreakpoint } from "./useBreakpoint";

export interface PanelSpec {
  readonly id: string;
  readonly label: string;
  readonly content: ReactNode;
}

export interface PageLayoutProps {
  readonly children: ReactNode;
  /** 左栏（校准工作区/阅读器的部件列表）；窄屏折叠进抽屉。 */
  readonly rail?: PanelSpec;
  /** 右栏（摘要、步骤、原文等）。 */
  readonly aside?: PanelSpec;
  readonly original?: PanelSpec;
  readonly navigation?: PanelNavigation | null;
  readonly onOriginalClose?: () => void;
}

export interface PanelNavigation {
  readonly panelId: string;
  readonly focusId: string;
  readonly narrowFocusId?: string;
  readonly serial: number;
}

export function PageLayout({ children, rail, aside, original, navigation, onOriginalClose }: PageLayoutProps) {
  const breakpoint = useBreakpoint();
  const [openPanelId, setOpenPanelId] = useState<string | null>(null);
  const focusedNavigation = useRef<{ serial: number; breakpoint: string } | null>(null);
  const panels: PanelSpec[] = [];
  if (rail !== undefined) {
    panels.push(rail);
  }
  if (aside !== undefined) {
    panels.push(aside);
  }

  if (original !== undefined) panels.push(original);
  useEffect(() => {
    if (navigation !== undefined && navigation !== null) setOpenPanelId(navigation.panelId === "main" ? null : navigation.panelId);
  }, [navigation]);
  useEffect(() => {
    if (navigation === undefined || navigation === null || (breakpoint !== "wide" && openPanelId !== (navigation.panelId === "main" ? null : navigation.panelId))) return;
    if (focusedNavigation.current?.serial === navigation.serial && focusedNavigation.current.breakpoint === breakpoint) return;
    const frame = requestAnimationFrame(() => {
      const requested = document.getElementById(breakpoint === "narrow" ? navigation.narrowFocusId ?? navigation.focusId : navigation.focusId);
      const target = requested?.matches(":disabled")
        ? requested.closest<HTMLElement>("[tabindex]") ?? document.getElementById(`panel-${navigation.panelId}`)
        : requested ?? document.getElementById(`panel-${navigation.panelId}`);
      if (target !== null) {
        target.focus();
        target.scrollIntoView?.({ block: "nearest" });
        focusedNavigation.current = { serial: navigation.serial, breakpoint };
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [navigation, breakpoint, openPanelId]);
  const closePanel = useCallback(() => {
    if (openPanelId === "original" && onOriginalClose !== undefined) onOriginalClose();
    else setOpenPanelId(null);
  }, [openPanelId, onOriginalClose]);

  if (panels.length === 0) {
    return <div className="page-layout page-layout--single">{children}</div>;
  }

  if (breakpoint === "wide") {
    return (
      <div className={`page-layout page-layout--wide${rail === undefined ? " page-layout--no-rail" : ""}`}>
        {rail !== undefined && (
          <aside id={`panel-${rail.id}`} tabIndex={-1} className="page-layout__rail" aria-label={rail.label}>
            {rail.content}
          </aside>
        )}
        <div id="panel-main" tabIndex={-1} className="page-layout__main">{children}</div>
        {(aside !== undefined || original !== undefined) && (
          <aside className="page-layout__aside" aria-label={aside?.label ?? original?.label}>
            {aside !== undefined && <div id={`panel-${aside.id}`} tabIndex={-1}>{aside.content}</div>}
            {original !== undefined && <div id={`panel-${original.id}`} tabIndex={-1}>{original.content}</div>}
          </aside>
        )}
      </div>
    );
  }

  if (breakpoint === "mid") {
    return <MidLayout panels={panels} activeId={openPanelId} onSelect={setOpenPanelId} asideLabel={aside?.label ?? rail?.label ?? "侧栏"}>{children}</MidLayout>;
  }

  return <NarrowLayout panels={panels} openPanelId={openPanelId} onSelect={setOpenPanelId} onClose={closePanel}>{children}</NarrowLayout>;
}

function MidLayout({
  panels,
  asideLabel,
  activeId,
  onSelect,
  children,
}: {
  panels: PanelSpec[];
  asideLabel: string;
  activeId: string | null;
  onSelect: (id: string | null) => void;
  children: ReactNode;
}) {
  const open = activeId !== null;
  return (
    <div className="page-layout page-layout--mid">
      <div id="panel-main" tabIndex={-1} className="page-layout__main">{children}</div>
      <div className="page-layout__panel-bar">
        <button
          type="button"
          aria-expanded={open}
          aria-controls="page-side-panel"
          onClick={() => onSelect(open ? null : (panels[0]?.id ?? null))}
        >
          {open ? `隐藏${asideLabel}` : `显示${asideLabel}`}
        </button>
      </div>
      {open && (
        <aside className="page-layout__side-panel" id="page-side-panel" aria-label={asideLabel}>
          <PanelStack panels={panels} activeId={activeId ?? ""} onSelect={onSelect} />
        </aside>
      )}
    </div>
  );
}

function NarrowLayout({ panels, children, openPanelId, onSelect, onClose }: { panels: PanelSpec[]; children: ReactNode; openPanelId: string | null; onSelect: (id: string) => void; onClose: () => void }) {
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const closeDrawer = useCallback(() => closeRef.current(), []);
  const triggerRefs = useRef<Map<string, HTMLButtonElement>>(null);
  triggerRefs.current ??= new Map<string, HTMLButtonElement>();
  const triggers = triggerRefs.current;
  const activePanel = panels.find((panel) => panel.id === openPanelId) ?? null;
  // 稳定的回调：避免 Drawer 的焦点副作用因父组件重渲染而反复执行（焦点会来回跳）。
  const activePanelId = useRef<string | null>(null);
  activePanelId.current = activePanel?.id ?? activePanelId.current;
  const returnFocus = useCallback(
    () => (activePanelId.current === null ? null : (triggers.get(activePanelId.current) ?? null)),
    [triggers],
  );

  return (
    <div className="page-layout page-layout--narrow">
      <div className="page-layout__panel-bar">
        {panels.map((panel) => (
          <button
            key={panel.id}
            type="button"
            aria-haspopup="dialog"
            aria-expanded={openPanelId === panel.id}
            ref={(element) => {
              if (element === null) {
                triggers.delete(panel.id);
              } else {
                triggers.set(panel.id, element);
              }
            }}
            onClick={() => onSelect(panel.id)}
          >
            {panel.label}
          </button>
        ))}
      </div>
      <div id="panel-main" tabIndex={-1} className="page-layout__main">{children}</div>
      <Drawer
        open={activePanel !== null}
        onClose={closeDrawer}
        title={activePanel?.label ?? "侧栏"}
        returnFocus={returnFocus}
      >
        <div id={`panel-${activePanel?.id ?? "closed"}`} tabIndex={-1}>{activePanel?.content}</div>
      </Drawer>
    </div>
  );
}

/** 面板堆叠：单个面板直接渲染；多个面板用标签页切换（mid 侧栏）。 */
function PanelStack({ panels, activeId, onSelect }: { panels: PanelSpec[]; activeId: string; onSelect: (id: string) => void }) {
  const tabRefs = useRef(new Map<string, HTMLButtonElement>());
  const panelRef = useRef<HTMLDivElement>(null);
  const [hasFocusableContent, setHasFocusableContent] = useState(false);
  const active = panels.find((panel) => panel.id === activeId) ?? panels[0];
  useEffect(() => {
    const focusable = panelRef.current?.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]',
    );
    setHasFocusableContent(
      Array.from(focusable ?? []).some((element) => element.tabIndex >= 0 && !element.closest("[hidden]")),
    );
  }, [active]);
  const first = panels[0];
  if (first === undefined) {
    return null;
  }
  if (panels.length === 1) {
    return <>{first.content}</>;
  }
  const selected = active ?? first;
  return (
    <div className="panel-stack">
      <div role="tablist" aria-label="侧栏内容" className="panel-stack__tabs">
        {panels.map((panel) => (
          <button
            key={panel.id}
            type="button"
            role="tab"
            ref={(element) => {
              if (element === null) tabRefs.current.delete(panel.id);
              else tabRefs.current.set(panel.id, element);
            }}
            id={`panel-tab-${panel.id}`}
            aria-selected={panel.id === selected.id}
            aria-controls={`panel-${panel.id}`}
            tabIndex={panel.id === selected.id ? 0 : -1}
            onClick={() => onSelect(panel.id)}
            onKeyDown={(event) => {
              const index = panels.findIndex((candidate) => candidate.id === panel.id);
              const nextIndex =
                event.key === "ArrowRight" ? (index + 1) % panels.length
                  : event.key === "ArrowLeft" ? (index + panels.length - 1) % panels.length
                    : event.key === "Home" ? 0
                      : event.key === "End" ? panels.length - 1 : null;
              if (nextIndex === null) return;
              event.preventDefault();
              const next = panels[nextIndex];
              if (next === undefined) return;
              onSelect(next.id);
              tabRefs.current.get(next.id)?.focus();
            }}
          >
            {panel.label}
          </button>
        ))}
      </div>
      <div
        ref={panelRef}
        role="tabpanel"
        id={`panel-${selected.id}`}
        aria-labelledby={`panel-tab-${selected.id}`}
        tabIndex={hasFocusableContent ? -1 : 0}
      >
        {selected.content}
      </div>
    </div>
  );
}
