/**
 * 校准工作区（T19；路由 `/items/:itemId/drafts/:draftId/review`；PRD §6.1.3/§6.1.4）。
 *
 * 交互合同（QA 按此复核）：
 * - **三栏**：左栏部件列表（3D 热点的**文字替代路径**）、中栏 3D 视口、右栏步骤 +
 *   原文 + 确认/修订面板；窄屏（<768px）退化为「主栏 + 抽屉」；
 * - **部件 ↔ 热点双向联动**（UI-046）：点部件 → 高亮并居中对应热点；点 3D 热点 →
 *   左栏选中对应部件；
 * - **拾取模式与旋转模式互斥**（UI-047/UI-048）：只有拾取模式下点击模型表面才建点；
 *   拖动/长按只旋转相机（判定在 `ViewerStage`）；人工直接拾取得到 confirmed；
 * - **stale 不冒充有效热点**（UI-049）：失效区块单独列出，只能「重新绑定」（用同一
 *   热点 id 提交新模型的 anchor），界面不提供"强制确认"；
 * - **「事实确认」与「几何校准」分开**（§6.3.2）：确认知识（文字）与热点绑定/视角
 *   在文案、位置与操作名称上分开，不使用无定语的"确认"；
 * - **窄屏禁用几何校准与视角保存**，保留只读热点、文字确认与发布（§6.1.4）；
 * - **无自动发布路径**：发布是显式按钮 + 服务端不变量校验（面板见 `PublishPanel`）。
 */

import { Suspense, lazy, useCallback, useMemo, useRef, useState } from "react";
import { Link, useParams } from "react-router";
import { useQuery } from "@tanstack/react-query";

import { getDraft } from "../../api/endpoints";
import { describeError } from "../../api/client";
import { Skeleton } from "../../components/Skeleton";
import { EmptyNote } from "../../components/EmptyState";
import { useItemDocuments } from "../library/items";
import { PageLayout } from "../shell/PageLayout";
import { useBreakpoint } from "../shell/useBreakpoint";
import type { CameraPose } from "../viewer/coordinates";
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
import type { ViewerStageApi, ViewerPickResult } from "../viewer/ViewerStage";
import { ViewerPanel } from "../viewer/ViewerPanel";
import { KnowledgeReviewPanel } from "./KnowledgeReviewPanel";
import { PublishPanel } from "./PublishPanel";
import { useDraftMutations } from "./useDraftMutations";
import {
  hotspotPickUpsert,
  hotspotRebindUpsert,
  hotspotRemove,
  hotspotViews,
  publishChecklist,
  summarizePartHotspots,
  textOnlyPatch,
  usableHotspots,
} from "./review-state";

/** 原文面板单独成 chunk：PDF.js 体积不进 3D chunk，也不进首屏。 */
const OriginalDocumentPanel = lazy(() =>
  import("../viewer/OriginalDocumentPanel").then((module) => ({
    default: module.OriginalDocumentPanel,
  })),
);

