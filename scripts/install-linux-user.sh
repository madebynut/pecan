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
binary=$bindir/Pecan
icons=$data/icons/hicolor
mkdir -p "$bindir" "$data/applications" "$icons/256x256/apps" "$icons/scalable/apps"
# Replace by rename: copying over a running executable fails with "Text file busy".
cp "$root/Pecan" "$binary.new"
chmod 755 "$binary.new"
mv -f "$binary.new" "$binary"
cp "$root/com.pecan.reader.png" "$icons/256x256/apps/com.pecan.reader.png"
if [ -f "$root/com.pecan.reader.svg" ]; then
    cp "$root/com.pecan.reader.svg" "$icons/scalable/apps/com.pecan.reader.svg"
fi
# Desktop entry Exec values quote paths with spaces and escape ", `, $ and \.
quoted=$(printf '%s' "$binary" | sed -e 's/\\/\\\\\\\\/g' -e 's/["`$]/\\\\&/g')
desktop=$data/applications/com.pecan.reader.desktop
tmp=$desktop.tmp
while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
        Exec=*) printf 'Exec="%s" %%F\n' "$quoted" ;;
        TryExec=*) printf 'TryExec=%s\n' "$binary" ;;
        Icon=*) printf 'Icon=com.pecan.reader\n' ;;
        *) printf '%s\n' "$line" ;;
    esac
done < "$root/com.pecan.reader.desktop" > "$tmp"
chmod 644 "$tmp"
mv -f "$tmp" "$desktop"
if [ -f "$root/uninstall.sh" ]; then
    mkdir -p "$data/Pecan"
    cp "$root/uninstall.sh" "$data/Pecan/uninstall.sh"
    chmod 755 "$data/Pecan/uninstall.sh"
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data/applications" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$icons" || true
fi
printf '%s\n' "已安装到 $binary"
printf '%s\n' "可以从应用菜单启动碧果。若终端找不到命令，把 $bindir 加入 PATH。"
printf '%s\n' "设为 Markdown 默认阅读器：xdg-mime default com.pecan.reader.desktop text/markdown"
if [ -f "$data/Pecan/uninstall.sh" ]; then
    printf '%s\n' "卸载：$data/Pecan/uninstall.sh"
fi
