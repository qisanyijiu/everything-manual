"""一键：说明书 PDF + 视图 → 生成 → 分件 → 自动热点绑定 → 交互 → （可选）确认并发布。

用法：python run_manual.py <pentax|cyberdog> [--publish]
环境：EM_BASE（默认 http://127.0.0.1:8080/api/v1）、EM_PASSWORD、EM_LLM_API_KEY、TRIPO_API_KEY、HTTPS_PROXY（可选）、
      EM_WORK（工作目录，默认 /tmp/emdemo）。视图与视觉定位点来自 $EM_WORK/views.json。
说明：生成会产生真实费用（Tripo 约 30 credits + 分件 40 credits；说明书 AI 按页数）。
"""
import json, os, sys, time, uuid, urllib.request, http.cookiejar
import pypdfium2 as pdfium
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__)); sys.path.insert(0, HERE)
import tripo
from bindlib import load_seg
from align import drawing_mask, find_view, rot, pick_points, snap_missing
from llm import ask_text

BASE = os.environ.get("EM_BASE", "http://127.0.0.1:8080/api/v1")
WORK = os.environ.get("EM_WORK", "/tmp/emdemo")
MANUALS = {
    "pentax": {"pdf": "/Users/wault/Downloads/pentax17_om_sc_web.pdf", "item": {"name": "胶片相机", "brand": "PENTAX", "model": "PENTAX 17"},
               "title": "PENTAX 17 使用说明书", "legend_pages": [9, 10], "photos": {"front": "px_front", "back": "px_back"}},
    "cyberdog": {"pdf": "/Users/wault/Downloads/CyberDog 2 产品说明书.pdf", "item": {"name": "四足机器人", "brand": "Xiaomi", "model": "CyberDog 2"},
                 "title": "CyberDog 2 产品说明书", "legend_pages": [1], "photos": {"front": "cd_front", "back": "cd_back"}},
}

