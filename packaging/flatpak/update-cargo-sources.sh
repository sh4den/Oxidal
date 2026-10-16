#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
tools="$(mktemp -d)"
trap 'rm -rf "$tools"' EXIT

git clone --depth 1 https://github.com/flatpak/flatpak-builder-tools "$tools/fbt"
python3 -m venv "$tools/venv"
"$tools/venv/bin/pip" install --quiet aiohttp tomlkit
"$tools/venv/bin/python" "$tools/fbt/cargo/flatpak-cargo-generator.py" \
  "$here/../../Cargo.lock" -o "$here/cargo-sources.json"
