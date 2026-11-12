#!/bin/sh
# Install the unpacked Linux build for the current user.
set -eu
if [ -z "${HOME:-}" ]; then
    echo "HOME is not set" >&2
    exit 1
fi
root=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
data=${XDG_DATA_HOME:-"$HOME/.local/share"}
case "$data" in
    /*) ;;
    *) data=$HOME/.local/share ;;
esac
bindir=$HOME/.local/bin
icon=$data/icons/hicolor/256x256/apps/com.pecan.reader.png
mkdir -p "$bindir" "$data/applications" "$data/icons/hicolor/256x256/apps"
cp "$root/Pecan" "$bindir/Pecan"
chmod 755 "$bindir/Pecan"
cp "$root/com.pecan.reader.png" "$icon"
desktop=$data/applications/com.pecan.reader.desktop
tmp=$desktop.tmp
while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
        Exec=*) printf 'Exec=%s %%f\n' "$bindir/Pecan" ;;
        Icon=*) printf 'Icon=%s\n' "$icon" ;;
        *) printf '%s\n' "$line" ;;
    esac
done < "$root/com.pecan.reader.desktop" > "$tmp"
mv "$tmp" "$desktop"
chmod 644 "$desktop"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data/applications" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$data/icons/hicolor" || true
fi
printf '%s\n' "已安装到 $bindir/Pecan"
printf '%s\n' "可以从应用菜单启动碧果。若终端找不到命令，把 $bindir 加入 PATH。"
