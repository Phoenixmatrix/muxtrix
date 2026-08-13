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
forces winit to X11 and wgpu to Mesa Vulkan/llvmpipe, injects input through the
XTest extension, captures an Iced GPU screenshot, and exits automatically. It
does not connect to WSLg and cannot place a window on the host desktop. See
`docs/TESTING.md` for its assertions and architecture.

Cross-target checks are added only when the relevant Rust target and native
linker are available. Native Windows validation uses a Visual Studio Developer
PowerShell with Rust MSVC and the same Zig 0.15.2 pin; see `docs/TESTING.md`.

## Continuous builds

Every push to `main` runs `.github/workflows/build.yml` on a GitHub-hosted
Linux runner. The job runs formatting, unit and integration tests, and Clippy
with warnings denied before building `muxtrix` and `muxtrixctl` in release
mode, then runs the real application E2E on a private Xvfb display. The
workflow can also be run manually from GitHub's Actions page.

## Releases

Releases are cut from tags in the exact `vMAJOR.MINOR.PATCH` form and publish
versioned Linux x64 and Windows x64 assets. The Windows ZIP is created with
stable entry order and timestamps and contains only `muxtrix.exe` and
`muxtrixctl.exe` at its root. Release binaries are unsigned; verify downloads
against the published `SHA256SUMS`.

Linux release binaries are built through `cargo-zigbuild` against glibc 2.34
even when the release host is newer. Keep the version suffix on
`x86_64-unknown-linux-gnu.2.34` and pass the unsuffixed Rust target to
`cargo-deb`; otherwise a release made on a newer Ubuntu can silently require
that host's newer libc.

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

## Dependency policy

- Prefer stable Rust and crates with Linux, Windows, and macOS support.
- Pin versions in `Cargo.lock` for reproducible application builds.
- Keep unsafe/native bindings isolated behind small adapters.
- Use pnpm rather than npm if non-Rust package tooling becomes necessary.
