import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { errorResponse, jsonResponse, renderApp, setViewportWidth } from "../../test/render";
import { rememberPreparationId } from "./preparation-pointer";

const ITEM = { id: "item-ia", name: "体验相机", model: "IA-01", brand: null, variant: null,
  revision: 1, createdAt: "2026-09-19T00:00:00Z", updatedAt: "2026-09-19T00:00:00Z", archivedAt: null };
const DOCUMENT = { id: "doc-1", title: "说明书", sourceSha256: "c".repeat(64), sourceAssetId: "asset-original" };
const PREPARATION = { id: "prep-1", documentId: "doc-1", sourceSha256: DOCUMENT.sourceSha256, state: "ready", pageCount: 2,
  createdAt: "2026-09-19T00:00:00Z", updatedAt: "2026-09-19T00:00:00Z", revision: 1, clientDerived: true };
const READINESS = { compatible: true, formatVersion: "v1", reason: null, explanation: null, completedPageCount: 2, completedPages: [1,2], missingPages: [] };
const LABEL = "我已阅读并确认将上述资料发送给对应供应商";

function quote(id = "quote-1", expiresAt = new Date(Date.now() + 600_000).toISOString()) {
  const amounts = {
    tripo: { estimatedMinor: 900, upperBoundMinor: 1000, estimatedDisplay: "9.00 credits", upperBoundDisplay: "10.00 credits" },
    manualAi: { estimatedMinor: 8000, upperBoundMinor: 10000, estimatedDisplay: "0.008 USD", upperBoundDisplay: "0.01 USD" },
  };
  return { id, itemId: ITEM.id, preparationId: "prep-1", expiresAt, amounts, pageCount: 2,
    priceVersion: "price-ia", priceSnapshotDate: "2026-09-19", pageRange: { from: 1, to: 2 },
    budgetNotice: "预算只限制本应用主动发起的请求，不是供应商账户级硬封顶。",
    sendScope: {
      tripo: { views: [{ photoId: "photo-front", view: "front", sha256: "a".repeat(64) }, { photoId: "photo-left", view: "left", sha256: "b".repeat(64) }],
        model: "tripo-fixture", preset: "standard", parameters: { faceLimit: 100000, texture: true, pbr: true, textureQuality: "standard", geometryQuality: "standard" } },
      manualAi: { itemName: ITEM.name, itemModel: ITEM.model, pageFrom: 1, pageTo: 2, pageCount: 2,
        textPages: [1], imagePages: [2], model: "manual-fixture", promptVersion: "v1", maxOutputTokens: 1000 },
      priceVersion: "price-ia", priceSnapshotDate: "2026-09-19", plannedUpperBound: amounts,
      budgetNotice: "预算只限制本应用主动发起的请求，不是供应商账户级硬封顶。",
    },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

type Handler = (url: string, init: RequestInit) => Response | Promise<Response> | undefined;
function setup(handler: Handler = () => undefined, generation = true) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = String(input);
    const custom = handler(url, init);
    if (custom !== undefined) return custom;
    if (url.endsWith("/auth/session")) return jsonResponse({ data: { admin: { id: "admin" }, csrfToken: "fixture", expiresAt: "2099-01-01T00:00:00Z" } });
    if (url === `/api/v1/items/${ITEM.id}`) return jsonResponse({ data: ITEM }, { etag: '"r1"' });
    if (url.includes("/items/summaries?")) return jsonResponse({ data: [{ itemId: ITEM.id, action: "confirm", documentId: DOCUMENT.id, preparationId: PREPARATION.id, latestQuoteId: null, consumedJobId: null, quoteExpiresAt: null, steps: { basic: "complete", document: "complete", views: "complete", prepare: "complete", confirm: "missing" } }] });
    if (/\/estimates\/[^/]+$/.test(url)) return jsonResponse({ data: { ...quote(), confirmedAt: "2026-09-19T01:00:00Z", consumedJobId: null } });
    if (url.endsWith("/documents")) return jsonResponse({ data: [DOCUMENT], nextCursor: null });
    if (url.endsWith("/photos")) return jsonResponse({ data: [{ id: "photo-front", view: "front" }, { id: "photo-left", view: "left" }], nextCursor: null });
    if (url.endsWith("/settings/status")) return jsonResponse({ data: { capabilities: { generation } } });
    if (url.includes("/documents/doc-1/preparations?")) return jsonResponse({ data: [{ preparation: PREPARATION, readiness: READINESS }], nextCursor: null, recommendedPreparationId: "prep-1", recommended: { preparation: PREPARATION, readiness: READINESS } });
    if (url.endsWith("/preparations/prep-1")) return jsonResponse({ data: { ...PREPARATION, pages: [], missingPages: [], readiness: READINESS } });
    if (url.endsWith("/estimates")) return jsonResponse({ data: quote() });
    if (url.endsWith("/confirm")) return jsonResponse({ data: { confirmedAt: "2026-09-19T01:00:00Z" } });
    throw new Error(`Unexpected ${init.method ?? "GET"} ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
  renderApp({ route: `/items/${ITEM.id}/import/confirm` });
  return fetchMock;
}

const generate = () => screen.getByTestId("generate-button");
it("PC06 历史报价模型不可用时禁止确认/生成并保留设置纠正入口", async () => {
  const calls = setup((url) => url.endsWith("/estimates") ? jsonResponse({ data: { ...quote(), modelIssue: "suspectedCredential" } }) : undefined);
  await screen.findAllByText("此报价的模型信息不可用，请重新获取报价。");
  expect(screen.getByRole("checkbox", { name: LABEL })).toBeDisabled();
  expect(generate()).toBeDisabled();
  expect(screen.getByRole("link", { name: "前往设置" })).toHaveAttribute("href", "/settings");
  expect(calls.mock.calls.filter(([, init]) => init?.method === "POST" && String(init.body ?? "").includes("quoteId"))).toHaveLength(0);
});
async function ready() { await screen.findByTestId("quote-panel"); }
async function confirm() {
  const checkbox = screen.getByRole("checkbox", { name: LABEL });
  await waitFor(() => expect(checkbox).toBeEnabled());
  fireEvent.click(checkbox);
  await waitFor(() => expect(generate()).toBeEnabled());
}

beforeEach(() => { setViewportWidth(1440); rememberPreparationId(ITEM.id, "prep-1"); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); sessionStorage.clear(); localStorage.clear(); });

describe("interaction-a：当前报价的显式确认", () => {
  it("保存中不放行，取消立即禁用，重新勾选必须再等服务端确认", async () => {
    let pending = deferred<Response>();
    setup((url) => url.endsWith("/confirm") ? pending.promise : undefined);
    await ready();
    const checkbox = screen.getByRole("checkbox", { name: LABEL });
    expect(checkbox).not.toBeChecked();
    fireEvent.click(checkbox);
    expect(checkbox).toBeChecked();
    expect(screen.getByText("正在保存确认…")).toBeInTheDocument();
    expect(generate()).toBeDisabled();
    await act(async () => pending.resolve(jsonResponse({ data: { confirmedAt: "2026-09-19T01:00:00Z" } })));
    await waitFor(() => expect(generate()).toBeEnabled());
    fireEvent.click(checkbox);
    expect(generate()).toBeDisabled();
    expect(screen.queryByTestId("confirmed-at")).not.toBeInTheDocument();
    pending = deferred<Response>();
    fireEvent.click(checkbox);
    expect(generate()).toBeDisabled();
    await act(async () => pending.resolve(jsonResponse({ data: { confirmedAt: "2026-09-19T01:01:00Z" } })));
    await waitFor(() => expect(generate()).toBeEnabled());
  });

  it("确认失败回退勾选并保留可重试错误", async () => {
    setup((url) => url.endsWith("/confirm") ? errorResponse(500, "INTERNAL_ERROR", "确认暂未保存，请重试") : undefined);
    await ready();
    fireEvent.click(screen.getByRole("checkbox", { name: LABEL }));
    expect(await screen.findByTestId("confirm-error")).toHaveTextContent("确认暂未保存");
    expect(screen.getByRole("checkbox", { name: LABEL })).not.toBeChecked();
    expect(generate()).toBeDisabled();
  });

  it("旧报价确认晚返回不会确认新报价，到期只在用户操作后重报", async () => {
    const oldConfirmation = deferred<Response>();
    let quoteCount = 0;
    setup((url) => {
      if (url.endsWith("/estimates")) return jsonResponse({ data: ++quoteCount === 1 ? quote("old", new Date(Date.now() + 2000).toISOString()) : quote("new") });
      if (url.endsWith("/old/confirm")) return oldConfirmation.promise;
      return undefined;
    });
    await ready();
    fireEvent.click(screen.getByRole("checkbox", { name: LABEL }));
    const future = Date.now() + 3000;
    vi.spyOn(Date, "now").mockReturnValue(future);
    await waitFor(() => expect(screen.getByTestId("quote-expiry")).toHaveTextContent("已过期"), { timeout: 2500 });
    expect(quoteCount).toBe(1);
    fireEvent.click(screen.getByTestId("requote-button"));
    await waitFor(() => expect(screen.getByRole("checkbox", { name: LABEL })).toBeEnabled());
    expect(quoteCount).toBe(2);
    await act(async () => oldConfirmation.resolve(jsonResponse({ data: { confirmedAt: "2026-09-19T01:00:00Z" } })));
    expect(screen.queryByTestId("confirmed-at")).not.toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: LABEL })).not.toBeChecked();
    expect(generate()).toBeDisabled();
    await confirm();
  });

  it("预算格式与各供应商低上界错误保持独立", async () => {
    setup(); await ready(); await confirm();
    fireEvent.change(screen.getByLabelText("Tripo credits"), { target: { value: "bad" } });
    expect(generate()).toBeDisabled();
    expect(screen.getByTestId("generate-reason")).toHaveTextContent("格式不正确");
    fireEvent.change(screen.getByLabelText("Tripo credits"), { target: { value: "9" } });
    expect(generate()).toBeDisabled();
    expect(screen.getByTestId("generate-reason")).toHaveTextContent("低于");
    expect(screen.getByLabelText("Tripo credits")).toHaveAttribute("aria-describedby", "budget-tripo-hint");
    expect(screen.getByLabelText("说明书 AI USD")).toHaveAttribute("aria-describedby", "budget-hint");
    fireEvent.change(screen.getByLabelText("Tripo credits"), { target: { value: "10" } });
    expect(generate()).toBeEnabled();
  });

  it("提交中禁用，断连后同页面显式重试保留幂等键，受理后不再提交", async () => {
    let pending = deferred<Response>();
    const calls: RequestInit[] = [];
    setup((url, init) => {
      if (url.endsWith("/jobs")) { calls.push(init); return pending.promise; }
      return undefined;
    });
    await ready(); await confirm();
    fireEvent.click(generate()); fireEvent.click(generate());
    expect(generate()).toBeDisabled();
    expect(screen.getByRole("checkbox", { name: LABEL })).toBeDisabled();
    expect(calls).toHaveLength(1);
    await act(async () => pending.reject(new TypeError("Failed to fetch")));
    await screen.findByRole("button", { name: "重试同一提交（使用原授权）" });
    expect(generate()).toBeDisabled();
    pending = deferred<Response>();
    fireEvent.click(screen.getByRole("button", { name: "重试同一提交（使用原授权）" }));
    expect(calls).toHaveLength(2);
    expect(new Headers(calls[0]?.headers).get("Idempotency-Key")).toBe(new Headers(calls[1]?.headers).get("Idempotency-Key"));
    expect(new Headers(calls[0]?.headers).get("Idempotency-Key")).toBeTruthy();
    await act(async () => pending.resolve(jsonResponse({ data: { id: "job-1" } }, { status: 202 })));
    expect(await screen.findByTestId("job-accepted")).toHaveTextContent("需要服务进程保持运行");
    expect(screen.getByRole("link", { name: "查看任务详情" })).toHaveAttribute("href", "/jobs/job-1");
    expect(screen.queryByTestId("generate-button")).not.toBeInTheDocument();
    expect(calls).toHaveLength(2);
  });

  it("已有任务响应锁定生成；输入变化响应保留返回资料入口", async () => {
    let existing = false;
    setup((url) => {
      if (url.endsWith("/jobs")) return errorResponse(409, "CONFLICT", "不能提交", existing ? { reason: "quoteAlreadyUsed", jobId: "existing-job" } : { reason: "inputChanged" });
      if (existing && /\/estimates\/[^/]+$/.test(url)) return jsonResponse({ data: { ...quote(), consumedJobId: "existing-job", confirmedAt: "2026-09-19T01:00:00Z" } });
      return undefined;
    });
    await ready(); await confirm();
    fireEvent.click(generate());
    expect(await screen.findByRole("link", { name: "返回检查视图与资料" })).toHaveAttribute("href", `/items/${ITEM.id}/import/views`);
    existing = true;
    await waitFor(() => expect(generate()).toBeEnabled());
    fireEvent.click(generate());
    expect(await screen.findByRole("link", { name: "查看任务详情" })).toHaveAttribute("href", "/jobs/existing-job");
    expect(screen.queryByTestId("generate-button")).not.toBeInTheDocument();
  });

  it("供应商未就绪时展示真实缺项，不请求报价也不显示金额", async () => {
    const calls = setup(undefined, false);
    expect(await screen.findByRole("link", { name: "查看服务状态" })).toBeInTheDocument();
    expect(screen.queryByTestId("quote-panel")).not.toBeInTheDocument();
    expect(generate()).toBeDisabled();
    expect(calls.mock.calls.some(([url]) => String(url).endsWith("/estimates"))).toBe(false);
  });
});

describe("PC03B persisted quote recovery", () => {
  it("saved consumption restores the unique job before expired quote or pending configuration gates", async () => {
    const calls=setup((url) => {
      if (url.includes("/items/summaries?")) return jsonResponse({data:[{itemId:ITEM.id,documentId:DOCUMENT.id,latestQuoteId:"old-consumed",steps:{basic:"complete",document:"complete",views:"complete",prepare:"complete",confirm:"complete"}}]});
      if (url.endsWith("/estimates/old-consumed")) return jsonResponse({data:{...quote("old-consumed","2020-01-01T00:00:00Z"),consumedJobId:"saved-job",confirmedAt:"2020-01-01T00:00:00Z"}});
      if (url.endsWith("/settings/status")) return jsonResponse({data:{capabilities:{generation:true},providerConfigPending:true}});
      return undefined;
    });
    expect(await screen.findByTestId("job-accepted")).toHaveTextContent("saved-job");
    expect(screen.getByRole("link",{name:"查看任务详情"})).toHaveAttribute("href","/jobs/saved-job");
    expect(calls.mock.calls.filter(([,init])=>init?.method==="POST")).toHaveLength(0);
  });
  it("lost response plus failed consumption GET stays read-only; retry GET can recover acceptance", async () => {
    let failed=true;
    const calls=setup((url) => {
      if(url.endsWith("/jobs")) return Promise.reject(new TypeError("lost response"));
      if(url.endsWith("/estimates/quote-1")) return failed ? errorResponse(500,"INTERNAL_ERROR","fixture cannot read") : jsonResponse({data:{...quote(),consumedJobId:"recovered-job",confirmedAt:"2026-09-19T01:00:00Z"}});
      return undefined;
    });
    await ready();await confirm();fireEvent.click(generate());
    await waitFor(()=>expect(screen.getByTestId("submission-recovery")).toHaveTextContent("fixture cannot read"));
    expect(generate()).toBeDisabled();expect(screen.getByRole("checkbox",{name:LABEL})).toBeDisabled();
    expect(calls.mock.calls.filter(([url,init])=>String(url).endsWith("/jobs")&&init?.method==="POST")).toHaveLength(1);
    expect(calls.mock.calls.filter(([url,init])=>String(url).endsWith("/estimates")&&init?.method==="POST")).toHaveLength(1);
    failed=false;fireEvent.click(screen.getByRole("button",{name:"重新核对结果"}));
    expect(await screen.findByTestId("job-accepted")).toHaveTextContent("recovered-job");
    expect(calls.mock.calls.filter(([url,init])=>String(url).endsWith("/jobs")&&init?.method==="POST")).toHaveLength(1);
  });
  it("summary failure is unknown and never creates a quote by guessing readiness", async () => {
    const calls=setup((url)=>url.includes("/items/summaries?")?errorResponse(500,"INTERNAL_ERROR","summary unavailable"):undefined);
    await waitFor(()=>expect(screen.getByTestId("submission-recovery")).toHaveTextContent("summary unavailable"));
    expect(screen.getAllByText("状态未读取")).toHaveLength(5);
    expect(calls.mock.calls.filter(([,init])=>init?.method==="POST")).toHaveLength(0);
    expect(generate()).toBeDisabled();
  });
});

it("PC03B 412 keeps entered authorization while GET checks saved facts; no automatic second submit",async()=>{
  const calls=setup((url)=>url.endsWith("/jobs")?errorResponse(412,"PRECONDITION_FAILED","fixture revision changed"):undefined);
  await ready();await confirm();fireEvent.change(screen.getByLabelText("Tripo credits"),{target:{value:"42"}});fireEvent.click(generate());
  await screen.findByTestId("submit-error");await waitFor(()=>expect(screen.getByLabelText("Tripo credits")).toBeEnabled());expect(screen.getByLabelText("Tripo credits")).toHaveValue("42.00");
  expect(calls.mock.calls.filter(([url,init])=>String(url).endsWith("/jobs")&&init?.method==="POST")).toHaveLength(1);
  expect(calls.mock.calls.filter(([url,init])=>String(url).endsWith("/estimates")&&init?.method==="POST")).toHaveLength(1);
});
