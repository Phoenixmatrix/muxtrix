# Architecture

## Decision summary

Muxtrix uses GPUI, Zed's UI framework, for the application chrome and a
custom terminal element. GPUI renders through wgpu on Linux and through the
native Metal and DirectX backends on macOS and Windows; terminal cells are
drawn through the same GPU-backed renderer rather than embedded as a second
native window.

Muxtrix uses the safe Rust `libghostty-vt` crate instead of the broader
`libghostty` API. The VT crate is the appropriate boundary today because it is
available on Linux, Windows, and macOS and owns terminal parsing, screen state,
scrollback, input encoding, selection, and render-state extraction. Muxtrix
continues to own PTYs, fonts, glyph shaping/atlases, GPU drawing, panes, and
application lifecycle. This boundary can be revisited when full libghostty has
a stable cross-platform rendering API suitable for GPUI composition.

The published `libghostty-vt-sys` 0.2.1 source is vendored with the narrow
upstream MSVC static-archive link correction. Keeping the released source pin
avoids coupling that Windows fix to unrelated Ghostty and Zig upgrades; the
path patch should be removed when a compatible fixed release is available.

The current vertical slice projects Ghostty snapshots as fixed-height rows of
fixed-column shaped text runs rendered by GPUI. Snapshots retain foreground and
background colors, font/decorative attributes, cursor state, and each cell's
terminal width. Ghostty's dirty-row state lets snapshots share unchanged row
text and cell buffers. The run model combines adjacent ASCII cells with
identical style, isolates Unicode fallback glyphs whose natural metrics may
vary, and assigns wide glyphs their two terminal columns. As in Ghostty's
sprite-font resolver, U+2500..=U+257F bypasses configured-font lookup and maps
to semantic cell geometry; this lives in the run model because
`libghostty-vt` does not include Ghostty's font renderer. A pane-sized clipped
container draws the default background while runs draw non-default cell
backgrounds, selections, and the cursor. Standard VT window-title metadata
crosses the same snapshot boundary and updates only the owning pane. The next
renderer milestone will batch all terminal primitives into a dedicated widget
with a persistent glyph atlas. No CPU bitmap or nested native-window renderer
is planned.

Kitty graphics stay on the same boundary. `libghostty-vt` parses commands,
limits each terminal's decoded image storage to 64 MiB, decodes PNG payloads on
the terminal actor thread, and resolves placement geometry, source rectangles,
viewport positions, and z-layers. Snapshots share decoded RGBA buffers by
Ghostty generation stamp; the runtime retains one GPUI render image per visible
generation instead of copying or uploading pixels on every repaint. Separate
canvas passes place images below cell backgrounds, below glyphs, or above
glyphs, matching Ghostty's renderer order; a renderer-owned overlay pass keeps
selection and cursor backgrounds visible over below-text images. Placement
extents scale from Ghostty's integer cell metrics to the renderer's fractional grid,
and the pane clip contains off-screen geometry.

Terminal themes are applied on the Ghostty owner thread as default foreground,
background, cursor, and ANSI palette values. Ghostty retains direct RGB cells
and active OSC color overrides, so terminal programs remain authoritative over
colors they explicitly set. Applying a preset publishes a new snapshot without
restarting the PTY. Selection and cursor-text colors remain renderer-owned UI
values from the same preset. See `docs/TERMINAL_THEMES.md` for the precedence
contract.

The Ghostty handles are intentionally thread-confined. Each pane's terminal
actor owns its VT state and receives PTY output/input messages over channels.
Runtime handles are keyed by `PaneId`, so split, focus, event delivery, and
close stay isolated. The terminal element reports its bounds and resizes the corresponding PTY and VT grid whenever a
pane's terminal body changes dimensions. Pixel-only drag events are coalesced:
the actor receives a resize only when terminal rows or columns change, while
the latest viewport is still retained. When the grid changes, the element keeps the
last valid snapshot visible and clipped to the current pane while rejecting
queued frames whose rows or columns do not match the requested grid. Resize and
input commands remain ordered on the terminal actor, so input cannot render at
stale pre-resize coordinates. Terminal actors coalesce consecutive PTY
chunks before snapshotting and keep a single latest-frame slot while preserving
notifications and exit/error events. A bounded wake channel wakes the runtime
when terminal or control work arrives, and every frame drains the terminals
before drawing, so a frame never paints a grid older than the bytes it has.
Snapshots cross into the application's update loop; raw Ghostty handles do
not.

Pane trees contain leaves, recursive splits, and stacks. A stack keeps its live
pane IDs in visual order while the view expands only the focused pane and projects
the others as title-height sheets; no PTY ownership moves. Recursive split
nodes are addressed by workspace, tab, and tree path. Each split records its
own painted extent, and global pointer movement updates only the active node's
bounded `SplitRatio`. Keyboard growth snapshots the focused pane's tree before
adjusting a split or folding blocked siblings into a stack, which makes
decrease an exact undo chain. Terminal mouse-wheel deltas remain pane-scoped and
are applied on the Ghostty owner thread through `scroll_viewport`. Snapshot
metadata carries Ghostty's exact scrollbar total, visible length, and offset;
the element draws a hover-only thumb as an overlay so its appearance never changes PTY
columns. Selection is pane-local UI state mapped from pointer coordinates to
the visible cell grid and projected into the fixed-column style runs.

