/**
 * 发布版阅读器（T19；路由 `/items/:itemId/releases/:releaseId`；REQ-036 / AC-057）。
 *
 * 交互合同（QA 按此复核）：
 * - **四方联动**：部件列表 ↔ 3D 热点 ↔ 步骤导航 ↔ 原文页；任一侧选择在其它侧可见；
 * - **部件列表是 3D 热点的文字替代**：选择、状态、跳转都在左栏可用（3D 只作增强）；
 * - **引用跳到正确的 1-based 页码**：步骤/部件的出处按钮把右侧原文面板切到该页；
 * - **键盘路径**：列表项、步骤按钮、翻页与视角按钮全部可 Tab/回车操作，焦点可见；
 * - **减少动效**：不播放相机自动动画/过渡（视角切换即时到位）；
 * - 数据全部来自 release 的**不可变 manifest**（`manifest.knowledge` 与
 *   `manifest.model.assetId` → 本地资产），不读会变的草稿内容。
 */

import { Suspense, lazy, useMemo, useRef, useState } from "react";
import { Link, useParams } from "react-router";
import { useQuery } from "@tanstack/react-query";

import { describeError } from "../../api/client";
import { assetContentUrl, getRelease } from "../../api/endpoints";
import { EmptyNote } from "../../components/EmptyState";
import { Skeleton } from "../../components/Skeleton";
import { PageLayout } from "../shell/PageLayout";
import { checkAnchor } from "../viewer/coordinates";
import {
  readDraftHotspots,
  readDraftMissing,
  readDraftModel,
  readDraftParts,
  readDraftReview,
  readDraftSpecs,
  readDraftStepPoses,
  readDraftSteps,
} from "../viewer/draft-view";
import { ViewerPanel } from "../viewer/ViewerPanel";

const OriginalDocumentPanel = lazy(() =>
  import("../viewer/OriginalDocumentPanel").then((module) => ({
    default: module.OriginalDocumentPanel,
  })),
);

interface ManifestShape {
  readonly knowledge?: unknown;
  readonly review?: unknown;
  readonly model?: { readonly assetId?: unknown } | null;
  readonly documents?: readonly {
    readonly documentId?: unknown;
    readonly sourceAssetId?: unknown;
  }[];
  readonly counts?: Record<string, unknown>;
}

