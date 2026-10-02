import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { expect, request, test, type APIRequestContext, type Page } from "@playwright/test";
import { apiLogin, loginViaUi, uploadFixture, seedItemWithDocument, fetchPreparation, fetchAsset } from "./helpers";
import { BACKEND_PASSWORD, LocalFixture, TestBackend } from "./job-recovery-harness";
import { E2E_WEB_PORT, REPO_ROOT } from "./runtime";
import { installRealBackendRouting } from "./viewer-harness";

test.describe.configure({ mode: "serial", timeout: 90_000 });
const web = `http://127.0.0.1:${E2E_WEB_PORT}`;
const evidence = path.join(REPO_ROOT,"artifacts/prd-completion/pc03a-rd");
let backend: TestBackend; let fixture: LocalFixture; let api: APIRequestContext; let csrf = "";
interface Seed { itemId: string; docId: string; sha: string; prepId: string; }
function pdf(label: string): Buffer {
  const objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R 5 0 R 7 0 R] /Count 3 >>"];
  for(let n=1;n<=3;n++) {
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 480 600] /Resources << /Font << /F1 9 0 R >> >> /Contents ${n*2+2} 0 R >>`);
    const stream=`BT /F1 28 Tf 40 510 Td (${label} PAGE ${n}) Tj ET\n`;
    objects.push(`<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}endstream`);
  }
  objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
  let out="%PDF-1.4\n"; const offsets=[0];
  for(const [i,obj] of objects.entries()) { offsets.push(Buffer.byteLength(out)); out+=`${i+1} 0 obj\n${obj}\nendobj\n`; }
  const xref=Buffer.byteLength(out);
  out+=`xref\n0 ${objects.length+1}\n0000000000 65535 f \n${offsets.slice(1).map(o=>`${String(o).padStart(10,"0")} 00000 n \n`).join("")}trailer\n<< /Size ${objects.length+1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
  return Buffer.from(out);
}
async function post(url: string, data: unknown) {
  const result=await api.post(backend.base+url,{headers:{"x-csrf-token":csrf},data});
  expect(result.ok(),await result.text()).toBe(true); return (await result.json()).data;
}
async function seed(label: string): Promise<Seed> {
  const item=await post("/api/v1/items",{name:label,model:"PC03A fixture"});
  const upload=await api.post(`${backend.base}/api/v1/items/${item.id}/assets`,{headers:{"x-csrf-token":csrf},multipart:{purpose:"document",file:{name:"manual.pdf",mimeType:"application/pdf",buffer:pdf(label)}}});
  expect(upload.status()).toBe(201); const asset=(await upload.json()).data;
  const doc=await post(`/api/v1/items/${item.id}/documents`,{title:label,sourceAssetId:asset.id});
  const prep=await post(`/api/v1/documents/${doc.id}/preparations`,{sourceSha256:doc.sourceSha256});
  const image=await uploadFixture(api,backend.base,csrf,item.id,"pageImage","sample-photo-front.jpg");
  const page=await api.put(`${backend.base}/api/v1/preparations/${prep.id}/pages/1`,{headers:{"x-csrf-token":csrf},data:{imageAssetId:image.id,textAssetId:null,viewport:{width:32,height:32,rotation:0}}});
  expect(page.status()).toBe(200);
  // Emulate migration from schema 7: v1 must be inferred from actual page assets, not its age.
  sql(`UPDATE preparations SET format_version=NULL WHERE id='${prep.id}'`);
  return {itemId:item.id,docId:doc.id,sha:doc.sourceSha256,prepId:prep.id};
}
function sql(statement: string): string { return execFileSync("sqlite3",[path.join(backend.dataDir,"manual.sqlite3"),statement],{encoding:"utf8"}); }
function counts() { return sql("SELECT json_object('preparations',(SELECT count(*) FROM preparations),'pages',(SELECT count(*) FROM pages),'assets',(SELECT count(*) FROM assets),'quotes',(SELECT count(*) FROM quotes),'jobs',(SELECT count(*) FROM jobs),'costs',(SELECT count(*) FROM cost_ledger),'attempts',(SELECT count(*) FROM provider_attempts));"); }
async function current(seed: Seed) { const r=await api.get(`${backend.base}/api/v1/preparations/${seed.prepId}`); expect(r.status()).toBe(200); return (await r.json()).data; }
async function open(page: Page, seed: Seed, step="prepare") {
  const routing=await installRealBackendRouting(page,backend.base); await loginViaUi(page,web,BACKEND_PASSWORD);
  await page.goto(`${web}/items/${seed.itemId}/import/${step}?documentId=${seed.docId}`); return routing;
}
test.beforeAll(async()=>{ fs.mkdirSync(evidence,{recursive:true}); fixture=new LocalFixture(); await fixture.start(); backend=new TestBackend("pc03a-rd", process.env.EM_E2E_SERVER_BINARY); await backend.start(fixture); api=await request.newContext(); csrf=await apiLogin(api,backend.base,BACKEND_PASSWORD); });
test.afterAll(async()=>{ await api?.dispose(); await backend?.cleanup(fixture); });

