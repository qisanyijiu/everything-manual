"""自动热点绑定 + 交互定义写回草稿（候选状态，待人工复核）。

输入：视觉定位 + 视角拟合 + 投影得到的命中（*_hits.json）、LLM 图例映射（*_legend_map.json）、
分件包围盒（*_bounds.json）。输出：PATCH hotspots(candidate) + interactive(bindings/actions/poses)。
"""
import json, sys
from em import *

def hotspots_and_bindings(draft, hits_by_view, legend):
    K=draft["knowledge"]; model=K["model"]; num2part={c["number"]:c["partId"] for c in legend["callouts"] if c["partId"]}
    existing={h["partId"]:h for h in K.get("hotspots",[])}
    upserts=[]; nodes={}
    for view in hits_by_view:
        for h in view["hits"]:
            pid=num2part.get(h["number"]); hit=h["hit"]
            if not pid or not hit: continue
            nodes.setdefault(pid,[])
            if hit["node"] not in nodes[pid]: nodes[pid].append(hit["node"])
            if pid in [u["partId"] for u in upserts]: continue
            cur=existing.get(pid)
            if cur and cur["status"]=="confirmed": continue  # 不覆盖人工确认
            u={"partId":pid,"status":"candidate","anchor":{"modelRevisionId":model["revisionId"],"modelSha256":model["sha256"],"positionLocal":[round(v,5) for v in hit["point"]]}}
            if cur: u["id"]=cur["id"]
            upserts.append(u)
    bindings=[{"partId":p,"nodes":n,"status":"auto"} for p,n in nodes.items()]
    return upserts,bindings

def rot(nodes,pivot,axis,angle): return {"nodes":nodes,"kind":"rotate","pivot":pivot,"axis":axis,"angleDeg":angle}
def tr(nodes,v): return {"nodes":nodes,"kind":"translate","vector":v}

def patch(draft_key, body):
    login(); item=S["item"]; did=S[draft_key]
    _,et=ok(req("GET",f"/items/{item}/drafts/{did}"),"draft")
    j,_=ok(req("PATCH",f"/items/{item}/drafts/{did}",body,headers={"If-Match":et}),"patch")
    return j["data"]
