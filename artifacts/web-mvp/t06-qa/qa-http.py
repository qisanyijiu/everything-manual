#!/usr/bin/env python3
"""QA 独立 HTTP 客户端（不用 curl 的 HEAD 包装，直接量 body 长度）。

用法：qa-http.py <method> <url> <cookie|-> [Header: value ...]
输出：一行 JSON 到 stdout：{"status":int,"headers":{...小写...},"bodyLen":int,"bodyFile":path}
body 写到 <url 路径最后一段>.body.qa（当前目录）。
"""
import http.client
import json
import sys
import urllib.parse

method = sys.argv[1]
url = sys.argv[2]
cookie = sys.argv[3]
headers = {}
for item in sys.argv[4:]:
    name, _, value = item.partition(":")
    headers[name.strip()] = value.strip()

parsed = urllib.parse.urlsplit(url)
conn = http.client.HTTPConnection(parsed.hostname, parsed.port, timeout=120)
if cookie and cookie != "-":
    headers["Cookie"] = cookie
conn.request(method, parsed.path, headers=headers)
response = conn.getresponse()
body = response.read()
out = {
    "status": response.status,
    "headers": {k.lower(): v for k, v in response.getheaders()},
    "bodyLen": len(body),
}
if body:
    name = (parsed.path.rstrip("/").rsplit("/", 1)[-1] or "body") + ".body.qa"
    with open(name, "wb") as handle:
        handle.write(body)
    out["bodyFile"] = name
print(json.dumps(out))
conn.close()
