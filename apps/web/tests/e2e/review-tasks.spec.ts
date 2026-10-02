import path from "node:path";
import fs from "node:fs";
import { expect, request, test, type APIRequestContext, type Page } from "@playwright/test";
import { apiLogin, loginViaUi } from "./helpers";
import { BACKEND_PASSWORD, LocalFixture, TestBackend, seedJob, waitForJob } from "./job-recovery-harness";
import { installRealBackendRouting } from "./viewer-harness";
import { E2E_WEB_PORT, REPO_ROOT } from "./runtime";
import { readDraftParts, readDraftSteps, readDraftSpecs, readDraftModel, readDraftHotspots } from "../../src/features/viewer/draft-view";
import type { DraftDto } from "../../src/api/endpoints";
test.describe.configure({ mode: "serial", timeout: 120_000 });
const web=`http://127.0.0.1:${E2E_WEB_PORT}`;
const evidence=path.join(REPO_ROOT,"artifacts/prd-completion/pc02b-rd");
let backend:TestBackend; let fixture:LocalFixture; let api:APIRequestContext; let csrf="";
test.beforeAll(async()=>{
  fs.mkdirSync(evidence,{recursive:true});
  fixture=new LocalFixture(); Object.assign(fixture.state,{manualMode:"success",submitMode:"success",modelMode:"valid",manualDelayMs:0}); await fixture.start();
  backend=new TestBackend("pc02b-rd",process.env.EM_E2E_SERVER_BINARY??path.join(REPO_ROOT,"var/pc03a-qa-round4/everything-manual-fixture")); await backend.start(fixture); api=await request.newContext();
});
test.afterAll(async()=>{await api?.dispose();await backend?.cleanup(fixture);});
async function seed(label:string){
  const job=await seedJob(api,backend,label); const done=await waitForJob(api,backend.base,job.jobId,job=>job.status==="succeeded","fixture draft",120_000);
  if(!done.draftId)throw new Error("missing draft"); csrf=await apiLogin(api,backend.base,BACKEND_PASSWORD);
  const endpoint=`/api/v1/items/${job.itemId}/drafts/${done.draftId}`;
  const get=async()=>{const r=await api.get(backend.base+endpoint);expect(r.status()).toBe(200);const etag=r.headers()["etag"];if(!etag)throw new Error("ETag required");return {draft:(await r.json()).data as DraftDto,etag};};
  const patch=async(body:unknown)=>{const{etag}=await get();const r=await api.patch(backend.base+endpoint,{headers:{"x-csrf-token":csrf,"if-match":etag},data:body});expect(r.status(),await r.text()).toBe(200);};
  const {draft}=await get();const parts=readDraftParts(draft.knowledge),steps=readDraftSteps(draft.knowledge),specs=readDraftSpecs(draft.knowledge);
  if(!parts[0]||!steps[0])throw new Error("fixture knowledge incomplete");
  return {endpoint,url:`${web}/items/${job.itemId}/drafts/${done.draftId}/review`,get,patch,parts,steps,specs};
}
async function open(page:Page, seed:{url:string}){const routes=await installRealBackendRouting(page,backend.base);await loginViaUi(page,web,BACKEND_PASSWORD);await page.goto(seed.url);await expect(page.getByTestId("review-tasks")).toBeVisible();return routes;}
async function ready(data:Awaited<ReturnType<typeof seed>>){const entities=Object.fromEntries([...data.parts,...data.steps,...data.specs].map(e=>[e.id,{reviewStatus:"confirmed",...(data.parts.some(p=>p.id===e.id)?{textOnly:true}:{})}]));await data.patch({entities,modelReview:{loaded:true,userConfirmed:true}});}