test("PC3-001/002/005: fresh context discovers legacy partial; only missing pages, explicit seal, fresh confirmation ready is read-only",async({browser})=>{
  const seedData=await seed("DISCOVERY"); const context=await browser.newContext({viewport:{width:375,height:812}}); const page=await context.newPage();
  const writes:string[]=[]; page.on("request",r=>{if(["PUT","POST"].includes(r.method()) && !r.url().includes("/auth/")) writes.push(`${r.method()} ${new URL(r.url()).pathname}`);});
  const before=counts(); const routing=await open(page,seedData);
  await expect(page.getByTestId("prepare-progress")).toHaveText("已完成 1 页，总页数待读取原件");
  expect(await page.evaluate(()=>sessionStorage.length)).toBe(0); expect(counts()).toBe(before); expect(writes).toEqual([]); expect(routing.assetRequests).toEqual([]);
  const start=page.getByTestId("prepare-start"); await start.focus(); await start.press("Enter");
  await expect(page.getByTestId("prepare-seal")).toBeVisible();
  expect((await current(seedData)).state).toBe("preparing");
  expect(writes.filter(x=>x.startsWith("PUT"))).toEqual([`PUT /api/v1/preparations/${seedData.prepId}/pages/2`,`PUT /api/v1/preparations/${seedData.prepId}/pages/3`]);
  expect(writes.some(x=>/documents.*preparations|complete/.test(x))).toBe(false);
  await page.getByTestId("prepare-seal").click(); await expect(page.getByTestId("prepare-sealed")).toHaveText("准备完成 · 3 页");
  await page.screenshot({path:path.join(evidence,"prepared-375.png"),fullPage:true});
  const after=counts(); await context.close();
  const fresh=await browser.newContext(); const confirm=await fresh.newPage(); const readyWrites:string[]=[];
  confirm.on("request",r=>{if(["PUT","POST"].includes(r.method()) && !r.url().includes("/auth/")) readyWrites.push(r.url());});
  const readyRouting=await open(confirm,seedData,"confirm");
  await expect(confirm.getByText("可直接使用的准备结果 · 推荐")).toBeVisible(); await expect(confirm.getByText("准备完成 · 3 页")).toBeVisible();
  await expect(confirm.getByTestId("generate-button")).toBeDisabled(); // no photos, no automatic generation
  expect(readyWrites).toEqual([]); expect(readyRouting.assetRequests).toEqual([]); expect(counts()).toBe(after);
  expect(fixture.paidSubmissions()).toBe(0); expect(fixture.counts.manual).toBe(0); await fresh.close();
});

