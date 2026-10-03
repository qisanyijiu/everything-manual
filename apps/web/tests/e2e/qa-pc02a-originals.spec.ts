import { createHash } from "node:crypto";
import path from "node:path";
import { expect, request, test, type APIRequestContext, type Locator, type Page } from "@playwright/test";
import { Pc02aQaBackend, WEB, OUT, seed, open, saveEvidence } from "./qa-pc02a-backend";

test.describe.configure({mode:"serial",timeout:120000});
let backend:Pc02aQaBackend,api:APIRequestContext,data:Awaited<ReturnType<typeof seed>>;
async function tabTo(page:Page,target:Locator){for(let i=0;i<150;i++){await page.keyboard.press("Tab");if(await target.evaluate(el=>el===document.activeElement))return;}throw new Error("Original reader action not reachable by Tab");}
async function pixels(page:Page){await expect(page.getByTestId("original-canvas")).toBeVisible();await expect.poll(()=>page.getByTestId("original-canvas").evaluate(el=>(el as HTMLCanvasElement).width)).toBeGreaterThan(0);return createHash("sha256").update(await page.getByTestId("original-canvas").evaluate(el=>(el as HTMLCanvasElement).toDataURL())).digest("hex");}
test.beforeAll(async()=>{backend=new Pc02aQaBackend();await backend.start();api=await request.newContext();data=await seed(api,backend);const status=await api.get(backend.base+"/api/v1/settings/status");expect((await status.json()).data.providersConfigured).toEqual({tripo:false,manualAi:false});expect(backend.counters()).toEqual([{preparations:0,jobs:0,attempts:0,ledger:0,drafts:0,releases:0}]);});
test.afterAll(async()=>{await api?.dispose();await backend?.cleanup();});

test("QA PC2-001/003/004 official original real entry, keyboard pages, honest URL and no generated state",async({page})=>{
  const before=backend.counters();const state=await open(page,backend,`/items/${data.officialItemId}`);
  const entry=page.getByRole("link",{name:`查看原件 · ${data.lack.title}`,exact:true});
  await expect(entry).toBeVisible();await tabTo(page,entry);await page.keyboard.press("Enter");
  await expect(page.getByRole("heading",{name:data.lack.title,exact:true})).toBeVisible();await expect(page.getByTestId("original-page-label")).toHaveText("第 1 / 8 页");
  const first=await pixels(page);await page.getByTestId("original-text").locator("summary").click();await expect(page.getByTestId("original-text").locator("pre")).toContainText("LACK");
  const input=page.getByLabel("页码",{exact:true});await tabTo(page,input);await input.fill("8");await page.keyboard.press("Enter");await expect(page.getByTestId("original-page-label")).toHaveText("第 8 / 8 页");
  await expect(page).toHaveURL(/page=8$/);expect(await pixels(page)).not.toBe(first);await expect(page.getByRole("button",{name:"下一页",exact:true})).toBeDisabled();
  for(const invalid of ["0","1.5","9","text"]){await input.fill(invalid);await input.press("Enter");await expect(input).toHaveAttribute("aria-invalid","true");await expect(page.getByRole("alert")).toContainText("请输入 1 至 8 的整数页码");await expect(page).toHaveURL(/page=8$/);await expect(page.getByTestId("original-page-label")).toHaveText("第 8 / 8 页");}
  await tabTo(page,page.getByRole("link",{name:"返回物品资料",exact:true}).last());await page.keyboard.press("Enter");await expect(entry).toBeFocused();
  await expect(page.getByText(data.lack.sourceUrl??"",{exact:false})).toBeVisible();await page.getByRole("listitem").filter({has:entry}).getByText("文件校验信息",{exact:true}).click();await expect(page.getByText(`SHA256 ${data.lack.sourceSha256}`,{exact:true})).toBeVisible();
  for(const invalid of ["0","1.5","9","text"]){await page.goto(`${WEB}/items/${data.officialItemId}/documents/${data.lack.id}?page=${invalid}`);await expect(page.getByRole("alert")).toContainText("此页码不可用");await expect(page.getByTestId("original-canvas")).toBeHidden();await input.fill("1");await input.press("Enter");await expect(page.getByTestId("original-page-label")).toHaveText("第 1 / 8 页");}
  expect(backend.counters()).toEqual(before);expect(backend.counts()).toEqual({upload:0,submit:0,task:0,manual:0,cdn:0});expect(state.external+state.pageErrors).toBe(0);
  saveEvidence("official-entry",{officialItemId:data.officialItemId,documentId:data.lack.id,pages:8,sourceSha256:data.lack.sourceSha256,sourceUrl:data.lack.sourceUrl,before,after:backend.counters(),providerRequests:0,external:state.external,pageErrors:state.pageErrors});
});

