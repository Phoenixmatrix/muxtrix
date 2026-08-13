# Testing strategy

Muxtrix uses three headless layers. Together they cover serialized workspace
state, real PTYs and Ghostty parsing, application runtime ownership, actual OS
keyboard delivery, and a complete Iced/wgpu render frame.

## Deterministic suite

Run:

```sh
cargo test --workspace --all-targets
```

The Linux all-target suite covers:

| Area | Assertions |
| --- | --- |
| GPU bootstrap | WSL defaults, NVIDIA preference, EGL logging, preservation of the native window-system environment, and preservation of explicit overrides |
| Domain model | horizontal/vertical nested splits, focus, close invariants, last-pane protection, ratios, and JSON restoration |
| Launch planning | native profiles and explicit Windows-to-WSL command construction |
| Native PTY | spawn, resize, immediate post-resize input, input containing spaces, output streaming, exit, and cleanup |
| Ghostty VT | ANSI colors and attributes, theme defaults, direct-RGB preservation, OSC palette precedence across theme changes, cursor metadata, terminal query replies, split UTF-8 output, wide-cell ownership, latest-frame coalescing, dirty-row snapshot reuse, sanitized OSC window titles, native viewport scrollback and exact scrollbar metrics, immediate resized frames, stale-grid rejection, OSC 9/99/777 parsing across read boundaries, and error-free shutdown |
| Session isolation | two simultaneous live PTY/Ghostty actors receive and publish distinct markers |
| Local control | typed socket round-trip, pane split/close routing, dangling-option rejection, exact native/WSL endpoint propagation, WSL environment bridge, and real-app ping |
| Hook lifecycle | idempotent add, selective remove, re-add, executable and lifecycle-semantic drift repair, custom WSL bridge command, malformed-file safety, permission preservation, concurrent third-party edits, and solely-created-file cleanup |
| Application state | key encoding, Space/Ctrl+Space, navigation/control/Meta keys, pane-to-runtime ownership, focused routing, independent workspace lifecycle, rename-input keyboard ownership, active-workspace fleet projection, preserved tab-band ordering, flat agent filtering, fixed two-line fleet activity, local Git branch/directory context, semantic pane-signal colors, Native/WSL profile selection, registry-backed WSL filtering, distro-default shell launch, dynamic installed UI/monospace family and terminal-weight discovery, 16 persisted Ghostty-compatible theme presets, embedded icon validity, legacy font compatibility, point-based font metrics, fixed-column Unicode and wide-glyph projection, exhaustive U+2500..=U+257F semantic box-drawing coverage, grid resize coalescing with retained-frame continuity, style-run coalescing, scrollback-anchored cell selection, interactive scrollbar geometry, OSC pane/native-window titles, WSL Wayland resize-increment isolation, bounded split dragging, Zellij-style resize undo and neighbor stacking, Base/Vertical/Horizontal/Stacked/Half-stacked layout cycling, stacked-pane keyboard navigation, wheel-delta conversion, shell-exit detach/restart, close cleanup, command search and keyboard selection, pane-local Idle-to-Running agent recognition, screen-authoritative Codex/Claude attention, automatic-review suppression, visible-prompt ownership across late tool output, agent launches, settings persistence/validation, platform shortcut labels, sidebar collapse, dense pane-header consolidation, pane maximize/restore, pane context overflow, pane attention read state, state-aware agent activity, and rejection of unowned agent lifecycle events |

Unix-only process tests use short-lived `/bin/sh` children. Windows-specific
launch-plan tests remain platform-neutral, but native ConPTY execution requires
the Windows environment described below.

## Native Windows checks

A native Windows checkout has been validated with Rust 1.97.1 MSVC, Zig
0.15.2, and MSVC 19.44 x64. Run from a Visual Studio Developer PowerShell:

```powershell
cargo fmt --all --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p muxtrix --bin muxtrix
cargo build -p muxtrix-control --bin muxtrixctl
cargo run -p muxtrix --bin muxtrix-gpu-probe -- `
  --require-hardware `
  --require-adapter NVIDIA
```

The Windows suite runs the platform-applicable subset; Unix-only PTY integration
targets correctly contribute zero tests there. This gate covers ConPTY shell exit and
restart, named-pipe request round trips, split Unicode Ghostty input, both
binaries, and hardware adapter selection without opening the application UI.

## Real-application E2E

Run on Linux:

```sh
cargo test -p muxtrix --features e2e --test headless_e2e -- --test-threads=1
```

