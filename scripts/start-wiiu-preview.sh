#!/usr/bin/env bash
# Compatibility entry point: every preview now uses the shared library.
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
exec bash "$repo_root/scripts/start-project.sh" "$@"
