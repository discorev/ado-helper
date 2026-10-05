#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

docker run --rm \
  --volume "$ROOT:/work:ro" \
  --workdir /work \
  rust:1.91-bookworm \
  bash -euo pipefail -c '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq dbus dbus-x11 git gnome-keyring >/dev/null
    export CARGO_TARGET_DIR=/tmp/ado-target
    cargo test --locked
    dbus-run-session -- bash -euo pipefail -c '\''
      export HOME=/tmp/ado-secret-service-home
      mkdir -p "$HOME/.local/share/keyrings"
      printf "%s" dummy-password | gnome-keyring-daemon --unlock --components=secrets >/tmp/keyring-environment
      cargo test --locked --test ported secret_service_round_trip -- --ignored --exact
    '\''
  '
