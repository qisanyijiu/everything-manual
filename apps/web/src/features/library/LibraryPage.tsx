/**
 * 资料库（PRD §6.1.2 `/`、§6.2 UI-005）：
 * - `GET /items`（默认 20/页，游标继续加载）；游标、归档开关与搜索词由 URL 承载
 *   （§6.1.1「列表状态由 URL 承载，不用内存状态代替」）；
 * - 空态：还没有物品 + 主按钮「新建物品」；加载：行骨架；失败：加载失败，重试且保留已加载行；
 * - 行显示名称、型号、状态、更新时间；归档物品默认隐藏，可由「显示已归档」切出；
 * - 搜索首版只服务端不支持检索（A-06）：在**已加载行**内按名称/型号筛选，界面写明范围。
 */

import { Link, useSearchParams } from "react-router";

import { describeError } from "../../api/client";
import { EmptyState } from "../../components/EmptyState";
import { PageLayout } from "../shell/PageLayout";
import { Skeleton } from "../../components/Skeleton";
import { formatLocalDateTime } from "../../lib/format";
import { ITEMS_PAGE_SIZE, useItemList } from "./items";
import type { ItemDto } from "../../api/endpoints";

export function LibraryPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const archived = searchParams.get("archived") === "true";
  const startCursor = searchParams.get("cursor");
  const search = searchParams.get("q") ?? "";

  const query = useItemList({ archived, startCursor });
  const loadedItems = (query.data?.pages ?? []).flatMap((page) => page.items);
  const visibleItems =
    search === ""
      ? loadedItems
      : loadedItems.filter((item) => matchesSearch(item, search));

  async function loadMore() {
    const pages = query.data?.pages ?? [];
    // 新一页从上一页的 nextCursor 开始；只在该页加载成功后把它写进 URL，
    // 刷新后从同一位置继续（§6.1.1：游标由 URL 承载，不用内存状态代替）。
    const nextStart = pages[pages.length - 1]?.nextCursor ?? null;
    await query.fetchNextPage();
    updateParams({ cursor: nextStart });
  }

  function updateParams(changes: Record<string, string | null>) {
    const next = new URLSearchParams(searchParams);
    for (const [key, value] of Object.entries(changes)) {
      if (value === null || value === "") {
        next.delete(key);
      } else {
        next.set(key, value);
      }
    }
    setSearchParams(next, { replace: true });
  }

  return (
    <PageLayout
      aside={{ id: "summary", label: "资料库摘要", content: <LibrarySummary loadedCount={loadedItems.length} archived={archived} /> }}
    >
      <section className="page library-page" aria-labelledby="library-title">
        <header className="page__header">
          <h1 id="library-title">资料库</h1>
          <Link className="button-primary" to="/items/new">
            新建物品
          </Link>
        </header>

        <form className="library-toolbar" role="search" onSubmit={(event) => event.preventDefault()}>
          <div className="field">
            <label className="field__label" htmlFor="library-search">
              搜索
            </label>
            <input
              id="library-search"
              className="field__input"
              type="search"
              value={search}
              aria-describedby="library-search-hint"
              onChange={(event) => updateParams({ q: event.target.value })}
            />
            <p className="field__hint" id="library-search-hint">
              在当前已加载的 {loadedItems.length} 条中按名称/型号筛选（首版无全文检索，A-06）。
            </p>
          </div>
          <div className="library-toolbar__toggle">
            <input
              id="library-include-archived"
              type="checkbox"
              checked={archived}
              onChange={(event) =>
                // 游标绑定过滤条件（ADR-016 第 3 条）：切换归档范围时回到第一页。
                updateParams({ archived: event.target.checked ? "true" : null, cursor: null })
              }
            />
            <label htmlFor="library-include-archived">显示已归档</label>
          </div>
        </form>

        {startCursor !== null && (
          <p className="library-page__resume">
            本页从 URL 记录的游标继续显示。{" "}
            <button type="button" className="link-button" onClick={() => updateParams({ cursor: null })}>
              回到列表开头
            </button>
          </p>
        )}

        {query.isPending && <Skeleton label="正在加载物品…" rows={5} />}

        {query.isError && (
          <div className="error-panel" role="alert">
            <h2>加载失败</h2>
            <p>{describeError(query.error).message}</p>
            <button type="button" onClick={() => void query.refetch()}>
              重试
            </button>
          </div>
        )}

        {!query.isPending && loadedItems.length === 0 && !query.isError && (
          <EmptyState
            title="还没有物品"
            description={
              archived
                ? "没有已归档的物品。"
                : "先新建一个物品，再上传说明书原件与多视图照片。"
            }
            action={
              archived ? undefined : (
                <Link className="button-primary" to="/items/new">
                  新建物品
                </Link>
              )
            }
          />
        )}

        {loadedItems.length > 0 && (
          <>
            <ul className="item-list">
              {visibleItems.map((item) => (
                <ItemRow key={item.id} item={item} />
              ))}
            </ul>
            {visibleItems.length === 0 && search !== "" && (
              <p className="empty-note">当前已加载的行中没有匹配「{search}」的物品。</p>
            )}
            <div className="library-page__more">
              {query.hasNextPage ? (
                <button
                  type="button"
                  disabled={query.isFetchingNextPage}
                  onClick={() => void loadMore()}
                >
                  {query.isFetchingNextPage ? "正在加载…" : "加载更多"}
                </button>
              ) : (
                <p className="empty-note">已显示全部 {loadedItems.length} 条（每页 {ITEMS_PAGE_SIZE} 条）。</p>
              )}
            </div>
          </>
        )}
      </section>
    </PageLayout>
  );
}

