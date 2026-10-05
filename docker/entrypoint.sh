#!/bin/sh
set -eu
umask 077

binary=/usr/local/bin/everything-manual

fail() { printf '%s\n' "$*" >&2; exit 2; }

if [ "${1:-}" = prepare-secrets ]; then
  [ "$(id -u)" = 0 ] || fail 'prepare-secrets requires the isolated preparation service.'
  runtime_uid=${EM_RUNTIME_UID:-10001}
  runtime_gid=${EM_RUNTIME_GID:-10001}
  for identity in "$runtime_uid" "$runtime_gid"; do
    case "$identity" in ''|*[!0-9]*) fail 'Runtime UID and GID must be nonzero integers.' ;; esac
    [ "$identity" -gt 0 ] 2>/dev/null || fail 'Runtime UID and GID must be nonzero integers.'
  done
  for directory in /data /runtime-config /run/private; do
    [ -d "$directory" ] && [ ! -L "$directory" ] || fail 'A runtime directory is unavailable or is a symlink.'
    chown "$runtime_uid:$runtime_gid" "$directory"
    chmod 700 "$directory"
  done
  for name in admin_password master_key; do
    source="/run/secrets/$name"
    [ -f "$source" ] && [ ! -L "$source" ] && [ -s "$source" ] || fail 'A required secret file is unavailable.'
    [ "$(wc -c < "$source")" -le 262144 ] || fail 'Secret file exceeds the supported size.'
  done
  key=$(tr -d '\r\n' < /run/secrets/master_key)
  [ "${#key}" -eq 64 ] || fail 'The master key must contain exactly 64 hexadecimal characters.'
  case "$key" in *[!0-9a-fA-F]*) fail 'The master key must contain exactly 64 hexadecimal characters.' ;; esac
  unset key
  for mapping in admin_password:password.txt master_key:master.key; do
    source=${mapping%%:*}
    destination=${mapping#*:}
    [ ! -L "/run/private/$destination" ] || fail 'A private runtime file is a symlink.'
    cp "/run/secrets/$source" "/run/private/$destination"
    chmod 600 "/run/private/$destination"
    chown "$runtime_uid:$runtime_gid" "/run/private/$destination"
  done
  printf '%s\n' 'Private runtime secrets prepared in memory.'
  # A tmpfs-backed Docker volume is discarded when its final container unmounts
  # it. Keep one non-root process attached so app stop/backup/restart works.
  exec su-exec "$runtime_uid:$runtime_gid" sh -c '
    child=
    stop() {
      if [ -n "$child" ]; then
        kill -TERM "$child" 2>/dev/null || true
        wait "$child" 2>/dev/null || true
      fi
      exit 0
    }
    trap stop TERM INT
    while :; do
      sleep 3600 &
      child=$!
      wait "$child" || true
    done
  '
fi

[ "$(id -u)" != 0 ] || fail 'The application must run as a non-root user.'
if [ "${1:-}" = serve ]; then
  [ -f /run/private/master.key ] && [ -r /run/private/master.key ] || fail 'Prepare the private runtime secrets before starting the service.'
  [ -d "${EM_PROVIDER_OVERRIDES_DIR:-/runtime-config}" ] || fail 'The private API configuration directory is unavailable.'
  if [ ! -f "${EM_DATA_DIR:-/data}/manual.sqlite3" ]; then
    "$binary" init --data-dir "${EM_DATA_DIR:-/data}" --password-file /run/private/password.txt
  fi
fi
# The server becomes PID 1 and receives SIGTERM directly; no shell swallows it.
exec "$binary" "$@"
