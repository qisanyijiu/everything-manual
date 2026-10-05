#!/bin/sh
# Only this no-network helper reads host 0600 TLS secrets as root.
set -eu
umask 077

fail() { printf '%s\n' "$*" >&2; exit 2; }
[ "$(id -u)" = 0 ] || fail 'TLS preparation requires the isolated root helper.'
case "${EM_PUBLIC_ORIGIN:-}" in
  https://?*) ;;
  *) fail 'Production EM_PUBLIC_ORIGIN must be an explicit HTTPS origin.' ;;
esac
[ -d /run/tls ] && [ ! -L /run/tls ] || fail 'TLS runtime volume is unavailable.'
for name in tls_certificate tls_private_key; do
  source="/run/secrets/$name"
  [ -f "$source" ] && [ ! -L "$source" ] && [ -s "$source" ] || fail 'A required TLS secret file is unavailable.'
  [ "$(wc -c < "$source")" -le 262144 ] || fail 'TLS secret file exceeds the supported size.'
done
grep -q '^-----BEGIN CERTIFICATE-----' /run/secrets/tls_certificate || fail 'TLS certificate must be PEM.'
grep -Eq '^-----BEGIN (RSA |EC )?PRIVATE KEY-----' /run/secrets/tls_private_key || fail 'TLS private key must be an unencrypted PEM key.'
chown 101:101 /run/tls
chmod 700 /run/tls
for mapping in tls_certificate:fullchain.pem tls_private_key:privkey.pem; do
  source=${mapping%%:*}
  destination=${mapping#*:}
  [ ! -L "/run/tls/$destination" ] || fail 'A TLS runtime file is a symlink.'
  cp "/run/secrets/$source" "/run/tls/$destination"
  chmod 600 "/run/tls/$destination"
  chown 101:101 "/run/tls/$destination"
done
printf '%s\n' 'TLS secrets prepared in private memory.'
# Keep this tmpfs mounted across application/proxy maintenance.
exec su-exec 101:101 sh -c '
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