export function ReleaseReaderPage() {
  const params = useParams();
  const itemId = params.itemId ?? "";
  const releaseId = params.releaseId ?? "";
  const releaseQuery = useQuery({
    queryKey: ["release", itemId, releaseId],
    queryFn: () => getRelease(itemId, releaseId),
    enabled: itemId !== "" && releaseId !== "",
  });

  const [pageNumber, setPageNumber] = useState(1);
  const [pageCount, setPageCount] = useState<number | null>(null);
  const [selectedPartId, setSelectedPartId] = useState<string | null>(null);
  const [selectedHotspotId, setSelectedHotspotId] = useState<string | null>(null);
  const [stepIndex, setStepIndex] = useState(0);
  const [notice, setNotice] = useState<string | null>(null);
  const partsRef = useRef<HTMLDivElement | null>(null);

  const manifest = (releaseQuery.data?.data.manifest ?? null) as ManifestShape | null;
  const knowledge = manifest?.knowledge;
  const model = useMemo(() => readDraftModel(knowledge), [knowledge]);
  const parts = useMemo(() => readDraftParts(knowledge), [knowledge]);
  const steps = useMemo(() => readDraftSteps(knowledge), [knowledge]);
  const specs = useMemo(() => readDraftSpecs(knowledge), [knowledge]);
  const hotspots = useMemo(() => readDraftHotspots(knowledge), [knowledge]);
  const stepPoses = useMemo(() => readDraftStepPoses(knowledge), [knowledge]);
  const review = useMemo(() => readDraftReview(manifest?.review), [manifest]);
  const missing = useMemo(() => readDraftMissing(knowledge), [knowledge]);

  // 只把"锚点仍属于发布模型版本"的热点交给 3D（stale 不显示；发布不变量已保证
  // 不存在冒充 confirmed 的绑定，这里再挡一次读取侧防线）。
  const displayHotspots = useMemo(() => {
    if (model === null) {
      return [];
    }
    return hotspots.flatMap((hotspot) => {
      if (hotspot.anchor === null || !checkAnchor(hotspot.anchor, model).usable) {
        return [];
      }
      return [
        { id: hotspot.id, partId: hotspot.partId, positionLocal: hotspot.anchor.positionLocal },
      ];
    });
  }, [hotspots, model]);

  const documentAssetId = useMemo(() => {
    const document = manifest?.documents?.[0];
    const assetId = document?.sourceAssetId;
    return typeof assetId === "string" && assetId !== "" ? assetId : null;
  }, [manifest]);

  const textOnlyParts = useMemo(
    () => parts.filter((part) => review.entities[part.id]?.textOnly === true),
    [parts, review],
  );

  if (releaseQuery.isLoading) {
    return <Skeleton label="正在读取发布版本…" rows={4} />;
  }
  if (releaseQuery.isError) {
    return (
      <div className="page-error" role="alert">
        <p>发布版本读取失败：{describeError(releaseQuery.error).message}</p>
        <Link to={`/items/${itemId}/releases`}>返回版本列表</Link>
      </div>
    );
  }

  const release = releaseQuery.data?.data;
  const safeStepIndex = Math.min(Math.max(stepIndex, 0), Math.max(steps.length - 1, 0));
  const currentStep = steps[safeStepIndex];
  const focusParts = () => partsRef.current?.focus();

  const goToStep = (index: number) => {
    if (steps.length === 0) {
      return;
    }
    const bounded = Math.min(Math.max(index, 0), steps.length - 1);
    const target = steps[bounded] as (typeof steps)[number];
    setStepIndex(bounded);
    const evidence = target.evidence[0];
    if (evidence !== undefined) {
      setPageNumber(evidence.pageNumber);
    }
    setSelectedPartId(target.partIds[0] ?? null);
    setSelectedHotspotId(null);
  };

  return (
    <div>
      <h1>已发布说明书</h1>
      {release !== undefined && (
        <p className="page-note" data-testid="release-context">
          发布版本 {release.id} · 草稿 r{release.draftRevision} · 模型 {release.modelRevisionId} ·
          manifest {release.manifestSha256.slice(0, 12)}…（不可变）
        </p>
      )}
      <p className="page-subtitle">
        本页内容来自发布时的冻结 manifest；之后对草稿的修改不会改变这里。3D 只作增强，文字与原文始终可用。
      </p>
      {missing.length > 0 && (
        <section className="notice-panel" aria-label="发布内容说明">
          <h2>发布内容说明</h2>
          <ul>
            {missing.map((entry) => (
              <li key={entry.code}>{entry.message}</li>
            ))}
          </ul>
        </section>
      )}
      {notice !== null && (
        <p className="notice-panel" role="status" data-testid="reader-notice">
          {notice}
        </p>
      )}

      <PageLayout
        rail={{
          id: "parts",
          label: "部件",
          content: (
            <div ref={partsRef} tabIndex={-1} data-testid="parts-panel">
              <section aria-label="部件列表">
                <h2>部件</h2>
                {parts.length === 0 ? (
                  <EmptyNote>该发布版本没有部件列表。</EmptyNote>
                ) : (
                  <ul className="entity-list" data-testid="parts-list">
                    {parts.map((part) => {
                      const partHotspots = displayHotspots.filter(
                        (hotspot) => hotspot.partId === part.id,
                      );
                      const textOnly = review.entities[part.id]?.textOnly === true;
                      return (
                        <li key={part.id} data-testid={`reader-part-${part.id}`}>
                          <button
                            type="button"
                            aria-current={selectedPartId === part.id}
                            onClick={() => {
                              setSelectedPartId(part.id);
                              setSelectedHotspotId(partHotspots[0]?.id ?? null);
                              setNotice(
                                partHotspots.length > 0
                                  ? `已在 3D 中定位部件「${part.name}」的热点`
                                  : `部件「${part.name}」没有 3D 热点（发布时标记为文字条目）`,
                              );
                            }}
                          >
                            {part.name}
                          </button>
                          <span className="status-label">
                            {textOnly
                              ? "仅文本条目"
                              : partHotspots.length > 0
                                ? `热点 ${partHotspots.length}`
                                : "无热点"}
                          </span>
                          {part.description !== "" && <p>{part.description}</p>}
                          {part.evidence.length > 0 && (
                            <p className="step-evidence">
                              原文：
                              {part.evidence.map((evidence, index) => (
                                <button
                                  key={`${part.id}-page-${index}`}
                                  type="button"
                                  onClick={() => setPageNumber(evidence.pageNumber)}
                                >
                                  第 {evidence.pageNumber} 页
                                </button>
                              ))}
                            </p>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                )}
                {textOnlyParts.length > 0 && (
                  <p className="page-note" data-testid="text-only-note">
                    仅文本条目（发布时保留并明显标识）：{textOnlyParts.map((part) => part.name).join("、")}
                  </p>
                )}
              </section>
            </div>
          ),
        }}
        aside={{
          id: "steps",
          label: "步骤与原文",
          content: (
            <div data-testid="steps-panel">
              <section aria-label="步骤列表">
                <h2>步骤</h2>
                {steps.length === 0 ? (
                  <EmptyNote>该发布版本没有步骤列表。</EmptyNote>
                ) : (
                  <>
                    <div className="step-nav" role="group" aria-label="步骤导航">
                      <button
                        type="button"
                        disabled={safeStepIndex <= 0}
                        onClick={() => goToStep(safeStepIndex - 1)}
                      >
                        上一步
                      </button>
                      <span data-testid="reader-step-position">
                        第 {safeStepIndex + 1} / {steps.length} 步
                      </span>
                      <button
                        type="button"
                        disabled={safeStepIndex >= steps.length - 1}
                        onClick={() => goToStep(safeStepIndex + 1)}
                      >
                        下一步
                      </button>
                    </div>
                    <ol className="entity-list" data-testid="steps-list">
                      {steps.map((step, index) => (
                        <li key={step.id}>
                          <button
                            type="button"
                            aria-current={index === safeStepIndex}
                            onClick={() => goToStep(index)}
                          >
                            {step.title}
                          </button>
                          {index === safeStepIndex && (
                            <div className="step-detail">
                              {step.orderedActions.length > 0 && (
                                <ol>
                                  {step.orderedActions.map((action, actionIndex) => (
                                    <li key={`${step.id}-action-${actionIndex}`}>{action}</li>
                                  ))}
                                </ol>
                              )}
                              {step.safetyNotes.length > 0 && (
                                <ul>
                                  {step.safetyNotes.map((note, noteIndex) => (
                                    <li key={`${step.id}-note-${noteIndex}`}>注意：{note}</li>
                                  ))}
                                </ul>
                              )}
                              {step.partIds.length > 0 && (
                                <p className="step-parts">
                                  引用部件：
                                  {step.partIds.map((partId) => (
                                    <button
                                      key={`${step.id}-part-${partId}`}
                                      type="button"
                                      className={
                                        selectedPartId === partId
                                          ? "step-parts__chip step-parts__chip--active"
                                          : "step-parts__chip"
                                      }
                                      onClick={() => setSelectedPartId(partId)}
                                    >
                                      {parts.find((part) => part.id === partId)?.name ?? partId}
                                    </button>
                                  ))}
                                </p>
                              )}
                              {step.evidence.length > 0 && (
                                <p className="step-evidence">
                                  原文：
                                  {step.evidence.map((evidence, evidenceIndex) => (
                                    <button
                                      key={`${step.id}-page-${evidenceIndex}`}
                                      type="button"
                                      onClick={() => setPageNumber(evidence.pageNumber)}
                                    >
                                      第 {evidence.pageNumber} 页
                                    </button>
                                  ))}
                                </p>
                              )}
                              {stepPoses[step.id] !== undefined && (
                                <p className="page-note">该步骤保存了视角（发布时冻结）。</p>
                              )}
                            </div>
                          )}
                        </li>
                      ))}
                    </ol>
                  </>
                )}
              </section>
              <Suspense
                fallback={
                  <p className="original-panel__message" role="status">
                    正在加载原文模块…
                  </p>
                }
              >
                <OriginalDocumentPanel
                  assetId={documentAssetId}
                  pageNumber={pageNumber}
                  onPageChange={(next) => setPageNumber(Math.max(1, next))}
                  onPageCount={setPageCount}
                />
              </Suspense>
              {specs.length > 0 && (
                <section aria-label="规格">
                  <h3>规格</h3>
                  <ul className="entity-list" data-testid="specs-list">
                    {specs.map((spec) => (
                      <li key={spec.id}>
                        {spec.label}：{spec.value}
                      </li>
                    ))}
                  </ul>
                </section>
              )}
            </div>
          ),
        }}
      >
        <ViewerPanel
          model={model}
          hotspots={displayHotspots}
          selectedHotspotId={selectedHotspotId}
          onHotspotSelect={(hotspotId) => {
            setSelectedHotspotId(hotspotId);
            const hotspot = displayHotspots.find((candidate) => candidate.id === hotspotId);
            if (hotspot !== undefined) {
              setSelectedPartId(hotspot.partId);
            }
          }}
          onUseTextPath={focusParts}
        />
        {currentStep !== undefined && (
          <p className="page-note" data-testid="reader-current-step">
            当前步骤：{currentStep.title}（第 {safeStepIndex + 1} / {steps.length} 步）
          </p>
        )}
        <p className="page-note" data-testid="reader-context">
          页码 {pageNumber}
          {pageCount !== null ? ` / ${pageCount}` : ""}（1-based） · 模型资产{" "}
          <a href={model !== null ? assetContentUrl(model.assetId) : "#"}>本地 GLB</a>
        </p>
      </PageLayout>
    </div>
  );
}

export default ReleaseReaderPage;
