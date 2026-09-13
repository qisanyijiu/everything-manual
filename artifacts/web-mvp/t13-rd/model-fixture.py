#!/usr/bin/env python3
"""T13 手工冒烟用**本机 fixture**（只绑定 127.0.0.1；零真实外网调用）。

覆盖两条路径：
1. `提交 → 成功 → 下载 → 校验 → 不可变 revision`：Tripo 端点 + 模型 CDN 端点都在本机；
2. `链接过期 → 重新查询已知任务取新链接 → 成功`：第一次 `GET /model.glb` 返回 403（签名链接
   过期），随后 `GET /v3/tasks/<id>` 返回**新**的 `model_url`（`/model-v2.glb`），下载成功。

行为：
- `POST /v3/files`：multipart 上传 → `{"code":0,"data":{"file_token":"<n>"}}`；
- `POST /v3/generation/multiview-to-model`：返回固定 `task_id`（**付费提交计数**从记录里读）；
- `GET /v3/tasks/<id>`：第 1 次 success + 过期链接；之后 success + 新链接；
- `GET /model.glb`：403（过期）；
- `GET /model-v2.glb`：样例 GLB 字节（`--glb` 指定）。

每个请求把 `方法 路径?查询 关键请求头(Authorization 只记是否存在) 计数` 记录到 `--log`
（JSON Lines），供人工核对"付费 POST 一次、CDN 请求不带 Authorization、链接只按查询刷新"。

用法：model-fixture.py --port 18312 --log requests.jsonl --glb <path/to/sample-model.glb>
"""

import argparse
import json
import re
import socketserver
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TASK_ID = "manual-task-t13-0001"


class FixtureServer(ThreadingHTTPServer):
    """跳过 `HTTPServer.server_bind` 里的 `socket.getfqdn`（受限网络下可能挂起）。"""

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        host, port = self.server_address[:2]
        self.server_name = host
        self.server_port = port


class Handler(BaseHTTPRequestHandler):
    server_version = "t13-fixture"

    def log_message(self, fmt, *args):
        pass

    def _record(self, note, extra=None):
        entry = {
            "method": self.command,
            "path": self.path,
            "authorization": "authorization" in {k.lower() for k in self.headers.keys()},
            "note": note,
        }
        if extra:
            entry.update(extra)
        with open(self.server.log_path, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(entry, ensure_ascii=False) + "\n")

    def _send_json(self, payload, status=200):
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _send_bytes(self, body, status=200, content_type="model/gltf-binary"):
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        raw = self.rfile.read(length) if length else b""
        if self.path == "/v3/files":
            self.server.upload_count += 1
            self._record("upload", {"count": self.server.upload_count})
            self._send_json(
                {"code": 0, "data": {"file_token": f"token-{self.server.upload_count}"}}
            )
            return
        if self.path == "/v3/generation/multiview-to-model":
            self.server.submit_count += 1
            self._record(
                "paid-submit",
                {"count": self.server.submit_count, "body": json.loads(raw or b"{}")},
            )
            self._send_json({"code": 0, "data": {"task_id": TASK_ID}})
            return
        self._record("unexpected-post")
        self._send_json({"code": 1, "message": "unexpected"}, status=404)

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path.startswith("/v3/tasks/"):
            self.server.poll_count += 1
            expired = f"http://127.0.0.1:{self.server.server_port}/model.glb?sign=expired-old"
            fresh = f"http://127.0.0.1:{self.server.server_port}/model-v2.glb?sign=fresh-new"
            url = expired if self.server.poll_count == 1 else fresh
            self._record("task-query", {"count": self.server.poll_count, "modelUrl": url})
            self._send_json(
                {
                    "code": 0,
                    "data": {
                        "task_id": TASK_ID,
                        "status": "success",
                        "progress": 100,
                        "credits_consumed": 30,
                        "output": {
                            "model_url": url,
                            "rendered_image_url": "https://cdn.example.invalid/preview.png",
                        },
                    },
                }
            )
            return
        if path == "/model.glb":
            self._record("model-download-expired")
            self._send_bytes(b'{"error":"link expired"}', status=403, content_type="application/json")
            return
        if path == "/model-v2.glb":
            self._record("model-download-fresh")
            with open(self.server.glb_path, "rb") as handle:
                self._send_bytes(handle.read())
            return
        self._record("unexpected-get")
        self._send_json({"error": "not found"}, status=404)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--glb", required=True)
    args = parser.parse_args()

    server = FixtureServer(("127.0.0.1", args.port), Handler)
    assert server.server_address[0] == "127.0.0.1", "fixture 只允许回环监听"
    server.log_path = args.log
    server.glb_path = args.glb
    server.upload_count = 0
    server.submit_count = 0
    server.poll_count = 0
    print(f"listening on 127.0.0.1:{args.port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    sys.exit(main())
