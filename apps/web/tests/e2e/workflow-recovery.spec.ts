import path from "node:path";
import fs from "node:fs";
import { execFileSync } from "node:child_process";
import { expect, request, test, type APIRequestContext, type Page } from "@playwright/test";
import { apiLogin, loginViaUi, seedItemWithDocument, seedPhoto, seedReadyPreparation } from "./helpers";
import { BACKEND_PASSWORD, LocalFixture, TestBackend, waitForJob } from "./job-recovery-harness";
import { installRealBackendRouting } from "./viewer-harness";
import { E2E_WEB_PORT, REPO_ROOT } from "./runtime";
test.describe.configure({mode:"serial",timeout:120_000});
const web=`http://127.0.0.1:${E2E_WEB_PORT}`;
const evidence=path.join(REPO_ROOT,"artifacts/prd-completion/pc03b-rd");
let backend:TestBackend,fixture:LocalFixture,api:APIRequestContext;
test.beforeAll(async()=>{fs.mkdirSync(evidence,{recursive:true});fixture=new LocalFixture();await fixture.start();backend=new TestBackend("pc03b-rd",process.env.EM_E2E_SERVER_BINARY??path.join(REPO_ROOT,"target/debug/everything-manual"));await backend.start(fixture);api=await request.newContext();});
test.afterAll(async()=>{await api?.dispose();await backend?.cleanup(fixture);});
const count=(table:string,itemId:string)=>Number(execFileSync("sqlite3",[path.join(backend.dataDir,"manual.sqlite3"),`SELECT COUNT(*) FROM ${table} WHERE item_id='${itemId}'`],{encoding:"utf8"}).trim());
async function seed(name:string){const data=await seedItemWithDocument(api,backend.base,BACKEND_PASSWORD,"sample-manual-text.pdf",name);const csrf=await apiLogin(api,backend.base,BACKEND_PASSWORD);const preparationId=await seedReadyPreparation(api,backend.base,csrf,data);await seedPhoto(api,backend.base,csrf,data.itemId,"front","sample-photo-front.jpg");await seedPhoto(api,backend.base,csrf,data.itemId,"left","sample-photo-left.png");return{...data,preparationId,csrf};}
async function open(page:Page,url:string){const routing=await installRealBackendRouting(page,backend.base);await loginViaUi(page,web,BACKEND_PASSWORD);await page.goto(url);return routing;}
const confirm=async(page:Page)=>{await expect(page.getByTestId("quote-panel")).toBeVisible();await page.getByLabel("我已阅读并确认将上述资料发送给对应供应商").check();await expect(page.getByTestId("generate-button")).toBeEnabled();};
function failure(){return{status:500,contentType:"application/json",body:JSON.stringify({error:{code:"INTERNAL_ERROR",message:"本机故障注入：报价读取失败",requestId:"pc3b-read-failed",details:null}})};}