test("PC3-004: stop waits for in-flight PUT, keeps pages, resumes one missing page; concurrent seal only recovers by read",async({page})=>{
  const seedData=await seed("STOP"); await open(page,seedData);
  let release!:()=>void; const gate=new Promise<void>(resolve=>{release=resolve;}); let entered!:()=>void; const held=new Promise<void>(resolve=>{entered=resolve;});
  await page.route(`**/preparations/${seedData.prepId}/pages/2`,async route=>{entered(); await gate; await route.fallback();});
  await page.getByTestId("prepare-start").click(); await held;
  await page.getByTestId("prepare-cancel").click(); await expect(page.getByTestId("prepare-cancel")).toHaveText("正在停止并读取进度…"); await expect(page.getByTestId("prepare-start")).toBeHidden();
  release(); await expect(page.getByText("已停止，已完成页已保留。可稍后继续准备。")).toBeVisible();
  expect((await current(seedData)).readiness.completedPages).toEqual([1,2]);
  const puts:string[]=[]; page.on("request",r=>{if(r.method()==="PUT") puts.push(new URL(r.url()).pathname);});
  await page.getByTestId("prepare-start").click(); await expect(page.getByTestId("prepare-seal")).toBeVisible(); expect(puts).toEqual([`/api/v1/preparations/${seedData.prepId}/pages/3`]);
  await page.route(`**/preparations/${seedData.prepId}/complete`,async route=>{
    const detail=await api.get(`${backend.base}/api/v1/preparations/${seedData.prepId}`); const etag=detail.headers()["etag"]; if(!etag)throw new Error("missing ETag");
    const seal=await api.post(`${backend.base}/api/v1/preparations/${seedData.prepId}/complete`,{headers:{"x-csrf-token":csrf,"if-match":etag},data:{pageCount:3}}); expect(seal.status()).toBe(200); await route.fallback();
  });
  await page.getByTestId("prepare-seal").click(); await expect(page.getByRole("alert")).toContainText("记录已更新，请重新读取进度"); await expect(page.getByTestId("prepare-start")).toBeDisabled();
  await page.getByRole("button",{name:"重新读取进度",exact:true}).click(); await expect(page.getByTestId("prepare-sealed")).toBeVisible(); await expect(page.getByTestId("prepare-seal")).toBeHidden();
  expect(sql("SELECT COUNT(*) FROM jobs").trim()).toBe("0"); expect(sql("SELECT COUNT(*) FROM cost_ledger").trim()).toBe("0"); expect(fixture.counts).toEqual({upload:0,submit:0,task:0,manual:0,cdn:0});
  fs.writeFileSync(path.join(evidence,"stop-conflict-counts.json"),JSON.stringify({storedPages:(await current(seedData)).readiness.completedPages,paidCalls:fixture.counts,counts:JSON.parse(counts())}));
});

test("PC3-003/005: discovery failure cannot create, retry restores facts, incompatible record is retained on explicit restart",async({page})=>{
  const seedData=await seed("RETRY"); let fail=true;
  await installRealBackendRouting(page,backend.base);
  await page.route(`**/documents/${seedData.docId}/preparations?*`,async route=>{if(fail)await route.fulfill({status:500,contentType:"application/json",body:JSON.stringify({error:{code:"INTERNAL_ERROR",message:"fixture discovery failure",details:null,requestId:"fixture"}})});else await route.fallback();});
  await loginViaUi(page,web,BACKEND_PASSWORD); await page.goto(`${web}/items/${seedData.itemId}/import/prepare?documentId=${seedData.docId}`);
  await expect(page.getByRole("heading",{name:"准备记录读取失败"})).toBeVisible(); await expect(page.getByTestId("prepare-start")).toBeHidden(); await expect(page.getByText("当前原件尚无准备记录。点击开始后才会读取并制作页资料。")).toBeHidden();
  fail=false; await page.getByRole("button",{name:"重新读取",exact:true}).click(); await expect(page.getByTestId("prepare-progress")).toContainText("已完成 1 页");
  sql(`UPDATE preparations SET format_version=2 WHERE id='${seedData.prepId}'`); await page.reload();
  await expect(page.getByTestId("prepare-restart")).toBeVisible(); await page.locator(".preparation-discovery summary").click(); await expect(page.getByText("准备格式不受支持，请重新准备；旧记录保留。")).toBeVisible();
  const original=sql(`SELECT state || ':' || revision || ':' || format_version FROM preparations WHERE id='${seedData.prepId}'`);
  await page.getByTestId("prepare-restart").click(); await expect(page.getByTestId("prepare-seal")).toBeVisible();
  expect(sql(`SELECT state || ':' || revision || ':' || format_version FROM preparations WHERE id='${seedData.prepId}'`)).toBe(original);
  expect(sql(`SELECT count(*) FROM preparations WHERE document_id='${seedData.docId}'`).trim()).toBe("2");
});

