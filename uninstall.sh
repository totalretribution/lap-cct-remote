#!/usr/bin/env bash
# Remove the LAP CCT light remote. Keeps the pairing in ~/.config/light-remote; delete that too if you like.
set -euo pipefail

if [ "${1:-}" = --system ]; then
    [ "$(id -u)" = 0 ] || { echo "--system must run as root" >&2; exit 1; }
    rm -f /usr/local/bin/light-tray /usr/local/libexec/light-radio \
        /usr/local/share/applications/light-tray.desktop /usr/local/share/icons/hicolor/scalable/apps/light-remote.svg
    rm -f /usr/local/share/icons/hicolor/*x*/apps/light-remote.png
    gtk-update-icon-cache -q -t /usr/local/share/icons/hicolor 2>/dev/null || true
    exit 0
fi

HERE="$(cd "$(dirname "$0")" && pwd)"
pkill -x light-tray || true
rm -f ~/.config/autostart/light-tray.desktop
pkexec "$HERE/uninstall.sh" --system
echo "Removed. Your pairing is still in ~/.config/light-remote."
