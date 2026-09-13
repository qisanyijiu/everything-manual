/**
 * 物品概览（PRD §6.1.2 `/items/:itemId`）。
 *
 * T08 实现范围：物品身份与版本（GET /items/{id}）、资料清单只读（documents/photos，
 * T07 已交付的读取侧路由）、归档/恢复（PATCH + If-Match + 412 恢复路径）、下一步入口。
 * 上传、准备、报价与生成、草稿与发布分别在 T09/T16/T19 交付，这里只给明确的未实现说明，
 * 不用 mock 数据假装可用。
 */

import { useState } from "react";
import { Link, useParams } from "react-router";

import { describeError, isApiError } from "../../api/client";
import { ConflictNotice } from "../../components/ConflictNotice";
import { EmptyNote } from "../../components/EmptyState";
import { Skeleton } from "../../components/Skeleton";
import { useNotify } from "../../components/notifications";
import { assetContentUrl } from "../../api/endpoints";
import { formatLocalDateTime } from "../../lib/format";
import { readCurrentRevision } from "../../components/form";
import { JobSnapshotNotice } from "./JobSnapshotNotice";
import { useItemDetail, useItemDocuments, useItemPhotos, usePatchItem } from "./items";

export function ItemOverviewPage() {
  const { itemId } = useParams();
  const id = itemId ?? "";
  const itemQuery = useItemDetail(id);
  const documentsQuery = useItemDocuments(id);
  const photosQuery = useItemPhotos(id);
  const patchMutation = usePatchItem();
  const notify = useNotify();
  const [etag, setEtag] = useState<string | null>(null);
  const [conflict, setConflict] = useState<{ currentRevision: number | null } | null>(null);

  const item = itemQuery.data?.data;
  const effectiveEtag = etag ?? itemQuery.data?.etag ?? null;

  if (itemQuery.isPending) {
    return (
      <section className="page">
        <Skeleton label="正在读取物品…" rows={4} />
      </section>
    );
  }

  if (item === undefined) {
    const info = describeError(itemQuery.error);
    return (
      <section className="page">
        <div className="error-panel" role="alert">
          <h1>无法读取物品</h1>
          <p>{info.message}</p>
          {info.requestId !== null && (
            <p>
              诊断请求 ID：<code>{info.requestId}</code>
            </p>
          )}
          <div className="error-panel__actions">
            <button type="button" onClick={() => void itemQuery.refetch()}>
              重试
            </button>
            <Link to="/">返回资料库</Link>
          </div>
        </div>
      </section>
    );
  }

  const archived = item.archivedAt !== null && item.archivedAt !== undefined;

  async function toggleArchive() {
    if (effectiveEtag === null) {
      setConflict({ currentRevision: null });
      return;
    }
    try {
      const result = await patchMutation.mutateAsync({
        itemId: id,
        body: { archived: !archived },
        ifMatch: effectiveEtag,
      });
      setEtag(result.etag);
      setConflict(null);
      notify(result.data.archivedAt !== null && result.data.archivedAt !== undefined ? "物品已归档" : "物品已恢复");
    } catch (error) {
      if (isApiError(error) && (error.status === 412 || error.status === 428)) {
        setConflict({ currentRevision: readCurrentRevision(error.details) });
        return;
      }
      const info = describeError(error);
      notify(`操作失败：${info.message}`, { kind: "alert", requestId: info.requestId });
    }
  }

  async function refreshAfterConflict() {
    const result = await itemQuery.refetch();
    setEtag(result.data?.etag ?? null);
    setConflict(null);
  }

  return (
    <section className="page item-overview" aria-labelledby="item-title">
      <header className="page__header">
        <div>
          <h1 id="item-title">{item.name}</h1>
          <p className="page__lead">
            {item.model}
            {item.brand !== null && item.brand !== undefined && ` · ${item.brand}`}
            {item.variant !== null && item.variant !== undefined && ` · ${item.variant}`}
          </p>
        </div>
        <div className="page__actions">
          <Link className="button" to={`/items/${item.id}/edit`}>
            编辑
          </Link>
          <button type="button" onClick={() => void toggleArchive()} disabled={patchMutation.isPending}>
            {patchMutation.isPending ? "处理中…" : archived ? "恢复" : "归档"}
          </button>
        </div>
      </header>

      {conflict !== null && (
        <ConflictNotice
          currentRevision={conflict.currentRevision}
          refreshing={itemQuery.isFetching}
          onRefresh={() => void refreshAfterConflict()}
        />
      )}

      {/* UI-021：进行中任务使用冻结快照，编辑不影响已开始的生成。 */}
      <JobSnapshotNotice itemId={item.id} />

      <dl className="meta-list">
        <div>
          <dt>状态</dt>
          <dd>
            <span className="status-label">{archived ? "已归档" : "使用中"}</span>
            {archived && item.archivedAt !== null && item.archivedAt !== undefined && (
              <span className="meta-list__extra">（{formatLocalDateTime(item.archivedAt)}）</span>
            )}
          </dd>
        </div>
        <div>
          <dt>版本</dt>
          <dd>
            r{item.revision}（更新于 {formatLocalDateTime(item.updatedAt)}）
          </dd>
        </div>
        <div>
          <dt>创建</dt>
          <dd>{formatLocalDateTime(item.createdAt)}</dd>
        </div>
      </dl>

      <section className="panel" aria-labelledby="documents-title">
        <h2 id="documents-title">说明书原件</h2>
        {documentsQuery.isPending && <Skeleton label="正在读取资料清单…" rows={2} />}
        {documentsQuery.isError && (
          <div className="error-panel" role="alert">
            <p>资料清单加载失败：{describeError(documentsQuery.error).message}</p>
            <button type="button" onClick={() => void documentsQuery.refetch()}>
              重试
            </button>
          </div>
        )}
        {documentsQuery.data !== undefined && documentsQuery.data.documents.length === 0 && (
          <EmptyNote>
            尚未绑定说明书原件。
            <Link to={`/items/${item.id}/import/document`}>去第 2 步「说明书」上传并绑定</Link>
            。
          </EmptyNote>
        )}
        {documentsQuery.data !== undefined && documentsQuery.data.documents.length > 0 && (
          <ul className="entity-list">
            {documentsQuery.data.documents.map((document) => (
              <li key={document.id}>
                <span className="entity-list__title">{document.title}</span>
                <span className="entity-list__meta">
                  原件 sha256 {document.sourceSha256.slice(0, 12)}… · 绑定于{" "}
                  {formatLocalDateTime(document.createdAt)}
                </span>
                {document.sourceUrl !== null && document.sourceUrl !== undefined && (
                  <span className="entity-list__meta">
                    出处链接：{document.sourceUrl}（仅记录，服务器不访问）
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="panel" aria-labelledby="photos-title">
        <h2 id="photos-title">视图照片</h2>
        {photosQuery.isPending && <Skeleton label="正在读取照片槽位…" rows={2} />}
        {photosQuery.isError && (
          <div className="error-panel" role="alert">
            <p>照片槽位加载失败：{describeError(photosQuery.error).message}</p>
            <button type="button" onClick={() => void photosQuery.refetch()}>
              重试
            </button>
          </div>
        )}
        {photosQuery.data !== undefined && photosQuery.data.photos.length === 0 && (
          <EmptyNote>
            尚未上传照片。
            <Link to={`/items/${item.id}/import/views`}>去第 3 步「视图排列」上传</Link>
            。
          </EmptyNote>
        )}
        {photosQuery.data !== undefined && photosQuery.data.photos.length > 0 && (
          <ul className="entity-list">
            {photosQuery.data.photos.map((photo) => (
              <li key={photo.id}>
                <span className="entity-list__title">视图：{photo.view}</span>
                <a className="entity-list__link" href={assetContentUrl(photo.assetId)}>
                  查看照片内容
                </a>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="panel" aria-labelledby="next-steps-title">
        <h2 id="next-steps-title">下一步</h2>
        <ul className="link-list">
          <li>
            <Link to={`/items/${item.id}/import/document`}>绑定说明书原件（向导第 2 步）</Link>
          </li>
          <li>
            <Link to={`/items/${item.id}/import/views`}>上传与排列视图（向导第 3 步）</Link>
          </li>
          <li>
            <Link to={`/items/${item.id}/import/prepare`}>准备资料（向导第 4 步）</Link>
          </li>
          <li>
            <Link to={`/items/${item.id}/import/confirm`}>报价、确认与生成（向导第 5 步）</Link>
          </li>
        </ul>
        <EmptyNote>
          阅读器（T18/T19）尚未交付：「打开说明书」入口暂时不可用；
          生成后的任务状态与恢复入口见
          <Link to={`/jobs?itemId=${encodeURIComponent(item.id)}`}>本物品的任务</Link>
          （或顶栏「任务中心」）。
        </EmptyNote>
      </section>

      <p>
        <Link to="/">返回资料库</Link>
      </p>
    </section>
  );
}