test("PC3-003: changing document rejects foreign hint and ignores delayed old discovery",async({page})=>{
  const seedData=await seed("FIRST");
  const upload=await api.post(`${backend.base}/api/v1/items/${seedData.itemId}/assets`,{headers:{"x-csrf-token":csrf},multipart:{purpose:"document",file:{name:"second.pdf",mimeType:"application/pdf",buffer:pdf("SECOND")}}});
  const asset=(await upload.json()).data; const doc=await post(`/api/v1/items/${seedData.itemId}/documents`,{title:"SECOND",sourceAssetId:asset.id});
  const alternative=await post(`/api/v1/documents/${seedData.docId}/preparations`,{sourceSha256:seedData.sha,createNew:true});
  await open(page,seedData); await expect(page.getByTestId("prepare-progress")).toContainText("已完成 1 页");
  await page.locator(".preparation-discovery summary").click();
  await page.locator(`input[value="${alternative.id}"]`).check();
  await expect(page.getByTestId("prepare-progress")).toHaveText("已完成 0 页，总页数待读取原件");
  await page.locator(`input[value="${seedData.prepId}"]`).check();
  await expect(page.getByTestId("prepare-progress")).toContainText("已完成 1 页");
  await page.evaluate(({item,prep})=>sessionStorage.setItem(`em.prepare.${item}`,prep),{item:seedData.itemId,prep:seedData.prepId});
  let release!:()=>void; const gate=new Promise<void>(resolve=>{release=resolve;}); let entered!:()=>void; const held=new Promise<void>(resolve=>{entered=resolve;});
  await page.route(`**/documents/${seedData.docId}/preparations?*`,async route=>{entered();await gate;await route.fallback();});
  await page.reload(); await held;
  await page.getByLabel("所选原件").selectOption(doc.id);
  await expect(page.getByText("当前原件尚无准备记录。点击开始后才会读取并制作页资料。")).toBeVisible();
  await expect(page.getByText("之前的记录不适用于当前原件；以下以服务端读取结果为准。")).toBeVisible();
  release(); await expect(page.getByLabel("所选原件")).toHaveValue(doc.id); await expect(page.getByTestId("prepare-progress")).toBeHidden();
  await expect(page.getByTestId("prepare-start")).toHaveText("开始准备");
  expect((await current(seedData)).readiness.completedPages).toEqual([1]);
});

