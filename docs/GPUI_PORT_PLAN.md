# Muxtrix GPUI port — implementation plan

Status: plan, not started. Branch `feat/gpui-port`, worktree `~/.muxtrix/worktrees/gpui-port`.

This document is written for an agent that will execute the port without further
context. It records what exists today (with file:line anchors into this tree),
the target architecture, every decision already made, the order of work, and the
acceptance criteria for each phase. Read it fully before touching code.

---

## 0. Goals and non-negotiables

1. **Replace Iced 0.14 with GPUI** (Zed's UI framework) plus **gpui-component**
   (longbridge) for stock widgets. Rust stays the only application language.
2. **Keep libghostty-vt and the whole `muxtrix-terminal` crate untouched.**
   `libghostty-vt` is a VT state machine only; it never rendered a pixel. All
   rendering is host-side, and that is what gets rewritten.
3. **Layout stays the same.** Every screen keeps its current structure, sizes and
   behaviour (rail 272/46 px, app bar 43 px, GitHub panel 372 px, pane header
   chrome, stacked-pane sheets, dialogs, palette, settings pages). Visual
   *style* (colors, radii, type, motion, control polish) may improve.
4. **Every feature keeps working** — defined as: every unit test in the workspace
   passes, all 23 named checks in `crates/muxtrix-app/tests/headless_e2e.rs`
   pass, and every case in `scripts/capture-gallery/matrix.mjs` (~114 cases,
   84 capture states) still captures.
5. Platforms: Linux (X11 + Wayland, including WSLg), Windows, macOS. Hardware
   acceleration on all three.
6. Workspace lints stay: `unsafe_code = "deny"`, clippy `unwrap_used`/`todo`/
   `dbg_macro` denied, `-D warnings` in CI.
7. **Nothing goes to third-party repositories. Ever.** ("Upstream" in this
   document always means a third-party project such as Zed, gpui-component
   or wgpu — never this repo's `main`.) Do NOT open pull requests, issues,
   discussions, or comments on `zed-industries/zed`, `longbridge/gpui-component`,
   `gfx-rs/wgpu`, or any other third-party repository. Do not push to any
   remote other than `Phoenixmatrix/muxtrix`. If a change to a dependency is
   required, it lives as a vendored copy or a `[patch]` entry pointing at a
   **local path or a branch in a repository owned by this project's user**
   that the user creates themselves — the agent never creates forks or
   branches on other organisations' repositories. Any proposal to contribute
   upstream is written down in the PR description for the user to act on;
   the agent does not act on it.

Constraints inherited from `AGENTS.md`: never open a visible GUI during
automated checks; Windows subprocesses must go through
`crates/muxtrix-app/src/process.rs::console_command`; each completed task ends
in a pull request.

---

## 1. What exists today (inventory)

### 1.1 Crate map

| Crate | Iced coupling | Port action |
|---|---|---|
| `muxtrix-app` (`crates/muxtrix-app`) | All of it (`main.rs` 28 174 lines, `e2e.rs`, `settings.rs` fonts, `gpu.rs`, `doctor.rs`, `popover.rs`, `ellipsized_text.rs`, `box_drawing.rs`, `terminal_image.rs`) | Rewrite view/runtime layers |
| `muxtrix-terminal` | none | untouched |
| `muxtrix-domain`, `-control`, `-platform`, `-sessions` | none | untouched |
| `vendor/libghostty-vt-sys` | none | untouched |

Pure-logic modules in `muxtrix-app` with **zero** Iced references, reused as-is:
`agent_screen.rs`, `agents_roster.rs`, `commands.rs`, `github.rs`, `metrics.rs`,
`themes.rs`, `process.rs`.

### 1.2 `main.rs` structure (line numbers as of commit `e7e5516`)

- Bootstrap: `main()` L96–134 (`iced::application(boot, update, view)`), window
  settings L136–150 (1280×800, min 720×480, Linux app id `muxtrix`), icon
  L177–184 (embedded RGBA).
- State: `struct Muxtrix` L186–326; `ActiveView { Workspace, Settings,
  ThemeGallery, GitHubDiff }` L502; `SettingsPage { Preferences, Worktrees }`
  L513.
- `enum Message` L1168–1342, **146 variants**. Variants carrying Iced types:
  `Keyboard(keyboard::Event)` L1183, `ResizePane(PaneId, Size)` L1184,
  `ResizeSplit(SplitKey, Size)` L1185, `PointerMoved(Point)` L1187,
  `TerminalPointerMoved(PaneId, Point)` L1192, `TerminalScrollbarMoved(PaneId,
  Point)` L1193, `ScrollTerminal(PaneId, ScrollDelta)` L1202,
  `ScrollHoveredTerminal(ScrollDelta)` L1203, `WindowOpened(window::Id, Size)`
  L1204, `WindowResized(Size)` L1205, `E2eScreenshot(window::Screenshot)`
  L1338.
- `DesignTokens` L1425–1496: 17 `Color` fields (`app, rail, panel,
  panel_raised, overlay, line, line_strong, scrim, text, muted, faint, accent,
  success, warning, danger, github_open, github_merged`), light + dark ramps.
- `theme()` L2083 (Iced `Theme::Light`/`TokyoNight`, vestigial — every surface
  uses `DesignTokens`).
- `subscription()` L2090–2127: terminal wake stream over `async_channel`
  (`EventSubscription` L1084, stream fn L18803) → `PollTerminal`; 500 ms
  `BlinkCursor`; `event::listen_with(app_event L18809)` → `Keyboard`,
  `PointerMoved`, `EndPointerInteraction`, `ScrollHoveredTerminal`,
  `WindowOpened/Resized/FocusChanged`; 90 ms `AnimateGitHubLoading` (gated);
  1 ms gated `RefreshGitHubPullRequestsAfterAgentTurn` poll; 50 ms `E2eTick`.
- `update()` L2131 → ~L8170. This is the business logic and stays.
- `view()` L8179 and view methods L8350–L14600 (see §1.3).
- Terminal grid renderer `styled_terminal` L18874–19103 and the run model
  L19111–19350 (`TerminalRunStyle`, `TerminalRunKind`, `terminal_row_style_runs`
  L19226, `push_terminal_run`, `bold_size_scale`, underline decoration).
- Pane-tree geometry helpers L15387–15816 (pure, reusable).
- Icons: `IconKind` L16619 (24 variants) and `icon()` L16647 mapping to
  `assets/icons/*.svg`, tinted at render time.
- Widget ids L1400–1416: `PALETTE_INPUT_ID, SETTINGS_SCROLL_ID,
  PALETTE_SCROLL_ID, GITHUB_FILE_SCROLL_ID, GITHUB_PULL_REQUEST_SCROLL_ID,
  GITHUB_PULL_REQUEST_QUERY_ID, WORKSPACE_CREATE_INPUT_ID, RENAME_INPUT_ID,
  WORKTREE_INPUT_ID`, plus literal `"muxtrix-github-diff"`.
- Programmatic focus (`operation::focus`) L3835, 3927, 4099, 5585–5623, 5786,
  5807. Programmatic scroll (`snap_to`) L3577–3580, 5787–5789, 15117.
- Clipboard L2319, 2408, 2420, 2425, 5214, 5218, 6386, 6397.
- `window::set_resize_increments` L3611 (WSL + Wayland only, L19366).
- Unit tests: L20700 → end of file, constructed against `Muxtrix` + `update`.

### 1.3 View surface to reproduce

Workspace shell: `view` 8179 · `sidebar` 11136 · `workspace_row` 11388 ·
`fleet_workspace_group` 11463 · `fleet_header` 11481 · `collapsed_sidebar`
11607 · `global_alert_row` 11818 · `fleet_row` 11855 · `pane_pip` 12137 ·
`github_status_button` 11050 · `workspace_view` 12494 · `commands_pill` 12591 ·
`app_bar_tabs` 12651 · `view_tree` 14219 · `view_pane_stack` 14309 ·
`view_stacked_pane_header` 14327 · `view_pane` 14413 · `terminal_scrollbar`
18728 · `pane_icon_button` 17192 · pane menu 17221–17294.

Dialogs: `workspace_create_dialog` 8350 · `default_agent_dialog` 8395 ·
`rename_dialog` 8461 · `worktree_dialog` 8513 · `session_picker_dialog` 8822 ·
`worktree_restart_dialog` 9016 · `close_workspace_dialog` 9298 ·
`command_palette` 14027.

GitHub: `github_panel_view` 9387 · `github_panel_loading_state` 9671 ·
`github_local_view` 9847 · `github_pull_requests_view` 9900 ·
`github_pull_request_list` 10004 (virtualised, 58 px rows) · `github_file_list`
10630 (virtualised, 42 px rows) · `github_diff_view` 10750 ·
`github_diff_document_view` 10892 · `github_diff_line` 10975.

Settings: `settings_view` 12796 · `preferences_settings_view` 13325 (sections
at 13360/13507/13543/13622/13634/13679/13792) · `agent_hook_row` 13875 ·
`worktree_settings_view` 12872 · `worktree_settings_row` 13081 ·
`terminal_theme_preview` 17864 · `settings_row/section` 18056/18089 ·
`theme_gallery_view` 8682.

Shared leaves: `status_pill` 16685 · `selection_bar` 16709 · `rail_marker`
16733 · `signal_dot` 16751 · `roster_ring` 16772 · `section_label` 16791 ·
`app_tooltip` 17174 · `ruled_surface` 17138.

Iced widget usage counts: `button` 96, `scrollable` 24, `stack` 22,
`text_input` 20, `pick_list` 18 (9 are font pickers capped at
`FONT_FAMILY_MENU_MAX_HEIGHT` 320), `mouse_area` 18, `opaque` 16, `slider` 6,
`toggler` 4, `rich_text` 4, `tooltip` 2, `svg` 2, `canvas` 2, `sensor` 2.

### 1.4 Custom widgets (behaviour that must survive)

- `popover.rs` — overlay-layer pane menu, right-aligned to anchor −6 px, 38 px
  below anchor top, clamped to viewport; **any unhandled press anywhere
  dismisses and is consumed** (e2e check `pane_menu_click_away_observed`).
- `ellipsized_text.rs` — single-line, shaped-width binary-search ellipsis,
  trailing `…` always inside bounds, full text exposed to a11y/inspection.
- `box_drawing.rs` — U+2500–257F drawn as geometry (ported from Ghostty's
  `box.zig`); arms end on cell edges; adjacent cells share one geometry run.
  e2e asserts `magenta_rounded_box_continuity` and
  `cyan_heavy_box_continuity` as single connected components.
- `terminal_image.rs` — Kitty/Sixel placements in three z-planes
  (`BelowBackground`, `BelowText`, `AboveText`), source-crop-by-clip,
  fractional-cell scaling (`scaled_destination`), linear filtering, handles keyed
  by `image.generation`.

### 1.5 Fonts and metrics

- `settings.rs:6` is the only Iced import there: `FontWeight::iced()` L157,
  `bold_variant` L171, `UiFont::iced()` L241, `TerminalFont::iced()` L420,
  `weight_from_numeric/weight_numeric` L442–469, `AppSettings::ui_font()` L746,
  `font_with_style` L847, and `intern_font_family` L559 (leaks `&'static str`
  because `iced::Font::with_name` demands it).
- Cell metrics come from `metrics.rs` (fontdb + ttf-parser, **no Iced**):
  `terminal_cell_width = font_px * advance_ratio`, `terminal_cell_height =
  font_px * line_height`, `terminal_font_pixels = pt * 96/72` (1.0 on macOS).
  `metrics::system_monospace_family()` exists only because Iced's
  `Font::MONOSPACE` could differ from fontconfig's choice.
- `metrics::glyph_fallback` selects substitute faces/scales for non-ASCII runs.

### 1.6 GPU bootstrap

`gpu.rs`: on WSL, re-execs the binary with `WGPU_BACKEND=gl`,
`GALLIUM_DRIVER=d3d12`, `MESA_D3D12_DEFAULT_ADAPTER_NAME=NVIDIA` (if
`/usr/lib/wsl/lib/nvidia-smi` exists), `EGL_LOG_LEVEL=fatal`, because Vulkan
under WSLg (Mesa `dzn`) is unreliable. `probe_adapters()` and
`doctor.rs:176–240` report adapters through `iced::wgpu`.
`src/bin/muxtrix-gpu-probe.rs` is a standalone probe.

### 1.7 e2e harness

- `src/e2e.rs` (feature `e2e`): `Scenario::from_environment` reads
  `MUXTRIX_E2E_*`; `drive_e2e` runs every 50 ms tick; `TickAction`s are
  `Wait`, `ScrollSettingsToEnd/ToTerminal/ToGitHub`, `ScrollGitHubToEnd`,
  `ScrollGitHubPullRequestsToEnd`, `Capture`; `Capture` calls
  `iced::window::screenshot`, pixel-asserts in-process
  (`light_horizontal_continuity`, `magenta_rounded_box_continuity`,
  `cyan_heavy_box_continuity`), writes the RGBA dump + JSON report, then
  `iced::exit()`. Injects `Message::Keyboard(arrow_down())` L39.
- `tests/headless_e2e.rs` (Linux only): re-execs itself under `xvfb-run`,
  launches the real binary with `WINIT_UNIX_BACKEND=x11`,
  `WGPU_BACKEND=vulkan`, `GALLIUM_DRIVER=llvmpipe`, drives it with x11rb XTEST
  (keys, chords, typing, clicks, wheel, scrollbar drag), pings the control
  socket, stress-resizes, waits ≤25 s for self-exit, asserts 23 checks.
- `scripts/capture-gallery/` runs the compiled test binary per matrix case and
  converts the RGBA dump to PNG.

### 1.8 Build/CI

`rust-toolchain.toml` is `channel = "stable"`. CI (`.github/workflows/build.yml`)
installs X/GL/Vulkan/Wayland dev headers + `mesa-vulkan-drivers` + `xvfb`, runs
fmt/test/clippy, then the e2e test. Linux release uses `cargo zigbuild
--target x86_64-unknown-linux-gnu.2.34`. Windows icon via `winresource`.

---

## 2. Target stack (decisions made)

### 2.1 Dependencies

GPUI on crates.io (`gpui 0.2.2`, Oct 2025) **predates** the Linux wgpu renderer
(merged 2026‑02‑13, zed PR #46758). We need wgpu on Linux (GL fallback for
WSLg), so use git dependencies pinned to a rev, and pin gpui-component to a rev
whose own `Cargo.lock` points at the same zed rev.

```toml
# Cargo.toml (workspace)
[workspace.dependencies]
gpui           = { git = "https://github.com/zed-industries/zed", rev = "<ZED_REV>" }
gpui_platform  = { git = "https://github.com/zed-industries/zed", rev = "<ZED_REV>", features = ["font-kit", "x11", "wayland"] }
gpui-component = { git = "https://github.com/longbridge/gpui-component", rev = "<GC_REV>" }
wgpu           = "29"          # for gpu.rs / doctor probe only; match zed's pin
```

Starting pins (verify they still build together at execution time; if not,
take gpui-component `main` and read `<ZED_REV>` out of its `Cargo.lock` entry
`name = "gpui"`):

- `<ZED_REV>` = `e0931d5a9dbf4f781b336fdf448739e74a2ac0b5` (what gpui-component
  `5a5e2ab`, 2026‑08‑22, locks to).
- `<GC_REV>` = `5a5e2ab`.

Zed's toolchain is `1.97.1`; set `rust-toolchain.toml` to a pinned stable ≥ that
(`channel = "1.97.1"` is the safe choice; bump in lock-step with the zed rev).

Remove `iced` from `[workspace.dependencies]` only in the final phase; until then
both stacks coexist behind a cargo feature (§4, Phase 2).

Linux build deps to add in CI and `docs/DEVELOPMENT.md`: `libfontconfig-dev`,
`libfreetype-dev`, `libxkbcommon-dev`, `libxkbcommon-x11-dev`, `libwayland-dev`,
`libvulkan1`/`libvulkan-dev`, `libgl1-mesa-dev`, `libzstd-dev`. Keep
`mesa-vulkan-drivers` + `xvfb` for e2e. Verify `cargo zigbuild ...gnu.2.34`
still links font-kit/fontconfig; if it does not, fall back to building the
Linux release in an Ubuntu 22.04 container (glibc 2.35) — decide in Phase 7.

### 2.2 Rendering backends (what GPUI gives us)

| Platform | Backend | Notes |
|---|---|---|
| macOS | Metal (native) | nothing to do |
| Windows | DirectX 11/12 (native, `gpui_windows`) | nothing to do |
| Linux | wgpu, `Backends::VULKAN | GL` (`gpui_wgpu/src/wgpu_context.rs`) | adapter chosen by scoring: device type (discrete > integrated > other > virtual > cpu), then backend (Vulkan > GL). Env override is `ZED_DEVICE_ID=<vendor:device>` only — **there is no env var to force GL**. |

Consequence for WSLg: GPUI will prefer the `dzn` Vulkan adapter over the d3d12
GL adapter, which is exactly what `gpu.rs` works around today. This is the
single biggest platform risk; Phase 1 exists to measure it.

### 2.3 Application architecture

Keep the Elm core; replace the shell. Concretely:

- **`Muxtrix` + `Message` + `update()` stay** (they hold ~8 000 lines of logic
  and all unit tests). `Muxtrix` becomes the single GPUI root entity
  (`Entity<Muxtrix>`), rendered by `impl Render for Muxtrix`.
- **`Task<Message>` is replaced by an `Effect` enum** returned from `update`
  (`Vec<Effect>`). An effect runner in `runtime.rs` executes them with GPUI
  (`cx.spawn`, `cx.background_spawn`, clipboard, focus, scroll, quit). This
  keeps `update()` testable without a GPUI `App` and is the only structural
  change to the core. Effects needed (derived from every `Task` use today):
  `Spawn(Box<dyn FnOnce() -> Message + Send>)` (blocking work on the
  background executor), `SpawnStream(...)` for the daemon/control channels,
  `Focus(FocusTarget)`, `ScrollTo(ScrollTarget, ScrollPosition)`,
  `ClipboardWrite(String)`, `ClipboardRead(fn(Option<String>) -> Message)`,
  `SetWindowTitle`, `Exit`, `E2eCapture`.
- **Geometry types in messages** (`Point`, `Size`, `ScrollDelta`) become small
  app-owned structs in `muxtrix_app::geom` (f32 fields), converted at the GPUI
  boundary. Unit tests keep constructing them without a GPUI context.
- **Keyboard**: `Message::Keyboard(iced::keyboard::Event)` becomes
  `Message::Keyboard(KeyInput)` where `KeyInput { key: Key, modifiers:
  Modifiers, text: Option<String>, repeat: bool }` is app-owned. One translator
  `input::from_keystroke(&gpui::Keystroke) -> KeyInput` and one
  `input::to_terminal_bytes(&KeyInput, mode) -> Vec<u8>` (port the existing
  encoding logic from `update`'s `Keyboard` arm).
- **Views are plain functions/`impl IntoElement` in per-screen modules**, taking
  `&Muxtrix` and a `Dispatch` handle (`cx.listener`-style closure producing
  `Message`), mirroring today's `view_*` split.
- **Theming**: `DesignTokens` stays the single source of truth and is mapped
  into gpui-component's `Theme` (`ThemeColor` overrides: background, foreground,
  muted, accent, border, danger, etc.) so stock widgets pick the same palette.
  Appearance switch re-applies the theme (`Theme::global_mut(cx)` +
  `Theme::sync_...` per gpui-component docs).
- **Stock widgets from gpui-component**: `Button`, `InputState` + `TextInput`,
  `Select` (→ `pick_list`), `Slider`, `Switch` (→ `toggler`), `Checkbox`,
  `Tooltip`, `Scrollbar`, `Dialog`/modal, `PopupMenu`/`Popover`, `Sidebar`,
  `Tab`, `VirtualList`/`uniform_list`. Custom GPUI elements only for: terminal
  grid, pane split tree + divider drag, tab strip drag-reorder, ellipsized text
  (GPUI `StyledText` with `text_ellipsis`/`truncate` — verify it measures shaped
  width; if not, port `ellipsize_to_width` onto
  `window.text_system().shape_line`).
- **Icons**: implement `gpui::AssetSource` over `include_bytes!` for
  `assets/icons/*.svg`; render with `svg().path("icons/add.svg").text_color(...)`
  (GPUI tints SVGs by `text_color`).
- **Fonts**: `gpui::Font { family: SharedString, weight: FontWeight, style,
  features, fallbacks }`. No `&'static str` requirement → delete
  `intern_font_family` and the leak. Keep `InstalledFontCatalog` (fontdb) for
  the pickers. **Cell width is measured through GPUI**
  (`text_system.advance(font_id, size, 'M')`, as Zed's terminal does) so the grid
  and the renderer can never disagree; keep `metrics::advance_ratio` only as the
  headless/no-window fallback (PTY sizing before the window exists, unit tests).
  `metrics::glyph_fallback` is retained for the first port (GPUI shapes with
  fallback fonts itself; remove our fallback scaling only if the e2e glyph
  checks still pass without it — measure, don't assume).

### 2.4 Terminal element (the heart of the port)

`terminal_element.rs`: `impl Element for TerminalElement` modelled on Zed's
`crates/terminal_view/src/terminal_element.rs` (Apache‑2.0; port the
structure, not the code).

- `request_layout`: fixed size from the pane bounds.
- `prepaint`: compute `cell_width` (advance of `M`), `line_height` from settings,
  rows/cols from bounds minus 8 px `TERMINAL_PADDING`; if they differ from the
  runtime's size emit `Message::ResizePane` (this replaces the Iced `sensor`).
  Convert `GridSnapshot` through the existing `terminal_row_style_runs` into
  per-row `LayoutRun { origin, shaped: ShapedLine, style }` using
  `window.text_system().shape_line(text, font_size, &[TextRun { len, font,
  color, background_color: None, underline, strikethrough }], Some(cell_width))`
  — passing `cell_width` forces monospace advance exactly like Zed.
  Register the element's `FocusHandle` and an `EntityInputHandler` (IME,
  composition, `replace_text_in_range` → `Message::TerminalInput`).
- `paint` order (matches today's six-layer `stack`): images `BelowBackground`
  (`window.paint_image`) → background quads per run (`paint_quad(fill(..))`)
  → images `BelowText` → overlay quads (selection, cursor block) → text
  (`shaped_line.paint(origin, line_height, window, cx)`), with box-drawing runs
  painted via `window.paint_path` (`PathBuilder`) using the ported
  `box_drawing` geometry and full-block `█` as quads → dotted/solid link
  underlines as quads → images `AboveText` → cursor outline when unfocused →
  hovered-link cursor style (`window.set_cursor_style(CursorStyle::PointingHand)`).
  Use `window.paint_layer(bounds, ..)` + `with_content_mask` for clipping
  (replaces `container.clip(true)`).
- Mouse: `window.on_mouse_event::<MouseDownEvent/MouseUpEvent/MouseMoveEvent/
  ScrollWheelEvent>` inside `paint`, hit-tested against `bounds`, producing the
  same messages as today (`TerminalMousePressed/Released/PointerMoved/
  ScrollTerminal`, `EnterTerminal/LeaveTerminal`). Wheel: `ScrollDelta::Lines`
  vs `Pixels` map to the existing `ScrollDelta` handling (L7043 quantisation).
- Scrollbar: separate overlay element 12 px wide at the right edge, 3 px thumb,
  shown while hovered and `scrollbar.is_scrollable()`; drag → `TerminalScrollbarMoved`.
- Box drawing: `box_drawing.rs` keeps its `Glyph`/`Lines`/`BoxMetrics` model
  verbatim; replace the `canvas::Frame` drawing functions with a small
  `BoxPainter` trait implemented on `gpui::PathBuilder` (`move_to`, `line_to`,
  `arc`/`cubic_bezier_to` for rounded corners, `fill`/`stroke` →
  `PathBuilder::stroke(path, StrokeOptions)`). Keep every coordinate rule.
- Images: cache `gpui::RenderImage` (`Arc<RenderImage>` from RGBA frames)
  keyed by `image.generation` in `TerminalRuntime`, replacing
  `BTreeMap<u64, iced Handle>`. Port `scaled_destination` and
  `placement_geometry` unchanged; crop via `with_content_mask`.

Performance target: one `shape_line` per row per frame with GPUI's shape cache
(`shape_line_by_hash` / `LineLayoutCache`) and zero per-cell allocation; a
200×60 grid must paint in < 1 ms on the software adapter used in CI.

### 2.5 Input and keybindings

- Global shortcuts (Ctrl+P palette, Ctrl+, settings, Ctrl+Shift+C/V, number
  keys, Tab/Escape, pane navigation) → GPUI `actions!` + `KeyBinding`s bound on
  the root with key-context `"Muxtrix"`, and `"Terminal"` context on the
  terminal element so bindings can be precise about what the terminal swallows.
  Rule to preserve: **the terminal receives every key the app does not
  explicitly bind**, including Tab/Escape when a terminal is focused (mirrors
  `app_event` L18809).
- Text inputs are gpui-component `InputState`s stored in `Muxtrix` (one per
  today's widget id: palette query, workspace create, rename, worktree name,
  GitHub PR query, per-agent hook commands). `Focus(FocusTarget)` effect calls
  `state.focus(window, cx)`.
- Pointer tracking that today is global (`PointerMoved`, `EndPointerInteraction`
  for split/tab drags) → `window.on_mouse_event` registered by the root
  element during paint (capture phase).

### 2.6 Windows, clipboard, lifecycle

- `WindowOptions { window_bounds: Some(Windowed(1280×800)), window_min_size:
  Some(720×480), titlebar: Some(TitlebarOptions { title: "Muxtrix" }),
  app_id: Some("muxtrix"), focus: true, kind: Normal }`.
- Window icon: Linux via `assets/muxtrix.desktop` + hicolor icon (already
  packaged); Windows via the existing `winresource` step; macOS via bundle
  (`packaging/`, unchanged). GPUI has no runtime icon API — drop
  `muxtrix_window_icon()`.
- Title: `window.set_window_title(...)` from the effect runner whenever
  `Muxtrix::title()` changes.
- Resize/focus: `cx.observe_window_bounds` → `WindowResized`;
  `cx.observe_window_activation` → `WindowFocusChanged`;
  `window.on_window_should_close` → existing close path; `cx.quit()` for `Exit`.
- `set_resize_increments` (WSL + Wayland only) has no GPUI API. **Accepted
  loss**; note it in `docs/GPU.md`. If users miss it, note the gap for the user
  to raise upstream themselves (see goal 7 — the agent never contacts upstream).
- Clipboard: `cx.write_to_clipboard(ClipboardItem::new_string(s))`,
  `cx.read_from_clipboard().and_then(|i| i.text())`.
- Timers: `cx.spawn(async move |this, cx| loop { cx.background_executor()
  .timer(d).await; this.update(cx, |m, cx| m.dispatch(Message::BlinkCursor,
  cx))?; })` for the 500 ms blink, the 90 ms GitHub loader (gated by the same
  conditions as today), and the 50 ms e2e tick. Agent-turn pull-request
  refreshes are consumed in the same `PollTerminal` pass that receives the
  control event, so there is no idle deferral timer. Terminal wakeups: the
  `async_channel::Receiver<()>` is awaited in a spawned loop → `PollTerminal`.
- Daemon/control/agent background work: every `Task::perform(async fn)` and
  `Task::run(stream)` in `update` becomes `Effect::Spawn`/`SpawnStream`
  executed with `cx.background_spawn` and posted back through
  `this.update(cx, ..)`.

### 2.7 e2e and screenshots

GPUI's `Window::render_to_image` only exists on the *test* platform, so the
in-app `iced::window::screenshot` path cannot be kept. New design — simpler and
framework-agnostic:

1. `e2e.rs` keeps `Scenario`, `TickAction`s and all state assertions. The
   scroll actions call the `ScrollTo` effect on the new scroll handles. The
   `Capture` action no longer screenshots; it writes the JSON report, marks the
   control server as `capture_ready`, and keeps the window open.
2. `tests/headless_e2e.rs` polls the control socket for a new
   `{"method":"e2e_status"}` → `{"capture_ready":true}`, then grabs the frame
   with x11rb `GetImage` on the app window (it already has the window id from
   `wait_for_app_window`), writes `MUXTRIX_E2E_SCREENSHOT_RGBA`, runs the three
   pixel-continuity checks (move `light_horizontal_continuity`,
   `magenta_rounded_box_continuity`, `cyan_heavy_box_continuity` and
   `colored_box_continuity` from `e2e.rs` into a shared `e2e_pixels.rs`
   module compiled into both), then sends `{"method":"quit"}` (new control
   method → `Effect::Exit`).
3. `scripts/capture-gallery/capture-one.sh` is unchanged in interface (it runs
   the test binary, reads the RGBA file) — verify paths only.
4. Add `wait_for_first_frame`: GPUI on X11 + llvmpipe may map the window before
   the first present; poll `GetImage` until the background pixel equals the
   theme background.

Unit-test layer: add `gpui::TestAppContext` tests (`#[gpui::test]`) for the
terminal element (layout → rows/cols, `painted_quads()` for background runs)
and for key translation. These run headless with no X server.

---

## 3. Target file layout (`crates/muxtrix-app/src`)

```
main.rs                 CLI dispatch (--sessiond, doctor, --version), gpu bootstrap, run()
app.rs                  struct Muxtrix, Message, Effect, update() + pure helpers (moved verbatim)
runtime.rs              GPUI glue: Application setup, root entity, effect runner, timers, channels
geom.rs                 Point/Size/ScrollDelta app types + From<gpui::…>
input.rs                Keystroke -> KeyInput, KeyInput -> terminal bytes, actions!/keymap
theme.rs                DesignTokens (moved) + to_gpui_component_theme()
assets.rs               AssetSource over assets/icons, IconKind -> path
ui/mod.rs               shared leaves: status_pill, signal_dot, rail_marker, roster_ring, section_label, tooltip, icon_button
ui/ellipsized.rs
ui/popover.rs           anchored/deferred pane menu with dismiss-on-outside-press
ui/scroll.rs            ScrollTarget -> ScrollHandle registry
views/root.rs           view(): shell, docked/floating GitHub panel, scrim + modal, toast
views/sidebar.rs        sidebar, collapsed_sidebar, workspace_row, fleet_*, global_alert_row
views/workspace.rs      workspace_view, app_bar_tabs (drag reorder), commands_pill, status bar
views/panes.rs          view_tree (split drag), view_pane_stack, view_pane header + menu
views/dialogs.rs        the 7 dialogs
views/palette.rs
views/github/{panel,lists,diff}.rs
views/settings/{mod,preferences,worktrees,theme_gallery,previews}.rs
terminal/element.rs     TerminalElement (Element + EntityInputHandler)
terminal/runs.rs        TerminalRunStyle/Kind, terminal_row_style_runs, link detection (moved)
terminal/box_drawing.rs model unchanged, PathBuilder painter
terminal/images.rs      RenderImage cache + placement geometry (moved)
terminal/scrollbar.rs
e2e.rs, e2e_pixels.rs   (feature e2e)
settings.rs, metrics.rs, themes.rs, github.rs, commands.rs, agent_screen.rs,
agents_roster.rs, process.rs, doctor.rs, gpu.rs   (existing, minimally edited)
```

---

## 4. Phases, in order

Each phase ends with `cargo fmt --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace --all-targets --locked`,
and — from Phase 6 — the headless e2e test. Each phase is its own PR onto
`feat/gpui-port` (Phase 0 goes to `main` directly).

### Phase 0 — Split `main.rs` while still on Iced (mergeable to `main`)

Purely mechanical, no behaviour change. Do this first because porting a
28k-line file in place is not reviewable.

1. Move `enum Message`, `struct Muxtrix`, `update()` and pure helpers to
   `app.rs`; view methods to `views/*.rs` as `impl Muxtrix` blocks in separate
   files (Rust allows split `impl`s); terminal run model to
   `terminal/runs.rs`; `DesignTokens` to `theme.rs`; pane-tree geometry helpers
   to `layout.rs`; tests to `tests/` modules next to the code they test.
2. Introduce `geom.rs` (`Point`, `Size`, `ScrollDelta`) and switch `Message`
   variants and unit tests to them, converting at the Iced boundary. Introduce
   `KeyInput` and `input::from_iced(..)` the same way.
3. Introduce `Effect` and change `update()` to return `Vec<Effect>`; add an
   Iced effect runner that maps each `Effect` to the current `Task`. All unit
   tests now assert on `Vec<Effect>` instead of `Task` — this is the one place
   test code changes.

Exit: identical screenshots across the full capture-gallery matrix (diff the
PNGs against a baseline run from `main`; allow zero pixel differences), all
tests green, PR merged to `main`.

### Phase 1 — Platform spike (time-boxed, 2 days)

Goal: prove GPUI renders on every target we ship before writing app code.

1. New throwaway crate `crates/muxtrix-gpui-spike` (not a workspace member of
   release builds; add to `[workspace.members]` behind a comment, delete at the
   end of Phase 2). Depends on `gpui`, `gpui_platform`, `gpui-component` pinned
   as in §2.1.
2. It opens a 1280×800 window with: a gpui-component `Sidebar` + `Button` +
   `TextInput` + `Select` + `Slider` + `Switch`; and a custom element that
   paints a 200×60 grid of random coloured cells with `shape_line` using the
   user's configured monospace family — i.e. a synthetic `TerminalElement`.
   Logs `adapter name/backend` (GPUI logs `Selected GPU adapter` at info level)
   and the frame time.
3. Run matrix and record results in `docs/GPU.md` under a new "GPUI" heading:
   - Linux native X11 and Wayland (NVIDIA, AMD/Intel Mesa).
   - **WSLg** with NVIDIA (`dzn`) and without (`d3d12` GL). If the Vulkan
     adapter is selected and is broken/slow, try `ZED_DEVICE_ID` to steer, then
     implement the fallback below.
   - Windows 11 native (DirectX).
   - macOS (Metal).
   - Xvfb + llvmpipe (the CI path): `xvfb-run cargo run -p muxtrix-gpui-spike`
     must render and the `GetImage` capture must show the grid.
4. WSLg fallback, only if needed: patch `gpui_wgpu::WgpuContext::new_with_options`
   to honour `wgpu::Backends::from_env()` (the standard `WGPU_BACKEND` env var)
   when set. Keep it as a `[patch."https://github.com/zed-industries/zed"]`
   pointing at a **local vendored copy under `vendor/`** (same pattern as
   `vendor/libghostty-vt-sys`) or at a fork the **user** has created under
   their own GitHub account. **Do not open an upstream PR, issue, or fork**
   (goal 7); describe the patch in the PR body so the user can decide whether
   to contribute it. `gpu.rs` then keeps its WSL re-exec with
   `WGPU_BACKEND=gl` unchanged.
5. Measure build time of the spike from clean; note it in the spike PR.

Exit: a table in `docs/GPU.md` with pass/fail + adapter per platform, the fork
patch (if any) pinned, and a go/no-go note. **If Linux WSLg cannot be made to
work acceptably with or without the patch, stop and report — do not proceed to
Phase 2.**

### Phase 2 — Shell + terminal element behind a feature flag

1. Add feature `gpui` to `muxtrix-app` (default off). `main.rs` selects the Iced
   runtime or `runtime.rs` by feature. Both compile; only one runs.
2. `runtime.rs`: `gpui_platform::application().run(|cx| { gpui_component::init(cx);
   theme::install(cx, &settings); cx.open_window(opts, |window, cx| { let app =
   cx.new(|cx| Muxtrix::boot_gpui(window, cx)); cx.new(|cx| Root::new(app,
   window, cx)) }) })`. Implement the effect runner, timers, terminal wake
   loop, window observers, clipboard, exit, title.
3. `terminal/element.rs` per §2.4, `terminal/box_drawing.rs` painter,
   `terminal/images.rs`, `terminal/scrollbar.rs`, `input.rs`.
4. `views/root.rs` + `views/workspace.rs` + `views/panes.rs` minimal: a single
   workspace, tab strip, split tree with divider drag, pane header with the real
   icon buttons, the pane menu popover, the terminal. No sidebar/settings yet
   (render placeholders).
5. GPUI tests: `terminal_element_reports_rows_and_cols`,
   `background_runs_paint_as_quads`, `box_drawing_paths_join_at_cell_edges`
   (assert path points land on `cell_width` multiples), `keystroke_translation`
   table covering every arm of the old `Keyboard` handler.

Exit: `cargo run --features gpui` on Linux shows a working terminal: typing,
resize re-grids the PTY, splits, stacked panes, mouse reporting, selection +
copy/paste, scrollback wheel + scrollbar drag, links (Ctrl+Shift hover + click),
Kitty image (`kitten icat`), box-drawing TUI (`htop`/`lazygit`) with joined
borders, cursor blink, unfocused cursor outline. Manually verified by the
host user only (AGENTS.md: no GUI in automation).

### Phase 3 — Sidebar, fleet, alerts, app bar, tooltips, toast

Port `views/sidebar.rs` (expanded + collapsed rail, workspace rows with tab
drop targets, fleet header segmented control, fleet rows, roster ring, alerts,
GitHub status chip), `commands_pill`, status bar, the bottom toast pill, and
`ui/*` leaves. Use `uniform_list` for fleet/workspace lists only if row heights
remain identical. Tooltips via gpui-component `Tooltip` with the same copy.

Exit: every `capture` state under the `workspace`/`sidebar`/`fleet` groups in
`matrix.mjs` renders (temporarily screenshot by hand via the spike's `GetImage`
helper until Phase 6 lands). Layout measurements (rail 272/46, app bar 43,
row heights) match the Iced build within ±1 px — compare against the Phase 0
baseline PNGs.

### Phase 4 — Dialogs, command palette, settings, theme gallery

1. Dialogs → gpui-component `Dialog`/modal over the root with the existing
   widths (460/480/600) and the scrim token; Escape/outside-press dismissal
   identical to today (`opaque` + `mouse_area` semantics).
2. Command palette → `TextInput` + `uniform_list`; keep `PALETTE_*` focus and
   scroll-to-top behaviour; keep `enabled_palette_selection` logic untouched.
3. Settings → `Select` for pickers (font family menus capped at 320 px),
   `Slider` for sizes, `Switch` for toggles, `TextInput` for hook commands, the
   versions section, Windows-only shell backend section (`cfg(windows)`), dirty
   tracking + save/cancel. `ScrollTo(Settings, ratio)` effects for the
   `OpenDefaultAgentSettings` and e2e scroll actions.
4. Worktree manager page (`WorktreeLanes` table) and theme gallery (two per
   row, live previews built from the same run model used by the terminal —
   replace the `rich_text` sample with a tiny `TerminalElement` fed a synthetic
   `GridSnapshot`, which also removes the second `rich_text` site).

Exit: all `settings`/`dialog`/`palette`/`theme` capture states render; settings
round-trip through `AppSettings::save_to` unchanged.

### Phase 5 — GitHub panel and diff view

Docked (≥1080 px) vs floating panel, tab bar, loading animation (90 ms tick),
local view, PR list (58 px rows) and file list (42 px rows) as `uniform_list`
with the existing virtual-window math retired in favour of GPUI's (keep the
keyboard-focus model `GitHubPanelKeyboardFocus` and its scroll-into-view), diff
view with soft-wrap notice and `"muxtrix-github-diff"` scroll target.

Exit: all `github` capture states render; keyboard navigation parity checked
against the Iced build by the host user.

### Phase 6 — e2e harness, CI, gallery

1. Implement §2.7: `e2e_status`/`quit` control methods, `GetImage` capture in
   `headless_e2e.rs`, shared `e2e_pixels.rs`, `wait_for_first_frame`.
2. Update `headless_e2e.rs` env: drop `WINIT_UNIX_BACKEND`; keep `DISPLAY`,
   unset `WAYLAND_DISPLAY`; keep `GALLIUM_DRIVER=llvmpipe`; set `WGPU_BACKEND`
   only if the Phase 1 patch landed. Window discovery by geometry still works
   (GPUI creates a normal X11 toplevel).
3. CI: add the apt packages from §2.1; pin toolchain; cache `~/.cargo/git`.
   Expect the `checks` job to need a longer timeout (GPUI cold build) — measure
   and set it.
4. `scripts/capture-gallery`: run the full matrix; fix every failing case.

Exit: `cargo test -p muxtrix --features e2e,gpui --test headless_e2e --
--test-threads=1` passes with all 23 checks on CI; the gallery builds 186
frames; `AGENTS.md` "Seeing the UI" section updated to describe the new capture
flow.

### Phase 7 — Cut over and clean up

1. Make `gpui` the only runtime: delete the Iced runtime, `popover.rs`,
   `ellipsized_text.rs`, the old `styled_terminal`, `iced` from all
   `Cargo.toml`s, `iced::wgpu` uses in `gpu.rs`/`doctor.rs`/`muxtrix-gpu-probe`
   (switch to the direct `wgpu` dep), `intern_font_family`, and
   `metrics::system_monospace_family` if Phase 2 measurements showed GPUI's
   monospace resolution matches fontdb's.
2. Packaging: Linux `.deb` depends (`$auto` picks up new shared libs — verify
   with `scripts/verify-deb-package.sh`), zigbuild viability decision,
   Homebrew template unchanged, Windows manifest via `windows-manifest`
   feature (already default in `gpui`), THIRD_PARTY_NOTICES regenerated
   (`gpui` Apache‑2.0, `gpui-component` Apache‑2.0, zed `font-kit` fork).
3. Docs: `PRODUCT.md:54,91,97`, `DESIGN.md:3,15,302`, `docs/ARCHITECTURE.md`,
   `docs/GPU.md`, `docs/DEVELOPMENT.md`, `docs/TESTING.md` — replace every
   "Iced" statement with the GPUI equivalent.
4. Visual polish pass (this is where the "make it look good" payoff lives):
   adopt gpui-component's control styling for inputs/selects/sliders/switches,
   focus rings, hover/pressed states, dialog shadows, and motion; keep
   `DesignTokens` values. Run the `ux-screenshot-review` skill on the gallery.

Exit: `main` builds without `iced` anywhere in `Cargo.lock`; release v0.2.0.

**Decisions taken (Aug 2026):**

- *zigbuild:* dropped. GPUI links the host's `libxkbcommon` dynamically, and a
  24.04 library needs glibc 2.38 symbols a 2.34-pinned link cannot promise.
  The Linux release job runs on `ubuntu-22.04` with plain `cargo build`; the
  package baseline in `scripts/verify-deb-package.sh` is glibc 2.35, which is
  the same "Ubuntu 22.04 LTS and newer" floor the install notes always gave.
- *Headless e2e on a two-core llvmpipe runner:* the scenario records what it
  has to have seen on every frame, not only on its tick; the harness keeps
  key auto-repeat off, re-pins the window before aiming pointer events, sweeps
  the mouse probe until the probe itself stops listening, and clicks the
  initial pane before typing the probe (the scenario's own splits move focus).
  Budgets are 60 s. The `e2e_*_trace` counters in `app.rs` feed the failure
  report; CI cannot be watched, so the report has to carry the evidence.
- *Item 4 (visual polish pass)* is not part of the cut-over PR; parity with
  the Iced gallery (8.4 % mean pixel diff, 164/186 states under 15 %) is.

---

## 5. Mapping tables for the porting agent

### 5.1 Iced → GPUI / gpui-component

| Iced | Replacement |
|---|---|
| `column![]`/`row![]`/`container` | `div().flex().flex_col()` / `.flex_row()`; padding/size via Tailwind-style methods (`p_2()`, `w(px(272.))`, `h_full()`) |
| `stack([a, b])` | `div().relative().child(a).child(div().absolute().inset_0().child(b))` |
| `opaque(x)` | `div().occlude()` |
| `mouse_area(x).on_press/on_move/on_enter/on_exit/on_scroll` | `div().id(..).on_mouse_down/.on_mouse_move/.on_hover/.on_scroll_wheel` or `window.on_mouse_event` inside a custom element |
| `scrollable(x).id(ID)` | `div().id(..).overflow_y_scroll().track_scroll(&ScrollHandle)`; lists → `uniform_list(..).track_scroll(UniformListScrollHandle)` |
| `operation::snap_to(ID, RelativeOffset)` | `ScrollHandle::set_offset` / `UniformListScrollHandle::scroll_to_item` via `Effect::ScrollTo` |
| `operation::focus(ID)` | `FocusHandle::focus(window)` / `InputState::focus` via `Effect::Focus` |
| `text_input` | gpui-component `TextInput::new(&InputState)`; `on_input`/`on_submit` → subscribe to `InputEvent::Change/PressEnter` |
| `pick_list` | gpui-component `Select` (font menus: `.max_height(px(320.))`) |
| `slider` | gpui-component `Slider` (`SliderState`) |
| `toggler` | gpui-component `Switch` |
| `button(..).style(..)` | gpui-component `Button::new(id).ghost()/.outline()/.primary()` with theme overrides, or `div().id().cursor_pointer().hover(..).active(..)` for bespoke chrome buttons |
| `tooltip(x, copy, pos)` | `.tooltip(|window, cx| Tooltip::new(copy).build(window, cx))` |
| `svg(handle).style(color)` | `svg().path(AssetPath).size(px(n)).text_color(color)` |
| `canvas(Program)` | custom `Element` + `window.paint_path`/`paint_quad` |
| `rich_text`/`Span` | `StyledText::new(text).with_runs(Vec<TextRun>)` or `shape_line` in the element |
| `sensor(..).on_resize` | bounds known in `prepaint`; emit message when changed |
| `Popover` overlay | `deferred(anchored().position(p).anchor(Corner::TopRight).snap_to_window_with_margin(px(6.)).child(menu))` + root `on_mouse_down_out`/`window.on_mouse_event` capture for dismiss-and-consume |
| `iced::time::every` | `cx.spawn` + `background_executor().timer` loop |
| `Subscription::run_with(stream)` | `cx.spawn` awaiting the channel |
| `iced::clipboard::{read,write}` | `cx.read_from_clipboard()` / `cx.write_to_clipboard` |
| `iced::exit()` | `cx.quit()` |
| `window::screenshot` | removed — X11 `GetImage` in the test (§2.7) |
| `Font::with_name(&'static str)` | `Font { family: SharedString::from(String), .. }` |
| `font::Weight::*` | `gpui::FontWeight::*` (numeric `FontWeight(f32)`, so `weight_from_numeric` becomes `FontWeight(n as f32)`) |
| `Color` | `gpui::Hsla`/`Rgba` (`rgb(0xRRGGBB)`, `rgba(0xRRGGBBAA)`, `hsla(..)`); DesignTokens store `Rgba` |
| `Theme::Light/TokyoNight` | gpui-component `Theme` light/dark + `ThemeColor` overrides from tokens |

### 5.2 Iced keyboard → `KeyInput`

`gpui::Keystroke { modifiers: Modifiers { control, alt, shift, platform,
function }, key: String (lower-case name: "a", "enter", "escape", "tab",
"up", "pageup", "f1", ...), key_char: Option<String> }`. Map `key` names to
the app `Key` enum one-to-one with Iced's `Named` names; `key_char` is the text
to send when no binding fires; on Windows/Linux `platform` is the Win/Super
key, on macOS Command — keep `clipboard_shortcut_for` semantics (L15319).

### 5.3 Messages whose payload type changes

`Keyboard`, `ResizePane`, `ResizeSplit`, `PointerMoved`, `TerminalPointerMoved`,
`TerminalScrollbarMoved`, `ScrollTerminal`, `ScrollHoveredTerminal`,
`WindowOpened` (drop the window id), `WindowResized`, `E2eScreenshot` (deleted;
replaced by `E2eCaptureReady`).

---

## 6. Risks and how each is handled

| Risk | Mitigation |
|---|---|
| WSLg picks the broken `dzn` Vulkan adapter | Phase 1 spike; local `WGPU_BACKEND` patch via `vendor/` or user-owned fork (no upstream PR); existing `gpu.rs` re-exec stays |
| GPUI git API churn between pins | Single `<ZED_REV>` in one place; bump only at phase boundaries; gpui-component pinned to a rev that locks the same zed rev |
| Cold build time / CI minutes | `sccache`/`~/.cargo/git` cache; measure in Phase 1; consider building GPUI deps once in a container image |
| `cargo zigbuild` glibc 2.34 vs font-kit/fontconfig | Resolved in Phase 7: release built on `ubuntu-22.04`, baseline glibc 2.35 |
| Pixel-exact expectations (`DESIGN.md` baselines, box-drawing continuity) | Phase 0 baseline PNGs; per-phase ±1 px checks; e2e continuity checks moved to the test binary |
| IME/composition on Windows and macOS | `EntityInputHandler` on the terminal element (Phase 2), manual check by host user |
| No runtime window icon / resize increments in GPUI | Desktop file + resource icon; increments dropped (documented) |
| `unsafe_code = deny` | GPUI is a dependency; our crates stay safe-only. Do not add `unsafe` to work around platform gaps — carry a local `[patch]` instead (never an upstream PR) |
| Accessibility regression (Iced exposed widget text via operations) | GPUI has AccessKit wiring (`_accessibility.rs`); label interactive elements; revisit in Phase 7 |

---

## 7. Definition of done

- `feat/gpui-port` merged to `main` with `iced` absent from `Cargo.lock`.
- CI green on all three OS jobs; Linux `checks` job includes the e2e test.
- Capture gallery: 186 frames, every `check` in `matrix.mjs` satisfied.
- Manual sign-off by the host user on: Linux X11, Linux Wayland, WSLg
  (NVIDIA and non-NVIDIA), Windows 11, macOS — each covering typing, splits,
  TUI box drawing, images, copy/paste, palette, settings, GitHub panel.
- `docs/` and `PRODUCT.md`/`DESIGN.md` no longer mention Iced.
