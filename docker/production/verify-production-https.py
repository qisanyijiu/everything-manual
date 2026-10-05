#!/usr/bin/env python3
"""Isolated HTTPS checks of existing immutable images; no provider/API spending.

Requires Docker/Compose, Python 3.11+, OpenSSL and a completed T20 fixture backup.
Only test certificate trust is installed in the process-local SSL context.
"""
from __future__ import annotations

import argparse
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import sqlite3
import ssl
import subprocess
import sys
import time
import urllib.error
import urllib.request


class VerificationError(Exception):
    pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True, help="Existing sha256 image ID or repository@sha256 digest")
    parser.add_argument("--proxy-image", required=True, help="Existing immutable Nginx image reference")
    parser.add_argument("--sample-backup", required=True, type=Path, help="Completed, synthetic T20 fixture backup only")
    parser.add_argument("--fixture-password-file", type=Path, help="Optional fixture password; otherwise the T20 fixture password")
    parser.add_argument("--platform", default="linux/amd64", choices=["linux/amd64", "linux/arm64"])
    parser.add_argument("--port", default=18443, type=int)
    parser.add_argument("--out", required=True, type=Path, help="New evidence directory; never overwritten")
    parser.add_argument("--keep", action="store_true", help="Keep only this verifier's isolated containers/volumes for debugging")
    args = parser.parse_args()
    immutable = re.compile(r"(?:sha256:[a-fA-F0-9]{64}|[A-Za-z0-9._:/-]+@sha256:[a-fA-F0-9]{64})\Z")
    if not all(immutable.fullmatch(ref) for ref in (args.image, args.proxy_image)):
        parser.error("Both images must use an immutable image ID or repository@sha256 digest, never a tag.")
    if not 1024 <= args.port <= 65535:
        parser.error("The isolated local TLS port must be 1024–65535.")
    proto = Path(__file__).resolve().parent
    sample = args.sample_backup.resolve()
    if not (sample / "database/manual.sqlite3").is_file() or not (sample / "manifest.json").is_file():
        parser.error("The synthetic sample backup is incomplete.")
    # Prevent a caller's live queued fixture from launching work after restore.
    try:
        with sqlite3.connect((sample / "database/manual.sqlite3").as_uri() + "?mode=ro", uri=True) as db:
            active = db.execute("SELECT count(*) FROM jobs WHERE status IN ('queued','running','retry_wait','waiting_provider')").fetchone()[0]
            if active:
                parser.error("The sample must contain no active jobs; use the completed T20 synthetic fixture.")
    except sqlite3.Error:
        parser.error("The sample fixture database cannot be read safely.")
    if args.out.exists() or args.out.is_symlink():
        parser.error("The evidence directory must be new.")
    for executable in ("docker", "openssl"):
        if not shutil.which(executable):
            parser.error(f"Required verification tool unavailable: {executable}")
    with socket.socket() as probe:
        try:
            probe.bind(("127.0.0.1", args.port))
        except OSError:
            parser.error("The selected test TLS port is occupied.")
    os.umask(0o077)
    out = args.out.resolve()
    private = out / "private"
    private.mkdir(parents=True, mode=0o700)
    fixture_password = args.fixture_password_file.read_text().strip() if args.fixture_password_file else "test-password-t20-backup"
    if not fixture_password:
        parser.error("The fixture password file is empty.")
    (private / "password.txt").write_text(fixture_password + "\n")
    master_key = secrets.token_hex(32)
    (private / "master.key").write_text(master_key + "\n")
    # An explicit empty env file prevents loading a deployment .env beside the
    # portable compose file. All test values come from the scrubbed environment.
    (private / "compose.env").write_text("")
    origin = f"https://127.0.0.1:{args.port}"
    project = "em-vs05-https-" + time.strftime("%Y%m%d%H%M%S", time.gmtime()) + "-" + secrets.token_hex(4)
    fixture_volume = project + "-fixture-input"
    loader = project + "-fixture-loader"
    # Caller deployment bindings and credentials must not override this fixture.
    env = {key: value for key, value in os.environ.items() if not key.startswith(("EM_", "COMPOSE_"))}
    env.update({
        "EM_IMAGE": args.image, "EM_PROXY_IMAGE": args.proxy_image,
        "EM_DOCKER_PLATFORM": args.platform, "EM_PUBLIC_ORIGIN": origin,
        "EM_HTTPS_BIND": "127.0.0.1", "EM_HTTPS_PORT": str(args.port),
        "EM_ADMIN_PASSWORD_FILE": str(private / "password.txt"),
        "EM_MASTER_KEY_FILE": str(private / "master.key"),
        "EM_TLS_CERT_FILE": str(private / "certificate.pem"),
        "EM_TLS_KEY_FILE": str(private / "tls-key.pem"),
    })
    report: dict = {"status": "failed", "project": project, "platform": args.platform,
                    "origin": origin, "paidRequests": 0, "providerRequests": 0,
                    "systemTrustModified": False, "checks": [], "images": {}, "sourceFiles": {}}
    sequence = 0
    created_volume = False
    created_loader = False
    stack_attempted = False

    def check(condition: bool, label: str) -> None:
        if not condition:
            raise VerificationError(label)
        report["checks"].append(label)

    def run(argv: list[str], label: str, *, persist: bool = True, must_pass: bool = True) -> str:
        nonlocal sequence
        sequence += 1
        result = subprocess.run(argv, env=env, capture_output=True, text=True, check=False)
        report.setdefault("commands", []).append({"label": label, "exitCode": result.returncode})
        if persist:
            (out / f"{sequence:02d}-{label}.log").write_text(result.stdout + result.stderr)
        if result.returncode != 0 and must_pass:
            raise VerificationError(f"Local command failed: {label} (exit {result.returncode})")
        return result.stdout

    def compose(*argv: str, label: str, must_pass: bool = True) -> str:
        return run(["docker", "compose", "--env-file", str(private / "compose.env"), "--project-directory", str(proto), "-f", str(proto / "compose.production.yaml"),
                    "-p", project, *argv], label, must_pass=must_pass)

    def inspected(container: str) -> dict:
        return json.loads(run(["docker", "inspect", container], "inspect-container", persist=False))[0]

    def inspect_runtime(label: str) -> dict[str, str]:
        ids = {name: compose("ps", "-q", name, label=f"{label}-{name}-id").strip()
               for name in ("manual", "proxy", "prepare", "prepare-tls")}
        inventory = {}
        for name, uid in (("manual", "10001:10001"), ("proxy", "101:101")):
            info = inspected(ids[name])
            check(info["Config"]["User"] == uid, f"{label}: {name} is configured non-root")
            check(info["HostConfig"]["ReadonlyRootfs"], f"{label}: {name} root filesystem is read-only")
            check(info["HostConfig"]["CapDrop"] == ["ALL"], f"{label}: {name} drops all capabilities")
            check("no-new-privileges:true" in info["HostConfig"]["SecurityOpt"], f"{label}: {name} cannot gain privileges")
            requested = args.image if name == "manual" else args.proxy_image
            check(info["Config"]["Image"] == requested, f"{label}: {name} retains its immutable image reference")
            check(info["State"]["Health"]["Status"] == "healthy", f"{label}: {name} is healthy")
            inventory[name] = {"imageReference": info["Config"]["Image"], "imageId": info["Image"],
                               "user": uid, "readonlyRoot": True, "mountDestinations": [m["Destination"] for m in info["Mounts"]]}
        app = inspected(ids["manual"])
        configured = dict(entry.split("=", 1) for entry in app["Config"]["Env"])
        check(configured["EM_PUBLIC_ORIGIN"] == origin, f"{label}: HTTPS public origin is preserved")
        check(configured["EM_TRUSTED_PROXY_CIDRS"] == "127.0.0.1/32", f"{label}: only adjacent loopback proxy is trusted")
        check(configured["EM_SESSION__COOKIE_SECURE"] == "always", f"{label}: cookies are explicitly Secure")
        check(all(b["HostIp"] == "127.0.0.1" for bindings in app["HostConfig"]["PortBindings"].values() for b in bindings),
              f"{label}: verifier publishes TLS only on host loopback")
        check(set(app["HostConfig"]["PortBindings"]) == {"8443/tcp"}, f"{label}: no HTTP/backend port is published")
        check(inspected(ids["proxy"])["HostConfig"]["NetworkMode"] == "container:" + ids["manual"],
              f"{label}: TLS proxy shares the app network namespace")
        for name, uid in (("prepare", "10001"), ("prepare-tls", "101")):
            info = inspected(ids[name])
            check(info["HostConfig"]["NetworkMode"] == "none", f"{label}: {name} has no network")
            actual = run(["docker", "exec", ids[name], "sh", "-c", "awk '/^Uid:/{print $2}' /proc/1/status"], f"{label}-{name}-uid").strip()
            check(actual == uid, f"{label}: {name} drops its running process privileges")
        run(["docker", "exec", ids["proxy"], "sh", "-c",
             "set -eu; test \"$(id -u)\" = 101; test \"$(stat -c %a /run/tls)\" = 700; "
             "for f in /run/tls/fullchain.pem /run/tls/privkey.pem; do "
             "test \"$(stat -c %a \"$f\")\" = 600; test \"$(stat -c %u \"$f\")\" = 101; done; "
             "if touch /run/tls/write-probe 2>/dev/null; then rm /run/tls/write-probe; exit 1; fi; "
             "nginx -t -c /etc/nginx/nginx.conf"], f"{label}-tls-permissions")
        check(True, f"{label}: TLS files are 0600/UID101 and mounted read-only; nginx validates the certificate/key")
        run(["docker", "exec", ids["manual"], "sh", "-c",
             "set -eu; test \"$(id -u)\" = 10001; for x in cargo rustc node npm python python3; do ! command -v \"$x\"; done; "
             "test ! -d /src/everything-manual; test ! -d /src; test ! -d /app/apps/web; "
             "test \"$(stat -c %a /run/private/master.key)\" = 600; "
             "cd /usr/local/bin; sha256sum -c /opt/everything-manual/SHA256SUMS"], f"{label}-source-free")
        check(True, f"{label}: production runtime has no Rust/Node/Python/source and binary SHA256 matches")
        run(["docker", "exec", ids["manual"], "cat", "/opt/everything-manual/build-info.json"], f"{label}-build-info")
        for volume in (project + "_private-runtime", project + "_tls-runtime"):
            meta = json.loads(run(["docker", "volume", "inspect", volume], f"{label}-volume", persist=False))[0]
            check(meta["Options"].get("type") == "tmpfs", f"{label}: {volume.rsplit('_', 1)[1]} is memory-backed")
        report[label + "Runtime"] = inventory
        return ids

    try:
        for filename in ("compose.production.yaml", "config.toml", "nginx.production.conf", "prepare-tls.sh", "verify-production-https.py"):
            report["sourceFiles"][filename] = hashlib.sha256((proto / filename).read_bytes()).hexdigest()
        run(["docker", "version"], "docker-version")
        run(["docker", "compose", "version"], "compose-version")
        for name, ref in (("application", args.image), ("proxy", args.proxy_image)):
            meta = json.loads(run(["docker", "image", "inspect", "--platform", args.platform, ref], f"inspect-{name}-image", persist=False))[0]
            check(meta["Os"] == "linux" and meta["Architecture"] == args.platform.split("/")[1], f"{name}: existing image matches requested platform")
            report["images"][name] = {"reference": ref, "id": meta["Id"], "os": meta["Os"], "architecture": meta["Architecture"]}
        run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-sha256", "-days", "1", "-nodes",
             "-subj", "/CN=localhost", "-addext", "subjectAltName=IP:127.0.0.1,DNS:localhost",
             "-keyout", str(private / "tls-key.pem"), "-out", str(private / "certificate.pem")], "test-certificate")
        for path in private.iterdir():
            path.chmod(0o600)
        compose("config", "--quiet", label="compose-schema")
        # Prove HTTPS is mandatory using only Compose interpolation: no containers.
        rejected_env = env.copy()
        rejected_env.pop("EM_PUBLIC_ORIGIN")
        rejected = subprocess.run(["docker", "compose", "--env-file", str(private / "compose.env"), "--project-directory", str(proto), "-f", str(proto / "compose.production.yaml"),
                                   "-p", project, "config", "--quiet"], env=rejected_env, capture_output=True)
        check(rejected.returncode != 0, "Production Compose rejects a missing public origin")
        run(["docker", "volume", "create", fixture_volume], "fixture-volume")
        created_volume = True
        stack_attempted = True
        compose("create", "--no-build", "--pull", "never", "manual", label="create-data-volume")
        # The root loader has no network and only synthetic fixture/data mounts.
        run(["docker", "create", "--name", loader, "--platform", args.platform, "--network", "none", "--user", "0:0",
             "--entrypoint", "sh", "-v", f"{project}_manual-data:/data", "-v", fixture_volume + ":/fixture",
             args.image, "-c", "/usr/local/bin/everything-manual restore --from /fixture/input --data-dir /data && chown -R 10001:10001 /data"], "fixture-loader-create")
        created_loader = True
        run(["docker", "cp", str(sample), loader + ":/fixture/input"], "copy-synthetic-backup")
        run(["docker", "start", "-a", loader], "fixture-restore")
        check(inspected(loader)["State"]["ExitCode"] == 0, "Synthetic fixture restores before service startup")
        # Negative production configuration: the HTTPS front door must remain
        # unavailable, even if the backend itself can read its restored data.
        env["EM_PUBLIC_ORIGIN"] = f"http://127.0.0.1:{args.port}"
        compose("up", "-d", "--force-recreate", "--no-build", "--pull", "never", "--wait", "--wait-timeout", "45",
                label="reject-http-origin", must_pass=False)
        check(report["commands"][-1]["exitCode"] != 0, "HTTP public origin cannot start a healthy production stack")
        rejected_log = compose("logs", "--no-color", "prepare-tls", label="http-origin-helper")
        check("Production EM_PUBLIC_ORIGIN must be an explicit HTTPS origin." in rejected_log,
              "HTTP origin is rejected by the intended fail-closed TLS helper")
        unavailable = False
        probe_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        probe_context.load_verify_locations(cafile=str(private / "certificate.pem"))
        try:
            with socket.create_connection(("127.0.0.1", args.port), timeout=3) as raw:
                with probe_context.wrap_socket(raw, server_hostname="127.0.0.1"):
                    pass
        except (OSError, ssl.SSLError, TimeoutError):
            unavailable = True
        check(unavailable, "HTTP origin exposes no usable TLS front door")
        compose("stop", "-t", "60", label="stop-http-negative")

        # The helper only checks PEM envelopes; Nginx must also parse/reject a
        # malformed certificate before allowing any HTTPS service to start.
        env["EM_PUBLIC_ORIGIN"] = origin
        invalid_cert = private / "invalid-certificate.pem"
        invalid_cert.write_text("-----BEGIN CERTIFICATE-----\ninvalid-fixture-base64\n-----END CERTIFICATE-----\n")
        invalid_cert.chmod(0o600)
        env["EM_TLS_CERT_FILE"] = str(invalid_cert)
        compose("up", "-d", "--force-recreate", "--no-build", "--pull", "never", "--wait", "--wait-timeout", "180",
                "prepare", "prepare-tls", "manual", label="invalid-certificate-helpers")
        compose("run", "--rm", "--no-deps", "proxy", "-t", "-c", "/etc/nginx/nginx.conf",
                label="invalid-certificate-nginx", must_pass=False)
        check(report["commands"][-1]["exitCode"] != 0, "Nginx configuration validation rejects malformed TLS PEM")
        invalid_log = (out / f"{sequence:02d}-invalid-certificate-nginx.log").read_text()
        check("cannot load certificate" in invalid_log, "Malformed TLS failure is certificate parsing, not an unrelated command error")
        compose("stop", "-t", "60", label="stop-certificate-negative")
        env["EM_TLS_CERT_FILE"] = str(private / "certificate.pem")
        compose("up", "-d", "--force-recreate", "--no-build", "--pull", "never", "--wait", "--wait-timeout", "180", label="https-start")
        first_ids = inspect_runtime("initial")
        # This context trusts exactly the temporary self-signed test certificate.
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.load_verify_locations(cafile=str(private / "certificate.pem"))
        check(context.verify_mode == ssl.CERT_REQUIRED and context.check_hostname,
              "Test HTTPS client enforces certificate and hostname validation")
        cookies = http.cookiejar.CookieJar()
        client = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                            urllib.request.HTTPSHandler(context=context),
                                            urllib.request.HTTPCookieProcessor(cookies))
        csrf = None

        def request(path: str, method: str = "GET", body=None, headers=None, expected: int = 200, add_csrf: bool = True):
            merged = {"Origin": origin}
            if csrf and add_csrf:
                merged["x-csrf-token"] = csrf
            if headers:
                merged.update(headers)
            if isinstance(body, dict):
                body = json.dumps(body).encode()
                merged["Content-Type"] = "application/json"
            req = urllib.request.Request(origin + path, data=body, headers=merged, method=method)
            try:
                response = client.open(req, timeout=30)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                payload = response.read()
                check(response.status == expected, f"HTTPS {method} {path}: status {expected}")
                if path != "/api/v1/auth/login":
                    check(fixture_password.encode() not in payload and master_key.encode() not in payload,
                          f"HTTPS {method} {path}: no fixture password/master reflected")
                return payload, response.headers

        def data(path: str, method: str = "GET", body=None, **kwargs):
            payload, headers = request(path, method, body, **kwargs)
            return json.loads(payload)["data"], headers

        ready, _ = request("/api/v1/health/ready")
        check(bool(json.loads(ready)), "Actual certificate-validated HTTPS serves readiness JSON")
        request("/api/v1/items", expected=401)
        spoofed = {"X-Forwarded-Proto": "http", "X-Forwarded-For": "198.51.100.1", "Forwarded": 'for=198.51.100.1;proto=http'}
        login, login_headers = data("/api/v1/auth/login", "POST", {"password": fixture_password}, headers=spoofed)
        csrf = login["csrfToken"]
        session_cookie = login_headers.get("Set-Cookie", "")
        check(all(flag in session_cookie for flag in ("Secure", "HttpOnly", "SameSite=Strict")),
              "TLS login returns Secure/HttpOnly/SameSite=Strict even with forged forwarding headers")
        check(len(list(cookies)) > 0 and all(cookie.secure for cookie in cookies), "HTTPS client retains only Secure session cookies")
        data("/api/v1/auth/session")
        request("/api/v1/auth/login", "POST", {"password": fixture_password}, headers={"Origin": f"http://127.0.0.1:{args.port}"}, expected=403)
        request("/api/v1/auth/login", "POST", {"password": fixture_password}, headers={"Origin": "https://wrong-origin.invalid"}, expected=403)
        request("/api/v1/items", "POST", {"name": "HTTPS fixture"}, add_csrf=False, expected=403)
        request("/api/v1/items", "POST", {"name": "HTTPS fixture"}, headers={"x-csrf-token": "invalid-fixture-token"}, expected=403)
        request("/api/v1/items", "POST", {"name": "HTTPS fixture"}, headers={"Origin": "https://wrong-origin.invalid"}, expected=403)
        item, _ = data("/api/v1/items", "POST", {"name": "HTTPS fixture persistence", "brand": "fixture", "model": "no-provider-requests"}, expected=201)
        fixture_item_id = item["id"]
        index, _ = request("/")
        check(b"<html" in index.lower(), "Production embedded SPA loads through HTTPS")
        nested, _ = request("/settings", headers={"Accept": "text/html"})
        check(b"<html" in nested.lower(), "Nested production SPA refresh works through HTTPS")
        missing, missing_headers = request("/api/v1/vs05-missing", expected=404)
        check("application/json" in missing_headers.get("Content-Type", ""), "Missing API uses JSON 404")
        missing, _ = request("/assets/vs05-missing.js", expected=404)
        check(b"<html" not in missing.lower(), "Missing static resource is not returned as the SPA")
        queue = [path for path in re.findall(r'(?:src|href)=["\']([^"\']+)["\']', index.decode()) if path.startswith("/")]
        resources = {}
        while queue:
            path = queue.pop()
            if path in resources:
                continue
            payload, headers = request(path)
            check(bool(payload) and "text/html" not in headers.get("Content-Type", ""), "Embedded HTTPS resource: " + path)
            resources[path] = hashlib.sha256(payload).hexdigest()
            if path.endswith((".js", ".css")):
                text = payload.decode()
                queue.extend("/assets/" + ref for ref in re.findall(r'["\'`(](?:\./)?([\w.-]+\.(?:js|css|woff2?|ttf|otf))["\'`)]', text))
                queue.extend(re.findall(r'["\'`](/assets/[^"\'`]+)["\'`]', text))
        check(any("pdf.worker" in path for path in resources), "Embedded PDF.js worker is fetched over validated HTTPS")
        report["embeddedResourceHashes"] = resources
        items, _ = data("/api/v1/items")
        frozen = None
        for candidate in items:
            releases, _ = data(f"/api/v1/items/{candidate['id']}/releases")
            if releases:
                frozen, _ = data(f"/api/v1/items/{candidate['id']}/releases/{releases[0]['id']}")
                frozen_item_id = candidate["id"]
                break
        check(frozen is not None, "Synthetic sample contains an immutable release")
        assets = [{"id": frozen["manifest"]["model"]["assetId"], "sha256": frozen["manifest"]["model"]["sha256"]}]
        assets.extend({"id": doc["sourceAssetId"], "sha256": doc["sourceSha256"]} for doc in frozen["manifest"]["documents"])

        def read_assets() -> None:
            for asset in assets:
                payload, _ = request(f"/api/v1/assets/{asset['id']}/content")
                check(hashlib.sha256(payload).hexdigest() == asset["sha256"], "Frozen asset SHA256 through HTTPS: " + asset["id"])

        read_assets()
        providers, _ = data("/api/v1/settings/providers")
        check(not providers["active"]["tripo"]["keyConfigured"] and not providers["active"]["manualAi"]["keyConfigured"],
              "Isolated runtime has no configured generation-provider keys")
        # All five failures must count against the same actual client, regardless
        # of five forged client addresses. The sixth must be rate-limited.
        for index in range(5):
            request("/api/v1/auth/login", "POST", {"password": "wrong-fixture-" + secrets.token_hex(12)},
                    headers={"X-Forwarded-For": f"198.51.100.{index + 10}"}, expected=401)
        _, rate_headers = request("/api/v1/auth/login", "POST", {"password": fixture_password},
                                  headers={"X-Forwarded-For": "203.0.113.250"}, expected=429)
        check(bool(rate_headers.get("Retry-After")), "Forged XFF cannot bypass actual-client login failure limits")
        compose("stop", "-t", "60", "proxy", "manual", label="stop-for-restart")
        check(inspected(first_ids["manual"])["State"]["ExitCode"] == 0, "Production application exits cleanly on SIGTERM")
        compose("up", "-d", "--no-build", "--pull", "never", "--wait", "--wait-timeout", "180", label="restart")
        inspect_runtime("restarted")
        check(report["initialRuntime"]["manual"]["imageId"] == report["restartedRuntime"]["manual"]["imageId"] and
              report["initialRuntime"]["proxy"]["imageId"] == report["restartedRuntime"]["proxy"]["imageId"],
              "Restart uses exactly the same application/proxy image identities")
        cookies.clear()
        csrf = None
        login, _ = data("/api/v1/auth/login", "POST", {"password": fixture_password})
        csrf = login["csrfToken"]
        persisted, _ = data(f"/api/v1/items/{fixture_item_id}")
        check(persisted["model"] == "no-provider-requests", "TLS-created fixture item survives restart")
        reread, _ = data(f"/api/v1/items/{frozen_item_id}/releases/{frozen['id']}")
        check(reread["manifestSha256"] == frozen["manifestSha256"], "Frozen release manifest survives TLS restart unchanged")
        read_assets()
        report["release"] = {"id": frozen["id"], "manifestSha256": frozen["manifestSha256"], "assetHashes": assets}
        report["status"] = "passed"
    except Exception as error:
        report["error"] = str(error) if isinstance(error, VerificationError) else type(error).__name__
    finally:
        if stack_attempted:
            compose("logs", "--no-color", label="containers", must_pass=False)
        if not args.keep:
            # Remove the stopped loader before deleting its mounted data volume.
            if created_loader:
                run(["docker", "rm", "-f", loader], "cleanup-loader", must_pass=False)
            if stack_attempted:
                compose("down", "--volumes", "--remove-orphans", label="cleanup-project", must_pass=False)
            if created_volume:
                run(["docker", "volume", "rm", fixture_volume], "cleanup-fixture-volume", must_pass=False)
            failures = [entry["label"] for entry in report.get("commands", [])
                        if entry["label"].startswith("cleanup-") and entry["exitCode"] != 0]
            if failures:
                report["cleanupFailures"] = failures
                report["status"] = "failed"
                report.setdefault("error", "Cleanup of this verifier's isolated resources failed.")
        # No session/CSRF values are written to evidence. Do not publish private/.
        sensitive = [fixture_password.encode(), master_key.encode()]
        key_file = private / "tls-key.pem"
        if key_file.exists():
            sensitive.append(key_file.read_bytes().strip())
        for path in out.rglob("*"):
            if path.is_file() and private not in path.parents:
                if any(value in path.read_bytes() for value in sensitive if value):
                    report["status"] = "failed"
                    report["error"] = "A fixture secret appeared in public evidence."
                    break
        (out / "summary.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps({"status": report["status"], "checks": len(report["checks"]), "report": str(out / "summary.json")}, ensure_ascii=False))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
