/**
 * 物品相关查询与变更（TanStack Query；PRD REQ-010 / UI-005 / UI-006）。
 * 服务端状态只在 Query 缓存里；组件只持有表单等局部视图状态（architecture.md §3）。
 */

import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import {
  createItem,
  getItem,
  listDocuments,
  listItems,
  listPhotos,
  patchItem,
  type ItemCreateRequest,
  type ItemPatchRequest,
} from "../../api/endpoints";

export const ITEMS_PAGE_SIZE = 20;

export const itemKeys = {
  root: ["items"] as const,
  /** 列表查询的统一前缀（变更成功后只失效列表，详情用响应数据就地更新）。 */
  listRoot: ["items", "list"] as const,
  list: (archived: boolean, startCursor: string | null) =>
    ["items", "list", archived, startCursor] as const,
  detail: (itemId: string) => ["items", "detail", itemId] as const,
  documents: (itemId: string) => ["items", "documents", itemId] as const,
  photos: (itemId: string) => ["items", "photos", itemId] as const,
  /** 准备记录（按会话指针查询；指针缺失时为 null，不伪造状态）。 */
  preparation: (itemId: string, preparationId: string | null) =>
    ["items", "preparation", itemId, preparationId] as const,
};

/** 列表：游标继续加载；`startCursor` 由 URL 承载（§6.1.1）。 */
export function useItemList(params: { archived: boolean; startCursor: string | null }) {
  return useInfiniteQuery({
    queryKey: itemKeys.list(params.archived, params.startCursor),
    queryFn: ({ pageParam }) =>
      listItems({
        archived: params.archived,
        cursor: pageParam === null ? undefined : pageParam,
        limit: ITEMS_PAGE_SIZE,
      }),
    initialPageParam: params.startCursor,
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
  });
}

export function useItemDetail(itemId: string | null) {
  return useQuery({
    queryKey: itemKeys.detail(itemId ?? ""),
    queryFn: () => getItem(itemId ?? ""),
    enabled: itemId !== null && itemId !== "",
  });
}

export function useItemDocuments(itemId: string | null) {
  return useQuery({
    queryKey: itemKeys.documents(itemId ?? ""),
    queryFn: () => listDocuments(itemId ?? ""),
    enabled: itemId !== null && itemId !== "",
  });
}

export function useItemPhotos(itemId: string | null) {
  return useQuery({
    queryKey: itemKeys.photos(itemId ?? ""),
    queryFn: () => listPhotos(itemId ?? ""),
    enabled: itemId !== null && itemId !== "",
  });
}

export function useCreateItem() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: ItemCreateRequest) => createItem(body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: itemKeys.listRoot });
    },
  });
}

export interface PatchItemVariables {
  readonly itemId: string;
  readonly body: ItemPatchRequest;
  /** GET 响应头的 ETag 原样回传（`"r7"`）。 */
  readonly ifMatch: string;
}

export function usePatchItem() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ itemId, body, ifMatch }: PatchItemVariables) =>
      patchItem(itemId, body, ifMatch),
    onSuccess: (result, variables) => {
      queryClient.setQueryData(itemKeys.detail(variables.itemId), result);
      void queryClient.invalidateQueries({ queryKey: itemKeys.listRoot });
    },
  });
}