function matchesSearch(item: ItemDto, search: string): boolean {
  const needle = search.trim().toLowerCase();
  if (needle === "") {
    return true;
  }
  return (
    item.name.toLowerCase().includes(needle) || item.model.toLowerCase().includes(needle)
  );
}

function ItemRow({ item }: { item: ItemDto }) {
  return (
    <li className="item-row">
      <div className="item-row__identity">
        <Link to={`/items/${item.id}`} className="item-row__name">
          {item.name}
        </Link>
        <p className="item-row__meta">
          {item.model}
          {item.brand !== null && item.brand !== undefined && ` · ${item.brand}`}
          {item.variant !== null && item.variant !== undefined && ` · ${item.variant}`}
        </p>
      </div>
      <div className="item-row__status">
        <span className="status-label">
          {item.archivedAt !== null && item.archivedAt !== undefined ? "已归档" : "使用中"}
        </span>
        <span className="item-row__revision">r{item.revision}</span>
      </div>
      <div className="item-row__time">
        <span className="item-row__time-label">更新于</span>
        <time dateTime={item.updatedAt}>{formatLocalDateTime(item.updatedAt)}</time>
      </div>
      <div className="item-row__actions">
        {/* 行内入口按当前后端能力落地：不可用的入口明确禁用并给原因（不假装可用）。 */}
        <Link to={`/items/${item.id}/import/prepare`}>继续准备</Link>
        {/* 任务中心已交付（T17）：按物品过滤是该列表接口支持的查询（contracts §3）。 */}
        <Link to={`/jobs?itemId=${encodeURIComponent(item.id)}`}>查看任务</Link>
        <button
          type="button"
          disabled
          aria-describedby={`item-row-unavailable-${item.id}`}
          title="发布与阅读器（T18/T19）尚未交付"
        >
          打开说明书
        </button>
        <Link to={`/items/${item.id}`} className="item-row__open">
          打开
        </Link>
      </div>
      {/* 禁用原因常驻（REQ-039/AC-060：不假装可用）——放在四列之外独占整行，
          避免长句把 grid 的 `auto` 轨道撑开、挤压名称列（BUG-005）。 */}
      <span id={`item-row-unavailable-${item.id}`} className="item-row__note">
        发布与阅读器（T18/T19）尚未交付：「打开说明书」保持禁用。
      </span>
    </li>
  );
}

/**
 * 右栏摘要：只呈现真实计数与入口。
 * PRD §6.1.2 里的「最近使用/进行中任务」摘要需要服务端排序键或任务接口
 * （T15/T17），当前不伪造这些数字。
 */
function LibrarySummary({ loadedCount, archived }: { loadedCount: number; archived: boolean }) {
  return (
    <div className="summary-panel">
      <h2 className="summary-panel__title">资料库摘要</h2>
      <dl className="summary-panel__list">
        <div>
          <dt>已加载</dt>
          <dd>{loadedCount} 条</dd>
        </div>
        <div>
          <dt>当前范围</dt>
          <dd>{archived ? "仅已归档" : "仅使用中"}</dd>
        </div>
      </dl>
      <p className="empty-note">
        进行中任务摘要需要任务中心接口（T15/T17），本页暂不显示该数字。
      </p>
    </div>
  );
}
