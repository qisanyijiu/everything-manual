import { describe, expect, it } from "vitest";
import { workflowAction } from "./workflow";
import type { ItemSummaryDto } from "../../api/endpoints";
import { wizardStepHref } from "../import/WizardSteps";
const summary:ItemSummaryDto={itemId:"item",action:"confirm",targetId:"quote",latestReleaseId:"release",documentId:"doc",preparationId:"prep",latestQuoteId:"quote",consumedJobId:null,quoteExpiresAt:null,steps:{basic:"complete",document:"complete",views:"complete",prepare:"complete",confirm:"missing"}};
describe("server-selected workflow destinations",()=>{
  it("maps every server action to an internal entity path, keeping input scope",()=>{
    expect(workflowAction(summary).href).toBe("/items/item/import/confirm?documentId=doc&preparationId=prep&quoteId=quote");
    for(const[action,label,path]of[["handleJob","处理任务","/jobs/target"],["viewJob","查看进度","/jobs/target"],["reviewDraft","继续复核","/items/item/drafts/target/review"],["readRelease","阅读说明书","/items/item/releases/target"]]as const){expect(workflowAction({...summary,action,targetId:"target"})).toMatchObject({label,href:path});}
    expect(workflowAction({...summary,action:"prepare"}).href).toBe("/items/item/import/prepare?documentId=doc&preparationId=prep");
    expect(workflowAction({...summary,action:"addViews"}).label).toBe("补齐资料");
    expect(workflowAction({...summary,action:"addDocument"}).status).toBe("待绑定说明书");
  });
  it("wizard navigation retains explicit document/preparation/quote context only",()=>{
    expect(wizardStepHref("item","import/views",new URLSearchParams("documentId=old&preparationId=ready&quoteId=saved&untrusted=https://example.test"))).toBe("/items/item/import/views?documentId=old&preparationId=ready&quoteId=saved");
  });
});