test("BUG-PC3-001: native radio arrows preserve focus and expansion during pending detail reads",async({page})=>{
  await page.setViewportSize({width:375,height:812});
  const seeded=await seed("KEYBOARD");
  const detail=await api.get(`${backend.base}/api/v1/preparations/${seeded.prepId}`);
  const etag=detail.headers()["etag"]; if(!etag)throw new Error("missing ETag");
  const seal=await api.post(`${backend.base}/api/v1/preparations/${seeded.prepId}/complete`,{headers:{"x-csrf-token":csrf,"if-match":etag},data:{pageCount:1}});
  expect(seal.status()).toBe(200);
  const alternative=await post(`/api/v1/documents/${seeded.docId}/preparations`,{sourceSha256:seeded.sha,createNew:true});
  const before=counts(); const writes:string[]=[];
  page.on("request",r=>{if(["PUT","POST"].includes(r.method())&&!r.url().includes("/auth/"))writes.push(r.url());});
  await open(page,seeded); await expect(page.getByTestId("prepare-sealed")).toBeVisible();
  const records=page.locator(".preparation-discovery"); const disclosure=records.locator("details");
  await disclosure.locator("summary").click();
  const recommended=records.locator(`input[value="${seeded.prepId}"]`);
  const other=records.locator(`input[value="${alternative.id}"]`);
  const node=await other.elementHandle(); if(!node)throw new Error("missing alternative radio");
  let release!:()=>void; const gate=new Promise<void>(resolve=>{release=resolve;});
  let entered!:()=>void; const held=new Promise<void>(resolve=>{entered=resolve;});
  await page.route(`**/preparations/${alternative.id}`,async route=>{entered();await gate;await route.fallback();});
  await recommended.focus(); await recommended.press("ArrowDown"); await held;
  await expect(other).toBeChecked(); await expect(other).toBeFocused();
  expect(await node.evaluate(el=>el.isConnected)).toBe(true);
  await expect(disclosure).toHaveAttribute("open","");
  await expect(page.getByTestId("prepare-start")).toBeHidden();
  await expect(page.getByTestId("prepare-seal")).toBeHidden();
  release(); await expect(page.getByTestId("prepare-progress")).toHaveText("已完成 0 页，总页数待读取原件");
  await expect(other).toBeFocused(); expect(await node.evaluate(el=>el.isConnected)).toBe(true);
  await other.press("ArrowUp"); await expect(recommended).toBeFocused(); await expect(recommended).toBeChecked();
  await expect(page.getByTestId("prepare-sealed")).toBeVisible();
  await expect(disclosure).toHaveAttribute("open","");
  expect(writes).toEqual([]); expect(counts()).toBe(before); expect(fixture.paidSubmissions()).toBe(0);
});

test("T09 regression: scan/rotated/CJK use real local PDF assets; encrypted and oversized PDFs create no preparation",async({page})=>{
  const routing=await installRealBackendRouting(page,backend.base); await loginViaUi(page,web,BACKEND_PASSWORD);
  for(const name of ["sample-manual-scan.pdf","sample-manual-rotated.pdf","sample-manual-nonlatin.pdf","sample-manual-encrypted.pdf","sample-manual-many-pages.pdf"]) {
    const seeded=await seedItemWithDocument(api,backend.base,BACKEND_PASSWORD,name,name); csrf=await apiLogin(api,backend.base,BACKEND_PASSWORD);
    const before=counts(); await page.goto(`${web}/items/${seeded.itemId}/import/prepare`);
    await page.getByTestId("prepare-start").click();
    if(name.includes("encrypted") || name.includes("many-pages")) {
      await expect(page.getByText(name.includes("encrypted") ? "该 PDF 已加密，首版不支持，请先解除加密后再上传" : "PDF 共 101 页，超过 100 页上限")).toBeVisible();
      await expect(page.getByTestId("prepare-cancel")).toBeHidden();
      expect(counts()).toBe(before); continue;
    }
    await expect(page.getByTestId("prepare-seal")).toBeVisible();
    const result=await api.get(`${backend.base}/api/v1/documents/${seeded.documentId}/preparations`); const prepId=(await result.json()).recommendedPreparationId;
    const detail=await fetchPreparation(api,backend.base,prepId);
    expect(detail.pages.length).toBe(name.includes("nonlatin")?1:2);
    for(const record of detail.pages) {
      expect(record.imageAssetId).not.toBeNull(); expect(record.viewport).not.toBeNull();
      expect(Math.max(record.viewport!.width,record.viewport!.height)).toBeLessThanOrEqual(2000);
      if(name.includes("scan"))expect(record.textAssetId).toBeNull();
    }
    if(name.includes("rotated")) {
      expect(detail.pages[1]?.viewport?.rotation).toBe(90); expect(detail.pages[1]!.viewport!.width).toBeGreaterThan(detail.pages[1]!.viewport!.height);
      expect((await fetchAsset(api,backend.base,detail.pages[1]!.textAssetId!)).bytes.toString("utf8")).toContain("ROTATE-PAGE-TWO");
    }
    if(name.includes("nonlatin")) expect((await fetchAsset(api,backend.base,detail.pages[0]!.textAssetId!)).bytes.toString("utf8")).toContain("部件一：松开四颗螺丝");
  }
  expect(routing.external).toEqual([]); expect(fixture.counts).toEqual({upload:0,submit:0,task:0,manual:0,cdn:0});
});
