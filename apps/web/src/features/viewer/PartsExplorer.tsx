import { useEffect, useId, useMemo, useRef, useState } from "react";
import { partNodeLabel } from "./interactive-view";
import "./parts-explorer.css";

export interface PartsExplorerProps {
  readonly nodeNames: readonly string[];
  readonly selectedNode: string | null;
  readonly expandedNodes: ReadonlySet<string>;
  readonly onSelectNode: (name: string) => void;
  readonly onToggleNode: (name: string) => void;
  readonly onExpandAll: () => void;
  readonly onRestoreParts: () => void;
}

export function PartsExplorer({ nodeNames, selectedNode, expandedNodes, onSelectNode, onToggleNode, onExpandAll, onRestoreParts }: PartsExplorerProps) {
  const [query, setQuery] = useState("");
  const rootRef = useRef<HTMLElement | null>(null);
  const fullscreenTarget = useRef<HTMLElement | null>(null);
  const fullscreenButton = useRef<HTMLButtonElement | null>(null);
  const [canFullscreen, setCanFullscreen] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const [fullscreenPending, setFullscreenPending] = useState(false);
  const [fullscreenError, setFullscreenError] = useState<string | null>(null);
  const queryId = useId();
  useEffect(() => {
    const main = rootRef.current?.closest<HTMLElement>(".page-layout__main") ?? null;
    const panel = rootRef.current?.closest(".interaction-panel");
    // 当前主栏必须直接同时包含原模型和交互面板。结果页嵌套容器不进入此模式。
    const target = main?.querySelector(":scope > .viewer-panel") && panel?.parentElement === main ? main : null;
    fullscreenTarget.current = target;
    setCanFullscreen(target !== null && typeof target.requestFullscreen === "function" && typeof document.exitFullscreen === "function" && document.fullscreenEnabled !== false);
    let wasFullscreen = false;
    const onChange = (): void => {
      const active = target !== null && document.fullscreenElement === target;
      setFullscreen(active);
      if (!active) target?.classList.remove("parts-observation-fullscreen");
      if (wasFullscreen && !active) {
        setFullscreenError(null);
        fullscreenButton.current?.focus({ preventScroll: true });
      }
      wasFullscreen = active;
    };
    const onEscape = async (event: KeyboardEvent): Promise<void> => {
      if (event.key !== "Escape" || target === null || document.fullscreenElement !== target) return;
      try {
        await document.exitFullscreen();
      } catch {
        // 浏览器原生 Esc 也可能同时完成退出；不为已退出或其他全屏显示错误。
        if (document.fullscreenElement === target) setFullscreenError("无法退出全屏，请使用退出全屏按钮重试。");
      }
    };
    document.addEventListener("fullscreenchange", onChange);
    document.addEventListener("keydown", onEscape);
    onChange();
    return () => {
      document.removeEventListener("fullscreenchange", onChange);
      document.removeEventListener("keydown", onEscape);
      target?.classList.remove("parts-observation-fullscreen");
      if (target !== null && document.fullscreenElement === target) void document.exitFullscreen().catch(() => undefined);
      fullscreenTarget.current = null;
    };
  }, [nodeNames.length]);
  const toggleFullscreen = async (): Promise<void> => {
    const target = fullscreenTarget.current;
    if (target === null || fullscreenPending) return;
    setFullscreenError(null);
    setFullscreenPending(true);
    const exiting = document.fullscreenElement === target;
    try {
      if (exiting) {
        await document.exitFullscreen();
      } else {
        target.classList.add("parts-observation-fullscreen");
        // 紧随按钮 user gesture 调用，保留同一 DOM、Canvas 和动画状态。
        await target.requestFullscreen();
      }
    } catch {
      if (document.fullscreenElement !== target) target.classList.remove("parts-observation-fullscreen");
      setFullscreenError(exiting ? "无法退出全屏，请按 Esc 退出。" : "暂时无法进入全屏，可直接在当前页面继续观察分件。");
    } finally {
      setFullscreenPending(false);
    }
  };
  const visible = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return nodeNames.filter((name) => `${name} ${partNodeLabel(name)}`.toLocaleLowerCase().includes(normalized));
  }, [nodeNames, query]);
  if (nodeNames.length === 0) return null;
  return (
    <section ref={rootRef} className="parts-explorer" aria-label="全部编号分件" data-testid="parts-explorer">
      <div className="parts-explorer__heading">
        <strong>分件观察 <span>{nodeNames.length} 件</span></strong>
        <span role="status" data-testid="parts-expanded-count">已展开 {expandedNodes.size} / {nodeNames.length}</span>
      </div>
      <p className="field__hint">每个编号分件均可选中、展开和复原。编号是模型几何名称，尚未全部对应说明书部件；展开方向用于观察外观。</p>
      {canFullscreen && <div className="parts-explorer__fullscreen">
        <button ref={fullscreenButton} type="button" onClick={() => void toggleFullscreen()} disabled={fullscreenPending} aria-pressed={fullscreen} data-testid="parts-fullscreen">{fullscreen ? "退出全屏" : "全屏观察"}</button>
        <span className="field__hint">{fullscreen ? "模型和分件并排显示，按 Esc 也可退出。" : "将模型与分件放在同一屏幕中观察。"}</span>
      </div>}
      {fullscreenError !== null && <p className="field__error" role="alert">{fullscreenError}</p>}
      <div className="parts-explorer__tools">
        <button type="button" onClick={onExpandAll} disabled={expandedNodes.size === nodeNames.length} data-testid="parts-expand-all">展开全部</button>
        <button type="button" onClick={onRestoreParts} disabled={expandedNodes.size === 0} data-testid="parts-restore-all">复原全部分件</button>
      </div>
      <label htmlFor={queryId}>查找编号分件</label>
      <input id={queryId} type="search" value={query} placeholder="输入编号或节点名称" onChange={(event) => setQuery(event.target.value)} />
      <div className="parts-explorer__list" role="group" aria-label="编号分件列表">
        {visible.map((name) => (
          <button type="button" className="parts-explorer__node" key={name} aria-pressed={selectedNode === name} onClick={() => onSelectNode(name)} data-node-name={name} data-testid={`inspect-node-${name}`}>
            <span>{partNodeLabel(name)}</span><small>{expandedNodes.has(name) ? "已展开" : "已合拢"}</small>
          </button>
        ))}
        {visible.length === 0 && <p className="field__hint">没有匹配的分件。</p>}
      </div>
      {selectedNode !== null ? (
        <div className="parts-explorer__selection" data-testid="part-inspection-selection">
          <div><strong>{partNodeLabel(selectedNode)}</strong><code>{selectedNode}</code></div>
          <button type="button" aria-pressed={expandedNodes.has(selectedNode)} onClick={() => onToggleNode(selectedNode)} data-testid="part-inspection-toggle">{expandedNodes.has(selectedNode) ? "复原此分件" : "展开此分件"}</button>
        </div>
      ) : <p className="field__hint">在列表或模型上选择分件，查看高亮并独立展开。</p>}
    </section>
  );
}
