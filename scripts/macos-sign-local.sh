#!/usr/bin/env bash
# Sign a separate local-preview binary with an existing, stable macOS identity.
# This does not create an identity, edit a Keychain ACL, or replace a running app.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/macos-sign-local.sh <source-binary> <new-output-binary> <identity-sha1>

Sign one new local-preview binary with an existing macOS code-signing identity.
The output path must not exist. Stop the service before replacing its executable.
The identity SHA-1 is the 40-character fingerprint shown by:
  security find-identity -v -p codesigning

This is for local preview only; it does not notarize or sign the dist release.
EOF
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then
  usage
  exit 0
fi
if [[ $# -ne 3 ]]; then
  usage >&2
  exit 2
fi
if [[ $(uname -s) != Darwin ]]; then
  echo 'This helper runs only on macOS.' >&2
  exit 2
fi

source_binary=$1
output_binary=$2
identity_sha1=$3
if [[ ! $identity_sha1 =~ ^[[:xdigit:]]{40}$ ]]; then
  echo 'Provide the SHA-1 fingerprint of an existing code-signing identity.' >&2
  exit 2
fi
if [[ ! -f $source_binary || ! -x $source_binary ]]; then
  echo 'Source must be an executable regular file.' >&2
  exit 2
fi
if [[ -e $output_binary || -L $output_binary ]]; then
  echo 'Output path already exists; choose a new path.' >&2
  exit 2
fi
if [[ ! -d $(dirname "$output_binary") ]]; then
  echo 'Output directory does not exist.' >&2
  exit 2
fi
command -v codesign >/dev/null || { echo 'codesign is unavailable.' >&2; exit 2; }

umask 077
temporary_binary=$(mktemp "${output_binary}.tmp.XXXXXXXX")
trap 'rm -f "$temporary_binary"' EXIT
cp "$source_binary" "$temporary_binary"
chmod u+x "$temporary_binary"
codesign --force --sign "$identity_sha1" \
  --identifier org.everything-manual.preview --timestamp=none \
  "$temporary_binary"
codesign --verify --strict --verbose=2 "$temporary_binary"
signature_details=$(codesign --display --verbose=4 "$temporary_binary" 2>&1)
designated_requirement=$(codesign --display --requirements - "$temporary_binary" 2>&1)
if [[ $signature_details != *'Identifier=org.everything-manual.preview'* ||
      $signature_details == *'Signature=adhoc'* ||
      $designated_requirement == *'cdhash '* ]]; then
  echo 'Signed binary did not retain the expected identity.' >&2
  exit 1
fi

# A hard link creates the requested path only if absent; the signed bytes stay intact.
ln "$temporary_binary" "$output_binary"
echo "Signed local-preview binary: $output_binary"
echo 'Use the same identity and identifier for future preview builds.'
