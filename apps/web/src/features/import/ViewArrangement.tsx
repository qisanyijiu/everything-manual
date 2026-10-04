/**
 * 视图排列（ADR-044）：从说明书 PDF 拆出的候选图 + 已有照片 → 拖拽到 5 个槽位确定最终排列。
 *
 * 交互：
 * - 「从说明书提取候选图」：在浏览器内用 PDF.js 拆出图片区域，上传为照片资产并登记候选；
 *   服务端由说明书 AI 给出建议视图（只是建议）；
 * - 拖拽：托盘 ↔ 槽位、槽位 ↔ 槽位（交换）、槽位 → 托盘（移出）；
 * - 删除：候选右上角「删除」（软删除，可在本页撤销）；槽位里的「移出」只移回托盘；
 * - 键盘/无鼠标替代：每张卡片的「放到…」下拉框，等效于拖拽；
 * - 「按建议填入空槽」：按置信度把候选放进空槽；
 * - 「保存排列」：一次性提交全部槽位（`PUT /photos/arrangement`），互换视图不会冲突。
 */

import { useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { describeError } from "../../api/client";
import {
  arrangePhotos,
  assetContentUrl,
  createViewCandidate,
  fetchAssetContent,
  listViewCandidates,
  setViewCandidateDismissed,
  type PhotoDto,
  type ViewCandidateDto,
} from "../../api/endpoints";
import { useNotify } from "../../components/notifications";
import { itemKeys, useItemDocuments } from "../library/items";
import { workflowKeys } from "../library/workflow";
import { EMPTY_SLOTS, autoFill, drop, removeCard, sameArrangement, slotOf, slotsPayload, type CardRef, type DropTarget, type Slots } from "./arrangement";
import { uploadAsset } from "./upload";
import { VIEW_LABELS, VIEW_ORDER, type ViewSlot } from "./views";

const DRAG_MIME = "application/x-em-card";

interface Card extends CardRef {
  readonly label: string;
  readonly suggestedView: string | null;
  readonly confidence: number | null;
  readonly note: string | null;
  readonly pageNumber: number | null;
}

function cardFromCandidate(c: ViewCandidateDto): Card {
  return {
    assetId: c.assetId,
    candidateId: c.id,
    label: c.pageNumber ? `第 ${c.pageNumber} 页` : "上传",
    suggestedView: c.suggestedView ?? null,
    confidence: c.confidence ?? null,
    note: c.note ?? null,
    pageNumber: c.pageNumber ?? null,
  };
}

function slotsFromPhotos(photos: readonly PhotoDto[]): Slots {
  const next: Record<ViewSlot, CardRef | null> = { ...EMPTY_SLOTS };
  for (const photo of photos) {
    const view = photo.view as ViewSlot;
    if (VIEW_ORDER.includes(view) && next[view] === null) {
      next[view] = { assetId: photo.assetId, candidateId: null };
    }
  }
  return next;
}

export interface ViewArrangementProps {
  readonly itemId: string;
  readonly photos: readonly PhotoDto[];
  /** 每次拖拽/交换/删除后通知父组件当前草稿槽位（含未保存），用于实时更新缺项提示。 */
  readonly onSlotsChange?: (draftViews: ReadonlySet<string>) => void;
  /** 引用：父组件可调用 `saveRef.current?.()` 在导航前自动保存。 */
  readonly saveRef?: { current: (() => Promise<void>) | null };
}

export function ViewArrangement({ itemId, photos, onSlotsChange, saveRef }: ViewArrangementProps) {
  const queryClient = useQueryClient();
  const notify = useNotify();
  const candidatesQuery = useQuery({ queryKey: ["view-candidates", itemId], queryFn: () => listViewCandidates(itemId) });
  const documentsQuery = useItemDocuments(itemId);
  const saved = useMemo(() => slotsFromPhotos(photos), [photos]);
  const [slots, setSlots] = useState<Slots>(saved);
  const [dragging, setDragging] = useState<string | null>(null);
  const [hover, setHover] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastDismissed, setLastDismissed] = useState<ViewCandidateDto | null>(null);
  const [suggestViews, setSuggestViews] = useState(false);

  // 服务端照片变化（保存成功/其它页面修改）时同步；本地有未保存修改时不覆盖。
  const [dirty, setDirty] = useState(false);
  useEffect(() => {
    if (!dirty) {
      setSlots(saved);
      onSlotsChange?.(new Set(VIEW_ORDER.filter((view) => saved[view] !== null)));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [saved, dirty]);
  useEffect(() => {
    if (saveRef !== undefined) {
      saveRef.current = dirty ? saveNow : null;
    }
    return () => { if (saveRef !== undefined) { saveRef.current = null; } };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dirty]);

  const candidates = useMemo(() => candidatesQuery.data ?? [], [candidatesQuery.data]);
  const cards = useMemo(() => {
    const map = new Map<string, Card>();
    for (const c of candidates) {
      map.set(c.assetId, cardFromCandidate(c));
    }
    for (const photo of photos) {
      if (!map.has(photo.assetId)) {
        map.set(photo.assetId, { assetId: photo.assetId, candidateId: null, label: "已有照片", suggestedView: null, confidence: null, note: null, pageNumber: null });
      }
    }
    return map;
  }, [candidates, photos]);
  const tray = [...cards.values()].filter((card) => slotOf(slots, card.assetId) === null);
  // 说明书 AI 判为"不像产品视图"的候选折叠起来（仍可展开拖用）；按建议置信度排序。
  const isRejected = (card: Card): boolean => card.suggestedView === null && (card.note ?? "").startsWith("不像产品视图");
  const trayMain = tray.filter((card) => !isRejected(card)).sort((a, b) => (b.confidence ?? 0) - (a.confidence ?? 0));
  const trayRejected = tray.filter(isRejected);

  const autoSaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const slotsRef = useRef(slots);
  const update = (next: Slots): void => {
    setSlots(next);
    slotsRef.current = next;
    const isDirty = !sameArrangement(next, saved);
    setDirty(isDirty);
    onSlotsChange?.(new Set(VIEW_ORDER.filter((view) => next[view] !== null)));
    if (isDirty) {
      if (autoSaveTimer.current !== null) { clearTimeout(autoSaveTimer.current); }
      autoSaveTimer.current = setTimeout(() => { autoSaveTimer.current = null; void autoSave(); }, 800);
    }
  };
  useEffect(() => () => { if (autoSaveTimer.current !== null) { clearTimeout(autoSaveTimer.current); } }, []);
  const place = (assetId: string, target: DropTarget): void => {
    const card = cards.get(assetId);
    if (card !== undefined) {
      update(drop(slots, card, target));
    }
  };

  const onDragStart = (event: DragEvent, assetId: string): void => {
    event.dataTransfer.setData(DRAG_MIME, assetId);
    event.dataTransfer.effectAllowed = "move";
    setDragging(assetId);
  };
  const onDrop = (event: DragEvent, target: DropTarget): void => {
    event.preventDefault();
    const assetId = event.dataTransfer.getData(DRAG_MIME) || dragging;
    setHover(null);
    setDragging(null);
    if (assetId) {
      place(assetId, target);
    }
  };
  const dropZone = (key: string, target: DropTarget) => ({
    onDragOver: (event: DragEvent) => {
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      setHover(key);
    },
    onDragLeave: () => setHover((current) => (current === key ? null : current)),
    onDrop: (event: DragEvent) => onDrop(event, target),
  });

  async function refresh(): Promise<void> {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["view-candidates", itemId] }),
      queryClient.invalidateQueries({ queryKey: itemKeys.photos(itemId) }),
      queryClient.invalidateQueries({ queryKey: workflowKeys.root }),
    ]);
  }

  async function extract(): Promise<void> {
    const document = documentsQuery.data?.documents[0];
    if (document === undefined) {
      setError("还没有绑定说明书 PDF：请先在第 2 步上传说明书。");
      return;
    }
    setBusy("正在打开说明书…");
    setError(null);
    try {
      const [{ openPdfDocument }, { extractFigures }] = await Promise.all([import("./pdf/vendor"), import("./pdf/extract")]);
      const bytes = await fetchAssetContent(document.sourceAssetId);
      const pdf = await openPdfDocument(new Uint8Array(bytes.bytes));
      try {
        const pages = Array.from({ length: Math.min(pdf.numPages, 40) }, (_, i) => i + 1);
        const figures = await extractFigures({
          pdf,
          pageNumbers: pages,
          onProgress: (page) => setBusy(`正在拆分第 ${page} / ${pages.length} 页…`),
        });
        if (figures.length === 0) {
          notify("没有在说明书里找到像产品视图的图片，可以手动上传照片。");
          return;
        }
        // 并发上传 + 判断（每批 4 张；判断走服务端说明书 AI，单张约数秒）。
        let done = 0;
        const queue = figures.map((figure, index) => ({ figure, index }));
        const worker = async (): Promise<void> => {
          for (let next = queue.shift(); next !== undefined; next = queue.shift()) {
            const { figure, index } = next;
            const asset = await uploadAsset(itemId, "photo", figure.blob, `page-${figure.pageNumber}-${index + 1}.jpg`);
            await createViewCandidate(itemId, { assetId: asset.id, documentId: document.id, pageNumber: figure.pageNumber, source: figure.source, classify: suggestViews });
            done += 1;
            setBusy(`${suggestViews ? "正在判断视图" : "正在保存候选图"}（${done} / ${figures.length}）…`);
            if (done % 4 === 0) {
              void queryClient.invalidateQueries({ queryKey: ["view-candidates", itemId] });
            }
          }
        };
        setBusy(`${suggestViews ? "正在判断视图" : "正在保存候选图"}（0 / ${figures.length}）…`);
        await Promise.all(Array.from({ length: Math.min(4, figures.length) }, () => worker()));
        notify(`已从说明书拆出 ${figures.length} 张候选图：拖到下方槽位确定视图，或点「按建议填入空槽」。`);
      } finally {
        await pdf.loadingTask.destroy();
      }
    } catch (failure) {
      setError(`提取候选图失败：${describeError(failure).message}`);
    } finally {
      setBusy(null);
      await refresh();
    }
  }

  async function dismiss(card: Card): Promise<void> {
    if (card.candidateId === null) {
      update(removeCard(slots, card.assetId));
      return;
    }
    setError(null);
    try {
      await setViewCandidateDismissed(itemId, card.candidateId, true);
      update(removeCard(slots, card.assetId));
      setLastDismissed(candidates.find((c) => c.id === card.candidateId) ?? null);
      await queryClient.invalidateQueries({ queryKey: ["view-candidates", itemId] });
    } catch (failure) {
      setError(`删除失败：${describeError(failure).message}`);
    }
  }

  async function undoDismiss(): Promise<void> {
    if (lastDismissed === null) {
      return;
    }
    await setViewCandidateDismissed(itemId, lastDismissed.id, false);
    setLastDismissed(null);
    await queryClient.invalidateQueries({ queryKey: ["view-candidates", itemId] });
  }

  async function saveNow(showNotice = true): Promise<void> {
    if (autoSaveTimer.current !== null) { clearTimeout(autoSaveTimer.current); autoSaveTimer.current = null; }
    setBusy("正在保存排列…");
    setError(null);
    try {
      await arrangePhotos(itemId, slotsPayload(slotsRef.current));
      setDirty(false);
      onSlotsChange?.(new Set(VIEW_ORDER.filter((view) => slotsRef.current[view] !== null)));
      if (showNotice) { notify("视图排列已保存。"); }
      await refresh();
    } catch (failure) {
      setError(`保存失败：${describeError(failure).message}`);
    } finally {
      setBusy(null);
    }
  }
  async function autoSave(): Promise<void> {
    if (!sameArrangement(slotsRef.current, saved)) { await saveNow(false); }
  }

  const renderCard = (card: Card, where: "tray" | ViewSlot) => {
    const suggestion = card.suggestedView ? VIEW_LABELS[card.suggestedView as ViewSlot] : null;
    return (
      <div
        key={card.assetId}
        className={`arrange-card${dragging === card.assetId ? " is-dragging" : ""}`}
        draggable
        onDragStart={(event) => onDragStart(event, card.assetId)}
        onDragEnd={() => setDragging(null)}
        data-testid={`arrange-card-${card.assetId}`}
      >
        <img src={assetContentUrl(card.assetId)} alt={`候选图（${card.label}${suggestion ? `，建议${suggestion}` : ""}）`} draggable={false} />
        <div className="arrange-card__meta">
          <span>{card.label}</span>
          {suggestion !== null && (
            <span className="status-label" title={card.note ?? undefined}>
              建议：{suggestion}
              {card.confidence !== null ? ` ${Math.round(card.confidence * 100)}%` : ""}
            </span>
          )}
          {suggestion === null && card.note !== null && <span className="arrange-card__note" title={card.note}>{card.note}</span>}
        </div>
        <div className="arrange-card__tools">
          <label className="visually-hidden" htmlFor={`place-${card.assetId}`}>把这张图放到</label>
          <select
            id={`place-${card.assetId}`}
            value={where}
            onChange={(event) => place(card.assetId, event.target.value === "tray" ? { kind: "tray" } : { kind: "slot", view: event.target.value as ViewSlot })}
          >
            <option value="tray">候选区</option>
            {VIEW_ORDER.map((view) => (
              <option key={view} value={view}>
                {VIEW_LABELS[view]}
              </option>
            ))}
          </select>
          {where === "tray" ? (
            <button type="button" className="link-button" onClick={() => void dismiss(card)} data-testid={`dismiss-${card.assetId}`}>
              删除
            </button>
          ) : (
            <button type="button" className="link-button" onClick={() => place(card.assetId, { kind: "tray" })}>
              移出
            </button>
          )}
        </div>
      </div>
    );
  };

  return (
    <section className="arrange" aria-labelledby="arrange-title" data-testid="view-arrangement">
      <div className="arrange__head">
        <h2 id="arrange-title">从说明书挑选视图</h2>
        <div className="arrange__actions">
          <button type="button" className="button" onClick={() => void extract()} disabled={busy !== null} data-testid="extract-candidates">
            从说明书提取候选图
          </button>
          <button type="button" className="button" onClick={() => update(autoFill(slots, candidates))} disabled={busy !== null || candidates.length === 0} data-testid="autofill">
            按建议填入空槽
          </button>
          <button type="button" className="button" onClick={() => void saveNow()} disabled={busy !== null || !dirty} data-testid="save-arrangement">
            立即保存
          </button>
        </div>
      </div>
      <label className="field__hint">
        <input
          type="checkbox"
          checked={suggestViews}
          disabled={busy !== null}
          onChange={(event) => setSuggestViews(event.target.checked)}
          data-testid="classify-view-candidates"
        />{" "}
        使用说明书 AI 建议视图（会发送候选图片，可能产生费用）
      </label>
      {dirty && <p className="status-note" data-testid="arrangement-unsaved" role="status">排列有修改，将在操作停止后自动保存…</p>}
      <p className="field__hint">
        拖动图片到下方槽位；拖到已占用的槽会互换；拖回候选区即移出。也可以用每张图下方的下拉框完成同样的操作。建议视图来自说明书 AI，只作参考。
      </p>
      {busy !== null && <p role="status" className="status-note">{busy}</p>}
      {error !== null && <p role="alert" className="field__error">{error}</p>}
      {lastDismissed !== null && (
        <p role="status" className="status-note">
          已删除一张候选图。<button type="button" className="link-button" onClick={() => void undoDismiss()}>撤销</button>
        </p>
      )}

      <ol className="arrange-slots">
        {VIEW_ORDER.map((view) => {
          const card = slots[view] ? cards.get(slots[view].assetId) : undefined;
          return (
            <li
              key={view}
              className={`arrange-slot${hover === view ? " is-hover" : ""}${card ? " is-filled" : ""}`}
              {...dropZone(view, { kind: "slot", view })}
              data-testid={`arrange-slot-${view}`}
              aria-label={`${VIEW_LABELS[view]}槽位${card ? "（已放置）" : "（空）"}`}
            >
              <h3>
                {VIEW_LABELS[view]}
                {view === "front" && <span className="view-slot__required">必需</span>}
                {view === "detail" && <span className="field__hint">不发送给 Tripo</span>}
              </h3>
              {card ? renderCard(card, view) : <p className="arrange-slot__empty">拖到这里</p>}
            </li>
          );
        })}
      </ol>

      <div
        className={`arrange-tray${hover === "tray" ? " is-hover" : ""}`}
        {...dropZone("tray", { kind: "tray" })}
        data-testid="arrange-tray"
        aria-label="候选区"
      >
        <h3>候选区（{trayMain.length}）</h3>
        {candidatesQuery.isPending && <p className="field__hint">正在读取候选图…</p>}
        {!candidatesQuery.isPending && tray.length === 0 && (
          <p className="field__hint">没有待选图。点「从说明书提取候选图」，或在下方逐个视图上传照片。</p>
        )}
        <div className="arrange-tray__grid">{trayMain.map((card) => renderCard(card, "tray"))}</div>
        {trayRejected.length > 0 && (
          <details className="arrange-tray__rejected" data-testid="rejected-candidates">
            <summary>其余 {trayRejected.length} 张不像产品视图（文字、表格、二维码等），展开查看</summary>
            <div className="arrange-tray__grid">{trayRejected.map((card) => renderCard(card, "tray"))}</div>
          </details>
        )}
      </div>
    </section>
  );
}
