#!/usr/bin/env bash
# Generate the indexes for Muxtrix's unsigned flat apt repository.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 REPOSITORY_DIRECTORY" >&2
    exit 2
fi

repository="$1"
if [[ ! -d "${repository}/pool" ]]; then
    echo "error: ${repository}/pool does not exist" >&2
    exit 1
fi

for command in dpkg-scanpackages apt-ftparchive gzip; do
    if ! command -v "${command}" > /dev/null; then
        echo "error: required command not found: ${command}" >&2
        exit 1
    fi
done

release_output="$(mktemp "${TMPDIR:-/tmp}/muxtrix-apt-release.XXXXXX")"
trap 'rm -f "${release_output}"' EXIT

(
    cd "${repository}"
    rm -f Packages Packages.* Release
    dpkg-scanpackages --multiversion pool /dev/null > Packages
    # dpkg-scanpackages terminates its final stanza with an extra blank line,
    # which makes `git diff --check` reject the generated repository.
    sed -i '${/^$/d;}' Packages
    gzip --no-name --keep --force Packages

    # apt-ftparchive otherwise sees the previous Release while it scans and
    # emits a stale checksum for Release inside Release itself.
    rm -f Release
    apt-ftparchive \
        -o APT::FTPArchive::Release::Origin=Muxtrix \
        -o APT::FTPArchive::Release::Label=Muxtrix \
        -o APT::FTPArchive::Release::Architectures=amd64 \
        -o APT::FTPArchive::Release::Description="Muxtrix Debian packages" \
        release . > "${release_output}"
    mv "${release_output}" Release
)
