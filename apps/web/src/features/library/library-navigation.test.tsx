import { StrictMode } from "react";
import { act, render } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { afterEach, expect, it, vi } from "vitest";
import { rememberLibraryItem, useLibraryPosition } from "./library-navigation";

afterEach(() => vi.unstubAllGlobals());
it("restores cached rows after StrictMode cancels its first animation frame", () => {
  const frames = new Map<number, FrameRequestCallback>(); let serial = 0;
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => { frames.set(++serial, callback); return serial; }));
  vi.stubGlobal("cancelAnimationFrame", vi.fn((id: number) => frames.delete(id)));
  const scroll = vi.fn(); vi.stubGlobal("scrollTo", scroll);
  rememberLibraryItem("fixture", "default", "?q=cached");
  function CachedList() { useLibraryPosition(true); return <div data-library-item="fixture">row</div>; }
  render(<StrictMode><MemoryRouter><CachedList /></MemoryRouter></StrictMode>);
  expect(frames.size).toBe(1);
  act(() => { for (const callback of frames.values()) callback(0); frames.clear(); });
  expect(scroll).toHaveBeenCalledTimes(1);
});
