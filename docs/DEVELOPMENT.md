# Development

## Headless checks

Ghostty VT currently requires Zig 0.15.2. The upstream Rust binding's README
claims 0.16.x, but the Ghostty source pinned by `libghostty-vt` 0.2.1 rejects
that version at compile time. Local development therefore uses 0.15.2 until
the binding resolves the mismatch. Future CI must use the same pin.

Install that toolchain once per machine:

```sh
scripts/setup-zig.sh
```

It verifies the published SHA-256, installs under
`~/.local/share/zig/<version>`, and links `~/.local/bin/zig` at the active
version. Re-running once the pin is active does nothing, so it is safe from any
setup path. Because versions install side by side, changing the pin later is a
symlink change rather than a reinstall — bump `ZIG_VERSION` and both checksums
together, and only after the binding moves off 0.15.2. Override
`ZIG_INSTALL_PREFIX` or `ZIG_BIN_DIR` to install elsewhere.

Run these without starting the application:

```sh
cargo fmt --all --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Run the real-application E2E separately on Linux:

```sh
cargo test -p muxtrix --features e2e --test headless_e2e -- --test-threads=1
```

This requires `Xvfb` and `xvfb-run`. The test creates a private virtual display,
forces X11 and wgpu to Mesa Vulkan/llvmpipe, injects input through the XTest
extension, photographs the window from the X server, and exits automatically. It
does not connect to WSLg and cannot place a window on the host desktop. See
`docs/TESTING.md` for its assertions and architecture.

Cross-target checks are added only when the relevant Rust target and native
linker are available. Native Windows validation uses a Visual Studio Developer
PowerShell with Rust MSVC and the same Zig 0.15.2 pin; see `docs/TESTING.md`.

## Continuous builds

Every push to `main` runs `.github/workflows/build.yml`. A Linux checks job runs
formatting, unit and integration tests, Clippy with warnings denied, and the
real application E2E on a private Xvfb display. Native release jobs then build
Linux x64, Windows x64, and macOS arm64 artifacts. The workflow can also be run
manually from GitHub's Actions page.

## Releases

Releases are cut from tags in the exact `vMAJOR.MINOR.PATCH` form and publish
versioned Linux x64, Windows x64, and macOS arm64 assets. The tag must match the
workspace version. All three platform jobs and a real Homebrew formula install
must pass before the GitHub release is created. Scoop, Homebrew, and apt are
then updated in parallel from those exact versioned assets.

The Windows ZIP is created with stable entry order and timestamps and contains
only `muxtrix.exe`, `muxtrixctl.exe`, and `THIRD_PARTY_NOTICES.md` at its root.
The macOS binaries are ad-hoc signed and checked as arm64 before packaging, but
are not Developer ID signed or notarized. Verify every download against the
published `SHA256SUMS`.

Linux release binaries are built through `cargo-zigbuild` against glibc 2.34
even when the release host is newer. Keep the version suffix on
`x86_64-unknown-linux-gnu.2.34` and pass the unsuffixed Rust target to
`cargo-deb`; otherwise a release made on a newer Ubuntu can silently require
that host's newer libc.

See `docs/RELEASING.md` for the repository secrets, package repository
permissions, and tag procedure.

If a sandbox prevents Cargo or Zig from writing their normal user caches,
point `CARGO_HOME` and `ZIG_GLOBAL_CACHE_DIR` at ignored, writable directories.

## GUI safety

`cargo run -p muxtrix` opens the native application window and is not part of
routine validation. Do not run it on a shared host without explicit permission.
The `headless_e2e` target
is the approved exception: its real window exists only on a private Xvfb server
and is never attached to the user's display.

## Graphics diagnostics

Use the Muxtrix probe to inspect the same wgpu adapter policy without creating
a window:

```sh
cargo run --bin muxtrix-gpu-probe -- \
  --require-hardware \
  --require-adapter NVIDIA
```

See `docs/GPU.md` for WSLg backend selection and overrides. `vulkaninfo` alone
is not authoritative because the accelerated WSLg route is typically Mesa D3D12
through OpenGL/EGL, not Vulkan.

## Linux build dependencies

Beyond a Rust toolchain, a Linux build needs the X, GL, Vulkan and Wayland
development headers. The GPUI port adds `libxkbcommon-x11-dev`, without which
everything compiles and only the final link fails:

```bash
sudo apt-get install -y \
  libfontconfig-dev libfreetype-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libvulkan-dev libgl1-mesa-dev libzstd-dev
```

`mesa-vulkan-drivers` and `xvfb` are additionally required to run the headless
e2e test and the capture gallery.

## Dependency policy

- Prefer stable Rust and crates with Linux, Windows, and macOS support.
- Pin versions in `Cargo.lock` for reproducible application builds.
- Keep unsafe/native bindings isolated behind small adapters.
- Use pnpm rather than npm if non-Rust package tooling becomes necessary.
