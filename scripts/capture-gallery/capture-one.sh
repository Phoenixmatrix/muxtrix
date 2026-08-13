#!/usr/bin/env bash
# Captures one Muxtrix frame headlessly and writes it as a PNG.
#
# Usage: capture-one.sh SLUG CAPTURE VIEWPORT [SETTINGS_JSON]
#   SLUG           output basename under $OUT_DIR
#   CAPTURE        MUXTRIX_E2E_CAPTURE value, or "-" for the workspace view
#   VIEWPORT       WIDTHxHEIGHT
#   SETTINGS_JSON  optional settings profile seeded before the app starts
set -uo pipefail

slug="$1"
capture="$2"
viewport="$3"
profile="${4:-}"

: "${OUT_DIR:?OUT_DIR must be set}"
: "${TEST_BIN:?TEST_BIN must be set}"

width="${viewport%x*}"
height="${viewport#*x}"
rgba="$(mktemp --suffix=.rgba)"
log="$OUT_DIR/logs/$slug.log"
mkdir -p "$OUT_DIR/logs"
trap 'rm -f "$rgba"' EXIT

env_args=("MUXTRIX_E2E_SCREENSHOT_RGBA=$rgba" "MUXTRIX_E2E_VIEWPORT=$viewport")
[[ "$capture" != "-" ]] && env_args+=("MUXTRIX_E2E_CAPTURE=$capture")
[[ -n "$profile" ]] && env_args+=("MUXTRIX_E2E_SETTINGS=$profile")

if ! env "${env_args[@]}" "$TEST_BIN" >"$log" 2>&1; then
    echo "FAIL $slug" >&2
    exit 1
fi

python3 - "$rgba" "$OUT_DIR/$slug.png" "$width" "$height" <<'EOF'
import struct, sys, zlib

rgba_path, png_path, width, height = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
data = open(rgba_path, "rb").read()
expected = width * height * 4
if len(data) != expected:
    raise SystemExit(f"RGBA dump is {len(data)} bytes but {width}x{height} needs {expected}")
raw = b"".join(b"\x00" + data[y * width * 4:(y + 1) * width * 4] for y in range(height))

def chunk(kind, body):
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(raw, 6))
    + chunk(b"IEND", b"")
)
open(png_path, "wb").write(png)
EOF
status=$?
if [[ $status -ne 0 ]]; then
    echo "FAIL $slug (png)" >&2
    exit 1
fi
echo "OK $slug"
