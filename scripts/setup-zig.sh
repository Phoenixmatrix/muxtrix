#!/usr/bin/env bash
# Installs the Zig toolchain the vendored Ghostty build requires.
#
# The version is pinned deliberately. `libghostty-vt` 0.2.1's pinned Ghostty
# source rejects Zig 0.16.x at compile time even though upstream documentation
# claims that range, so tracking "latest" breaks the build. Bump ZIG_VERSION and
# both checksums together, and only after the binding moves.
#
# Installs side by side under a versioned directory and points one symlink at
# the active version, so switching back is a single command.
#
# Idempotent: re-running once the pinned version is active does nothing.
#
# Usage:
#   scripts/setup-zig.sh
#
# Environment:
#   ZIG_INSTALL_PREFIX  versioned install root (default ~/.local/share/zig)
#   ZIG_BIN_DIR         directory for the `zig` symlink (default ~/.local/bin)

set -euo pipefail

ZIG_VERSION="0.15.2"

# Checksums published at https://ziglang.org/download/index.json for this pin.
SHA256_X86_64_LINUX="02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239"
SHA256_AARCH64_LINUX="958ed7d1e00d0ea76590d27666efbf7a932281b3d7ba0c6b01b0ff26498f667f"

PREFIX="${ZIG_INSTALL_PREFIX:-$HOME/.local/share/zig}"
BIN_DIR="${ZIG_BIN_DIR:-$HOME/.local/bin}"

die() {
    echo "setup-zig: $*" >&2
    exit 1
}

for tool in curl tar sha256sum; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is required but not installed"
done

case "$(uname -s)" in
    Linux) ;;
    *)
        die "only Linux is supported here; native Windows uses its own pinned
             toolchain, see docs/TESTING.md"
        ;;
esac

case "$(uname -m)" in
    x86_64)  ZIG_ARCH="x86_64";  EXPECTED_SHA256="$SHA256_X86_64_LINUX" ;;
    aarch64) ZIG_ARCH="aarch64"; EXPECTED_SHA256="$SHA256_AARCH64_LINUX" ;;
    *) die "unsupported architecture $(uname -m)" ;;
esac

INSTALL_DIR="$PREFIX/$ZIG_VERSION"
LINK="$BIN_DIR/zig"

if [ -x "$LINK" ] && [ "$("$LINK" version 2>/dev/null || true)" = "$ZIG_VERSION" ]; then
    echo "setup-zig: Zig $ZIG_VERSION already active at $LINK"
    exit 0
fi

TARBALL="zig-${ZIG_ARCH}-linux-${ZIG_VERSION}.tar.xz"
URL="https://ziglang.org/download/${ZIG_VERSION}/${TARBALL}"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo "setup-zig: downloading $URL"
curl -fsSL --retry 3 -o "$WORK_DIR/$TARBALL" "$URL"

echo "setup-zig: verifying checksum"
ACTUAL_SHA256="$(sha256sum "$WORK_DIR/$TARBALL" | cut -d' ' -f1)"
if [ "$ACTUAL_SHA256" != "$EXPECTED_SHA256" ]; then
    die "checksum mismatch for $TARBALL
  expected $EXPECTED_SHA256
  actual   $ACTUAL_SHA256"
fi

echo "setup-zig: extracting"
tar -xJf "$WORK_DIR/$TARBALL" -C "$WORK_DIR"
EXTRACTED="$WORK_DIR/zig-${ZIG_ARCH}-linux-${ZIG_VERSION}"
[ -x "$EXTRACTED/zig" ] || die "archive did not contain an executable zig"

mkdir -p "$PREFIX" "$BIN_DIR"
rm -rf "$INSTALL_DIR"
mv "$EXTRACTED" "$INSTALL_DIR"
ln -sfn "$INSTALL_DIR/zig" "$LINK"

INSTALLED="$("$LINK" version)"
[ "$INSTALLED" = "$ZIG_VERSION" ] || die "installed Zig reports $INSTALLED, expected $ZIG_VERSION"

echo "setup-zig: installed Zig $ZIG_VERSION at $INSTALL_DIR"
echo "setup-zig: linked $LINK"

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "setup-zig: warning: $BIN_DIR is not on PATH; add it to use zig directly" ;;
esac
