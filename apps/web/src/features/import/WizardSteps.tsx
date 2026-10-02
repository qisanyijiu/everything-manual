import { useEffect, useState } from "react";
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

import { Link, useParams, useSearchParams } from "react-router";
import { useItemSummaries } from "../library/workflow";
import type { ItemSummaryDto } from "../../api/endpoints";

export interface WizardStepSpec {
  readonly key: keyof ItemSummaryDto["steps"];
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

export function wizardStepHref(itemId: string, segment: string, context?: URLSearchParams): string {
  const kept = new URLSearchParams();
  for (const key of ["documentId", "preparationId", "quoteId"]) { const value = context?.get(key); if (value) kept.set(key, value); }
  return `/items/${itemId}/${segment}${kept.size ? `?${kept}` : ""}`;
}

export interface WizardStepsProps {
  readonly currentSegment: string;
  /** 物品 id；缺省时从路由参数取（`/items/new` 上没有物品，用 `nullable` 语义）。 */
  readonly itemId?: string | null;
}

/** 五步步骤条：当前步 `aria-current="step"`；可导航步骤是链接（键盘可达）。 */
export function WizardSteps({ currentSegment, itemId }: WizardStepsProps) {
  const params = useParams();
  const [search] = useSearchParams();
  const effectiveItemId = itemId !== undefined ? itemId : (params.itemId ?? null);
  const summaries = useItemSummaries(effectiveItemId ? [effectiveItemId] : [], search.get("documentId"));
  const [now, setNow] = useState(() => Date.now());
  const expiration = summaries.data?.[0]?.quoteExpiresAt;
  useEffect(() => { if (!expiration) return; const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer); }, [expiration]);
  const steps = summaries.isError ? undefined : summaries.data?.[0]?.steps;
  return (
    <><ol className="wizard-steps" aria-label="新建向导步骤">
      {WIZARD_STEPS.map((step) => {
        const active = step.segment === currentSegment;
        const state = !effectiveItemId ? "missing" : steps && step.key === "confirm" && expiration && now >= Date.parse(expiration) ? "needsReview" : steps?.[step.key];
        const label = state === "complete" ? "已完成" : state === "missing" ? "待补充" : state === "needsReview" ? "需重新检查" : "状态未读取";
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
              <Link to={wizardStepHref(effectiveItemId, step.segment, search)}>{step.label}</Link>
            )}
            <small className={`wizard-steps__state wizard-steps__state--${state ?? "unknown"}`}>{label}</small>
            {state === "needsReview" && <span className="visually-hidden">当前资料或报价条件已改变，请重新核对本步骤</span>}
          </li>
        );
      })}
    </ol>
    {summaries.isError && <p role="alert">步骤状态未读取。<button type="button" onClick={() => void summaries.refetch()}>重新读取步骤状态</button></p>}
    {steps && (Object.values(steps).includes("needsReview") || !!expiration && now >= Date.parse(expiration)) && <p className="field__hint">标记「需重新检查」的步骤受当前资料或报价有效期影响；请重新核对、获取报价并确认。已受理任务和已发布版仍保留。</p>}</>
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
  readonly nextHref?: string;
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
  nextHref,
}: WizardNavProps) {
  const [search] = useSearchParams();
  const index = wizardStepIndex(currentSegment);
  const previous = index > 0 ? WIZARD_STEPS[index - 1] : undefined;
  const next = index >= 0 && index < WIZARD_STEPS.length - 1 ? WIZARD_STEPS[index + 1] : undefined;
  const reasonId = "wizard-next-reason";

  return (
    <div className="wizard-nav">
      {previous !== undefined ? (
        <Link className="button" to={wizardStepHref(itemId, previous.segment, search)}>
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
            <Link className="button-primary" to={nextHref ?? wizardStepHref(itemId, next.segment, search)}>
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
