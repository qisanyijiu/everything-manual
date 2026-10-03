/**
 * 交互层状态（姿势 / 动作 / 高亮）——结果页、复核页、阅读器共用。
 *
 * 返回值可直接展开给 `ViewerPanel.interactive` 与 `InteractionPanel`；没有交互层时为 null，
 * 调用方按原有方式渲染整体模型（不受影响）。
 */

import { useCallback, useMemo, useState } from "react";

import type { InteractiveView, ModelActionView } from "./interactive";

export interface InteractiveController {
  readonly view: InteractiveView;
  readonly panel: {
    readonly actions: InteractiveView["actions"];
    readonly poses: InteractiveView["poses"];
    readonly activeToggles: ReadonlySet<string>;
    readonly poseId: string | null;
    readonly onAction: (action: ModelActionView) => void;
    readonly onPose: (poseId: string | null) => void;
    readonly onReset: () => void;
  };
  readonly viewerProp: {
    readonly partsAssetId: string;
    readonly partsSha256: string;
    readonly stage: {
      readonly poses: InteractiveView["poses"];
      readonly poseId: string | null;
      readonly actionRequest: { readonly action: ModelActionView; readonly nonce: number } | null;
      readonly highlightNodes: readonly string[];
      readonly resetNonce: number;
      readonly partNodes: ReadonlyMap<string, string>;
    };
  };
  /** 部件被选中时可触发的动作（点击热点/部件时提示）。 */
  readonly actionsForPart: (partId: string | null) => readonly ModelActionView[];
}

export function useInteractive(view: InteractiveView | null, selectedPartId: string | null): InteractiveController | null {
  const [poseId, setPoseId] = useState<string | null>(null);
  const [actionRequest, setActionRequest] = useState<{ action: ModelActionView; nonce: number } | null>(null);
  const [toggles, setToggles] = useState<ReadonlySet<string>>(new Set());
  const [resetNonce, setResetNonce] = useState(0);

  const onAction = useCallback((action: ModelActionView) => {
    setActionRequest((current) => ({ action, nonce: (current?.nonce ?? 0) + 1 }));
    if (action.mode === "toggle") {
      setToggles((current) => {
        const next = new Set(current);
        if (next.has(action.id)) {
          next.delete(action.id);
        } else {
          next.add(action.id);
        }
        return next;
      });
    }
  }, []);
  const onReset = useCallback(() => {
    setToggles(new Set());
    setPoseId(null);
    setResetNonce((value) => value + 1);
  }, []);

  const highlightNodes = useMemo(() => {
    if (view === null || selectedPartId === null) {
      return [];
    }
    return view.bindings.find((binding) => binding.partId === selectedPartId)?.nodes ?? [];
  }, [view, selectedPartId]);

  const partNodes = useMemo(
    () => new Map((view?.bindings ?? []).flatMap((binding) => (binding.nodes[0] ? [[binding.partId, binding.nodes[0]] as const] : []))),
    [view],
  );

  const actionsForPart = useCallback(
    (partId: string | null) =>
      view === null || partId === null ? [] : view.actions.filter((action) => action.triggerPartIds.includes(partId)),
    [view],
  );

  if (view === null) {
    return null;
  }
  return {
    view,
    panel: { actions: view.actions, poses: view.poses, activeToggles: toggles, poseId, onAction, onPose: setPoseId, onReset },
    viewerProp: {
      partsAssetId: view.partsModel.assetId,
      partsSha256: view.partsModel.sha256,
      stage: { poses: view.poses, poseId, actionRequest, highlightNodes, resetNonce, partNodes },
    },
    actionsForPart,
  };
}
