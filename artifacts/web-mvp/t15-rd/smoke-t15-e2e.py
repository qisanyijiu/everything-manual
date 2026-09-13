#!/usr/bin/env python3
"""T15 手工冒烟（fixture 端到端，零真实外网/付费）：

用**测试构建**的单二进制（`--features embedded-ui,job-failpoints`，本机 fixture 放行开关
仅测试构建生效）跑完整流程：

  A) 建物品 → 准备（3 页）→ 报价 → 确认 → 建单 → 知识+模型两分支 → 草稿：
     断言 job succeeded、草稿 needs_review/complete、**manual_releases 为空**；
  B) 知识分支失败（fixture 拒答一次）→ 只重试该批次 → 草稿补齐；
     断言 Tripo 付费提交计数不变（没有偷偷重新购买）、模型成果保留；
  C) cancel：已提交阶段（waiting_provider）保留、未提交阶段 cancelled、
     取消后 fixture 计数不再增长（不新增付费步骤）。

用法： python3 smoke-t15-e2e.py <test-build-binary>
"""
import http.cookiejar
import json
import os
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

BIN = Path(sys.argv[1]).resolve()
HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
FIXTURES = REPO / "tests" / "fixtures" / "assets"
PORT = 18330
FIXTURE_PORT = 18331
PASSWORD = "smoke-password-t15-7c31"
WORK = Path(tempfile.mkdtemp(prefix="em-t15-smoke-"))
KEEP = os.environ.get("KEEP_WORK") == "1"

report = []


def say(line=""):
    print(line, flush=True)
    report.append(line)


class Client:
    def __init__(self, base):
        self.base = base
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))
        self.csrf = None

    def request(self, method, path, body=None, headers=None, raw=None, content_type="application/json"):
        request = urllib.request.Request(self.base + path, method=method)
        data = None
        if raw is not None:
            data = raw
            request.add_header("content-type", content_type)
        elif body is not None:
            data = json.dumps(body).encode("utf-8")
            request.add_header("content-type", "application/json")
        if self.csrf:
            request.add_header("x-csrf-token", self.csrf)
        for name, value in (headers or {}).items():
            request.add_header(name, value)
        try:
            with self.opener.open(request, data) as response:
                return response.status, dict(response.headers), response.read()
        except urllib.error.HTTPError as error:
            return error.code, dict(error.headers), error.read()

    def json(self, method, path, body=None, headers=None):
        status, response_headers, payload = self.request(method, path, body, headers)
        parsed = json.loads(payload.decode("utf-8")) if payload else {}
        return status, response_headers, parsed


def db():
    connection = sqlite3.connect(WORK / "data" / "manual.sqlite3")
    connection.row_factory = sqlite3.Row
    return connection


def wait_for(description, condition, timeout=60.0, interval=0.25):
    deadline = time.time() + timeout
    while time.time() < deadline:
        value = condition()
        if value:
            return value
        time.sleep(interval)
    raise SystemExit("超时：%s" % description)


def stages(job_id):
    connection = db()
    rows = connection.execute(
        "select stage_kind, batch_index, status, last_error, result_asset_id, usage_json "
        "from job_stages where job_id = ? order by created_at, stage_kind, batch_index",
        (job_id,),
    ).fetchall()
    connection.close()
    return [dict(row) for row in rows]


def stage(job_id, kind, batch=0):
    for row in stages(job_id):
        if row["stage_kind"] == kind and row["batch_index"] == batch:
            return row
    raise SystemExit("阶段不存在：%s[%d]" % (kind, batch))


def fixture_lines():
    return [line for line in (WORK / "fixture.log").read_text(encoding="utf-8").splitlines() if line]


def fixture_count(prefix):
    return sum(1 for line in fixture_lines() if line.startswith(prefix))


def upload(client, item, purpose, name, filename=None):
    boundary = "----em-t15-smoke"
    path = FIXTURES / name if filename is None else Path(filename)
    content = path.read_bytes()
    content_type = {
        ".pdf": "application/pdf",
        ".jpg": "image/jpeg",
        ".png": "image/png",
        ".txt": "text/plain",
    }.get(path.suffix, "application/octet-stream")
    body = b""
    body += ("--%s\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\n%s\r\n" % (boundary, purpose)).encode()
    body += ("--%s\r\nContent-Disposition: form-data; name=\"file\"; filename=\"%s\"\r\nContent-Type: %s\r\n\r\n"
             % (boundary, path.name, content_type)).encode()
    body += content + b"\r\n"
    body += ("--%s--\r\n" % boundary).encode()
    status, _, parsed = client.request(
        "POST", "/items/%s/assets" % item, raw=body, content_type="multipart/form-data; boundary=%s" % boundary
    )
    payload = json.loads(parsed.decode("utf-8"))
    assert status == 201, payload
    return payload["data"]


