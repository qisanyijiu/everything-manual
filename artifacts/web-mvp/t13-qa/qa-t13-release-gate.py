#!/usr/bin/env python3
"""QA 回合 15 独立验收：**发布（dist）构建必须忽略 `download.allow_local_fixture`**。

背景（crates/server/src/assets/glb/download.rs 模块文档）：
本机 fixture 放行需要"显式测试配置 + 测试构建开关（`job-failpoints` feature）"两道门。
本脚本用 `cargo xtask dist` 产出的**发布二进制**，故意误配 `allow_local_fixture = true`，
让真实任务走到 `model_download` 阶段，独立核对：

  1. serve 日志出现 `download_local_fixture_ignored`（该键在发布构建中不生效）；
  2. `model_download` 阶段 = needs_input 且缺项码 = `download_insecure_scheme`；
  3. QA 自建 fixture 记录里**没有任何** `GET /model.glb`（连也不连）；
  4. 付费提交（POST /v3/generation/multiview-to-model）恒为 1 次（不放行不等于重新购买）；
  5. `assets(purpose=model)` 与 `model_revisions` 均为 0；
  6. 采样 `lsof -p <serve pid> -a -i -P -n`：非 loopback 连接 = 0。

本脚本的 fixture 与断言均由 QA 现场编写（不复用 RD 的 `model-fixture.py`）；全部网络目标
为 127.0.0.1，**零真实外网调用**。

用法：qa-t13-release-gate.py --binary <dist 二进制绝对路径> --artifacts <证据目录>
"""

import argparse
import http.cookiejar
import json
import os
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TASK_ID = "qa-gate-task-1"
CANARY_SIGN = "qa-gate-canary-signature"

FAILURES = []
CHECKS = []


def check(name, ok, detail):
    CHECKS.append((name, bool(ok), detail))
    if not ok:
        FAILURES.append(f"{name}: {detail}")
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}：{detail}", flush=True)


class FixtureServer(ThreadingHTTPServer):
    """只绑定 127.0.0.1 的本机 fixture（Tripo 三端点 + 模型 CDN）。"""

    daemon_threads = True
    allow_reuse_address = True


class FixtureState:
    def __init__(self, glb_path, log_path):
        self.glb_path = glb_path
        self.log_path = log_path
        self.lock = threading.Lock()
        self.uploads = 0
        self.submits = 0
        self.task_queries = 0
        self.model_requests = 0
        self.port = None

    def record(self, entry):
        with self.lock:
            with open(self.log_path, "a", encoding="utf-8") as handle:
                handle.write(json.dumps(entry, ensure_ascii=False) + "\n")

    def entries(self):
        if not os.path.exists(self.log_path):
            return []
        with open(self.log_path, encoding="utf-8") as handle:
            return [json.loads(line) for line in handle if line.strip()]


def make_handler(state):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args):
            return

        def _json(self, payload, status=200):
            body = json.dumps(payload).encode("utf-8")
            self.send_response(status)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _record(self, note, extra=None):
            entry = {
                "method": self.command,
                "path": self.path,
                "note": note,
                "authorization": "authorization" in {k.lower() for k in self.headers.keys()},
            }
            if extra:
                entry.update(extra)
            state.record(entry)

        def do_POST(self):
            length = int(self.headers.get("content-length", "0"))
            raw = self.rfile.read(length) if length else b""
            if self.path == "/v3/files":
                with state.lock:
                    state.uploads += 1
                    token = f"qa-gate-token-{state.uploads}"
                self._record("upload", {"token": token})
                self._json({"code": 0, "data": {"file_token": token}})
                return
            if self.path == "/v3/generation/multiview-to-model":
                with state.lock:
                    state.submits += 1
                    count = state.submits
                self._record(
                    "paid-submit",
                    {"count": count, "bodyBytes": len(raw)},
                )
                self._json({"code": 0, "data": {"task_id": TASK_ID}})
                return
            self._record("unexpected-post")
            self._json({"code": 1, "message": "unexpected"}, status=404)

        def do_GET(self):
            path = self.path.split("?", 1)[0]
            if path.startswith("/v3/tasks/"):
                with state.lock:
                    state.task_queries += 1
                    count = state.task_queries
                url = f"http://127.0.0.1:{state.port}/model.glb?sign={CANARY_SIGN}"
                self._record("task-query", {"count": count, "modelUrl": url})
                self._json(
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
                with state.lock:
                    state.model_requests += 1
                self._record("model-download-ATTEMPT")
                with open(state.glb_path, "rb") as handle:
                    body = handle.read()
                self.send_response(200)
                self.send_header("content-type", "model/gltf-binary")
                self.send_header("content-length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            self._record("unexpected-get")
            self._json({"error": "not found"}, status=404)

    return Handler


class Api:
    def __init__(self, base):
        self.base = base
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar)
        )
        self.csrf = None

    def request(self, method, path, body=None, headers=None, raw=None, content_type=None):
        url = self.base + path
        data = raw
        head = dict(headers or {})
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            head["content-type"] = "application/json"
        if content_type:
            head["content-type"] = content_type
        if self.csrf:
            head["x-csrf-token"] = self.csrf
        request = urllib.request.Request(url, data=data, headers=head, method=method)
        try:
            with self.opener.open(request, timeout=30) as response:
                payload = response.read()
                return response.status, json.loads(payload) if payload else {}
        except urllib.error.HTTPError as error:
            payload = error.read()
            try:
                return error.code, json.loads(payload)
            except json.JSONDecodeError:
                return error.code, {"raw": payload.decode("utf-8", "replace")}

    def multipart(self, purpose, filename, content_type, payload):
        boundary = f"----qa-gate-{purpose}-{filename}"
        body = bytearray()
        body.extend(
            f"--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\n{purpose}\r\n".encode()
        )
        body.extend(
            (
                f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; "
                f"filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
            ).encode()
        )
        body.extend(payload)
        body.extend(f"\r\n--{boundary}--\r\n".encode())
        return bytes(body), f"multipart/form-data; boundary={boundary}"