class Api:
    def __init__(self):
        self.jar = http.cookiejar.CookieJar(); self.csrf = ""
        self.op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar), urllib.request.ProxyHandler({}))
    def call(self, method, path, body=None, raw=None, ctype=None, headers=None, ok=True):
        h = {"X-CSRF-Token": self.csrf, **(headers or {})}; data = None
        if raw is not None: data = raw; h["Content-Type"] = ctype
        elif body is not None: data = json.dumps(body).encode(); h["Content-Type"] = "application/json"
        r = urllib.request.Request(BASE + path, data=data, method=method, headers=h)
        try:
            resp = self.op.open(r, timeout=300); txt = resp.read(); return resp.status, (json.loads(txt) if txt else None), resp.headers.get("ETag")
        except urllib.error.HTTPError as e:
            txt = e.read()
            if ok: raise SystemExit(f"{method} {path} → {e.code}: {txt[:600]!r}")
            return e.code, json.loads(txt) if txt else None, e.headers.get("ETag")
    def login(self):
        _, j, _ = self.call("POST", "/auth/login", {"password": os.environ["EM_PASSWORD"]}); self.csrf = j["data"]["csrfToken"]
    def multipart(self, fields, file_field, filename, data, mime):
        b = "----em" + uuid.uuid4().hex; parts = []
        for k, v in fields.items(): parts.append(f'--{b}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode())
        parts.append(f'--{b}\r\nContent-Disposition: form-data; name="{file_field}"; filename="{filename}"\r\nContent-Type: {mime}\r\n\r\n'.encode() + data + b"\r\n")
        parts.append(f"--{b}--\r\n".encode()); return b"".join(parts), "multipart/form-data; boundary=" + b
    def upload(self, item, purpose, path, mime):
        raw, ct = self.multipart({"purpose": purpose}, "file", os.path.basename(path), open(path, "rb").read(), mime)
        return self.call("POST", f"/items/{item}/assets", raw=raw, ctype=ct)[1]["data"]["id"]

def log(*a): print(time.strftime("%H:%M:%S"), *a, flush=True)

def prepare(api, cfg, wd):
    pdf = pdfium.PdfDocument(cfg["pdf"]); meta = []
    for i in range(len(pdf)):
        p = pdf[i]; w, h = p.get_size(); img = p.render(scale=2000 / max(w, h)).to_pil().convert("RGB"); img.save(f"{wd}/page-{i+1}.jpg", quality=85)
        open(f"{wd}/page-{i+1}.txt", "w").write(p.get_textpage().get_text_bounded()); meta.append((i + 1, img.size))
    _, j, _ = api.call("POST", "/items", cfg["item"]); item = j["data"]["id"]
    a = api.upload(item, "document", cfg["pdf"], "application/pdf")
    _, j, _ = api.call("POST", f"/items/{item}/documents", {"sourceAssetId": a, "title": cfg["title"]}); doc = j["data"]
    _, j, _ = api.call("POST", f"/documents/{doc['id']}/preparations", {"sourceSha256": doc["sourceSha256"]}); prep = j["data"]["id"]
    for n, (W, H) in meta:
        img = api.upload(item, "pageImage", f"{wd}/page-{n}.jpg", "image/jpeg"); txt = api.upload(item, "pageText", f"{wd}/page-{n}.txt", "text/plain")
        api.call("PUT", f"/preparations/{prep}/pages/{n}", {"textAssetId": txt, "imageAssetId": img, "viewport": {"width": W, "height": H, "rotation": 0}})
    _, _, et = api.call("GET", f"/preparations/{prep}")
    api.call("POST", f"/preparations/{prep}/complete", {"pageCount": len(meta)}, headers={"If-Match": et})
    photos = []
    for view, name in cfg["photos"].items():
        aid = api.upload(item, "photo", f"{WORK}/photos/{name}.jpg", "image/jpeg")
        photos.append(api.call("POST", f"/items/{item}/photos", {"assetId": aid, "view": view})[1]["data"]["id"])
    log(f"prepared {len(meta)} pages, {len(photos)} photos")
    return item, prep, photos

def generate(api, item, prep, photos):
    _, j, _ = api.call("POST", f"/items/{item}/estimates", {"preparationId": prep, "photoIds": photos, "modelPreset": "tripo-h-v3.1-standard"}); q = j["data"]
    log("quote", q["amounts"]["tripo"]["upperBoundDisplay"], q["amounts"]["manualAi"]["upperBoundDisplay"])
    api.call("POST", f"/items/{item}/estimates/{q['id']}/confirm")
    lim = {"tripoCreditMinor": q["amounts"]["tripo"]["upperBoundMinor"], "manualAiUsdMicros": q["amounts"]["manualAi"]["upperBoundMinor"]}
    _, j, _ = api.call("POST", f"/items/{item}/jobs", {"quoteId": q["id"], "preparationId": prep, "photoIds": photos, "limits": lim}, headers={"Idempotency-Key": str(uuid.uuid4())})
    job = j["data"]["id"]; log("job", job)
    for _ in range(360):
        _, j, et = api.call("GET", f"/jobs/{job}"); d = j["data"]
        if d["status"] == "succeeded": return job, d["draftId"]
        if d["status"] in ("failed", "cancelled"): raise SystemExit(f"job {d['status']}")
        for s in d["stages"]:
            if s["status"] == "submission_unknown":   # 网关超时：按对账流程授权重发（可能重复计费，已在 ADR 记录）
                _, _, et = api.call("GET", f"/jobs/{job}")
                api.call("POST", f"/jobs/{job}/reconcile", {"stageId": s["id"], "action": "authorizeReplacement", "acknowledgeDuplicateRisk": True, "limits": lim}, headers={"If-Match": et}); log("reconciled batch", s["batchIndex"])
            elif s["status"] == "needs_input" and s["retry"]["allowed"]:
                _, _, et = api.call("GET", f"/jobs/{job}")
                api.call("POST", f"/jobs/{job}/retry", {"stageId": s["id"]}, headers={"If-Match": et, "Idempotency-Key": str(uuid.uuid4())}, ok=False)
        time.sleep(10)
    raise SystemExit("job timeout")

def tripo_task_id(job):
    import sqlite3
    db = sqlite3.connect(f"file:{os.environ.get('EM_DB', '/Users/wault/Work/tripothon/everything-manual/var/dev/manual.sqlite3')}?mode=ro", uri=True)
    row = db.execute("select a.remote_task_id from provider_attempts a join job_stages s on s.id=a.stage_id where a.job_id=? and s.stage_kind='tripo_submit' and a.submit_state='accepted'", (job,)).fetchone()
    return row[0]

def segment_and_attach(api, item, draft, job, wd):
    tid = tripo_task_id(job)
    d = tripo.call("POST", "/mesh/segment", {"model": "v2.0-20260430", "input": tid, "segmentation_granularity": "detailed", "split_by_connectivity": True})
    r = tripo.wait(d["data"]["task_id"], 1500); seg = f"{wd}/parts.glb"; tripo.download(r["data"]["output"]["model_url"], seg)
    log("segmented", r["data"].get("credits_consumed"), "credits")
    _, _, et = api.call("GET", f"/items/{item}/drafts/{draft}")
    raw, ct = api.multipart({"source": "tripo:mesh_segment v2.0-20260430"}, "file", "parts.glb", open(seg, "rb").read(), "model/gltf-binary")
    _, j, _ = api.call("POST", f"/items/{item}/drafts/{draft}/parts-model", raw=raw, ctype=ct, headers={"If-Match": et})
    log("parts attached:", len(j["data"]["knowledge"]["interactive"]["partsModel"]["nodeNames"]), "nodes"); return seg

def read_legend(cfg):
    """图例优先取 PDF 文字层；文字层里没有编号条目时（图例是图片的一部分）改用视觉模型读图。"""
    import re
    from vision import ask
    pdf = pdfium.PdfDocument(cfg["pdf"]); texts = []
    for i in cfg["legend_pages"]:
        text = pdf[i].get_textpage().get_text_bounded()
        if len(re.findall(r"(?m)^\s*\d{1,2}[\.、\s]", text)) >= 3:
            texts.append(text); continue
        p = pdf[i]; w, h = p.get_size(); path = f"{WORK}/legend-page-{i+1}.png"
        p.render(scale=2400 / max(w, h)).to_pil().convert("RGB").save(path)
        schema = {"type": "object", "additionalProperties": False, "required": ["entries"], "properties": {"entries": {"type": "array", "items": {"type": "object", "additionalProperties": False, "required": ["number", "name"], "properties": {"number": {"type": "integer"}, "name": {"type": "string"}}}}}}
        r = ask(path, "这是产品说明书的一页，图上有部件编号图例（编号 + 部件名称的列表）。逐条转录图例：number=编号，name=部件名称（原文）。只输出图例条目。", schema)
        texts.append("\n".join(f"{e['number']}. {e['name']}" for e in r["entries"]))
        log(f"legend page {i+1}: text layer empty, vision read {len(r['entries'])} entries")
    return "\n".join(texts)

def legend_map(api, item, draft, cfg):
    _, j, _ = api.call("GET", f"/items/{item}/drafts/{draft}"); K = j["data"]["knowledge"]["knowledge"]
    legend = read_legend(cfg)
    parts = [{"id": p["id"], "name": p["name"], "description": p["description"][:80]} for p in K["parts"]]
    schema = {"type": "object", "additionalProperties": False, "required": ["callouts"], "properties": {"callouts": {"type": "array", "items": {"type": "object", "additionalProperties": False, "required": ["number", "label", "partId"], "properties": {"number": {"type": "integer"}, "label": {"type": "string"}, "partId": {"type": ["string", "null"]}}}}}}
    prompt = ("下面是产品说明书的部件图例原文（编号 + 名称），以及从说明书抽取出的部件条目（id/name/description）。对图例中每个编号：label=图例名称；partId=最匹配的部件条目 id"
              "（必须来自列表；只匹配机身上的实体部件；同名多条选最贴近实体部件的一条；没有对应条目则为 null，不要勉强匹配包装/配件/安全条目）。\n【图例原文】\n" + legend + "\n【部件条目】\n" + json.dumps(parts, ensure_ascii=False))
    return ask_text(prompt, schema)["callouts"], K

def bind(seg, views):
    mesh, nof = load_seg(seg); out = []
    for v in views:
        img = Image.open(v["image"]).convert("RGB"); pts = [(c["x"], c["y"]) for c in v["points"]]
        dm, dbox = drawing_mask(img); best = find_view(mesh, dm, [tuple(g) for g in v["grid"]] if v.get("grid") else None); Rm = rot(*best[1])
        hits = snap_missing(mesh, Rm, dbox, pts, pick_points(mesh, Rm, dbox, pts, nof), nof)
        log(f"view fit {best[1]} IoU={best[0]:.3f}, hits {sum(h is not None for h in hits)}/{len(hits)}")
        out += [{"number": c["number"], "hit": h} for c, h in zip(v["points"], hits)]
    return out

def node_bounds(seg):
    import trimesh
    sc = trimesh.load(seg); out = {}
    for n in sc.graph.nodes_geometry:
        T, g = sc.graph[n]; m = sc.geometry[g].copy(); m.apply_transform(T); out[n] = (m.bounds[0].tolist(), m.bounds[1].tolist())
    return out

def main():
    key = sys.argv[1]; publish = "--publish" in sys.argv; cfg = MANUALS[key]
    wd = f"{WORK}/{key}"; os.makedirs(wd, exist_ok=True)
    api = Api(); api.login()
    if "--resume" in sys.argv:   # 复用已生成的草稿与分件（不重新生成、不重复计费），只重做绑定与交互
        prev = json.load(open(f"{wd}/state.json")); item, draft, job = prev["item"], prev["draft"], prev["job"]; seg = f"{wd}/parts.glb"
        if prev.get("release"):
            raise SystemExit("该草稿已发布；发布版本不可变，请对新的生成重做绑定")
    else:
        item, prep, photos = prepare(api, cfg, wd)
        job, draft = generate(api, item, prep, photos); log("draft", draft)
        json.dump({"item": item, "draft": draft, "job": job}, open(f"{wd}/state.json", "w"))
        seg = segment_and_attach(api, item, draft, job, wd)
    callouts, K = legend_map(api, item, draft, cfg)
    num2part = {c["number"]: c["partId"] for c in callouts if c["partId"]}; label = {c["partId"]: c["label"] for c in callouts if c["partId"]}
    log(f"legend → parts: {len(num2part)}/{len(callouts)}")
    hits = bind(seg, json.load(open(f"{WORK}/views.json"))[key])
    _, j, et = api.call("GET", f"/items/{item}/drafts/{draft}"); model = j["data"]["knowledge"]["model"]
    ups, nodes = [], {}
    for h in hits:
        pid = num2part.get(h["number"]); hit = h["hit"]
        if not pid or not hit: continue
        nodes.setdefault(pid, [])
        if hit["node"] not in nodes[pid]: nodes[pid].append(hit["node"])
        if pid not in [u["partId"] for u in ups]:
            ups.append({"partId": pid, "status": "candidate", "anchor": {"modelRevisionId": model["revisionId"], "modelSha256": model["sha256"], "positionLocal": [round(v, 5) for v in hit["point"]]}})
    bindings = [{"partId": p, "nodes": n, "status": "auto"} for p, n in nodes.items()]
    import interactions
    actions, poses = interactions.build(key, label, nodes, node_bounds(seg), K, seg)
    _, j, _ = api.call("PATCH", f"/items/{item}/drafts/{draft}", {"hotspots": {"upsert": ups}, "interactive": {"bindings": bindings, "actions": actions, "poses": poses}}, headers={"If-Match": et})
    log(f"candidates {len(ups)}, bindings {len(bindings)}, actions {[a['id'] for a in actions]}, poses {[p['id'] for p in poses]}")
    state = {"item": item, "draft": draft, "job": job}
    if publish:
        state["release"] = interactions.confirm_and_publish(api, item, draft); log("published", state["release"])
    json.dump(state, open(f"{wd}/state.json", "w"), indent=1); print(json.dumps(state))

if __name__ == "__main__":
    main()
