# Product

## Platform

adaptive

## Users

Muxtrix is for software developers supervising multiple terminals, local tools,
and concurrent coding agents. Its primary users need to move quickly between
several active tasks without losing context, especially when working across
native Windows and WSL2 environments.

## Product Purpose

Muxtrix is a GPU-accelerated desktop terminal workspace for running, organizing,
and supervising concurrent development sessions. It should let a developer
move between panes and tasks quickly, understand what every pane is doing, and
notice when an agent or command is blocked or needs human attention without
interrupting productive work elsewhere.

Success means the workspace remains legible and controllable as the number of
simultaneous terminals, tools, and agents grows. Switching tasks should be
fast, attention state should be obvious, and background activity should remain
informative rather than disruptive.

## Positioning

Muxtrix combines a cross-platform, GPU-rendered terminal workspace with
first-class agent supervision, reversible agent integrations, and a
programmable local control API. The same native application supports
Windows processes, WSL2-hosted sessions, and Linux, with macOS targeted but
not yet published.

## Operating Context

- Developers keep multiple shells, builds, editors, and coding-agent sessions
  active at the same time.
- Work is organized into independently focusable terminal panes and recursive
  splits, with rapid keyboard-driven navigation between tasks.
- Background sessions may complete, fail, request permission, or otherwise
  require human attention while work continues in the foreground.
- Notifications, pane attention state, and agent metadata help users identify
  what is running, what changed, and where intervention is required.
- Windows users may run Muxtrix natively while choosing either native Windows
  processes or sessions hosted in WSL2.
- The application also targets native Linux and macOS use, with macOS supported
  on a best-effort basis until a test environment is available.
- Local automation uses `muxtrixctl` and a private Unix socket or Windows named
  pipe rather than a remotely exposed control service.

## Capabilities and Constraints

- Muxtrix is written primarily in Rust using Iced and its wgpu renderer.
- Terminal state uses `libghostty-vt`; Muxtrix owns PTYs, panes, process
  backends, terminal drawing, application state, and lifecycle.
- Application chrome and terminal content share a GPU-accelerated compositor.
- The product targets Windows, Linux, WSL2, and eventually macOS from one
  maintainable architecture while preserving platform-native behavior where
  appropriate.
- Windows-native and Windows-to-WSL2 sessions are explicit, separate process
  backends rather than an implicit environment switch.
- Terminal panes must remain independent in process ownership, input, output,
  resize behavior, focus, and lifecycle.
- Keyboard-first control, rapid pane navigation, command discovery, readable
  typography, font configuration, and interface scaling are durable product
  requirements.
- Agent integrations must be opt-in and easy to add, remove, or re-add without
  disturbing configuration owned by the user or other tools.
- Notifications must clearly distinguish blocked, waiting, completed, and
  failed work without stealing focus or interrupting unrelated foreground work.
- Automated checks run headlessly. Browser-, DOM-, HTML-, and JavaScript-based
  UI workflows do not apply to the native Rust interface.
- Remote transport, authentication, session synchronization, and mobile access
  remain future decisions rather than current product claims.

## Brand Commitments

- The product name is Muxtrix.
- The experience should be clean, polished, and cohesive at an Apple-level
  quality bar while remaining an efficient professional tool for developers.
- Familiar native desktop behavior and direct, precise language take priority
  over novelty that slows operation.
- The durable visual register is a familiar, exceptionally finished native
  developer workspace that should sit naturally alongside the best contemporary
  developer tools rather than adopting an ornamental metaphor.

## Evidence on Hand

- The implemented Rust workspace and current application surface live under
  `crates/`, with the Iced application in `crates/muxtrix-app`.
- `docs/ARCHITECTURE.md` records the GPU, terminal, process-host, persistence,
  and control-service boundaries.
- `docs/AGENT_INTEGRATIONS.md` documents reversible Codex and Claude Code
  lifecycle integration.
- `docs/TESTING.md` documents deterministic integration coverage and the
  private-display Iced/wgpu E2E harness.
- The repository contains no testimonials, customer claims, benchmarks,
  pricing, or market evidence; future product work must not fabricate them.

## Product Principles

1. Preserve flow: navigation, focus changes, and task switching should be fast
   enough to become habitual and should never disrupt unrelated work.
2. Make state unmistakable: every pane should communicate what it is doing and
   whether it is active, waiting, blocked, completed, failed, or unattended.
3. Earn trust through reversibility: automation and agent integrations must be
   explicit, inspectable, removable, and respectful of existing configuration.
4. Deliver native quality everywhere: cross-platform reach must not excuse
   inconsistent input, rendering, accessibility, or platform behavior.
5. Scale from one shell to many agents: the interface should remain calm,
   legible, and controllable as concurrent activity grows.

## Accessibility & Inclusion

Follow native desktop accessibility best practices for Windows, Linux, and
macOS. Preserve complete keyboard operation, visible focus, readable and
configurable text sizing, sufficient contrast, meaningful state labels beyond
color alone, reduced-motion preferences where motion is introduced, and
assistive-technology semantics supported by the native UI framework.
