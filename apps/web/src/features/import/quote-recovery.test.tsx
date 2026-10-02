import { StrictMode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { jsonResponse } from "../../test/render";
import { useQuoteRecovery } from "./quote-recovery";
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
