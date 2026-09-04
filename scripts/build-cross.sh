#!/usr/bin/env bash
# build-cross.sh — cross-compile souveraine for aarch64 (Pixel 3 / Arch ARM).
#
# The sysroot is /usr/aarch64-linux-gnu on the laptop and
# ~/aarch64-sysroot on a build host.  Set SOUVERAINE_AARCH64_SYSROOT for the latter.
# PKG_CONFIG_* is exported here, not in .cargo/config.toml — cargo's [env]
# block doesn't reliably reach build-script probes.  The aarch64 linker scripts
# need a small unprivileged overlay so absolute /usr/lib paths stay inside the
# target sysroot rather than resolving to the x86 host.
set -euo pipefail

cd "$(dirname "$0")/.."

SYSROOT="${SOUVERAINE_AARCH64_SYSROOT:-/usr/aarch64-linux-gnu}"
TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
LINKER_SCRIPTS="$TARGET_DIR/aarch64-linker-scripts"

[[ -d "$SYSROOT/usr/lib" ]] || {
    echo "aarch64 sysroot is missing: $SYSROOT" >&2
    exit 1
}

mkdir -p "$LINKER_SCRIPTS"
# The artifact runner does not guarantee /dev/fd, so keep this stream on the
# pipeline instead of feeding the loop through process substitution.
find "$SYSROOT/usr/lib" -maxdepth 1 -type f -name '*.so' \
    -exec grep -lE '(^|[ (])/(usr/)?lib/' {} + |
while IFS= read -r script; do
    name="${script##*/}"
    tmp="$LINKER_SCRIPTS/.${name}.$$"
    # '=' tells GNU ld to resolve this path below --sysroot.  Only linker
    # scripts (plain-text *.so files) are copied; actual shared objects stay
    # in the sysroot.
    sed -E 's#([ (])/(usr/)?lib/#\1=/\2lib/#g' "$script" > "$tmp"
    mv "$tmp" "$LINKER_SCRIPTS/$name"
done

export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_LIBDIR="$SYSROOT/usr/lib/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="$SYSROOT"
export PKG_CONFIG="$PWD/.cargo/aarch64-pkg-config"
export SOUVERAINE_AARCH64_SYSROOT="$SYSROOT"
export SOUVERAINE_AARCH64_LINKER_SCRIPTS="$LINKER_SCRIPTS"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$PWD/scripts/aarch64-linker"

BIN_NAME=souveraine
EXPECT_BIN_NAME=false
for arg in "$@"; do
    if $EXPECT_BIN_NAME; then
        BIN_NAME="$arg"
        EXPECT_BIN_NAME=false
        continue
    fi
    case "$arg" in
        --bin) EXPECT_BIN_NAME=true ;;
        --bin=*) BIN_NAME="${arg#--bin=}" ;;
    esac
done

cargo build --release --target aarch64-unknown-linux-gnu "$@"

BIN="$TARGET_DIR/aarch64-unknown-linux-gnu/release/$BIN_NAME"
[[ -x "$BIN" ]] || {
    echo "requested build produced no executable: $BIN" >&2
    exit 1
}
echo "== built: $BIN =="
file "$BIN"
