#!/usr/bin/env bash
# Render the binary Homebrew formula used by the custom Muxtrix tap.

set -euo pipefail

if [[ $# -ne 4 ]]; then
    echo "usage: $0 VERSION ARCHIVE_URL SHA256 OUTPUT" >&2
    exit 2
fi

version="$1"
archive_url="$2"
sha256="$3"
output="$4"

if [[ ! "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: invalid release version: ${version}" >&2
    exit 1
fi
if [[ ! "${archive_url}" =~ ^(https|file)://[A-Za-z0-9._~:/@%+-]+$ ]]; then
    echo "error: unsupported archive URL: ${archive_url}" >&2
    exit 1
fi
if [[ ! "${sha256}" =~ ^[0-9a-f]{64}$ ]]; then
    echo "error: invalid SHA-256: ${sha256}" >&2
    exit 1
fi

script_dir="$(cd "$(dirname "$0")" && pwd)"
template="${script_dir}/../packaging/homebrew/muxtrix.rb.template"
mkdir -p "$(dirname "${output}")"
sed \
    -e "s|@VERSION@|${version}|g" \
    -e "s|@URL@|${archive_url}|g" \
    -e "s|@SHA256@|${sha256}|g" \
    "${template}" > "${output}"
