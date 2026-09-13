#!/usr/bin/env python3
"""T12 手工冒烟用**本机 Tripo fixture**（只绑定 127.0.0.1；零真实外网调用）。

行为（脚本化的最简形态，字节协议与 tests/fixtures/responses/tripo/*.json 一致）：
- ``POST /v3/files``：multipart 上传 → ``{"code":0,"data":{"file_token":"<n>-<filename>"}}``
- ``POST /v3/generation/multiview-to-model``：返回固定 ``task_id``，并把请求体 JSON
  写到 ``--log`` 指定的文件（供人工核对 inputs/model/参数，**不会**转发到任何外部地址）
- ``GET /v3/tasks/<id>``：前 ``--running-polls`` 次返回 ``running``，之后返回 ``success``
  （``output.model_url`` 指向 .invalid 域，附带 ``credits_consumed: 30``）

用法：tripo-fixture.py --port 18212 --log requests.log [--running-polls 1]
"""

import argparse
import json
import re
import socketserver
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class FixtureServer(ThreadingHTTPServer):
    """跳过 `HTTPServer.server_bind` 里的 `socket.getfqdn`。

    受限网络下 getfqdn 可能长时间挂起（本机实测约 10s+），期间套接字已 bind 但
    尚未 listen —— 连接会被内核静默丢弃，看起来像"连接超时"。冒烟要可复现，
    因此这里不解析主机名（fixture 只服务 127.0.0.1）。
    """

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        host, port = self.server_address[:2]
        self.server_name = host
        self.server_port = port


class Handler(BaseHTTPRequestHandler):
    server_version = "t12-fixture"

    def log_message(self, fmt, *args):  # 静默：原始记录写到 --log 文件
        pass

    def _record(self, note):
        with open(self.server.log_path, "a", encoding="utf-8") as handle:
            handle.write(note + "\n")

    def _send(self, payload, status=200):
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        raw = self.rfile.read(length) if length else b""
        if self.path == "/v3/files":
            match = re.search(rb'filename="([^"]*)"', raw)
            name = match.group(1).decode("utf-8", "replace") if match else "unknown"
            self.server.upload_count += 1
            token = f"fixture-token-{self.server.upload_count}-{name}"
            self._record(f"POST /v3/files filename={name} bytes={len(raw)} → token={token}")
            self._send({"code": 0, "data": {"file_token": token}})
            return
        if self.path == "/v3/generation/multiview-to-model":
            body = json.loads(raw.decode("utf-8"))
            params = {key: body.get(key) for key in (
                "model", "texture", "pbr", "texture_quality", "geometry_quality",
                "face_limit", "quad", "generate_parts")}
            self._record(
                "POST /v3/generation/multiview-to-model "
                f"inputs={json.dumps(body.get('inputs'), ensure_ascii=False)} "
                f"params={json.dumps(params, ensure_ascii=False)}"
            )
            if any(key in body for key in ("model_version", "files", "type")):
                self._record("!! 检测到 v2 字段形态（不应出现）")
            self._send({"code": 0, "data": {"task_id": self.server.task_id}})
            return
        self._send({"code": 4040, "message": f"unknown path {self.path}"}, status=404)

    def do_GET(self):
        if self.path.startswith("/v3/tasks/"):
            task_id = self.path[len("/v3/tasks/"):]
            self.server.poll_count += 1
            if self.server.poll_count <= self.server.running_polls:
                self._record(f"GET /v3/tasks/{task_id} → running")
                self._send({"code": 0, "data": {
                    "task_id": task_id, "status": "running", "progress": 40, "output": None,
                }})
                return
            self._record(f"GET /v3/tasks/{task_id} → success（credits_consumed=30）")
            self._send({"code": 0, "data": {
                "task_id": task_id,
                "status": "success",
                "progress": 100,
                "credits_consumed": 30,
                "output": {
                    "model_url": "https://cdn.example.invalid/fixture/smoke-model.glb?sign=smoke-signature",
                    "rendered_image_url": "https://cdn.example.invalid/fixture/smoke-preview.png",
                },
            }})
            return
        self._send({"code": 4040, "message": f"unknown path {self.path}"}, status=404)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--running-polls", type=int, default=1)
    args = parser.parse_args()

    server = FixtureServer(("127.0.0.1", args.port), Handler)
    server.log_path = args.log
    server.task_id = "smoke-task-0001"
    server.upload_count = 0
    server.poll_count = 0
    server.running_polls = args.running_polls
    print(f"fixture listening on 127.0.0.1:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    sys.exit(main())
