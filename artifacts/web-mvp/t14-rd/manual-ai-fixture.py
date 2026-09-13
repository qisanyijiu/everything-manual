#!/usr/bin/env python3
"""T14 手工冒烟的**本机 fixture**：OpenAI Responses 形态的最小脚本化服务器。

- 只绑定 127.0.0.1；未知路径一律 501（缺脚本必须失败，不返回通用成功）；
- `POST /v1/responses`：从请求体的 `input_text` 里解析本批页号（`[第 N 页]`），
  返回 `manual_extract_v1` 的**构造**响应（非官方原文）：
  - parts：每页一个部件 + 一个同名"后盖"（**第二批的 description 不同** → 合并时保留冲突）；
  - steps：一条步骤（引用本批的"后盖"部件局部 id）；
  - specs：`供电`（第二批的 value 不同 → 冲突）；
  - evidence 的页号只引用本批输入页（服务端会再次校验）。
- 每次请求把方法/路径/页号/请求体摘要写入 `fixture.log`（供冒烟脚本展示与断言）。
"""
import json
import re
import socketserver
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG_PATH = sys.argv[2] if len(sys.argv) > 2 else "fixture.log"


def log(line):
    with open(LOG_PATH, "a", encoding="utf-8") as handle:
        handle.write(line + "\n")


def build_output(prompt_text):
    pages = sorted({int(m) for m in re.findall(r"\[第 (\d+) 页\]", prompt_text)})
    if not pages:
        raise ValueError("请求里没有页标记")
    batch_no = 0 if max(pages) <= 5 else 1
    parts = []
    # 同名部件"后盖"：第二批给出不同事实 → 合并保留冲突（不丢出处）。
    cover_description = (
        "四颗不脱落螺钉固定（第 1 批读到）"
        if batch_no == 0
        else "卡扣固定：沿边缘撬开（第 2 批读到）"
    )
    parts.append(
        {
            "id": "p-cover",
            "name": "后盖",
            "description": cover_description,
            "evidence": [{"pageNumber": pages[0], "quote": "第 %d 页原文引文" % pages[0]}],
        }
    )
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
            "evidence": [{"pageNumber": pages[0], "quote": "第 %d 页原文引文" % pages[0]}],
            "safetyNotes": ["操作前断电。"],
        }
    ]
    specs = [
        {
            "id": "sp-1",
            "label": "供电",
            "value": "DC 12 V / 2.5 A" if batch_no == 0 else "DC 9 V / 1.5 A",
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
    """跳过 `getfqdn()` 反向解析：受限/离线环境下它会阻塞监听建立。"""

    daemon_threads = True
    allow_reuse_address = True

    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        host, port = self.server_address[:2]
        self.server_name = host
        self.server_port = port


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):  # 静默：记录统一走 fixture.log
        pass

    def _send(self, status, payload, content_type="application/json"):
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        raw = self.rfile.read(length) if length else b""
        if self.path != "/v1/responses":
            log("POST %s -> 501（缺脚本：不返回通用成功）" % self.path)
            self._send(501, {"error": {"message": "fixture has no route for %s" % self.path}})
            return
        request = json.loads(raw.decode("utf-8"))
        prompt = request["input"][0]["content"][0]["text"]
        output, pages = build_output(prompt)
        images = [
            item["image_url"][:30] + "…"
            for item in request["input"][0]["content"]
            if item.get("type") == "input_image"
        ]
        log(
            "POST /v1/responses pages=%s model=%s max_output_tokens=%s store=%s "
            "images=%d injection_marker=%s"
            % (
                pages,
                request.get("model"),
                request.get("max_output_tokens"),
                request.get("store"),
                len(images),
                "yes" if "evil.invalid" in prompt else "no",
            )
        )
        self._send(
            200,
            {
                "id": "resp_fixture_%d" % len(pages),
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

    def do_GET(self):
        log("GET %s -> 501（缺脚本）" % self.path)
        self._send(501, {"error": {"message": "fixture has no route"}})


if __name__ == "__main__":
    port = int(sys.argv[1])
    open(LOG_PATH, "w", encoding="utf-8").close()
    server = Server(("127.0.0.1", port), Handler)
    print("fixture listening on 127.0.0.1:%d" % port, flush=True)
    server.serve_forever()