export function CalibrationWorkspace() {
  const params = useParams();
  const itemId = params.itemId ?? "";
  const draftId = params.draftId ?? "";
  const breakpoint = useBreakpoint();
  const narrow = breakpoint === "narrow";
  const draftQuery = useQuery({
    queryKey: ["draft", itemId, draftId],
    queryFn: () => getDraft(itemId, draftId),
    enabled: itemId !== "" && draftId !== "",
  });
  const documentsQuery = useItemDocuments(itemId === "" ? null : itemId);
  const mutations = useDraftMutations(itemId, draftId);

  const [pickMode, setPickMode] = useState(false);
  const [selectedPartId, setSelectedPartId] = useState<string | null>(null);
  const [selectedHotspotId, setSelectedHotspotId] = useState<string | null>(null);
  const [bindingPartId, setBindingPartId] = useState<string | null>(null);
  const [rebindingHotspotId, setRebindingHotspotId] = useState<string | null>(null);
  const [pageNumber, setPageNumber] = useState(1);
  const [pageCount, setPageCount] = useState<number | null>(null);
  const [activeStepId, setActiveStepId] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [errorNotice, setErrorNotice] = useState<string | null>(null);
  const partsRef = useRef<HTMLDivElement | null>(null);
  const stageApi = useRef<ViewerStageApi | null>(null);

  const draft = draftQuery.data?.data;
  const etag = draftQuery.data?.etag ?? null;
  const knowledge = draft?.knowledge;
  const model = useMemo(() => readDraftModel(knowledge), [knowledge]);
  const parts = useMemo(() => readDraftParts(knowledge), [knowledge]);
  const steps = useMemo(() => readDraftSteps(knowledge), [knowledge]);
  const specs = useMemo(() => readDraftSpecs(knowledge), [knowledge]);
  const hotspots = useMemo(() => readDraftHotspots(knowledge), [knowledge]);
  const stepPoses = useMemo(() => readDraftStepPoses(knowledge), [knowledge]);
  const review = useMemo(() => readDraftReview(draft?.review), [draft?.review]);
  const missing = useMemo(() => readDraftMissing(knowledge), [knowledge]);
  const views = useMemo(() => hotspotViews(hotspots, model), [hotspots, model]);
  const displayHotspots = useMemo(() => usableHotspots(views), [views]);
  const checklist = useMemo(
    () =>
      publishChecklist({
        parts,
        steps,
        specs,
        hotspots: views,
        entityReviews: review.entities,
        modelReview: review.modelReview,
        model,
      }),
    [parts, steps, specs, views, review, model],
  );

  const documents = documentsQuery.data?.documents ?? [];
  const evidenceDocumentId = useMemo(() => {
    for (const step of steps) {
      for (const evidence of step.evidence) {
        if (evidence.documentId !== null) {
          return evidence.documentId;
        }
      }
    }
    for (const part of parts) {
      for (const evidence of part.evidence) {
        if (evidence.documentId !== null) {
          return evidence.documentId;
        }
      }
    }
    return null;
  }, [parts, steps]);
  const document =
    documents.find((entry) => entry.id === evidenceDocumentId) ?? documents[0] ?? null;
  const activeStep = steps.find((step) => step.id === activeStepId) ?? null;

  const focusParts = useCallback(() => {
    partsRef.current?.focus();
  }, []);

  const selectPart = useCallback(
    (partId: string) => {
      setSelectedPartId(partId);
      const confirmed = views.find(
        (hotspot) => hotspot.partId === partId && hotspot.usable,
      );
      setSelectedHotspotId(confirmed?.id ?? null);
      if (confirmed?.anchor !== null && confirmed !== undefined && confirmed.anchor !== null) {
        // 部件列表 → 3D：把对应热点居中（UI-046）。
        stageApi.current?.focusLocal(confirmed.anchor.positionLocal);
      }
    },
    [views],
  );

  const selectHotspot = useCallback(
    (hotspotId: string) => {
      setSelectedHotspotId(hotspotId);
      const hotspot = views.find((candidate) => candidate.id === hotspotId);
      if (hotspot !== undefined) {
        setSelectedPartId(hotspot.partId);
      }
    },
    [views],
  );

  /** 一次人工拾取：新建绑定或（待重新绑定时）把新锚点交给同一热点。 */
  const handlePick = useCallback(
    (pick: ViewerPickResult) => {
      if (model === null || etag === null) {
        return;
      }
      if (rebindingHotspotId !== null) {
        const partId = partIdOf(rebindingHotspotId, views);
        mutations.rebindHotspot(etag, {
          hotspots: {
            upsert: [hotspotRebindUpsert(rebindingHotspotId, partId, model, pick.local)],
          },
        });
        setRebindingHotspotId(null);
        setPickMode(false);
        return;
      }
      if (bindingPartId === null) {
        return;
      }
      mutations.createHotspot(etag, {
        hotspots: { upsert: [hotspotPickUpsert(bindingPartId, model, pick.local)] },
      });
      setBindingPartId(null);
      setPickMode(false);
    },
    [bindingPartId, etag, model, mutations, rebindingHotspotId, views],
  );

  if (draftQuery.isLoading) {
    return <Skeleton label="正在读取草稿…" rows={4} />;
  }
  if (draftQuery.isError) {
    const described = describeError(draftQuery.error);
    return (
      <div className="page-error" role="alert">
        <p>草稿读取失败：{described.message}</p>
        {described.requestId !== null && <p className="diagnostic-id">诊断 ID：{described.requestId}</p>}
        <Link to={`/items/${itemId}`}>返回物品概览</Link>
      </div>
    );
  }

  const pendingCounts = checklist.counts;
  const mutationError = mutations.lastError;

  return (
    <div>
      <h1>阅读与复核</h1>
      <p className="page-subtitle">
        校准工作区：确认知识（文字）、绑定热点与保存步骤视角（几何校准），再显式发布。
        生成完成不等于已发布，本页不提供自动发布入口。
      </p>
      {draft !== undefined && (
        <p className="page-note" data-testid="draft-context">
          草稿 {draft.id} · r{draft.revision} · 状态 {draft.status === "ready" ? "已标记复核完成" : "待复核"} · 未确认知识{" "}
          {pendingCounts.unreviewed} · 缺热点部件 {pendingCounts.missingHotspots} · 已失效热点{" "}
          {pendingCounts.staleHotspots}{" "}
          <button
            type="button"
            className="link-button"
            data-testid="refresh-draft"
            onClick={() => {
              // 刷新 = 重新读取服务端事实（不清空本地表单；412 提示也在这里清除）。
              mutations.clearError();
              setNotice(null);
              setErrorNotice(null);
              void draftQuery.refetch();
            }}
          >
            刷新草稿数据
          </button>
        </p>
      )}
      {missing.length > 0 && (
        <section className="notice-panel" aria-label="草稿缺项">
          <h2>草稿缺项</h2>
          <ul>
            {missing.map((entry) => (
              <li key={entry.code}>{entry.message}</li>
            ))}
          </ul>
        </section>
      )}
      {(notice !== null || errorNotice !== null || mutationError !== null) && (
        <div
          className={errorNotice !== null || mutationError !== null ? "page-error" : "notice-panel"}
          role={errorNotice !== null || mutationError !== null ? "alert" : "status"}
          data-testid="workspace-notice"
        >
          {notice !== null && <p>{notice}</p>}
          {errorNotice !== null && <p>{errorNotice}</p>}
          {mutationError !== null && <p>{mutationError}</p>}
        </div>
      )}
      {mutations.conflictRevision !== null && (
        <div className="page-error" role="alert" data-testid="conflict-panel">
          <p>
            该内容已被其他操作更新（当前 r{mutations.conflictRevision}）：刷新后重试，不会自动覆盖。
          </p>
          <button
            type="button"
            onClick={() => {
              mutations.clearError();
              void draftQuery.refetch();
            }}
          >
            刷新草稿
          </button>
        </div>
      )}

      <PageLayout
        rail={{
          id: "parts",
          label: "部件与热点",
          content: (
            <div ref={partsRef} tabIndex={-1} data-testid="parts-panel">
              <PartsPanel
                parts={parts}
                views={views}
                review={review.entities}
                selectedPartId={selectedPartId}
                narrow={narrow}
                pickMode={pickMode}
                bindingPartId={bindingPartId}
                onSelect={selectPart}
                onStartBinding={(partId) => {
                  setBindingPartId(partId);
                  setRebindingHotspotId(null);
                  setPickMode(true);
                }}
                onCancelBinding={() => {
                  setBindingPartId(null);
                  setPickMode(false);
                }}
                onStartRebinding={(hotspotId) => {
                  setRebindingHotspotId(hotspotId);
                  setBindingPartId(null);
                  setPickMode(true);
                }}
                onUnbind={(hotspotId) => {
                  if (etag !== null) {
                    setNotice("已解绑热点（部件回到未绑定）");
                    mutations.createHotspot(etag, { hotspots: hotspotRemove(hotspotId) });
                  }
                }}
                onMarkTextOnly={(partId) => {
                  if (etag !== null) {
                    setNotice("已标记为「仅文本条目」（保留在发布内容并明显标识）");
                    mutations.updateEntities(etag, { entities: textOnlyPatch(partId) });
                  }
                }}
              />
            </div>
          ),
        }}
        aside={{
          id: "steps",
          label: "步骤与原文",
          content: (
            <div data-testid="steps-panel">
              <StepsPanel
                steps={steps}
                poses={stepPoses}
                activeStepId={activeStep?.id ?? null}
                narrow={narrow}
                onActivate={(stepId) => setActiveStepId(stepId)}
                onJumpToPage={(page) => setPageNumber(Math.max(1, page))}
                onSavePose={(stepId) => {
                  const pose = stageApi.current?.pose() ?? null;
                  if (pose === null || etag === null) {
                    setErrorNotice("当前视角不可用（模型未加载）：无法保存步骤视角");
                    return;
                  }
                  setNotice("已保存该步骤视角（相对 asset-root；视角是观察位置，不是机械动作）");
                  mutations.savePose(etag, stepId, {
                    positionLocal: [...pose.positionLocal],
                    targetLocal: [...pose.targetLocal],
                    upLocal: [...pose.upLocal],
                    fov: pose.fov,
                  });

                }}
                onApplyPose={(pose) => {
                  stageApi.current?.applyPose(pose);
                }}
                onClearPose={(stepId) => {
                  if (etag !== null) {
                    setNotice("已清除该步骤视角");
                    mutations.clearPose(etag, stepId);
                  }
                }}
              />
              <Suspense
                fallback={
                  <p className="original-panel__message" role="status">
                    正在加载原文模块…
                  </p>
                }
              >
                <OriginalDocumentPanel
                  assetId={document?.sourceAssetId ?? null}
                  pageNumber={pageNumber}
                  onPageChange={(next) => setPageNumber(Math.max(1, next))}
                  onPageCount={setPageCount}
                />
              </Suspense>
              <KnowledgeReviewPanel
                parts={parts}
                steps={steps}
                specs={specs}
                entityReviews={review.entities}
                modelReview={review.modelReview}
                model={model}
                modelLoaded={mutations.modelReady}
                narrow={narrow}
                onDeclareModelReady={() => {
                  if (etag !== null) {
                    setNotice("已声明：已在浏览器成功打开此模型（服务器记录声明时间）");
                    mutations.updateModelReview(etag, {
                      modelReview: {
                        loaded: true,
                        userConfirmed: review.modelReview?.userConfirmed === true,
                      },
                    });
                  }
                }}
                onDeclareModelConfirmed={() => {
                  if (etag !== null) {
                    setNotice("已声明：我已核对模型与资料一致");
                    mutations.updateModelReview(etag, {
                      modelReview: { loaded: true, userConfirmed: true },
                    });
                  }
                }}
                onSetEntityReview={(entityId, decision) => {
                  if (etag !== null) {
                    setNotice(
                      decision === "confirmed"
                        ? "已确认事实（文字确认，与几何校准分开）"
                        : "已取消确认",
                    );
                    mutations.updateEntities(etag, {
                      entities: { [entityId]: { reviewStatus: decision } },
                    });
                  }
                }}
                onSaveEntityEdit={(entityId, fields) => {
                  if (etag !== null) {
                    setNotice("已保存人工修订（原文本与出处保留，供应商快照未修改）");
                    mutations.updateEntities(etag, {
                      entities: { [entityId]: { reviewStatus: "confirmed", userEdited: fields } },
                    });
                  }
                }}
              />
            </div>
          ),
        }}
      >
        <ViewerPanel
          model={model}
          hotspots={displayHotspots}
          pickMode={pickMode && !narrow}
          selectedHotspotId={selectedHotspotId}
          onPick={handlePick}
          onHotspotSelect={selectHotspot}
          onUseTextPath={focusParts}
          onModelReady={() => mutations.setModelReady(true)}
          apiRef={stageApi}
        />
        <PickToolbar
          pickMode={pickMode}
          narrow={narrow}
          bindingPartId={bindingPartId}
          rebinding={rebindingHotspotId !== null}
          onToggle={() => setPickMode((value) => !value)}
          onCancel={() => {
            setPickMode(false);
            setBindingPartId(null);
            setRebindingHotspotId(null);
          }}
        />
        <PublishPanel
          checklist={checklist}
          etag={etag}
          itemId={itemId}
          draftId={draftId}
          narrow={narrow}
          onPublished={(releaseId, replayed) => {
            setNotice(
              replayed
                ? `发布请求已重放：返回同一发布版本（${releaseId}）`
                : `已发布不可变版本（${releaseId}）：之后对草稿的修改不会改变它`,
            );
            setErrorNotice(null);
            void draftQuery.refetch();
          }}
          onIssues={(message) => setErrorNotice(message)}
          onRefetchRequested={() => void draftQuery.refetch()}
        />
        <p className="page-note" data-testid="reader-context">
          草稿 {draft?.id ?? ""} · 完备性 {draft?.completeness ?? "?"} · 页码 {pageNumber}
          {pageCount !== null ? ` / ${pageCount}` : ""}（1-based）
        </p>
      </PageLayout>
    </div>
  );
}

