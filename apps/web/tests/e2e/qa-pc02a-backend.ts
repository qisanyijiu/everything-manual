import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { expect, type APIRequestContext, type Page } from "@playwright/test";
import { Pc06QaBackend, PASSWORD } from "./qa-pc06-backend";
import { apiLogin, loginViaUi } from "./helpers";
import { REPO_ROOT, fixturePath } from "./runtime";

export const WEB = "http://127.0.0.1:15477";
export const OUT = path.join(REPO_ROOT,process.env.EM_PC02A_QA_OUTPUT ?? "var/pc02a-qa-round3","evidence");
export { PASSWORD };
/** Reuse only isolated process lifecycle; neither provider has a configured key. */
export class Pc02aQaBackend extends Pc06QaBackend {
  constructor() { super(); this.source="toml"; }
  override config() {
    fs.writeFileSync(path.join(this.workDir,"config.toml"),[
      `public_origin = ${JSON.stringify(WEB)}`,
      "[providers.tripo]", 'api_key_env = "EM_PC02A_UNUSED_TRIPO"',
      "[providers.manual_ai]", 'api_key_env = "EM_PC02A_UNUSED_MANUAL"', 'model = "qa-unconfigured"', "",
    ].join("\n"),{mode:0o600});
  }
  counters() { return this.db("SELECT (SELECT count(*) FROM preparations) AS preparations,(SELECT count(*) FROM jobs) AS jobs,(SELECT count(*) FROM provider_attempts) AS attempts,(SELECT count(*) FROM cost_ledger) AS ledger,(SELECT count(*) FROM manual_drafts) AS drafts,(SELECT count(*) FROM manual_releases) AS releases"); }
}
export interface QaDocument { id:string; title:string; sourceAssetId:string; sourceSha256:string; sourceUrl:string|null; }
export function saveEvidence(name:string,value:unknown) { fs.mkdirSync(OUT,{recursive:true}); fs.writeFileSync(path.join(OUT,name+".json"),JSON.stringify(value,null,2)+"\n"); }
export async function item(api:APIRequestContext,b:Pc02aQaBackend,title:string) {
  const csrf=await apiLogin(api,b.base,PASSWORD);
  const response=await api.post(b.base+"/api/v1/items",{headers:{"x-csrf-token":csrf},data:{name:title,model:"QA original reader",brand:"QA"}});
  expect(response.status()).toBe(201); return {id:(await response.json()).data.id as string,csrf};
}
export async function bind(api:APIRequestContext,b:Pc02aQaBackend,itemId:string,csrf:string,title:string,bytes:Buffer,sourceUrl:string|null=null):Promise<QaDocument> {
  const asset=await api.post(`${b.base}/api/v1/items/${itemId}/assets`,{headers:{"x-csrf-token":csrf},multipart:{purpose:"document",file:{name:"qa-source.pdf",mimeType:"application/pdf",buffer:bytes}}});
  expect(asset.status()).toBe(201); const assetId=(await asset.json()).data.id as string;
  const response=await api.post(`${b.base}/api/v1/items/${itemId}/documents`,{headers:{"x-csrf-token":csrf},data:{sourceAssetId:assetId,title,...(sourceUrl?{sourceUrl}:{})}}); expect(response.status()).toBe(201);
  const document=(await response.json()).data as QaDocument; expect(document.sourceSha256).toBe(createHash("sha256").update(bytes).digest("hex")); return document;
}
export async function seed(api:APIRequestContext,b:Pc02aQaBackend) {
  const official=await item(api,b,"PC02A 官方 LACK 原件独立验收");
  const bytes=fs.readFileSync(path.join(REPO_ROOT,"var/manual-samples/ikea-lack-AA-2544914-1.pdf"));
  expect(createHash("sha256").update(bytes).digest("hex")).toBe("963c6e96c0c6773085769764df3aa73f74ffbd2841ef4f7dbc1b4b0269a21a8d");
  const lack=await bind(api,b,official.id,official.csrf,"IKEA LACK 官方说明书（本机 QA）",bytes,"https://www.ikea.com/th/en/assembly_instructions/lack-side-table-white__AA-2544914-1-100.pdf");
  const scanBytes=fs.readFileSync(path.join(REPO_ROOT,"var/manual-samples/roland-tr-808-original-om.pdf"));
  expect(createHash("sha256").update(scanBytes).digest("hex")).toBe("bf5e15408c3aee59fd43135ee51834daaeec6cd54a7fb118984727d15e22e7fb");
  const scan=await bind(api,b,official.id,official.csrf,"Roland TR-808 官方扫描原件（本机 QA）",scanBytes,"https://cdn.roland.com/assets/media/pdf/TR-808_OM.pdf");
  const multi=await item(api,b,"PC02A 同物品多原件合成引用验收");
  const first=await bind(api,b,multi.id,multi.csrf,"QA X100 英文操作原件",fs.readFileSync(fixturePath("sample-manual-text.pdf")));
  const second=await bind(api,b,multi.id,multi.csrf,"QA X100 中文部件补充原件",fs.readFileSync(fixturePath("sample-manual-nonlatin.pdf")));
  // Newest-first document ordering puts both reference targets beyond page one.
  for(let i=0;i<20;i++) {
    const response=await api.post(`${b.base}/api/v1/items/${multi.id}/documents`,{headers:{"x-csrf-token":multi.csrf},data:{sourceAssetId:first.sourceAssetId,title:`QA 分页原件 ${i+1}`}}); expect(response.status()).toBe(201);
  }
  return {officialItemId:official.id,lack,scan,multiItemId:multi.id,first,second};
}
export async function open(page:Page,b:Pc02aQaBackend,route:string) {
  const state={external:0,pageErrors:0,documentQueries:0};
  page.on("pageerror",()=>state.pageErrors++);
  await page.route("**/*",async entry=>{
    const url=new URL(entry.request().url());
    if(["http:","https:"].includes(url.protocol)&&!["127.0.0.1","localhost"].includes(url.hostname)){state.external++;await entry.abort();return;}
    if(url.pathname.endsWith("/documents"))state.documentQueries++;
    await entry.continue({url:url.origin===WEB&&url.pathname.startsWith("/api/v1/")?b.base+url.pathname+url.search:url.href});
  });
  await loginViaUi(page,WEB,PASSWORD); await page.goto(WEB+route); return state;
}