test("QA PC2-001/003 pagination includes document 21 and distinct real asset pages",async({page})=>{
  const before=backend.counters();const state=await open(page,backend,`/items/${data.multiItemId}`);await expect(page.getByRole("link",{name:/^查看原件 ·/})).toHaveCount(22);expect(state.documentQueries).toBeGreaterThanOrEqual(2);
  const digests:string[]=[];
  for(const doc of [data.first,data.second]){const entry=page.getByRole("link",{name:`查看原件 · ${doc.title}`,exact:true});await tabTo(page,entry);await page.keyboard.press("Enter");await expect(page.getByRole("heading",{name:doc.title,exact:true})).toBeVisible();digests.push(await pixels(page));await page.getByRole("link",{name:"返回物品资料",exact:true}).last().click();await expect(entry).toBeFocused();}
  expect(digests[0]).not.toBe(digests[1]);expect(backend.counters()).toEqual(before);expect(state.external+state.pageErrors).toBe(0);
  saveEvidence("pagination-assets",{itemId:data.multiItemId,documentIds:[data.first.id,data.second.id],listCount:22,documentQueries:state.documentQueries,distinctRenderedPages:true,counterDelta:0});
});

test("QA PC2-001/004 official scan keeps image-only pages readable without invented text",async({page})=>{
  const before=backend.counters();const state=await open(page,backend,`/items/${data.officialItemId}`);
  const entry=page.getByRole("link",{name:`查看原件 · ${data.scan.title}`,exact:true});await tabTo(page,entry);await page.keyboard.press("Enter");
  await expect(page.getByTestId("original-page-label")).toHaveText("第 1 / 42 页");const first=await pixels(page);
  await page.getByTestId("original-text").locator("summary").click();await expect(page.getByText("本页没有可读取的文字层，可查看上方页图。",{exact:true})).toBeVisible();
  const input=page.getByLabel("页码",{exact:true});await input.fill("42");await input.press("Enter");await expect(page.getByTestId("original-page-label")).toHaveText("第 42 / 42 页");expect(await pixels(page)).not.toBe(first);
  await expect(page.getByRole("button",{name:"下一页",exact:true})).toBeDisabled();await expect(page.getByTestId("original-text").locator("pre")).toHaveCount(0);
  await page.screenshot({path:path.join(OUT,"official-scan-page42.png")});await page.getByRole("link",{name:"返回物品资料",exact:true}).last().click();await expect(entry).toBeFocused();
  expect(backend.counters()).toEqual(before);expect(state.external+state.pageErrors).toBe(0);
  saveEvidence("official-scan",{itemId:data.officialItemId,documentId:data.scan.id,pages:42,sourceSha256:data.scan.sourceSha256,sourceUrl:data.scan.sourceUrl,sampledPages:[1,42],noInventedText:true,counterDelta:0});
});

