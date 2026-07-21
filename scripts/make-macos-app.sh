#!/bin/sh
# Package the hitair-gui binary into a proper macOS .app bundle, so it's a real
# Finder app (Dock icon, no Terminal window) instead of a bare Unix executable.
#
#   scripts/make-macos-app.sh <gui-binary> <output.app> <version>
#
# Run from the repo root on a macOS host (uses `sips` + `iconutil`).
set -eu

[ $# -eq 3 ] || { echo "usage: $0 <gui-binary> <output.app> <version>" >&2; exit 1; }
bin="$1"
app="$2"
version="$3"
icon_src="crates/hitair-gui/assets/icon.png"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/hitair-gui"
chmod +x "$app/Contents/MacOS/hitair-gui"

# Icon: PNG → .iconset (all the sizes macOS wants) → .icns.
iconset="$(mktemp -d)/hitair.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$icon_src" \
    --out "$iconset/icon_${size}x${size}.png" >/dev/null
  d=$((size * 2))
  sips -z "$d" "$d" "$icon_src" \
    --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/hitair.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>hitair</string>
  <key>CFBundleDisplayName</key><string>hitair</string>
  <key>CFBundleIdentifier</key><string>be.londer.hitair</string>
  <key>CFBundleExecutable</key><string>hitair-gui</string>
  <key>CFBundleIconFile</key><string>hitair</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSMinimumSystemVersion</key><string>10.13</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

echo "built $app (v$version)"
