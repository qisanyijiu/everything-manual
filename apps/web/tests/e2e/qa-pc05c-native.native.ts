/** Root-authorized FF157 product equivalent for native visibility only. */
import http from "node:http";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { expect, request, test } from "@playwright/test";
import { apiLogin } from "./helpers";
import { evidence, PASSWORD, Pc05cBackend, WEB } from "./qa-pc05c-backend";
import { jobFixture } from "./qa-pc05c-fixture";
import { detail } from "./qa-pc05c-jobs";
import { REPO_ROOT } from "./runtime";

test("C7 official Firefox157 real product window hidden and restored with trusted events and elapsed countdown", async () => {
  const b = new Pc05cBackend(); const api = await request.newContext();
  const bridge = http.createServer((req, res) => {
    const upstream = http.request(b.base + (req.url ?? "/"), { method: req.method, headers: req.headers }, response => { res.writeHead(response.statusCode ?? 502, response.headers); response.pipe(res); });
    upstream.on("error", () => { if (!res.headersSent) res.writeHead(502); res.end(); }); req.pipe(upstream);
  });
  try {
    await b.start(); await apiLogin(api, b.base, PASSWORD); b.fixture.taskReplies.push({ kind: "retry", after: 90 });
    const f = await jobFixture(api, b); await expect.poll(async () => (await detail(api, b, f.jobId)).stages.find(s => s.stageKind === "tripo_poll")?.status, { timeout: 60000 }).toBe("retry_wait");
    const stage = (await detail(api, b, f.jobId)).stages.find(s => s.stageKind === "tripo_poll")!; expect(stage.safeRetry).toEqual({ number: 1, limit: 5 }); expect(Date.parse(stage.nextRunAt!) - Date.now()).toBeGreaterThan(60000);
    await new Promise<void>((resolve, reject) => { bridge.once("error", reject); bridge.listen(18491, "127.0.0.1", resolve); });
    const module = await import(pathToFileURL(path.join(REPO_ROOT, "var/pc05c-qa/native-visibility-product.mjs")).href);
    const output = path.join(process.env.EM_PC05C_QA_OUTPUT_DIR ?? path.join(REPO_ROOT, "var/pc05c-qa"), "firefox-native-product");
    const report = await module.verifyNativeProductVisibility({ url: `${WEB}/jobs/${f.jobId}`, password: PASSWORD, output, readFacts: async () => b.facts() });
    evidence("c7-product-native-visibility", { jobId: f.jobId, stageId: stage.id, safeRetry: stage.safeRetry, nextRunAt: stage.nextRunAt, browser: report.browserVersion, pass: report.pass, output, sameProductionFreeze: true, scope: "native visibility product supplement only" });
  } finally {
    bridge.closeAllConnections(); if (bridge.listening) await new Promise<void>(resolve => bridge.close(() => resolve())); await api.dispose(); await b.cleanup();
  }
});
