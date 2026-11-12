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
cp "$project_root/LICENSE" "$staged_bundle/Contents/Resources/LICENSE"
cp "$project_root/NOTICE" "$staged_bundle/Contents/Resources/NOTICE"
cp -R "$project_root/licenses" "$staged_bundle/Contents/Resources/licenses"
cp "$project_root/assets/brand/pecan.icns" "$staged_bundle/Contents/Resources/Pecan.icns"
sed "s/@VERSION@/$version/g" "$project_root/assets/macos/Info.plist" > "$staged_bundle/Contents/Info.plist"
plutil -lint "$staged_bundle/Contents/Info.plist"
# MACOS_SIGNING_IDENTITY names a Developer ID Application certificate in the keychain.
# Without it the bundle is ad-hoc signed, which Gatekeeper rejects for downloaded apps.
if [ -n "${MACOS_SIGNING_IDENTITY:-}" ]; then
    codesign --force --sign "$MACOS_SIGNING_IDENTITY" --options runtime --timestamp "$staged_bundle"
else
    codesign --force --sign - --timestamp=none "$staged_bundle"
fi
codesign --verify --strict --verbose=2 "$staged_bundle"
# Notarization needs an Apple ID app-specific password: APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD.
if [ -n "${MACOS_SIGNING_IDENTITY:-}" ] && [ -n "${APPLE_ID:-}" ]; then
    submission="$staging/notarize.zip"
    ditto -c -k --sequesterRsrc --keepParent "$staged_bundle" "$submission"
    xcrun notarytool submit "$submission" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" --wait --timeout 30m
    xcrun stapler staple "$staged_bundle"
    xcrun stapler validate "$staged_bundle"
    spctl --assess --type execute --verbose=2 "$staged_bundle"
fi
if [ -e "$bundle" ]; then
    mv "$bundle" "$staging/previous.app"
fi
mv "$staged_bundle" "$bundle"
archive="$project_root/dist/Pecan-$version-macOS-$arch.zip"
ditto -c -k --sequesterRsrc --keepParent "$bundle" "$archive"
echo "$archive"