test("PC2B-005/006: real counts and keyboard task focus across 375/1024/1440; readback success and explicit next",async({page})=>{
  const data=await seed("TASK NAV");const before=JSON.stringify(fixture.counts);await page.setViewportSize({width:375,height:812});await open(page,data);await page.screenshot({path:path.join(evidence,"tasks-375.png"),fullPage:true});
  const part=data.parts[0]!;const task=page.getByTestId(`todo-fact-${part.id}`);
  await expect(page.getByLabel("只看未完成")).toBeChecked(); await expect(page.getByRole("heading",{name:`文字事实 · 未完成 ${data.parts.length+data.steps.length+data.specs.length}`})).toBeVisible();
  await task.getByRole("button").focus();await page.keyboard.press("Enter");await expect(page.locator(`[id="review-fact-${part.id}"]`)).toBeFocused();
  await page.getByTestId(`knowledge-${part.id}`).getByRole("button",{name:"复制为本地修订"}).click();await page.getByLabel("部件名",{exact:true}).fill("未保存的盖板修订");
  await page.getByRole("button",{name:"关闭",exact:true}).click();await page.getByRole("button",{name:"部件与热点",exact:true}).click();
  await page.getByTestId(`part-row-${part.id}`).getByRole("button",{name:/查看出处/}).click();await expect(page.locator("#original-heading")).toBeFocused();await page.getByRole("button",{name:"返回出处"}).click();
  await page.getByRole("button",{name:"关闭",exact:true}).click();await page.getByRole("button",{name:"步骤与原文",exact:true}).click();await expect(page.getByLabel("部件名",{exact:true})).toHaveValue("未保存的盖板修订");
  for(const width of [1024,1440]){await page.setViewportSize({width,height:900});await expect(page.getByLabel("部件名",{exact:true})).toHaveValue("未保存的盖板修订");}
  await page.screenshot({path:path.join(evidence,"editing-1440.png"),fullPage:true});
  await page.getByRole("button",{name:"保存人工修订（并确认事实）"}).click();await expect(page.getByText("此项已处理",{exact:true})).toBeVisible();await expect(task).toHaveCount(0);
  const revision=(await data.get()).draft.revision;await page.getByRole("button",{name:"下一项",exact:true}).focus();await page.keyboard.press("Enter");await expect(page.locator(`[id="review-fact-${data.parts[1]?.id??data.steps[0]!.id}"]`)).toBeFocused();expect((await data.get()).draft.revision).toBe(revision);
  await page.getByRole("button",{name:"返回发布区",exact:true}).click();await expect(page.locator("#publish-heading")).toBeFocused();await page.getByLabel("只看未完成").uncheck();await expect(task).toContainText("已保存人工修订");
  await page.setViewportSize({width:375,height:812});await page.getByTestId(`todo-binding-${part.id}`).getByRole("button").click();await expect(page.locator(`[id="geometry-note-${part.id}"]`)).toBeFocused();await expect(page.getByTestId(`part-row-${part.id}`).getByRole("button",{name:"绑定热点",exact:true})).toBeDisabled();expect((await data.get()).draft.revision).toBe(revision);
  expect(JSON.stringify(fixture.counts)).toBe(before);
});

test("PC2B-007: real concurrent PATCH 412 preserves buffers; one confirmation, failed GET, then explicit successful refresh",async({page})=>{
  const data=await seed("CONFLICT");await open(page,data);const part=data.parts[0]!;
  await page.getByTestId(`todo-fact-${part.id}`).getByRole("button").click();await page.getByTestId(`knowledge-${part.id}`).getByRole("button",{name:"复制为本地修订"}).click();await page.getByLabel("部件名",{exact:true}).fill("不能丢失");
  await data.patch({entities:{[part.id]:{reviewStatus:"confirmed"}}});
  await page.getByRole("button",{name:"保存人工修订（并确认事实）"}).click();await expect(page.getByTestId("conflict-panel")).toBeVisible();await expect(page.getByLabel("部件名",{exact:true})).toHaveValue("不能丢失");
  // PC05B: reading is non-destructive; one shared HTML dialog explicitly discards.
  const confirm=page.getByRole("dialog",{name:"离开当前页面？"});let nativeDialogs=0;page.on("dialog",async d=>{nativeDialogs++;await d.dismiss();});
  await page.getByRole("button",{name:"核对最新版本",exact:true}).click();await expect(confirm).toHaveCount(1);await expect(confirm).toContainText("本页所有未保存的知识修订");await expect(confirm.getByRole("button",{name:"继续处理"})).toBeFocused();await confirm.getByRole("button",{name:"继续处理"}).click();await expect(page.getByLabel("部件名",{exact:true})).toHaveValue("不能丢失");
  let fail=false;await page.route(`**${data.endpoint}`,async route=>{if(fail&&route.request().method()==="GET")await route.fulfill({status:500,contentType:"application/json",body:JSON.stringify({error:{code:"INTERNAL_ERROR",message:"fixture refresh failure",requestId:"pc2b-refresh",details:null}})});else await route.fallback();});
  await page.getByRole("button",{name:"核对最新版本",exact:true}).click();await expect(confirm).toHaveCount(1);fail=true;await confirm.getByRole("button",{name:"丢弃本页修改并加载最新版本"}).click();await expect(page.getByTestId("workspace-notice")).toContainText("读取失败，本地编辑仍保留");await expect(page.getByLabel("部件名",{exact:true})).toHaveValue("不能丢失");await expect(page.getByTestId("conflict-panel")).toBeVisible();
  fail=false;await page.getByRole("button",{name:"核对最新版本",exact:true}).click();await expect(confirm).toHaveCount(1);await confirm.getByRole("button",{name:"丢弃本页修改并加载最新版本"}).click();await expect(page.getByLabel("部件名",{exact:true})).toHaveCount(0);await expect(page.getByTestId("conflict-panel")).toHaveCount(0);expect(nativeDialogs).toBe(0);
});

