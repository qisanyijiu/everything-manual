import os
import json,urllib.request
KEY=os.environ["EM_LLM_API_KEY"]  # 不在仓库里保存任何密钥
def ask_text(prompt,schema,model="gpt-5.5",max_tokens=8000):
    body={"model":model,"input":[{"role":"user","content":[{"type":"input_text","text":prompt}]}],
          "text":{"format":{"type":"json_schema","name":"out","strict":True,"schema":schema}},"max_output_tokens":max_tokens,"store":False}
    r=urllib.request.Request("https://lumina.tripo3d.com/v1/responses",data=json.dumps(body).encode(),headers={"Authorization":"Bearer "+KEY,"Content-Type":"application/json"})
    d=json.load(urllib.request.urlopen(r,timeout=400))
    return json.loads("".join(c.get("text","") for o in d.get("output",[]) for c in (o.get("content") or []) if c.get("type")=="output_text"))
