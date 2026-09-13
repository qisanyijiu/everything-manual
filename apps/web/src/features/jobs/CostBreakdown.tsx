/**
 * 费用区块（PRD §6.2 UI-027 / REQ-023；UI-033 的清单点）。
 *
 * 规则：
 * - Tripo **credits** 与说明书 AI **USD** 分列显示，**不相加**成无单位数字
 *   （金额来自服务端整数最小单位，`reservedDisplay` 已带单位；前端不换算、不浮点累加）；
 * - 显示已消耗（settled）、预留中（reserved）、**unknown 仍保留的预留**
 *   （写作"未决预留（等待对账）"，**不显示成 0**）；
 * - 常驻说明服务端 `budgetNotice`：本应用发起上限，不是供应商账户级封顶；
 * - 读取失败时不显示 0（由调用方决定展示错误态，本组件只在拿到数据时渲染）。
 */

import type { ReservationDto } from "../../api/endpoints";

const PROVIDER_LABELS: Record<string, string> = {
  tripo: "Tripo（credits）",
  manual_ai: "说明书 AI（USD）",
};

const STATE_LABELS: Record<string, string> = {
  reserved: "预留中（已占用预算）",
  settled: "已结算（按供应商计费事实）",
  released: "已释放（未计费）",
  unknown: "未决预留（等待对账）",
};

/** 该状态是否属于"已消耗"（拿到计费事实并结算）。 */
function isConsumed(state: string): boolean {
  return state === "settled";
}

export function costStateLabel(state: string): string {
  return STATE_LABELS[state] ?? `未知预留状态（${state}）`;
}

export function providerLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider;
}

export function CostBreakdown({
  reservations,
  budgetNotice,
  title = "费用",
}: {
  reservations: readonly ReservationDto[];
  /** 服务端固定文案（预算语义；缺省时不渲染说明，绝不自行编造）。 */
  budgetNotice: string | null;
  title?: string;
}) {
  const providers = [...new Set(reservations.map((entry) => entry.provider))];
  return (
    <section className="cost-breakdown" aria-labelledby="cost-breakdown-title" data-testid="cost-breakdown">
      <h2 id="cost-breakdown-title" className="summary-panel__title">
        {title}
      </h2>
      {reservations.length === 0 && (
        <p className="empty-note" data-testid="cost-empty">
          该任务没有费用记录（未产生付费提交）。
        </p>
      )}
      {providers.map((provider) => {
        const entries = reservations.filter((entry) => entry.provider === provider);
        return (
          <div key={provider} className="cost-breakdown__provider" data-testid={`cost-provider-${provider}`}>
            <h3>{providerLabel(provider)}</h3>
            <dl className="amount-list">
              {entries.map((entry) => (
                <div key={`${entry.provider}-${entry.state}`}>
                  <dt>{costStateLabel(entry.state)}</dt>
                  <dd>
                    <span
                      data-testid={`cost-amount-${entry.provider}-${entry.state}`}
                      className={entry.state === "unknown" ? "amount-list__unknown" : undefined}
                    >
                      {entry.reservedDisplay}
                    </span>
                    {isConsumed(entry.state) && <span className="amount-list__label">已消耗</span>}
                  </dd>
                </div>
              ))}
            </dl>
          </div>
        );
      })}
      <p className="field__hint" data-testid="budget-notice">
        {budgetNotice ??
          "本应用以本次授权的估算为发起上限；金额分列显示，不相加、不换算成同一币种。"}
      </p>
    </section>
  );
}
