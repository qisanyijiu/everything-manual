/** PC05A: server-side literal search; each URL position retains its own query cache. */
import { useEffect, useRef, useState } from "react";
import { Link, useLocation, useSearchParams } from "react-router";
import { useQueryClient } from "@tanstack/react-query";

import { describeError, isApiError } from "../../api/client";
import { EmptyState } from "../../components/EmptyState";
import { PageLayout } from "../shell/PageLayout";
import { Skeleton } from "../../components/Skeleton";
import { formatLocalDateTime } from "../../lib/format";
import { itemKeys, useItemList } from "./items";
import { useItemSummaries, WorkflowActions } from "./workflow";
import type { ItemSummaryDto } from "../../api/endpoints";
import { readFieldErrors } from "../../components/form";
import { rememberLibraryItem, useLibraryPosition } from "./library-navigation";
import type { ItemDto } from "../../api/endpoints";
import { Icon } from "../../components/Icon";

export function LibraryPage() {
  const [searchParams, setSearchParams] = useSearchParams();
  const location = useLocation();
  const queryClient = useQueryClient();
  const archived = searchParams.get("archived") === "true";
  const startCursor = searchParams.get("cursor");
  const search = (searchParams.get("q") ?? "").trim();
  // A submitted query belongs to the URL. Do not optimistically clear its text before
  // the Router transition commits: a fast Back may cancel that transition entirely.
  const [inputDraft, setInputDraft] = useState<{ key: string; value: string } | null>(null);
  const input = inputDraft?.key === location.key ? inputDraft.value : search;
  const [inputError, setInputError] = useState<string | null>(null);
  useEffect(() => { setInputError(null); }, [location.key]);
  const query = useItemList({ archived, startCursor, q: search });
  const [lastData, setLastData] = useState(query.data);
  useEffect(() => { if (query.data) setLastData(query.data); }, [query.data]);
  const data = query.data ?? lastData;
  const loadedItems = (data?.pages ?? []).flatMap((page) => page.items);
  const summaries = useItemSummaries(loadedItems.map((item) => item.id));
  const badQuery = [...search].length > 200;
  const cursorError = isApiError(query.error) && query.error.status === 422 && readFieldErrors(query.error.details).some((field) => field.field === "cursor");
  const errorInfo = query.isError ? describeError(query.error) : null;
  const scope = `${location.key}:${archived}:${search}:${startCursor ?? ""}`;
  const activeScope = useRef(scope);
  const loadingMore = useRef(false);
  useEffect(() => { activeScope.current = scope; return () => { activeScope.current = ""; }; }, [scope]);
  useLibraryPosition(query.data !== undefined && !query.isFetching);

  async function loadMore() {
    if (loadingMore.current || query.isFetching || !query.data) return;
    const nextStart = query.data.pages.at(-1)?.nextCursor;
    if (!nextStart) return;
    loadingMore.current = true;
    const previous = query.data;
    try {
      const result = await query.fetchNextPage();
      if (result.isError || !result.data || activeScope.current !== scope) return;
      // Preserve both history entries: advancing the URL must not discard preceding rows,
      // nor make Back display pages that were loaded only by a later navigation.
      queryClient.setQueryData(itemKeys.list(archived, nextStart, search), result.data);
      queryClient.setQueryData(itemKeys.list(archived, startCursor, search), previous);
      updateParams({ cursor: nextStart });
    } finally { loadingMore.current = false; }
  }

  function updateParams(changes: Record<string, string | null>) {
    const next = new URLSearchParams(searchParams);
    for (const [key, value] of Object.entries(changes)) {
      if (value === null || value === "") next.delete(key); else next.set(key, value);
    }
    setSearchParams(next);
  }
  function submitSearch() {
    const q = input.trim();
    if ([...q].length > 200) { setInputError("搜索词最多200个字符，请缩短后重试"); return; }
    setInputError(null); setInputDraft(null);
    updateParams({ q, cursor: null });
  }

  return (
    <div className="library-home">
      <div className="library-heading">
        <div><p className="eyebrow">YOUR PERSONAL COLLECTION</p><h1 id="library-title">资料库<span className="heading-dot" aria-hidden="true">.</span></h1><p className="page__lead">收藏物品的每一份资料，让了解与使用更简单。</p></div>
        <Link className="button-primary" to="/items/new"><Icon name="plus" size={18} />新建物品</Link>
      </div>
    <PageLayout
      aside={{ id: "summary", label: "资料库摘要", content: <LibrarySummary loadedCount={loadedItems.length} archived={archived} /> }}
    >
      <section className="page library-page" aria-labelledby="library-title">
        <header className="collection-heading"><h2>我的物品 <span>{loadedItems.length}</span></h2><span>{archived ? "已归档收藏" : "使用中的收藏"}</span></header>

        <form className="library-toolbar" role="search" onSubmit={(event) => { event.preventDefault(); submitSearch(); }}>
          <div className="field library-search">
            <label htmlFor="library-search">按名称或型号搜索</label>
            <input id="library-search" className="field__input" type="search" placeholder="输入名称或型号"
              value={input} aria-describedby={`library-search-hint${inputError || badQuery ? " library-search-error" : ""}`}
              aria-invalid={Boolean(inputError || badQuery)} onChange={(event) => { setInputDraft({ key: location.key, value: event.target.value }); setInputError(null); }} />
            <p className="field__hint" id="library-search-hint">搜索{archived ? "已归档" : "使用中"}物品的名称或型号，最多200个字符。</p>
            {(inputError || badQuery) && <p id="library-search-error" className="field__error" role="alert">{inputError ?? "搜索词最多200个字符，请缩短后重试"}</p>}
          </div>
          <div className="library-search-actions">
            <button type="submit">搜索</button>
            {(search !== "" || input !== "") && <button type="button" onClick={() => { setInputDraft(null); setInputError(null); updateParams({ q: null, cursor: null }); }}>清除搜索</button>}
          </div>
          <div className="library-toolbar__toggle">
            <input id="library-include-archived" type="checkbox" checked={archived}
              onChange={(event) => updateParams({ archived: event.target.checked ? "true" : null, cursor: null })} />
            <label htmlFor="library-include-archived">显示已归档</label>
          </div>
        </form>

        {startCursor !== null && !cursorError && (
          <p className="library-page__resume">
            本页从 URL 记录的游标继续显示。{" "}
            <button type="button" className="link-button" onClick={() => updateParams({ cursor: null })}>
              回到列表开头
            </button>
          </p>
        )}

        {query.isPending && !data && !badQuery && <Skeleton label="正在查找物品…" rows={5} />}
        {query.isFetching && data && <p role="status">{query.isFetchingNextPage ? "正在加载更多…" : "正在查找…，下方为上次结果"}</p>}
        {errorInfo && <div className="error-panel" role="alert">
          <h2>{cursorError ? "列表条件已变化，请回到开头" : "此次搜索未完成"}</h2>
          {data && <p>此次搜索未完成，仍显示上次结果。</p>}
          <p>{errorInfo.message}</p>
          {errorInfo.requestId && <p>诊断请求 ID：<code>{errorInfo.requestId}</code></p>}
          {cursorError ? <button type="button" onClick={() => updateParams({ cursor: null })}>回到列表开头</button>
            : <button type="button" disabled={query.isFetching} onClick={() => { if (query.isFetchNextPageError) void loadMore(); else void query.refetch(); }}>重试</button>}
        </div>}
        {query.isSuccess && loadedItems.length === 0 && <EmptyState
          title={search ? "当前范围没有匹配物品" : archived ? "没有已归档的物品" : "还没有物品"}
          description={search ? "换个名称或型号，或清除搜索查看当前范围。" : "收藏资料，从新建物品开始。"}
          action={search ? <button type="button" onClick={() => { setInputDraft(null); updateParams({ q: null, cursor: null }); }}>清除搜索</button>
            : archived ? <button type="button" onClick={() => updateParams({ archived: null, cursor: null })}>查看使用中的物品</button>
            : <Link className="button-primary" to="/items/new">新建物品</Link>} />}
        {summaries.isError && <p className="notice-inline" role="alert">处理状态暂不可用。<button type="button" className="link-button" onClick={() => void summaries.refetch()}>重新读取处理状态</button></p>}
        {loadedItems.length > 0 && <>
          <ul className="item-list">
            {loadedItems.map((item) => <ItemRow key={item.id} item={item}
              onOpen={() => rememberLibraryItem(item.id, location.key, location.search)}
              summary={summaries.data?.find((summary) => summary.itemId === item.id)} unavailable={summaries.isError} loading={summaries.isPending} />)}
          </ul>
          <div className="library-page__more">
            {query.data && query.hasNextPage ? <button type="button" disabled={query.isFetching} onClick={() => void loadMore()}>{query.isFetchingNextPage ? "正在加载…" : "加载更多"}</button>
              : query.isSuccess && <p className="empty-note">已显示 {loadedItems.length} 件物品，当前分页已到末尾。</p>}
          </div>
        </>}

      </section>
    </PageLayout>
    </div>
  );
}