def wait_for_ready(base, timeout=40):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(base + "/health/ready", timeout=2) as response:
                if response.status == 200:
                    return True
        except Exception:
            time.sleep(0.2)
    return False


def non_loopback_connections(pid):
    try:
        output = subprocess.run(
            ["lsof", "-p", str(pid), "-a", "-i", "-P", "-n"],
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout
    except Exception as error:  # pragma: no cover - 环境缺 lsof
        return [f"lsof 不可用：{error}"]
    suspicious = []
    for line in output.splitlines()[1:]:
        if "127.0.0.1" in line or "[::1]" in line or "localhost" in line:
            continue
        suspicious.append(line.strip())
    return suspicious


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--artifacts", required=True)
    args = parser.parse_args()

    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
    fixtures = os.path.join(repo, "tests", "fixtures", "assets")
    binary = os.path.abspath(args.binary)
    artifacts = os.path.abspath(args.artifacts)
    os.makedirs(artifacts, exist_ok=True)

    print(f"== 发布二进制：{binary}", flush=True)
    print(f"== 证据目录：{artifacts}", flush=True)

    work = tempfile.mkdtemp(prefix="qa-t13-gate-")
    fixture_log = os.path.join(work, "fixture.jsonl")
    serve_log = os.path.join(work, "serve.log")
    state = FixtureState(os.path.join(fixtures, "sample-model.glb"), fixture_log)
    server = FixtureServer(("127.0.0.1", 0), make_handler(state))
    state.port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    print(f"== QA fixture 监听 127.0.0.1:{state.port}（只绑定回环）", flush=True)

    app_port = 18661
    base = f"http://127.0.0.1:{app_port}/api/v1"
    data_dir = os.path.join(work, "data")
    password = "qa-gate-password-77f2"
    password_file = os.path.join(work, "pw.txt")
    with open(password_file, "w", encoding="utf-8") as handle:
        handle.write(password + "\n")
    os.chmod(password_file, 0o600)

    serve_process = None
    try:
        init = subprocess.run(
            [binary, "init", "--data-dir", data_dir, "--password-file", password_file],
            capture_output=True,
            text=True,
            timeout=60,
        )
        check("init 退出码 0", init.returncode == 0, f"exit={init.returncode}")

        with open(os.path.join(data_dir, "config.toml"), "w", encoding="utf-8") as handle:
            handle.write(
                f"""price_catalog_path = "{repo}/price-catalog.example.toml"

[providers.tripo]
base_url = "http://127.0.0.1:{state.port}/v3"
api_key_env = "QA_GATE_TRIPO_KEY"

[providers.manual_ai]
model = "gpt-5-mini"
api_key_env = "QA_GATE_MANUAL_AI_KEY"

[download]
allowed_hosts = ["127.0.0.1"]
# 故意误配：发布构建必须忽略（两道门的第二道）。
allow_local_fixture = true
"""
            )

        environment = dict(os.environ)
        environment["QA_GATE_TRIPO_KEY"] = "qa-gate-fake-tripo-key"
        environment["QA_GATE_MANUAL_AI_KEY"] = "qa-gate-fake-manual-ai-key"
        serve_handle = open(serve_log, "w", encoding="utf-8")
        serve_process = subprocess.Popen(
            [binary, "serve", "--data-dir", data_dir, "--listen", f"127.0.0.1:{app_port}"],
            stdout=serve_handle,
            stderr=subprocess.STDOUT,
            env=environment,
        )
        check("serve 就绪", wait_for_ready(base), f"GET {base}/health/ready")

        api = Api(base)
        status, login = api.request("POST", "/auth/login", body={"password": password})
        check("登录 200", status == 200, f"status={status}")
        api.csrf = (login.get("data") or {}).get("csrfToken")

        status, item = api.request(
            "POST", "/items", body={"name": "QA13 门禁物品", "model": "QA-GATE-X"}
        )
        check("创建物品 201", status == 201, f"status={status}")
        item_id = item["data"]["id"]

        with open(os.path.join(fixtures, "sample-manual-text.pdf"), "rb") as handle:
            pdf = handle.read()
        raw, content_type = api.multipart("document", "manual.pdf", "application/pdf", pdf)
        status, doc_asset = api.request(
            "POST", f"/items/{item_id}/assets", raw=raw, content_type=content_type
        )
        check("上传 PDF 201", status == 201, f"status={status}")
        status, document = api.request(
            "POST",
            f"/items/{item_id}/documents",
            body={
                "sourceAssetId": doc_asset["data"]["id"],
                "title": "QA 门禁说明书",
            },
        )
        check("创建 document 201", status == 201, f"status={status}")
        status, preparation = api.request(
            "POST",
            f"/documents/{document['data']['id']}/preparations",
            body={"sourceSha256": document["data"]["sourceSha256"]},
        )
        check("创建 preparation 201", status == 201, f"status={status}")
        preparation_id = preparation["data"]["id"]

        with open(os.path.join(fixtures, "sample-photo-front.jpg"), "rb") as handle:
            jpeg = handle.read()
        for page in (1, 2):
            raw, content_type = api.multipart(
                "pageText", "page.txt", "text/plain", f"qa-gate-text-{page}".encode() * 8
            )
            status, text_asset = api.request(
                "POST", f"/items/{item_id}/assets", raw=raw, content_type=content_type
            )
            assert status == 201, (status, text_asset)
            raw, content_type = api.multipart("pageImage", "page.jpg", "image/jpeg", jpeg)
            status, image_asset = api.request(
                "POST", f"/items/{item_id}/assets", raw=raw, content_type=content_type
            )
            assert status == 201, (status, image_asset)
            status, page_response = api.request(
                "PUT",
                f"/preparations/{preparation_id}/pages/{page}",
                body={
                    "textAssetId": text_asset["data"]["id"],
                    "imageAssetId": image_asset["data"]["id"],
                    "viewport": {"width": 1240, "height": 1754, "rotation": 0},
                },
            )
            check(f"上传第 {page} 页 200", status == 200, f"status={status}")
        status, _ = api.request("GET", f"/preparations/{preparation_id}")
        # ETag 通过 urllib 的响应头读取一次。
        request = urllib.request.Request(
            base + f"/preparations/{preparation_id}", method="GET"
        )
        if api.csrf:
            request.add_header("x-csrf-token", api.csrf)
        with api.opener.open(request, timeout=30) as response:
            etag = response.headers.get("ETag")
        check("准备详情有 ETag", bool(etag), f"ETag={etag}")
        status, completed = api.request(
            "POST",
            f"/preparations/{preparation_id}/complete",
            body={"pageCount": 2},
            headers={"if-match": etag},
        )
        check("封存准备 200", status == 200, f"status={status}")

        photo_ids = []
        for view, name, content_type_name in (
            ("front", "sample-photo-front.jpg", "image/jpeg"),
            ("left", "sample-photo-left.png", "image/png"),
        ):
            with open(os.path.join(fixtures, name), "rb") as handle:
                payload = handle.read()
            raw, content_type = api.multipart("photo", name, content_type_name, payload)
            status, asset = api.request(
                "POST", f"/items/{item_id}/assets", raw=raw, content_type=content_type
            )
            assert status == 201, (status, asset)
            status, photo = api.request(
                "POST",
                f"/items/{item_id}/photos",
                body={"assetId": asset["data"]["id"], "view": view},
            )
            assert status == 201, (status, photo)
            photo_ids.append(photo["data"]["id"])

        status, estimate = api.request(
            "POST",
            f"/items/{item_id}/estimates",
            body={
                "preparationId": preparation_id,
                "photoIds": photo_ids,
                "modelPreset": "tripo-h-v3.1-standard",
            },
        )
        check("estimate 201", status == 201, f"status={status}")
        quote_id = estimate["data"]["id"]
        status, _ = api.request(
            "POST", f"/items/{item_id}/estimates/{quote_id}/confirm"
        )
        check("确认报价 200", status == 200, f"status={status}")
        status, job = api.request(
            "POST",
            f"/items/{item_id}/jobs",
            body={
                "quoteId": quote_id,
                "preparationId": preparation_id,
                "photoIds": photo_ids,
                "limits": {"tripoCreditMinor": 30000, "manualAiUsdMicros": 500000},
            },
            headers={"idempotency-key": "qa-gate-job-1"},
        )
        check("建单 202", status == 202, f"status={status}")
        job_id = job["data"]["id"]
        print(f"== job={job_id}（等待 model_download 结论）", flush=True)

        database = os.path.join(data_dir, "manual.sqlite3")
        status_value = "missing"
        deadline = time.time() + 90
        suspicious_seen = []
        while time.time() < deadline:
            with sqlite3.connect(database) as connection:
                row = connection.execute(
                    "SELECT status FROM job_stages WHERE job_id = ? AND stage_kind = 'model_download'",
                    (job_id,),
                ).fetchone()
            status_value = row[0] if row else "missing"
            suspicious_seen.extend(non_loopback_connections(serve_process.pid))
            if status_value in ("needs_input", "failed", "succeeded"):
                break
            time.sleep(1)

        check(
            "model_download 终态 = needs_input（发布构建不放行本机 fixture）",
            status_value == "needs_input",
            f"实际 {status_value}",
        )
        with sqlite3.connect(database) as connection:
            needs_input, last_error = connection.execute(
                "SELECT needs_input_json, last_error FROM job_stages WHERE job_id = ? AND stage_kind = 'model_download'",
                (job_id,),
            ).fetchone()
            model_assets = connection.execute(
                "SELECT COUNT(*) FROM assets WHERE purpose = 'model'"
            ).fetchone()[0]
            revisions = connection.execute(
                "SELECT COUNT(*) FROM model_revisions"
            ).fetchone()[0]
            attempts = connection.execute(
                "SELECT COUNT(*) FROM provider_attempts"
            ).fetchone()[0]
        print(f"  needs_input = {needs_input}", flush=True)
        print(f"  last_error  = {last_error}", flush=True)
        check(
            "缺项码 = download_insecure_scheme",
            "download_insecure_scheme" in (needs_input or ""),
            f"{needs_input}",
        )
        check("model 资产数 = 0", model_assets == 0, f"实际 {model_assets}")
        check("model_revisions = 0", revisions == 0, f"实际 {revisions}")
        check("付费提交 attempt 恰 1 条", attempts == 1, f"实际 {attempts}")

        entries = state.entries()
        model_hits = [entry for entry in entries if entry["path"].startswith("/model.glb")]
        submits = [entry for entry in entries if entry["note"] == "paid-submit"]
        check("fixture 收到 0 次模型下载请求", not model_hits, f"实际 {len(model_hits)} 次")
        check("fixture 记到付费提交 1 次", len(submits) == 1, f"实际 {len(submits)} 次")
        check(
            "付费提交请求带 Authorization（对照）",
            bool(submits) and submits[0]["authorization"] is True,
            f"{submits[:1]}",
        )

        with open(serve_log, encoding="utf-8") as handle:
            serve_text = handle.read()
        check(
            "serve 日志出现 download_local_fixture_ignored",
            "download_local_fixture_ignored" in serve_text,
            "事件已记录（该键在发布构建中不生效）",
        )
        check(
            "serve 日志不含签名 canary",
            CANARY_SIGN not in serve_text,
            "签名查询串未进日志",
        )
        check(
            "服务进程无非 loopback 连接",
            not suspicious_seen,
            f"{suspicious_seen[:3]}",
        )

        shutil.copy(serve_log, os.path.join(artifacts, "qa-gate-serve.log"))
        shutil.copy(fixture_log, os.path.join(artifacts, "qa-gate-fixture.jsonl"))
        with open(os.path.join(artifacts, "qa-gate-report.json"), "w", encoding="utf-8") as handle:
            json.dump(
                {
                    "binary": binary,
                    "checks": [
                        {"name": name, "ok": ok, "detail": detail}
                        for name, ok, detail in CHECKS
                    ],
                    "needs_input": needs_input,
                    "last_error": last_error,
                    "model_assets": model_assets,
                    "model_revisions": revisions,
                    "provider_attempts": attempts,
                    "fixture_entries": entries,
                    "non_loopback_connections": suspicious_seen,
                },
                handle,
                ensure_ascii=False,
                indent=2,
            )
    finally:
        if serve_process is not None:
            serve_process.send_signal(signal.SIGTERM)
            try:
                serve_process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                serve_process.kill()
        server.shutdown()
        server.server_close()
        shutil.rmtree(work, ignore_errors=True)

    passed = sum(1 for _, ok, _ in CHECKS if ok)
    print(f"\n== 汇总：{passed}/{len(CHECKS)} 项通过", flush=True)
    if FAILURES:
        for failure in FAILURES:
            print(f"!! FAIL {failure}", flush=True)
        return 1
    print("== 发布构建门禁核对通过：误配 allow_local_fixture 不放行本机 fixture。", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
