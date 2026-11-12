#!/bin/sh
set -eu
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT HUP INT TERM
# Windows, Linux and the in-app logo use the full artwork.
cargo run --offline -p pecan-mermaid --example build_icons -- assets/brand/pecan-icon.svg "$staging/Pecan.iconset"
python3 scripts/pack-ico.py "$staging/Pecan.iconset" assets/brand/pecan.ico
cp "$staging/Pecan.iconset/icon_256x256.png" assets/brand/pecan-icon-256.png
# macOS follows Apple's icon grid so the Dock shows Pecan at the size of other apps.
cargo run --offline -p pecan-mermaid --example build_icons -- assets/brand/pecan-icon.svg "$staging/Pecan-macOS.iconset" --macos
python3 scripts/pack-icon.py "$staging/Pecan-macOS.iconset" assets/brand/pecan.icns
iconutil -c iconset assets/brand/pecan.icns -o "$staging/decoded.iconset"
python3 scripts/verify-icon.py "$staging/decoded.iconset"
