#!/bin/sh
set -eu
project_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"
target=${1:-$(rustc -vV | sed -n 's/^host: //p')}
version=$(python3 scripts/release-version.py)
case "$target" in
    x86_64-*) arch=x64 ;;
    aarch64-*) arch=arm64 ;;
    *) arch=${target%%-*} ;;
esac
cargo build --locked --release --bin Pecan --target "$target"
name="Pecan-$version-Linux-$arch"
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
cp "$project_root/assets/brand/pecan-icon.svg" "$bundle/com.pecan.reader.svg"
cp "$project_root/scripts/install-linux-user.sh" "$bundle/install.sh"
cp "$project_root/scripts/uninstall-linux-user.sh" "$bundle/uninstall.sh"
chmod 755 "$bundle/Pecan" "$bundle/install.sh" "$bundle/uninstall.sh"
archive="$project_root/dist/$name.zip"
rm -f "$archive"
(cd "$staging" && zip -r -q "$archive" "$name")
echo "$archive"
