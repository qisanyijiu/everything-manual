import { Suspense, lazy, useState } from "react";
import type { DraftEvidence } from "./draft-view";
import type { PanelNavigation } from "../shell/PageLayout";

const OriginalDocumentPanel = lazy(() => import("./OriginalDocumentPanel"));
export interface ReaderDocument { readonly id: string; readonly title: string; readonly sourceAssetId: string; }
interface Source { readonly panelId: string; readonly focusId: string; readonly narrowFocusId?: string; }
export const evidenceId = (entityId: string, index: number) => `evidence-${entityId}-${index}`;

/** Selection lives above responsive layouts, so mounting another panel cannot reset it. */
export function useDocumentNavigation(documents: readonly ReaderDocument[]) {
  const [selectedId, setSelectedId] = useState<string | null | undefined>();
  const [pageNumber, setPageNumber] = useState(1);
  const [pageCount, setPageCount] = useState<number | null>(null);
  const [source, setSource] = useState<Source | null>(null);
  const [navigation, setNavigation] = useState<PanelNavigation | null>(null);
  const selected = selectedId === undefined ? documents[0] : documents.find((entry) => entry.id === selectedId);
  const navigate = (target: Source) => setNavigation((old) => ({ ...target, serial: (old?.serial ?? 0) + 1 }));
  const selectEvidence = (evidence: DraftEvidence) => {
    setSelectedId(evidence.documentId);
    setPageNumber(evidence.pageNumber);
    setPageCount(null);
  };
  const openEvidence = (evidence: DraftEvidence, origin: Source) => {
    selectEvidence(evidence);
    setSource(origin);
    navigate({ panelId: "original", focusId: "original-heading" });
  };
  const returnToSource = source === null ? undefined : () => {
    navigate(source);
    setSource(null);
  };
  const panel = (
    <section className="original-section" aria-labelledby="original-heading">
      <h2 id="original-heading" tabIndex={-1}>原文 · {selected?.title ?? "出处不可用"}</h2>
      {returnToSource !== undefined && <button type="button" onClick={returnToSource}>返回出处</button>}
      {documents.length > 1 && <div className="original-section__select"><label htmlFor="original-document-select">原件</label>
        <select id="original-document-select" value={selected?.id ?? ""} onChange={(event) => {
          setSelectedId(event.target.value); setPageNumber(1); setPageCount(null);
        }}>
          {selected === undefined && <option value="" disabled>此出处的原件不可用</option>}
          {documents.map((entry) => <option key={entry.id} value={entry.id}>{entry.title}</option>)}
        </select>
      </div>}
      {selected === undefined ? <p role="alert">{selectedId === undefined ? "该物品还没有绑定说明书原件。" : `此出处的原件不可用 · 文档 ${selectedId ?? "未提供"} · 第 ${pageNumber} 页`}</p> :
        <Suspense fallback={<p role="status">正在加载原文模块…</p>}>
          <OriginalDocumentPanel key={selected.id} assetId={selected.sourceAssetId} pageNumber={pageNumber}
            onPageChange={setPageNumber} onPageCount={setPageCount} fromEvidence={source !== null} />
        </Suspense>}
    </section>
  );
  return { pageNumber, pageCount, navigation, navigate, panel, openEvidence, selectEvidence, returnToSource };
}

export function EvidenceLinks({ entityId, evidence, documents, onOpen }: {
  readonly entityId: string;
  readonly evidence: readonly DraftEvidence[];
  readonly documents: readonly ReaderDocument[];
  readonly onOpen: (evidence: DraftEvidence, focusId: string) => void;
}) {
  return <div className="step-evidence">{evidence.map((entry, index) => {
    const document = documents.find((candidate) => candidate.id === entry.documentId);
    const id = evidenceId(entityId, index);
    return <div key={id}>
      <button type="button" id={id} onClick={() => onOpen(entry, id)}>查看出处 · {document?.title ?? entry.documentId ?? "未提供文档"} · 第 {entry.pageNumber} 页</button>
      {document === undefined && <p className="page-note">此出处的原件不可用</p>}
    </div>;
  })}</div>;
}
