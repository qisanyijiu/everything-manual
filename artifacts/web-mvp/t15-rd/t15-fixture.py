#!/usr/bin/env python3
"""T15 手工冒烟的本机 fixture：Tripo v3 + 说明书 AI（Responses）+ 模型 CDN 三段合一。

用法：
    python3 t15-fixture.py <port> <log-path> <sample-glb-path> [refuse-once-control-file]

行为（只绑定 127.0.0.1；未知路径一律 501——缺脚本必须失败，不返回通用成功）：
- `POST /v1/responses`：从 `input[0].content[0].text` 解析本批页号（`[第 N 页]`），
  返回 `manual_extract_v1` 的**构造**结果（非官方原文），evidence 只引用本批输入页；
  若给出了控制文件且存在，则**消费它**（删除）并改为返回 refusal（结果未知的失败路径：
  该批不产出正式知识），供"知识分支失败 → 仅重试该分支"演示。
- `POST /v3/files`：multipart 上传 → `{code:0,data:{file_token:...}}`（每次一个序号 token）。
- `POST /v3/generation/multiview-to-model`：付费提交（**唯一计费点**）→ task_id。
- `GET /v3/tasks/<id>`：第 1 次 `running`，之后 `success`（model_url 指向本 fixture 的 CDN）。
- `GET /cdn/model.glb`：样例 GLB 字节。
每次请求写一行 `fixture.log`（方法/路径/摘要），并按路径累计计数（供冒烟脚本断言
"取消后 / 重试时没有新增付费步骤"）。
"""
import json
import re
import socketserver
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

PORT = int(sys.argv[1])
LOG_PATH = sys.argv[2]
GLB_PATH = Path(sys.argv[3])
REFUSE_FILE = Path(sys.argv[4]) if len(sys.argv) > 4 else None

LOCK = threading.Lock()
COUNTS = {"upload": 0, "submit": 0, "tasks": 0, "manual": 0, "cdn": 0}
TASK_ID = "t15-smoke-task-0001"


def log(line):
    with open(LOG_PATH, "a", encoding="utf-8") as handle:
        handle.write(line + "\n")


def count(key):
    with LOCK:
        COUNTS[key] += 1
        return COUNTS[key]


def build_output(prompt_text):
    pages = sorted({int(m) for m in re.findall(r"\[第 (\d+) 页\]", prompt_text)})
    if not pages:
        raise ValueError("请求里没有页标记")
    parts = [
        {
            "id": "p-cover",
            "name": "后盖",
            "description": "机身背部可拆盖板，由四颗不脱落螺钉固定。",
            "evidence": [{"pageNumber": pages[0], "quote": "Loosen the four captive screws."}],
        }
    ]
    for page in pages:
        parts.append(
            {
                "id": "p-%d" % page,
                "name": "页%d部件" % page,
                "description": "第 %d 页描述的部件" % page,
                "evidence": [{"pageNumber": page, "quote": None}],
            }
        )
    steps = [
        {
            "id": "s-1",
            "title": "取下后盖",
            "orderedActions": ["松开固定件", "取下后盖"],
            "partIds": ["p-cover"],
            "evidence": [{"pageNumber": pages[0], "quote": "Loosen the four captive screws."}],
            "safetyNotes": ["操作前断电。"],
        }
    ]
    specs = [
        {
            "id": "sp-1",
            "label": "供电",
            "value": "DC 12 V / 2.5 A",
            "evidence": [{"pageNumber": pages[-1], "quote": None}],
        }
    ]
    return {
        "schemaVersion": "manual_extract_v1",
        "parts": parts,
        "steps": steps,
        "specs": specs,
        "uncertainties": [],
    }, pages


