# Roadmap

The target is fluid multi-session multitasking and programmability across
Linux, Windows, WSL2, and macOS.

## Completed hardening and UX milestone (2026-08-09)

- Added a Windows shell-backend setting that launches either a native Windows
  shell or a selected WSL distribution. It preserves pane identity and control
  bridge automatically for Windows-to-WSL agent sessions.
- Fixed terminal grid synchronization after pane or application resizing. Input
  written after a resize must appear at the live prompt rather than at stale
  pre-resize coordinates. Cover rapid resize followed by typing in the native
  terminal tests and the private-display E2E.
- Fixed agent lifecycle routing across process boundaries by propagating the
  server's exact endpoint alongside `MUXTRIX_PANE_ID`; WSL receives both through
  `WSLENV`, so hook delivery never depends on a different HOME-derived pipe.
- Enumerated supported weights for the selected terminal font and let the user
  persist a weight such as SemiBold without offering variants the family does
  not provide.
- Removed the fleet row's horizontal outer inset so entries and separators use
  the full sidebar width while retaining internal text padding.
- Built the Windows GUI executable as a GUI subsystem application without an
  unwanted companion Command Prompt; `muxtrixctl` remains a console program.
- Simplified the top-level naming hierarchy: removed the redundant app name at
  the upper left, distinguish the actual workspace name from the application,
  and add create, rename, switch, and close workspace management.
- Replaced fleet shortcuts such as `mod 1` and `mod 2` with the real
  platform-specific chord (`Ctrl+1` on Windows/Linux or `Cmd+1` on macOS), or
  remove the label where the shortcut is not actionable.

## M0: foundation

- Rust workspace, formatting, linting, unit tests, and cross-platform CI
- Serializable workspaces, surfaces, tabs, and recursive split trees
- Iced application shell with explicit wgpu renderer
- Ghostty VT adapter with deterministic headless tests
- Local PTY abstraction and lifecycle
- Native Windows and Windows-to-WSL2 launch profiles

## M1: usable terminal workspace

- Polished workspace chrome and consistent pane/action controls
- Cross-platform Ctrl+P/Cmd+P command palette with a shared action registry
- Persistent interface and terminal font settings with PTY metric recalculation
- GPU cell renderer, glyph atlas, and selection; cursor and mouse-wheel
  scrollback are complete in the current rich-text bridge
- Multiple workspaces with vertical sidebar
- Horizontal/vertical splits, draggable resizing, focus navigation, and pane
  tabs (complete)
- Shell/profile selection and working-directory inheritance
- Copy/paste with Ghostty default chords and bracketed-paste encoding
  (complete)
- Links, search, zoom, and configurable shortcuts
- Session persistence and restart restoration

## M2: agent supervision

- OSC 9/99/777 notification handling (in-app complete)
- Pane attention rings and unread counts (complete); desktop notifications and sounds
- Sidebar metadata: process, working directory, git branch, listening ports,
  and notification text
- Reversible hooks for Codex and Claude Code (complete); adapters for other
  agent CLIs remain
- Configured Codex/Claude launch actions and lifecycle pane badges (complete)
- Session activity history and notification center (in-app notification center complete)

## M3: programmability and remote work

- CLI plus Unix-socket/named-pipe API (first local protocol complete)
- Focus/split/close/send/read-capture/notify/agent-launch commands (complete)
- SSH profiles and remote tmux attach workflows
- Agent-created pane/sub-agent discovery protocol
- Skills/workflow integration

## M4: adjacent surfaces

- Embedded browser surface with navigation and developer automation API
- Live Markdown viewer
- Browser DOM snapshot/click/type/evaluate/console/network commands
- Cross-platform update packaging, crash reports, and diagnostics

## Deferred parity

Mobile terminal synchronization is intentionally deferred until desktop
session security, persistence, and remote transport are mature.
