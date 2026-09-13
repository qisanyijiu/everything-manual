#!/usr/bin/env python3
"""T12 QA 独立验算：读取 fixture 请求记录 + SQLite 事实 + serve 日志，按场景断言。

用法：qa-t12-verify.py <scenario> <db> <job_id> <fixture_log> <front_sha256> <left_sha256> <fake_key> <server_log>
退出码：0 = 全部断言通过；1 = 有失败（逐条打印）。
"""

import json
import sqlite3
import sys

SIGNATURE_CANARY = "qa-signature-canary-9931"
TASK_ID = "qa-task-0001"
EXPECTED_SUBMIT_KEYS = {
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

failures = []
checks = 0


def check(condition, message):
    global checks
    checks += 1
    if not condition:
        failures.append(message)
        print(f"  [FAIL] {message}")
    else:
        print(f"  [ok]   {message}")


def load_events(path):
    events = []
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if line:
                events.append(json.loads(line))
    return events


def stage_row(db, job_id, kind):
    row = db.execute(
        "SELECT id, status, usage_json, last_error, attempt_count, poll_count "
        "FROM job_stages WHERE job_id = ? AND stage_kind = ?",
        (job_id, kind),
    ).fetchone()
    if row is None:
        return None
    return {
        "id": row[0],
        "status": row[1],
        "usage": json.loads(row[2]) if row[2] else None,
        "last_error": row[3],
        "attempt_count": row[4],
        "poll_count": row[5],
    }


def attempts(db, job_id):
    rows = db.execute(
        "SELECT id, stage_id, submit_state, remote_task_id, last_error, started_at "
        "FROM provider_attempts WHERE job_id = ? ORDER BY started_at, created_at",
        (job_id,),
    ).fetchall()
    return [
        {
            "id": row[0],
            "stage_id": row[1],
            "submit_state": row[2],
            "remote_task_id": row[3],
            "last_error": row[4],
            "started_at": row[5],
        }
        for row in rows
    ]


def ledger(db, job_id):
    snapshot = db.execute("SELECT snapshot_id FROM jobs WHERE id = ?", (job_id,)).fetchone()[0]
    rows = db.execute(
        "SELECT provider, currency, reserved, actual, state FROM cost_ledger WHERE snapshot_id = ?",
        (snapshot,),
    ).fetchall()
    return {
        row[0]: {"currency": row[1], "reserved": row[2], "actual": row[3], "state": row[4]}
        for row in rows
    }


def main():
    (
        scenario,
        db_path,
        job_id,
        fixture_log,
        front_sha,
        left_sha,
        fake_key,
        server_log,
    ) = sys.argv[1:9]
    events = load_events(fixture_log)
    uploads = [event for event in events if event["event"] == "upload"]
    submits = [event for event in events if event["event"] == "submit"]
    polls = [event for event in events if event["event"] == "poll"]
    db = sqlite3.connect(db_path)
    upload_stage = stage_row(db, job_id, "tripo_upload")
    submit_stage = stage_row(db, job_id, "tripo_submit")
    poll_stage = stage_row(db, job_id, "tripo_poll")
    attempt_rows = attempts(db, job_id)
    entries = ledger(db, job_id)
    tripo = entries.get("tripo")
    manual_ai = entries.get("manual_ai")
    job_status = db.execute("SELECT status FROM jobs WHERE id = ?", (job_id,)).fetchone()[0]
    with open(server_log, encoding="utf-8", errors="replace") as handle:
        server_text = handle.read()

    print(f"=== 场景 {scenario}：fixture 记录（uploads={len(uploads)} submits={len(submits)} polls={len(polls)}）")
    print(f"    阶段：upload={upload_stage['status'] if upload_stage else None} "
          f"submit={submit_stage['status'] if submit_stage else None} "
          f"poll={poll_stage['status'] if poll_stage else None}；job={job_status}")
    print(f"    attempts={[(a['submit_state'], a['remote_task_id']) for a in attempt_rows]}")
    print(f"    ledger：tripo={tripo}；manual_ai={manual_ai}")

    # ------- 通用：凭据与 multipart 形态（所有发生了上传的场景） -------
    if uploads:
        for event in uploads:
            check(event.get("authorization") == f"Bearer {fake_key}",
                  f"上传携带 Authorization: Bearer <配置的测试键>（实际 {event.get('authorization')!r}）")
            check(event.get("multipart_ok") is True, "上传是合法 multipart")
            parts = event.get("parts") or []
            check(len(parts) == 1, f"上传 body 恰好 1 个 part（实际 {len(parts)}）")
            if parts:
                part = parts[0]
                check(part["name"] == "file",
                      f"multipart 字段名必须是 file（实际 {part['name']!r}）")
                check(part["filename"] in ("front.jpg", "left.png"),
                      f"文件名带正确扩展名（实际 {part['filename']!r}）")
        by_name = {event["parts"][0]["filename"]: event["parts"][0] for event in uploads if event.get("parts")}
        if scenario == "token_unknown":
            # 该场景第一个上传即失败（适配器按协议不符处理并立即返回），不要求两张都上传。
            for part in by_name.values():
                check(part["content_type"] in ("image/jpeg", "image/png"),
                      f"part Content-Type 合法（实际 {part['content_type']!r}）")
        elif "front.jpg" in by_name and "left.png" in by_name:
            check(by_name["front.jpg"]["content_type"] == "image/jpeg",
                  f"front 的 part Content-Type=image/jpeg（实际 {by_name['front.jpg']['content_type']!r}）")
            check(by_name["left.png"]["content_type"] == "image/png",
                  f"left 的 part Content-Type=image/png（实际 {by_name['left.png']['content_type']!r}）")
            check(by_name["front.jpg"]["sha256"] == front_sha,
                  "front 图片字节原样进入 multipart（sha256 与源文件一致）")
            check(by_name["left.png"]["sha256"] == left_sha,
                  "left 图片字节原样进入 multipart（sha256 与源文件一致）")
        else:
            check(False, f"上传文件名集合应为 front.jpg + left.png（实际 {sorted(by_name)}）")

    if submits:
        for event in submits:
            check(event.get("authorization") == f"Bearer {fake_key}",
                  "提交携带 Authorization: Bearer（值 = 配置键）")
            check(event.get("content_type") == "application/json", "提交 Content-Type=application/json")
            parsed = event.get("parsed")
            check(parsed is not None, "提交 body 是合法 JSON")
            check(event.get("v2_fields_present") == [],
                  f"请求体没有 v2 字段形态（实际 {event.get('v2_fields_present')}）")
            check(event.get("unexpected_top_level_keys") == [],
                  f"请求体没有预期外顶层字段（实际 {event.get('unexpected_top_level_keys')}）")
            if isinstance(parsed, dict):
                check(set(parsed) == EXPECTED_SUBMIT_KEYS,
                      f"请求体顶层字段集合恰为冻结的 9 项（实际 {sorted(parsed)}）")
                check(parsed.get("model") == "v3.1-20260211", "model=v3.1-20260211")
                check(parsed.get("texture") is True, "texture=true")
                check(parsed.get("pbr") is True, "pbr=true")
                check(parsed.get("texture_quality") == "standard", "texture_quality=standard")
                check(parsed.get("geometry_quality") == "standard", "geometry_quality=standard")
                check(parsed.get("face_limit") == 100000, "face_limit=100000")
                check(parsed.get("quad") is False, "quad=false")
                check(parsed.get("generate_parts") is False, "generate_parts=false")
                inputs = parsed.get("inputs")
                check(isinstance(inputs, list) and len(inputs) == 2,
                      f"inputs 是 2 项数组（实际 {inputs!r}）")
                if isinstance(inputs, list) and len(inputs) == 2:
                    check(list(inputs[0].keys()) == ["front"], "inputs[0] 是 front view-key")
                    check(list(inputs[1].keys()) == ["left"], "inputs[1] 是 left view-key（侧视图）")
                    if scenario == "token_verbatim":
                        check(inputs[0]["front"] == " TOKEN with Spaces_1 ",
                              f"front token 逐字符透传（不 trim/不截断；实际 {inputs[0]['front']!r}）")
                        check(inputs[1]["left"] == " TOKEN with Spaces_2 ",
                              f"left token 逐字符透传（实际 {inputs[1]['left']!r}）")
                    else:
                        check(inputs[0]["front"].startswith("qa-token-"),
                              f"front token 来自上传响应（实际 {inputs[0]['front']!r}）")
                        check(inputs[1]["left"].startswith("qa-token-"),
                              f"left token 来自上传响应（实际 {inputs[1]['left']!r}）")
    if polls:
        for event in polls:
            check(event.get("authorization") == f"Bearer {fake_key}", "查询携带 Authorization: Bearer")

    # ------- 场景专属断言 -------
    if scenario == "happy":
        check(len(uploads) == 2, f"上传恰好 2 次（实际 {len(uploads)}）")
        check(len(submits) == 1, f"付费提交恰好 1 次（实际 {len(submits)}）")
        check(len(polls) == 2, f"查询恰好 2 次（running→success；实际 {len(polls)}）")
        check(upload_stage["status"] == "succeeded", "tripo_upload=succeeded")
        check(submit_stage["status"] == "succeeded", "tripo_submit=succeeded")
        check(poll_stage["status"] == "succeeded", "tripo_poll=succeeded")
        # 父 job 仍是 running：manual_extract 分支属 T14，未接入时被延后（不假成功）。
        check(job_status == "running",
              f"父 job=running（Tripo 链已成功；manual 分支延后到 T14；实际 {job_status}）")
        usage = submit_stage["usage"] or {}
        check(usage.get("remoteTaskId") == TASK_ID, "tripo_submit.usage.remoteTaskId 落库")
        check(usage.get("requestHash") == submits[0]["body_sha256"],
              "落库 requestHash == 实际发出的 body sha256（同一份字节）")
        upload_usage = upload_stage["usage"] or {}
        tokens = {item["view"]: item for item in upload_usage.get("uploads", [])}
        check(set(tokens) == {"front", "left"}, f"上传事实含 front/left（实际 {sorted(tokens)}）")
        check(all(item["tokenField"] == "file_token" for item in tokens.values()),
              "上传 token 来源字段已记录（file_token）")
        poll_usage = poll_stage["usage"] or {}
        check(poll_usage.get("rawStatus") == "success", "poll.usage.rawStatus=success")
        check(poll_usage.get("normalizedStatus") == "success", "poll.usage.normalizedStatus=success")
        model_url = poll_usage.get("modelUrl") or ""
        check(model_url.endswith(f"?sign={SIGNATURE_CANARY}"), "modelUrl 保存为阶段事实（含签名）")
        billing = poll_usage.get("billing") or {}
        check(billing.get("literal") == "30", "billing.literal 原样保存（30）")
        check(billing.get("creditMinor") == 3000, "billing.creditMinor=3000（30 credits 精确换算）")
        check(billing.get("sourceField") == "credits_consumed", "billing.sourceField 已记录")
        check(billing.get("currency") == "credit_minor", "billing.currency=credit_minor")
        check(len(attempt_rows) == 1, f"provider_attempts 恰好 1 条（实际 {len(attempt_rows)}）")
        if attempt_rows:
            check(attempt_rows[0]["submit_state"] == "accepted", "attempt=accepted")
            check(attempt_rows[0]["remote_task_id"] == TASK_ID, "attempt.remote_task_id 已持久化")
        check(tripo and tripo["state"] == "settled" and tripo["actual"] == 3000,
              f"Tripo 预留按实际结算（实际 {tripo}）")
        check(manual_ai and manual_ai["state"] == "reserved", "Manual AI 预留不受影响")
        check(SIGNATURE_CANARY not in server_text, "serve 日志不含模型签名串（日志脱敏）")
        check(fake_key not in server_text, "serve 日志不含测试密钥")
        check("provider_handlers_registered" in server_text, "serve 日志有 provider_handlers_registered")

    elif scenario == "disconnect":
        check(len(uploads) == 2, f"上传 2 次（实际 {len(uploads)}）")
        check(len(submits) == 1, f"付费提交恰好 1 次（断连后从不重发；实际 {len(submits)}）")
        check(len(polls) == 0, f"没有进入查询阶段（实际 {len(polls)}）")
        check(submit_stage["status"] == "submission_unknown",
              f"tripo_submit=submission_unknown（实际 {submit_stage['status']}）")
        check(len(attempt_rows) == 1, f"attempt 恰好 1 条（实际 {len(attempt_rows)}）")
        if attempt_rows:
            check(attempt_rows[0]["submit_state"] == "unknown", "attempt=unknown")
            check(attempt_rows[0]["remote_task_id"] is None, "没有远端 task ID")
        check(tripo and tripo["state"] == "unknown" and tripo["actual"] is None,
              f"预留保留为 unknown 且 actual 为 NULL（实际 {tripo}）")

    elif scenario == "business_error":
        check(len(submits) == 1, f"业务拒绝后不重发（实际 {len(submits)}）")
        check(len(polls) == 0, "没有进入查询阶段")
        check(submit_stage["status"] == "failed", f"tripo_submit=failed（实际 {submit_stage['status']}）")
        check("1201" in (submit_stage["last_error"] or ""), "last_error 含业务 code 1201")
        check("invalid image token" in (submit_stage["last_error"] or ""), "last_error 含脱敏 message")
        check(len(attempt_rows) == 1 and attempt_rows[0]["submit_state"] == "failed",
              f"attempt=failed（实际 {[(a['submit_state']) for a in attempt_rows]}）")
        check(tripo and tripo["state"] == "released" and tripo["actual"] is None,
              f"明确拒绝 → 预留释放（实际 {tripo}）")

    elif scenario == "unknown":
        check(len(submits) == 1, f"付费提交 1 次（实际 {len(submits)}）")
        check(len(polls) >= 3, f"持续查询未知状态（实际 {len(polls)} 次）")
        check(poll_stage["status"] == "waiting_provider",
              f"未知状态进入可诊断等待，不成功也不失败（实际 {poll_stage['status']}）")
        poll_usage = poll_stage["usage"] or {}
        check(poll_usage.get("rawStatus") == "qa_new_state_zz", "原值保留在 rawStatus")
        check(poll_usage.get("normalizedStatus") == "unrecognized", "归一化=unrecognized（不猜测）")
        check(len(attempt_rows) == 1 and attempt_rows[0]["submit_state"] == "accepted",
              "购买事实不变（accepted）")

    elif scenario == "no_model":
        check(len(submits) == 1, f"付费提交 1 次（实际 {len(submits)}）")
        check(poll_stage["status"] in ("retry_wait", "failed"),
              f"success 缺模型不组装成功（实际 {poll_stage['status']}）")
        check(poll_stage["status"] != "succeeded", "不得 succeeded")
        poll_usage = poll_stage["usage"] or {}
        check(poll_usage.get("rawStatus") == "success", "rawStatus=success 原样保留")
        check("modelUrl" not in poll_usage, "usage 中没有 modelUrl（缺模型）")
        check(tripo and tripo["state"] == "reserved" and tripo["actual"] is None,
              f"无计费事实时不猜测金额结算（实际 {tripo}）")

    elif scenario == "token_unknown":
        check(len(uploads) >= 1, "上传被尝试")
        check(len(submits) == 0, f"上传失败时绝不发付费 POST（实际 {len(submits)}）")
        check(len(polls) == 0, "没有进入查询阶段")
        check(upload_stage["status"] in ("retry_wait", "failed"),
              f"上传阶段未成功（实际 {upload_stage['status']}）")
        check("token" in (upload_stage["last_error"] or ""),
              f"last_error 说明 token 字段问题（{upload_stage['last_error']!r}）")
        check(tripo and tripo["state"] == "reserved", "预留未被触碰")

    elif scenario == "token_verbatim":
        check(len(submits) == 1, f"付费提交 1 次（实际 {len(submits)}）")
        check(poll_stage["status"] == "succeeded", "poll=succeeded")
        upload_usage = upload_stage["usage"] or {}
        tokens = {item["view"]: item["token"] for item in upload_usage.get("uploads", [])}
        check(tokens.get("front") == " TOKEN with Spaces_1 ",
              f"库内 front token 逐字符保留（实际 {tokens.get('front')!r}）")
        check(tokens.get("left") == " TOKEN with Spaces_2 ",
              f"库内 left token 逐字符保留（实际 {tokens.get('left')!r}）")
        sent_inputs = submits[0]["parsed"]["inputs"]
        check(sent_inputs[0].get("front") == tokens.get("front")
              and sent_inputs[1].get("left") == tokens.get("left"),
              "提交使用的 token 与上传响应逐字一致")

    elif scenario == "poll_503":
        check(len(submits) == 1, f"查询失败不改变购买事实（提交 1 次；实际 {len(submits)}）")
        check(len(polls) >= 2, f"第一次查询失败后按退避重试（实际 {len(polls)} 次查询）")
        check(poll_stage["status"] == "succeeded",
              f"查询恢复后正常完成（查询失败 ≠ 生成失败；实际 {poll_stage['status']}）")
        check(any(event["event"] == "poll_503" for event in events), "fixture 确实返回过一次 503")
        check(tripo and tripo["state"] == "settled" and tripo["actual"] == 3000,
              f"最终按实际结算（实际 {tripo}）")

    elif scenario == "no_billing":
        check(len(submits) == 1, f"付费提交 1 次（实际 {len(submits)}）")
        check(poll_stage["status"] == "succeeded",
              f"有可下载模型即组装成功（实际 {poll_stage['status']}）")
        poll_usage = poll_stage["usage"] or {}
        check("modelUrl" in poll_usage, "modelUrl 已保存为事实")
        check("billing" not in poll_usage, "无计费字段：usage 中不出现 billing 事实")
        check(tripo and tripo["state"] == "reserved" and tripo["actual"] is None,
              f"无计费事实不猜测金额、不结算（实际 {tripo}）")

    elif scenario == "submit_429":
        check(len(submits) == 2, f"429 后按退避重试一次（第二次成功；实际 {len(submits)} 次）")
        if len(submits) >= 2:
            check(submits[0]["body_sha256"] == submits[1]["body_sha256"],
                  "重试发出的是同一份请求体（同 hash）")
        check(len(attempt_rows) == 2, f"429 → 新 attempt（实际 {len(attempt_rows)} 条）")
        if len(attempt_rows) == 2:
            check(attempt_rows[0]["submit_state"] == "failed", "第一次 attempt=failed（429 未被接受）")
            check("429" in (attempt_rows[0]["last_error"] or ""), "第一次 attempt 记录了 429")
            check(attempt_rows[1]["submit_state"] == "accepted", "第二次 attempt=accepted")
        check(tripo and tripo["state"] == "settled" and tripo["actual"] == 3000,
              f"重试成功后按实际结算（实际 {tripo}）")
        check(poll_stage is not None and poll_stage["status"] == "succeeded", "最终 poll=succeeded")

    else:
        check(False, f"未知场景：{scenario}")

    print(f"=== 断言 {checks} 项，失败 {len(failures)} 项")
    if failures:
        for item in failures:
            print(f"  - {item}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
