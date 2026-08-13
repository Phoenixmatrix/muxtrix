#!/usr/bin/env bash
# Validate and push the version tag that starts the all-platform release.
# GitHub Actions builds and publishes Linux, Windows, macOS, apt, Scoop, and
# Homebrew from that single tag. This script never publishes partial assets.

set -euo pipefail
cd "$(dirname "$0")/.."

dry_run=false
skip_gates=false
for argument in "$@"; do
    case "$argument" in
        --dry-run) dry_run=true ;;
        --skip-gates) skip_gates=true ;;
        *) echo "unknown argument: $argument" >&2; exit 2 ;;
    esac
done

version="$(cargo metadata --format-version 1 --no-deps \
    | jq -r '.packages[] | select(.name == "muxtrix") | .version')"
tag="v${version}"
echo "==> Preparing ${tag}"

if [[ "$(git branch --show-current)" != "main" ]]; then
    echo "error: releases must be tagged from main" >&2
    exit 1
fi
if [[ -n "$(git status --porcelain)" ]]; then
    echo "error: working tree is not clean" >&2
    exit 1
fi

local_head="$(git rev-parse HEAD)"
remote_head="$(git ls-remote --heads origin refs/heads/main | awk '{ print $1 }')"
if [[ -z "${remote_head}" || "${remote_head}" != "${local_head}" ]]; then
    echo "error: HEAD must exactly match origin/main before release" >&2
    exit 1
fi
if git rev-parse --verify --quiet "refs/tags/${tag}" > /dev/null \
    || git ls-remote --exit-code --tags origin "refs/tags/${tag}" > /dev/null 2>&1; then
    echo "error: tag ${tag} already exists" >&2
    exit 1
fi

if ! $skip_gates; then
    echo "==> Quality gates"
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --all-targets --locked
fi

if $dry_run; then
    echo "==> Dry run complete; ${tag} is ready to tag"
    exit 0
fi

git tag --annotate "${tag}" --message "Muxtrix ${version}"
git push origin "refs/tags/${tag}"
echo "==> ${tag} pushed; GitHub Actions is publishing every platform and package repository"
