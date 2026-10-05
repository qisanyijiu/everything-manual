#!/usr/bin/env bash
# Verify an already-built image in independent Compose projects and named volumes.
# Requires Docker Compose v2+, Python 3.11+, and an existing fixture sample backup.
# Never uses real preview data/credentials or generation APIs. One HTTPS
# example.com/models probe sends a random fake QA token and incurs no API fee.
set -euo pipefail
umask 077
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${EM_IMAGE:-everything-manual:local}"
SAMPLE_BACKUP=""
OUT_DIR=""
HTTP_PORT="${EM_DOCKER_VERIFY_PORT:-18090}"
KEEP=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --image) IMAGE="$2"; shift 2 ;;
    --sample-backup) SAMPLE_BACKUP="$2"; shift 2 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    --port) HTTP_PORT="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h|--help)
      printf '%s\n' 'Usage: scripts/verify-docker.sh --sample-backup DIR [--image IMAGE] [--out DIR] [--port 18090] [--keep]'
      exit 0 ;;
    *) printf 'Unknown argument: %s\n' "$1" >&2; exit 2 ;;
  esac
done
[[ -n "$SAMPLE_BACKUP" ]] || { printf '%s\n' '--sample-backup is required for complete release/PDF/GLB verification.' >&2; exit 2; }
SAMPLE_BACKUP="$(cd "$SAMPLE_BACKUP" && pwd -P)"
[[ -f "$SAMPLE_BACKUP/database/manual.sqlite3" && -f "$SAMPLE_BACKUP/manifest.json" ]] || { printf '%s\n' 'The sample backup is incomplete.' >&2; exit 2; }
case "$HTTP_PORT" in ''|*[!0-9]*) exit 2 ;; esac
OUT_DIR="${OUT_DIR:-$REPO_ROOT/var/preview/validation/docker-$(date +%Y%m%d)/run-$(date -u +%Y%m%dT%H%M%SZ)-$$}"
[[ ! -e "$OUT_DIR" ]] || { printf '%s\n' 'The evidence directory already exists; refusing overwrite.' >&2; exit 2; }
mkdir -p "$OUT_DIR/private"
OUT_DIR="$(cd "$OUT_DIR" && pwd -P)"
python3 - "$OUT_DIR/private" <<'PY'
import os, pathlib, secrets, sys
p=pathlib.Path(sys.argv[1])
for name,value in [('password.txt','test-password-t20-backup\n'), ('master.key',secrets.token_hex(32)+'\n'), ('canaries.txt','docker-qa-'+secrets.token_hex(24)+'\n'+'docker-qa-'+secrets.token_hex(24)+'\n')]:
    with open(p/name,'x') as f: f.write(value)
    os.chmod(p/name,0o600)
PY
export EM_IMAGE="$IMAGE"
export EM_ADMIN_PASSWORD_FILE="$OUT_DIR/private/password.txt"
export EM_MASTER_KEY_FILE="$OUT_DIR/private/master.key"
export EM_RUNTIME_UID=10001 EM_RUNTIME_GID=10001
export EM_HTTP_PORT="$HTTP_PORT" EM_PUBLIC_ORIGIN="http://127.0.0.1:$HTTP_PORT"
# Named volumes deliberately override a caller's real deployment bind mounts.
export EM_DATA_PATH=manual-data EM_RUNTIME_CONFIG_PATH=manual-config
export EM_CONFIG_FILE="$REPO_ROOT/docker/config.toml"
export EM_DOCKER_PLATFORM="${EM_DOCKER_PLATFORM:-linux/amd64}"
unset EM_CONTAINER_PRICE_CATALOG_PATH
BASE_URL="http://127.0.0.1:$HTTP_PORT"
PROJECT="em-verify-$(date -u +%Y%m%d%H%M%S)-$$"
CURRENT_PROJECT="$PROJECT-empty"
PROJECTS=("$PROJECT-empty" "$PROJECT-sample" "$PROJECT-restore" "$PROJECT-tls")
EXTRA_CONTAINERS=()
EXTRA_VOLUMES=()
STATUS=failed

