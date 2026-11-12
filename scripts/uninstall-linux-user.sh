#!/bin/sh
# Remove a per-user Linux installation. Settings, history and logs are kept unless --purge is given.
set -eu
if [ -z "${HOME:-}" ]; then
    echo "HOME is not set" >&2
    exit 1
fi
absolute() {
    case "${1:-}" in
        /*) printf '%s' "$1" ;;
        *) printf '%s' "$2" ;;
    esac
}
data=$(absolute "${XDG_DATA_HOME:-}" "$HOME/.local/share")
config=$(absolute "${XDG_CONFIG_HOME:-}" "$HOME/.config")
state=$(absolute "${XDG_STATE_HOME:-}" "$HOME/.local/state")
icons=$data/icons/hicolor
rm -f "$HOME/.local/bin/Pecan" \
    "$data/applications/com.pecan.reader.desktop" \
    "$icons/256x256/apps/com.pecan.reader.png" \
    "$icons/scalable/apps/com.pecan.reader.svg"
rm -rf "$data/Pecan"
if [ "${1:-}" = "--purge" ]; then
    rm -rf "$config/Pecan" "$state/Pecan"
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data/applications" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$icons" || true
fi
printf '%s\n' "已卸载碧果。"
if [ "${1:-}" != "--purge" ]; then
    printf '%s\n' "设置与最近记录保留在 $config/Pecan；使用 --purge 一并删除。"
fi