test("PC2B-007/008: known and unknown 422 stay visible, explicit reread and real immutable publication",async({page})=>{
  const data=await seed("PUBLISH");await ready(data);await page.setViewportSize({width:375,height:812});const before=JSON.stringify(fixture.counts);await open(page,data);
  let fail=true;await page.route(`**${data.endpoint}/publish`,async route=>{if(fail)await route.fulfill({status:422,contentType:"application/json",body:JSON.stringify({error:{code:"VALIDATION_FAILED",message:"需核对发布内容",requestId:"pc2b-safe-diagnostic",details:{issues:[{code:"knowledgeUnreviewed",entityKind:"part",entityId:data.parts[0]!.id,message:"部件需要复核"},{code:"futureRule",entityKind:"part",entityId:data.parts[0]!.id,message:"未知规则仍需核对"}]}}})});else await route.fallback();});
  const publish=page.getByRole("button",{name:"发布（生成不可变版本）",exact:true});await expect(publish).toBeEnabled();await publish.click();const issues=page.getByTestId("publish-issues");await expect(issues).toContainText("pc2b-safe-diagnostic");await expect(issues.getByRole("button",{name:/去处理/})).toHaveCount(1);await expect(publish).toBeDisabled();
  await issues.getByRole("button",{name:/去处理/}).click();await expect(page.getByTestId(`knowledge-${data.parts[0]!.id}`)).toBeFocused();await page.getByRole("button",{name:"返回发布区",exact:true}).click();await expect(issues).toContainText("未知规则仍需核对");
  fail=false;await issues.getByRole("button",{name:"重新读取检查结果"}).click();await expect(issues).toContainText("未知规则仍需核对");await expect(issues).toContainText("pc2b-safe-diagnostic");await expect(issues).toContainText("服务端问题尚未确认解决");const retry=page.getByRole("button",{name:"重新检查并发布（通过后生成不可变版本）",exact:true});await expect(retry).toBeEnabled();await expect(page.getByTestId("publish-success")).toHaveCount(0);await retry.focus();await page.keyboard.press("Enter");await expect(page.getByTestId("publish-success")).toContainText("不可再修改");expect(JSON.stringify(fixture.counts)).toBe(before);
});


test("PC2B-007: real publish 412 keeps local edit and requires explicit read confirmation",async({page})=>{
  const data=await seed("PUBLISH CONFLICT");await ready(data);await open(page,data);
  await page.getByLabel("只看未完成").uncheck();const part=data.parts[0]!;
  await page.getByTestId(`todo-fact-${part.id}`).getByRole("button").click();await page.getByTestId(`knowledge-${part.id}`).getByRole("button",{name:"复制为本地修订"}).click();await page.getByLabel("部件名",{exact:true}).fill("发布冲突仍保留");
  await data.patch({entities:{[part.id]:{reviewStatus:"needs_review"}}});
  await page.getByRole("button",{name:"发布（生成不可变版本）",exact:true}).click();await expect(page.getByTestId("publish-conflict")).toBeVisible();await expect(page.getByLabel("部件名",{exact:true})).toHaveValue("发布冲突仍保留");
  const confirm=page.getByRole("dialog",{name:"离开当前页面？"});
  await page.getByTestId("publish-conflict").getByRole("button",{name:"核对最新版本"}).click();await expect(confirm).toHaveCount(1);await confirm.getByRole("button",{name:"继续处理"}).click();await expect(page.getByLabel("部件名",{exact:true})).toHaveValue("发布冲突仍保留");
  await page.getByTestId("publish-conflict").getByRole("button",{name:"核对最新版本"}).click();await expect(confirm).toHaveCount(1);await confirm.getByRole("button",{name:"丢弃本页修改并加载最新版本"}).click();await expect(page.getByTestId("publish-conflict")).toHaveCount(0);await expect(page.getByLabel("部件名",{exact:true})).toHaveCount(0);await expect(page.getByRole("button",{name:"发布（生成不可变版本）",exact:true})).toBeDisabled();
});