class Server(ThreadingHTTPServer):
    """跳过 getfqdn() 反查（离线/受限环境会阻塞监听建立）。"""

    daemon_threads = True
    allow_reuse_address = True

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        host, port = self.server_address[:2]
        self.server_name = host
        self.server_port = port


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):  # 统一记录到 fixture.log
        pass

    def _send(self, status, payload, content_type="application/json"):
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def _send_bytes(self, status, body, content_type):
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        raw = self.rfile.read(length) if length else b""
        path = self.path.split("?", 1)[0]
        if path == "/v1/responses":
            self._manual(raw)
        elif path == "/v3/files":
            n = count("upload")
            log("POST /v3/files #%d bytes=%d" % (n, len(raw)))
            self._send(200, {"code": 0, "data": {"image_token": "smoke-token-%d" % n}})
        elif path == "/v3/generation/multiview-to-model":
            n = count("submit")
            log("POST /v3/generation/multiview-to-model #%d（付费提交）body=%s" % (n, raw.decode("utf-8", "replace")[:200]))
            self._send(200, {"code": 0, "data": {"task_id": TASK_ID}})
        else:
            log("POST %s -> 501（缺脚本）" % self.path)
            self._send(501, {"error": {"message": "fixture has no route for %s" % self.path}})

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path.startswith("/v3/tasks/"):
            n = count("tasks")
            # 交替 running/success：既演示"远端仍在进行（waiting_provider）"，
            # 又保证每个任务的后续查询最终拿到成功状态（冒烟只关心阶段语义）。
            if n % 2 == 1:
                log("GET %s #%d -> running" % (path, n))
                self._send(
                    200,
                    {"code": 0, "data": {"task_id": TASK_ID, "status": "running", "progress": 42}},
                )
            else:
                model_url = "http://127.0.0.1:%d/cdn/model.glb" % PORT
                log("GET %s #%d -> success（model_url=本机 CDN）" % (path, n))
                self._send(
                    200,
                    {
                        "code": 0,
                        "data": {
                            "task_id": TASK_ID,
                            "status": "success",
                            "progress": 100,
                            "credits_consumed": 30,
                            "output": {
                                "model_url": model_url,
                                "rendered_image_url": "https://cdn.example.invalid/preview.png",
                            },
                        },
                    },
                )
        elif path == "/cdn/model.glb":
            count("cdn")
            log("GET /cdn/model.glb（样例 GLB）")
            self._send_bytes(200, GLB_PATH.read_bytes(), "model/gltf-binary")
        else:
            log("GET %s -> 501（缺脚本）" % self.path)
            self._send(501, {"error": {"message": "fixture has no route"}})

    def _manual(self, raw):
        n = count("manual")
        request = json.loads(raw.decode("utf-8"))
        prompt = request["input"][0]["content"][0]["text"]
        if REFUSE_FILE is not None and REFUSE_FILE.exists():
            REFUSE_FILE.unlink()
            log("POST /v1/responses #%d -> refusal（控制文件已消费）" % n)
            self._send(
                200,
                {
                    "id": "resp_smoke_refusal_%d" % n,
                    "object": "response",
                    "status": "completed",
                    "model": request.get("model"),
                    "output": [
                        {
                            "type": "message",
                            "role": "assistant",
                            "content": [
                                {
                                    "type": "refusal",
                                    "refusal": "冒烟样例：本次拒答（用于演示知识分支失败与按分支重试）。",
                                }
                            ],
                        }
                    ],
                    "usage": {"input_tokens": 100, "output_tokens": 10, "total_tokens": 110},
                },
            )
            return
        output, pages = build_output(prompt)
        log("POST /v1/responses #%d pages=%s model=%s max_output_tokens=%s store=%s"
            % (n, pages, request.get("model"), request.get("max_output_tokens"), request.get("store")))
        self._send(
            200,
            {
                "id": "resp_smoke_%d" % n,
                "object": "response",
                "status": "completed",
                "model": request.get("model"),
                "output": [
                    {
                        "type": "message",
                        "role": "assistant",
                        "content": [
                            {
                                "type": "output_text",
                                "annotations": [],
                                "text": json.dumps(output, ensure_ascii=False),
                            }
                        ],
                    }
                ],
                "usage": {"input_tokens": 1000, "output_tokens": 200, "total_tokens": 1200},
            },
        )


if __name__ == "__main__":
    open(LOG_PATH, "w", encoding="utf-8").close()
    server = Server(("127.0.0.1", PORT), Handler)
    print("fixture listening on 127.0.0.1:%d" % PORT, flush=True)
    server.serve_forever()