The outer Rust test starts a nested copy of itself with `xvfb-run`. The nested
test launches the production Muxtrix binary with feature-gated observability:

```text
Rust E2E test
  -> private Xvfb display (never WSLg)
  -> real Muxtrix process
  -> winit X11 window
  -> Iced/wgpu Vulkan renderer
  -> Mesa llvmpipe offscreen frame
```

The test uses X11's XTest extension to focus the actual mapped window, open the
command palette with Ctrl+P, open settings with Ctrl+Comma, and type
`echo alpha beta` plus Enter as OS keyboard events. This catches shortcut,
view-routing, and Space handling regressions that direct state tests cannot.
The running app then performs and asserts:

- the private local control endpoint responds to a typed ping;
- repeated real X11 window resizes from 1280x800 down to 820x560 and back leave
  the app and control service responsive;
- the command palette and settings page both render from their real shortcuts;
- Arrow Up/Down keyboard events visibly move command-palette selection;
- the external command and its spaces reached the initial shell;
- an actual X11 mouse-wheel event reached the terminal scrollback handler;
- an actual X11 scrollbar track click and thumb drag reached the absolute
  Ghostty viewport handler without changing terminal layout;
- the focused Ghostty cursor is visible;
- a horizontal split launches a second independent shell and reduces columns;
- a vertical nested split launches a third independent shell and reduces rows;
- pane-specific marker output never appears in another pane;
- closing the focused pane removes exactly its runtime and preserves survivors;
- typing `exit` detaches the ended PTY, restarts that pane, and leaves the app responsive;
- OSC 777 from a background shell creates unread pane activity in the fleet,
  without duplicating it into global Attention, and clears when focused;
- sidebar collapse/expand, pane maximize/restore, and pane overflow preserve all
  terminal runtimes and render through real application state;
- the domain pane tree remains valid; and
- Iced returns a populated 1280x800 RGBA screenshot with meaningful color
  diversity, proving that a complete wgpu frame rendered.

The default E2E ends at 1280x800. A compact responsive frame can be exercised
without changing the host session:

```sh
MUXTRIX_E2E_VIEWPORT=820x560 cargo test -p muxtrix \
  --features e2e --test headless_e2e -- --test-threads=1
```

For a final-frame capture of the real Settings surface, set
`MUXTRIX_E2E_CAPTURE=settings`; use `MUXTRIX_E2E_CAPTURE=palette` for the
keyboard-selected command palette, or `MUXTRIX_E2E_CAPTURE=stacked-layout`
for the real three-pane title-sheet stack. `terminal-glyphs` renders rounded
light and square heavy boxes with pixel-connectivity assertions, plus double,
dashed, diagonal, junction, horizontal-rule, and full-block fixtures. The
worktree flow exposes
`worktree-dialog`, `worktree-manager`, `worktree-switcher`, and
`worktree-restart-confirmation` captures. The GitHub flow exposes `github-panel`
for a ready pull request with a 74-file windowed list, `github-blocked`,
`github-merge-confirmation`, `github-no-pr`, `github-scrolled`, `github-auth`,
and `github-auth-collapsed` for its important readiness, confirmation, empty,
scrolling, and auth states. `github-loading` and `github-refreshing` cover the
same animated loading shell before data and while replacing cached review
state. The full-screen file review flow adds `github-diff`,
`github-diff-binary`, `github-diff-loading`, and `github-diff-error` for its
normal and hardened fallback states.
Combine any capture with
`MUXTRIX_E2E_SCREENSHOT_RGBA=/tmp/muxtrix-settings.rgba` when inspecting the GPU
output without exposing a host window.

The app writes a temporary JSON report and exits through `iced::exit`. Process
guards terminate the app and X server on failures. The E2E scenario is compiled
only with the `e2e` feature and activates only when the test supplies its report
path, so normal builds and launches have no test behavior.

## Renderer separation

The E2E test intentionally uses Vulkan/llvmpipe because Xvfb cannot provide the
WSLg EGL/D3D12 surface used in production. This validates window composition and
rendering without exposing a host window. Hardware selection remains covered by
the separate no-window probe:

```sh
cargo run --bin muxtrix-gpu-probe -- \
  --require-hardware \
  --require-adapter NVIDIA
```

That probe requires a WSL2 host with GPU passthrough exposing `/dev/dxg`, where
it selects the host GPU through Mesa D3D12. Environments that do not expose
`/dev/dxg` cannot satisfy the hardware requirement.
