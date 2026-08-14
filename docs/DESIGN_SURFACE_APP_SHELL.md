---
version: 1
slug: "crates-muxtrix-app-src-main-rs"
primary_target: "crates/muxtrix-app/src/main.rs"
related_targets: ["crates/muxtrix-app/src/settings.rs","crates/muxtrix-app/src/commands.rs","crates/muxtrix-app/src/github.rs","crates/muxtrix-app/src/e2e.rs"]
---

# Muxtrix native workspace

- Scope: the native Iced application shell in `crates/muxtrix-app/src/main.rs`,
  including workspace/fleet navigation, command palette, settings, terminal
  pane chrome, and the focused-pane GitHub review panel.
- Mode: Operate.
- Audience and job: developers supervising several terminals and coding agents;
  keep the focused terminal primary while scanning truthful pane, repository,
  pull-request, review, check, and merge state without leaving the workspace.
- Constraints: preserve working shortcuts, terminal behavior, private control
  transport, headless testing, cross-platform architecture, collapse behavior,
  semantic tokens, and the established 272/46 px fleet rail. No browser/DOM UI
  workflow applies to this native surface.

## Chosen direction

Extend the approved dark live-gate-board world with a compact, ruled GitHub
review ledger docked at the right on wide windows and overlaid on compact ones.
Anchor it on a readiness header, PR identity, and dense changed-file rows using
Muxtrix typography, semantic state colors, line grammar, native controls, and
truthful local/GitHub data.

- Memorable moment: an agent can remain visible in the terminal while the same
  workspace shows a PR's readiness, checks, changed files, and guarded merge.
- The expanded fleet footer shows GitHub account identity and state immediately
  left of collapse; the collapsed rail keeps the GitHub icon and state dot.
- Long PR lists render only the visible rows plus overscan. File paths truncate
  before fixed addition/deletion lanes, and scrolling never changes terminal
  geometry.
- Merge is enabled only for a clean, mergeable, review/check-ready PR, opens an
  inline confirmation, keeps the branch, and guards the confirmed head commit.
- Unauthenticated panels replace data with one centered explanation and auth
  action, then refresh in place when GitHub CLI's browser flow completes.
- Settings is a full-window operating surface with persistent
  Preferences/Worktrees navigation and an explicit return to the terminal.
  Worktree inventory loads Git metadata in the background and uses the space
  for separate identity, branch, usage/protection, local-risk, and action lanes.

## Component grammar

- Corners: square continuous regions and pane fields; 4-6 px radii only for
  compact standalone controls, inputs, and the command palette.
- Lines: 1 px cool graphite rules divide top bar, fleet rows, attention rows,
  pane headers, and split boundaries. The focused pane alone receives a 1 px
  blue perimeter.
- Elevation: no card shadows in the application shell. The command palette may
  use one restrained overlay shadow because it is truly floating.
- Type: native workhorse sans for UI, with a tight role scale; configured
  monospace for terminal content. Small text remains readable and follows the
  interface-size setting.
- State: blue is current focus, amber is human attention, green is completion,
  red is failure, and neutrals represent running or idle state. Every state has
  a text label in addition to color. Rail keyboard navigation uses a complete
  blue perimeter and blue-tinted fill, distinct from the current row's neutral
  fill and leading blue selection bar.
- Motion: state transitions are immediate or short fades; no decorative motion.

## Implementation inventory

| Visible ingredient | Commitment | Medium |
| --- | --- | --- |
| Native top command bar | Muxtrix, command access, Settings | Semantic Iced widgets |
| Workspace inventory | Ruled workspace rows above Fleet with rolled state, counts, real context, inline rename, and nested draggable tabs | Semantic Iced widgets and domain/runtime state |
| Workspace tab strip | Scrollable ordered tabs with signal, name, pane count, close, drag/drop, and one-pane add action | Semantic Iced widgets and domain/runtime state |
| Fleet projection | Selected-workspace panes only; Tabs preserves tab bands, Agents is a flat filter in tab/pane order, and Repos groups every pane under repository-only bands with a `No Repo` fallback | Semantic Iced widgets, cached Git metadata, and domain/runtime state |
| Conditional global Attention | Ruled rows only for non-pane messages; omitted when empty | Semantic Iced widgets and state |
| Collapsible fleet rail | Rich 3-4 line task rows with title, identity/state, latest activity, context | Semantic Iced widgets and domain/runtime state |
| Collapse control | Subtle double chevrons at lower-right; compact rail retains state signals | Semantic Iced button |
| Pane tab | Compact left tab with title or agent state and close action | Semantic Iced widgets |
| Pane controls | Split right, split down, maximize/restore, overflow | Code-native controls with accessible labels |
| Terminal field | Existing Ghostty snapshot projection remains dominant and readable | Existing GPU-backed Iced rich text |
| Focus and attention | Blue focus perimeter; amber waiting state on pane and fleet row | Semantic color tokens and borders |
| Settings | Full-window Preferences/Worktrees navigation, grouped native preferences, configured-agent default selection, and asynchronous ruled worktree inventory | Semantic Iced form and list controls |
| Optional status bar | Hidden by default; enabled from Settings | Semantic Iced widgets |
| Command palette | Search field and ruled command rows in the same system | Semantic Iced overlay |
| GitHub review panel | 372 px ruled dock on wide windows, opaque overlay on compact windows, auth/readiness/checks/virtualized files/guarded merge | Semantic Iced widgets plus Git and GitHub CLI state |
| Raster media | None; the approved comp is a north star, not a shipped bitmap | Accepted omission |

## Non-literal comp details

The comp's demonstration repository names, terminal output, branches, agent
summaries, and global hook message are synthetic layout content. Production UI
must derive those fields from real Muxtrix state, omit unavailable metadata,
and never invent activity. Pane-control glyphs may be translated to the closest
consistent code-native icon treatment supported by Iced.

## Unresolved implementation decisions

- Native system appearance detection may vary by platform; when unavailable,
  System may resolve to the polished dark scheme while preserving the setting.
- Process, git branch, and PR metadata are shown only when real state exists;
  this redesign must not fabricate them.
- Light appearance inherits semantic tokens but remains less visually polished
  than dark appearance across the application.
- Merge queues and alternate merge strategies remain GitHub-owned behavior;
  this panel currently offers a normal merge commit only when readiness is clean.
