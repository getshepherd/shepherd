#!/bin/sh
# Signs each archive given for the updater off macOS (src/update.rs): the Ed25519 signature of its SHA-256, by the
# private key in SHEPHERD_UPDATE_KEY (PEM), into <archive>.sig beside it. The app checks it against its UPDATE_KEY
# before it puts the binary in place. Release.yml runs it; a release without the key would leave every Linux and
# Windows install unable to update, so it fails instead.
set -eu
if [ -z "${SHEPHERD_UPDATE_KEY:-}" ]; then
  echo "SHEPHERD_UPDATE_KEY is not set: the updater can't trust unsigned archives" >&2
  exit 1
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
printf '%s\n' "$SHEPHERD_UPDATE_KEY" > "$work/key.pem"
for archive in "$@"; do
  openssl dgst -sha256 -binary "$archive" > "$work/digest"
  openssl pkeyutl -sign -inkey "$work/key.pem" -rawin -in "$work/digest" -out "$archive.sig"
  echo "signed $archive"
done