## Component boundaries

```text
GPUI application + compositor
  |-- workspace/sidebar model
  |-- recursive split tree
  |-- terminal canvas -> glyph atlas + cell decorations
  |-- notification and activity model
  `-- command dispatcher / local IPC API
             |
terminal session actor (one per live terminal)
  |-- libghostty-vt terminal state
  |-- PTY reader/writer
  `-- render snapshot producer
             |
process host
  |-- Unix PTY (Linux/macOS)
  |-- ConPTY (Windows native)
  |-- wsl.exe + ConPTY (Windows-to-WSL2)
  `-- later: SSH transport
```

The implemented actor uses one blocking PTY reader thread and one owning
session thread. Only byte buffers cross from the reader. The session thread
feeds Ghostty, writes any terminal-generated replies to the PTY, creates render
snapshots, and publishes those snapshots to the application loop. This preserves
ordering and keeps every Ghostty handle on the thread that created it.

## GPU strategy

GPUI renders through wgpu on Linux, which maps to Vulkan or OpenGL, and through
Direct3D on Windows and Metal on macOS. WSLg exposes the host GPU through `/dev/dxg`. Muxtrix
defaults WSL to wgpu GL plus Mesa Gallium D3D12; when the NVIDIA WSL driver is
present it also prefers the NVIDIA adapter. These defaults are process-local
and only fill missing environment variables, so users can override every
choice. The bootstrap deliberately preserves the native window-system
environment because the accelerated GL/D3D12 renderer requires WSLg's Wayland
surface. No separate X server or global WSL modification
is required. See `docs/GPU.md` for the exact bootstrap and headless probe.

Terminal rendering will batch cells by texture atlas and decoration style. The
CPU performs VT parsing and glyph shaping; glyph rasterization is cached in GPU
textures and wgpu draws backgrounds, glyphs, selections, cursors, and pane
effects. This keeps terminal rendering inside GPUI's compositor and avoids
cross-window synchronization.

## Process backends

A process-host trait separates terminal state from where commands execute:

- `Local`: Unix PTY on Linux/macOS or ConPTY on Windows.
- `Wsl`: Windows-only host that starts `wsl.exe` with an optional distribution,
  working directory, and optional explicit command. When no program is set,
  `wsl.exe` starts the distribution's configured default shell as an explicit
  login shell, under its default user in `~`. This avoids an embedded ConPTY
  launch falling back to Bash when the account uses another shell. The app
  itself stays a native Windows GUI.
- `Ssh`: later milestone; transport remains outside Ghostty VT.

The WSL host must translate Windows/UNC paths explicitly and must not pretend a
native Windows process is a Linux process. Profiles persist the backend choice.
The Windows host shares pane identity and an optional control endpoint with WSL
through `WSLENV`. Linux-side hooks can invoke `muxtrixctl.exe` via standard WSL
interop, keeping control on the Windows named pipe without opening TCP access.
For user-scope integration, the Windows app resolves the selected distro name,
Linux home, and WSL-visible executable path, then uses the same reversible hook
manager against the distro's configuration through `\\wsl.localhost`.
Installed distribution names come directly from the current user's Lxss
registry entries, matching Windows Terminal's dynamic-profile strategy and
filtering utility distributions without spawning a console process. Linux home
and executable-path resolution cannot be inferred from that registry data, so
Muxtrix performs one combined hidden WSL query on the integration worker and
caches the resulting hook context per selected distribution.

## Persistence and programmability

The serializable domain model is independent of the UI framework. A workspace owns
one or more ordered tabs; each tab owns a split tree whose leaves are panes, and
each pane owns terminal, browser, or document surfaces. Runtime handles remain
keyed by globally unique pane identity and are never serialized. Schema v2
migrates the former workspace-owned tree into a default `Tab 1`. State is
written using an atomic replace and versioned schema.

A session-stable local IPC endpoint exposes the same typed commands used by
keyboard actions. Each attached GUI owns a different Unix socket or Windows
named pipe and publishes its pane IDs in a private per-user route registry.
Pane identity selects the correct window; commands without pane context are
accepted only when one window is active. Newline-delimited JSON remains an
internal protocol, and remote transport requires authentication and version
negotiation first.

Agent hook configuration is an adapter boundary rather than application state.
Each installed handler has a Muxtrix marker, and uninstall selectively removes
only those handlers. A pre-change backup is retained for recovery while the
integration is active, but normal uninstall never restores the whole file and
therefore cannot clobber later edits from a person or another tool.
For Codex and Claude Code, those hooks own session identity and coarse turn
boundaries, not live interactive state. Fresh Ghostty frames pass through a
conservative agent-screen classifier; only positive visible prompt evidence may
author `Waiting`. Claude also polls its machine-readable interactive-session
status off the UI thread and associates records to panes only by unique session
ID, process PID, or cwd evidence. Structured `busy` corrects an idle-looking
composer but cannot override a visible blocker. This prevents a pre-review
permission hook or unrelated parallel tool completion from owning human
attention without making optional TUI chrome a single point of failure. See
[Agent state detection](AGENT_STATE_DETECTION.md) for the decision record.
