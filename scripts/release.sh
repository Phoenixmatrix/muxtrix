#!/usr/bin/env bash
# Local release pipeline: builds Linux and Windows artifacts on this
# machine (Windows via cargo-zigbuild cross-compilation — no Windows-side
# toolchain needed) and publishes the GitHub release, Scoop manifest, and
# apt repository without GitHub Actions.
#
# Usage:
#   scripts/release.sh            full release for the version in Cargo.toml
#   scripts/release.sh --dry-run  build and package only; publish nothing
#   scripts/release.sh --skip-gates   skip fmt/clippy/test (already green)
#
# Requires: rustup target x86_64-pc-windows-gnu, cargo-zigbuild, zig,
# cargo-deb, gh (authenticated), dpkg-deb, dpkg-scanpackages, apt-ftparchive,
# desktop-file-validate, and file.

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
echo "==> Releasing ${tag}"

if [[ -n "$(git status --porcelain)" ]]; then
    echo "error: working tree is not clean" >&2
    exit 1
fi

if ! $dry_run && gh release view "${tag}" > /dev/null 2>&1; then
    echo "error: release ${tag} already exists; versioned assets are immutable" >&2
    exit 1
fi

if ! $skip_gates; then
    echo "==> Quality gates"
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
fi

linux_rust_target="x86_64-unknown-linux-gnu"
linux_zig_target="${linux_rust_target}.2.34"

echo "==> Building Linux x64 (glibc 2.34 baseline)"
cargo zigbuild --release --locked --target "${linux_zig_target}" -p muxtrix --bin muxtrix
cargo zigbuild --release --locked --target "${linux_zig_target}" -p muxtrix-control --bin muxtrixctl

echo "==> Building Windows x64 (zig cross)"
cargo zigbuild --release --locked --target x86_64-pc-windows-gnu -p muxtrix --bin muxtrix
cargo zigbuild --release --locked --target x86_64-pc-windows-gnu -p muxtrix-control --bin muxtrixctl

echo "==> Packaging"
rm -rf dist
mkdir -p dist/linux
linux_asset="muxtrix-${version}-linux-x64.tar.gz"
windows_asset="muxtrix-${version}-x64.zip"
deb_asset="muxtrix_${version}_amd64.deb"
cp \
    "target/${linux_rust_target}/release/muxtrix" \
    "target/${linux_rust_target}/release/muxtrixctl" \
    dist/linux/
cp THIRD_PARTY_NOTICES.md dist/linux/
tar --create --gzip --file "dist/${linux_asset}" --directory dist/linux .
# Keep the executables and their required third-party notices at the zip root.
python3 - "$version" <<'PYEOF'
import sys, zipfile
version = sys.argv[1]
with zipfile.ZipFile(f"dist/muxtrix-{version}-x64.zip", "w", zipfile.ZIP_DEFLATED) as archive:
    for name in ("muxtrix.exe", "muxtrixctl.exe"):
        archive.write(f"target/x86_64-pc-windows-gnu/release/{name}", name)
    archive.write("THIRD_PARTY_NOTICES.md", "THIRD_PARTY_NOTICES.md")
PYEOF
cargo deb \
    -p muxtrix \
    --target "${linux_rust_target}" \
    --no-build \
    --no-strip \
    --output "dist/${deb_asset}"
scripts/verify-deb-package.sh "dist/${deb_asset}"
python3 - "dist/${windows_asset}" <<'PYEOF'
import sys, zipfile
expected = {
    "muxtrix.exe",
    "muxtrixctl.exe",
    "THIRD_PARTY_NOTICES.md",
}
with zipfile.ZipFile(sys.argv[1]) as archive:
    actual = set(archive.namelist())
if actual != expected:
    raise SystemExit(f"unexpected Windows archive contents: {sorted(actual)}")
PYEOF
tar --extract --gzip --to-stdout --file "dist/${linux_asset}" ./THIRD_PARTY_NOTICES.md > /dev/null
(
    cd dist
    sha256sum "${linux_asset}" "${windows_asset}" "${deb_asset}" > SHA256SUMS
)
rm -rf dist/linux
echo "==> Packaged:"
ls -l dist

if $dry_run; then
    echo "==> Dry run complete; nothing published"
    exit 0
fi

echo "==> Tagging and publishing release"
if ! git rev-parse "${tag}" > /dev/null 2>&1; then
    git tag "${tag}"
fi
git push origin main "${tag}"
gh release create "${tag}" \
    "dist/${windows_asset}" \
    "dist/${linux_asset}" \
    "dist/${deb_asset}" \
    dist/SHA256SUMS \
    --verify-tag \
    --title "Muxtrix ${version}" \
    --notes "Muxtrix ${version} for Windows x64 and Linux x64. These builds are unsigned; verify them against SHA256SUMS."

echo "==> Updating Scoop bucket"
scoop_dir="$(mktemp -d)"
git clone --depth 1 "https://github.com/Phoenixmatrix/scoop-muxtrix.git" "${scoop_dir}"
hash="$(sha256sum "dist/${windows_asset}" | cut -d ' ' -f 1)"
url="https://github.com/Phoenixmatrix/muxtrix/releases/download/${tag}/${windows_asset}"
jq --arg version "${version}" --arg url "${url}" --arg hash "${hash}" \
    '.version = $version
     | .architecture."64bit".url = $url
     | .architecture."64bit".hash = $hash' \
    "${scoop_dir}/bucket/muxtrix.json" > "${scoop_dir}/bucket/muxtrix.json.tmp"
mv "${scoop_dir}/bucket/muxtrix.json.tmp" "${scoop_dir}/bucket/muxtrix.json"
git -C "${scoop_dir}" commit -am "Update Muxtrix to ${version}"
git -C "${scoop_dir}" push origin HEAD:main
rm -rf "${scoop_dir}"

echo "==> Updating apt repository branch"
apt_dir="$(mktemp -d)"
if git ls-remote --exit-code origin apt-repo > /dev/null 2>&1; then
    git clone --branch apt-repo --single-branch \
        "$(git remote get-url origin)" "${apt_dir}"
else
    git -C "${apt_dir}" init --initial-branch apt-repo
    git -C "${apt_dir}" remote add origin "$(git remote get-url origin)"
fi
mkdir -p "${apt_dir}/pool"
cp "dist/${deb_asset}" "${apt_dir}/pool/"
scripts/generate-apt-repository.sh "${apt_dir}"
scripts/verify-apt-repository.sh "${apt_dir}" "${version}"
(
    cd "${apt_dir}"
    git add --all
    git commit -m "Publish muxtrix ${version} to the apt repository"
    git push origin HEAD:apt-repo
)
rm -rf "${apt_dir}"

echo "==> Release ${tag} published: GitHub assets, Scoop, and apt updated"
