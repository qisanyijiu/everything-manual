import { useQuery } from "@tanstack/react-query";
import { listDocuments, type DocumentPage } from "../../api/endpoints";
import { itemKeys } from "../library/items";

/** A reference may point beyond the first list page; absence is only final after all cursors. */
export function useReaderDocuments(itemId: string | null) {
  return useQuery({
    queryKey: [...itemKeys.documents(itemId ?? ""), "reader-all"],
    enabled: itemId !== null && itemId !== "",
    queryFn: async ({ signal }): Promise<DocumentPage> => {
      const documents: DocumentPage["documents"] = [];
      const seen = new Set<string>();
      let cursor: string | null = null;
      do {
        const page = await listDocuments(itemId ?? "", cursor, signal);
        documents.push(...page.documents);
        cursor = page.nextCursor;
        if (cursor !== null) {
          if (seen.has(cursor)) throw new Error("原件清单分页异常，请重新加载。");
          seen.add(cursor);
        }
      } while (cursor !== null);
      return { documents, nextCursor: null, etag: null };
    },
  });
}
