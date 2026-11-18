#!/bin/sh
# Installs wubble's desktop entry and icons for the current user, so the shell shows the
# wubble icon for the running window (on Wayland the icon comes only from the desktop file,
# matched by the app id `wubble`; on X11 by StartupWMClass).
#
#   scripts/install-linux-desktop.sh [--exec <path-to-wubble>]   install
#   scripts/install-linux-desktop.sh --uninstall                 remove
#
# --exec writes an absolute binary path into Exec= (for a dev build that is not on PATH,
# e.g. target/debug/wubble). Nothing is installed outside ~/.local/share.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
src="$here/assets/icons/linux"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
apps="$data/applications"
icons="$data/icons/hicolor"

refresh() {
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f "$icons" || true
}

case "${1:-}" in
--uninstall)
    rm -f "$apps/wubble.desktop"
    for f in "$src"/hicolor/*/apps/wubble.*; do
        rel=${f#"$src"/hicolor/}
        rm -f "$icons/$rel"
    done
    refresh
    echo "wubble desktop entry and icons removed from $data"
    ;;
--exec | "")
    exec_path=wubble
    if [ "${1:-}" = "--exec" ]; then
        exec_path=${2:?--exec needs a path}
        [ -x "$exec_path" ] || { echo "install-linux-desktop: $exec_path is not executable" >&2; exit 1; }
        exec_path=$(cd "$(dirname "$exec_path")" && pwd)/$(basename "$exec_path")
    fi
    mkdir -p "$apps"
    for f in "$src"/hicolor/*/apps/wubble.*; do
        rel=${f#"$src"/hicolor/}
        mkdir -p "$icons/$(dirname "$rel")"
        cp "$f" "$icons/$rel"
    done
    sed "s|^Exec=wubble %U|Exec=$exec_path %U|" "$src/wubble.desktop" >"$apps/wubble.desktop"
    refresh
    echo "installed $apps/wubble.desktop (Exec=$exec_path) and the icons under $icons"
    ;;
*)
    echo "usage: $0 [--exec <path-to-wubble> | --uninstall]" >&2
    exit 2
    ;;
esac
