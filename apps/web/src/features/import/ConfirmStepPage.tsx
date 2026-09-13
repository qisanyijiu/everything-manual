/**
 * 向导第 5 步：预算与隐私确认（PRD §6.1.2 `/items/:itemId/import/confirm`；
 * §6.2 UI-004、UI-013、UI-019–UI-026）。
 *
 * 硬约束（QA 按此复核）：
 * - **分列金额**：Tripo credits 与 Manual AI USD 各自带单位展示，**不相加**成无单位数字（UI-022）；
 *   数值与价格版本/快照日期/有效期/保守上界全部取自服务端报价（前端不自行计算金额）；
 * - **告知与确认**（UI-024）：列出将发送给 Tripo 的视图与将发送给说明书 AI 的页范围/型号文本、
 *   模型名与参数、价格版本与预算上界；勾选框**默认不勾选**，勾选动作调用
 *   `POST .../estimates/{quoteId}/confirm` 写 audit_events（服务端不允许未确认提交）；
 * - **报价过期（U-05）**：不自动重新报价，用户点「重新获取报价」；过期后生成按钮禁用；
 * - **生成**（UI-020/UI-025）：一次操作生成一个 `Idempotency-Key` 并在重试中复用；
 *   点击后进入「提交中」并禁用；202 只表示已入队——前端**不判断远端成功**，
 *   进入等待状态；正确性依赖服务端幂等（同键重放返回同一 job）；
 * - **预算语义**：界面常驻服务端 `budgetNotice`（本应用发起上限，不是供应商账户级封顶）。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useParams } from "react-router";

import { describeError, isApiError } from "../../api/client";
import {
  confirmEstimate,
  createEstimate,
  createJob,
  fetchSettingsStatus,
  type JobDto,
  type QuoteDto,
} from "../../api/endpoints";
import { readReason } from "../../components/form";
import { Skeleton } from "../../components/Skeleton";
import { useNotify } from "../../components/notifications";
import { formatLocalDateTime } from "../../lib/format";
import { MissingItemsList } from "./MissingItemsList";
import { WizardSteps } from "./WizardSteps";
import { PageLayout } from "../shell/PageLayout";
import { itemKeys, useItemDetail, useItemDocuments, useItemPhotos } from "../library/items";
import { getPreparation, type PreparationDetail } from "./api";
import { recallPreparationId } from "./preparation-pointer";
import { CREDIT_MINOR_SCALE, USD_MICROS_SCALE, minorToInputString, parseMinorInput } from "./money";
import { tripoPhotoIds, generationGaps, VIEW_LABELS, type MissingItem, type ViewSlot } from "./views";

/**
 * 支持的模型预设。
 *
 * 背景（记录为限制）：`/settings/status` 目前不返回价格目录里的预设清单，
 * `POST /estimates` 又要求显式 `modelPreset`；T11 的实现只支持
 * `tripo-h-v3.1-standard`（与 `price-catalog.example.toml` 一致）。
 * 这里使用该唯一受支持预设；若部署者的价格目录不同，服务端会返回
 * 422 `modelPresetUnsupported` 并在 `details.supportedPresets` 给出实际清单，
 * 页面照实显示（不猜测、不回落）。
 */
const MODEL_PRESET = "tripo-h-v3.1-standard";

interface SubmissionError {
  readonly message: string;
  readonly hint: string | null;
  /** 已有任务时给出 job id（可链接到任务中心）。 */
  readonly existingJobId: string | null;
  /** 报价过期/输入变化时建议重新报价或回向导。 */
  readonly recovery: "requote" | "rewizard" | "none";
}

