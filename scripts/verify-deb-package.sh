#!/usr/bin/env bash
# Validate binaries and Linux desktop integration inside a Muxtrix .deb.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 PACKAGE.deb" >&2
    exit 2
fi

package="$1"
if [[ ! -f "${package}" ]]; then
    echo "error: package not found: ${package}" >&2
    exit 1
fi

for command in dpkg dpkg-deb desktop-file-validate file; do
    if ! command -v "${command}" > /dev/null; then
        echo "error: required command not found: ${command}" >&2
        exit 1
    fi
done

if [[ "$(dpkg-deb --field "${package}" Package)" != "muxtrix" ]]; then
    echo "error: Debian package name is not muxtrix" >&2
    exit 1
fi
if [[ "$(dpkg-deb --field "${package}" Architecture)" != "amd64" ]]; then
    echo "error: Debian package architecture is not amd64" >&2
    exit 1
fi

dependencies="$(dpkg-deb --field "${package}" Depends)"
if [[ ", ${dependencies}," != *", hicolor-icon-theme,"* ]]; then
    echo "error: Debian package does not depend on hicolor-icon-theme" >&2
    exit 1
fi
if [[ "${dependencies}" != *"libc6 (>= "* ]]; then
    echo "error: Debian package has no generated libc6 dependency" >&2
    exit 1
fi
while read -r libc_requirement; do
    if dpkg --compare-versions "${libc_requirement}" gt 2.34; then
        echo "error: libc6 requirement ${libc_requirement} exceeds the 2.34 release baseline" >&2
        exit 1
    fi
done < <(grep --only-matching --extended-regexp 'libc6 \(>= [^)]+\)' <<< "${dependencies}" \
    | sed --expression='s/^libc6 (>= //' --expression='s/)$//')

package_root="$(mktemp -d "${TMPDIR:-/tmp}/muxtrix-deb.XXXXXX")"
trap 'rm -rf "${package_root}"' EXIT
dpkg-deb --extract "${package}" "${package_root}"

test -x "${package_root}/usr/bin/muxtrix"
test -x "${package_root}/usr/bin/muxtrixctl"
test -f "${package_root}/usr/share/doc/muxtrix/THIRD_PARTY_NOTICES.md"

desktop_file="${package_root}/usr/share/applications/muxtrix.desktop"
icon_file="${package_root}/usr/share/icons/hicolor/256x256/apps/muxtrix.png"
desktop-file-validate "${desktop_file}"
grep --fixed-strings --line-regexp 'Exec=muxtrix' "${desktop_file}" > /dev/null
grep --fixed-strings --line-regexp 'Icon=muxtrix' "${desktop_file}" > /dev/null
file "${icon_file}" | grep --fixed-strings 'PNG image data, 256 x 256, 8-bit/color RGBA' > /dev/null

for binary in muxtrix muxtrixctl; do
    if ldd "${package_root}/usr/bin/${binary}" 2>&1 | grep --quiet 'not found'; then
        echo "error: ${binary} has unresolved shared-library dependencies" >&2
        ldd "${package_root}/usr/bin/${binary}" >&2
        exit 1
    fi
done

echo "verified Debian package: ${package}"
