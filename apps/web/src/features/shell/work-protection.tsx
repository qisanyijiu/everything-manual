import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type SetStateAction } from "react";
import { UNSAFE_NavigationContext, useLocation, type To } from "react-router";

interface Work { active: boolean; message: string; discard?: () => void }
interface Prompt { action: () => void; message: string; accept: string; discard: boolean; focus: Element | null }
interface Protection {
  entries: Map<symbol, { current: Work }>;
  memory: Map<string, unknown>;
  request: (action: () => void, options?: { message: string; accept: string }) => void;
  bypass: (action: () => void) => void;
}
const Context = createContext<Protection | null>(null);

// popstate targets window itself. A later capture listener is not a reliable
// blocker for Router's already registered listener. Install this relay before
// BrowserRouter mounts; it has no behavior without a mounted protection scope.
let popGuard: ((event: PopStateEvent) => void) | null = null;
const relayPop = (event: PopStateEvent) => popGuard?.(event);
window.addEventListener("popstate", relayPop, true);
if (import.meta.hot) import.meta.hot.dispose(() => window.removeEventListener("popstate", relayPop, true));

/** One router-wide gate. Keys/text remain in memory only; settings never uses this draft cache. */
export function WorkProtection({ children }: { children: ReactNode }) {
  const navigation = useContext(UNSAFE_NavigationContext);
  const location = useLocation();
  const locationRef = useRef(location);
  locationRef.current = location;
  const entries = useRef(new Map<symbol, { current: Work }>());
  const memory = useRef(new Map<string, unknown>());
  const bypassing = useRef(false);
  const [prompt, setPrompt] = useState<Prompt | null>(null);
  const pending = useRef<Prompt | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const active = useCallback(() => [...entries.current.values()].map(entry => entry.current).filter(entry => entry.active), []);
  const bypass = useCallback((action: () => void) => {
    // Expiry or a completed save supersedes any choice from the previous page.
    if (pending.current) { pending.current = null; setPrompt(null); dialog.current?.close(); }
    const previous = bypassing.current; bypassing.current = true;
    try { action(); } finally { bypassing.current = previous; }
  }, []);
  const request = useCallback<Protection["request"]>((action, options) => {
    if (bypassing.current) { action(); return; }
    const work = active();
    if (!options && work.length === 0) { action(); return; }
    if (pending.current) return;
    const next = { action, message: options?.message ?? [...new Set(work.map(entry => entry.message))].join(" "),
      accept: options?.accept ?? "离开页面", discard: !options, focus: document.activeElement };
    pending.current = next; setPrompt(next);
  }, [active]);
  const dismiss = () => {
    const focus = pending.current?.focus;
    pending.current = null; setPrompt(null); dialog.current?.close();
    if (focus instanceof HTMLElement && focus.isConnected) focus.focus();
  };
  useEffect(() => {
    if (prompt) { dialog.current?.showModal(); cancel.current?.focus(); }
    else dialog.current?.close();
  }, [prompt]);
  const historyIndex = useRef<number | null>(null);
  const restoring = useRef(false);
  const allowedPop = useRef(false);
  const afterRestore = useRef<(() => void) | null>(null);
  const navigator = useMemo(() => {
    const samePage = (to: To) => {
      const current = locationRef.current;
      const url = new URL(typeof to === "string" ? to : `${to.pathname ?? current.pathname}${to.search ?? ""}${to.hash ?? ""}`, window.location.origin + current.pathname + current.search);
      return url.pathname === current.pathname && url.search === current.search;
    };
    return { ...navigation.navigator,
      push: (...args: Parameters<typeof navigation.navigator.push>) => samePage(args[0]) ? navigation.navigator.push(...args) : request(() => navigation.navigator.push(...args)),
      replace: (...args: Parameters<typeof navigation.navigator.replace>) => samePage(args[0]) ? navigation.navigator.replace(...args) : request(() => navigation.navigator.replace(...args)),
      go: (delta: number) => {
        // An unguarded go may be outside history and emit no POP. It must not
        // leave a bypass armed for a later, genuinely dirty Back operation.
        const guarded = active().length > 0;
        request(() => { if (guarded && historyIndex.current !== null) allowedPop.current = true; navigation.navigator.go(delta); });
      },
    };
  }, [navigation, request, active]);
  useLayoutEffect(() => { historyIndex.current = typeof window.history.state?.idx === "number" ? window.history.state.idx : null; }, [location]);
  useEffect(() => {
    const unload = (event: BeforeUnloadEvent) => { if (!bypassing.current && active().length) { event.preventDefault(); event.returnValue = ""; } };
    const pop = (event: PopStateEvent) => {
      if (allowedPop.current) { allowedPop.current = false; return; }
      if (restoring.current) { restoring.current = false; event.stopImmediatePropagation(); const action = afterRestore.current; afterRestore.current = null; action?.(); return; }
      if (bypassing.current || !active().length) return;
      if (window.location.pathname === locationRef.current.pathname && window.location.search === locationRef.current.search) return;
      const previous = historyIndex.current, target: unknown = window.history.state?.idx;
      if (previous === null || typeof target !== "number" || previous === target) return;
      event.stopImmediatePropagation(); restoring.current = true;
      window.history.go(previous - target);
      request(() => { const action = () => { allowedPop.current = true; window.history.go(target - previous); }; if (restoring.current) afterRestore.current = action; else action(); });
    };
    window.addEventListener("beforeunload", unload);
    popGuard = pop;
    return () => { window.removeEventListener("beforeunload", unload); if (popGuard === pop) popGuard = null; };
  }, [active, request]);
  const value = useMemo(() => ({ entries: entries.current, memory: memory.current, request, bypass }), [request, bypass]);
  return <Context.Provider value={value}><UNSAFE_NavigationContext.Provider value={{ ...navigation, navigator }}>
    {children}
    <dialog ref={dialog} className="settings-leave-dialog work-leave-dialog" aria-labelledby="work-leave-title" onCancel={event => { event.preventDefault(); dismiss(); }} onKeyDown={event => {
      if (event.key !== "Tab") return;
      const buttons = event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)");
      const first = buttons[0], last = buttons[buttons.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }}>
      <h2 id="work-leave-title">离开当前页面？</h2><p>{prompt?.message}</p>
      <div className="form-actions"><button ref={cancel} type="button" onClick={dismiss}>继续处理</button><button type="button" onClick={() => {
        const selected = pending.current;
        pending.current = null; setPrompt(null); dialog.current?.close();
        if (!selected) return;
        if (selected.discard) active().forEach(entry => entry.discard?.());
        bypass(selected.action);
      }}>{prompt?.accept ?? "离开页面"}</button></div>
    </dialog>
  </UNSAFE_NavigationContext.Provider></Context.Provider>;
}