compose() { docker compose --project-directory "$REPO_ROOT" -f "$REPO_ROOT/compose.yaml" -p "$CURRENT_PROJECT" "$@"; }
cleanup() {
  local exit_code=$?
  trap - EXIT
  set +e
  for project in "${PROJECTS[@]}"; do
    CURRENT_PROJECT="$project"
    compose logs --no-color >"$OUT_DIR/$project-containers.log" 2>&1
    if [[ "$KEEP" -eq 0 ]]; then compose down --volumes --remove-orphans >"$OUT_DIR/$project-cleanup.log" 2>&1; fi
  done
  if [[ "$KEEP" -eq 0 ]]; then
    for container in ${EXTRA_CONTAINERS[@]+"${EXTRA_CONTAINERS[@]}"}; do docker rm -f "$container" >/dev/null 2>&1; done
    for volume in ${EXTRA_VOLUMES[@]+"${EXTRA_VOLUMES[@]}"}; do docker volume rm "$volume" >/dev/null 2>&1; done
  fi
  python3 - "$OUT_DIR" "$STATUS" "$exit_code" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1]); reports=[]
for f in sorted(p.glob('*-http.json')):
    r=json.loads(f.read_text());reports.append({'file':f.name,'status':r['status'],'checks':len(r['checks'])})
requests=sum(json.loads(f.read_text()).get('providerRequests',0) for f in p.glob('*-http.json'))
(p/'summary.json').write_text(json.dumps({'status':sys.argv[2],'exitCode':int(sys.argv[3]),'paidRequests':0,'providerRequests':requests,'fixtureOnly':True,'httpReports':reports},indent=2)+'\n')
PY
  printf 'Docker verification %s; evidence: %s\n' "$STATUS" "$OUT_DIR"
  exit "$exit_code"
}
trap cleanup EXIT
docker version >"$OUT_DIR/docker-version.txt"
docker compose version >"$OUT_DIR/compose-version.txt"
docker image inspect "$IMAGE" >"$OUT_DIR/image.json"
# The local tag may be rebuilt in another terminal during verification.
# Pin all Compose and maintenance containers to the same immutable image ID.
IMAGE="$(docker image inspect --format '{{.Id}}' "$IMAGE")"
export EM_IMAGE="$IMAGE"
PROXY_IMAGE="${EM_PROXY_IMAGE:-nginx:1.28-alpine}"
docker image inspect "$PROXY_IMAGE" >"$OUT_DIR/proxy-image.json"
# A containerd multi-platform index may have no default-host architecture.
# The repository digest remains resolvable for the explicit target platform;
# its config ID alone is not an image reference on this engine.
PROXY_DIGEST="$(python3 - "$OUT_DIR/proxy-image.json" <<'PY'
import json,sys
digests=json.load(open(sys.argv[1]))[0].get('RepoDigests') or []
print(digests[0] if digests else '')
PY
)"
export EM_PROXY_IMAGE="${PROXY_DIGEST:-$PROXY_IMAGE}"
compose config --quiet
compose config >"$OUT_DIR/compose-resolved.yaml"

http_check() {
  local name="$1" mode="$2" state="$3"
  shift 3
  python3 "$REPO_ROOT/scripts/verify-docker-http.py" "$mode" \
    --base-url "$BASE_URL" --password-file "$OUT_DIR/private/password.txt" \
    --canary-file "$OUT_DIR/private/canaries.txt" --state "$OUT_DIR/$state" \
    --report "$OUT_DIR/$name-http.json" --fixtures "$REPO_ROOT/tests/fixtures/assets" "$@"
}

