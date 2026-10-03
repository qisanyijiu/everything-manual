import { randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { expect, request, test, type APIRequestContext, type Locator, type Page } from "@playwright/test";
import { apiLogin, loginViaUi } from "./helpers";
import { waitForJob, fetchJobDetail } from "./job-recovery-harness";
import { Pc06QaBackend, WEB, PASSWORD, OUT, GOOD, names, canary, evidence, settings, body, put, attach, quote } from "./qa-pc06-backend";

test.describe.configure({ mode: "serial", timeout: 150000 });
const hidden = "模型需修正（疑似误填密钥，已隐藏）";
const message = "这里需要模型名称；API 密钥请在密钥操作中选择替换后输入";
async function tabTo(page: Page, locator: Locator) {
  for (let i = 0; i < 120; i++) { await page.keyboard.press("Tab"); if (await locator.evaluate(element => element === document.activeElement)) return; }
  throw new Error("QA target not reachable through keyboard Tab");
}
async function browserSafe(page: Page, b: Pc06QaBackend) {
  const exposed = await page.evaluate(markers => markers.some(value => [document.documentElement.outerHTML, location.href, JSON.stringify(localStorage), JSON.stringify(sessionStorage)].some(text => text.includes(value))), b.markers);
  expect(exposed, "DOM attributes/text/URL/storage leak boolean").toBe(false);
}
async function boot(b: Pc06QaBackend, api: APIRequestContext) { await b.start(); return apiLogin(api, b.base, PASSWORD); }
function byteSnapshot(b: Pc06QaBackend) { return fs.existsSync(b.overlay) ? fs.readFileSync(b.overlay).toString("base64") : null; }
function counts(b: Pc06QaBackend) { return b.db("SELECT (SELECT count(*) FROM jobs) AS jobs,(SELECT count(*) FROM provider_attempts) AS attempts,(SELECT count(*) FROM cost_ledger) AS ledger"); }
async function confirmed(api: APIRequestContext, b: Pc06QaBackend) {
  const q = await quote(api, b); const r = await api.post(`${b.base}/api/v1/items/${q.itemId}/estimates/${q.quoteId}/confirm`, { headers: { "x-csrf-token": q.csrf } }); expect(r.status()).toBe(200); return q;
}
async function createJob(api: APIRequestContext, b: Pc06QaBackend, q: Awaited<ReturnType<typeof quote>>) {
  const r = await api.post(`${b.base}/api/v1/items/${q.itemId}/jobs`, { headers: { "x-csrf-token": q.csrf, "idempotency-key": randomUUID() }, data: q.jobBody }); expect(r.status()).toBe(202); return (await r.json()).data.id as string;
}

test("QA PC6-001/002 finite classification, direct PUT atomicity, auth and browser parity", async ({ page, request: api }) => {
  const b = new Pc06QaBackend(); const marker = canary(); b.markers.push(marker);
  const positive = [marker, "sk-0123456789abcdef", `  ${marker}  `, `bEaReR   ${marker}`, "sk-proj-qa_fake_0123456789", `\uFEFF${marker}\uFEFF`, `\u0085${marker}\u0085`];
  const negative = ["sk-0123456789abcde", "sk-local", "org/custom-model", "local:model-v2", "SK-0123456789abcdef", "text sk-0123456789abcdef", "sk-0123456789abcdef.", "sk-0123456789abcde中文"];
  const observations: unknown[] = [];
  try {
    const csrf = await boot(b, api); const state = await attach(page, b); await loginViaUi(page, WEB, PASSWORD);
    for (const name of names) {
      await page.goto(WEB + "/settings"); const model = page.getByLabel(name === "tripo" ? "Tripo 模型" : "说明书 AI 模型", { exact: true });
      for (const [index, candidate] of positive.entries()) {
        const view = await settings(api, b); const before = byteSnapshot(b);
        const data = { ...body(view), [name]: { ...body(view)[name], model: candidate, keyAction: "replace", apiKey: "qa-fake-rejected-key" } };
        const r = await put(api, b, csrf, data); expect(r.status()).toBe(422); expect((await r.json()).error.details.fields.some((f: { message: string }) => f.message === message)).toBe(true);
        const fullToken = candidate.match(/sk-[A-Za-z0-9_-]+/u)?.[0]; if (fullToken) expect((await r.text()).includes(fullToken), "model token reflection boolean").toBe(false);
        expect((await settings(api, b)).revision === view.revision).toBe(true); expect(byteSnapshot(b) === before).toBe(true);
        await model.fill(candidate); await expect(model).toHaveAttribute("type", "password"); await expect(model).toHaveAttribute("aria-invalid", "true"); await browserSafe(page, b);
        observations.push({ provider: name, case: index, serverRefused: true, clientClassified: true, bytesUnchanged: true });
      }
      for (const candidate of negative) {
        await model.fill(candidate); await expect(model).toHaveAttribute("type", "text"); await expect(model).toHaveAttribute("aria-invalid", "false");
        const view = await settings(api, b); const r = await put(api, b, csrf, { ...body(view), [name]: { ...body(view)[name], model: candidate } }); expect(r.status()).toBe(200);
      }
      await model.fill("Bearer\tsk-0123456789abcdef"); await expect(model).toHaveAttribute("type", "text");
      const view = await settings(api, b); const r = await put(api, b, csrf, { ...body(view), [name]: { ...body(view)[name], model: "Bearer\tsk-0123456789abcdef" } }); expect(r.status()).toBe(422);
      expect((await r.json()).error.details.fields.every((f: { message: string }) => f.message !== message)).toBe(true);
    }
    const view = await settings(api, b); const data = body(view); const before = byteSnapshot(b);
    const anonymous = await request.newContext(); try { expect((await anonymous.put(b.base + "/api/v1/settings/providers", { data })).status()).toBe(401); } finally { await anonymous.dispose(); }
    expect((await put(api, b, "wrong-csrf", data)).status()).toBe(403);
    expect((await put(api, b, csrf, { ...data, revision: randomUUID() })).status()).toBe(409);
    expect(byteSnapshot(b) === before).toBe(true); expect(b.counts()).toEqual({ upload: 0, submit: 0, task: 0, manual: 0, cdn: 0 });
    expect(state.external + state.pageErrors).toBe(0); expect(state.consoleLeak).toBe(false);
    evidence("classification", { positive: observations, negativePerProvider: negative.length, controlRejectedWithoutSecretClassification: true, authorization: [401,403,409], providerRequests: 0, browserWrites: state.writes });
  } finally { await b.cleanup(); }
});

test("QA PC6-003/004/006/009 legacy key modes, read-only bytes, independent key correction and restart", async ({ request: api }) => {
  const results: unknown[] = [];
  for (const mode of ["inherit", "clear", "replace"] as const) {
    const b = new Pc06QaBackend(); const marker = canary(); b.markers.push(marker);
    try {
      let csrf = await boot(b, api); let view = await settings(api, b);
      const replacement = "qa-model-correction-new-key";
      const write = body(view);
      for (const name of names) write[name] = { ...write[name], model: "org/custom-before-legacy", keyAction: mode === "inherit" ? "keep" : mode };
      const data = mode === "replace" ? { ...write, tripo: { ...write.tripo, apiKey: replacement }, manualAi: { ...write.manualAi, apiKey: replacement } } : write;
      expect((await put(api, b, csrf, data)).status()).toBe(200); await b.stop();
      const overlay = JSON.parse(fs.readFileSync(b.overlay, "utf8")); for (const name of names) overlay[name].model = marker;
      fs.writeFileSync(b.overlay, JSON.stringify(overlay), { mode: 0o600 }); const before = byteSnapshot(b); await b.start();
      csrf = await apiLogin(api, b.base, PASSWORD); view = await settings(api, b);
      for (const phase of ["active", "saved"]) for (const name of names) {
        expect(view[phase][name].model).toBeNull(); expect(view[phase][name].modelIssue).toBe("suspectedCredential"); expect(view[phase][name].keyConfigured).toBe(mode !== "clear"); expect(view[phase][name].modelSource).toBe("web");
      }
      expect(view.pending).toBe(false); const status = await api.get(b.base + "/api/v1/settings/status"); const statusData = (await status.json()).data; b.noLeak(statusData); expect(statusData.capabilities.generation).toBe(false);
      const check = b.command(["check", "--data-dir", b.dataDir]); expect(check.code).toBe(0); b.noLeak(check.output); expect(check.output.includes("疑似误填密钥")).toBe(true); expect(byteSnapshot(b) === before).toBe(true);
      for (const model of [null, "", " "]) {
        const r = await put(api, b, csrf, { ...body(view), tripo: { ...body(view).tripo, model }, manualAi: { ...body(view).manualAi, model: GOOD.manualAi } }); expect(r.status()).toBe(422); expect(byteSnapshot(b) === before).toBe(true);
      }
      const clear = { ...body(view), tripo: { ...body(view).tripo, model: "", clearModel: true }, manualAi: { ...body(view).manualAi, model: GOOD.manualAi } };
      const corrected = await put(api, b, csrf, clear); expect(corrected.status()).toBe(200); view = (await corrected.json()).data;
      expect(view.active.tripo.modelIssue).toBe("suspectedCredential"); expect(view.saved.tripo.modelIssue).toBeNull(); expect(view.saved.tripo.model).toBeNull(); expect(view.pending).toBe(true);
      const disk = JSON.parse(fs.readFileSync(b.overlay, "utf8")); for (const name of names) expect(JSON.stringify(disk[name].key) === JSON.stringify(overlay[name].key)).toBe(mode !== "replace");
      // Ciphertexts use random nonces when resealed; mode and effective key facts are the invariant.
      for (const name of names) expect(disk[name].key.mode).toBe(mode); b.noLeak(JSON.stringify(disk));
      await b.restart(); csrf = await apiLogin(api, b.base, PASSWORD); view = await settings(api, b); expect(view.pending).toBe(false); expect(view.active.tripo.model).toBeNull(); expect(view.active.manualAi.model).toBe(GOOD.manualAi); expect(view.active.manualAi.keyConfigured).toBe(mode !== "clear");
      const restore = await put(api, b, csrf, { revision: view.revision, tripo: { action: "restore" }, manualAi: { action: "restore" } }); expect(restore.status()).toBe(200);
      await b.restart(); await apiLogin(api, b.base, PASSWORD); view = await settings(api, b); expect(view.active.tripo.modelSource).toBe("deployment"); expect(view.active.tripo.keyConfigured).toBe(true);
      expect(b.counts()).toEqual({ upload: 0, submit: 0, task: 0, manual: 0, cdn: 0 }); results.push({ mode, initialReadBytesUnchanged: true, implicitClearRefused: true, explicitClear: true, keyModePreserved: true, restartAndRestore: true, requests: 0 });
    } finally { await b.cleanup(); }
  }
  evidence("legacy-modes", results);
});

test("QA PC6-004/005/010 frozen UI explicit correction, restore errors, keyboard and 375/1440", async ({ page, request: api }) => {
  const b = new Pc06QaBackend(); const marker = canary(); b.markers.push(marker); b.models.manualAi = marker; b.source = "toml";
  try {
    await boot(b, api); const configBefore=fs.readFileSync(path.join(b.workDir,"config.toml")); const check=b.command(["check","--data-dir",b.dataDir]); expect(check.code).toBe(0); b.noLeak(check.output); expect(check.output.includes("疑似误填密钥")).toBe(true); expect(fs.readFileSync(path.join(b.workDir,"config.toml")).equals(configBefore)).toBe(true);
    const state = await attach(page, b); await loginViaUi(page, WEB, PASSWORD); await page.goto(WEB + "/settings");
    const manual = page.getByLabel("说明书 AI 模型", { exact: true }); const tripo = page.getByLabel("Tripo 模型", { exact: true }); const save = page.getByRole("button", { name: "保存配置", exact: true });
    await expect(manual).toHaveValue(""); await expect(save).toBeDisabled(); await expect(page.getByText(hidden, { exact: true })).toBeVisible();
    const geometry: unknown[] = [];
    for (const width of [375,1440]) {
      await page.setViewportSize({ width, height: 950 }); await tabTo(page, page.getByRole("button", { name: "清空误填模型", exact: true })); await page.keyboard.press("Enter");
      const cancel = page.getByRole("button", { name: "取消", exact: true }); const confirm = page.getByRole("button", { name: "确认清空模型", exact: true }); await expect(cancel).toBeFocused();
      for (const button of [cancel, confirm]) { const box = await button.boundingBox(); expect(box?.height).toBeGreaterThanOrEqual(44); expect(box?.width).toBeGreaterThanOrEqual(44); }
      const measure = await page.evaluate(() => ({ width: innerWidth, scroll: document.documentElement.scrollWidth })); expect(measure.scroll).toBeLessThanOrEqual(width); geometry.push(measure);
      await browserSafe(page,b); fs.mkdirSync(OUT,{recursive:true}); await page.screenshot({ path: path.join(OUT, `clear-${width}.png`), fullPage:true });
      await page.keyboard.press("Tab"); await expect(confirm).toBeFocused(); await page.keyboard.press("Space"); await expect(manual).toBeFocused();
      await tabTo(page,page.getByRole("button",{name:"撤销清空",exact:true})); await page.keyboard.press("Enter"); await expect(manual).toBeFocused();
    }
    await tripo.fill("org/custom-tripo"); await save.click(); await expect(manual).toBeFocused(); expect(state.writes).toBe(0);
    await expect(manual).toHaveAttribute("aria-invalid","true"); const associations=await manual.getAttribute("aria-describedby"); expect(associations?.includes("error")).toBe(true);
    const manualCard=page.locator("section.api-provider-card").filter({has:page.getByRole("heading",{name:"说明书 AI",exact:true})});
    const tripoCard=page.locator("section.api-provider-card").filter({has:page.getByRole("heading",{name:"Tripo · 模型生成",exact:true})});
    const ownReplacement="qa-pc06-browser-replacement"; const otherReplacement="qa-pc06-other-card-replacement";
    b.markers.push(ownReplacement,otherReplacement);
    const beforeRestore=await settings(api,b); const bytesBeforeRestore=byteSnapshot(b);
    await tripoCard.getByLabel("替换",{exact:true}).check(); const otherKey=page.getByLabel("新的 Tripo 密钥",{exact:true}); await otherKey.fill(otherReplacement);
    await manualCard.getByLabel("替换",{exact:true}).check(); const key=page.getByLabel("新的说明书 AI密钥",{exact:true}); await key.fill(ownReplacement);
    await manual.fill("org/unsaved-before-restore");
    await page.getByLabel("说明书 AI Base URL",{exact:true}).fill(beforeRestore.saved.manualAi.baseUrl+"/qa-unsaved");
    // AS §6.3: entering restore clears this card's unsaved key; cancelling uses saved values + keep.
    // PC06 §10.5: a refused restore must preserve the other card's edits and replacement key.
    await manualCard.getByRole("button",{name:"恢复部署配置",exact:true}).click();
    await expect(key).toHaveCount(0); await expect(otherKey).toHaveValue(otherReplacement); expect(state.writes).toBe(0);
    await save.click();
    await expect(manualCard.getByText("部署模型疑似误填密钥，不能恢复。请取消恢复并填写正确模型，或修正部署配置。",{exact:true})).toBeVisible();
    expect(await page.evaluate(()=>document.activeElement?.classList.contains("api-restore-note"))).toBe(true);
    await expect(tripo).toHaveValue("org/custom-tripo"); await expect(otherKey).toHaveValue(otherReplacement);
    expect(state.writes).toBe(1); expect((await settings(api,b)).revision).toBe(beforeRestore.revision); expect(byteSnapshot(b)).toBe(bytesBeforeRestore); await browserSafe(page,b);
    await manualCard.getByRole("button",{name:"取消恢复",exact:true}).click();
    await expect(manualCard.getByLabel("保留现有",{exact:true})).toBeChecked(); await expect(key).toHaveCount(0);
    await expect(manual).toHaveValue(""); await expect(page.getByLabel("说明书 AI Base URL",{exact:true})).toHaveValue(beforeRestore.saved.manualAi.baseUrl);
    await expect(tripo).toHaveValue("org/custom-tripo"); await expect(otherKey).toHaveValue(otherReplacement);
    await manualCard.getByLabel("替换",{exact:true}).check(); await expect(key).toHaveValue(""); await key.fill(ownReplacement);
    await manual.fill("org/custom-model"); await expect(manual).toHaveAttribute("type","text"); await save.click(); await expect(page.getByText("已保存，重启服务后生效",{exact:true}).first()).toBeVisible();
    await expect(page.getByText(hidden,{exact:true})).toBeVisible(); expect(state.writes).toBe(2);
    const view=await settings(api,b); expect(view.active.manualAi.modelIssue).toBe("suspectedCredential"); expect(view.saved.manualAi.modelIssue).toBeNull(); expect(view.saved.manualAi.keySource).toBe("web");
    await b.restart(); await page.reload(); await expect(manual).toHaveValue("org/custom-model"); await expect(page.getByText(hidden,{exact:true})).toHaveCount(0); await browserSafe(page,b);
    expect(state.external+state.pageErrors).toBe(0); expect(state.consoleLeak).toBe(false); expect(b.counts()).toEqual({upload:0,submit:0,task:0,manual:0,cdn:0});
    evidence("ui-correction",{geometry,initialPristine:true,implicitClearClientRejected:true,restoreFailureVisibleFocus:true,restoreOwnKeyCleared:true,cancelRestoresSavedAndKeep:true,otherCardEditsAndKeyRetained:true,explicitReplacementAfterCancel:true,restoreFailureBytesUnchanged:true,pendingAndRestart:true,wireWrites:state.writes,providerRequests:0,leak:false,external:state.external,pageErrors:state.pageErrors});
  } finally { await b.cleanup(); }
});

test("QA PC6-007 price preset interpolation refuses a mistaken model before persistence",async({request:api})=>{
  const b=new Pc06QaBackend(); const marker=canary(); b.markers.push(marker);
  try {
    await boot(b,api); const q=await quote(api,b); const before=counts(b); const providerBefore=b.counts(); await b.stop();
    const catalog=path.join(b.workDir,"price-catalog.toml"); const original=fs.readFileSync(catalog,"utf8");
    const changed=original.replace(`model = "${GOOD.tripo}"`,`model = "${marker}"`); expect(changed!==original).toBe(true); fs.writeFileSync(catalog,changed,{mode:0o600}); await b.start(); const csrf=await apiLogin(api,b.base,PASSWORD);
    const r=await api.post(`${b.base}/api/v1/items/${q.itemId}/estimates`,{headers:{"x-csrf-token":csrf},data:{preparationId:q.preparationId,photoIds:q.photoIds,modelPreset:"tripo-h-v3.1-standard"}});
    expect(r.status()).toBe(422); b.noLeak(await r.text()); expect((await r.json()).error.details.reason).toBe("providerModelInvalid"); expect(counts(b)).toEqual(before); expect(b.counts()).toEqual(providerBefore);
    evidence("price-interpolation",{normalActiveModel:true,catalogModelClassified:true,responseStatus:422,reason:"providerModelInvalid",providerRequests:0,newJobAttemptLedgerDelta:0,leak:false});
  } finally {await b.cleanup();}
});

test("QA PC6-007/008 historical quote and confirmation copies hide all model sources without mutation", async ({ page, request: api }) => {
  const b=new Pc06QaBackend(); const marker=canary(); b.markers.push(marker);
  try {
    await boot(b,api); const q=await confirmed(api,b); const endpoint=`${b.base}/api/v1/items/${q.itemId}/estimates/${q.quoteId}`;
    const baseline=b.db("SELECT quote_json,provider_config,confirmation_json,confirmed_at,consumed_at FROM quotes WHERE id=?",[q.quoteId]) as Array<Record<string,string|null>>;
    const original=baseline[0]; if(!original) throw new Error("QA quote absent");
    const before=counts(b); const providerBefore=b.counts();
    const cases: Array<{column:string;pointer:string[]}>=[
      ...[ ["providerConfig","tripo","model"],["providerConfig","manualAi","model"],["sendScope","tripo","model"],["sendScope","tripo","parameters","model"],["sendScope","manualAi","model"] ].map(pointer=>({column:"quote_json",pointer})),
      {column:"provider_config",pointer:["manualAi","model"]},
      {column:"confirmation_json",pointer:["sendScope","manualAi","model"]},
    ];
    const rows:unknown[]=[]; let publicPayload:unknown;
    for(const scenario of cases) {
      b.db("UPDATE quotes SET quote_json=?,provider_config=?,confirmation_json=? WHERE id=?",[original.quote_json,original.provider_config,original.confirmation_json,q.quoteId],true);
      const mutated=JSON.parse(original[scenario.column] ?? "{}"); let position=mutated;
      for(const key of scenario.pointer.slice(0,-1)) position=position[key]; position[scenario.pointer.at(-1)!]=marker;
      b.db(`UPDATE quotes SET ${scenario.column}=? WHERE id=?`,[JSON.stringify(mutated),q.quoteId],true);
      const rawBefore=JSON.stringify(b.db("SELECT quote_json,provider_config,confirmation_json,confirmed_at,consumed_at FROM quotes WHERE id=?",[q.quoteId]));
      const read=await api.get(endpoint); expect(read.status()).toBe(200); publicPayload=await read.json(); b.noLeak(publicPayload);
      const current=(publicPayload as {data:Record<string,unknown>}).data; expect(current.modelIssue).toBe("suspectedCredential"); expect(current.id).toBe(q.quoteId); expect(current.amounts).toEqual(q.payload.amounts);
      expect(current.confirmedAt).not.toBeNull(); expect(current.consumedAt).toBeNull();
      const confirm=await api.post(endpoint+"/confirm",{headers:{"x-csrf-token":q.csrf}}); expect(confirm.status()).toBe(422); b.noLeak(await confirm.text()); expect((await confirm.json()).error.details.reason).toBe("quoteModelInvalid");
      const create=await api.post(`${b.base}/api/v1/items/${q.itemId}/jobs`,{headers:{"x-csrf-token":q.csrf,"idempotency-key":randomUUID()},data:q.jobBody}); expect(create.status()).toBe(422); b.noLeak(await create.text());
      expect(JSON.stringify(b.db("SELECT quote_json,provider_config,confirmation_json,confirmed_at,consumed_at FROM quotes WHERE id=?",[q.quoteId]))===rawBefore).toBe(true); expect(counts(b)).toEqual(before); expect(b.counts()).toEqual(providerBefore);
      rows.push({source:scenario.column,pointer:scenario.pointer.join("/"),publicMasked:true,bytesUnchanged:true,confirm:422,newJob:422});
    }
    // Present the real safe GET DTO to the confirmation component; this transport adapter
    // is UI-only evidence, separate from the unmodified HTTP GET/confirm/job checks above.
    const state=await attach(page,b); await loginViaUi(page,WEB,PASSWORD);
    await page.evaluate(({item,prep})=>sessionStorage.setItem(`em.prepare.${item}`,prep),{item:q.itemId,prep:q.preparationId});
    await page.route(`**/api/v1/items/${q.itemId}/estimates`,async route=>{ if(route.request().method()==="POST") await route.fulfill({status:201,contentType:"application/json",body:JSON.stringify(publicPayload)}); else await route.continue(); });
    await page.goto(WEB+`/items/${q.itemId}/import/confirm`);
    await expect(page.getByText(/此报价的模型信息不可用/).first()).toBeVisible(); await expect(page.getByRole("link",{name:"前往设置",exact:true}).first()).toBeVisible(); await browserSafe(page,b);
    expect(state.external+state.pageErrors).toBe(0); expect(state.consoleLeak).toBe(false);
    evidence("historical-quotes",{cases:rows,quoteId:q.quoteId,countersBefore:before,countersAfter:counts(b),providerBefore,providerAfter:b.counts(),uiSafeDtoAdapter:true,uiUnavailable:true,leak:false});
  } finally {await b.cleanup();}
});

test("QA PC6-007 real HTTP unknown reconciliation survives bad model and never repurchases",async({request:api})=>{
  const b=new Pc06QaBackend(); const marker=canary(); b.markers.push(marker);
  try {
    await boot(b,api); b.fixture.upstream.state.submitMode="http500"; const q=await confirmed(api,b); const jobId=await createJob(api,b,q);
    await waitForJob(api,b.base,jobId,v=>v.stages.some(s=>s.stageKind==="tripo_submit"&&s.status==="submission_unknown")&&v.stages.some(s=>s.stageKind==="manual_merge"&&s.status==="succeeded"),"independent unknown and completed manual branch");
    const before=b.counts(); expect(before.submit).toBe(1); await b.stop(); b.models.tripo=marker; await b.start(); const csrf=await apiLogin(api,b.base,PASSWORD);
    let job=await fetchJobDetail(api,b.base,jobId); const stage=job.stages.find(s=>s.stageKind==="tripo_submit"); if(!stage) throw new Error("QA submit stage absent");
    const replacement=await api.post(`${b.base}/api/v1/jobs/${jobId}/reconcile`,{headers:{"x-csrf-token":csrf,"if-match":job.etag??""},data:{action:"authorizeReplacement",stageId:stage.id,acknowledgeDuplicateRisk:true,limits:{tripoCreditMinor:3000,manualAiUsdMicros:100000}}});
    expect(replacement.status()).toBe(422); expect((await replacement.json()).error.details.reason).toBe("providerModelInvalid"); b.noLeak(await replacement.text());
    const settingsBefore=await settings(api,b); const blocked=await put(api,b,csrf,{...body(settingsBefore),tripo:{...body(settingsBefore).tripo,model:GOOD.tripo}}); expect(blocked.status()).toBe(422); expect((await blocked.json()).error.details.reason).toBe("providerConfigBusy");
    job=await fetchJobDetail(api,b.base,jobId); const attached=await api.post(`${b.base}/api/v1/jobs/${jobId}/reconcile`,{headers:{"x-csrf-token":csrf,"if-match":job.etag??""},data:{action:"attachRemoteTask",stageId:stage.id,remoteTaskId:"t17-fixture-task-0001",acknowledgeMatches:true}});
    expect(attached.status()).toBe(200); b.noLeak(await attached.text());
    await waitForJob(api,b.base,jobId,v=>v.stages.some(s=>s.stageKind==="tripo_poll"&&s.status==="succeeded"),"remote receipt resumes readonly query");
    const after=b.counts(); expect(after.submit).toBe(before.submit); expect(after.task-before.task).toBeGreaterThanOrEqual(2); expect(after.manual).toBe(before.manual); expect(after.upload).toBe(before.upload);
    evidence("remote-reconciliation",{jobId,before,after,replacementRefused:422,correctionBusyRefused:422,attachRemoteTask:200,receiptResumed:true,paidSubmissionDelta:0,leak:false});
  } finally {await b.cleanup();}
});

test("QA PC6-006/007 frozen worker gate, and existing release/PDF read with bad current model",async({page,request:api})=>{
  const b=new Pc06QaBackend(); const marker=canary(); b.markers.push(marker);
  try {
    await boot(b,api); const q=await confirmed(api,b); const jobId=await createJob(api,b,q); const done=await waitForJob(api,b.base,jobId,v=>v.status==="succeeded","independent fixture ready");
    if(!done.draftId) throw new Error("QA draft missing"); const draftUrl=`${b.base}/api/v1/items/${q.itemId}/drafts/${done.draftId}`;
    const draftRead=await api.get(draftUrl); const draft=(await draftRead.json()).data; const knowledge=draft.knowledge.knowledge as Record<string,Array<{id:string}>>; const parts=knowledge.parts??[];
    const entities=Object.fromEntries([...parts,...(knowledge.steps??[]),...(knowledge.specs??[])].map(entity=>[entity.id,{reviewStatus:"confirmed",...(parts.some(p=>p.id===entity.id)?{textOnly:true}:{})}]));
    const patch=await api.patch(draftUrl,{headers:{"x-csrf-token":q.csrf,"if-match":draftRead.headers().etag??""},data:{entities,modelReview:{loaded:true,userConfirmed:true}}}); expect(patch.status()).toBe(200);
    const current=await api.get(draftUrl); const published=await api.post(draftUrl+"/publish",{headers:{"x-csrf-token":q.csrf,"if-match":current.headers().etag??"","idempotency-key":randomUUID()}}); expect(published.status()).toBe(201); const releaseId=(await published.json()).data.id as string;
    await b.stop(); b.models.manualAi=marker; await b.start(); let csrf=await apiLogin(api,b.base,PASSWORD); const before=b.counts(); const databaseBefore=counts(b);
    const rejected=await api.post(`${b.base}/api/v1/items/${q.itemId}/estimates`,{headers:{"x-csrf-token":csrf},data:{preparationId:q.preparationId,photoIds:q.photoIds,modelPreset:"tripo-h-v3.1-standard"}}); expect(rejected.status()).toBe(422); expect((await rejected.json()).error.details.reason).toBe("providerModelInvalid"); b.noLeak(await rejected.text());
    expect((await api.get(b.base+"/api/v1/health/ready")).status()).toBe(200); const asset=await api.get(`${b.base}/api/v1/assets/${q.sourceAssetId}/content`); expect(asset.status()).toBe(200); expect((await asset.body()).subarray(0,5).toString()).toBe("%PDF-");
    expect((await api.get(`${b.base}/api/v1/items/${q.itemId}/releases/${releaseId}`)).status()).toBe(200);
    const state=await attach(page,b); await loginViaUi(page,WEB,PASSWORD); await page.goto(WEB+`/items/${q.itemId}/releases/${releaseId}`);
    await expect(page.getByRole("heading",{name:"已发布说明书",exact:true})).toBeVisible(); await expect(page.getByTestId("original-canvas")).toBeVisible(); await expect.poll(()=>page.getByTestId("original-canvas").evaluate(el=>(el as HTMLCanvasElement).width)).toBeGreaterThan(0);
    await browserSafe(page,b); expect(counts(b)).toEqual(databaseBefore); expect(b.counts()).toEqual(before); expect(state.external+state.pageErrors).toBe(0);
    // Inject only this QA-owned historical snapshot. Normal current configuration then
    // proves that execution is gated by the actual frozen model, independently of settings.
    await b.stop(); b.models.manualAi=GOOD.manualAi;
    const snapshots=b.db("SELECT provider_config FROM generation_snapshots WHERE id=(SELECT snapshot_id FROM jobs WHERE id=?)",[jobId]) as Array<{provider_config:string}>;
    const snapshot=snapshots[0]; if(!snapshot) throw new Error("QA snapshot absent"); const frozen=JSON.parse(snapshot.provider_config); frozen.manualAi.model=marker;
    b.db("UPDATE generation_snapshots SET provider_config=? WHERE id=(SELECT snapshot_id FROM jobs WHERE id=?)",[JSON.stringify(frozen),jobId],true);
    const stages=b.db("SELECT id FROM job_stages WHERE job_id=? AND stage_kind='manual_extract' LIMIT 1",[jobId]) as Array<{id:string}>; const stage=stages[0]; if(!stage) throw new Error("QA extract stage absent");
    b.db("DELETE FROM provider_attempts WHERE stage_id=?",[stage.id]); b.db("UPDATE job_stages SET status='failed' WHERE id=?",[stage.id]); b.db("UPDATE jobs SET status='needs_input' WHERE id=?",[jobId]);
    await b.start(); csrf=await apiLogin(api,b.base,PASSWORD); const detail=await fetchJobDetail(api,b.base,jobId); const retryBefore=counts(b); const wireBefore=b.counts();
    const retry=await api.post(`${b.base}/api/v1/jobs/${jobId}/retry`,{headers:{"x-csrf-token":csrf,"if-match":detail.etag??"","idempotency-key":randomUUID()},data:{stageId:stage.id}}); expect(retry.status()).toBe(422); b.noLeak(await retry.text()); expect((await retry.json()).error.details.reason).toBe("quoteModelInvalid"); expect(counts(b)).toEqual(retryBefore);
    await b.stop(); b.db("UPDATE job_stages SET status='queued' WHERE id=?",[stage.id]); b.db("UPDATE jobs SET status='queued' WHERE id=?",[jobId]); await b.start(); await apiLogin(api,b.base,PASSWORD);
    await waitForJob(api,b.base,jobId,v=>v.stages.some(s=>s.id===stage.id&&s.status==="needs_input"),"frozen model worker gate"); expect(counts(b)).toEqual(retryBefore); expect(b.counts()).toEqual(wireBefore);
    evidence("existing-reading-and-frozen-worker",{itemId:q.itemId,releaseId,sourceAssetId:q.sourceAssetId,jobId,currentModelQuoteRefused:422,pdfBytesReadable:true,pdfCanvasRendered:true,releaseReadable:true,readCounterDelta:0,frozenRetryRefused:422,frozenWorkerNeedsInput:true,newAttemptReservationDelta:0,providerDelta:0,leak:false});
  } finally {await b.cleanup();}
});