test("PC3B-006: 20 rows use a bounded summary request, stable pagination and unknown state keeps overview",async({page})=>{
  const csrf=await apiLogin(api,backend.base,BACKEND_PASSWORD);
  for(let n=0;n<21;n++){const r=await api.post(`${backend.base}/api/v1/items`,{headers:{"x-csrf-token":csrf},data:{name:`BATCH ${n}`,model:`M-${n}`}});expect(r.status()).toBe(201);}
  const urls:string[]=[];const observedAt=Date.now();const activityMethods:string[]=[];page.on("request",r=>{if(r.url().includes("/api/v1/"))urls.push(r.url());if(new URL(r.url()).pathname==="/api/v1/jobs/activity")activityMethods.push(r.method());});await page.setViewportSize({width:375,height:812});await open(page,web+"/");
  await expect(page.locator(".item-list .item-row")).toHaveCount(20);await expect(page.locator(".item-list").getByRole("link",{name:"补齐资料",exact:true})).toHaveCount(20);
  expect(urls.filter(u=>{const pathname=new URL(u).pathname;return pathname!=="/api/v1/jobs/activity"&&/\/jobs|\/drafts|\/releases|\/preparations/.test(pathname);})).toHaveLength(0);expect(activityMethods.every(method=>method==="GET")).toBe(true);expect(activityMethods.length,"fixed activity polling must not become per-row fanout").toBeLessThanOrEqual(Math.ceil((Date.now()-observedAt)/2000)+2);expect(urls.filter(u=>u.includes("/items/summaries?")).length).toBeLessThanOrEqual(2); // login redirect and explicit navigation only
  const firstPageIds=await page.locator(".item-list .item-row").evaluateAll(rows=>rows.map(row=>row.getAttribute("data-library-item")));
  await page.getByRole("button",{name:"加载更多",exact:true}).click();await expect(page.locator(".item-list .item-row")).toHaveCount(21);await expect(page.locator(".item-list").getByRole("link",{name:"补齐资料",exact:true})).toHaveCount(21);
  const appendedId=await page.locator(".item-list .item-row").last().getAttribute("data-library-item");expect(appendedId).toBeTruthy();expect(firstPageIds).not.toContain(appendedId);
  const appendedRow=page.locator(`.item-list .item-row[data-library-item="${appendedId}"]`);
  let fail=true;await page.route("**/api/v1/items/summaries?*",async route=>{if(fail)await route.fulfill(failure());else await route.fallback();});await page.reload();await expect(appendedRow).toContainText("处理状态暂不可用");await expect(appendedRow.getByRole("link",{name:"查看物品",exact:true})).toHaveAttribute("href",`/items/${appendedId}`);await expect(appendedRow.getByRole("link",{name:"补齐资料",exact:true})).toHaveCount(0);
  fail=false;await page.getByRole("button",{name:"重新读取处理状态",exact:true}).focus();await page.keyboard.press("Enter");await expect(appendedRow.getByRole("link",{name:"补齐资料",exact:true})).toHaveAttribute("href",`/items/${appendedId}/import/document`);
  await page.screenshot({path:path.join(evidence,"summary-375.png"),fullPage:true});
});

test("PC3B-008/009: committed POST response lost, failed GET and fresh context recover the same job without new writes",async({page,browser})=>{
  const data=await seed("LOST COMMITTED");await page.setViewportSize({width:375,height:812});await open(page,`${web}/items/${data.itemId}/import/confirm`);await confirm(page);
  let jobId="",posts=0,failRead=true;const quoteId=new URL(page.url()).searchParams.get("quoteId");expect(quoteId).toBeTruthy();
  await page.route(`**/api/v1/items/${data.itemId}/jobs`,async route=>{if(route.request().method()!=="POST"){await route.fallback();return;}posts++;const r=await route.fetch({url:backend.base+new URL(route.request().url()).pathname});expect(r.status()).toBe(202);jobId=(await r.json()).data.id;await route.abort("connectionfailed");});
  await page.route(`**/api/v1/items/${data.itemId}/estimates/${quoteId}`,async route=>{if(failRead)await route.fulfill(failure());else await route.fallback();});
  await page.getByTestId("generate-button").focus();await page.keyboard.press("Enter");await expect(page.getByTestId("submission-recovery")).toContainText("报价读取失败");await expect(page.getByTestId("generate-button")).toBeDisabled();expect(posts).toBe(1);expect(count("jobs",data.itemId)).toBe(1);expect(count("quotes",data.itemId)).toBe(1);
  await page.reload();await expect(page.getByTestId("submission-recovery")).toContainText("报价读取失败");expect(posts).toBe(1);await page.screenshot({path:path.join(evidence,"recovery-375.png"),fullPage:true});
  await waitForJob(api,backend.base,jobId,j=>j.status==="succeeded","local fake completion");const counts={...fixture.counts};
  const fresh=await browser.newContext({viewport:{width:375,height:812}});const next=await fresh.newPage();let mutations=0;next.on("request",r=>{if(["POST","PUT","PATCH"].includes(r.method())&&!r.url().endsWith("/auth/login"))mutations++;});await open(next,`${web}/items/${data.itemId}/import/confirm`);await expect(next.getByTestId("job-accepted")).toContainText(jobId);await expect(next.getByRole("link",{name:"查看任务详情",exact:true})).toHaveAttribute("href",`/jobs/${jobId}`);expect(mutations).toBe(0);expect(count("jobs",data.itemId)).toBe(1);expect(count("quotes",data.itemId)).toBe(1);expect(fixture.counts).toEqual(counts);await fresh.close();
  failRead=false;await page.getByRole("button",{name:"重新核对结果",exact:true}).click();await expect(page.getByTestId("job-accepted")).toContainText(jobId);expect(posts).toBe(1);
  fs.writeFileSync(path.join(evidence,"consumption-counts.json"),JSON.stringify({jobs:count("jobs",data.itemId),quotes:count("quotes",data.itemId),postRequests:posts,providerCounts:counts,providerCountsAfterRecovery:fixture.counts},null,2));
});

