#!/usr/bin/env bash
# Build and install the LAP CCT light remote for the current user. Run from a normal terminal: ./install.sh
#
# Installs:
#   /usr/local/bin/light-tray        tray icon
#   /usr/local/libexec/light-radio   radio helper, with cap_net_raw, runnable by any user
#   the light-remote icon, a menu entry, and a login autostart entry for the tray.
# Root steps go through pkexec (one password prompt).
set -euo pipefail

BIN=/usr/local/bin
LIBEXEC=/usr/local/libexec
APPS=/usr/local/share/applications
ICONS=/usr/local/share/icons/hicolor

# --- root part, run by pkexec below: install.sh --system <release dir> <repo dir>
if [ "${1:-}" = --system ]; then
    release="$2" repo="$3"
    [ "$(id -u)" = 0 ] || { echo "--system must run as root" >&2; exit 1; }

    install -o root -g root -m 0755 "$release/light-tray" "$BIN/light-tray"
    install -o root -g root -m 0755 -D "$release/light-radio" "$LIBEXEC/light-radio"
    setcap cap_net_raw+ep "$LIBEXEC/light-radio"
    install -o root -g root -m 0644 -D "$repo/crates/light-tray/assets/light-tray.desktop" "$APPS/light-tray.desktop"
    assets="$repo/crates/light-tray/assets"
    install -o root -g root -m 0644 -D "$assets/light-remote.svg" "$ICONS/scalable/apps/light-remote.svg"
    for size in 16 22 32 48 64 128; do
        install -o root -g root -m 0644 -D "$assets/light-remote-$size.png" "$ICONS/${size}x${size}/apps/light-remote.png"
    done
    gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true
    exit 0
fi

HERE="$(cd "$(dirname "$0")" && pwd)"

# --- check what's needed
missing=()
command -v cargo >/dev/null || missing+=("Rust (https://rustup.rs)")
command -v pkexec >/dev/null || missing+=("pkexec (polkit)")
[ -x /usr/sbin/setcap ] || [ -x /sbin/setcap ] || command -v setcap >/dev/null \
    || missing+=("setcap (Debian/Ubuntu/Mint: libcap2-bin; Fedora/Arch: libcap)")
[ -e /sys/class/bluetooth/hci0 ] || missing+=("a Bluetooth adapter (hci0)")
systemctl is-active --quiet bluetooth 2>/dev/null || missing+=("the bluetooth service running (systemctl enable --now bluetooth)")
if [ ${#missing[@]} -gt 0 ]; then
    echo "Missing:"; printf '  - %s\n' "${missing[@]}"; exit 1
fi

# --- build
(cd "$HERE" && cargo build --release --locked)

# --- install (root)
pkexec "$HERE/install.sh" --system "$HERE/target/release" "$HERE"

# --- tray autostart
mkdir -p ~/.config/autostart
install -m 0644 "$HERE/crates/light-tray/assets/light-tray.desktop" ~/.config/autostart/light-tray.desktop

# --- (re)start the tray
pkill -x light-tray || true
sleep 0.5
setsid -f "$BIN/light-tray" >/dev/null 2>&1

echo
echo "Installed. Right-click the tray icon and choose 'Pair remote…', then press POWER twice on your remote."
echo "On the icon: scroll for brighter/dimmer, middle-click for power."
echo "To remove: $HERE/uninstall.sh"