export function useWorkProtection() {
  const context = useContext(Context);
  if (!context) throw new Error("WorkProtection is required");
  return context;
}
export function usePageWork(work: Work) {
  const { entries } = useWorkProtection();
  const entry = useRef(work); entry.current = work;
  useLayoutEffect(() => { const key = Symbol(); entries.set(key, entry); return () => { entries.delete(key); }; }, [entries]);
}

/** Only ordinary item/knowledge edits use this hook. No Storage, IndexedDB, file or key fields. */
export function useMemoryEdit<T>(key: string, initial: T) {
  const { memory } = useWorkProtection();
  const [slot] = useState(() => {
    const found = memory.get(key) as { value: T } | undefined;
    const entry = found ?? { value: initial };
    memory.set(key, entry); return entry;
  });
  const [state, setState] = useState(slot.value);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const set = useCallback((update: SetStateAction<T>) => {
    // Explicit discard/logout invalidates outstanding writes to this memory slot.
    if (memory.get(key) !== slot) return;
    slot.value = typeof update === "function" ? (update as (previous: T) => T)(slot.value) : update;
    if (mounted.current) setState(slot.value);
  }, [key, memory, slot]);
  const clear = useCallback(() => { memory.delete(key); }, [key, memory]);
  return [state, set, clear] as const;
}
