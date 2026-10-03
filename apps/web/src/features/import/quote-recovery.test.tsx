import { StrictMode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { jsonResponse } from "../../test/render";
import { completedJobRequoteProblem, useQuoteRecovery } from "./quote-recovery";
import type { JobDetailDto } from "../../api/endpoints";
afterEach(()=>{localStorage.clear();vi.unstubAllGlobals();});
it("StrictMode effect replay preserves an in-flight persisted-operation GET result",async()=>{
  localStorage.setItem("manual:submission:item",JSON.stringify({quoteId:"saved",key:"original-key",body:{quoteId:"saved",preparationId:"prep",photoIds:["front","left"],limits:{tripoCreditMinor:3000,manualAiUsdMicros:1000}}}));
  let resolve!:(value:Response)=>void;const pending=new Promise<Response>(done=>{resolve=done;});
  const fetch=vi.fn((input:RequestInfo|URL)=>String(input).endsWith("/estimates/saved")?pending:Promise.resolve(jsonResponse(String(input).endsWith("/estimates/another")?{data:{id:"another",consumedJobId:"another-job"}}:{data:[{latestQuoteId:"saved"}]})));
  vi.stubGlobal("fetch",fetch);
  function Probe({requested = null}:{requested?:string|null}){const r=useQuoteRecovery("item",requested);return <p>{r.phase} {r.quote?.consumedJobId}</p>;}
  const client=new QueryClient({defaultOptions:{queries:{retry:false}}});const mounted=render(<StrictMode><QueryClientProvider client={client}><Probe/></QueryClientProvider></StrictMode>);
  await screen.findByText("checking");
  await act(async()=>resolve(jsonResponse({data:{id:"saved",consumedJobId:"unique-job"}})));
  await waitFor(()=>expect(screen.getByText("verified unique-job")).toBeInTheDocument());
  expect(localStorage.getItem("manual:submission:item")).toBeNull();
  mounted.rerender(<StrictMode><QueryClientProvider client={client}><Probe requested="another"/></QueryClientProvider></StrictMode>);
  await screen.findByText("verified another-job");
  expect(fetch.mock.calls.filter(([url])=>String(url).endsWith("/estimates/saved"))).toHaveLength(1);
});

function finalJob(): JobDetailDto {
  return { id: "job", item: { id: "item", name: "Synthetic", model: "test" }, snapshotId: "snapshot", status: "cancelled", revision: 2,
    stages: [{ id: "stage", stageKind: "manual_extract", status: "failed", batchIndex: 0, attemptCount: 1, pollCount: 0, needsInput: [], retry: { allowed: false }, updatedAt: "2026-10-03T00:00:00Z" }],
    attempts: [{ id: "attempt", stageId: "stage", submitState: "failed", startedAt: "2026-10-03T00:00:00Z" }],
    reservations: [{ provider: "manual_ai", currency: "usdMicros", state: "reserved", reservedMinor: 1000, reservedDisplay: "0.001 USD" }],
    budgetNotice: "Synthetic", createdAt: "2026-10-03T00:00:00Z", updatedAt: "2026-10-03T00:00:00Z" };
}
it("已取消且明确失败的付费请求允许新报价，reserved授权保留不冒充未知结果", () => {
  expect(completedJobRequoteProblem(finalJob(), "job", "item")).toBeNull();
  const sync = finalJob(); sync.attempts[0]!.submitState = "accepted";
  expect(completedJobRequoteProblem(sync, "job", "item")).toBeNull();
});
it.each(["intent", "submitting", "unknown", "unexpected"])("未决/未知 attempt %s 阻断重新报价", (state) => {
  const job = finalJob(); job.attempts[0]!.submitState = state;
  expect(completedJobRequoteProblem(job, "job", "item")).not.toBeNull();
});
it.each(["queued", "running", "retry_wait", "waiting_provider", "submission_unknown", "needs_input"])("终态父任务内仍有 %s 阶段时不放行", (state) => {
  const job = finalJob(); job.stages[0]!.status = state;
  expect(completedJobRequoteProblem(job, "job", "item")).not.toBeNull();
});
it("未知账务、缺失事实或错误任务归属均阻断", () => {
  const job = finalJob();
  expect(completedJobRequoteProblem(job, "other-job", "item")).not.toBeNull();
  expect(completedJobRequoteProblem(job, "job", "other-item")).not.toBeNull();
  expect(completedJobRequoteProblem({ ...job, stages: [] }, "job", "item")).not.toBeNull();
  expect(completedJobRequoteProblem({ ...job, reservations: [] }, "job", "item")).not.toBeNull();
  job.reservations[0]!.state = "unknown";
  expect(completedJobRequoteProblem(job, "job", "item")).not.toBeNull();
});
it("Tripo提交成功和本地poll失败/取消均不证明远端结束；必须匹配终态task事实", () => {
  const job = finalJob();
  job.stages[0]!.stageKind = "tripo_submit"; job.stages[0]!.status = "succeeded";
  job.stages.push({ ...job.stages[0]!, id: "poll", stageKind: "tripo_poll", status: "cancelled" });
  job.attempts[0]!.submitState = "accepted"; job.attempts[0]!.remoteTaskId = "remote-task";
  expect(completedJobRequoteProblem(job, "job", "item")).not.toBeNull();
  const poll = job.stages[1]!; poll.status = "failed";
  for (const status of ["running", "queued", "unrecognized"]) {
    poll.usage = { remoteTaskId: "remote-task", normalizedStatus: status };
    expect(completedJobRequoteProblem(job, "job", "item")).not.toBeNull();
  }
  poll.usage = { remoteTaskId: "wrong-task", normalizedStatus: "success" };
  expect(completedJobRequoteProblem(job, "job", "item")).not.toBeNull();
  for (const status of ["success", "failed", "cancelled", "banned", "expired"]) {
    poll.usage = { remoteTaskId: "remote-task", normalizedStatus: status };
    expect(completedJobRequoteProblem(job, "job", "item")).toBeNull();
  }
});

it.each([
  {photoIds:["front",{}],limits:{tripoCreditMinor:3000,manualAiUsdMicros:1000}},
  {photoIds:["front","left"],limits:{tripoCreditMinor:"3000",manualAiUsdMicros:1000}},
  {photoIds:["front","left"],limits:{tripoCreditMinor:1e100,manualAiUsdMicros:-1}},
  {photoIds:["front","left"],limits:[]},
])("corrupt/old hints are ignored and bounded server discovery restores the saved quote: %j",async(body)=>{
  localStorage.setItem("manual:submission:item",JSON.stringify({quoteId:"corrupt",key:"hint-key",body:{quoteId:"corrupt",preparationId:"prep",...body}}));
  const fetch=vi.fn((input:RequestInfo|URL)=>Promise.resolve(jsonResponse(String(input).includes("/items/summaries?")?{data:[{latestQuoteId:"server-quote"}]}:{data:{id:"server-quote",consumedJobId:"server-job"}})));
  vi.stubGlobal("fetch",fetch);
  function Probe(){const r=useQuoteRecovery("item",null);return <p>{r.phase} {r.quote?.consumedJobId} {r.operation ? "pending" : "no hint"}</p>;}
  const client=new QueryClient({defaultOptions:{queries:{retry:false}}});render(<QueryClientProvider client={client}><Probe/></QueryClientProvider>);
  await screen.findByText("verified server-job no hint");
  expect(fetch.mock.calls.some(([url])=>String(url).endsWith("/estimates/corrupt"))).toBe(false);
  expect(fetch.mock.calls.some(([url])=>String(url).endsWith("/estimates/server-quote"))).toBe(true);
});
