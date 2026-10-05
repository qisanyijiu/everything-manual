#!/usr/bin/env python3
"""Host-side HTTP checks for an isolated Docker fixture; no provider calls."""
import argparse
import hashlib
import http.cookiejar
import json
from pathlib import Path
import re
import sys
import time
import urllib.error
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["seed", "read", "tls", "tls-seed", "tls-probe"])
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--origin")
    parser.add_argument("--password-file", required=True, type=Path)
    parser.add_argument("--canary-file", type=Path)
    parser.add_argument("--state", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--fixtures", required=True, type=Path)
    parser.add_argument("--require-release", action="store_true")
    args = parser.parse_args()
    origin = args.origin or args.base_url
    password = args.password_file.read_text().strip()
    cookies = http.cookiejar.CookieJar()
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPCookieProcessor(cookies))
    checks = []
    csrf = None
    sensitive = [password]
    if args.canary_file:
        sensitive.extend(args.canary_file.read_text().splitlines())

    def check(condition, description):
        if not condition:
            raise ValueError(description)
        checks.append(description)

    def request(path, method="GET", body=None, headers=None, expected=200):
        merged = {"Origin": origin}
        if csrf:
            merged["x-csrf-token"] = csrf
        if headers:
            merged.update(headers)
        if isinstance(body, dict):
            body = json.dumps(body).encode()
            merged["Content-Type"] = "application/json"
        req = urllib.request.Request(args.base_url + path, data=body, headers=merged, method=method)
        try:
            response = client.open(req, timeout=30)
        except urllib.error.HTTPError as error:
            response = error
        payload = response.read()
        if response.status != expected:
            raise ValueError(f"HTTP {method} {path}: expected {expected}, received {response.status}")
        # Login legitimately returns derived CSRF/session credentials. Other
        # responses must never reflect known fixture password or API canaries.
        if path != "/api/v1/auth/login":
            check(all(value.encode() not in payload for value in sensitive), f"No secret reflected: {method} {path}")
        return payload, response.headers

    def data(path, method="GET", body=None, expected=200):
        payload, headers = request(path, method, body, expected=expected)
        return json.loads(payload)["data"], headers

    def upload(item, purpose, filename, mime, content):
        boundary = "em-docker-verification-boundary"
        payload = (f"--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\n{purpose}\r\n"
                   f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n").encode() + content + f"\r\n--{boundary}--\r\n".encode()
        response, _ = request(f"/api/v1/items/{item}/assets", "POST", payload,
                              {"Content-Type": f"multipart/form-data; boundary={boundary}"}, expected=201)
        asset = json.loads(response)["data"]
        check(asset["sha256"] == hashlib.sha256(content).hexdigest(), f"Uploaded {purpose} SHA256 matches")
        return asset

    def read_asset(asset_id, sha, size=None):
        content, headers = request(f"/api/v1/assets/{asset_id}/content")
        check(hashlib.sha256(content).hexdigest() == sha, f"Asset hash matches: {asset_id}")
        if size is not None:
            check(len(content) == size, f"Asset size matches: {asset_id}")
        check(headers.get("Accept-Ranges") == "bytes", "Asset supports byte ranges")
        check(headers.get("ETag", "").strip('"') == sha, "Asset ETag matches SHA256")
        ranged, range_headers = request(f"/api/v1/assets/{asset_id}/content", headers={"Range": "bytes=0-99"}, expected=206)
        check(ranged == content[:100] and range_headers.get("Content-Range") == f"bytes 0-99/{len(content)}", "Range returns exact byte prefix")
        head, head_headers = request(f"/api/v1/assets/{asset_id}/content", "HEAD")
        check(head == b"" and head_headers.get("Content-Length") == str(len(content)), "HEAD returns correct length without body")
        return content

    report = {"mode": args.mode, "baseUrl": args.base_url, "providerRequests": 0, "checks": checks}
    try:
        for _ in range(60):
            try:
                ready, _ = request("/api/v1/health/ready")
                check(bool(json.loads(ready)), "Readiness is JSON 200")
                break
            except (urllib.error.URLError, TimeoutError, ValueError):
                time.sleep(1)
        else:
            raise ValueError("Application did not become ready within 60 seconds")
        request("/api/v1/items", expected=401)
        check(True, "Unauthenticated library is rejected")
        login, login_headers = data("/api/v1/auth/login", "POST", {"password": password})
        csrf = login["csrfToken"]
        cookie = login_headers.get("Set-Cookie", "")
        check("HttpOnly" in cookie and "SameSite" in cookie, "Session cookie has HttpOnly and SameSite")
        check(("Secure" in cookie) == origin.startswith("https://"), "Secure cookie follows the configured public origin")
        # In TLS-mode only the reverse proxy would carry this cookie over actual
        # HTTPS. This transport probe deliberately authenticates no HTTP reads.
        if args.mode == "tls-probe":
            providers, _ = data("/api/v1/settings/providers")
            check(providers["active"]["manualAi"]["baseUrl"] == "https://example.com", "TLS probe uses only the fixed public example.com endpoint")
            report["providerRequests"] = 1
            report["paidRequests"] = 0
            report["upstreamTlsProbe"] = {"url": "https://example.com/models", "credential": "random fake QA canary; no real API key", "requests": 1}
            response, _ = request("/api/v1/settings/providers/manual-ai/models", "POST", {}, expected=502)
            check(json.loads(response)["error"]["message"] == "当前说明书 AI 服务不支持读取模型，请手动填写服务支持的模型名称", "Real upstream HTTP 404/405 proves HTTPS certificate validation and response receipt")
        elif args.mode == "tls":
            request("/api/v1/auth/login", "POST", {"password": password}, {"Origin": args.base_url}, expected=403)
            check(True, "HTTP Origin is rejected for an HTTPS deployment")
        else:
            index, _ = request("/")
            check(b"<html" in index.lower(), "Embedded SPA served")
            nested, _ = request("/settings", headers={"Accept": "text/html"})
            check(b"<html" in nested.lower(), "Nested SPA route refresh served")
            unknown, unknown_headers = request("/api/v1/docker-verification-missing", expected=404)
            check("application/json" in unknown_headers.get("Content-Type", ""), "Unknown API is JSON 404")
            missing, _ = request("/assets/docker-verification-missing.js", expected=404)
            check(b"<html" not in missing.lower(), "Missing static file does not return the SPA")
            roots = set(re.findall(r'(?:src|href)=["\']([^"\']+)["\']', index.decode()))
            queue = [path for path in roots if path.startswith("/")]
            seen = set()
            workers = 0
            while queue:
                path = queue.pop()
                if path in seen:
                    continue
                seen.add(path)
                payload, headers = request(path)
                check(bool(payload) and "text/html" not in headers.get("Content-Type", ""), f"Embedded resource served: {path}")
                if "pdf.worker" in path:
                    workers += 1
                if path.endswith((".js", ".css")):
                    text = payload.decode("utf-8")
                    for reference in re.findall(r'["\'`(](?:\./)?([\w.-]+\.(?:js|css|woff2?|ttf|otf))["\'`)]', text):
                        queue.append("/assets/" + reference)
                    for reference in re.findall(r'["\'`](/assets/[^"\'`]+)["\'`]', text):
                        queue.append(reference)
            check(workers > 0, "Embedded PDF.js worker fetched from the served module graph")
            providers, _ = data("/api/v1/settings/providers")
            state = json.loads(args.state.read_text()) if args.state.exists() else {}
            state["staticPaths"] = sorted(seen)
            if args.mode == "tls-seed":
                keys = args.canary_file.read_text().splitlines()
                saved, _ = data("/api/v1/settings/providers", "PUT", {
                    "revision": providers["revision"],
                    "tripo": {"action": "update", "baseUrl": providers["active"]["tripo"]["baseUrl"], "model": providers["active"]["tripo"]["model"], "keyAction": "keep"},
                    "manualAi": {"action": "update", "baseUrl": "https://example.com", "model": "fixture-no-requests", "keyAction": "replace", "apiKey": keys[1]},
                })
                check(saved["pending"], "Upstream TLS probe configuration waits for restart")
                state["providerRevision"] = saved["revision"]
            elif args.mode == "seed":
                item, _ = data("/api/v1/items", "POST", {"name": "Docker Linux 验证样本", "brand": "fixture", "model": "offline-only"}, expected=201)
                state["itemId"] = item["id"]
                state["assets"] = []
                for purpose, filename, mime in [("document", "sample-manual-text.pdf", "application/pdf"), ("photo", "sample-photo-front.jpg", "image/jpeg")]:
                    content = (args.fixtures / filename).read_bytes()
                    state["assets"].append(upload(item["id"], purpose, filename, mime, content))
                keys = args.canary_file.read_text().splitlines()
                saved, _ = data("/api/v1/settings/providers", "PUT", {
                    "revision": providers["revision"],
                    "tripo": {"action": "update", "baseUrl": "https://tripo.example.invalid/v3", "model": "v3.1-20260211", "keyAction": "replace", "apiKey": keys[0]},
                    "manualAi": {"action": "update", "baseUrl": "https://manual.example.invalid/v1", "model": "fixture-no-requests", "keyAction": "replace", "apiKey": keys[1]},
                })
                check(saved["pending"], "Saved API configuration requires restart")
                state["providerRevision"] = saved["revision"]
            elif state.get("itemId"):
                item, _ = data(f"/api/v1/items/{state['itemId']}")
                check(item["model"] == "offline-only", "Created item survived restart/restore")
                if state.get("providerRevision"):
                    check(providers["revision"] == state["providerRevision"] and not providers["pending"], "Encrypted provider configuration survived restart")
                    check(providers["active"]["tripo"]["keyConfigured"] and providers["active"]["manualAi"]["keyConfigured"], "Both encrypted API keys decrypt after restart")
            for asset in state.get("assets", []):
                read_asset(asset["id"], asset["sha256"], asset["size"])
            if args.require_release:
                items, _ = data("/api/v1/items")
                release = None
                for item in items:
                    releases, _ = data(f"/api/v1/items/{item['id']}/releases")
                    if releases:
                        release, _ = data(f"/api/v1/items/{item['id']}/releases/{releases[0]['id']}")
                        break
                check(release is not None, "Restored sample contains an immutable release")
                frozen = {"releaseId": release["id"], "manifestSha256": release["manifestSha256"]}
                if "release" in state:
                    check(frozen == state["release"], "Release ID and manifest hash survived restart/backup/restore")
                state["release"] = frozen
                state["releaseItemId"] = item["id"]
                manifest = release["manifest"]
                state["releaseAssets"] = [{"id": manifest["model"]["assetId"], "sha256": manifest["model"]["sha256"]}]
                read_asset(manifest["model"]["assetId"], manifest["model"]["sha256"])
                for document in manifest["documents"]:
                    state["releaseAssets"].append({"id": document["sourceAssetId"], "sha256": document["sourceSha256"]})
                    read_asset(document["sourceAssetId"], document["sourceSha256"])
                check(True, "Immutable release PDF and GLB hashes match")
            args.state.parent.mkdir(parents=True, exist_ok=True)
            args.state.write_text(json.dumps(state, ensure_ascii=False, indent=2) + "\n")
            args.state.chmod(0o600)
        report["status"] = "passed"
    except Exception as error:
        # Never print response bodies or unknown exception strings containing
        # credentials. Requests above generate fixed metadata-only diagnostics.
        report["status"] = "failed"
        report["error"] = str(error) if isinstance(error, ValueError) else type(error).__name__
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    args.report.chmod(0o600)
    print(json.dumps({"status": report["status"], "checks": len(checks), "report": str(args.report)}))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