inspect_runtime() {
  local label="$1" manual prepare
  manual="$(compose ps -q manual)"
  prepare="$(compose ps -q prepare)"
  docker inspect "$manual" >"$OUT_DIR/$label-runtime.json"
  docker inspect "$prepare" >"$OUT_DIR/$label-prepare.json"
  docker exec "$manual" sh -c '
    set -eu
    test "$(id -u)" = 10001
    executable=$(readlink /proc/1/exe)
    case "$executable" in
      /usr/local/bin/everything-manual) ;;
      */rosetta) tr "\000" " " </proc/1/cmdline | grep -q /usr/local/bin/everything-manual ;;
      *) exit 1 ;;
    esac
    printf "PID 1 executable: %s\n" "$executable"
    tr "\000" " " </proc/1/cmdline
    printf "\n"
    ! command -v node
    ! command -v python
    ! command -v python3
    test ! -d /src/everything-manual
    test "$(stat -c %a /runtime-config)" = 700
    test "$(stat -c %a /run/private/master.key)" = 600
    test "$(stat -c %a /run/private/password.txt)" = 600
    if touch /run/private/docker-write-probe 2>/dev/null; then rm /run/private/docker-write-probe; exit 1; fi
    sha256sum /usr/local/bin/everything-manual
    (cd /usr/local/bin && sha256sum -c /opt/everything-manual/SHA256SUMS)
    uname -a
    grep "^Uid:" /proc/1/status
  ' >"$OUT_DIR/$label-runtime-contract.txt"
  docker cp "$manual:/opt/everything-manual/build-info.json" "$OUT_DIR/$label-build-info.json"
  docker cp "$manual:/opt/everything-manual/dynamic-dependencies.txt" "$OUT_DIR/$label-dynamic-dependencies.txt"
  docker exec "$prepare" sh -c 'grep "^Uid:" /proc/1/status; test "$(awk "/^Uid:/{print \$2}" /proc/1/status)" = 10001' >"$OUT_DIR/$label-prepare-uid.txt"
  docker volume inspect "${CURRENT_PROJECT}_private-runtime" >"$OUT_DIR/$label-secret-volume.json"
  python3 - "$OUT_DIR" "$label" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1]);label=sys.argv[2]
r=json.loads((p/f'{label}-runtime.json').read_text())[0]
assert r['HostConfig']['ReadonlyRootfs']
assert r['Config']['User']=='10001:10001'
assert r['HostConfig']['CapDrop']==['ALL']
assert '/tmp' in r['HostConfig']['Tmpfs']
assert all(entry['HostIp']=='127.0.0.1' for entries in r['HostConfig']['PortBindings'].values() for entry in entries)
secret=json.loads((p/f'{label}-secret-volume.json').read_text())[0]
assert secret['Options']['type']=='tmpfs'
build=json.loads((p/f'{label}-build-info.json').read_text())
assert build['target'] in ['x86_64-unknown-linux-musl','aarch64-unknown-linux-musl']
assert build['dynamicDependencies']['staticLinked'] is True
assert build['isolation']['hits']==0
assert 'embedded-ui' in build['features'] and 'job-failpoints' not in build['features']
dependencies=(p/f'{label}-dynamic-dependencies.txt').read_text()
assert '(NEEDED)' not in dependencies
assert 'DT_NEEDED 条目数：0' in dependencies
for value in [*(p/'private/canaries.txt').read_text().splitlines(),(p/'private/master.key').read_text().strip()]:
    assert value not in json.dumps(r)
PY
}

restart_and_check() {
  local label="$1" state="$2"
  shift 2
  local manual
  manual="$(compose ps -q manual)"
  compose stop -t 60 proxy manual >"$OUT_DIR/$label-stop.log" 2>&1
  [[ "$(docker inspect -f '{{.State.ExitCode}}' "$manual")" = 0 ]]
  compose up -d --no-build --pull never --wait --wait-timeout 180 >"$OUT_DIR/$label-restart.log" 2>&1
  http_check "$label" read "$state" "$@"
}

create_volume() {
  local volume="$1"
  docker volume create "$volume" >/dev/null
  EXTRA_VOLUMES+=("$volume")
  docker run --rm --platform "$EM_DOCKER_PLATFORM" --network none --user 0:0 --entrypoint sh -v "$volume:/volume" "$IMAGE" \
    -c 'set -eu; chown 10001:10001 /volume; chmod 700 /volume'
}

