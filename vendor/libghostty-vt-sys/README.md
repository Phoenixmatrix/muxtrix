# libghostty-vt-sys

Raw FFI bindings for libghostty-vt.

- Fetches and builds `libghostty-vt.a` from ghostty sources via Zig by default.
- Exposes checked-in generated bindings in `src/bindings.rs`.
- Static linking is the baseline rather than a Cargo feature. Enable the
  additive `link-dynamic` feature to link the shared library instead.
- Set `GHOSTTY_SOURCE_DIR` to force the build to use a local Ghostty checkout.
- Set `GHOSTTY_ZIG_SYSTEM_DIR` to force Zig package resolution through a
  pre-fetched `zig build --system` directory. This is intended for Nix and other
  sandboxed package managers that cannot fetch during build scripts.
- Set `LIBGHOSTTY_VT_SYS_OPTIMIZE` to `Debug`, `ReleaseSafe`, `ReleaseFast`, or
  `ReleaseSmall` to override the Zig optimize mode used by vendored builds.
- If the `pkg-config` feature is enabled, the build will use an installed
  `libghostty-vt` found through `pkg-config` only when `GHOSTTY_SOURCE_DIR` is
  unset. With the default static link mode, it probes Ghostty's
  `libghostty-vt-static` pkg-config module instead.
- libghostty-vt is pre-1.0, so these bindings do not guarantee compatibility
  with arbitrary installed C API revisions.

## Muxtrix vendoring note

This directory is the published `libghostty-vt-sys` 0.2.1 crate with one
intentional build-script correction. On MSVC, static builds link
`ghostty-vt-static.lib` instead of the DLL import library `ghostty-vt.lib`.
The correction comes from upstream commit
`bac73b914d936e945de4a6b93bed75ae1ce8895c`.

Muxtrix vendors the released crate instead of depending on that Git revision
because the revision also advances Ghostty's source pin and Zig requirement.
Remove this patch and the root `[patch.crates-io]` entry after a released crate
contains the MSVC correction while remaining compatible with Muxtrix's pinned
toolchain.
