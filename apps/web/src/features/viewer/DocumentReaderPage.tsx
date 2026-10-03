import { useReaderDocuments } from "./reader-documents";
import { Link, useParams, useSearchParams } from "react-router";
import { describeError } from "../../api/client";
import { Skeleton } from "../../components/Skeleton";

import { OriginalDocumentPanel } from "./OriginalDocumentPanel";

/** Original reading only consumes the bound document and protected asset; no preparation/job. */
export default function DocumentReaderPage() {
  const { itemId = "", documentId = "" } = useParams();
  const [search, setSearch] = useSearchParams();
  const documents = useReaderDocuments(itemId);
  const document = documents.data?.documents.find((entry) => entry.id === documentId);
  const rawPage = search.get("page") ?? "1";
  const page = /^\d+$/.test(rawPage) ? Number(rawPage) : Number.NaN;
  const returnLink = <Link className="button" to={`/items/${itemId}#document-${documentId}`}>返回物品资料</Link>;
  if (documents.isPending) return <Skeleton label="正在读取原件资料…" rows={3} />;
  if (documents.isError || document === undefined) {
    const info = documents.isError ? describeError(documents.error) : null;
    return <section className="page"><h1>原件不可用</h1><div role="alert">
      <p>{info?.message ?? "该物品没有这份说明书原件，请返回资料区选择。"}</p>
      {info?.requestId != null && <p>诊断请求 ID：{info.requestId}</p>}
      {documents.isError && <button onClick={() => void documents.refetch()}>重新加载原文</button>}
    </div>{returnLink}</section>;
  }
  return <section className="page document-reader" aria-labelledby="document-title">
    <header className="page__header"><h1 id="document-title">{document.title}</h1>{returnLink}</header>
    <p className="page-note">说明书原件 · PDF 实际页码</p>
    <OriginalDocumentPanel key={document.id} assetId={document.sourceAssetId} pageNumber={page}
      onPageChange={(next) => setSearch({ page: String(next) })} />
    <footer className="document-reader__footer">{returnLink}</footer>
  </section>;
}
