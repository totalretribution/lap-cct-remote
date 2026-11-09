#!/usr/bin/env bash
# Build .deb and .rpm packages into packages/.
#
#   ./package.sh            version from Cargo.toml
#   ./package.sh 0.2.0      this version instead (CI passes the release's tag)
#
# The binaries are built against glibc $GLIBC with cargo-zigbuild, so one build runs on older
# distributions too: Debian 11, Ubuntu 20.04, Mint 20, and anything newer (Fedora included).
#
# Needs zig, and: cargo install --locked cargo-deb cargo-generate-rpm cargo-zigbuild
set -euo pipefail

GLIBC=2.31
TARGET=x86_64-unknown-linux-gnu
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/packages"
VERSION="${1:-}"

# Debian and RPM versions can't contain '-', so keep to numbers, dots and letters
if [ -n "$VERSION" ] && ! [[ "$VERSION" =~ ^[0-9]+(\.[0-9A-Za-z]+)*$ ]]; then
    echo "Bad version '$VERSION': use something like 0.2.0"
    exit 1
fi

missing=()
for tool in cargo-deb cargo-generate-rpm cargo-zigbuild zig; do
    command -v "$tool" >/dev/null || missing+=("$tool")
done
if [ ${#missing[@]} -gt 0 ]; then
    echo "Missing: ${missing[*]}"
    echo "  cargo install --locked cargo-deb cargo-generate-rpm cargo-zigbuild"
    echo "  zig: https://ziglang.org/download/ (or your distribution's package)"
    exit 1
fi

cd "$HERE"
cargo zigbuild --release --locked --target "$TARGET.$GLIBC" -p light-tray -p light-radio

deb_args=() rpm_args=()
if [ -n "$VERSION" ]; then
    deb_args=(--deb-version "$VERSION-1")
    rpm_args=(--set-metadata "version = \"$VERSION\"")
fi

rm -rf "$OUT"
mkdir -p "$OUT"
cargo deb -p light-tray --no-build --target "$TARGET" --output "$OUT" "${deb_args[@]}"
cargo generate-rpm -p crates/light-tray --target "$TARGET" --output "$OUT" "${rpm_args[@]}"

echo
echo "Packages:"
ls -1 "$OUT"
