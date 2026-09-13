#!/usr/bin/env python3
"""T12 QA 独立验收用**本机 Tripo fixture**（QA 回合 14 自建；只绑定 127.0.0.1）。

与 RD 的 tripo-fixture.py 相互独立：本文件由 QA 自己实现字节记录与场景控制，
用来复现协议行为（不采信 RD 日志）。所有请求目标只可能是 127.0.0.1。

场景（--scenario）：
- happy          : 上传返回 file_token；提交 1 次成功；查询 running→success（credits_consumed=30，
                   签名串 qa-signature-canary-9931）
- disconnect     : 提交记录请求后**直接断开连接**（无任何响应字节）→ 客户端无法证明未被接受
- unknown        : 查询始终返回未知状态 qa_new_state_zz
- business_error : 提交返回 HTTP 200 + code=1201（业务拒绝）
- no_model       : 查询 success 但 output.model_url=null
- token_unknown  : 上传返回 {token_value}（两个候选字段名都不是）→ 上传必须失败、不得发付费 POST
- submit_429     : 第一次提交返回 429 + Retry-After: 1，第二次成功
- poll_503       : 第一次查询 503，之后 success（查询失败 ≠ 生成失败）
- no_billing     : success 带 model_url 但**无**计费字段（不得猜测金额结算）
- token_verbatim : 上传 token 含首尾空白与大小写混排（必须逐字符透传，不 trim/不截断）

日志：每个请求写一行 JSON 到 --log（含方法/路径/凭据头/体哈希/解析结论）。
"""

import argparse
import hashlib
import json
import re
import socketserver
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SIGNATURE_CANARY = "qa-signature-canary-9931"
TASK_ID = "qa-task-0001"
MODEL_URL = f"https://cdn.example.invalid/qa/model.glb?sign={SIGNATURE_CANARY}"
V2_FORBIDDEN = ("model_version", "files", "type")
SUBMIT_EXPECTED_KEYS = {
    "inputs",
    "model",
    "texture",
    "pbr",
    "texture_quality",
    "geometry_quality",
    "face_limit",
    "quad",
    "generate_parts",
}