restore_backup() {
  local data_volume="$1" backup_volume="$2" backup_dir="$3" label="$4"
  local container="$PROJECT-$label-loader"
  create_volume "$data_volume"
  create_volume "$backup_volume"
  docker create --name "$container" --platform "$EM_DOCKER_PLATFORM" --network none --user 0:0 --entrypoint sh \
    -v "$data_volume:/data" -v "$backup_volume:/fixtures" "$IMAGE" -c \
    '/usr/local/bin/everything-manual restore --from /fixtures/input --data-dir /data && chown -R 10001:10001 /data' >/dev/null
  EXTRA_CONTAINERS+=("$container")
  docker cp "$backup_dir" "$container:/fixtures/input"
  docker start -a "$container" >"$OUT_DIR/$label-restore.log" 2>&1
  [[ "$(docker inspect -f '{{.State.ExitCode}}' "$container")" = 0 ]]
}

printf '%s\n' '[1/6] Empty-data initialization, authentication, embedded UI/PDF.js, uploads, encrypted API settings'
compose up -d --no-build --pull never --wait --wait-timeout 180 >"$OUT_DIR/empty-start.log" 2>&1
http_check empty seed empty-state.json
inspect_runtime empty
restart_and_check empty-restart empty-state.json
# Capture only ciphertext and permissions; no key/master values are printed.
docker exec "$(compose ps -q manual)" sh -c 'set -eu; test "$(stat -c %a /runtime-config/provider-overrides.json)" = 600; test ! -e /data/provider-overrides.json' >"$OUT_DIR/empty-encryption-contract.txt"
docker cp "$(compose ps -q manual):/runtime-config/provider-overrides.json" "$OUT_DIR/encrypted-provider-overrides.json"
python3 - "$OUT_DIR" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1]); raw=(p/'encrypted-provider-overrides.json').read_bytes()
assert all(value.encode() not in raw for value in (p/'private/canaries.txt').read_text().splitlines())
assert b'AES-256-GCM' in raw
json.loads(raw)
PY
compose stop -t 60 proxy manual >"$OUT_DIR/empty-stop.log" 2>&1

printf '%s\n' '[2/6] Restore the complete fixture backup, read immutable release/PDF/GLB, restart persistence'
CURRENT_PROJECT="$PROJECT-sample"
restore_backup "${CURRENT_PROJECT}_manual-data" "$PROJECT-fixture-input" "$SAMPLE_BACKUP" fixture
compose up -d --no-build --pull never --wait --wait-timeout 180 >"$OUT_DIR/sample-start.log" 2>&1
http_check sample read sample-state.json --require-release
inspect_runtime sample
restart_and_check sample-restart sample-state.json --require-release

printf '%s\n' '[3/6] Stopped-service backup, fresh-volume restore, exact release and asset hash comparison'
compose stop -t 60 proxy manual >"$OUT_DIR/sample-stop.log" 2>&1
BACKUP_VOLUME="$PROJECT-backup-output"
create_volume "$BACKUP_VOLUME"
compose run --rm --no-deps -v "$BACKUP_VOLUME:/backup" manual backup --data-dir /data --out /backup/export >"$OUT_DIR/sample-backup.log" 2>&1
COPY_CONTAINER="$PROJECT-backup-reader"
docker create --name "$COPY_CONTAINER" --platform "$EM_DOCKER_PLATFORM" --network none --user 0:0 --entrypoint sh -v "$BACKUP_VOLUME:/backup" "$IMAGE" -c true >/dev/null
EXTRA_CONTAINERS+=("$COPY_CONTAINER")
docker cp "$COPY_CONTAINER:/backup/export" "$OUT_DIR/backup"
[[ ! -f "$OUT_DIR/backup/provider-overrides.json" && ! -f "$OUT_DIR/backup/master.key" ]]
CURRENT_PROJECT="$PROJECT-restore"
restore_backup "${CURRENT_PROJECT}_manual-data" "$PROJECT-restored-input" "$OUT_DIR/backup" restored
cp "$OUT_DIR/sample-state.json" "$OUT_DIR/restored-state.json"
compose up -d --no-build --pull never --wait --wait-timeout 180 >"$OUT_DIR/restored-start.log" 2>&1
http_check restored read restored-state.json --require-release
compose stop -t 60 proxy manual >"$OUT_DIR/restored-stop.log" 2>&1