function partIdOf(hotspotId: string, views: { id: string; partId: string }[]): string {
  return views.find((hotspot) => hotspot.id === hotspotId)?.partId ?? "";
}

/** 拾取/旋转模式工具栏（UI-048：两个模式互斥，可键盘切换）。 */
function PickToolbar({
  pickMode,
  narrow,
  bindingPartId,
  rebinding,
  onToggle,
  onCancel,
}: {
  pickMode: boolean;
  narrow: boolean;
  bindingPartId: string | null;
  rebinding: boolean;
  onToggle: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="pick-toolbar" role="group" aria-label="热点校准模式">
      <button
        type="button"
        aria-pressed={pickMode}
        disabled={narrow}
        onClick={onToggle}
        data-testid="pick-mode-toggle"
      >
        {pickMode ? "切换到旋转模式" : "进入拾取模式（点击模型表面建点）"}
      </button>
      {pickMode && (
        <button type="button" onClick={onCancel}>
          取消拾取
        </button>
      )}
      <span className="pick-toolbar__hint" data-testid="pick-hint">
        {narrow
          ? "热点校准需要在 ≥768px 的桌面窗口完成；此处只能查看只读热点与部件列表。视角保存同样属于校准。"
          : rebinding
            ? "重新绑定：请在模型上点选新的位置（同一热点将改用当前模型的锚点）"
            : bindingPartId !== null
              ? "拾取模式：请点击模型表面完成绑定；拖动仍是旋转（不会误建点）"
              : "热点绑定以鼠标点选为主（部件列表是热点的文字替代路径）"}
      </span>
      <span className="pick-toolbar__note">
        视角是观察位置，不是机械动作。
      </span>
    </div>
  );
}