const missing="01930000-0000-7000-8000-000000000090";
// Only reference metadata is synthetic. Both PDF byte streams, document pagination and failures use the real isolated backend.
async function references(page:Page,release:boolean){
  const ref=(documentId:string,pageNumber:number)=>({documentId,pageNumber,quote:"Synthetic QA reference metadata"});
  const knowledge={model:{assetId:missing,revisionId:missing,sha256:"b".repeat(64),validationState:"validated"},knowledge:{
    parts:[{id:"qa-part",name:"QA 部件",description:"跨文档独立 QA",evidence:[ref(data.first.id,1),ref(missing,1),ref(data.first.id,99)]}],
    steps:[{id:"qa-step",title:"QA 步骤",orderedActions:["核对补充原件"],partIds:["qa-part"],safetyNotes:[],evidence:[ref(data.second.id,1)]}],specs:[]},hotspots:[],stepPoses:{}};
  const resource=release?"releases/qa-pc02a-release":"drafts/qa-pc02a-draft";
  await page.route(`**/api/v1/items/${data.multiItemId}/${resource}`,entry=>entry.fulfill({status:200,contentType:"application/json",headers:{etag:'"1"'},body:JSON.stringify({data:release?{
    id:"qa-pc02a-release",itemId:data.multiItemId,draftRevision:1,modelRevisionId:missing,manifestSha256:"b".repeat(64),manifest:{knowledge,review:{},documents:[data.first,data.second].map(doc=>({...doc,documentId:doc.id}))}
  }:{id:"qa-pc02a-draft",itemId:data.multiItemId,revision:1,status:"needs_review",completeness:"complete",knowledge,review:{}}})}));
  return `/items/${data.multiItemId}/${resource}${release?"":"/review"}`;
}
async function showPanel(page:Page,width:number,panel:"parts"|"steps"){
  if(width<768)await page.getByRole("button",{name:panel==="parts"?/^(部件|部件与热点)$/:"步骤与原文",exact:true}).click();
  else if(width<1280){const toggle=page.getByRole("button",{name:/^(显示|隐藏)步骤与原文$/});await expect(toggle).toBeVisible();if((await toggle.textContent())?.startsWith("显示"))await toggle.click();await page.getByRole("tab",{name:panel==="parts"?/^(部件|部件与热点)$/:"步骤与原文",exact:true}).click();}
}
async function original(page:Page,which:"first"|"second",number=1){
  const doc=data[which];await expect(page.locator("#original-heading")).toContainText(doc.title);await expect(page.getByTestId("original-page-label")).toHaveText(`第 ${number} / ${which==="first"?2:1} 页`);
  await pixels(page);const text=page.getByTestId("original-text");if(await text.getAttribute("open")===null)await text.locator("summary").click();
  await expect(text.locator("pre")).toContainText(which==="first"?`Page ${number} of 2`:"部件一：松开四颗螺丝");await expect(page.getByTestId("original-canvas")).toHaveCount(1);
}

test("QA PC2-002/003 real assets behind synthetic part/step refs: 3 widths, 2 consumers, focus and resize",async({page})=>{
  const before=backend.counters();const state=await open(page,backend,`/items/${data.multiItemId}`);const results=[];
  for(const release of [false,true]){
    const route=await references(page,release);
    for(const width of [375,1024,1440]){
      await page.setViewportSize({width,height:900});await page.goto(WEB+route);const hashes:string[]=[];
      for(const panel of ["parts","steps"] as const){
        await showPanel(page,width,panel);const source=page.locator(panel==="parts"?"#evidence-qa-part-0":"#evidence-qa-step-0");await expect(source).toContainText(panel==="parts"?data.first.title:data.second.title);
        expect((await source.boundingBox())?.height).toBeGreaterThanOrEqual(44);await tabTo(page,source);await page.keyboard.press("Enter");await expect(page.locator("#original-heading")).toBeFocused();
        await original(page,panel==="parts"?"first":"second");hashes.push(await pixels(page));if(width===375)await expect(page.getByRole("dialog")).toHaveCount(1);if(width===1024)await expect(page.getByRole("tab",{name:"原文",exact:true})).toHaveAttribute("aria-selected","true");
        const back=page.getByRole("button",{name:"返回出处",exact:true});expect((await back.boundingBox())?.height).toBeGreaterThanOrEqual(44);expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
        if(panel==="steps"&&width===375)await page.screenshot({path:path.join(OUT,`${release?"release":"draft"}-375.png`)});
        await tabTo(page,back);await page.keyboard.press("Enter");await expect(source).toBeFocused();if(width===375){await page.keyboard.press("Escape");await expect(page.getByRole("dialog")).toHaveCount(0);}
      }
      expect(hashes[0]).not.toBe(hashes[1]);results.push({release,width,distinctCanvas:true,sourceFocusReturned:true});
    }
    await page.locator("#evidence-qa-part-1").click();await expect(page.locator(".original-section").getByRole("alert")).toContainText("此出处的原件不可用");await expect(page.getByTestId("original-canvas")).toHaveCount(0);
    await page.getByRole("button",{name:"返回出处",exact:true}).click();await expect(page.locator("#evidence-qa-part-1")).toBeFocused();await page.locator("#evidence-qa-part-2").click();await expect(page.locator(".original-section").getByRole("alert")).toContainText("此出处页码超出原件范围");await expect(page.getByTestId("original-canvas")).toBeHidden();
    await page.getByLabel("原件",{exact:true}).selectOption(data.second.id);await original(page,"second");await page.getByLabel("原件",{exact:true}).selectOption(data.first.id);await original(page,"first");
    await page.getByLabel("页码",{exact:true}).fill("2");await page.getByLabel("页码",{exact:true}).press("Enter");await original(page,"first",2);
    for(const width of [1024,375,1440]){await page.setViewportSize({width,height:900});await expect(page.locator(`.page-layout--${width<768?"narrow":width<1280?"mid":"wide"}`)).toBeVisible();await original(page,"first",2);await expect(page.getByLabel("原件",{exact:true})).toHaveValue(data.first.id);if(width===375)await expect(page.getByRole("dialog")).toHaveCount(1);}
    await page.getByRole("button",{name:"返回出处",exact:true}).click();await expect(page.locator("#evidence-qa-part-2")).toBeFocused();
  }
  expect(backend.counters()).toEqual(before);expect(state.external+state.pageErrors).toBe(0);expect(backend.counts()).toEqual({upload:0,submit:0,task:0,manual:0,cdn:0});
  saveEvidence("reference-matrix",{metadata:"synthetic only; real uploaded PDF/document assets",results,missingAndOutOfRange:true,resizePreserved:true,documentQueries:state.documentQueries,counterDelta:0});
});