test("PC3B-008: unconsumed failed transport only permits explicit same-key same-body retry",async({page})=>{
  const data=await seed("NOT FORWARDED");await open(page,`${web}/items/${data.itemId}/import/confirm`);await confirm(page);const requests:{key:string|undefined;body:string|null}[]=[];
  await page.route(`**/api/v1/items/${data.itemId}/jobs`,async route=>{requests.push({key:route.request().headers()["idempotency-key"],body:route.request().postData()});if(requests.length===1)await route.abort("connectionfailed");else await route.fallback();});
  await page.getByTestId("generate-button").click();await expect(page.getByRole("button",{name:"重试同一提交（使用原授权）",exact:true})).toBeEnabled();expect(count("jobs",data.itemId)).toBe(0);await expect(page.getByTestId("generate-button")).toBeDisabled();
  await expect(page.getByLabel("Tripo credits",{exact:true})).toBeDisabled();await page.getByRole("button",{name:"重试同一提交（使用原授权）",exact:true}).focus();await page.keyboard.press("Enter");await expect(page.getByTestId("job-accepted")).toBeVisible();expect(requests).toHaveLength(2);expect(requests[0]).toEqual(requests[1]);expect(count("jobs",data.itemId)).toBe(1);expect(count("quotes",data.itemId)).toBe(1);
});

test("PC3B-007: chosen PDF context and real changed views produce distinct steps; navigation never marks completion",async({page})=>{
  const data=await seed("STEP FACTS");await open(page,`${web}/items/${data.itemId}/import/confirm?documentId=${data.documentId}&preparationId=${data.preparationId}`);await confirm(page);
  const steps=page.getByRole("list",{name:"新建向导步骤"});await expect(steps.getByText("已完成",{exact:true})).toHaveCount(5);
  const source=await api.get(`${backend.base}/api/v1/items/${data.itemId}/documents`);const document=(await source.json()).data.find((d:{id:string})=>d.id===data.documentId);const r=await api.post(`${backend.base}/api/v1/items/${data.itemId}/documents`,{headers:{"x-csrf-token":data.csrf},data:{sourceAssetId:document.sourceAssetId,title:"SECOND PDF"}});expect(r.status()).toBe(201);const second=(await r.json()).data.id;
  await page.goto(`${web}/items/${data.itemId}/import/prepare`);await expect(page.getByLabel("所选原件",{exact:true})).toHaveValue(second);await expect(steps.getByText("需重新检查",{exact:true})).toHaveCount(2);expect(count("jobs",data.itemId)).toBe(0);
  await page.goto(`${web}/items/${data.itemId}/import/prepare?documentId=${data.documentId}&preparationId=${data.preparationId}`);await expect(page.getByLabel("所选原件",{exact:true})).toHaveValue(data.documentId);await expect(steps.getByText("已完成",{exact:true})).toHaveCount(5);await steps.getByRole("link",{name:"视图排列",exact:true}).click();expect(new URL(page.url()).searchParams.get("documentId")).toBe(data.documentId);
  const photos=await api.get(`${backend.base}/api/v1/items/${data.itemId}/photos`);const left=(await photos.json()).data.find((p:{view:string})=>p.view==="left");const changed=await api.patch(`${backend.base}/api/v1/items/${data.itemId}/photos/${left.id}`,{headers:{"x-csrf-token":data.csrf,"if-match":`"r${left.revision}"`},data:{view:"detail"}});expect(changed.status()).toBe(200);await page.reload();await expect(steps.getByText("需重新检查",{exact:true})).toHaveCount(2);expect(count("quotes",data.itemId)).toBe(1);expect(count("jobs",data.itemId)).toBe(0);
});