printf '%s\n' '[4/6] Read the same release/PDF/GLB with Docker --network none and no source/Node/Python'
OFFLINE_CONTAINER="$PROJECT-offline"
docker run -d --name "$OFFLINE_CONTAINER" --platform "$EM_DOCKER_PLATFORM" --network none --read-only --user 10001:10001 \
  --tmpfs /tmp:rw,noexec,nosuid,nodev,size=64m,mode=1777 \
  -e EM_PUBLIC_ORIGIN=http://127.0.0.1:8081 \
  -v "${CURRENT_PROJECT}_manual-data:/data" -v "${CURRENT_PROJECT}_manual-config:/runtime-config" \
  -v "${CURRENT_PROJECT}_private-runtime:/run/private:ro" "$IMAGE" >/dev/null
EXTRA_CONTAINERS+=("$OFFLINE_CONTAINER")
docker inspect "$OFFLINE_CONTAINER" >"$OUT_DIR/offline-runtime.json"
python3 - "$OUT_DIR/offline-runtime.json" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))[0]['HostConfig']['NetworkMode']=='none'
PY
docker exec "$OFFLINE_CONTAINER" sh -c '
  set -eu
  for attempt in $(seq 1 60); do
    wget -q -T 1 -O /tmp/ready.json http://127.0.0.1:8081/api/v1/health/ready && break
    sleep 1
  done
  test -s /tmp/ready.json
  ! command -v node
  ! command -v python3
  test ! -d /src/everything-manual
  cat /proc/net/route
  if wget -q -T 3 -O /tmp/external https://example.com; then exit 1; fi
  printf "{\"password\":\"test-password-t20-backup\"}" >/tmp/login-request.json
  wget -S -T 5 --header="Origin: http://127.0.0.1:8081" --header="Content-Type: application/json" --post-file=/tmp/login-request.json -O /tmp/login-response.json http://127.0.0.1:8081/api/v1/auth/login 2>/tmp/login-headers
  grep -q "\"csrfToken\"" /tmp/login-response.json
  printf "Offline login succeeded\n"
  awk "tolower(\$1)==\"set-cookie:\" {split(\$2, value, \";\"); print value[1]}" /tmp/login-headers >/tmp/cookie.txt
  test -s /tmp/cookie.txt
  grep -q "^em_session=" /tmp/cookie.txt
  printf "Offline em_session cookie parsed\n"
  wget -q -T 5 --header="Cookie: $(cat /tmp/cookie.txt)" -O /tmp/items.json http://127.0.0.1:8081/api/v1/items
  printf "Offline authenticated library succeeded\n"
' >"$OUT_DIR/offline-network.log" 2>&1
# Read only public entity IDs/hashes from the prior report, never credentials.
python3 - "$OUT_DIR/backup" "$OUT_DIR/offline-assets.tsv" <<'PY'
import json,pathlib,sys
manifest=json.loads((pathlib.Path(sys.argv[1])/'manifest.json').read_text())
with open(sys.argv[2],'w') as out:
    for b in manifest['blobs']: out.write(f"{b['sha256']}\t{b['size']}\n")
PY
docker exec "$OFFLINE_CONTAINER" sh -c 'set -eu; wget -q -T 5 -O /tmp/index.html http://127.0.0.1:8081/; test -s /tmp/index.html; printf "Offline authenticated library and embedded UI readable\n"' >>"$OUT_DIR/offline-network.log" 2>&1
python3 - "$OUT_DIR" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1]); state=json.loads((p/'sample-state.json').read_text())
with open(p/'offline-requests.tsv','w') as output:
    output.write(f"/api/v1/items/{state['releaseItemId']}/releases/{state['release']['releaseId']}\trelease.json\n")
    for a in state['releaseAssets']:
        output.write(f"/api/v1/assets/{a['id']}/content\t{a['id']}.bin\n")
    for index, path in enumerate(state['staticPaths']):
        output.write(f"{path}\tstatic-{index}.bin\n")
