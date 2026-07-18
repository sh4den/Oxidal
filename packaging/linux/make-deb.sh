#!/usr/bin/env bash
set -euo pipefail

version="$1"
binary="$2"
arch="$3"
out_dir="$4"

staging="$(mktemp -d)"
root="$staging/oxidal_${version}_${arch}"
mkdir -p "$root/DEBIAN" "$root/usr/bin" "$root/usr/share/applications" "$out_dir"
cp "$binary" "$root/usr/bin/oxidal"
chmod 755 "$root/usr/bin/oxidal"
cp "$(dirname "$0")/oxidal.desktop" "$root/usr/share/applications/oxidal.desktop"

cat > "$root/DEBIAN/control" <<EOF
Package: oxidal
Version: $version
Section: net
Priority: optional
Architecture: $arch
Maintainer: sh4den <69421356+sh4den@users.noreply.github.com>
Homepage: https://github.com/sh4den/Oxidal
Depends: libc6, libfontconfig1, libxkbcommon0, libvulkan1, libudev1
Description: Cross-platform SSH, SFTP and serial terminal client
EOF

dpkg-deb --build --root-owner-group "$root" "$out_dir"
rm -rf "$staging"
