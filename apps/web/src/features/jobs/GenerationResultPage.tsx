/**
 * 生成结果页（路由 `/jobs/:jobId/result`）：一次生成的产物——3D 模型 + 说明书 AI 提取结果。
 *
 * - 数据来自该任务**自己的**草稿（每个任务一个冻结快照与一份草稿，旧结果不会被新任务覆盖），
 *   因此任何一次历史生成都能用同一地址回看；
 * - 只读预览：不放热点（热点属于复核阶段），不改草稿；复核与发布仍走草稿复核页；
 * - 任务未结束时显示进度摘要并继续轮询，完成后自动切到结果。
 */

import { useMemo, useState } from "react";
import { Link, useParams } from "react-router";
import { useQuery } from "@tanstack/react-query";

import { describeError } from "../../api/client";
import { getDraft } from "../../api/endpoints";
import { EmptyNote } from "../../components/EmptyState";
import { Skeleton } from "../../components/Skeleton";
import { formatLocalDateTime } from "../../lib/format";
import { PageLayout } from "../shell/PageLayout";
import { checkAnchor } from "../viewer/coordinates";
import { readDraftHotspots, readDraftModel, readDraftParts, readDraftSpecs, readDraftSteps } from "../viewer/draft-view";
import { InteractionPanel } from "../viewer/InteractionPanel";
import { readInteractive } from "../viewer/interactive-view";
import { useInteractive } from "../viewer/useInteractive";
import { ViewerPanel } from "../viewer/ViewerPanel";
import { CostBreakdown } from "./CostBreakdown";
import { useJobDetail } from "./jobs";
import { isTerminalJobStatus, jobStatusMeta } from "./status";

function pages(evidence: readonly { pageNumber: number }[]): string {
  const list = [...new Set(evidence.map((item) => item.pageNumber))].sort((a, b) => a - b);
  return list.length === 0 ? "" : `第 ${list.join("、")} 页`;
}

