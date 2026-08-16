# Muxtrix watchdog

Review for material correctness, safety, and regression risks. Prefer one concrete, actionable finding over style commentary.

Especially watch for:

- Violations of crate and runtime boundaries: domain state stays independent of Iced, process hosts stay outside terminal state, and raw Ghostty handles never leave their owning terminal actor thread.
- Ordering or lifecycle bugs around PTY output, resize, input, snapshots, bounded wake channels, pane closure, child-process cleanup, and shutdown. Stale-grid frames must not render after resize.
- Cross-pane or cross-workspace leaks. Runtime handles, terminal events, selections, activity, titles, and control commands must remain keyed to the correct `PaneId`.
- Unsafe handling of untrusted terminal output, escape sequences, OSC payloads, clipboard requests, control-service input, repository data, or agent hook data. Preserve sanitization, bounds, local-only IPC, and private endpoint permissions.
- Hook-management changes that overwrite third-party configuration, restore whole files, lose concurrent edits, or remove entries Muxtrix does not own. Muxtrix-owned changes must remain marked, selective, reversible, and permission-preserving.
- Platform assumptions that collapse Linux, native Windows, macOS, WSL, Unix PTYs, ConPTY, Windows paths, or WSL paths into one behavior. Keep platform-specific behavior behind existing adapters and preserve explicit user overrides.
- Persistence changes that serialize runtime handles, skip schema migration/versioning, or replace atomic writes with partial in-place updates.
- Terminal rendering changes that break fixed-cell geometry, wide-cell ownership, Unicode fallback, box drawing, clipping, theme/OSC precedence, cursor/selection layering, or dirty-row reuse.
- New unbounded queues, avoidable per-frame allocation or copying, polling loops, periodic UI ticks, blocking work on the Iced update/render path, or redundant snapshots/wakeups.
- Visible UI changes accepted without the real headless `headless_e2e` capture path. Never recommend launching the GUI on the host.
- Behavior changes without focused coverage at the appropriate layer: deterministic domain/runtime tests first; real headless E2E for interaction or rendered-surface contracts.
- Changes that contradict `docs/ARCHITECTURE.md`, `docs/TESTING.md`, `docs/AGENT_STATE_DETECTION.md`, or `docs/TERMINAL_HOST_RESILIENCE.md` without updating the relevant decision record.
