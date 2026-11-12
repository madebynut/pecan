#!/bin/sh
set -eu
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"
target=${1:-$(rustc -vV | sed -n 's/^host: //p')}
version=$(python3 scripts/release-version.py)
cargo build --locked --release --bin Pecan --target "$target"
name="Pecan-$version-Linux-x64"
mkdir -p "$project_root/dist"
staging=$(mktemp -d "$project_root/dist/.bundle-XXXXXX")
trap 'rm -rf "$staging"' EXIT HUP INT TERM
bundle="$staging/$name"
mkdir -p "$bundle"
cp "$project_root/target/$target/release/Pecan" "$bundle/Pecan"
cp "$project_root/LICENSE" "$bundle/LICENSE"
cp "$project_root/NOTICE" "$bundle/NOTICE"
cp -R "$project_root/licenses" "$bundle/licenses"
cp "$project_root/assets/linux/com.pecan.reader.desktop" "$bundle/com.pecan.reader.desktop"
cp "$project_root/assets/brand/pecan-icon-256.png" "$bundle/com.pecan.reader.png"
cp "$project_root/scripts/install-linux-user.sh" "$bundle/install.sh"
chmod 755 "$bundle/Pecan" "$bundle/install.sh"
archive="$project_root/dist/$name.zip"
(cd "$staging" && zip -r -q "$archive" "$name")
echo "$archive"
