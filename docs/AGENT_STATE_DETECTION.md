# Agent state detection

## Decision

Codex uses its live terminal screen for `Running`, `Idle`, and `Needs input`.
Claude Code combines independent parent and background-task evidence. Its
session record (`~/.claude/sessions/<pid>.json`) describes the parent UI:
`busy`, `idle`, `waiting`, or `shell`. Hooks carry session identity, turn edges,
and complete `background_tasks` inventories; exact system-generated transcript
notifications can finish a task while the parent stays idle. A parent prompt
is not proof that its helpers or shells finished. Missing evidence produces
neutral `Unknown`, not invented completion. Oh My Pi supplies exact
active-turn and approval transitions through its managed extension, while OMP's
documented state-bearing OSC title supplies the correction and recovery layer.
`π >` means idle, `π !` means attention, and `π` followed by a supported Braille
spinner means working; ConPTY uses the static working form `π :`. Pi itself is
described only by its managed extension: it has no state-bearing title, its
owner can replace whatever chrome it paints, and its extension API reports
every edge the fleet row needs (`agent_start`, `agent_settled`,
`ui_prompt_start`, `ui_prompt_end`), so no screen rule applies to a Pi pane.

Other lifecycle hooks still identify the agent, session, working directory,
prompt submission, completion, and shutdown. Codex `SubagentStart` also keeps
delegated work active through the parent `Stop`. Codex permission and notification
hooks are not allowed to create human attention because its harness may resolve
those requests automatically. Claude Code's `PermissionRequest` is advisory
while a live session record is matched; that record confirms whether a
blocking dialog actually remains open. Oh My Pi's `agent_start` through
terminal `agent_end` interval and its approval events are exact. During that
interval, an idle title cannot demote the pane; this covers older OMP releases
that briefly published `π >` while an async job or scheduled continuation still
owned the turn. Pi's `agent_start` through `agent_settled` run and its
`ui_prompt_start` through `ui_prompt_end` waits are exact as well, with no
title to correct or be corrected by.

An unrecognized Codex screen or title preserves the last trusted state. Claude
recovers its parent state from its exact structured record and its task state
from durable evidence. Idle-looking chrome never resolves missing task evidence.

## Why the hook-only model was wrong

Codex calls `PermissionRequest` before choosing between its automatic approval
reviewer and the user. The hook payload identifies the request, but does not say
who will review it or expose the review decision. Its permission mode also does
not distinguish those paths. `PostToolUse` arrives only after a successful tool
has finished, not when automatic approval is granted, and may never arrive for
a failure or cancellation.

That ordering explains the observed sequence:

1. `PermissionRequest` changed the pane to `Needs input`.
2. Codex's reviewer approved without user action.
3. Muxtrix stayed in `Needs input` while the tool ran.
4. A later `PostToolUse` or turn boundary finally changed the state.

It also makes `PostToolUse` an unsafe resolver when tools overlap: output from
one tool must not clear a different approval prompt that is still visible.

## Research

Research was performed on 2026-08-12, including against the Codex source
(`be6e8ea`, 0.147-era): its hooks run before the automatic reviewer decision,
and its app-server protocol has structured auto-review and explicit
approval-request events. That confirms the cause of the observed sequence.
Direct app-server integration could be exact later, but would tightly couple
Muxtrix to a different transport than the real terminal process it hosts.

The implemented change adopts a conservative, evidence-ranked matching model:
session-integration hooks are separated from live state authority, and Codex's
`Needs input` still requires visible evidence on the live screen. Claude Code
was reworked again on 2026-08-26 after the screen-first model kept drifting:
its state now comes from the harness's own session record (see below).