PY
docker exec -i "$OFFLINE_CONTAINER" sh -c 'cat >/tmp/offline-requests.tsv' <"$OUT_DIR/offline-requests.tsv"
docker exec "$OFFLINE_CONTAINER" sh -c '
  set -eu
  mkdir /tmp/offline-proof
  while read -r path output; do
    wget -q -T 10 --header="Cookie: $(cat /tmp/cookie.txt)" -O "/tmp/offline-proof/$output" "http://127.0.0.1:8081$path"
    test -s "/tmp/offline-proof/$output"
  done </tmp/offline-requests.tsv
' >>"$OUT_DIR/offline-network.log" 2>&1
# Docker's archive endpoint cannot traverse this container's /tmp tmpfs.
# Stream only the public proof files through the actual mount namespace.
docker exec "$OFFLINE_CONTAINER" tar -C /tmp -cf - offline-proof | tar -C "$OUT_DIR" -xf -
python3 - "$OUT_DIR" <<'PY'
import hashlib,json,pathlib,sys
p=pathlib.Path(sys.argv[1]);state=json.loads((p/'sample-state.json').read_text());proof=p/'offline-proof'
r=json.loads((proof/'release.json').read_text())['data']
assert r['id']==state['release']['releaseId']
assert r['manifestSha256']==state['release']['manifestSha256']
for asset in state['releaseAssets']:
    assert hashlib.sha256((proof/(asset['id']+'.bin')).read_bytes()).hexdigest()==asset['sha256']
(p/'offline-http-verification.json').write_text(json.dumps({'status':'passed','networkMode':'none','release':state['release'],'assetHashesMatched':len(state['releaseAssets']),'embeddedResourcesRead':len(state['staticPaths']),'providerRequests':0},indent=2)+'\n')
PY
docker exec "$OFFLINE_CONTAINER" sh -c 'find /data/blobs -type f -exec sha256sum {} \;' >"$OUT_DIR/offline-blob-hashes.txt"
python3 - "$OUT_DIR" <<'PY'
import pathlib,sys
p=pathlib.Path(sys.argv[1]); actual={line.split()[0] for line in (p/'offline-blob-hashes.txt').read_text().splitlines()}
expected={line.split()[0] for line in (p/'offline-assets.tsv').read_text().splitlines()}
assert expected <= actual
PY
docker stop -t 60 "$OFFLINE_CONTAINER" >"$OUT_DIR/offline-stop.log"
[[ "$(docker inspect -f '{{.State.ExitCode}}' "$OFFLINE_CONTAINER")" = 0 ]]

printf '%s\n' '[5/6] HTTPS public-origin cookie/Origin checks and a real upstream certificate-validated HTTPS GET'
CURRENT_PROJECT="$PROJECT-tls"
export EM_PUBLIC_ORIGIN=https://manual.example.test
compose up -d --no-build --pull never --wait --wait-timeout 180 >"$OUT_DIR/tls-start.log" 2>&1
http_check tls tls tls-state.json --origin https://manual.example.test
compose stop -t 60 proxy manual >"$OUT_DIR/tls-stop.log" 2>&1
CURRENT_PROJECT="$PROJECT-empty"
export EM_PUBLIC_ORIGIN="$BASE_URL"
compose up -d --no-build --pull never --wait --wait-timeout 180 >"$OUT_DIR/upstream-tls-start.log" 2>&1
http_check upstream-tls-seed tls-seed empty-state.json
restart_and_check upstream-tls-restart empty-state.json
http_check upstream-tls-probe tls-probe empty-state.json
compose stop -t 60 proxy manual >"$OUT_DIR/upstream-tls-stop.log" 2>&1

printf '%s\n' '[6/6] Secret absence in evidence/backup and image environment'
python3 - "$OUT_DIR" <<'PY'
import pathlib,sys
p=pathlib.Path(sys.argv[1]); private=p/'private'
values=[*(private/'canaries.txt').read_text().splitlines(),(private/'master.key').read_text().strip()]
for path in p.rglob('*'):
    if not path.is_file() or private in path.parents: continue
    raw=path.read_bytes()
    assert all(value.encode() not in raw for value in values), f'Secret appeared in {path.name}'
PY
STATUS=passed
