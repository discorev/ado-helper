#!/bin/sh
set -eu

umask 022
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_DIR=$(dirname -- "$SCRIPT_DIR")
INSTALL_DIR=${ADO_INSTALL_DIR:-"$HOME/.local/bin"}
DESTINATION="$INSTALL_DIR/ado"
STAGING=

cleanup() {
  if [ -n "$STAGING" ]; then
    rm -f -- "$STAGING"
  fi
}
trap cleanup EXIT HUP INT TERM

cd "$PROJECT_DIR"
swift build -c release
mkdir -p -- "$INSTALL_DIR"
STAGING=$(mktemp "$INSTALL_DIR/.ado.install.XXXXXX")
install -m 0755 ".build/release/ado" "$STAGING"

if command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - --identifier dev.ollies.ado-helper "$STAGING"
fi

chmod 0755 "$STAGING"
mv -f -- "$STAGING" "$DESTINATION"
trap - EXIT HUP INT TERM

printf '%s\n' "Installed ado at $DESTINATION"
printf '%s\n' "Add $INSTALL_DIR to PATH if it is not already present."
printf '%s\n' "The executable is ad-hoc signed. Upgrades can cause macOS Keychain to ask for access again."
