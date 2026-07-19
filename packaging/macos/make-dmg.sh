#!/usr/bin/env bash
set -euo pipefail

version="$1"
binary="$2"
name="$3"
out_dir="$4"

staging="$(mktemp -d)"
app="$staging/Oxidal.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$out_dir"
cp "$binary" "$app/Contents/MacOS/Oxidal"
chmod 755 "$app/Contents/MacOS/Oxidal"

icon_src="$(dirname "$0")/../icon.png"
iconset="$(mktemp -d)/Oxidal.iconset"
mkdir -p "$iconset"
for size in 16 32 64 128 256 512; do
	sips -z "$size" "$size" "$icon_src" --out "$iconset/icon_${size}x${size}.png" >/dev/null
	sips -z "$((size * 2))" "$((size * 2))" "$icon_src" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Oxidal.icns"
rm -rf "$(dirname "$iconset")"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>Oxidal</string>
	<key>CFBundleIdentifier</key>
	<string>com.sh4den.oxidal</string>
	<key>CFBundleName</key>
	<string>Oxidal</string>
	<key>CFBundleDisplayName</key>
	<string>Oxidal</string>
	<key>CFBundleIconFile</key>
	<string>Oxidal</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
EOF

codesign --force --deep --sign - "$app"

ln -s /Applications "$staging/Applications"
hdiutil create -volname "Oxidal" -srcfolder "$staging" -ov -format UDZO "$out_dir/Oxidal-$version-$name.dmg"
rm -rf "$staging"
