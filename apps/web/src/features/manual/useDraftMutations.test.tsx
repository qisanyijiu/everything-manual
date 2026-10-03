import { WorkProtection } from "../shell/work-protection";
import { MemoryRouter } from "react-router";
import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { getDraft, patchDraft } from "../../api/endpoints";
import { useDraftMutations } from "./useDraftMutations";
vi.mock("../../api/endpoints", () => ({ getDraft: vi.fn(), patchDraft: vi.fn() }));
beforeEach(() => vi.resetAllMocks());
function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  client.setQueryData(["draft", "item", "draft"], { etag: '"r1"' });
  const hook = renderHook(({ model }) => useDraftMutations("item", "draft", model), { initialProps: { model: "m1/hash" }, wrapper: ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}><MemoryRouter><WorkProtection>{children}</WorkProtection></MemoryRouter></QueryClientProvider> });
  return { ...hook, client };
}
it("readback is required before success/cache update and duplicate writes are locked", async () => {
  const { result, client } = setup();
  let finish!: (value: Awaited<ReturnType<typeof getDraft>>) => void;
  vi.mocked(getDraft).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  vi.mocked(patchDraft).mockResolvedValue({} as Awaited<ReturnType<typeof patchDraft>>);
  let saved!: Promise<boolean>;
  await act(async () => { saved = result.current.updateEntities('"r1"', { entities: {} }); await Promise.resolve(); });
  expect(result.current.pending).toBe(true);
  expect(client.getQueryData(["draft", "item", "draft"])).toEqual({ etag: '"r1"' });
  await act(async () => { expect(await result.current.updateEntities('"r1"', { entities: {} })).toBe(false); });
  await act(async () => { finish({ etag: '"r2"' } as Awaited<ReturnType<typeof getDraft>>); expect(await saved).toBe(true); });
  expect(patchDraft).toHaveBeenCalledTimes(1);
  expect(client.getQueryData(["draft", "item", "draft"])).toEqual({ etag: '"r2"' });
});
it("written but unread result blocks automatic resubmit until explicit reread succeeds", async () => {
  const { result, client } = setup();
  vi.mocked(patchDraft).mockResolvedValue({} as Awaited<ReturnType<typeof patchDraft>>);
  vi.mocked(getDraft).mockRejectedValue(new Error("offline"));
  await act(async () => { expect(await result.current.updateEntities('"r1"', { entities: {} })).toBe(false); });
  expect(result.current.lastError).toContain("修改已提交"); expect(result.current.needsRead).toBe(true);
  expect(client.getQueryData(["draft", "item", "draft"])).toEqual({ etag: '"r1"' });
  await act(async () => { await result.current.updateEntities('"r1"', { entities: {} }); });
  expect(patchDraft).toHaveBeenCalledTimes(1);
  act(() => result.current.clearError()); expect(result.current.needsRead).toBe(false);
});
it("loaded declaration is tied to current model identity", () => {
  const { result, rerender } = setup(); act(() => result.current.setModelReady(true)); expect(result.current.modelReady).toBe(true);
  rerender({ model: "m2/other" }); expect(result.current.modelReady).toBe(false);
});