The background-task model was checked against Claude Code 2.1.289 using
controlled real shell and subagent runs. Claude added hook task inventories in
[2.1.145](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md).
The recent [iTerm2 implementation](https://github.com/gnachman/iTerm2/commit/18893448f5e4252916b03da6664f9db9f4630e33)
demonstrates why `Stop` can still contain running tasks, why an idle prompt
must preserve them, and why `SubagentStop` must exclude its own finishing ID.
Muxtrix additionally observes silent task completion and persists evidence
across GUI replacement; an in-memory task count alone cannot cover those cases.

## Implemented model

On each terminal poll, the application evaluates each pane's latest Ghostty
grid snapshot and OSC title. Retained frames are re-evaluated so an identity
hook arriving just after a stable prompt paint cannot miss it on the next poll.
Codex uses its live screen and title. Claude Code combines its parent record,
hook task inventories, and exact per-task terminal observations. Its screen is
a limited fallback, never authority to clear active or uncertain child work.
Oh My Pi uses its state-bearing title
except that its exact `agent_start` through terminal `agent_end` lifecycle
bracket prevents an idle title from ending active work. Oh My Pi also retains exact
session switch/branch, approval-request, context compaction/handoff, and
shutdown events from the managed extension. A Pi pane takes no screen
classification at all; its extension's events are applied as reported, and
its idle `session_start` may end the `Running` state a launch or a process
scan started, because nothing else ever will.

- Codex `Action Required` OSC titles and strong live confirmation/answer forms
  create `Needs input`.
- Codex spinner titles or its bottom `Working (... esc to interrupt)` footer
  create `Running`; a plain nonempty Codex title supplies idle evidence. Between
  `SubagentStart` and the parent `Stop`, idle-looking chrome is treated as
  `Running` because Codex can park its parent spinner while helpers work.
  Visible input blockers still create `Needs input` during that interval.
- Claude spinner titles and its active `/btw` overlay create `Running`.
- Claude confirmation/navigation forms and dynamic-workflow prompts create
  `Needs input`; its idle OSC title is only fallback parent-idle evidence.
- Oh My Pi's `π >` title creates `Idle` outside an active lifecycle bracket,
  `π !` creates `Needs input`, and its ten supported Braille separators create
  `Running`. `π :` is the static ConPTY working form. A state-disabled
  `π: <label>` title identifies Oh My Pi but does not invent a state.
- Pi's `π - <session> - <directory>` title identifies Pi and names the pane;
  it never creates a state, and neither does anything else on a Pi screen.
- A Claude frame showing the Agents view returns no classification at all, and
  is evaluated before every rule below it. The roster draws its own composer and
  its own spinner-free title, either of which a later rule would otherwise read
  as this conversation's state.
- Claude's rendered composer — a `❯` line inside the last pair of horizontal
  rules, with no menu over it — supplies fallback parent-idle evidence. It
  cannot clear aggregate background work or `Unknown`, and ranks below a
  visible blocker. A `❯ 1. Yes` answer line is not an idle composer.
- Claude's session records are associated one-to-one by hook session ID, then
  by the exact harness PID from the pane's process tree, then by a cwd that
  is unique on both sides for a record the prober confirmed alive. Ambiguous records are ignored. While a live record is matched,
  the screen classifier has no authority over that pane at all.
- Transcript viewers return no classification so historical text cannot
  repaint the pane.
- Loose prose such as "do you want to proceed?" is insufficient without the
  accompanying form controls.
- Retained idle evidence may initialize a detected agent or resolve a visible
  wait, but the exact frame retained when `UserPromptSubmit` arrives cannot
  regress that newer running state. Muxtrix records the frame revision at the
  transition; a subsequently rendered idle frame can resolve `Running` unless
  Codex's delegated-work bracket, Claude's active or uncertain task inventory,
  or Oh My Pi's exact active-lifecycle bracket is still open.
- A completed turn remains `Done` while its idle composer is visible, preserving
  the useful completion signal. Strong working evidence starts the next turn
  even if `UserPromptSubmit` was lost, so `Done` cannot become a permanent latch
  when hook delivery is unavailable. Oh My Pi maintenance completion remains
  `Running`; only terminal `agent_end` completes its active turn, and only
  `agent_settled` completes Pi's.

Typed control events retain the original hook event name. Codex waiting hooks
remain metadata only. Claude combines record-confirmed waits with exact
elicitation/notification evidence; `PostToolUse` cannot clear an unrelated
wait. Oh My Pi approval events and active-turn lifecycle brackets remain exact
state transitions, as do Pi's prompt and settlement events. Claude completion
side effects require aggregate completion, not a parent-only stop.

The managed Oh My Pi and Pi extensions are versioned. Existing modules without
the current behavior marker are migrated during normal Muxtrix hook
synchronization, while the explicit hook re-add path remains available.
Migration preserves the original uninstall backup and removes the old Oh My Pi
footer status writes. An Oh My Pi module from before Pi support, which still
reports as `pi`, is recognised by the title it reports and migrated the same
way; until Oh My Pi reloads it, the app relabels its events as Oh My Pi's.

## Recovery across session reattach

Agent identity is part of the daemon-owned serialized pane layout. A new
Muxtrix instance restores that identity before attaching the pane's byte
stream, then applies the screen classifier to the terminal grid rebuilt from
backlog replay. Claude additionally re-associates its structured session by
unique cwd when neither a new hook nor a host-visible process PID is available.
Its parent record cannot reconstruct independent task work. The hook client
therefore journals typed activity before IPC, including while the GUI is
absent. A replacement GUI replays the pane's checkpoint and unacknowledged
events, then reconciles fresh parent and task evidence. Recovered evidence
remains uncertain until corroborated; fresh parent waits and activity still
take precedence. No new hook is required merely to recover the prior identity.

Layouts created before durable identity was added remain recoverable. Once the
replayed grid arrives, Muxtrix accepts only agent-specific signatures: Codex's
composer, working footer, approval forms, or branded title; Claude Code's
prompt box, Agents view, or branded title; and Oh My Pi's exact brand-only,
state-disabled (`π: <label>`), idle (`π > <label>`), attention (`π ! <label>`),
or branded spinner title. A generic title or unbranded spinner is not enough to
invent an agent. The recovered identity is written into the next layout update,
so this fallback is normally needed only once.

Process-tree detection remains useful for locally launched Linux panes and
hooks still supply session IDs, cwd, and turn boundaries. Neither is the
reattach source of truth: process inspection is not portable to a Windows host
running an agent through WSL, and lifecycle delivery can race application
replacement.

## Claude Code's Agents view

Claude Code 2.1.229 can switch a pane between its conversation and a roster of
every interactive and background session on the machine. The switch is one
keystroke (`←` on an empty composer), and a pane can also start there.

Behaviour confirmed against 2.1.229 by recording the pane's raw output:

| Surface | OSC 0 title |
| --- | --- |
| Working | `◐ <task>` |
| Idle | `✳ <task>` |
| Agents view | `claude agents`, or `<n> awaiting input · claude agents` |
| Returning from it | `current session` |

The title is what detection runs on. When a terminal suppresses it, the roster's
own chrome is the fallback, verified against a live 2.1.229 roster: the composer
placeholder `describe a task for a new session` and the footer's
`ctrl+x to delete all`. The footer's leading verb alternates between
`enter to expand` and `enter to collapse` as the list folds, so no signature may
depend on it.

Both roster titles come from one function in the harness, so they are
exhaustive. Two consequences drove this change:

1. Inside the roster, no previous rule matched, so the pane froze at its last
   state — indefinitely, for a pane that starts there.
2. On the way back, `current session` replaces the `✳ ` idle marker and is
   never repainted until the next turn. The pane is idle with no idle evidence.

The composer rule fixes (2) without any dependency on the harness's titles. The
roster rule fixes (1) and, being ordered first, also stops the roster's own
composer from being read as this conversation going idle.

`<n> awaiting input` is deliberately **not** used as attention evidence: a
freshly idle session with an empty composer is counted in it. Roster attention
comes from `claude agents --json` instead, whose per-session `state` separates
`blocked` and `failed` from `working` and `done`.

The read costs a short-lived subprocess (~0.25 s), so it runs off the UI thread,
at most one at a time and at most every two seconds, and only while a pane is
projecting the Agents view. Entering the view forces an immediate read. Windows
panes using the WSL backend run the query inside the configured distribution,
with hidden console creation.

The roll-up skips `interactive` entries: every interactive Claude Code already
owns the fleet row of the pane it runs in, including the pane doing the
viewing. Unknown kinds remain in the aggregate so a field that disappears
degrades to over-reporting rather than to an empty roster. A failed query
preserves already-visible aggregate counts. Ordinary Claude panes no longer
depend on this command at all.

## Claude Code session records

Claude Code keeps one JSON file per running process under
`~/.claude/sessions/<pid>.json` (`CLAUDE_CONFIG_DIR` relocates it). Confirmed
against 2.1.246 by reading the bundle: the file is rewritten from a React
effect whenever the derived status changes, and `claude agents --json` is a
reader of these same files that strips the most useful fields. Each record
carries:

| Field | Meaning |
| --- | --- |
| `pid`, `procStart` | the harness process and its kernel start time |
| `sessionId`, `cwd`, `name`, `kind` | identity; `kind` is `interactive` or `bg` |
| `status` | `busy` while loading or delegating; `waiting` while any blocking dialog is up (permission, `AskUserQuestion`, plan approval, MCP elicitation, sandbox or worker request, any open dialog); `shell` in `!` shell mode; otherwise `idle` |
| `waitingFor` | why it is waiting: `permission prompt`, `input needed`, `dialog open`, `sandbox request`, `worker request` |
| `statusUpdatedAt`, `updatedAt` | wall-clock milliseconds of the write |

A background thread lists the directory every 150 ms (500 ms over a WSL UNC
share) and re-reads the records only when a file's name, size, or mtime moved.
Dead processes are dropped by one long-lived prober: a `sh` running a short
script that answers each PID with its `/proc/<pid>/stat` start time or `-`.
The same script runs on Linux (`sh`) and inside the WSL distribution
(`wsl.exe --exec sh`), so both hosts check liveness through one code path.
It sweeps every known PID every 3 s and immediately when a new PID appears;
a reply that takes more than 2 s kills and respawns it, and a host without
`/proc` retires it. A record is alive only when the start time equals its
`procStart`, so a reused PID cannot vouch for a finished session. A record
whose liveness is unknown can still be matched by hook session ID, never by
cwd alone. When a resumed session leaves an older file with the same
`sessionId`, the newest write wins.

Precedence for a matched pane:

- A fresh `waiting` record wins over background work. `busy` proves parent
  activity. `idle` and `shell` describe only the parent and cannot finish
  independently running tasks.
- Hooks are scoped to a session. Parent edges, task edges, and complete
  inventories have separate causal watermarks, so a delayed task start is not
  discarded merely because a newer parent stop arrived first. Duplicate
  delivery is idempotent. Equal-time conflicting evidence cannot prove a task
  ended. A session switch/resume establishes a lifecycle fence.
- `background_tasks: []` proves an empty inventory at that observation.
  An absent field proves nothing; a malformed field records uncertainty.
  `SubagentStart`/`SubagentStop` address exact IDs, and a `SubagentStop`
  inventory cannot keep its own finishing ID alive.
- Running shells and helpers keep a yielded parent `Running`. Explicit
  ambient tasks do not count as blocking work. A monitor without ambient
  classification is uncertain rather than silently ignored; see the upstream
  [missing ambient metadata issue](https://github.com/anthropics/claude-code/issues/98816).
  Registered `session_crons` describe future wakeups, not current activity.
- The transcript observer accepts only session-correlated, system-origin
  task-notification envelopes with an exact task ID and terminal status.
  Ordinary transcript prose, user-pasted notifications, EOF, silence, and
  elapsed time never finish a task. Partial records and unavailable/rotated
  sources preserve uncertainty instead of guessing.
- Ctrl+C and `StopFailure` affect the parent, not independent children.
  A session end retires its identity without a success notification; known
  remaining tasks must settle before that retirement is finalized.
- `Done`, completion attention, finished desktop notifications, and PR refresh
  require the aggregate completion edge. They do not fire when the parent
  yields to a shell/helper, evidence is missing, or the same delivery repeats.

The hook client retains bounded typed identity/activity fields rather than
persisting arbitrary hook JSON. Its per-pane journal is under the control
registry's `claude-activity` directory. Event publication and checkpointing use
cross-process locking and durable writes; only exact checkpointed delivery IDs
and observed gap tokens are removed. A lost-event boundary invalidates older
inventories but cannot invalidate a genuinely newer complete inventory.
Durable state excludes assistant-message bodies and transient display copy.
Pane-process replacement retires the old activity rather than restoring it
into a new shell that happens to reuse the pane ID.

Use **Repair** or **Re-add** after upgrading when the installed hooks are
outdated, then restart Claude Code to reload its configuration. Legacy clients
and Claude versions without task inventories can still report parent activity,
but an idle parent is not sufficient to claim all work is complete.

OSC titles, progress lines, footers, and composers remain identity and limited
fallback evidence. They cannot erase the task ledger or clear its uncertainty.

## Benefits

- Automatic approvals never flash or accumulate false human attention.
- Manual approvals still turn amber from the UI the user can actually act on.
- Claude reports the parent and its independent work together, while preserving
  the priority of an actual blocking dialog.
- Missing evidence is visible as neutral `Unknown`, not false `Idle`, `Done`,
  or `Needs input`. Fresh authoritative evidence resolves it.
- Oh My Pi's active lifecycle and Pi's exact extension contract are unchanged.
- Historical transcript questions and parallel tool completions cannot own the
  current attention state.
- Hooks remain useful for pane/session attribution and terminal-independent
  completion events.
- The screen classifier and structured-record matcher are deterministic and
  covered by headless unit and native application tests.

## Costs and limitations

- Agent UI text can change. Conservative rules then produce a false negative
  (`Running`/`Idle` or the prior state) until Muxtrix is updated, rather than a
  false `Needs input`.
- The initial rules are English-only and embedded in the binary. Muxtrix does
  not yet have versioned remote manifests, local overrides, or an explain
  command.
- OSC title evidence is strongest but may be disabled or changed by an agent;
  the screen fallbacks cover only known UI shapes.
- Muxtrix classifies its current Ghostty render snapshot rather than reading
  the live bottom buffer independently of a user's scrolled viewport.
  Codex and Claude normally own the alternate screen, but a future detector
  should expose an explicit unscrolled live-bottom snapshot before expanding
  this to primary-screen programs.
- A brand-new prompt shape may not raise attention. The terminal itself remains
  fully usable and visible; only the sidebar projection can be incomplete.
- A Pi pane without the managed module has no state source: a launched Pi
  stays `Running` until the module is installed and Pi reloads it. Pi's
  built-in dialogs that bypass its extension UI (the project trust prompt)
  are not reported, and releases older than 0.84.4 never emit the prompt
  events at all.
- Claude's parent record and system task-notification envelope are internal
  formats, checked against 2.1.246 and 2.1.289 respectively. Unsupported
  formats, unreadable sources, and journal gaps can produce `Unknown`;
  Muxtrix does not claim exact state where the harness supplies no evidence.
  Ambiguous session/PID/cwd matches are ignored rather than guessed.
- A host without `/proc` (macOS, native Windows) cannot probe liveness; a
  stale file there is excluded only by hook identity, and cwd-only matching
  is off.
- A Windows host reads a WSL distribution's records over `\\wsl.localhost`
  once hook discovery has resolved that distribution's home, and keeps one
  hidden `wsl.exe` prober alive while any record exists.
- The task observer reads only the hook-provided transcript path and strict
  task-notification envelopes. It is not a general transcript or output parser.
  Unsupported completion formats remain uncertain until a newer complete
  inventory or another exact terminal observation resolves them.
- Direct Codex app-server events could distinguish automatic review from an
  explicit approval request exactly. Adopting them would require a supported
  ownership/transport boundary for sessions launched as ordinary terminal
  programs and an equivalent strategy for Claude Code.

## Validation contract

Regression coverage pins the original five attention cases:

1. repeated Codex `PermissionRequest` / `PostToolUse` automatic-review cycles
   never create unread attention;
2. a recognized visible prompt creates `Needs input`;
3. a late `PostToolUse` cannot clear that visible prompt;
4. a subsequent working screen frame clears it;
5. an Oh My Pi idle title cannot override an active lifecycle, while the same title
   can still clear a stale screen- or process-detected `Running` state.

Pi fixtures pin the exact-lifecycle contract: a launched pane accepts Pi's
idle `session_start`, `ui_prompt_start` raises attention without any screen
evidence and `ui_prompt_end` clears it, only `agent_settled` completes the
turn, a settled error fails the pane with Pi's message, and a legacy Oh My Pi
module reporting as `pi` keeps its Oh My Pi identity.

Claude fixtures cover independent parent/task ordering, equal timestamps,
duplicates, missing/malformed/empty inventories, mixed helpers and shells,
silent exact task completion, interruption/failure, real input waits, ambient
monitors, scheduled wakeups, session switches/resume, and bounded durable
history. Offline hook tests launch the real `muxtrixctl` executable with an
isolated registry and exercise checkpoint/delivery races. Application cases
cover causal journal gaps, stale identity rejection, pane-process retirement,
and completion side effects. Record matching still rejects ambiguous cwd,
requires confirmed liveness for unique cwd, and accepts exact PID.

Headless `claude-background-work`, `claude-activity-unknown`, and
`claude-activity-recovery` scenarios exercise the production application.
Recovery includes an offline hook, transcript-only shell completion, actual
GUI process replacement, and reattachment to the original session daemon.