def parse_multipart(content_type, body):
    """解析 multipart/form-data；返回 [{'name','filename','content_type','size','sha256','headers'}]。"""
    match = re.search(r'boundary=(?:"([^"]+)"|([^;]+))', content_type or "")
    if not match:
        return None
    boundary = (match.group(1) or match.group(2)).strip()
    parts = []
    for chunk in body.split(b"--" + boundary.encode())[1:]:
        if chunk.startswith(b"--"):
            break
        chunk = chunk.lstrip(b"\r\n")
        if b"\r\n\r\n" not in chunk:
            continue
        head, data = chunk.split(b"\r\n\r\n", 1)
        if data.endswith(b"\r\n"):
            data = data[:-2]
        head_text = head.decode("utf-8", "replace")
        name = re.search(r'name="([^"]*)"', head_text)
        filename = re.search(r'filename="([^"]*)"', head_text)
        part_type = re.search(r"content-type:\s*([^\r\n;]+)", head_text, re.IGNORECASE)
        parts.append(
            {
                "headers": head_text,
                "name": name.group(1) if name else None,
                "filename": filename.group(1) if filename else None,
                "content_type": part_type.group(1).strip() if part_type else None,
                "size": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    return parts


class FixtureServer(ThreadingHTTPServer):
    """只绑定 127.0.0.1；跳过 getfqdn（受限网络下会挂起）。"""

    daemon_threads = True

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        host, port = self.server_address[:2]
        self.server_name = host
        self.server_port = port


class Handler(BaseHTTPRequestHandler):
    server_version = "t12-qa-fixture"
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        pass

    def _record(self, event, **fields):
        entry = {"event": event, **fields}
        with open(self.server.log_path, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(entry, ensure_ascii=False) + "\n")

    def _read_body(self):
        length = int(self.headers.get("content-length", "0") or "0")
        return self.rfile.read(length) if length else b""

    def _send_json(self, payload, status=200, extra_headers=None):
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        for name, value in (extra_headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)

    def _send_404(self):
        self._send_json({"code": 4040, "message": f"unknown path {self.path}"}, status=404)

    def do_POST(self):
        body = self._read_body()
        auth = self.headers.get("authorization")
        if self.path.endswith("/files") and self.path.startswith("/v3/"):
            self._handle_upload(body, auth)
            return
        if self.path == "/v3/generation/multiview-to-model":
            self._handle_submit(body, auth)
            return
        self._send_404()

    def _handle_upload(self, body, auth):
        content_type = self.headers.get("content-type", "")
        parts = parse_multipart(content_type, body)
        self.server.upload_count += 1
        record = {
            "path": self.path,
            "authorization": auth,
            "content_type": content_type,
            "multipart_ok": parts is not None,
            "parts": parts,
        }
        if parts is None:
            self._record("upload_bad_multipart", **record)
            self._send_json({"code": 1001, "message": "not multipart"}, status=400)
            return
        filename = None
        for part in parts:
            if part["name"] == "file":
                filename = part["filename"] or "unknown"
        self._record("upload", **record)
        if self.server.scenario == "token_unknown":
            # 两个候选字段名都不是：适配器必须失败，不得把别的字段当 token。
            self._send_json({"code": 0, "data": {"token_value": "not-a-known-token-field"}})
            return
        if self.server.scenario == "token_verbatim":
            # 含首尾空白与大小写混排的 opaque token：必须逐字符透传（不 trim/不截断）。
            token = f" TOKEN with Spaces_{self.server.upload_count} "
        else:
            token = f"qa-token-{self.server.upload_count}-{filename}"
        self._send_json({"code": 0, "data": {"file_token": token}})

    def _handle_submit(self, body, auth):
        self.server.submit_count += 1
        try:
            parsed = json.loads(body.decode("utf-8"))
        except Exception as error:  # noqa: BLE001 - 诊断用
            parsed = None
            self._record("submit_bad_json", error=str(error))
        self._record(
            "submit",
            path=self.path,
            authorization=auth,
            content_type=self.headers.get("content-type"),
            body_sha256=hashlib.sha256(body).hexdigest(),
            body_text=body.decode("utf-8", "replace"),
            parsed=parsed,
            v2_fields_present=[key for key in V2_FORBIDDEN if isinstance(parsed, dict) and key in parsed],
            unexpected_top_level_keys=(sorted(set(parsed) - SUBMIT_EXPECTED_KEYS) if isinstance(parsed, dict) else None),
        )
        scenario = self.server.scenario
        if scenario == "disconnect":
            # 供应商已收到请求（已记录）但客户端拿不到任何响应字节。
            self.close_connection = True
            try:
                self.connection.shutdown(2)
            except OSError:
                pass
            return
        if scenario == "business_error":
            self._send_json(
                {"code": 1201, "message": "invalid image token", "suggestion": "re-upload front photo"}
            )
            return
        if scenario == "submit_429" and self.server.submit_count == 1:
            self._send_json(
                {"code": 4290, "message": "too many requests"},
                status=429,
                extra_headers={"retry-after": "1"},
            )
            return
        self._send_json({"code": 0, "data": {"task_id": TASK_ID}})

    def do_GET(self):
        if self.path.startswith("/v3/tasks/"):
            task_id = self.path[len("/v3/tasks/"):]
            self.server.poll_count += 1
            self._record(
                "poll",
                path=self.path,
                authorization=self.headers.get("authorization"),
                poll_number=self.server.poll_count,
            )
            scenario = self.server.scenario
            if scenario == "poll_503" and self.server.poll_count == 1:
                self._record("poll_503", poll_number=self.server.poll_count)
                self._send_json(
                    {"code": 5000, "message": "service temporarily unavailable"},
                    status=503,
                )
                return
            if scenario == "unknown":
                self._send_json(
                    {
                        "code": 0,
                        "data": {
                            "task_id": task_id,
                            "status": "qa_new_state_zz",
                            "progress": 55,
                            "output": {},
                        },
                    }
                )
                return
            if scenario == "no_model":
                self._send_json(
                    {
                        "code": 0,
                        "data": {
                            "task_id": task_id,
                            "status": "success",
                            "progress": 100,
                            "output": {"model_url": None},
                        },
                    }
                )
                return
            if scenario == "no_billing":
                self._send_json(
                    {
                        "code": 0,
                        "data": {
                            "task_id": task_id,
                            "status": "success",
                            "progress": 100,
                            "output": {"model_url": MODEL_URL},
                        },
                    }
                )
                return
            if self.server.poll_count <= 1 and scenario in ("happy", "submit_429"):
                self._send_json(
                    {
                        "code": 0,
                        "data": {
                            "task_id": task_id,
                            "status": "running",
                            "progress": 40,
                            "output": None,
                        },
                    }
                )
                return
            if scenario in ("happy", "submit_429", "poll_503", "token_verbatim"):
                self._send_json(
                    {
                        "code": 0,
                        "data": {
                            "task_id": task_id,
                            "status": "success",
                            "progress": 100,
                            "credits_consumed": "30",
                            "output": {"model_url": MODEL_URL},
                        },
                    }
                )
                return
            # disconnect / business_error / token_unknown：理论上不会查询；给 running 以防万一。
            self._send_json(
                {
                    "code": 0,
                    "data": {"task_id": task_id, "status": "running", "progress": 10, "output": None},
                }
            )
            return
        self._send_404()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--scenario", required=True)
    args = parser.parse_args()

    server = FixtureServer(("127.0.0.1", args.port), Handler)
    server.log_path = args.log
    server.scenario = args.scenario
    server.upload_count = 0
    server.submit_count = 0
    server.poll_count = 0
    print(f"fixture listening on 127.0.0.1:{args.port} scenario={args.scenario}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    sys.exit(main())
