#!/usr/bin/env bash
set -euo pipefail

version="$1"
binary="$2"
name="$3"
out_dir="$4"

staging="$(mktemp -d)"
app="$staging/Oxidal.app"
mkdir -p "$app/Contents/MacOS" "$out_dir"
cp "$binary" "$app/Contents/MacOS/Oxidal"
chmod 755 "$app/Contents/MacOS/Oxidal"

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

ln -s /Applications "$staging/Applications"
hdiutil create -volname "Oxidal" -srcfolder "$staging" -ov -format UDZO "$out_dir/Oxidal-$version-$name.dmg"
rm -rf "$staging"
