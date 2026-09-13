/**
 * 版本列表（T19；路由 `/items/:itemId/releases`；PRD §6.1.2 / UI-055）。
 *
 * 语义（QA 按此复核）：
 * - 已发布版本**不可再修改**：每个版本给出发布时间、草稿 revision、模型 revision；
 * - 历史发布版各自指向发布时的模型 revision，继续可读（点进阅读器）；
 * - 不提供"编辑已发布内容"的入口；排序由服务端给定（发布时间倒序，U-03）。
 */

import { useQuery } from "@tanstack/react-query";
import { Link, useParams } from "react-router";

import { listReleases } from "../../api/endpoints";
import { describeError } from "../../api/client";
import { EmptyNote } from "../../components/EmptyState";
import { Skeleton } from "../../components/Skeleton";

export function ReleaseListPage() {
  const params = useParams();
  const itemId = params.itemId ?? "";
  const releasesQuery = useQuery({
    queryKey: ["releases", itemId],
    queryFn: () => listReleases(itemId),
    enabled: itemId !== "",
  });

  if (releasesQuery.isLoading) {
    return <Skeleton label="正在读取发布版本…" rows={3} />;
  }
  if (releasesQuery.isError) {
    return (
      <div className="page-error" role="alert">
        <p>版本列表读取失败：{describeError(releasesQuery.error).message}</p>
        <Link to={`/items/${itemId}`}>返回物品概览</Link>
      </div>
    );
  }
  const releases = releasesQuery.data ?? [];

  return (
    <div>
      <h1>发布版本</h1>
      <p className="page-subtitle">
        已发布版本不可再修改；之后对草稿的修改不会改变这里的内容。旧版本继续指向发布时的模型 revision，保持可读。
      </p>
      {releases.length === 0 ? (
        <EmptyNote>
          该物品还没有发布版本。进入草稿的校准工作区完成知识确认与热点校准后，显式发布才会产出版本。
        </EmptyNote>
      ) : (
        <ul className="entity-list" data-testid="release-list">
          {releases.map((release) => (
            <li key={release.id}>
              <p>
                <strong>发布版本 {release.id}</strong>
              </p>
              <p className="page-note">
                发布时间 {new Date(release.createdAt).toLocaleString("zh-CN", { hour12: false })} ·
                草稿 r{release.draftRevision} · 模型 {release.modelRevisionId}
              </p>
              <div className="row-actions">
                <Link to={`/items/${itemId}/releases/${release.id}`}>打开阅读器</Link>
              </div>
            </li>
          ))}
        </ul>
      )}
      <p className="page-note">
        <Link to={`/items/${itemId}`}>返回物品概览</Link>
      </p>
    </div>
  );
}

export default ReleaseListPage;