function ItemRow({ item, summary, unavailable, loading, onOpen }: { onOpen: () => void; item: ItemDto; summary?: ItemSummaryDto; unavailable: boolean; loading: boolean }) {
  return (
    <li className="item-row" data-library-item={item.id} onClickCapture={(event) => { if ((event.target as HTMLElement).closest("a")) onOpen(); }}>
      <div className="item-row__identity">
        <span className="item-row__art"><Icon name="cube" size={31} /></span><div>
        <Link to={`/items/${item.id}`} className="item-row__name">
          {item.name}
        </Link>
        <p className="item-row__meta">
          {item.model !== "" ? item.model : "未提供型号"}
          {item.brand !== null && item.brand !== undefined && item.brand !== "" && ` · ${item.brand}`}
          {item.variant !== null && item.variant !== undefined && item.variant !== "" && ` · ${item.variant}`}
        </p>
        </div>
      </div>
      <div className="item-row__status">
        <span
          className={`status-label ${item.archivedAt !== null && item.archivedAt !== undefined ? "status-label--archived" : "status-label--success"}`}
        >
          {item.archivedAt !== null && item.archivedAt !== undefined ? "已归档" : "使用中"}
        </span>
      </div>
      <div className="item-row__time">
        <span className="item-row__time-label">更新于</span>
        <time dateTime={item.updatedAt}>{formatLocalDateTime(item.updatedAt)}</time>
      </div>
      <div className="item-row__actions">
        <WorkflowActions itemId={item.id} summary={summary} unavailable={unavailable} loading={loading} />
      </div>
    </li>
  );
}

/** 摘要仅统计本次已加载的物品，不把分页计数冒充全库总数。 */
function LibrarySummary({ loadedCount, archived }: { loadedCount: number; archived: boolean }) {
  return (
    <div className="summary-panel library-summary">
      <span className="eyebrow">AT A GLANCE</span>
      <h2 className="summary-panel__title">资料库摘要</h2>
      <dl className="summary-panel__list">
        <div>
          <dt>已加载</dt>
          <dd className="summary-count">{loadedCount}<small>件物品</small></dd>
        </div>
        <div>
          <dt>当前范围</dt>
          <dd>{archived ? "仅已归档" : "仅使用中"}</dd>
        </div>
      </dl>
      <div className="summary-guide"><Icon name="book" size={23} /><h3>让资料变得有用</h3><p>上传原版说明书与不同角度的照片，生成后复核知识、校准热点，即可发布交互说明书。</p><Link to="/jobs">前往任务中心 <Icon name="arrow" size={16} /></Link></div>
    </div>
  );
}
