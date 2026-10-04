/** Lightweight interactive data parser and view types; no renderer or Three.js dependency. */

export type Vec3Tuple = readonly [number, number, number];

export interface TransformStepView {
  readonly nodes: readonly string[];
  readonly kind: "translate" | "rotate";
  readonly vector?: Vec3Tuple | null;
  readonly pivot?: Vec3Tuple | null;
  readonly axis?: Vec3Tuple | null;
  readonly angleDeg?: number | null;
}

export interface ModelActionView {
  readonly id: string;
  readonly label: string;
  readonly description: string | null;
  readonly triggerPartIds: readonly string[];
  readonly mode: "toggle" | "pulse";
  readonly durationMs: number;
  readonly steps: readonly TransformStepView[];
  readonly stepIds: readonly string[];
}

export interface ModelPoseView {
  readonly id: string;
  readonly label: string;
  readonly description: string | null;
  readonly durationMs: number;
  readonly steps: readonly TransformStepView[];
}

export interface PartBindingView {
  readonly partId: string;
  readonly nodes: readonly string[];
  readonly status: "auto" | "confirmed";
}

export interface InteractiveView {
  readonly partsModel: {
    readonly assetId: string;
    readonly sha256: string;
    readonly modelRevisionId: string;
    readonly modelSha256: string;
    readonly nodeNames: readonly string[];
  };
  readonly bindings: readonly PartBindingView[];
  readonly actions: readonly ModelActionView[];
  readonly poses: readonly ModelPoseView[];
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}
function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}
function asStrings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}
function asVec3(value: unknown): Vec3Tuple | null {
  return Array.isArray(value) && value.length === 3 && value.every((v) => typeof v === "number" && Number.isFinite(v))
    ? [value[0] as number, value[1] as number, value[2] as number]
    : null;
}
function readSteps(value: unknown): TransformStepView[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.flatMap((raw) => {
    const step = asRecord(raw);
    const kind = step?.kind;
    if (step === null || (kind !== "translate" && kind !== "rotate")) {
      return [];
    }
    return [
      {
        nodes: asStrings(step.nodes),
        kind,
        vector: asVec3(step.vector),
        pivot: asVec3(step.pivot),
        axis: asVec3(step.axis),
        angleDeg: typeof step.angleDeg === "number" ? step.angleDeg : null,
      },
    ];
  });
}

/**
 * 读取草稿/发布知识外壳里的交互层；分件附件不属于当前模型版本时返回 null
 * （与服务端同一条判定：旧附件不得驱动新模型）。
 */
export function readInteractive(
  knowledge: unknown,
  model: { revisionId: string; sha256: string } | null,
): InteractiveView | null {
  const shell = asRecord(knowledge);
  const interactive = asRecord(shell?.interactive);
  const parts = asRecord(interactive?.partsModel);
  if (interactive === null || parts === null || model === null) {
    return null;
  }
  const assetId = asString(parts.assetId);
  const sha256 = asString(parts.sha256);
  if (
    assetId === null ||
    sha256 === null ||
    parts.modelRevisionId !== model.revisionId ||
    parts.modelSha256 !== model.sha256
  ) {
    return null;
  }
  const bindings = (Array.isArray(interactive.bindings) ? interactive.bindings : []).flatMap((raw) => {
    const binding = asRecord(raw);
    const partId = asString(binding?.partId);
    return binding === null || partId === null
      ? []
      : [{ partId, nodes: asStrings(binding.nodes), status: binding.status === "confirmed" ? ("confirmed" as const) : ("auto" as const) }];
  });
  const actions = (Array.isArray(interactive.actions) ? interactive.actions : []).flatMap((raw) => {
    const action = asRecord(raw);
    const id = asString(action?.id);
    const label = asString(action?.label);
    if (action === null || id === null || label === null) {
      return [];
    }
    return [
      {
        id,
        label,
        description: asString(action.description),
        triggerPartIds: asStrings(action.triggerPartIds),
        mode: action.mode === "pulse" ? ("pulse" as const) : ("toggle" as const),
        durationMs: typeof action.durationMs === "number" ? action.durationMs : 600,
        steps: readSteps(action.steps),
        stepIds: asStrings(action.stepIds),
      },
    ];
  });
  const poses = (Array.isArray(interactive.poses) ? interactive.poses : []).flatMap((raw) => {
    const pose = asRecord(raw);
    const id = asString(pose?.id);
    const label = asString(pose?.label);
    return pose === null || id === null || label === null
      ? []
      : [
          {
            id,
            label,
            description: asString(pose.description),
            durationMs: typeof pose.durationMs === "number" ? pose.durationMs : 800,
            steps: readSteps(pose.steps),
          },
        ];
  });
  return {
    partsModel: { assetId, sha256, modelRevisionId: model.revisionId, modelSha256: model.sha256, nodeNames: asStrings(parts.nodeNames) },
    bindings,
    actions,
    poses,
  };
}

