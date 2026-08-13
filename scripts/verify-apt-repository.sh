#!/usr/bin/env bash
# Exercise a flat apt repository with an isolated, unprivileged apt client.

set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 REPOSITORY_DIRECTORY EXPECTED_UPSTREAM_VERSION" >&2
    exit 2
fi

repository="$(realpath "$1")"
expected_version="$2"

for file in Packages Packages.gz Release; do
    if [[ ! -f "${repository}/${file}" ]]; then
        echo "error: apt repository is missing ${file}" >&2
        exit 1
    fi
done

if ! gzip --decompress --stdout "${repository}/Packages.gz" \
    | cmp --silent - "${repository}/Packages"; then
    echo "error: Packages.gz does not match Packages" >&2
    exit 1
fi
if grep --extended-regexp --quiet '[[:space:]][0-9]+ Release$' "${repository}/Release"; then
    echo "error: Release must not contain a checksum for itself" >&2
    exit 1
fi

apt_root="$(mktemp -d "${TMPDIR:-/tmp}/muxtrix-apt-client.XXXXXX")"
trap 'rm -rf "${apt_root}"' EXIT
mkdir -p \
    "${apt_root}/cache/archives/partial" \
    "${apt_root}/etc/apt.conf.d" \
    "${apt_root}/etc/sourceparts" \
    "${apt_root}/state/lists/partial"
touch "${apt_root}/etc/apt.conf"

repository_uri="${repository// /%20}"
printf 'deb [trusted=yes] file:%s ./\n' "${repository_uri}" \
    > "${apt_root}/etc/sources.list"

apt_options=(
    -o "Dir::Etc::main=${apt_root}/etc/apt.conf"
    -o "Dir::Etc::parts=${apt_root}/etc/apt.conf.d"
    -o "Dir::Etc::sourcelist=${apt_root}/etc/sources.list"
    -o "Dir::Etc::sourceparts=${apt_root}/etc/sourceparts"
    -o "Dir::State=${apt_root}/state"
    -o "Dir::State::status=/var/lib/dpkg/status"
    -o "Dir::Cache=${apt_root}/cache"
    -o APT::Get::List-Cleanup=0
    -o Debug::NoLocking=1
)

apt-get "${apt_options[@]}" update
candidate="$(apt-cache "${apt_options[@]}" policy muxtrix \
    | awk '/Candidate:/ { print $2; exit }')"
if [[ "${candidate}" != "${expected_version}-1" ]]; then
    echo "error: expected apt candidate ${expected_version}-1, found ${candidate}" >&2
    exit 1
fi

(
    cd "${apt_root}/cache/archives"
    apt-get "${apt_options[@]}" download "muxtrix=${candidate}"
)
if ! find "${apt_root}/cache/archives" -maxdepth 1 -type f -name 'muxtrix_*.deb' \
    -print -quit | grep --quiet .; then
    echo "error: apt did not download the muxtrix package" >&2
    exit 1
fi

echo "verified apt repository: muxtrix ${candidate}"