/** 部件列表（3D 热点的文字替代路径；UI-045/UI-046/UI-049）。 */
function PartsPanel({
  parts,
  views,
  review,
  selectedPartId,
  narrow,
  pickMode,
  bindingPartId,
  onSelect,
  onStartBinding,
  onCancelBinding,
  onStartRebinding,
  onUnbind,
  onMarkTextOnly,
}: {
  parts: readonly { id: string; name: string; description: string }[];
  views: ReturnType<typeof hotspotViews>;
  review: Readonly<Record<string, { reviewStatus: string | null; userEdited: unknown; textOnly: boolean }>>;
  selectedPartId: string | null;
  narrow: boolean;
  pickMode: boolean;
  bindingPartId: string | null;
  onSelect: (partId: string) => void;
  onStartBinding: (partId: string) => void;
  onCancelBinding: () => void;
  onStartRebinding: (hotspotId: string) => void;
  onUnbind: (hotspotId: string) => void;
  onMarkTextOnly: (partId: string) => void;
}) {
  if (parts.length === 0) {
    return <EmptyNote>该草稿没有部件列表（知识分支未完成或未产出部件）。</EmptyNote>;
  }
  const staleHotspots = views.filter((hotspot) => hotspot.status === "stale");
  return (
    <section aria-label="部件列表">
      <h2>部件</h2>
      <ul className="entity-list" data-testid="parts-list">
        {parts.map((part) => {
          const summary = summarizePartHotspots(views, part.id);
          const entry = review[part.id];
          const reviewed = entry !== undefined && (entry.reviewStatus === "confirmed" || entry.userEdited != null);
          const textOnly = entry?.textOnly === true;
          const bound = summary.confirmed > 0;
          return (
            <li key={part.id} data-testid={`part-row-${part.id}`}>
              <button
                type="button"
                aria-current={selectedPartId === part.id}
                onClick={() => onSelect(part.id)}
              >
                {part.name}
              </button>
              <span className="status-label" data-testid={`part-review-${part.id}`}>
                {reviewed ? "已确认（文字）" : "待确认（文字）"}
              </span>
              <span className="status-label" data-testid={`part-hotspot-${part.id}`}>
                {textOnly
                  ? "仅文本条目（不进入 3D）"
                  : bound
                    ? `热点已确认 ${summary.confirmed}`
                    : "未绑定"}
                {summary.stale > 0 ? `（${summary.stale} 个已失效）` : ""}
              </span>
              {part.description !== "" && <p>{part.description}</p>}
              <div className="row-actions">
                {!textOnly && !bound && (
                  <button
                    type="button"
                    disabled={narrow}
                    title={narrow ? "热点校准需要在 ≥768px 的桌面窗口完成" : undefined}
                    onClick={() => (bindingPartId === part.id ? onCancelBinding() : onStartBinding(part.id))}
                  >
                    {bindingPartId === part.id ? "取消绑定" : "绑定热点"}
                  </button>
                )}
                {!textOnly && !bound && (
                  <button
                    type="button"
                    onClick={() => onMarkTextOnly(part.id)}
                    title="保留为文字条目：发布时保留并明显标识（先确认该部件）"
                  >
                    标记仅文本条目
                  </button>
                )}
                {views
                  .filter((hotspot) => hotspot.partId === part.id && hotspot.usable)
                  .map((hotspot) => (
                    <button
                      key={hotspot.id}
                      type="button"
                      onClick={() => onUnbind(hotspot.id)}
                    >
                      解绑热点
                    </button>
                  ))}
              </div>
            </li>
          );
        })}
      </ul>
      {staleHotspots.length > 0 && (
        <section className="stale-block" aria-label="已失效热点" data-testid="stale-hotspots">
          <h3>已失效热点（旧模型版本）</h3>
          <p>
            这些绑定属于旧模型版本，不再作为有效热点显示，也不能用于发布；旧锚点仅保留作解释。
          </p>
          <ul className="entity-list">
            {staleHotspots.map((hotspot) => (
              <li key={hotspot.id}>
                <span>部件 {hotspot.partId} 的旧绑定（{hotspot.anchor?.modelSha256.slice(0, 8)}…）</span>
                <button
                  type="button"
                  disabled={narrow}
                  title={narrow ? "热点校准需要在 ≥768px 的桌面窗口完成" : undefined}
                  onClick={() => onStartRebinding(hotspot.id)}
                >
                  在新模型上重新绑定
                </button>
              </li>
            ))}
          </ul>
          <p className="page-note">界面不提供把 stale 绑定"强制确认"的入口。</p>
        </section>
      )}
      {pickMode && (
        <p className="page-note" data-testid="pick-mode-active" role="status">
          拾取模式已开启。
        </p>
      )}
    </section>
  );
}

