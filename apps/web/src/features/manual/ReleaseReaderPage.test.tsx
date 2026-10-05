import { fireEvent, render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes, useNavigate } from "react-router";
import { beforeEach, expect, it, vi } from "vitest";
import type { ViewerPanelProps } from "../viewer/ViewerPanel";
import { getRelease } from "../../api/endpoints";
import { ReleaseReaderPage } from "./ReleaseReaderPage";

vi.mock("../../api/endpoints", async (original) => ({ ...await original<typeof import("../../api/endpoints")>(), getRelease: vi.fn() }));
vi.mock("../shell/useBreakpoint", () => ({ useBreakpoint: () => "wide" }));
vi.mock("../viewer/ViewerPanel", () => ({ ViewerPanel: (props: ViewerPanelProps) => <div data-testid="viewer" data-selected={props.selectedHotspotId}>
  {props.hotspots?.map((hotspot) => <button key={hotspot.id} onClick={() => props.onHotspotSelect?.(hotspot.id)}>热点 {hotspot.id}</button>)}
</div> }));
vi.mock("../viewer/OriginalDocumentPanel", () => ({ default: ({ assetId, pageNumber }: { assetId: string; pageNumber: number }) => <p data-testid="original-page">{assetId} 第 {pageNumber} 页</p> }));

const evidence = [{ documentId: "doc", pageNumber: 3, quote: "原始出处" }];
function manifest(version?: string) {
  return {
    knowledge: {
      model: { revisionId: "model", sha256: "hash", assetId: "glb", validationState: "validated" },
      knowledge: {
        parts: [
          { id: "p", name: "原部件", description: "原说明", evidence },
          { id: "text", name: "原文字条目", description: "", evidence: [] },
          { id: "q", name: "无定位部件", description: "", evidence: [] },
        ],
        steps: [{ id: "s", title: "原步骤", orderedActions: ["原操作"], safetyNotes: ["保留安全提示"], partIds: ["p"], evidence }],
        specs: [{ id: "v", label: "原规格", value: "原数值", evidence }],
      },
      hotspots: [{ id: "h", partId: "p", status: "confirmed", anchor: { modelRevisionId: "model", modelSha256: "hash", positionLocal: [1, 2, 3] } }],
    },
    documents: [{ documentId: "doc", sourceAssetId: "pdf", title: "原始说明书" }],
    review: version === undefined ? undefined : { entities: {
      p: { userEdited: { name: `${version}部件`, description: `${version}说明` } },
      text: { textOnly: true, userEdited: { name: `${version}文字条目` } },
      s: { userEdited: { title: `${version}步骤`, orderedActions: [`${version}操作一`, `${version}操作二`] } },
      v: { userEdited: { label: `${version}规格`, value: `${version}数值` } },
    } },
  };
}

function SwitchVersion() {
  const navigate = useNavigate();
  return <><button onClick={() => navigate("/items/item/releases/A")}>打开 A</button><button onClick={() => navigate("/items/item/releases/B")}>打开 B</button></>;
}
function show() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}><MemoryRouter initialEntries={["/items/item/releases/A"]}>
    <SwitchVersion /><Routes><Route path="/items/:itemId/releases/:releaseId" element={<ReleaseReaderPage />} /></Routes>
  </MemoryRouter></QueryClientProvider>);
}
beforeEach(() => {
  vi.mocked(getRelease).mockReset();
  vi.mocked(getRelease).mockImplementation(async (_itemId, releaseId) => ({ data: {
    id: releaseId, draftRevision: 2, modelRevisionId: "model", manifestSha256: "frozenhash", manifest: manifest(releaseId),
  } } as Awaited<ReturnType<typeof getRelease>>));
});