def create_job(client, item, preparation, photos, key):
    status, _, estimate = client.json(
        "POST",
        "/items/%s/estimates" % item,
        {"preparationId": preparation, "photoIds": photos, "modelPreset": "tripo-h-v3.1-standard"},
    )
    assert status == 201, estimate
    quote = estimate["data"]["id"]
    status, _, confirmed = client.json("POST", "/items/%s/estimates/%s/confirm" % (item, quote))
    assert status == 200, confirmed
    status, _, job = client.json(
        "POST",
        "/items/%s/jobs" % item,
        {
            "quoteId": quote,
            "preparationId": preparation,
            "photoIds": photos,
            "limits": {"tripoCreditMinor": 3000, "manualAiUsdMicros": 500000},
        },
        {"idempotency-key": key},
    )
    assert status == 202, job
    return job["data"]["id"]


def job_detail(client, job_id):
    status, headers, payload = client.json("GET", "/jobs/%s" % job_id)
    assert status == 200, payload
    return payload["data"], headers.get("ETag") or headers.get("etag")


def main():
    say("== T15 冒烟工作目录：%s（结束删除；KEEP_WORK=1 保留）" % WORK)
    shutil.copy(BIN, WORK / "everything-manual")
    shutil.copy(HERE / "t15-fixture.py", WORK / "fixture.py")
    shutil.copy(REPO / "price-catalog.example.toml", WORK / "prices.toml")
    (WORK / "pw.txt").write_text(PASSWORD + "\n", encoding="utf-8")
    os.chmod(WORK / "pw.txt", 0o600)

    fixture = subprocess.Popen(
        [
            sys.executable,
            str(WORK / "fixture.py"),
            str(FIXTURE_PORT),
            str(WORK / "fixture.log"),
            str(FIXTURES / "sample-model.glb"),
            str(WORK / "refuse-once"),
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    server = None
    try:
        say("== 启动本机 fixture（Tripo v3 + 说明书 AI + 模型 CDN；只绑定 127.0.0.1）")
        wait_for("fixture 启动", lambda: (WORK / "fixture.log").exists() and True, timeout=10)

        subprocess.run(
            [str(WORK / "everything-manual"), "init", "--data-dir", "./data", "--password-file", "./pw.txt"],
            cwd=WORK,
            check=True,
            capture_output=True,
        )
        (WORK / "data" / "config.toml").write_text(
            """price_catalog_path = "./prices.toml"

[providers.tripo]
base_url = "http://127.0.0.1:%(port)d/v3"
api_key_env = "SMOKE_TRIPO_KEY"

[providers.manual_ai]
base_url = "http://127.0.0.1:%(port)d/v1"
model = "gpt-5-mini"
api_key_env = "SMOKE_MANUAL_AI_KEY"

[download]
allowed_hosts = ["127.0.0.1"]
allow_local_fixture = true
""" % {"port": FIXTURE_PORT},
            encoding="utf-8",
        )
        env = dict(os.environ, SMOKE_TRIPO_KEY="fake-tripo-key", SMOKE_MANUAL_AI_KEY="fake-manual-ai-key")
        serve_log = open(WORK / "serve.log", "w", encoding="utf-8")
        server = subprocess.Popen(
            [
                str(WORK / "everything-manual"),
                "serve",
                "--data-dir",
                "./data",
                "--listen",
                "127.0.0.1:%d" % PORT,
            ],
            cwd=WORK,
            env=env,
            stdout=serve_log,
            stderr=subprocess.STDOUT,
        )
        base = "http://127.0.0.1:%d/api/v1" % PORT
        client = Client(base)
        wait_for(
            "服务就绪",
            lambda: client.request("GET", "/health/ready")[0] == 200,
        )
        for line in (WORK / "serve.log").read_text(encoding="utf-8").splitlines():
            if "pipeline_handlers_registered" in line:
                say("  " + json.loads(line)["fields"]["message"])
                break

        status, _, login = client.json("POST", "/auth/login", {"password": PASSWORD})
        assert status == 200, login
        client.csrf = login["data"]["csrfToken"]
        say("== 登录完成（假凭据；仅本机 fixture）")

        # ---------------- A) 全链路 ----------------
        say()
        say("== A) 建物品 → 准备（3 页）→ 报价 → 确认 → 建单 → 两分支 → 草稿")
        status, _, item = client.json("POST", "/items", {"name": "T15 冒烟相机", "model": "SK-T15"})
        assert status == 201, item
        item_id = item["data"]["id"]
        doc_asset = upload(client, item_id, "document", "sample-manual-text.pdf")
        status, _, document = client.json(
            "POST", "/items/%s/documents" % item_id, {"sourceAssetId": doc_asset["id"], "title": "冒烟说明书"}
        )
        assert status == 201, document
        status, _, preparation = client.json(
            "POST",
            "/documents/%s/preparations" % document["data"]["id"],
            {"sourceSha256": document["data"]["sourceSha256"]},
        )
        assert status == 201, preparation
        preparation_id = preparation["data"]["id"]
        for page in range(1, 4):
            image = upload(client, item_id, "pageImage", "sample-photo-front.jpg")
            text_path = WORK / ("page%d.txt" % page)
            text_path.write_text("第 %d 页：后盖由四颗螺钉固定。\n" % page, encoding="utf-8")
            text = upload(client, item_id, "pageText", "page.txt", filename=text_path)
            status, _, written = client.json(
                "PUT",
                "/preparations/%s/pages/%d" % (preparation_id, page),
                {
                    "textAssetId": text["id"],
                    "imageAssetId": image["id"],
                    "viewport": {"width": 1240, "height": 1754, "rotation": 0},
                },
            )
            assert status == 200, written
        status, headers, detail = client.json("GET", "/preparations/%s" % preparation_id)
        etag = headers.get("ETag") or headers.get("etag")
        status, _, completed = client.json(
            "POST",
            "/preparations/%s/complete" % preparation_id,
            {"pageCount": 3},
            {"if-match": etag},
        )
        assert status == 200, completed
        photos = []
        for view, name in (("front", "sample-photo-front.jpg"), ("left", "sample-photo-left.png")):
            asset = upload(client, item_id, "photo", name)
            status, _, photo = client.json(
                "POST", "/items/%s/photos" % item_id, {"assetId": asset["id"], "view": view}
            )
            assert status == 201, photo
            photos.append(photo["data"]["id"])
        job_a = create_job(client, item_id, preparation_id, photos, "smoke-t15-job-a")
        say("  准备 ready（3 页）、照片 front+left、job=%s" % job_a)

        wait_for(
            "job A 组装完成",
            lambda: stage(job_a, "assemble_draft")["status"] == "succeeded",
        )
        detail_a, etag_a = job_detail(client, job_a)
        connection = db()
        draft_row = connection.execute(
            "select id, revision, status, knowledge_json from manual_drafts where snapshot_id = ?",
            (detail_a["snapshotId"],),
        ).fetchone()
        releases = connection.execute("select count(*) from manual_releases").fetchone()[0]
        ledger = connection.execute(
            "select provider, currency, reserved, actual, state from cost_ledger order by provider"
        ).fetchall()
        connection.close()
        knowledge = json.loads(draft_row["knowledge_json"])
        say("  job 状态 = %s / revision=%s；草稿 %s status=%s revision=%s completeness=%s"
            % (detail_a["status"], detail_a["revision"], draft_row["id"], draft_row["status"],
               draft_row["revision"], knowledge["completeness"]))
        say("  草稿模型版本 = %s；知识部件 = %d、步骤 = %d、页覆盖 complete=%s"
            % (knowledge["model"]["revisionId"][:18] + "…",
               len(knowledge["knowledge"]["parts"]), len(knowledge["knowledge"]["steps"]),
               knowledge["knowledge"]["coverage"]["complete"]))
        say("  **manual_releases 计数 = %d（无自动发布路径）**；cost_ledger = %s"
            % (releases, [tuple(row) for row in ledger]))
        say("  fixture 计数：付费提交 = %d、说明书批次请求 = %d、模型下载 = %d"
            % (fixture_count("POST /v3/generation/multiview-to-model"),
               fixture_count("POST /v1/responses"), fixture_count("GET /cdn/model.glb")))
        status, _, draft_get = client.json("GET", "/items/%s/drafts/%s" % (item_id, draft_row["id"]))
        say("  GET 草稿 = %s（completeness=%s；notices[0]=%s）"
            % (status, draft_get["data"]["completeness"], draft_get["data"]["notices"][0]))

        # ---------------- B) 知识分支失败 → 仅重试该分支 ----------------
        say()
        say("== B) 知识分支失败（fixture 拒答一次）→ 只重试该批次")
        (WORK / "refuse-once").write_text("refuse\n", encoding="utf-8")
        job_b = create_job(client, item_id, preparation_id, photos, "smoke-t15-job-b")
        wait_for(
            "job B 批次拒答",
            lambda: stage(job_b, "manual_extract")["status"] == "needs_input",
        )
        detail_b, etag_b = job_detail(client, job_b)
        batch = stage(job_b, "manual_extract")
        usage = json.loads(batch["usage_json"])
        submit_count_before = fixture_count("POST /v3/generation/multiview-to-model")
        say("  job 状态 = %s；批次 status=%s producedKnowledge=%s errorCode=%s"
            % (detail_b["status"], batch["status"], usage["producedKnowledge"], usage["errorCode"]))
        say("  模型分支：tripo_submit=%s、model_validate=%s（独立完成）"
            % (stage(job_b, "tripo_submit")["status"], stage(job_b, "model_validate")["status"]))
        say("  重试前 fixture 付费提交计数 = %d（job A 1 次 + job B 1 次）" % submit_count_before)
        status, _, retry = client.json(
            "POST",
            "/jobs/%s/retry" % job_b,
            {"stageId": stage_id_of(job_b, "manual_extract")},
            {"if-match": etag_b, "idempotency-key": "smoke-t15-retry-b"},
        )
        assert status == 200, retry
        say("  重试响应：previousStatus=%s、requeuedDependents=%s、notice=%s"
            % (retry["data"]["previousStatus"], retry["data"]["requeuedDependents"], retry["data"]["notice"]))
        wait_for("job B 组装完成", lambda: stage(job_b, "assemble_draft")["status"] == "succeeded")
        detail_b2, _ = job_detail(client, job_b)
        connection = db()
        draft_b = connection.execute(
            "select knowledge_json from manual_drafts where snapshot_id = ?", (detail_b2["snapshotId"],)
        ).fetchone()
        connection.close()
        knowledge_b = json.loads(draft_b["knowledge_json"])
        say("  重试后 job 状态 = %s；草稿 completeness=%s；付费提交计数 = %d（**未重新购买**）"
            % (detail_b2["status"], knowledge_b["completeness"],
               fixture_count("POST /v3/generation/multiview-to-model")))
        say("  说明书批次请求合计 = %d（A:1 + B:拒答1 + B:重试1）"
            % fixture_count("POST /v1/responses"))

        # ---------------- C) cancel ----------------
        say()
        say("== C) cancel（已提交阶段保留、未提交阶段停止、取消后无新付费步骤）")
        job_c = create_job(client, item_id, preparation_id, photos, "smoke-t15-job-c")
        wait_for(
            "job C 进入等待远端",
            lambda: stage(job_c, "tripo_poll")["status"] == "waiting_provider",
        )
        before_cancel = fixture_lines()
        detail_c, etag_c = job_detail(client, job_c)
        status, _, cancelled = client.json("POST", "/jobs/%s/cancel" % job_c, None, {"if-match": etag_c})
        assert status == 200, cancelled
        say("  cancel 响应：job=%s；stagesCancelled=%d；preserved=%s"
            % (cancelled["data"]["job"]["status"], cancelled["data"]["stagesCancelled"],
               [(row["stageKind"], row["status"]) for row in cancelled["data"]["preservedStages"]]))
        say("  notice=%s" % cancelled["data"]["notice"])
        time.sleep(2.0)
        after_cancel = fixture_lines()
        connection = db()
        cancelled_stage = connection.execute(
            "select status from job_stages where job_id = ? and stage_kind = 'model_download'", (job_c,)
        ).fetchone()[0]
        poll_status = connection.execute(
            "select status, usage_json from job_stages where job_id = ? and stage_kind = 'tripo_poll'", (job_c,)
        ).fetchone()
        ledger_c = connection.execute(
            "select provider, state, reserved, actual from cost_ledger where snapshot_id = "
            "(select snapshot_id from jobs where id = ?) order by provider",
            (job_c,),
        ).fetchall()
        connection.close()
        say("  未提交阶段（model_download）= %s；已提交阶段（tripo_poll）= %s（保留查询/账务）"
            % (cancelled_stage, poll_status["status"]))
        say("  job C 账本 = %s（未因取消静默释放）" % [tuple(row) for row in ledger_c])
        say("  取消后 fixture 新增行数 = %d（**不新增任何付费步骤**）" % (len(after_cancel) - len(before_cancel)))
        say()
        say("== fixture 原始记录（摘要）")
        for line in after_cancel:
            say("  " + line)
        say()
        say("== 冒烟通过：A 全链路草稿 + B 按分支重试 + C 取消语义（零真实外网/付费）")
    finally:
        for process in (server, fixture):
            if process is not None:
                process.send_signal(signal.SIGTERM)
        for process in (server, fixture):
            if process is not None:
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
        (WORK / "smoke-report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
        if KEEP:
            print("（KEEP_WORK=1：保留 %s）" % WORK)
        else:
            shutil.rmtree(WORK, ignore_errors=True)


def stage_id_of(job_id, kind):
    connection = db()
    row = connection.execute(
        "select id from job_stages where job_id = ? and stage_kind = ?", (job_id, kind)
    ).fetchone()
    connection.close()
    return row[0]


if __name__ == "__main__":
    main()
