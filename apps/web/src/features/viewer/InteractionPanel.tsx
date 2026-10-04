/**
 * 交互面板（ADR-042）：动作按钮（例如"取下电池盖"）与整体姿势（例如机器人"趴下/坐下"）。
 *
 * - 选中部件时只突出与该部件相关的动作（触发部件）；其余动作仍可用；
 * - 键盘可达：全部为原生按钮；切换类动作用 `aria-pressed` 表达当前状态；
 * - 文案如实：动作只是外观示意，不代表真实机械结构或操作力度。
 */

import { useMemo } from "react";

import type { ModelActionView, ModelPoseView } from "./interactive-view";

export interface InteractionPanelProps {
  readonly actions: readonly ModelActionView[];
  readonly poses: readonly ModelPoseView[];
  readonly selectedPartId: string | null;
  readonly activeToggles: ReadonlySet<string>;
  readonly poseId: string | null;
  readonly onAction: (action: ModelActionView) => void;
  readonly onPose: (poseId: string | null) => void;
  readonly onReset: () => void;
}

export function InteractionPanel({
  actions,
  poses,
  selectedPartId,
  activeToggles,
  poseId,
  onAction,
  onPose,
  onReset,
}: InteractionPanelProps) {
  const ordered = useMemo(() => {
    if (selectedPartId === null) {
      return actions;
    }
    const related = actions.filter((action) => action.triggerPartIds.includes(selectedPartId));
    const rest = actions.filter((action) => !action.triggerPartIds.includes(selectedPartId));
    return [...related, ...rest];
  }, [actions, selectedPartId]);

  if (actions.length === 0 && poses.length === 0) {
    return null;
  }
  return (
    <section className="interaction-panel" aria-label="模型交互" data-testid="interaction-panel">
      {poses.length > 0 && (
        <div className="interaction-panel__group" role="group" aria-label="姿势">
          <span className="interaction-panel__label">姿势</span>
          {poses.map((pose) => (
            <button
              key={pose.id}
              type="button"
              className="interaction-chip"
              aria-pressed={poseId === pose.id || (poseId === null && pose.id === poses[0]?.id)}
              title={pose.description ?? undefined}
              onClick={() => onPose(pose.id)}
              data-testid={`pose-${pose.id}`}
            >
              {pose.label}
            </button>
          ))}
        </div>
      )}
      {ordered.length > 0 && (
        <div className="interaction-panel__group" role="group" aria-label="动作">
          <span className="interaction-panel__label">动作</span>
          {ordered.map((action) => {
            const related = selectedPartId !== null && action.triggerPartIds.includes(selectedPartId);
            return (
              <button
                key={action.id}
                type="button"
                className={`interaction-chip${related ? " interaction-chip--related" : ""}`}
                aria-pressed={action.mode === "toggle" ? activeToggles.has(action.id) : undefined}
                title={action.description ?? undefined}
                onClick={() => onAction(action)}
                data-testid={`action-${action.id}`}
              >
                {action.label}
              </button>
            );
          })}
          <button type="button" className="interaction-chip" onClick={onReset} data-testid="interaction-reset">
            复原
          </button>
        </div>
      )}
      <p className="field__hint">动作与姿势为外观示意（由分件模型做刚体变换），不代表真实机械结构。</p>
    </section>
  );
}
