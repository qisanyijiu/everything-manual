#!/usr/bin/env python3
"""Consolidate preview libraries without changing or deleting any source.

Prepare mode makes isolated, migrated snapshots and a reviewable report. Apply
requires --sources-stopped, rechecks the same sources, and installs a new target.
No server/worker/provider process is started; encrypted settings are opaque.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import stat
import subprocess
import sys
import tomllib


EXCLUDED_TABLES = {"admins", "sessions", "_sqlx_migrations"}
IMMUTABLE_TABLES = {"manual_releases", "generation_snapshots", "quotes", "cost_ledger", "provider_attempts"}


def digest(path: Path) -> str:
    sha = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            sha.update(chunk)
    return sha.hexdigest()


def regular(path: Path) -> None:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"Expected a regular, non-symlink file: {path}")


def private_copy(source: Path, target: Path) -> None:
    regular(source)
    target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    shutil.copyfile(source, target)
    target.chmod(0o600)


def connect_read(path: Path) -> sqlite3.Connection:
    regular(path)
    return sqlite3.connect(f"file:{path.as_posix()}?mode=ro", uri=True, timeout=30)


def snapshot(source: Path, target: Path) -> None:
    target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with connect_read(source) as src, sqlite3.connect(target) as dst:
        src.backup(dst)
    target.chmod(0o600)


def q(value: str) -> str:
    return '"' + value.replace('"', '""') + '"'


def tables(connection: sqlite3.Connection) -> list[str]:
    return [row[0] for row in connection.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    )]


def columns(connection: sqlite3.Connection, table: str) -> list[str]:
    return [row[1] for row in connection.execute(f"PRAGMA table_info({q(table)})")]


def primary_key(connection: sqlite3.Connection, table: str) -> list[str]:
    return [row[1] for row in sorted(connection.execute(f"PRAGMA table_info({q(table)})"), key=lambda row: row[5]) if row[5]]


def counts(connection: sqlite3.Connection) -> dict[str, int]:
    return {name: connection.execute(f"SELECT count(*) FROM {q(name)}").fetchone()[0] for name in tables(connection)}


def check_database(connection: sqlite3.Connection) -> None:
    if connection.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
        raise ValueError("SQLite integrity_check failed")
    if connection.execute("PRAGMA foreign_key_check").fetchall():
        raise ValueError("SQLite foreign_key_check failed")


def inventory(directory: Path) -> dict:
    with connect_read(directory / "manual.sqlite3") as connection:
        check_database(connection)
        result = {"source": str(directory), "tables": counts(connection)}
        result["schemaVersion"] = connection.execute("SELECT max(version) FROM _sqlx_migrations WHERE success=1").fetchone()[0]
        result["itemIds"] = sorted(row[0] for row in connection.execute("SELECT id FROM items"))
        result["pendingJobs"] = connection.execute("SELECT count(*) FROM jobs WHERE status NOT IN ('succeeded', 'cancelled', 'failed', 'needs_input')").fetchone()[0]
        result["blobBytes"] = connection.execute("SELECT coalesce(sum(size), 0) FROM blobs WHERE storage_state='stored'").fetchone()[0]
        return result


def migrate(source: Path, target: Path, binary: Path, password: Path, log: Path) -> None:
    target.mkdir(mode=0o700)
    snapshot(source / "manual.sqlite3", target / "manual.sqlite3")
    # A fresh non-secret configuration avoids old absolute paths and provider IO.
    config = target / "migration-config.toml"
    config.write_text('listen = "127.0.0.1:8080"\n', encoding="utf-8")
    environment = {key: value for key, value in os.environ.items() if not key.startswith("EM_")}
    environment["EM_SECRETS_BACKEND"] = "file"
    with log.open("wb") as output:
        process = subprocess.run([
            str(binary), "init", "--data-dir", str(target), "--config", str(config),
            "--password-file", str(password),
        ], env=environment, stdout=output, stderr=subprocess.STDOUT, check=False)
    if process.returncode:
        raise ValueError(f"Isolated schema migration failed; see {log}")
    config.unlink()


def copy_blob(source: Path, destination: Path, sha: str, size: int) -> None:
    if len(sha) != 64 or any(character not in "0123456789abcdef" for character in sha):
        raise ValueError("Invalid blob SHA256")
    src = source / "blobs" / sha[:2] / sha
    regular(src)
    if src.stat().st_size != size or digest(src) != sha:
        raise ValueError(f"Source blob hash/size mismatch: {src}")
    dst = destination / "blobs" / sha[:2] / sha
    if dst.exists():
        if dst.stat().st_size != size or digest(dst) != sha:
            raise ValueError(f"Destination blob conflict: {sha}")
    else:
        private_copy(src, dst)


def merge(snapshots: list[Path], original_sources: list[Path], target: Path) -> dict:
    shutil.copytree(snapshots[0], target)
    database = target / "manual.sqlite3"
    db = sqlite3.connect(database)
    db.execute("PRAGMA foreign_keys=OFF")
    admin = db.execute("SELECT id FROM admins").fetchall()
    if len(admin) != 1:
        raise ValueError("Primary snapshot must contain exactly one admin")
    admin_id = admin[0][0]
    db.execute("DELETE FROM sessions")
    comparisons = []
    shared_blobs = 0
    for index, source in enumerate(snapshots):
        with connect_read(source / "manual.sqlite3") as src:
            if tables(src) != tables(db):
                raise ValueError("Migrated schemas have different table sets")
            for table in tables(src):
                if table in EXCLUDED_TABLES:
                    continue
                cols = columns(src, table)
                if cols != columns(db, table):
                    raise ValueError(f"Column mismatch: {table}")
                keys = primary_key(src, table)
                if not keys:
                    raise ValueError(f"Table has no primary key: {table}")
                select = f"SELECT {','.join(map(q, cols))} FROM {q(table)}"
                for original_row in src.execute(select):
                    row = list(original_row)
                    if table == "idempotency_records":
                        row[cols.index("admin_id")] = admin_id
                    where = " AND ".join(f"{q(key)}=?" for key in keys)
                    values = [row[cols.index(key)] for key in keys]
                    existing = db.execute(f"{select} WHERE {where}", values).fetchone()
                    if existing is not None:
                        if list(existing) != row:
                            if table != "blobs" or any(existing[position] != row[position] for position, name in enumerate(cols) if name != "created_at"):
                                raise ValueError(f"Conflicting entity: {table}; primary key {values}")
                        if index and table == "blobs":
                            shared_blobs += 1
                    else:
                        try:
                            db.execute(f"INSERT INTO {q(table)} ({','.join(map(q, cols))}) VALUES ({','.join('?' for _ in cols)})", row)
                        except sqlite3.IntegrityError as error:
                            raise ValueError(f"Constraint collision in {table}; no target installed") from error
                if table in IMMUTABLE_TABLES:
                    matched = all(db.execute(f"{select} WHERE " + " AND ".join(f"{q(key)}=?" for key in keys), [row[cols.index(key)] for key in keys]).fetchone() == row for row in src.execute(select))
                    if not matched:
                        raise ValueError(f"Immutable row mismatch: {table}")
                    comparisons.append({"sourceIndex": index, "table": table, "rows": src.execute(f"SELECT count(*) FROM {q(table)}").fetchone()[0], "exact": True})
            for sha, size, state in src.execute("SELECT sha256,size,storage_state FROM blobs"):
                if state == "stored":
                    copy_blob(original_sources[index], target, sha, size)
                elif state == "missing":
                    raise ValueError("Source contains a missing blob; resolve before migration")
    check_database(db)
    db.commit()
    db.execute("PRAGMA foreign_keys=ON")
    check_database(db)
    final_counts = counts(db)
    db.close()
    database.chmod(0o600)
    return {"tables": final_counts, "immutableRowChecks": comparisons, "sharedBlobRows": shared_blobs, "singleAdmin": True, "sessionsRemoved": True, "integrityCheck": "ok", "foreignKeyCheck": "ok"}


def write_config(source: Path, destination: Path, final_data: Path) -> dict:
    config = source / "config.toml"
    regular(config)
    parsed = tomllib.loads(config.read_text(encoding="utf-8"))
    allowed = {"public_origin", "price_catalog_path", "concurrency", "download", "data_dir", "listen"}
    if set(parsed) - allowed:
        raise ValueError("Source configuration contains unexpected fields; review without printing secrets")
    hosts = parsed.get("download", {}).get("allowed_hosts", [])
    if not isinstance(hosts, list) or not all(isinstance(host, str) for host in hosts):
        raise ValueError("Invalid download allowed_hosts")
    batches = parsed.get("concurrency", {}).get("manual_ai_batches", 1)
    if not isinstance(batches, int) or batches not in (1, 2):
        raise ValueError("Invalid concurrency")
    catalog_source = Path(parsed.get("price_catalog_path", source / "price-catalog.toml"))
    if not catalog_source.is_absolute():
        catalog_source = source / catalog_source
    # Do not follow an arbitrary deployment-config path outside this sample.
    if catalog_source.resolve().parent != source.resolve():
        raise ValueError("Price catalog must be directly inside the selected source")
    private_copy(catalog_source, destination / "price-catalog.toml")
    text = '\n'.join([
        'listen = "127.0.0.1:8080"',
        'public_origin = "http://127.0.0.1:8080"',
        f'price_catalog_path = {json.dumps(str(final_data / "price-catalog.toml"))}',
        '[concurrency]', f'manual_ai_batches = {batches}',
        '[download]', f'allowed_hosts = {json.dumps(hosts)}', '',
    ])
    (destination / "config.toml").write_text(text, encoding="utf-8")
    (destination / "config.toml").chmod(0o600)
    return {"listen": "127.0.0.1:8080", "publicOrigin": "http://127.0.0.1:8080", "priceCatalogSha256": digest(destination / "price-catalog.toml"), "downloadHostCount": len(hosts)}


def archive(source: Path, target: Path) -> dict:
    # Copy files verbatim except the database: online SQLite backup produces a
    # coherent snapshot even if an unrelated fixture reader is still running.
    target.mkdir(parents=True, mode=0o700)
    raw_hash = digest(source / "manual.sqlite3")
    raw_target = target / "original-database-files"
    raw_target.mkdir(mode=0o700)
    for name in ("manual.sqlite3", "manual.sqlite3-wal", "manual.sqlite3-shm"):
        path = source / name
        if path.exists():
            private_copy(path, raw_target / name)
    for path in source.iterdir():
        if path.name in {"manual.sqlite3", "manual.sqlite3-wal", "manual.sqlite3-shm", "lock", "tmp", "logs"}:
            continue
        if path.is_symlink():
            raise ValueError(f"Archive source contains a symlink: {path}")
        if path.is_dir():
            shutil.copytree(path, target / path.name, symlinks=True)
        elif path.is_file():
            private_copy(path, target / path.name)
    snapshot(source / "manual.sqlite3", target / "manual.sqlite3")
    return {"source": str(source), "archive": str(target), "originalDatabaseSha256": raw_hash, "snapshotSha256": digest(target / "manual.sqlite3"), "sourceRetained": True}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--source", type=Path, action="append", help="Data-dir; repeated. Default: Wii U then Nikon.")
    parser.add_argument("--target", type=Path, default=Path("var/preview/data"))
    parser.add_argument("--binary", type=Path, default=Path("var/delivery-20261004/bin/everything-manual-release"))
    parser.add_argument("--password-file", type=Path, default=Path("var/delivery-20261004/live-password.txt"))
    parser.add_argument("--report-dir", type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--sources-stopped", action="store_true", help="Operator confirms source preview services have stopped.")
    args = parser.parse_args()
    repo = args.repo.resolve()
    resolve = lambda value: (value if value.is_absolute() else repo / value).resolve()
    sources = [resolve(value) for value in (args.source or [Path("var/chrome-live-wiiu-20261003"), Path("var/delivery-20261004/live-instance")])]
    target, binary, password = map(resolve, (args.target, args.binary, args.password_file))
    regular(binary)
    regular(password)
    if stat.S_IMODE(password.stat().st_mode) & 0o077:
        raise ValueError("Password file must have mode 0600")
    if args.apply and not args.sources_stopped:
        raise ValueError("Apply requires --sources-stopped; stop only the source preview processes first")
    if target.exists():
        raise ValueError("Target already exists; no overwrite is permitted")
    if args.apply and (target.parent / "private").exists():
        raise ValueError("Private metadata target already exists; no overwrite is permitted")
    if any(target == source or source in target.parents or target in source.parents for source in sources):
        raise ValueError("Target and source directories must be independent")
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    report_dir = resolve(args.report_dir) if args.report_dir else target.parent / f"migration-{stamp}"
    report_dir.mkdir(parents=True, mode=0o700)
    report = {"mode": "apply" if args.apply else "prepare", "startedAt": stamp, "target": str(target), "sourcesRetained": True, "providerRequests": 0, "workerStarts": 0}
    report_path = report_dir / "report.json"
    try:
        report["sourceInventory"] = [inventory(source) for source in sources]
        if any(source["pendingJobs"] for source in report["sourceInventory"]):
            raise ValueError("A source has an active pending/running job; stop and resolve it before consolidation")
        migrated = []
        for index, source in enumerate(sources):
            dest = report_dir / f"source-{index}-migrated"
            migrate(source, dest, binary, password, report_dir / f"source-{index}-init.log")
            migrated.append(dest)
        staging = report_dir / "merged-data"
        report["merge"] = merge(migrated, sources, staging)
        overlay = sources[-1] / "provider-overrides.json"
        regular(overlay)
        overlay_hash = digest(overlay)
        for source in sources:
            existing = source / "provider-overrides.json"
            if existing.exists() and digest(existing) != overlay_hash:
                raise ValueError("Encrypted provider overlays differ; choose configuration explicitly before consolidation")
        private_copy(overlay, staging / "provider-overrides.json")
        report["encryptedOverlay"] = {"opaque": True, "bytes": overlay.stat().st_size, "sha256": overlay_hash}
        report["config"] = write_config(sources[-1], staging, target)
        private = report_dir / "private"
        private_copy(password, private / "password.txt")
        for name, value in (("secrets-backend", "keychain\n"), ("native-binary", str(binary) + "\n")):
            (private / name).write_text(value, encoding="utf-8")
            (private / name).chmod(0o600)
        merged_ids = set(report["sourceInventory"][0]["itemIds"])
        for source in report["sourceInventory"][1:]:
            merged_ids.update(source["itemIds"])
        histories = sorted({path.parent.resolve() for path in (repo / "var").rglob("manual.sqlite3") if target.parent not in path.parents and path.parent.resolve() not in sources and path.parent.name != "database"})
        report["historicalInventory"] = []
        for historical in histories:
            info = inventory(historical)
            info["itemIdsMissingFromMergedPreview"] = sorted(set(info["itemIds"]) - merged_ids)
            info["classification"] = "synthetic-or-fixture-archive" if info["itemIdsMissingFromMergedPreview"] else "previous-preview-snapshot-archive"
            report["historicalInventory"].append(info)
            if "preview-before" in historical.name or historical.name == "preview-20260919":
                if info["itemIdsMissingFromMergedPreview"]:
                    raise ValueError(f"Historical preview has items absent from selected sources: {historical}; include it explicitly")
        report["validation"] = "passed"
        if args.apply:
            archive_root = target.parent / "archive" / stamp
            archive_root.mkdir(parents=True, mode=0o700)
            report["archives"] = []
            for source in sources + histories:
                relative = source.relative_to(repo / "var")
                report["archives"].append(archive(source, archive_root / relative))
            if target.exists():
                raise ValueError("Target appeared during preparation; refusing overwrite")
            target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            private_target = target.parent / "private"
            if private_target.exists():
                raise ValueError("Private metadata target already exists; refusing overwrite")
            os.rename(private, private_target)
            try:
                os.rename(staging, target)
            except Exception:
                os.rename(private_target, private)
                raise
            report["installed"] = True
        else:
            report["installed"] = False
            report["nextAction"] = "Stop source services, take formal backups, then repeat with --apply --sources-stopped"
    except Exception as error:
        report["validation"] = "failed"
        report["error"] = str(error)
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        report_path.chmod(0o600)
        print(f"Consolidation stopped; sources were not changed. Report: {report_path}", file=sys.stderr)
        return 1
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    report_path.chmod(0o600)
    print(json.dumps({"validation": "passed", "installed": report["installed"], "items": report["merge"]["tables"]["items"], "assets": report["merge"]["tables"]["assets"], "report": str(report_path)}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
