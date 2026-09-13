/**
 * 导入流程中的错误明细渲染（T09）。
 *
 * 单独成模块（不依赖 pdfjs-dist 与 React），便于单元测试逐字断言：
 * 封存 422 的 `details.missingPages` / `details.pages[]` 必须逐条变成可读缺项，
 * 让用户知道"补哪些页"（UI-018、AC-025）。
 */

/** 把封存 422 的 `details` 渲染成可读缺项列表（缺页号 / 资产问题）。 */
export function describeCompleteFailure(details: unknown): string[] {
  if (typeof details !== "object" || details === null) {
    return [];
  }
  const lines: string[] = [];
  const missing = (details as { missingPages?: unknown }).missingPages;
  if (Array.isArray(missing) && missing.length > 0) {
    lines.push(`缺页：第 ${missing.join("、")} 页`);
  }
  const pages = (details as { pages?: unknown }).pages;
  if (Array.isArray(pages)) {
    for (const page of pages) {
      const number = (page as { pageNumber?: unknown }).pageNumber;
      const problem = (page as { problem?: unknown }).problem;
      if (typeof number === "number" && typeof problem === "string") {
        lines.push(`第 ${number} 页：${problem}`);
      }
    }
  }
  return lines;
}
