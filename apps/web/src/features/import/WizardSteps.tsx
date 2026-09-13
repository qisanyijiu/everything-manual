/**
 * 向导五步导航（PRD §6.1.2 / §6.2 UI-019）。
 *
 * - 步骤条固定为 5 步：基本信息 → 说明书 → 视图排列 → 准备 → 预算/隐私确认；
 * - **URL 即步骤**：上一步/下一步只改 URL（`<Link>`），不携带状态、不触发保存，
 *   服务端已保存的资料在目标页按 URL 参数重新读取（刷新/返回都不丢资料）；
 * - 物品还不存在时（`/items/new`）后续步骤没有可导航的目标：渲染为不可用项并说明，
 *   不用死链接假装可点；
 * - 前置未满足时「下一步」保持可见但禁用，原因通过 `aria-describedby` 关联（可朗读）。
 */

import { Link, useParams } from "react-router";

export interface WizardStepSpec {
  readonly key: string;
  readonly label: string;
  readonly segment: string;
}

export const WIZARD_STEPS: readonly WizardStepSpec[] = [
  { key: "basic", label: "基本信息", segment: "edit" },
  { key: "document", label: "说明书", segment: "import/document" },
  { key: "views", label: "视图排列", segment: "import/views" },
  { key: "prepare", label: "准备", segment: "import/prepare" },
  { key: "confirm", label: "预算/隐私确认", segment: "import/confirm" },
];

export function wizardStepIndex(currentSegment: string): number {
  return WIZARD_STEPS.findIndex((step) => step.segment === currentSegment);
}

export function wizardStepHref(itemId: string, segment: string): string {
  return segment === "edit" ? `/items/${itemId}/edit` : `/items/${itemId}/${segment}`;
}

export interface WizardStepsProps {
  readonly currentSegment: string;
  /** 物品 id；缺省时从路由参数取（`/items/new` 上没有物品，用 `nullable` 语义）。 */
  readonly itemId?: string | null;
}

/** 五步步骤条：当前步 `aria-current="step"`；可导航步骤是链接（键盘可达）。 */
export function WizardSteps({ currentSegment, itemId }: WizardStepsProps) {
  const params = useParams();
  const effectiveItemId = itemId !== undefined ? itemId : (params.itemId ?? null);
  return (
    <ol className="wizard-steps" aria-label="新建向导步骤">
      {WIZARD_STEPS.map((step) => {
        const active = step.segment === currentSegment;
        return (
          <li
            key={step.key}
            className={
              active ? "wizard-steps__item wizard-steps__item--current" : "wizard-steps__item"
            }
          >
            {active ? (
              <span aria-current="step">{step.label}</span>
            ) : effectiveItemId === null || effectiveItemId === "" ? (
              <span className="wizard-steps__disabled">
                {step.label}
                <span className="visually-hidden">（创建物品后可进入）</span>
              </span>
            ) : (
              <Link to={wizardStepHref(effectiveItemId, step.segment)}>{step.label}</Link>
            )}
          </li>
        );
      })}
    </ol>
  );
}

export interface WizardNavProps {
  readonly currentSegment: string;
  readonly itemId: string;
  /** 下一步是否禁用（前置未满足时）。 */
  readonly nextDisabled?: boolean;
  /** 下一步被禁用的原因（常驻可见 + 与按钮 `aria-describedby` 关联）。 */
  readonly nextDisabledReason?: string | null;
  readonly nextLabel?: string;
}

/**
 * 上一步 / 下一步：**只改 URL** 的导航（不丢服务端已保存的资料）。
 *
 * 第一步没有「上一步」；最后一步没有「下一步」（生成动作属于页面自身，不属于导航）。
 */
export function WizardNav({
  currentSegment,
  itemId,
  nextDisabled = false,
  nextDisabledReason = null,
  nextLabel,
}: WizardNavProps) {
  const index = wizardStepIndex(currentSegment);
  const previous = index > 0 ? WIZARD_STEPS[index - 1] : undefined;
  const next = index >= 0 && index < WIZARD_STEPS.length - 1 ? WIZARD_STEPS[index + 1] : undefined;
  const reasonId = "wizard-next-reason";

  return (
    <div className="wizard-nav">
      {previous !== undefined ? (
        <Link className="button" to={wizardStepHref(itemId, previous.segment)}>
          上一步：{previous.label}
        </Link>
      ) : (
        <span />
      )}
      {next !== undefined && (
        <span className="wizard-nav__next">
          {nextDisabled && nextDisabledReason !== null ? (
            <button type="button" className="button-primary" disabled aria-describedby={reasonId}>
              下一步：{nextLabel ?? next.label}
            </button>
          ) : (
            <Link className="button-primary" to={wizardStepHref(itemId, next.segment)}>
              下一步：{nextLabel ?? next.label}
            </Link>
          )}
          {nextDisabled && nextDisabledReason !== null && (
            <span className="wizard-nav__reason" id={reasonId}>
              {nextDisabledReason}
            </span>
          )}
        </span>
      )}
    </div>
  );
}
