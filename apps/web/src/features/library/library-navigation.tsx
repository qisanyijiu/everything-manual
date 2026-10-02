import { useEffect, useLayoutEffect, useRef } from "react";
import { Link, useLocation } from "react-router";

interface Position { y: number; itemId: string | null; offset: number }
interface ReturnPoint { href: string; key: string }
// Navigation-only memory, bounded to this page session; no document or form data is persisted.
const positions = new Map<string, Position>();
const returns = new Map<string, ReturnPoint>();
function boundedSet<T>(map: Map<string, T>, key: string, value: T) {
  map.delete(key); map.set(key, value);
  if (map.size > 100) map.delete(map.keys().next().value!);
}
function position(itemId?: string): Position {
  const rows = [...document.querySelectorAll<HTMLElement>("[data-library-item]")];
  const row = rows.find((row) => row.dataset.libraryItem === itemId)
    ?? rows.find((row) => row.getBoundingClientRect().bottom > 100);
  return { y: window.scrollY, itemId: row?.dataset.libraryItem ?? null, offset: row?.getBoundingClientRect().top ?? 0 };
}
export function rememberLibraryItem(itemId: string, key: string, search: string) {
  boundedSet(positions, key, position(itemId));
  boundedSet(returns, itemId, { href: `/${search}`, key });
}
export function LibraryBackLink({ itemId }: { itemId: string }) {
  const point = returns.get(itemId);
  return <Link to={point?.href ?? "/"} state={point ? { libraryRestoreKey: point.key } : null}>返回资料库</Link>;
}
/** Restore after real rows are present. Browser history/URL remains authoritative for filtering. */
export function useLibraryPosition(ready: boolean) {
  const location = useLocation();
  const restored = useRef<string | null>(null);
  const saved = positions.get(location.key)
    ?? positions.get((location.state as { libraryRestoreKey?: string } | null)?.libraryRestoreKey ?? "");
  useLayoutEffect(() => {
    if (!ready || restored.current === location.key) return;
    if (!saved) { restored.current = location.key; return; }
    const frame = requestAnimationFrame(() => {
      const row = [...document.querySelectorAll<HTMLElement>("[data-library-item]")].find((row) => row.dataset.libraryItem === saved.itemId);
      window.scrollTo({ top: row ? window.scrollY + row.getBoundingClientRect().top - saved.offset : saved.y, behavior: "instant" });
      // Mark only an executed restoration: StrictMode may cancel the first frame.
      restored.current = location.key;
    });
    return () => cancelAnimationFrame(frame);
  }, [ready, location.key, saved]);
  useEffect(() => {
    if (!ready) return;
    const record = () => { if (restored.current === location.key) boundedSet(positions, location.key, position()); };
    window.addEventListener("scroll", record, { passive: true });
    return () => window.removeEventListener("scroll", record);
  }, [ready, location.key]);
}