test("QA PC2-004 local 404 retry and keyboard drawer return work with WebGL unavailable",async({page})=>{
  await page.addInitScript(()=>{const original=HTMLCanvasElement.prototype.getContext;HTMLCanvasElement.prototype.getContext=function(this:HTMLCanvasElement,type:string,...args:unknown[]){return type.includes("webgl")?null:Reflect.apply(original,this,[type,...args]);} as typeof original;});
  const before=backend.counters();const state=await open(page,backend,`/items/${data.multiItemId}`);const route=await references(page,true);const assetPattern=`**/api/v1/assets/${data.first.sourceAssetId}/content`;
  await page.route(assetPattern,entry=>entry.continue({url:`${backend.base}/api/v1/assets/${missing}/content`}));await page.setViewportSize({width:375,height:900});await page.goto(WEB+route);await expect(page.getByText(/浏览器 3D 上下文不可用/)).toBeVisible();
  await showPanel(page,375,"parts");const source=page.locator("#evidence-qa-part-0");await tabTo(page,source);await page.keyboard.press("Enter");await expect(page.getByTestId("original-error")).toContainText("诊断请求 ID");await page.unroute(assetPattern);
  await tabTo(page,page.getByRole("button",{name:"重新加载原文",exact:true}));await page.keyboard.press("Enter");await original(page,"first");await tabTo(page,page.getByRole("button",{name:"下一页",exact:true}));await page.keyboard.press("Enter");await original(page,"first",2);
  await page.keyboard.press("Escape");await expect(source).toBeFocused();await expect(page.getByRole("dialog")).toHaveCount(1);await page.keyboard.press("Escape");await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(backend.counters()).toEqual(before);expect(state.external+state.pageErrors).toBe(0);expect(backend.counts()).toEqual({upload:0,submit:0,task:0,manual:0,cdn:0});saveEvidence("local-failure",{real404:true,sameDocumentRetry:true,webglUnavailable:true,keyboardPagingAndEscape:true,counterDelta:0});
});

test("QA PC2-004 late PDF response cannot overwrite a newly selected document",async({page})=>{
  const before=backend.counters();const state=await open(page,backend,`/items/${data.multiItemId}`);const route=await references(page,true);let unblock:()=>void=()=>{};let fetched=false,finished=false;
  const held=new Promise<void>(resolve=>{unblock=resolve;});const assetPattern=`**/api/v1/assets/${data.first.sourceAssetId}/content`;
  await page.route(assetPattern,async entry=>{const response=await entry.fetch({url:`${backend.base}/api/v1/assets/${data.first.sourceAssetId}/content`});fetched=true;await held;try{await entry.fulfill({response});}catch{/* canceled request is the expected losing branch */}finally{finished=true;}});
  try{await page.setViewportSize({width:375,height:900});await page.goto(WEB+route);await showPanel(page,375,"parts");await page.locator("#evidence-qa-part-0").click();await expect.poll(()=>fetched).toBe(true);await expect(page.getByRole("status").filter({hasText:"正在加载原文"})).toBeVisible();
    await page.getByLabel("原件",{exact:true}).selectOption(data.second.id);await original(page,"second");const digest=await pixels(page);unblock();await expect.poll(()=>finished).toBe(true);await page.unroute(assetPattern);await original(page,"second");expect(await pixels(page)).toBe(digest);
    await page.getByRole("button",{name:"返回出处",exact:true}).click();await expect(page.locator("#evidence-qa-part-0")).toBeFocused();expect(backend.counters()).toEqual(before);expect(state.external+state.pageErrors).toBe(0);saveEvidence("late-response",{lateResponseIgnored:true,selectedDocumentId:data.second.id,counterDelta:0});
  }finally{unblock();}
});
