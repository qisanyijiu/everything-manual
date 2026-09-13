/**
 * 页面布局骨架（PRD §6.1.1 / §6.1.3 / §6.1.4）：
 * - `wide ≥1280px`：主栏 + 左栏（rail，可选 280–320px）+ 右栏（aside，可选 360–420px）并排；
 * - `mid 768–1279px`：主栏 + **单个可折叠侧栏**（多个面板时用标签页切换，不并排第三栏）；
 * - `narrow <768px`：单栏 + 抽屉（每个面板一个触发按钮，同一时刻只开一个抽屉）。
 *
 * 同一 URL 在三种断点下是同一个页面、同一份数据，只有布局与可用操作不同；
 * 布局切换由 CSS 媒体查询与 [`useBreakpoint`] 共同驱动，两者使用同一断点。
 */

import { useCallback, useRef, useState, type ReactNode } from "react";

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
}

export function PageLayout({ children, rail, aside }: PageLayoutProps) {
  const breakpoint = useBreakpoint();
  const panels: PanelSpec[] = [];
  if (rail !== undefined) {
    panels.push(rail);
  }
  if (aside !== undefined) {
    panels.push(aside);
  }

  if (panels.length === 0) {
    return <div className="page-layout page-layout--single">{children}</div>;
  }

  if (breakpoint === "wide") {
    return (
      <div className={`page-layout page-layout--wide${rail === undefined ? " page-layout--no-rail" : ""}`}>
        {rail !== undefined && (
          <aside className="page-layout__rail" aria-label={rail.label}>
            {rail.content}
          </aside>
        )}
        <div className="page-layout__main">{children}</div>
        {aside !== undefined && (
          <aside className="page-layout__aside" aria-label={aside.label}>
            {aside.content}
          </aside>
        )}
      </div>
    );
  }

  if (breakpoint === "mid") {
    return <MidLayout panels={panels} asideLabel={aside?.label ?? rail?.label ?? "侧栏"}>{children}</MidLayout>;
  }

  return <NarrowLayout panels={panels}>{children}</NarrowLayout>;
}

function MidLayout({
  panels,
  asideLabel,
  children,
}: {
  panels: PanelSpec[];
  asideLabel: string;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="page-layout page-layout--mid">
      <div className="page-layout__main">{children}</div>
      <div className="page-layout__panel-bar">
        <button
          type="button"
          aria-expanded={open}
          aria-controls="page-side-panel"
          onClick={() => setOpen((value) => !value)}
        >
          {open ? `隐藏${asideLabel}` : `显示${asideLabel}`}
        </button>
      </div>
      {open && (
        <aside className="page-layout__side-panel" id="page-side-panel" aria-label={asideLabel}>
          <PanelStack panels={panels} />
        </aside>
      )}
    </div>
  );
}

function NarrowLayout({ panels, children }: { panels: PanelSpec[]; children: ReactNode }) {
  const [openPanelId, setOpenPanelId] = useState<string | null>(null);
  const triggerRefs = useRef<Map<string, HTMLButtonElement>>(null);
  triggerRefs.current ??= new Map<string, HTMLButtonElement>();
  const triggers = triggerRefs.current;
  const activePanel = panels.find((panel) => panel.id === openPanelId) ?? null;
  // 稳定的回调：避免 Drawer 的焦点副作用因父组件重渲染而反复执行（焦点会来回跳）。
  const closeDrawer = useCallback(() => setOpenPanelId(null), []);
  const activePanelId = activePanel?.id ?? null;
  const returnFocus = useCallback(
    () => (activePanelId === null ? null : (triggers.get(activePanelId) ?? null)),
    [activePanelId, triggers],
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
            onClick={() => setOpenPanelId(panel.id)}
          >
            {panel.label}
          </button>
        ))}
      </div>
      <div className="page-layout__main">{children}</div>
      <Drawer
        open={activePanel !== null}
        onClose={closeDrawer}
        title={activePanel?.label ?? "侧栏"}
        returnFocus={returnFocus}
      >
        {activePanel?.content}
      </Drawer>
    </div>
  );
}

/** 面板堆叠：单个面板直接渲染；多个面板用标签页切换（mid 侧栏）。 */
function PanelStack({ panels }: { panels: PanelSpec[] }) {
  const [activeId, setActiveId] = useState(panels[0]?.id ?? "");
  const first = panels[0];
  if (first === undefined) {
    return null;
  }
  if (panels.length === 1) {
    return <>{first.content}</>;
  }
  const active = panels.find((panel) => panel.id === activeId) ?? first;
  return (
    <div className="panel-stack">
      <div role="tablist" aria-label="侧栏内容" className="panel-stack__tabs">
        {panels.map((panel) => (
          <button
            key={panel.id}
            type="button"
            role="tab"
            id={`panel-tab-${panel.id}`}
            aria-selected={panel.id === active.id}
            aria-controls={`panel-${panel.id}`}
            tabIndex={panel.id === active.id ? 0 : -1}
            onClick={() => setActiveId(panel.id)}
          >
            {panel.label}
          </button>
        ))}
      </div>
      <div role="tabpanel" id={`panel-${active.id}`} aria-labelledby={`panel-tab-${active.id}`}>
        {active.content}
      </div>
    </div>
  );
}
