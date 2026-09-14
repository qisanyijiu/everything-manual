#!/usr/bin/env python3
"""把 Cargo.lock 里 registry 包的 .crate 从 rsproxy 镜像预取进本地 cargo 缓存，并逐个比对
Cargo.lock 的 sha256。

背景（2026-09-13，T22 macOS 复跑）：
- 本机 ~/.cargo/registry 被清空（cargo clean 相关的磁盘清理），host 直连 static.crates.io
  实测约 8 KB/s、index.crates.io 约 28 KB/s，冷缓存构建不可行。
- 旧证据二进制内嵌 564 处 `registry/src/index.crates.io-1949cf8c6b5b557f/...`：**索引目录名
  是哈希敏感输入**，因此不能换用 rsproxy 索引（否则解包目录名变化 → 内嵌路径变化 → sha256 变化）。
  索引仍必须走官方 crates.io sparse 索引；只有 .crate 传输走镜像。
- 下载内容由 Cargo.lock 的 checksum 校验（内容与 crates.io 一致才算数；不一致立即报错退出）。

用法：python3 seed-crate-cache.py [--lock Cargo.lock] [--cache-dir DIR] [--jobs N]
"""

import argparse
import concurrent.futures
import hashlib
import os
import re
import sys
import urllib.request

LOCK_ENTRY = re.compile(r"^\[\[package\]\]$")
RS_PROXY = "https://rsproxy.cn/api/v1/crates/{name}/{version}/download"


def parse_lock(path: str):
    packages = []
    current = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if LOCK_ENTRY.match(line):
                current = {}
                packages.append(current)
                continue
            for key in ("name", "version", "checksum", "source"):
                prefix = f'{key} = "'
                if line.startswith(prefix) and line.endswith('"'):
                    current[key] = line[len(prefix) : -1]
    result = []
    for package in packages:
        source = package.get("source", "")
        if source == "registry+https://github.com/rust-lang/crates.io-index":
            result.append(
                (package["name"], package["version"], package["checksum"])
            )
    return result


def download(name: str, version: str, checksum: str, cache_dir: str):
    target = os.path.join(cache_dir, f"{name}-{version}.crate")
    if os.path.isfile(target) and os.path.getsize(target) > 0:
        with open(target, "rb") as handle:
            digest = hashlib.sha256(handle.read()).hexdigest()
        if digest == checksum:
            return ("cached", name, version)
        os.remove(target)
    url = RS_PROXY.format(name=name, version=version)
    last_error = None
    for _ in range(3):
        try:
            with urllib.request.urlopen(url, timeout=120) as response:
                payload = response.read()
            break
        except Exception as error:  # noqa: BLE001 - 重试后统一报错
            last_error = error
    else:
        return ("fail", name, version, str(last_error))
    digest = hashlib.sha256(payload).hexdigest()
    if digest != checksum:
        return ("checksum", name, version, digest, checksum)
    # 原子写入：并发运行的 cargo 可能同时检查该缓存文件，避免读到半截内容。
    temporary = f"{target}.seed-tmp"
    with open(temporary, "wb") as handle:
        handle.write(payload)
    os.replace(temporary, target)
    return ("ok", name, version)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lock", default="Cargo.lock")
    parser.add_argument(
        "--cache-dir",
        default=os.path.expanduser(
            "~/.cargo/registry/cache/index.crates.io-1949cf8c6b5b557f"
        ),
    )
    parser.add_argument("--jobs", type=int, default=12)
    args = parser.parse_args()

    os.makedirs(args.cache_dir, exist_ok=True)
    packages = parse_lock(args.lock)
    print(f"registry 包数：{len(packages)}；缓存目录：{args.cache_dir}", flush=True)

    stats = {"ok": 0, "cached": 0, "fail": 0, "checksum": 0}
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [
            pool.submit(download, name, version, checksum, args.cache_dir)
            for name, version, checksum in packages
        ]
        for index, future in enumerate(concurrent.futures.as_completed(futures), 1):
            result = future.result()
            stats[result[0]] += 1
            if result[0] in ("fail", "checksum"):
                print(f"  [异常] {result}", flush=True)
            if index % 50 == 0:
                print(f"  进度 {index}/{len(packages)}：{stats}", flush=True)

    print(f"完成：{stats}", flush=True)
    return 1 if stats["fail"] or stats["checksum"] else 0


if __name__ == "__main__":
    sys.exit(main())
