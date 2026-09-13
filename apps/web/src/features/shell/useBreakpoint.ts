/**
 * 断点判定（PRD §6.1.1）：`wide ≥1280px` 三栏、`mid 768–1279px` 主栏 + 单侧栏、
 * `narrow <768px` 单栏 + 抽屉。只用视口宽度，不用 UA；窗口拉宽后无需刷新即可恢复。
 *
 * 实现：优先 `matchMedia`（真实浏览器，与 CSS 媒体查询同一判定源）；
 * 无 `matchMedia` 的环境（jsdom 测试）退化为 `window.innerWidth` 比较，仍以视口为准。
 */

import { useEffect, useState } from "react";

export type Breakpoint = "wide" | "mid" | "narrow";

export const WIDE_QUERY = "(min-width: 1280px)";
export const MID_QUERY = "(min-width: 768px)";

function queryMatches(query: string): boolean {
  if (typeof window === "undefined") {
    return false;
  }
  if (typeof window.matchMedia === "function") {
    return window.matchMedia(query).matches;
  }
  if (query === WIDE_QUERY) {
    return window.innerWidth >= 1280;
  }
  if (query === MID_QUERY) {
    return window.innerWidth >= 768;
  }
  return false;
}

export function currentBreakpoint(): Breakpoint {
  if (queryMatches(WIDE_QUERY)) {
    return "wide";
  }
  if (queryMatches(MID_QUERY)) {
    return "mid";
  }
  return "narrow";
}

export function useBreakpoint(): Breakpoint {
  const [breakpoint, setBreakpoint] = useState<Breakpoint>(() => currentBreakpoint());

  useEffect(() => {
    const update = () => setBreakpoint(currentBreakpoint());
    const lists: MediaQueryList[] = [];
    if (typeof window.matchMedia === "function") {
      for (const query of [WIDE_QUERY, MID_QUERY]) {
        const list = window.matchMedia(query);
        list.addEventListener("change", update);
        lists.push(list);
      }
    }
    window.addEventListener("resize", update);
    update();
    return () => {
      for (const list of lists) {
        list.removeEventListener("change", update);
      }
      window.removeEventListener("resize", update);
    };
  }, []);

  return breakpoint;
}
