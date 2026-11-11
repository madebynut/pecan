#!/bin/sh
set -eu
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"
target=${1:-$(rustc -vV | sed -n 's/^host: //p')}
version=$(python3 scripts/release-version.py)
cargo build --locked --release --bin Pecan --target "$target"
arch=$(printf '%s' "$target" | cut -d- -f1)
[ "$arch" != aarch64 ] || arch=arm64
bundle="$project_root/dist/Pecan.app"
mkdir -p "$project_root/dist"
# Fresh inodes avoid stale macOS code-signing pages when replacing a previously launched binary.
staging=$(mktemp -d "$project_root/dist/.bundle-XXXXXX")
trap 'rm -rf "$staging"' EXIT HUP INT TERM
staged_bundle="$staging/Pecan.app"
mkdir -p "$staged_bundle/Contents/MacOS" "$staged_bundle/Contents/Resources"
cp "$project_root/target/$target/release/Pecan" "$staged_bundle/Contents/MacOS/Pecan"
cp -R "$project_root/licenses" "$staged_bundle/Contents/Resources/licenses"
cp "$project_root/assets/brand/pecan.icns" "$staged_bundle/Contents/Resources/Pecan.icns"
cat > "$staged_bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Pecan</string>
<key>CFBundleDisplayName</key><string>Pecan</string>
<key>CFBundleExecutable</key><string>Pecan</string>
<key>CFBundleIdentifier</key><string>com.pecan.reader</string>
<key>CFBundleIconFile</key><string>Pecan</string>
<key>CFBundleVersion</key><string>$version</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>
PLIST
codesign --force --sign - --timestamp=none "$staged_bundle"
codesign --verify --strict --verbose=2 "$staged_bundle"
plutil -lint "$staged_bundle/Contents/Info.plist"
if [ -e "$bundle" ]; then
    mv "$bundle" "$staging/previous.app"
fi
mv "$staged_bundle" "$bundle"
archive="$project_root/dist/Pecan-$version-macOS-$arch.zip"
ditto -c -k --sequesterRsrc --keepParent "$bundle" "$archive"
echo "$archive"