test("PC2B-008: real stale record routes to explanation or explicit rebind, preserves hotspot identity",async({page})=>{
  const data=await seed("STALE");const part=data.parts[0]!;const model=readDraftModel((await data.get()).draft.knowledge);if(!model)throw new Error("model missing");
  // Public API fixture: explicitly stale, with a valid anchor; stale is never treated as usable.
  await data.patch({hotspots:{upsert:[{partId:part.id,status:"stale",anchor:{modelRevisionId:model.revisionId,modelSha256:model.sha256,positionLocal:[0,0,1]}}]}});
  const old=readDraftHotspots((await data.get()).draft.knowledge)[0]!;expect(old.status).toBe("stale");const revision=(await data.get()).draft.revision;
  await page.setViewportSize({width:375,height:812});await open(page,data);await page.getByTestId(`todo-binding-${part.id}`).getByRole("button").click();await expect(page.locator(`[id="geometry-note-${old.id}"]`)).toBeFocused();await expect(page.locator(`[id="rebind-${old.id}"]`)).toBeDisabled();expect((await data.get()).draft.revision).toBe(revision);
  await page.setViewportSize({width:1440,height:1000});await expect(page.locator(`[id="rebind-${old.id}"]`)).toBeEnabled();await expect(page.locator(`[id="rebind-${old.id}"]`)).toBeFocused();await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
  await expect(page.getByTestId("pick-mode-active")).toHaveCount(0);await page.locator(`[id="rebind-${old.id}"]`).click();await expect(page.getByText(`当前绑定部件：${part.name}`,{exact:true})).toBeVisible();await page.getByRole("button",{name:"取消拾取",exact:true}).click();expect((await data.get()).draft.revision).toBe(revision);
  await page.locator(`[id="rebind-${old.id}"]`).click();const projection=await page.evaluate(()=>window.__EM_VIEWER__?.project([0,0,1]));if(!projection)throw new Error("projection missing");const box=await page.getByTestId("viewer-canvas").boundingBox();if(!box)throw new Error("canvas missing");await page.mouse.click(box.x+projection.screen[0],box.y+projection.screen[1]);
  await expect(page.getByText("此项已处理",{exact:true})).toBeVisible();const now=readDraftHotspots((await data.get()).draft.knowledge);expect(now).toHaveLength(1);expect(now[0]).toMatchObject({id:old.id,status:"confirmed"});await expect(page.getByTestId("pick-mode-active")).toHaveCount(0);
});


test("PC2B-006/008: model declarations remain two explicit independent actions with readback and next",async({page})=>{
  const data=await seed("MODEL REVIEW");await page.setViewportSize({width:1024,height:900});await open(page,data);await expect(page.getByTestId("viewer-status")).toContainText("模型已加载");
  await page.getByTestId("todo-modelLoaded").getByRole("button").focus();await page.keyboard.press("Enter");await expect(page.locator("#review-modelLoaded")).toBeFocused();await page.keyboard.press("Enter");await expect(page.getByText("此项已处理",{exact:true})).toBeVisible();
  const current=(await data.get()).draft.review as {modelReview?:{loaded:boolean;userConfirmed:boolean}};expect(current.modelReview).toMatchObject({loaded:true,userConfirmed:false});
  await page.getByRole("button",{name:"下一项",exact:true}).focus();await page.keyboard.press("Enter");await expect(page.locator("#review-modelConfirmed")).toBeFocused();await page.keyboard.press("Enter");await expect(page.getByText("此项已处理",{exact:true})).toBeVisible();
  await page.getByRole("button",{name:"返回发布区",exact:true}).click();await expect(page.locator("#publish-heading")).toBeFocused();await expect(page.getByRole("heading",{name:"模型核对 · 未完成 0",exact:true})).toBeVisible();await expect(page.getByRole("button",{name:"发布（生成不可变版本）",exact:true})).toBeDisabled();
});