export function ConfirmStepPage() {
  const { itemId } = useParams();
  const id = itemId ?? "";
  const notify = useNotify();
  const queryClient = useQueryClient();

  const itemQuery = useItemDetail(id === "" ? null : id);
  const documentsQuery = useItemDocuments(id === "" ? null : id);
  const photosQuery = useItemPhotos(id === "" ? null : id);
  const statusQuery = useQuery({ queryKey: ["settings", "status"], queryFn: fetchSettingsStatus });

  const [preparationPointer] = useState<string | null>(() => recallPreparationId(id));
  const preparationQuery = useQuery({
    queryKey: itemKeys.preparation(id, preparationPointer),
    queryFn: () => getPreparation(preparationPointer ?? ""),
    enabled: preparationPointer !== null,
  });

  const [quote, setQuote] = useState<QuoteDto | null>(null);
  const [quoting, setQuoting] = useState(false);
  const [quoteError, setQuoteError] = useState<SubmissionError | null>(null);
  const [quoteGaps, setQuoteGaps] = useState<MissingItem[]>([]);

  const [tripoBudget, setTripoBudget] = useState("");
  const [usdBudget, setUsdBudget] = useState("");
  const [confirmedAt, setConfirmedAt] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [confirmChecked, setConfirmChecked] = useState(false);
  const [confirmError, setConfirmError] = useState<string | null>(null);

  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<SubmissionError | null>(null);
  const [acceptedJob, setAcceptedJob] = useState<JobDto | null>(null);
  /** 服务端已判定"该操作已存在任务"时锁住生成入口（UI-026：不新建第二份）。 */
  const [lockedJobId, setLockedJobId] = useState<string | null>(null);
  const idempotencyKeyRef = useRef<string | null>(null);
  const requestedRef = useRef<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const photos = useMemo(() => photosQuery.data?.photos ?? [], [photosQuery.data]);
  const photoIds = useMemo(() => tripoPhotoIds(photos), [photos]);
  const preparationState: string | null = prepareState({
    pointer: preparationPointer,
    detail: preparationQuery.data,
  });
  const capability = statusQuery.data?.data.capabilities.generation ?? null;

  const loading =
    itemQuery.isPending ||
    documentsQuery.isPending ||
    photosQuery.isPending ||
    statusQuery.isPending ||
    (preparationPointer !== null && preparationQuery.isPending);

  const gaps = loading
    ? []
    : generationGaps({ itemId: id, preparationState, photos, generationCapability: capability });
  const blocked = gaps.length > 0;

  // 报价倒计时：每秒重算剩余时间（过期后生成按钮禁用，需显式重新报价）。
  useEffect(() => {
    if (quote === null) {
      return;
    }
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [quote]);

  const requestQuote = useCallback(async (): Promise<void> => {
    if (preparationPointer === null) {
      return;
    }
    setQuoting(true);
    setQuoteError(null);
    setQuoteGaps([]);
    try {
      const resource = await createEstimate(id, {
        preparationId: preparationPointer,
        photoIds,
        modelPreset: MODEL_PRESET,
      });
      setQuote(resource.data);
      setTripoBudget(minorToInputString(resource.data.amounts.tripo.upperBoundMinor, CREDIT_MINOR_SCALE));
      setUsdBudget(
        minorToInputString(resource.data.amounts.manualAi.upperBoundMinor, USD_MICROS_SCALE),
      );
      setConfirmedAt(null);
      setConfirmChecked(false);
      setConfirmError(null);
      setSubmitError(null);
      idempotencyKeyRef.current = null;
      setNow(Date.now());
    } catch (error) {
      setQuote(null);
      const info = describeError(error);
      const reason = isApiError(error) ? readReason(error.details) : null;
      const code = isApiError(error) ? error.code : null;
      if (code === "PROVIDER_NOT_CONFIGURED" || code === "PRICE_CATALOG_MISSING") {
        setQuoteError({
          message:
            "生成能力未就绪：服务端缺少供应商密钥或价格目录（设置页只显示状态，密钥由部署者配置）。",
          hint: info.message,
          existingJobId: null,
          recovery: "none",
        });
      } else if (reason === "preconditionsFailed") {
        setQuoteGaps(readGapItems(error));
        setQuoteError({
          message: "报价前置条件未满足：请按下列明细补齐后重试。",
          hint: null,
          existingJobId: null,
          recovery: "none",
        });
      } else if (reason === "modelPresetUnsupported") {
        setQuoteError({
          message: `当前价格目录不支持内置的模型预设（${MODEL_PRESET}）：${readSupportedPresets(error)}`,
          hint: "请部署者核对 price_catalog_path 指向的价格目录；此处不猜测其它预设。",
          existingJobId: null,
          recovery: "none",
        });
      } else {
        setQuoteError({ message: info.message, hint: null, existingJobId: null, recovery: "none" });
      }
    } finally {
      setQuoting(false);
    }
  }, [id, photoIds, preparationPointer]);

  // 前置满足且尚无报价时自动获取一次（同一输入签名只请求一次，避免 StrictMode 双调用）。
  useEffect(() => {
    if (loading || blocked || quote !== null || quoting || quoteError !== null) {
      return;
    }
    const signature = `${preparationPointer ?? ""}|${photoIds.join(",")}`;
    if (requestedRef.current === signature) {
      return;
    }
    requestedRef.current = signature;
    void requestQuote();
  }, [blocked, loading, photoIds, preparationPointer, quote, quoteError, quoting, requestQuote]);

  const tripoParsed = parseMinorInput(tripoBudget, CREDIT_MINOR_SCALE);
  const usdParsed = parseMinorInput(usdBudget, USD_MICROS_SCALE);
  const expiresAtMs = quote === null ? null : Date.parse(quote.expiresAt);
  const expired = expiresAtMs !== null && Number.isFinite(expiresAtMs) && now >= expiresAtMs;
  const remainingSeconds =
    expiresAtMs !== null && Number.isFinite(expiresAtMs)
      ? Math.max(0, Math.round((expiresAtMs - now) / 1000))
      : null;
  const budgetBelow =
    quote !== null &&
    tripoParsed.ok &&
    usdParsed.ok &&
    (tripoParsed.minor < quote.amounts.tripo.upperBoundMinor ||
      usdParsed.minor < quote.amounts.manualAi.upperBoundMinor);
  const budgetInvalid = !tripoParsed.ok || !usdParsed.ok;
  const canGenerate =
    quote !== null &&
    !expired &&
    confirmedAt !== null &&
    !budgetInvalid &&
    !budgetBelow &&
    !submitting &&
    acceptedJob === null &&
    lockedJobId === null;

  const generateDisabledReason = ((): string | null => {
    if (lockedJobId !== null) {
      return "该操作已存在一个任务：不会新建第二份；请打开已有任务查看状态。";
    }
    if (quote === null) {
      return "还没有报价：先完成资料准备与视图，然后获取报价。";
    }
    if (expired) {
      return "报价已过期，请重新获取报价。";
    }
    if (confirmedAt === null) {
      return "需要先勾选确认「将发送给供应商的资料」。";
    }
    if (budgetInvalid) {
      return "预算金额格式不正确：请修正后再生成。";
    }
    if (budgetBelow) {
      return "授权上限低于本次报价的保守上界：服务端会拒绝该上限。";
    }
    return null;
  })();

  async function checkConfirmation(checked: boolean): Promise<void> {
    if (quote === null) {
      return;
    }
    if (!checked) {
      setConfirmChecked(false);
      return;
    }
    // 乐观勾选：勾选是用户动作，服务端确认在后台写入；失败时回退并给出原因
    // （不能"点了没反应"——那会让用户无法判断是否已确认发送范围）。
    setConfirmChecked(true);
    setConfirming(true);
    setConfirmError(null);
    try {
      const confirmation = await confirmEstimate(id, quote.id);
      setConfirmedAt(confirmation.data.confirmedAt);
    } catch (error) {
      setConfirmChecked(false);
      setConfirmedAt(null);
      const info = describeError(error);
      const reason = isApiError(error) ? readReason(error.details) : null;
      setConfirmError(
        reason === "quoteExpired" ? "报价已过期：请重新获取报价后再确认。" : info.message,
      );
    } finally {
      setConfirming(false);
    }
  }

  async function submit(): Promise<void> {
    if (quote === null || !canGenerate || !tripoParsed.ok || !usdParsed.ok) {
      return;
    }
    // 一次操作一个幂等键：断线/失败重试复用同一键（服务端按 key 去重，不产生第二份生成单）。
    idempotencyKeyRef.current ??= randomKey();
    setSubmitting(true);
    setSubmitError(null);
    try {
      const result = await createJob(
        id,
        {
          quoteId: quote.id,
          preparationId: quote.preparationId,
          photoIds,
          limits: {
            tripoCreditMinor: tripoParsed.minor,
            manualAiUsdMicros: usdParsed.minor,
          },
        },
        idempotencyKeyRef.current,
      );
      setAcceptedJob(result.job);
      notify("任务已受理（202）：后台继续执行");
      await queryClient.invalidateQueries({ queryKey: ["jobs"] });
    } catch (error) {
      const failure = describeSubmitFailure(error);
      setSubmitError(failure);
      // 报价被消费/过期/输入变化后不可再用：让用户走"重新获取报价"（不自动重报）。
      // 新一次提交的请求体必然不同 → 必须换新幂等键（同键不同 body 会 409）。
      if (isApiError(error) && (error.code === "IDEMPOTENCY_CONFLICT" || error.status === 422)) {
        idempotencyKeyRef.current = null;
      }
      // 服务端已存在同一操作的任务：锁住生成入口，避免用户再建第二份（不重复收费）。
      if (failure.existingJobId !== null) {
        setLockedJobId(failure.existingJobId);
      }
    } finally {
      setSubmitting(false);
    }
  }

  const itemName = itemQuery.data?.data.name ?? "物品";
  const documents = documentsQuery.data?.documents ?? [];

  const quotePanel = (
    <div className="confirm-panel">
      <h2 className="summary-panel__title">报价与预算</h2>
      {quote === null && quoting && <Skeleton label="正在获取报价…" rows={4} />}
      {quote === null && !quoting && (
        <p className="empty-note" data-testid="quote-missing">
          {blocked
            ? "资料未齐：补齐缺项后自动获取报价。"
            : quoteError !== null
              ? "报价获取失败：见主栏错误说明。"
              : "尚未获取报价。"}
        </p>
      )}
      {quote !== null && (
        <div data-testid="quote-panel">
          <dl className="amount-list">
            <div>
              <dt>Tripo（credits）</dt>
              <dd>
                <span data-testid="quote-tripo-upper">{quote.amounts.tripo.upperBoundDisplay}</span>
                <span className="amount-list__label">保守上界</span>
              </dd>
              <dd className="amount-list__secondary">
                预计 {quote.amounts.tripo.estimatedDisplay}
              </dd>
            </div>
            <div>
              <dt>说明书 AI（USD）</dt>
              <dd>
                <span data-testid="quote-manual-upper">
                  {quote.amounts.manualAi.upperBoundDisplay}
                </span>
                <span className="amount-list__label">保守上界</span>
              </dd>
              <dd className="amount-list__secondary">
                预计 {quote.amounts.manualAi.estimatedDisplay}
              </dd>
            </div>
          </dl>
          <p className="field__hint">两个供应商的金额分列显示，不相加、不换算成同一币种。</p>
          <dl className="meta-list">
            <div>
              <dt>价格版本</dt>
              <dd>{quote.priceVersion}</dd>
            </div>
            <div>
              <dt>快照日期</dt>
              <dd>{quote.priceSnapshotDate}</dd>
            </div>
            <div>
              <dt>页数 / 页范围</dt>
              <dd>
                {quote.pageCount} 页（{quote.pageRange.from}–{quote.pageRange.to}）
              </dd>
            </div>
            <div>
              <dt>有效期</dt>
              <dd data-testid="quote-expiry">
                {expired
                  ? "已过期"
                  : `剩余 ${formatRemaining(remainingSeconds)}（至 ${formatLocalDateTime(quote.expiresAt)}）`}
              </dd>
            </div>
          </dl>

          <h3>本次授权上限</h3>
          <p className="field__hint" id="budget-hint">
            默认等于服务端计算的保守上界；低于上界会被服务端拒绝（不自动降质量、不换模型）。
          </p>
          <BudgetField
            id="budget-tripo"
            label="Tripo credits"
            value={tripoBudget}
            onChange={setTripoBudget}
            error={tripoParsed.ok ? null : tripoParsed.message}
            hint={budgetBelow && tripoParsed.ok ? "低于服务端上界" : null}
          />
          <BudgetField
            id="budget-manual"
            label="说明书 AI USD"
            value={usdBudget}
            onChange={setUsdBudget}
            error={usdParsed.ok ? null : usdParsed.message}
            hint={budgetBelow && usdParsed.ok ? "低于服务端上界" : null}
          />
          <p className="field__hint">{quote.budgetNotice}</p>
        </div>
      )}
    </div>
  );

  const disclosurePanel = (
    <div className="disclosure-panel">
      <SendScopePanel quote={quote} />
      {quote !== null && (
        <div className="confirm-box" data-testid="confirmation-box">
          <h3 className="summary-panel__title">云端发送确认</h3>
          <div className="checkbox-row">
            <input
              id="send-scope-confirm"
              type="checkbox"
              checked={confirmChecked}
              disabled={confirming || expired}
              aria-describedby="send-scope-confirm-hint"
              onChange={(event) => void checkConfirmation(event.target.checked)}
            />
            <label htmlFor="send-scope-confirm">我已阅读并确认将上述资料发送给对应供应商</label>
          </div>
          <p className="field__hint" id="send-scope-confirm-hint">
            默认不勾选；勾选动作写入服务端审计事件（audit_events），未确认的提交会被拒绝。
          </p>
          {confirming && (
            <p role="status" className="empty-note">
              正在记录确认…
            </p>
          )}
          {confirmedAt !== null && (
            <p role="status" className="status-note" data-testid="confirmed-at">
              已确认发送范围（{formatLocalDateTime(confirmedAt)}）
            </p>
          )}
          {confirmError !== null && (
            <p className="field__error" role="alert" data-testid="confirm-error">
              {confirmError}
            </p>
          )}
        </div>
      )}
    </div>
  );

  return (
    <PageLayout rail={{ id: "quote", label: "报价与预算", content: quotePanel }} aside={{ id: "scope", label: "将发送的资料与确认", content: disclosurePanel }}>
      <section className="page confirm-step" aria-labelledby="confirm-step-title">
        <WizardSteps currentSegment="import/confirm" itemId={id} />
        <h1 id="confirm-step-title">预算与隐私确认</h1>
        <p className="page__lead">
          {itemName}：报价只计算计划、不调用生成服务；确认后才允许提交（生成在后台继续执行）。
        </p>

        {loading && <Skeleton label="正在读取资料与视图…" rows={4} />}

        {!loading && documents.length === 0 && (
          <div className="error-panel" role="alert">
            <h2>还没有说明书原件</h2>
            <p>
              生成需要 ready 的准备记录。<Link to={`/items/${id}/import/document`}>先去绑定说明书原件</Link>
              。
            </p>
          </div>
        )}

        {!loading && (
          <MissingItemsList
            title="生成前还缺"
            gaps={gaps}
            emptyText="资料已齐：准备已封存，视图满足 front + 侧面至少一张。"
          />
        )}

        {!loading && gaps.length > 0 && quoteGaps.length > 0 && (
          <ul className="missing-list__items" role="alert" data-testid="server-gaps">
            {quoteGaps.map((gap) => (
              <li key={gap.code}>
                {gap.message}
                {gap.actionHref !== null && <Link to={gap.actionHref}>{gap.actionLabel}</Link>}
              </li>
            ))}
          </ul>
        )}

        {quoteError !== null && (
          <div className="error-panel" role="alert" data-testid="quote-error">
            <h2>无法获取报价</h2>
            <p>{quoteError.message}</p>
            {quoteError.hint !== null && <p className="error-panel__meta">{quoteError.hint}</p>}
            <div className="error-panel__actions">
              <button type="button" onClick={() => void requestQuote()} disabled={quoting}>
                {quoting ? "获取中…" : "重试获取报价"}
              </button>
              <Link to="/settings">查看服务状态</Link>
            </div>
          </div>
        )}

        {acceptedJob !== null ? (
          <div className="accepted-panel" role="status" data-testid="job-accepted">
            <h2>任务已受理（202）</h2>
            <p>
              任务 <code>{acceptedJob.id}</code> 已创建并进入执行队列；后台继续执行，
              关闭浏览器不影响已提交任务（需要服务进程保持运行）。
            </p>
            <p className="error-panel__meta">
              202 只表示服务端已入队（受理），不代表生成结果：请到任务中心查看阶段状态。
            </p>
            <p>
              <Link to={`/jobs/${acceptedJob.id}`}>查看任务详情</Link>
            </p>
          </div>
        ) : (
          <div className="confirm-actions">
            <button
              type="button"
              className="button-primary"
              data-testid="generate-button"
              aria-busy={submitting}
              disabled={!canGenerate}
              aria-describedby={generateDisabledReason !== null ? "generate-reason" : undefined}
              onClick={() => void submit()}
            >
              {submitting ? "正在创建任务…" : "生成 3D 与说明书草稿"}
            </button>
            {generateDisabledReason !== null && (
              <p className="confirm-actions__reason" id="generate-reason" data-testid="generate-reason">
                {generateDisabledReason}
              </p>
            )}
            {expired && quote !== null && (
              <button
                type="button"
                data-testid="requote-button"
                onClick={() => void requestQuote()}
                disabled={quoting}
              >
                {quoting ? "重新获取中…" : "重新获取报价"}
              </button>
            )}

            {submitError !== null && (
              <div className="error-panel" role="alert" data-testid="submit-error">
                <h2>提交被拒绝</h2>
                <p>{submitError.message}</p>
                {submitError.hint !== null && <p className="error-panel__meta">{submitError.hint}</p>}
                <div className="error-panel__actions">
                  {submitError.existingJobId !== null && (
                    <Link to={`/jobs/${submitError.existingJobId}`}>查看已有任务</Link>
                  )}
                  {submitError.recovery === "requote" && (
                    <button type="button" onClick={() => void requestQuote()} disabled={quoting}>
                      重新获取报价
                    </button>
                  )}
                  {submitError.recovery === "rewizard" && (
                    <Link to={`/items/${id}/import/views`}>返回检查视图与资料</Link>
                  )}
                </div>
              </div>
            )}
          </div>
        )}
      </section>
    </PageLayout>
  );
}

function prepareState(input: {
  pointer: string | null;
  detail: PreparationDetail | undefined;
}): string | null {
  if (input.pointer === null) {
    return null;
  }
  return input.detail?.detail.state ?? null;
}

function BudgetField({
  id,
  label,
  value,
  onChange,
  error,
  hint,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  error: string | null;
  hint: string | null;
}) {
  return (
    <div className="field">
      <label className="field__label" htmlFor={id}>
        {label}
      </label>
      <input
        id={id}
        className="field__input"
        type="text"
        inputMode="decimal"
        value={value}
        aria-invalid={error !== null ? true : undefined}
        aria-describedby={error !== null ? `${id}-error` : hint !== null ? `${id}-hint` : "budget-hint"}
        onChange={(event) => onChange(event.target.value)}
      />
      {error !== null && (
        <p className="field__error" id={`${id}-error`}>
          {error}
        </p>
      )}
      {error === null && hint !== null && (
        <p className="field__warning" id={`${id}-hint`}>
          {hint}
        </p>
      )}
    </div>
  );
}

/** 云端发送告知（UI-024）：逐项列出"将发送什么给谁"。 */
function SendScopePanel({ quote }: { quote: QuoteDto | null }) {
  if (quote === null) {
    return (
      <div className="summary-panel">
        <h2 className="summary-panel__title">将发送的资料与确认</h2>
        <p className="empty-note">还没有报价：确认区在报价可用后列出将发送给各供应商的资料。</p>
      </div>
    );
  }
  const scope = quote.sendScope;
  const tripoViews = scope.tripo.views;
  const manual = scope.manualAi;
  return (
    <div className="summary-panel" data-testid="send-scope">
      <h2 className="summary-panel__title">将发送的资料与确认</h2>

      <h3>发送给 Tripo（模型生成）</h3>
      <ul className="scope-list">
        {tripoViews.map((view) => (
          <li key={view.photoId}>
            {VIEW_LABELS[view.view as ViewSlot] ?? view.view}（{view.view}）照片：
            <code>{view.photoId.slice(0, 8)}…</code> sha256 <code>{view.sha256.slice(0, 12)}…</code>
          </li>
        ))}
      </ul>
      <p className="field__hint">
        模型：{scope.tripo.model}（预设 {scope.tripo.preset}）；参数：face_limit{" "}
        {scope.tripo.parameters.faceLimit}、texture{" "}
        {String(scope.tripo.parameters.texture)}、pbr {String(scope.tripo.parameters.pbr)}、
        {scope.tripo.parameters.textureQuality}/{scope.tripo.parameters.geometryQuality}。
        detail（特写）照片不发送。
      </p>

      <h3>发送给说明书 AI</h3>
      <ul className="scope-list">
        <li>
          物品身份文本：{manual.itemName} · {manual.itemModel}
        </li>
        <li>
          页范围：第 {manual.pageFrom}–{manual.pageTo} 页（共 {manual.pageCount} 页）
        </li>
        <li>页文字页：{formatPageList(manual.textPages)}</li>
        <li>页图页（扫描/无文字层）：{formatPageList(manual.imagePages)}</li>
        <li>
          模型：{manual.model}（prompt 版本 {manual.promptVersion}）；最大输出 token：
          {manual.maxOutputTokens}
        </li>
      </ul>

      <p className="field__hint">
        价格版本 {scope.priceVersion}（快照 {scope.priceSnapshotDate}）；本次保守上界：Tripo{" "}
        {scope.plannedUpperBound.tripo.upperBoundDisplay}、说明书 AI{" "}
        {scope.plannedUpperBound.manualAi.upperBoundDisplay}（分列，不相加）。
      </p>
      <p className="field__hint">{scope.budgetNotice}</p>
    </div>
  );
}

function formatPageList(pages: readonly number[]): string {
  return pages.length === 0 ? "无" : pages.map((page) => `第 ${page} 页`).join("、");
}

function formatRemaining(seconds: number | null): string {
  if (seconds === null) {
    return "未知";
  }
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return minutes > 0 ? `${minutes} 分 ${rest} 秒` : `${rest} 秒`;
}

function randomKey(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `em-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function readGapItems(error: unknown): MissingItem[] {
  const details = isApiError(error) ? error.details : null;
  if (typeof details !== "object" || details === null) {
    return [];
  }
  const items = (details as { items?: unknown }).items;
  if (!Array.isArray(items)) {
    return [];
  }
  return items.flatMap((entry) => {
    if (typeof entry !== "object" || entry === null) {
      return [];
    }
    const { code, message } = entry as { code?: unknown; message?: unknown };
    if (typeof code !== "string" || typeof message !== "string") {
      return [];
    }
    return [{ code, message, actionHref: null, actionLabel: null }];
  });
}

function readSupportedPresets(error: unknown): string {
  const details = isApiError(error) ? error.details : null;
  if (typeof details === "object" && details !== null) {
    const presets = (details as { supportedPresets?: unknown }).supportedPresets;
    if (Array.isArray(presets)) {
      return `服务端当前支持的预设：${presets.join("、") || "（空）"}。`;
    }
  }
  return "服务端未给出可用预设清单。";
}

/** 提交失败的可行动文案（UI-026：过期/输入变化/同键冲突分别给恢复路径）。 */
export function describeSubmitFailure(error: unknown): SubmissionError {
  if (isApiError(error)) {
    const reason = readReason(error.details);
    const details = error.details as Record<string, unknown> | null;
    const existingJobId =
      typeof details?.existingResourceId === "string"
        ? details.existingResourceId
        : typeof details?.jobId === "string"
          ? details.jobId
          : null;
    if (reason === "quoteExpired" || error.code === "QUOTE_EXPIRED") {
      return {
        message: "报价已过期：请重新获取报价后再提交。",
        hint: null,
        existingJobId: null,
        recovery: "requote",
      };
    }
    if (reason === "inputChanged") {
      return {
        message: "资料已更新：本次报价绑定的输入与当前资料不一致。",
        hint: "请回到向导核对视图与准备，再重新获取报价并确认发送内容。",
        existingJobId: null,
        recovery: "rewizard",
      };
    }
    if (reason === "idempotencyKeyReused" || error.code === "IDEMPOTENCY_CONFLICT") {
      return {
        message: "该操作已存在一个任务（同一幂等键被用于不同的请求内容）。",
        hint: "不会新建任务；请打开已有任务查看状态。",
        existingJobId,
        recovery: "none",
      };
    }
    if (reason === "quoteAlreadyUsed") {
      return {
        message: "这份报价已经创建过任务：一份报价只能创建一份任务（重生成需要新报价）。",
        hint: existingJobId !== null ? "可打开已有任务查看状态。" : null,
        existingJobId,
        recovery: "requote",
      };
    }
    if (reason === "budgetBelowPlannedUpperBound") {
      return {
        message: "授权上限低于服务端计算的保守上界：请调高预算后重试。",
        hint: formatBudgetDetails(details),
        existingJobId: null,
        recovery: "none",
      };
    }
    if (reason === "confirmationRequired") {
      return {
        message: "需要先确认「将发送给供应商的资料」。",
        hint: "请在右侧确认区勾选后再提交。",
        existingJobId: null,
        recovery: "none",
      };
    }
    if (error.code === "PROVIDER_NOT_CONFIGURED" || error.code === "PRICE_CATALOG_MISSING") {
      return {
        message: "生成能力未就绪：服务端缺少供应商密钥或价格目录。",
        hint: error.message,
        existingJobId: null,
        recovery: "none",
      };
    }
    return { message: error.message, hint: null, existingJobId, recovery: "none" };
  }
  const info = describeError(error);
  return {
    message: info.message,
    hint: "网络错误没有产生第二次建单：重试会复用同一个幂等键。",
    existingJobId: null,
    recovery: "none",
  };
}

function formatBudgetDetails(details: Record<string, unknown> | null): string | null {
  if (details === null) {
    return null;
  }
  const parts: string[] = [];
  for (const [key, value] of Object.entries(details)) {
    if (typeof key === "string" && typeof value === "number" && key !== "currentRevision") {
      parts.push(`${key}=${value}`);
    }
  }
  return parts.length > 0 ? parts.join("，") : null;
}
