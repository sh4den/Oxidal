#!/usr/bin/env bash
set -euo pipefail

version="$1"
binary="$2"
arch="$3"
out_dir="$4"

case "$arch" in
  amd64) rpm_arch=x86_64 ;;
  arm64) rpm_arch=aarch64 ;;
  *) rpm_arch="$arch" ;;
esac

here="$(cd "$(dirname "$0")" && pwd)"
staging="$(mktemp -d)"
mkdir -p "$staging/SOURCES" "$out_dir"
cp "$binary" "$staging/SOURCES/oxidal"
cp "$here/oxidal.desktop" "$here/io.github.sh4den.Oxidal.metainfo.xml" "$staging/SOURCES/"
cp -r "$here/icons" "$staging/SOURCES/icons"

rpmbuild -bb "$here/oxidal.spec" \
  --define "_topdir $staging" \
  --define "version $version" \
  --target "$rpm_arch"

cp "$staging"/RPMS/*/*.rpm "$out_dir/"
rm -rf "$staging"
