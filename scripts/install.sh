#!/bin/sh
set -eu

umask 022
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_DIR=$(dirname -- "$SCRIPT_DIR")
INSTALL_DIR=${ADO_INSTALL_DIR:-"$HOME/.local/bin"}
DESTINATION="$INSTALL_DIR/ado"
STAGING=
SIGNING_IDENTITY=${ADO_CODESIGN_IDENTITY:-}
PLATFORM=$(uname -s)

cleanup() {
  if [ -n "$STAGING" ]; then
    rm -f -- "$STAGING"
  fi
}
trap cleanup EXIT HUP INT TERM

if [ "$PLATFORM" = "Darwin" ] && [ -z "$SIGNING_IDENTITY" ]; then
  IDENTITIES=$(security find-identity -v -p codesigning)
  IDENTITY_HASHES=$(printf '%s\n' "$IDENTITIES" | awk '/^[[:space:]]*[0-9]+\)/ {print $2}')
  IDENTITY_COUNT=$(printf '%s\n' "$IDENTITY_HASHES" | awk 'NF {count++} END {print count+0}')
  if [ "$IDENTITY_COUNT" -ne 1 ]; then
    printf '%s\n' "Found $IDENTITY_COUNT valid signing identities. Set ADO_CODESIGN_IDENTITY to a certificate name or SHA-1 from 'security find-identity -v -p codesigning'." >&2
    printf '%s\n' "Set ADO_CODESIGN_IDENTITY=- only if you explicitly want ad-hoc signing." >&2
    exit 1
  fi
  SIGNING_IDENTITY=$IDENTITY_HASHES
fi

cd "$PROJECT_DIR"
cargo build --release --locked
mkdir -p -- "$INSTALL_DIR"
STAGING=$(mktemp "$INSTALL_DIR/.ado.install.XXXXXX")
install -m 0755 "target/release/ado" "$STAGING"

if [ "$PLATFORM" = "Darwin" ]; then
  codesign --force --sign "$SIGNING_IDENTITY" --identifier dev.ollies.ado-helper "$STAGING"
  codesign --verify --strict "$STAGING"
fi

chmod 0755 "$STAGING"
mv -f -- "$STAGING" "$DESTINATION"
trap - EXIT HUP INT TERM

printf '%s\n' "Installed ado at $DESTINATION"
printf '%s\n' "Add $INSTALL_DIR to PATH if it is not already present."
if [ "$PLATFORM" = "Darwin" ]; then
  if [ "$SIGNING_IDENTITY" = "-" ]; then
    printf '%s\n' "The executable is ad-hoc signed. Upgrades can cause macOS Keychain to ask for access again."
  else
    printf '%s\n' "The executable is certificate-signed with identity $SIGNING_IDENTITY."
    printf '%s\n' "Switching from an ad-hoc signature can require one-time Keychain approval. Use the same signing identity for future upgrades."
  fi
fi
