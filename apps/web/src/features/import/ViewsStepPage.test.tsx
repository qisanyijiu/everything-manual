/**
 * 视图排列页：缺项提示必须反映当前（含未保存）的槽位排列，而不只是已保存的照片。
 * 回归：把候选图拖入槽位后仍提示「缺少 front（正面）视图照片」，直到点击保存才消失。
 */

import { fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { jsonResponse, renderApp } from "../../test/render";

const ITEM = { id: "item-vc", name: "相机", model: "VC-1", brand: null, variant: null, revision: 1,
  createdAt: "2026-10-03T00:00:00Z", updatedAt: "2026-10-03T00:00:00Z", archivedAt: null };
const SESSION = { data: { admin: { id: "admin" }, csrfToken: "csrf", expiresAt: "2099-01-01T00:00:00Z" } };
const candidate = (id: string, view: string | null) => ({
  id, itemId: ITEM.id, assetId: `asset-${id}`, documentId: "doc-1", pageNumber: 6, source: "region",
  suggestedView: view, confidence: view ? 0.9 : null, note: null, createdAt: "2026-10-03T00:00:00Z",
});
const CANDIDATES = [candidate("c1", "front"), candidate("c2", "left")];

function stub() {
  const arranged: unknown[] = [];
  let photos: { id: string; itemId: string; assetId: string; view: string; revision: number; createdAt: string; updatedAt: string }[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url === "/api/v1/auth/session") return jsonResponse(SESSION);
    if (url === `/api/v1/items/${ITEM.id}`) return jsonResponse({ data: ITEM }, { etag: '"r1"' });
    if (url === `/api/v1/items/${ITEM.id}/photos`) return jsonResponse({ data: photos, nextCursor: null });
    if (url === `/api/v1/items/${ITEM.id}/view-candidates`) return jsonResponse({ data: CANDIDATES });
    if (url.startsWith(`/api/v1/items/${ITEM.id}/documents`)) return jsonResponse({ data: [], nextCursor: null });
    if (url === `/api/v1/items/${ITEM.id}/photos/arrangement` && init?.method === "PUT") {
      const slots = (JSON.parse(String(init.body)) as { slots: Record<string, string | null> }).slots;
      arranged.push(slots);
      photos = Object.entries(slots).flatMap(([view, assetId], i) =>
        assetId ? [{ id: `p${i}`, itemId: ITEM.id, assetId, view, revision: 1, createdAt: "x", updatedAt: "x" }] : []);
      return jsonResponse({ data: photos, nextCursor: null });
    }
    return jsonResponse({ data: [], nextCursor: null });
  });
  vi.stubGlobal("fetch", fetchMock);
  return { fetchMock, arranged };
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("视图排列页的缺项提示", () => {
  it("把候选放进槽位后立即更新缺项提示（无需先保存），并提示尚未保存", async () => {
    stub();
    renderApp({ route: `/items/${ITEM.id}/import/views` });
    await screen.findByTestId("arrange-tray");
    expect(await screen.findByText("缺少 front（正面）视图照片。")).toBeTruthy();

    // 键盘替代路径（与拖拽同一状态转换）：把 c1 放到正面、c2 放到左侧。
    fireEvent.change(await screen.findByLabelText("把这张图放到", { selector: "#place-asset-c1" }), { target: { value: "front" } });
    fireEvent.change(screen.getByLabelText("把这张图放到", { selector: "#place-asset-c2" }), { target: { value: "left" } });

    await waitFor(() => expect(screen.queryByText("缺少 front（正面）视图照片。")).toBeNull());
    expect(screen.queryByText(/缺少侧面视图/)).toBeNull();
    expect(screen.getByTestId("arrangement-unsaved").textContent).toContain("尚未保存");
  });

  it("按建议填入空槽后也立即更新缺项提示", async () => {
    stub();
    renderApp({ route: `/items/${ITEM.id}/import/views` });
    await screen.findByTestId("arrange-tray");
    expect(await screen.findByText("缺少 front（正面）视图照片。")).toBeTruthy();
    fireEvent.click(screen.getByTestId("autofill"));
    await waitFor(() => expect(screen.queryByText("缺少 front（正面）视图照片。")).toBeNull());
    expect(screen.queryByText(/缺少侧面视图/)).toBeNull();
    expect(screen.getByTestId("arrangement-unsaved")).toBeTruthy();
  });
});
