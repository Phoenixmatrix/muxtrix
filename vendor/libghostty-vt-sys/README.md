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

This directory is based on the published `libghostty-vt-sys` 0.2.1 crate.
On MSVC, static builds link `ghostty-vt-static.lib` instead of the DLL import
library `ghostty-vt.lib`, following upstream commit
`bac73b914d936e945de4a6b93bed75ae1ce8895c`.

`patches/scrollback-lines.patch` corrects the pinned Ghostty C API's
`max_scrollback` implementation: its public header and Rust wrapper specify
lines, but it forwards that number to a byte budget. The patch enables a
row-based eviction threshold for C API terminals while preserving native
Ghostty's byte budgets. It uses the existing whole-page recycling and tracked
pin handling, retaining at least the requested number of history rows with a
small page-granularity excess. Width changes do not reduce the row budget;
zero and alternate-screen no-history behavior remain intact. Cloned page
lists preserve the limit.

The build applies the patch idempotently to fetched sources and explicit
`GHOSTTY_SOURCE_DIR` checkouts. An incompatible source override fails at patch
application. An external `pkg-config` library must provide equivalent line
semantics; Muxtrix uses the patched vendored build.

Muxtrix keeps the tested Ghostty source pin and Zig requirement. Remove the
local corrections when a compatible released crate includes them.