it("renders edited text in all reading contexts with source comparison and stable ID/evidence linkage", async () => {
  show();
  const part = await screen.findByTestId("reader-part-p");
  const step = screen.getByTestId("reader-step-s");
  const spec = screen.getByTestId("reader-spec-v");
  const partButton = within(part).getByRole("button", { name: "A部件" });
  expect(part).toHaveTextContent("A说明");
  expect(within(step).getByRole("button", { name: "A步骤" })).toBeVisible();
  expect(step).toHaveTextContent("A操作一"); expect(step).toHaveTextContent("A操作二");
  expect(step).toHaveTextContent("保留安全提示");
  expect(spec).toHaveTextContent("A规格：A数值");
  expect(screen.getByTestId("reader-current-step")).toHaveTextContent("A步骤");
  expect(screen.getByTestId("text-only-note")).toHaveTextContent("A文字条目");
  fireEvent.click(partButton);
  expect(screen.getByTestId("reader-notice")).toHaveTextContent("A部件");
  expect(screen.getByTestId("viewer")).toHaveAttribute("data-selected", "h");
  fireEvent.click(within(step).getByRole("button", { name: "A部件" }));
  expect(partButton).toHaveAttribute("aria-current", "true");
  fireEvent.click(screen.getByRole("button", { name: "热点 h" }));
  expect(partButton).toHaveAttribute("aria-current", "true");
  for (const [id, originalText] of [["p", "原部件"], ["s", "原步骤"], ["v", "原规格：原数值"]]) {
    const revision = screen.getByTestId(`reader-revision-${id}`);
    expect(revision).toHaveTextContent("已修订（人工）");
    expect(within(revision).getByText(originalText!)).not.toBeVisible();
    fireEvent.click(within(revision).getByText("查看原文本"));
    expect(within(revision).getByText(originalText!)).toBeVisible();
  }
  for (const entry of [part, step, spec]) {
    fireEvent.click(within(entry).getByRole("button", { name: "查看出处 · 原始说明书 · 第 3 页" }));
    expect(await screen.findByTestId("original-page")).toHaveTextContent("pdf 第 3 页");
  }
});

it("switches releases using each frozen overlay, without reading a draft or mutating cached manifests", async () => {
  const frozenA = manifest("A"); const frozenB = manifest("B");
  const before = JSON.stringify([frozenA, frozenB]);
  vi.mocked(getRelease).mockImplementation(async (_itemId, releaseId) => ({ data: {
    id: releaseId, draftRevision: 2, modelRevisionId: "model", manifestSha256: "frozenhash", manifest: releaseId === "A" ? frozenA : frozenB,
  } } as Awaited<ReturnType<typeof getRelease>>));
  show();
  expect(await screen.findByTestId("reader-part-p")).toHaveTextContent("A部件");
  fireEvent.click(within(screen.getByTestId("reader-part-p")).getByRole("button", { name: "A部件" }));
  expect(screen.getByTestId("reader-notice")).toHaveTextContent("A部件");
  fireEvent.click(screen.getByRole("button", { name: "打开 B" }));
  expect(await screen.findByRole("button", { name: "B步骤" })).toBeVisible();
  expect(screen.queryByTestId("reader-notice")).not.toBeInTheDocument();
  expect(screen.getByTestId("reader-spec-v")).toHaveTextContent("B规格：B数值");
  fireEvent.click(screen.getByRole("button", { name: "打开 A" }));
  expect(await screen.findByRole("button", { name: "A步骤" })).toBeVisible();
  expect(screen.getByTestId("reader-part-p")).toHaveTextContent("A部件");
  expect(screen.getByTestId("reader-spec-v")).toHaveTextContent("A规格：A数值");
  expect(getRelease).toHaveBeenCalledWith("item", "A"); expect(getRelease).toHaveBeenCalledWith("item", "B");
  expect(JSON.stringify([frozenA, frozenB])).toBe(before);
});

