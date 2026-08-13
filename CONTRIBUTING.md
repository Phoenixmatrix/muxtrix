# Contributing to Muxtrix

Muxtrix is maintained by one person. Issues and pull requests are welcome, but
please open an issue to discuss anything substantial before writing a large
patch — it may not fit the direction, and reviewing a big PR takes time I may
not have quickly.

## Prerequisites

- **Rust**, stable channel. The pinned toolchain and components are declared in
  `rust-toolchain.toml`, so `rustup` picks them up automatically.
- **Zig 0.15.2**, required by Ghostty's VT engine. The pin is deliberate: the
  Ghostty source used by `libghostty-vt` 0.2.1 rejects 0.16.x at compile time
  despite the binding's README claiming otherwise. Install it with:

  ```sh
  scripts/setup-zig.sh
  ```

  The script verifies the published SHA-256, installs under
  `~/.local/share/zig/<version>`, and links `~/.local/bin/zig`. It is
  idempotent, and versions install side by side.

- **Linux development packages** for the GPU backend and the headless test
  display:

  ```sh
  sudo apt-get install --no-install-recommends \
    libgl1-mesa-dev libvulkan-dev libwayland-dev libx11-xcb-dev \
    libxkbcommon-dev libxkbcommon-x11-dev libxcursor-dev libxi-dev \
    libxinerama-dev libxrandr-dev mesa-vulkan-drivers \
    desktop-file-utils file pkg-config xauth xvfb
  ```

## Build

```sh
cargo build --release
```

Produces `muxtrix` and `muxtrixctl` in `target/release/`.

## Tests and checks

These four run headlessly and are what CI enforces on every push:

```sh
cargo fmt --all --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Clippy runs with warnings denied, so a warning is a failure.

The real-application end-to-end test runs separately, on Linux only:

```sh
cargo test -p muxtrix --features e2e --test headless_e2e -- --test-threads=1
```

It requires `Xvfb` and `xvfb-run`. The test creates its own private virtual
display, forces winit to X11 and wgpu to Mesa Vulkan/llvmpipe, injects input
through the XTest extension, captures a GPU screenshot, and exits on its own.
It never touches your desktop session. See [docs/TESTING.md](docs/TESTING.md)
for what it asserts and how it is put together.

**Automated checks must stay headless.** Do not launch the GUI in a shared
development environment unless the person using that host has explicitly opted
in.

## Architecture

Start with [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the boundaries
between the GPU layer, terminal emulation, process hosts, persistence, and the
control service. [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) covers
cross-compilation, the release build path, and cache overrides for sandboxed
environments. GPU backend selection on WSL2 is documented in
[docs/GPU.md](docs/GPU.md).

## Pull requests

- Keep formatting and Clippy clean; CI denies warnings.
- Add or update tests for behavior changes. The suite is deterministic and
  headless by design — please keep it that way.
- Write commit messages that explain the change, not the process.
- By contributing, you agree your contributions are licensed under the
  [MIT License](LICENSE).
