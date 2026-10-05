#!/usr/bin/env python3
"""Move verified preview originals into one archive, retaining relative aliases.

Default mode only validates and writes a private journal. Apply requires the
operator to confirm that source services stopped. No files are deleted, no
server/worker is launched, and private configuration contents are never logged.
"""
from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import fcntl
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import sys
from urllib.parse import quote


def absolute(path: Path, repo: Path) -> Path:
    return Path(os.path.abspath(path if path.is_absolute() else repo / path))


def inside(path: Path, root: Path) -> bool:
    return path != root and root in path.parents


def nonsymlink(path: Path, root: Path, *, directory: bool = False) -> None:
    if path != root and not inside(path, root):
        raise ValueError(f"Path is outside its permitted root: {path}")
    component = root
    for part in (None, *path.relative_to(root).parts):
        if part is not None:
            component = component / part
        if component.is_symlink():
            raise ValueError(f"Symlink is not permitted: {component}")
        if component != path and component.exists() and not component.is_dir():
            raise ValueError(f"Parent is not a directory: {component}")
    if directory and not path.is_dir():
        raise ValueError(f"Expected an existing directory: {path}")


def regular(path: Path, root: Path) -> None:
    nonsymlink(path, root)
    if not path.is_file():
        raise ValueError(f"Expected a regular file: {path}")


def digest(path: Path) -> str:
    sha = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            sha.update(chunk)
    return sha.hexdigest()


def quoted(value: str) -> str:
    return '"' + value.replace('"', '""') + '"'