export function GenerationResultPage() {
  const jobId = useParams().jobId ?? "";
  const jobQuery = useJobDetail(jobId === "" ? null : jobId);
  const job = jobQuery.data?.data ?? null;
  const draftId = job?.draftId ?? null;
  const itemId = job?.item.id ?? null;

  const draftQuery = useQuery({
    queryKey: ["draft", itemId, draftId],
    queryFn: () => getDraft(itemId ?? "", draftId ?? ""),
    enabled: itemId !== null && draftId !== null,
  });
  const knowledge = draftQuery.data?.data.knowledge;
  const model = useMemo(() => readDraftModel(knowledge), [knowledge]);
  const parts = useMemo(() => readDraftParts(knowledge), [knowledge]);
  const steps = useMemo(() => readDraftSteps(knowledge), [knowledge]);
  const specs = useMemo(() => readDraftSpecs(knowledge), [knowledge]);
  const [selectedPartId, setSelectedPartId] = useState<string | null>(null);
  // 预览展示候选（自动绑定）与已确认热点；锚点必须属于当前模型版本。
  const hotspots = useMemo(() => {
    if (model === null) {
      return [];
    }
    return readDraftHotspots(knowledge).flatMap((hotspot) =>
      hotspot.anchor !== null && checkAnchor(hotspot.anchor, model).usable
        ? [{ id: hotspot.id, partId: hotspot.partId, positionLocal: hotspot.anchor.positionLocal, status: hotspot.status }]
        : [],
    );
  }, [knowledge, model]);
  const interaction = useInteractive(useMemo(() => readInteractive(knowledge, model), [knowledge, model]), selectedPartId);
  const hotspotParts = useMemo(() => new Set(hotspots.map((hotspot) => hotspot.partId)), [hotspots]);
  const selectedHotspotId = hotspots.find((hotspot) => hotspot.partId === selectedPartId)?.id ?? null;

  if (jobQuery.isPending) {
    return <Skeleton rows={4} label="正在读取生成结果" />;
  }
  if (job === null) {
    const info = jobQuery.error === null ? null : describeError(jobQuery.error);
    return (
      <div className="page">
        <h1>生成结果</h1>
        <p className="field__error" role="alert">无法读取该任务：{info?.message ?? "任务不存在"}</p>
        <Link to="/jobs">返回任务中心</Link>
      </div>
    );
  }

  const meta = jobStatusMeta(job.status);
  const finished = isTerminalJobStatus(job.status);
  const reviewHref = draftId === null ? null : `/items/${job.item.id}/drafts/${draftId}/review`;

  return (
    <div className="page result-page" data-testid="generation-result">
      <header className="page__header">
        <div>
          <p className="eyebrow">GENERATION RESULT</p>
          <h1>{job.item.name}{job.item.model !== "" && ` · ${job.item.model}`}</h1>
          <p className="field__hint">
            生成于 {formatLocalDateTime(job.createdAt)} · 状态：{meta.label}
          </p>
        </div>
        <div className="page__header-actions">
          {reviewHref !== null && (
            <Link className="button-primary" to={reviewHref}>去复核并发布</Link>
          )}
          <Link className="button" to={`/items/${job.item.id}/generations`}>本物品的生成历史</Link>
          <Link className="button" to={`/jobs/${job.id}`}>任务详情</Link>
        </div>
      </header>

      {!finished && (
        <p className="page-note" role="status" data-testid="result-pending">
          生成仍在进行：{meta.nextStep} 完成后本页会自动显示结果，你也可以离开本页，完成时会收到提示。
        </p>
      )}
      {finished && draftId === null && (
        <p className="field__error" role="alert">
          本次生成没有产出草稿（{meta.label}）。请到任务详情查看各阶段原因。
        </p>
      )}

      {draftId !== null && (
        <PageLayout
          rail={{
            id: "result-parts",
            label: "部件",
            content: (
              <section aria-label="提取的部件">
                <h2>部件 <span className="status-label">{parts.length}</span></h2>
                {parts.length === 0 ? (
                  <EmptyNote>本次没有提取到部件。</EmptyNote>
                ) : (
                  <ul className="result-list">
                    {parts.map((part) => (
                      <li key={part.id} className={part.id === selectedPartId ? "is-selected" : undefined}>
                        <button
                          type="button"
                          className="link-button"
                          aria-pressed={part.id === selectedPartId}
                          onClick={() => setSelectedPartId(part.id === selectedPartId ? null : part.id)}
                        >
                          <strong>{part.name}</strong>
                        </button>
                        {part.id === selectedPartId && <span className="status-label">已选</span>}
                        {hotspotParts.has(part.id) && <span className="status-label status-label--warning">热点（待复核）</span>}
                        {part.description !== "" && <p>{part.description}</p>}
                        {part.evidence.length > 0 && <p className="step-evidence">原文：{pages(part.evidence)}</p>}
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            ),
          }}
          aside={{
            id: "result-knowledge",
            label: "步骤、规格与费用",
            content: (
              <>
                <section aria-label="提取的步骤">
                  <h2>步骤 <span className="status-label">{steps.length}</span></h2>
                  {steps.length === 0 ? (
                    <EmptyNote>本次没有提取到步骤。</EmptyNote>
                  ) : (
                    <ol className="result-list">
                      {steps.map((step) => (
                        <li key={step.id}>
                          <strong>{step.title}</strong>
                          <ul>
                            {step.orderedActions.map((action, index) => (
                              <li key={`${step.id}-${index}`}>{action}</li>
                            ))}
                          </ul>
                          {step.evidence.length > 0 && <p className="step-evidence">原文：{pages(step.evidence)}</p>}
                        </li>
                      ))}
                    </ol>
                  )}
                </section>
                <section aria-label="提取的规格">
                  <h2>规格 <span className="status-label">{specs.length}</span></h2>
                  {specs.length === 0 ? (
                    <EmptyNote>本次没有提取到规格。</EmptyNote>
                  ) : (
                    <dl className="result-specs">
                      {specs.map((spec) => (
                        <div key={spec.id}>
                          <dt>{spec.label}</dt>
                          <dd>{spec.value}</dd>
                        </div>
                      ))}
                    </dl>
                  )}
                </section>
                <CostBreakdown reservations={job.reservations} budgetNotice={job.budgetNotice} />
              </>
            ),
          }}
        >
          <section aria-label="3D 模型预览">
            {draftQuery.isPending ? (
              <Skeleton rows={3} label="正在读取草稿" />
            ) : draftQuery.isError ? (
              <p className="field__error" role="alert">
                读取草稿失败：{describeError(draftQuery.error).message}
              </p>
            ) : (
              <>
                <ViewerPanel
                  model={model}
                  hotspots={hotspots}
                  selectedHotspotId={selectedHotspotId}
                  onHotspotSelect={(hotspotId) =>
                    setSelectedPartId(hotspots.find((hotspot) => hotspot.id === hotspotId)?.partId ?? null)
                  }
                  interactive={interaction?.viewerProp}
                />
                {interaction !== null && <InteractionPanel {...interaction.panel} selectedPartId={selectedPartId} />}
                <p className="field__hint">
                  这是本次生成的原始结果：热点为自动绑定的候选（{hotspots.length} 个），需在复核页确认后才会发布。
                </p>
              </>
            )}
          </section>
        </PageLayout>
      )}
    </div>
  );
}

export default GenerationResultPage;