it("renders an older release without an overlay using original text and no revision marker", async () => {
  vi.mocked(getRelease).mockResolvedValue({ data: { id: "A", draftRevision: 1, modelRevisionId: "model", manifestSha256: "hash", manifest: manifest() } } as Awaited<ReturnType<typeof getRelease>>);
  show();
  expect(await screen.findByTestId("reader-part-p")).toHaveTextContent("原部件");
  expect(screen.getByTestId("reader-current-step")).toHaveTextContent("原步骤");
  expect(screen.queryByText("已修订（人工）")).not.toBeInTheDocument();
});

it("reads each release's safety correction or explicit removal while preserving original notes for comparison", async () => {
  const frozenA = manifest(); const frozenB = manifest();
  const releases = {
    A: { ...frozenA, review: { entities: { s: { userEdited: { safetyNotes: ["红灯亮起之后等待至少四秒"] } } } } },
    B: { ...frozenB, review: { entities: { s: { userEdited: { safetyNotes: [] } } } } },
  };
  const before = JSON.stringify(releases);
  vi.mocked(getRelease).mockImplementation(async (_itemId, releaseId) => ({ data: {
    id: releaseId, draftRevision: 2, modelRevisionId: "model", manifestSha256: "frozenhash", manifest: releases[releaseId as "A" | "B"],
  } } as Awaited<ReturnType<typeof getRelease>>));
  show();
  expect(await screen.findByText("注意：红灯亮起之后等待至少四秒")).toBeVisible();
  let revision = screen.getByTestId("reader-revision-s");
  expect(within(revision).getByText("原注意事项：保留安全提示")).not.toBeVisible();
  fireEvent.click(within(revision).getByText("查看原文本"));
  expect(within(revision).getByText("原注意事项：保留安全提示")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "打开 B" }));
  await screen.findByRole("button", { name: "原步骤" });
  expect(screen.queryByText("注意：红灯亮起之后等待至少四秒")).not.toBeInTheDocument();
  expect(screen.queryByText("注意：保留安全提示")).not.toBeInTheDocument();
  revision = screen.getByTestId("reader-revision-s");
  fireEvent.click(within(revision).getByText("查看原文本"));
  expect(within(revision).getByText("原注意事项：保留安全提示")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "打开 A" }));
  expect(await screen.findByText("注意：红灯亮起之后等待至少四秒")).toBeVisible();
  expect(JSON.stringify(releases)).toBe(before);
});

it("部件状态牌区分热点/无热点/仅文本，选中项带「已选」语义与 aria-current（VS-03 / AC-VS-005）", async () => {
  show();
  const part = await screen.findByTestId("reader-part-p");
  const textOnly = screen.getByTestId("reader-part-text");
  const noHotspot = screen.getByTestId("reader-part-q");

  // 状态牌：已确认热点（success）/无热点（warning）/仅文本条目（中性）。
  expect(part).toHaveTextContent("热点 1");
  expect(part.querySelector(".status-label--success")).not.toBeNull();
  expect(noHotspot).toHaveTextContent("无热点");
  expect(noHotspot.querySelector(".status-label--warning")).not.toBeNull();
  expect(textOnly).toHaveTextContent("仅文本条目");

  // 选中：底/边框/aria-current（样式由 theme.css token 提供）+ 「已选」文字语义。
  const partButton = within(part).getByRole("button", { name: "A部件" });
  expect(partButton).toHaveAttribute("aria-current", "false");
  fireEvent.click(partButton);
  expect(partButton).toHaveAttribute("aria-current", "true");
  expect(part).toHaveTextContent("已选");
  expect(textOnly).not.toHaveTextContent("已选");

  // 无热点部件：模型区不新增标记（热点集合不变），文字提示「暂无定位」。
  expect(screen.getByTestId("viewer").querySelectorAll("button")).toHaveLength(1);
  fireEvent.click(within(noHotspot).getByRole("button", { name: "无定位部件" }));
  expect(screen.getByTestId("reader-notice")).toHaveTextContent("暂无定位");
  expect(noHotspot).toHaveTextContent("已选");
  expect(noHotspot).not.toHaveTextContent("已修订");
});