def logical_database(path: Path) -> dict:
    """Compare all SQL data, including WAL, without storing row contents."""
    uri = f"file:{quote(str(path), safe='/')}?mode=ro"
    with sqlite3.connect(uri, uri=True, timeout=5) as connection:
        connection.execute("PRAGMA query_only=ON")
        connection.execute("BEGIN")
        if connection.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
            raise ValueError(f"SQLite integrity check failed: {path}")
        if connection.execute("PRAGMA foreign_key_check").fetchall():
            raise ValueError(f"SQLite foreign-key check failed: {path}")
        schema = connection.execute(
            "SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name"
        ).fetchall()
        schema_sha = hashlib.sha256(json.dumps(schema, separators=(",", ":")).encode()).hexdigest()
        data_sha = hashlib.sha256()
        counts = {}
        for name, in connection.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").fetchall():
            columns = [row[1] for row in connection.execute(f"PRAGMA table_info({quoted(name)})")]
            data_sha.update(json.dumps([name, columns], separators=(",", ":")).encode() + b"\n")
            count = 0
            order = ",".join(quoted(column) for column in columns)
            for row in connection.execute(f"SELECT * FROM {quoted(name)} ORDER BY {order}"):
                encoded = [("blob", value.hex()) if isinstance(value, bytes) else (type(value).__name__, value) for value in row]
                data_sha.update(json.dumps(encoded, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode() + b"\n")
                count += 1
            counts[name] = count
        return {"schemaSha256": schema_sha, "rowsSha256": data_sha.hexdigest(), "tableCounts": counts}


def tree_inventory(directory: Path) -> dict:
    fingerprint = hashlib.sha256()
    files = 0
    size = 0
    for parent, directories, names in os.walk(directory, followlinks=False):
        directories.sort()
        names.sort()
        for name in directories + names:
            path = Path(parent) / name
            info = path.lstat()
            if not (stat.S_ISDIR(info.st_mode) or stat.S_ISREG(info.st_mode)):
                raise ValueError(f"Archive contains a symlink or special file: {path}")
            if stat.S_ISREG(info.st_mode):
                files += 1
                size += info.st_size
                value = [str(path.relative_to(directory)), info.st_dev, info.st_ino, stat.S_IMODE(info.st_mode), info.st_size, info.st_mtime_ns]
                fingerprint.update(json.dumps(value, separators=(",", ":")).encode() + b"\n")
    return {"fileCount": files, "bytes": size, "metadataSha256": fingerprint.hexdigest()}


def fsync_directory(directory: Path) -> None:
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def save_journal(path: Path, value: dict, *, first: bool = False) -> None:
    data = (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()
    temporary = path.with_name(path.name + ".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        if first and os.path.lexists(path):
            raise ValueError("Journal already exists; refusing overwrite")
        os.replace(temporary, path)
        fsync_directory(path.parent)
    finally:
        if temporary.exists():
            temporary.unlink()


def create_parents(directory: Path, root: Path) -> None:
    nonsymlink(directory, root)
    missing = []
    current = directory
    while not current.exists():
        missing.append(current)
        current = current.parent
    if not current.is_dir():
        raise ValueError(f"Parent is not a directory: {current}")
    for path in reversed(missing):
        path.mkdir(mode=0o700)
        fsync_directory(path.parent)


def nearest_existing(directory: Path) -> Path:
    while not directory.exists():
        directory = directory.parent
    return directory


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--report", required=True, type=Path, help="Successful consolidation --apply report.json")
    parser.add_argument("--archive-root", type=Path, default=Path("var/preview/archive/originals"))
    parser.add_argument("--journal", type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--sources-stopped", action="store_true")
    args = parser.parse_args()
    repo = args.repo.resolve()
    var = repo / "var"
    preview = var / "preview"
    archive = preview / "archive"
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    report_path = absolute(args.report, repo)
    destination_root = absolute(args.archive_root, repo)
    journal_path = absolute(args.journal, repo) if args.journal else archive / f"originals-journal-{stamp}.json"
    journal = {"mode": "apply" if args.apply else "dry-run", "startedAt": stamp, "status": "validating", "report": str(report_path), "archiveRoot": str(destination_root), "providerRequests": 0, "workerStarts": 0, "moves": []}
    journal_started = False
    with contextlib.ExitStack() as stack:
        try:
            nonsymlink(var, repo, directory=True)
            nonsymlink(archive, repo, directory=True)
            regular(report_path, repo)
            if destination_root != archive / "originals" and not inside(destination_root, archive):
                raise ValueError("Archive root must be under var/preview/archive")
            nonsymlink(destination_root, repo)
            if not inside(journal_path, archive) or inside(journal_path, destination_root):
                raise ValueError("Journal must be inside archive and outside originals")
            nonsymlink(journal_path, repo)
            if os.path.lexists(journal_path):
                raise ValueError("Journal already exists; use a new path")
            if not journal_path.parent.is_dir():
                raise ValueError("Journal parent must already exist")
            if args.apply and not args.sources_stopped:
                raise ValueError("Apply requires --sources-stopped after checking all source services")
            report = json.loads(report_path.read_text(encoding="utf-8"))
            if report.get("mode") != "apply" or report.get("validation") != "passed" or report.get("installed") is not True:
                raise ValueError("Expected a successful, installed consolidation apply report")
            entries = report.get("archives")
            if not isinstance(entries, list) or not entries:
                raise ValueError("Consolidation report has no original sources")
            sources = []
            for entry in entries:
                source = absolute(Path(entry["source"]), repo)
                if not inside(source, var) or inside(source, preview):
                    raise ValueError(f"Source must be outside preview and inside var: {source}")
                nonsymlink(source, repo, directory=True)
                archived = absolute(Path(entry["archive"]), repo)
                if not inside(archived, archive) or inside(archived, destination_root):
                    raise ValueError("Expected an existing consolidation snapshot under archive")
                regular(archived / "manual.sqlite3", repo)
                sha = entry.get("originalDatabaseSha256", "")
                if len(sha) != 64 or any(character not in "0123456789abcdef" for character in sha):
                    raise ValueError("Invalid source database SHA256 in report")
                if digest(archived / "manual.sqlite3") != entry.get("snapshotSha256"):
                    raise ValueError(f"Consolidation snapshot changed: {archived}")
                regular(source / "manual.sqlite3", repo)
                target = destination_root / source.relative_to(var)
                nonsymlink(target, repo)
                if os.path.lexists(target):
                    raise ValueError(f"Destination already exists: {target}")
                if source.stat().st_dev != nearest_existing(target.parent).stat().st_dev:
                    raise ValueError("Source and archive must be on the same filesystem")
                sources.append(source)
                journal["moves"].append({"source": str(source), "destination": str(target), "relativeAlias": os.path.relpath(target, source.parent), "snapshot": str(archived), "originalDatabaseSha256": sha, "status": "planned"})
            if len(set(sources)) != len(sources) or any(a in b.parents or b in a.parents for index, a in enumerate(sources) for b in sources[index + 1:]):
                raise ValueError("Duplicate or ancestor/descendant source directories are not allowed")
            save_journal(journal_path, journal, first=True)
            journal_started = True
            # Existing application locks are acquired without creating source files.
            for source in sources:
                lock = source / "lock"
                if lock.exists() or lock.is_symlink():
                    regular(lock, repo)
                    stream = stack.enter_context(lock.open("rb"))
                    try:
                        fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                    except BlockingIOError as error:
                        raise ValueError(f"Source is still locked by another process: {source}") from error
            for move in journal["moves"]:
                source = Path(move["source"])
                current = logical_database(source / "manual.sqlite3")
                archived = logical_database(Path(move["snapshot"]) / "manual.sqlite3")
                if current != archived:
                    raise ValueError(f"Source SQL data differs from its consolidation snapshot: {source}")
                move["databaseSha256"] = digest(source / "manual.sqlite3")
                move["originalDatabaseHashMatches"] = move["databaseSha256"] == move["originalDatabaseSha256"]
                move["logicalDatabase"] = current
                move["tree"] = tree_inventory(source)
                move["directoryIdentity"] = [source.stat().st_dev, source.stat().st_ino]
                move["status"] = "validated"
                save_journal(journal_path, journal)
            journal["status"] = "preflight-passed"
            journal["sourceCount"] = len(sources)
            save_journal(journal_path, journal)
            if args.apply:
                # All sources are checked before the first original is moved.
                for move in journal["moves"]:
                    source, target = Path(move["source"]), Path(move["destination"])
                    create_parents(target.parent, repo)
                    nonsymlink(source, repo, directory=True)
                    if os.path.lexists(target) or digest(source / "manual.sqlite3") != move["databaseSha256"] or tree_inventory(source) != move["tree"]:
                        raise ValueError(f"A source or destination changed after preflight: {source}")
                    move["status"] = "moving"
                    journal["status"] = "applying"
                    save_journal(journal_path, journal)
                    os.rename(source, target)
                    fsync_directory(source.parent)
                    fsync_directory(target.parent)
                    move["status"] = "renamed"
                    save_journal(journal_path, journal)
                    os.symlink(move["relativeAlias"], source, target_is_directory=True)
                    fsync_directory(source.parent)
                    move["status"] = "linked"
                    save_journal(journal_path, journal)
                    if not source.is_symlink() or os.readlink(source) != move["relativeAlias"] or source.resolve() != target.resolve():
                        raise ValueError(f"Compatibility alias verification failed: {source}")
                    if [target.stat().st_dev, target.stat().st_ino] != move["directoryIdentity"] or digest(target / "manual.sqlite3") != move["databaseSha256"] or tree_inventory(target) != move["tree"]:
                        raise ValueError(f"Moved original changed: {target}")
                    move["status"] = "verified"
                    save_journal(journal_path, journal)
                journal["status"] = "passed"
                journal["movedCount"] = len(sources)
                save_journal(journal_path, journal)
            print(json.dumps({"status": journal["status"], "sources": len(sources), "moved": journal.get("movedCount", 0), "journal": str(journal_path)}, ensure_ascii=False))
            return 0
        except Exception as error:
            journal["status"] = "failed"
            journal["error"] = str(error)
            journal["recovery"] = "No automatic rollback. For each renamed original, verify its destination; remove only a matching compatibility symlink, then rename the original back. Never delete a data directory. The moving/renamed/linked journal entries retain both paths."
            if journal_started:
                try:
                    save_journal(journal_path, journal)
                except Exception:
                    print(f"Journal update failed; inspect the existing journal and recorded paths: {journal_path}", file=sys.stderr)
            print(f"Archive stopped: {error}. Journal: {journal_path if journal_started else 'not created'}", file=sys.stderr)
            return 1


if __name__ == "__main__":
    sys.exit(main())
