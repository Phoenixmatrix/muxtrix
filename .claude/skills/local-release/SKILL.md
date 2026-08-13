---
name: local-release
description: Build and publish a Muxtrix release (Linux + Windows) from this machine, bypassing GitHub Actions. Use ONLY when a local release is explicitly needed — Actions minutes exhausted, CI release lanes parked, or the user asks for a local build/release. Not part of the normal feature/commit cycle; ordinary commits just push to main and let CI run its Linux quality gates.
---

# Local release (Linux + Windows, no GitHub Actions)

`scripts/release.sh` reproduces everything the parked CI release lanes did:
quality gates, both platform builds, packaging, the GitHub release, the Scoop
manifest, and the flat apt repository — from this WSL machine. Windows builds
are **cross-compiled** with cargo-zigbuild (`x86_64-pc-windows-gnu`, Zig as
linker); nothing is installed or run on the Windows host.

## When to use

- The user asks for a release/build "locally" or "without Actions".
- CI release lanes are parked (see `.github/workflows/build.yml` — matrix
  entries and publish jobs behind comments/`if: false`).

Do NOT run this for routine commits: those push to main and CI runs the
Linux-only quality gates. A release is its own deliberate act.

## Prerequisites (one-time; all already present on this machine)

- `rustup target add x86_64-pc-windows-gnu`
- `cargo install cargo-zigbuild cargo-deb --locked`
- Zig on PATH (the vendored libghostty-vt-sys build already requires it)
- `binutils-mingw-w64-x86-64` **and** `gcc-mingw-w64-x86-64` (windres + its
  preprocessor — embeds the Windows app icon; the build **panics** without
  them for windows targets)
- `gh` authenticated; `dpkg-scanpackages` + `apt-ftparchive` (dpkg-dev,
  apt-utils) for the apt branch

## Release flow

1. Bump `version` in the workspace `Cargo.toml`, then `cargo check --workspace`
   — this refreshes `Cargo.lock`; forgetting it fails the script's `--locked`
   builds.
2. Note the release in `docs/ROADMAP.md` if it closes a milestone.
3. `cargo fmt --all` before committing — the script's gates run
   `fmt --check` and abort on drift.
4. Commit everything (script refuses a dirty tree).
5. Run `scripts/release.sh`. Flags:
   - `--dry-run` — build + package only, publish nothing (use to validate).
   - `--skip-gates` — skip fmt/clippy/test when they just passed.
6. The script then: builds Linux natively and Windows via zigbuild; packages
   `muxtrix-<v>-linux-x64.tar.gz`, `muxtrix-<v>-x64.zip` (exactly
   `muxtrix.exe` + `muxtrixctl.exe` at the zip root — the Scoop manifest
   validates this shape), `muxtrix_<v>_amd64.deb`, `SHA256SUMS`; pushes main
   + the `v<version>` tag; creates the GitHub release; updates the Scoop
   bucket (Phoenixmatrix/scoop-muxtrix) and the `apt-repo` branch.

## Verify after publishing

```bash
gh release view v<version> --json assets -q '[.assets[].name]'
gh api repos/Phoenixmatrix/scoop-muxtrix/contents/bucket/muxtrix.json \
  -q '.content' | base64 -d | grep '"version"'
```

The Scoop hash must equal `sha256sum dist/muxtrix-<v>-x64.zip`.

## Known traps

- **Immutable releases**: the script refuses to overwrite an existing
  `v<version>` release. A failed run after tagging → fix, then move the tag
  only if no release object was created.
- **Icon regression**: the Windows icon embeds via mingw windres in
  `crates/muxtrix-app/build.rs` (target-keyed, NOT `cfg(windows)` — build
  scripts run on the host). Verify with
  `x86_64-w64-mingw32-objdump -h target/x86_64-pc-windows-gnu/release/muxtrix.exe | grep rsrc`.
- **Toolchain caveat**: local Windows binaries are `windows-gnu`, not MSVC
  (CI used MSVC). Functionally equivalent so far; if a Windows-only
  regression appears after a local release, suspect this first.
- **Session daemons** on the user's Windows machine run from staged copies
  under `~/.muxtrix/bin`, so a running daemon never blocks a Scoop update.
