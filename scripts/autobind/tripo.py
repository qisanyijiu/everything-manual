import os
import json,urllib.request,sys,time
K=os.environ["TRIPO_API_KEY"]; B="https://openapi.tripo3d.ai/v3"
op=urllib.request.build_opener(urllib.request.ProxyHandler({"https":os.environ["HTTPS_PROXY"]} if os.environ.get("HTTPS_PROXY") else {}))
def call(method,path,body=None):
    r=urllib.request.Request(B+path,data=None if body is None else json.dumps(body).encode(),method=method,headers={"Authorization":"Bearer "+K,"Content-Type":"application/json"})
    try: return json.load(op.open(r,timeout=60))
    except urllib.error.HTTPError as e: return {"http":e.code,"body":e.read().decode()[:500]}
def wait(tid,limit=900):
    t0=time.time()
    while time.time()-t0<limit:
        d=call("GET",f"/tasks/{tid}")
        st=d.get("data",{}).get("status")
        if st in("success","failed","cancelled","banned","expired","unknown"): return d
        time.sleep(8)
    return d
def download(url,fn):
    open(fn,"wb").write(op.open(url,timeout=300).read())
import uuid
def upload(path,mime):
    b="----em"+uuid.uuid4().hex; data=(f'--{b}\r\nContent-Disposition: form-data; name="file"; filename="{path.split("/")[-1]}"\r\nContent-Type: {mime}\r\n\r\n').encode()+open(path,"rb").read()+f"\r\n--{b}--\r\n".encode()
    r=urllib.request.Request(B+"/files",data=data,method="POST",headers={"Authorization":"Bearer "+K,"Content-Type":"multipart/form-data; boundary="+b})
    try: return json.load(op.open(r,timeout=120))
    except urllib.error.HTTPError as e: return {"http":e.code,"body":e.read().decode()[:500]}
