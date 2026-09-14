#!/usr/bin/env python3
"""把 Cargo.lock 里 registry 包在官方 sparse 索引（index.crates.io）的条目补全到 cargo 的
本地索引缓存，供冷缓存构建使用。

为什么需要它（2026-09-13，T22 macOS 复跑环境）：
- 本机 ~/.cargo/registry 被清空；宿主直连 index.crates.io 单流实测约 12–28 KB/s（web-sys
  单文件 4.4 MB），cargo 自己顺序拉索引要数十分钟到数小时。
- 索引内容必须来自**官方 index.crates.io**：旧证据二进制内嵌 564 处
  `registry/src/index.crates.io-1949cf8c6b5b557f/...`，换镜像索引会改索引目录名 /
  依赖元数据来源，属哈希敏感输入（ADR-010 的 CARGO_SOURCE_* 镜像机制在本机 cargo 1.98.1
  实测不生效，见 implementation 记录）。
- 本脚本并行拉官方条目（只改变拉取并发，不改变内容），并按 cargo 源码
  `src/cargo/sources/registry/index/cache.rs`（cargo 0.98.0，INDEX_V_MAX=2、
  CURRENT_CACHE_VERSION=3）写出缓存文件：
    [0x03][u32le(2)][index_version 字符串][0]
    之后每条： [semver][0][JSON 行][0]
  index_version 用响应头的 `etag: "..."`（缺失时 `last-modified: ...`，再缺为 `Unknown`），
  与 cargo 自己写入的格式一致。
- cargo 在 `--offline`（或本次未请求更新）时直接使用缓存条目，不再发请求。

用法：python3 seed-index-cache.py [--lock Cargo.lock] [--index-root DIR] [--jobs N]
"""

import argparse
import concurrent.futures
import json
import os
import re
import struct
import sys
import time
import urllib.request

BASE = "https://index.crates.io"
LOCK_PACKAGE = re.compile(r"^\[\[package\]\]$")


def parse_lock(path: str):
    names = []
    current = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if LOCK_PACKAGE.match(line):
                current = {}
                continue
            match = re.match(r'^name = "(.*)"$', line)
            if match:
                current["name"] = match.group(1)
            if "crates.io-index" in line and current.get("name"):
                names.append(current["name"])
                current = {}
    return sorted(set(names))


def index_path(name: str) -> str:
    length = len(name)
    if length == 1:
        return f"1/{name}"
    if length == 2:
        return f"2/{name}"
    if length == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[:2]}/{name[2:4]}/{name}"


def build_cache(body: bytes, index_version: str) -> bytes:
    out = bytearray()
    out.append(3)
    out.extend(struct.pack("<I", 2))
    out.extend(index_version.encode())
    out.append(0)
    for line in body.decode("utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        version = json.loads(line)["vers"]
        out.extend(version.encode())
        out.append(0)
        out.extend(line.encode())
        out.append(0)
    return bytes(out)


def fetch(name: str, cache_root: str):
    path = index_path(name)
    target = os.path.join(cache_root, path)
    if os.path.isfile(target) and os.path.getsize(target) > 0:
        return ("present", name, 0)
    url = f"{BASE}/{path}"
    last_error = None
    for attempt in range(3):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "cargo-seed/1.0"})
            with urllib.request.urlopen(request, timeout=180) as response:
                body = response.read()
                etag = response.headers.get("ETag")
                last_modified = response.headers.get("Last-Modified")
            if not body:
                return ("fail", name, "空响应")
            if etag:
                index_version = f"etag: {etag}"
            elif last_modified:
                index_version = f"last-modified: {last_modified}"
            else:
                index_version = "Unknown"
            payload = build_cache(body, index_version)
            os.makedirs(os.path.dirname(target), exist_ok=True)
            temporary = f"{target}.seed-tmp"
            with open(temporary, "wb") as handle:
                handle.write(payload)
            os.replace(temporary, target)
            return ("ok", name, len(body))
        except Exception as error:  # noqa: BLE001 - 重试后统一报错
            last_error = error
            time.sleep(1 + attempt)
    return ("fail", name, str(last_error))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lock", default="Cargo.lock")
    parser.add_argument(
        "--index-root",
        default=os.path.expanduser(
            "~/.cargo/registry/index/index.crates.io-1949cf8c6b5b557f/.cache"
        ),
    )
    parser.add_argument("--jobs", type=int, default=16)
    args = parser.parse_args()

    names = parse_lock(args.lock)
    print(f"registry 唯一包名={len(names)}；缓存根={args.index_root}", flush=True)
    os.makedirs(args.index_root, exist_ok=True)

    stats = {"ok": 0, "present": 0, "fail": 0}
    bytes_fetched = 0
    started = time.time()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(fetch, name, args.index_root) for name in names]
        for index, future in enumerate(concurrent.futures.as_completed(futures), 1):
            kind, name, size = future.result()
            stats[kind] += 1
            if kind == "ok":
                bytes_fetched += size
            elif kind == "fail":
                print(f"  [失败] {name}: {size}", flush=True)
            if index % 25 == 0:
                print(
                    f"  进度 {index}/{len(names)}：{stats} 已拉取 {bytes_fetched/1024:.0f} KiB "
                    f"耗时 {time.time()-started:.0f}s",
                    flush=True,
                )
    print(
        f"完成：{stats}；拉取 {bytes_fetched/1024/1024:.1f} MiB；"
        f"总耗时 {time.time()-started:.0f}s",
        flush=True,
    )
    return 1 if stats["fail"] else 0


if __name__ == "__main__":
    sys.exit(main())