/** 步骤导航 + 视角（UI-050：保存与回到视角分开、窄屏禁用）。 */
function StepsPanel({
  steps,
  poses,
  activeStepId,
  narrow,
  onActivate,
  onJumpToPage,
  onSavePose,
  onApplyPose,
  onClearPose,
}: {
  steps: readonly {
    id: string;
    title: string;
    orderedActions: readonly string[];
    partIds: readonly string[];
    safetyNotes: readonly string[];
    evidence: readonly { pageNumber: number }[];
  }[];
  poses: Record<string, CameraPose>;
  activeStepId: string | null;
  narrow: boolean;
  onActivate: (stepId: string) => void;
  onJumpToPage: (pageNumber: number) => void;
  onSavePose: (stepId: string) => void;
  onApplyPose: (pose: CameraPose) => void;
  onClearPose: (stepId: string) => void;
}) {
  const [index, setIndex] = useState(0);
  if (steps.length === 0) {
    return <EmptyNote>该草稿没有步骤列表（知识分支未完成或未产出步骤）。</EmptyNote>;
  }
  const safeIndex = Math.min(Math.max(index, 0), steps.length - 1);
  const current = steps[safeIndex] as (typeof steps)[number];
  const pose = poses[current.id];
  const go = (next: number) => {
    const bounded = Math.min(Math.max(next, 0), steps.length - 1);
    setIndex(bounded);
    const target = steps[bounded] as (typeof steps)[number];
    onActivate(target.id);
    const evidence = target.evidence[0];
    if (evidence !== undefined) {
      onJumpToPage(evidence.pageNumber);
    }
  };
  return (
    <section aria-label="步骤列表" data-testid="step-navigation">
      <h2>步骤</h2>
      <div className="step-nav" role="group" aria-label="步骤导航">
        <button type="button" disabled={safeIndex <= 0} onClick={() => go(safeIndex - 1)}>
          上一步
        </button>
        <span data-testid="step-position">
          第 {safeIndex + 1} / {steps.length} 步
        </span>
        <button type="button" disabled={safeIndex >= steps.length - 1} onClick={() => go(safeIndex + 1)}>
          下一步
        </button>
      </div>
      <ol className="entity-list" data-testid="steps-list">
        {steps.map((step, stepIndex) => (
          <li key={step.id} data-testid={`step-row-${step.id}`}>
            <button
              type="button"
              aria-current={activeStepId === step.id || stepIndex === safeIndex}
              onClick={() => {
                setIndex(stepIndex);
                onActivate(step.id);
                const evidence = step.evidence[0];
                if (evidence !== undefined) {
                  onJumpToPage(evidence.pageNumber);
                }
              }}
            >
              {step.title}
            </button>
            {stepIndex === safeIndex && (
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
                  <p className="step-parts">引用部件：{step.partIds.join("、")}</p>
                )}
                {step.evidence.length > 0 && (
                  <p className="step-evidence">
                    原文：
                    {step.evidence.map((evidence, evidenceIndex) => (
                      <button
                        key={`${step.id}-page-${evidenceIndex}`}
                        type="button"
                        onClick={() => onJumpToPage(evidence.pageNumber)}
                      >
                        第 {evidence.pageNumber} 页
                      </button>
                    ))}
                  </p>
                )}
                <div className="row-actions">
                  <button
                    type="button"
                    disabled={narrow}
                    title={narrow ? "视角保存属于校准：需要在 ≥768px 的桌面窗口完成" : undefined}
                    onClick={() => onSavePose(step.id)}
                  >
                    保存当前视角
                  </button>
                  {pose !== undefined && (
                    <>
                      <button type="button" onClick={() => onApplyPose(pose)}>
                        回到该视角
                      </button>
                      <button type="button" disabled={narrow} onClick={() => onClearPose(step.id)}>
                        清除视角
                      </button>
                    </>
                  )}
                  <span className="status-label" data-testid={`step-pose-${step.id}`}>
                    {pose === undefined ? "未设置视角" : "视角已保存"}
                  </span>
                </div>
              </div>
            )}
          </li>
        ))}
      </ol>
    </section>
  );
}

export default CalibrationWorkspace;
