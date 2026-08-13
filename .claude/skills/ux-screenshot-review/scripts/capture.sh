#!/usr/bin/env bash
# Captures a real rendered Muxtrix frame headlessly (Xvfb + llvmpipe) via the
# e2e harness and converts it to PNG. Never opens a visible window.
#
# Usage: capture.sh OUTPUT.png [--viewport WIDTHxHEIGHT] [--capture STATE]
#
#   --viewport   Window size (default 1280x800; minimum 720x480).
#   --capture    End the scenario on a named surface instead of the default
#                workspace view. Every name is a branch of `stage_capture`
#                in crates/muxtrix-app/src/e2e.rs; run
#                `grep 'capturing("' crates/muxtrix-app/src/e2e.rs` for the
#                current list.
set -euo pipefail

output=""
viewport="1280x800"
capture=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --viewport) viewport="$2"; shift 2 ;;
        --capture) capture="$2"; shift 2 ;;
        *) output="$1"; shift ;;
    esac
done
if [[ -z "$output" ]]; then
    echo "usage: capture.sh OUTPUT.png [--viewport WIDTHxHEIGHT] [--capture STATE]" >&2
    exit 2
fi

width="${viewport%x*}"
height="${viewport#*x}"
rgba="$(mktemp --suffix=.rgba)"
trap 'rm -f "$rgba"' EXIT

env_args=(
    "MUXTRIX_E2E_SCREENSHOT_RGBA=$rgba"
    "MUXTRIX_E2E_VIEWPORT=$viewport"
)
if [[ -n "$capture" ]]; then
    env_args+=("MUXTRIX_E2E_CAPTURE=$capture")
fi

repo_root="$(cd "$(dirname "$0")/../../../.." && pwd)"
(cd "$repo_root" && env "${env_args[@]}" \
    cargo test -p muxtrix --features e2e --test headless_e2e -- --test-threads=1) >&2

python3 - "$rgba" "$output" "$width" "$height" <<'EOF'
import struct, sys, zlib

rgba_path, png_path, width, height = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
data = open(rgba_path, "rb").read()
expected = width * height * 4
if len(data) != expected:
    raise SystemExit(
        f"RGBA dump is {len(data)} bytes but {width}x{height} needs {expected}; "
        "pass the same --viewport the capture used"
    )
raw = b"".join(b"\x00" + data[y * width * 4:(y + 1) * width * 4] for y in range(height))

def chunk(kind, body):
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(raw, 9))
    + chunk(b"IEND", b"")
)
open(png_path, "wb").write(png)
print(png_path)
EOF
