# Agent state detection

## Decision

Codex uses its live terminal screen for `Running`, `Idle`, and `Needs input`.
Claude Code is described by Claude Code itself: the harness writes a session
record (`~/.claude/sessions/<pid>.json`) from its own UI state on every change,
naming whether it is `busy`, `idle`, `waiting` on a dialog, or in `shell` mode.
Muxtrix reads that record directly, lets its hooks supply the exact turn edges
and identity, and consults the screen only for a pane no live record could be
matched to. Oh My Pi supplies exact
active-turn and approval transitions through its managed extension, while OMP's
documented state-bearing OSC title supplies the correction and recovery layer.
`π >` means idle, `π !` means attention, and `π` followed by a supported Braille
spinner means working; ConPTY uses the static working form `π :`.

Other lifecycle hooks still identify the agent, session, working directory,
prompt submission, completion, and shutdown. Codex permission and notification
hooks are not allowed to create human attention because its harness may resolve
those requests automatically. Claude Code's `PermissionRequest` fires only when
a dialog is actually shown, so it is exact — and the session record confirms or
clears it within milliseconds either way. Pi's `agent_start` through
terminal `agent_end` interval and its approval events are exact. During that
interval, an idle title cannot demote the pane; this covers older OMP releases
that briefly published `π >` while an async job or scheduled continuation still
owned the turn.

An unrecognized Codex screen or title preserves the last trusted state. Claude
can additionally recover from its exact structured session record. This favors
a missed new prompt over inventing activity or repeatedly telling the user that
an automatic reviewer needs them.

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

## Implemented model

On each terminal poll, the application evaluates each pane's latest Ghostty
grid snapshot and OSC title. Retained frames are re-evaluated so an identity
hook arriving just after a stable prompt paint cannot miss it on the next poll.
Codex uses its live screen and title. Claude Code uses its session record and
hooks, with the screen as the fallback for an unmatched pane. Oh My Pi uses its state-bearing title
except that its exact `agent_start` through terminal `agent_end` lifecycle
bracket prevents an idle title from ending active work. Pi also retains exact
session switch/branch, approval-request, context compaction/handoff, and
shutdown events from the managed extension.

- Codex `Action Required` OSC titles and strong live confirmation/answer forms
  create `Needs input`.
- Codex spinner titles or its bottom `Working (... esc to interrupt)` footer
  create `Running`; a plain nonempty Codex title supplies idle evidence.
- Claude spinner titles and its active `/btw` overlay create `Running`.
- Claude confirmation/navigation forms and dynamic-workflow prompts create
  `Needs input`; its idle OSC title creates `Idle`.
- Oh My Pi's `π >` title creates `Idle` outside an active lifecycle bracket,
  `π !` creates `Needs input`, and its ten supported Braille separators create
  `Running`. `π :` is the static ConPTY working form. A state-disabled
  `π: <label>` title identifies Pi but does not invent a state.
- A Claude frame showing the Agents view returns no classification at all, and
  is evaluated before every rule below it. The roster draws its own composer and
  its own spinner-free title, either of which a later rule would otherwise read
  as this conversation's state.
- Claude's rendered composer — a `❯` line inside the last pair of horizontal
  rules, with no menu over it — creates `Idle`. It is ranked below every
  blocking rule so it can never clear a wait that is still painted, and it
  ignores a `❯ 1. Yes` answer line.
- Claude's session records are associated one-to-one by hook session ID, then
  by the exact harness PID (the pane's process tree on Linux, or the ancestry
  of the hook command that just called in), then by a cwd that is unique on
  both sides. Ambiguous records are ignored. While a live record is matched,
  the screen classifier has no authority over that pane at all.
- Transcript viewers return no classification so historical text cannot
  repaint the pane.
- Loose prose such as "do you want to proceed?" is insufficient without the
  accompanying form controls.
- Retained idle evidence may initialize a detected agent or resolve a visible
  wait, but the exact frame retained when `UserPromptSubmit` arrives cannot
  regress that newer running state. Muxtrix records the frame revision at the
  transition; a subsequently rendered idle frame can resolve `Running` unless
  Pi's exact active-lifecycle bracket is still open.
- A completed turn remains `Done` while its idle composer is visible, preserving
  the useful completion signal. Strong working evidence starts the next turn
  even if `UserPromptSubmit` was lost, so `Done` cannot become a permanent latch
  when hook delivery is unavailable. Pi maintenance completion remains
  `Running`; only terminal `agent_end` completes its active turn.

The typed control event carries the original hook event name. Codex and Claude
waiting hooks remain metadata only, and `PostToolUse` cannot clear their
screen-confirmed wait. Pi approval events and active-turn lifecycle brackets
remain exact state transitions. Completion, failure, stop, and new-prompt
lifecycle events retain their coarse roles for every supported harness.

The managed Pi extension is versioned. Existing modules without the current
behavior marker are migrated during normal Muxtrix hook synchronization, while
the explicit hook re-add path remains available. Migration preserves the
original uninstall backup and removes the old Pi-footer status writes.

## Recovery across session reattach

Agent identity is part of the daemon-owned serialized pane layout. A new
Muxtrix instance restores that identity before attaching the pane's byte
stream, then applies the screen classifier to the terminal grid rebuilt from
backlog replay. Claude additionally re-associates its structured session by
unique cwd when neither a new hook nor a host-visible process PID is available.
Current state therefore does not depend on an already-running agent emitting a
new hook into the replacement application instance. A Claude pane additionally
re-matches its session record by PID or unique cwd as soon as the watcher's
first read lands, so its state is exact again without any repaint.

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
Dead processes are dropped first: on Linux, `/proc/<pid>/stat` must exist and
its start time must equal `procStart`, so a reused PID cannot vouch for a
finished session. Elsewhere liveness is unknown and a record must be vouched
for by hook session ID or a cwd that is unique on both sides. When a resumed
session leaves an older file with the same `sessionId`, the newest write wins.

Precedence for a matched pane:

- The record decides `Running` (`busy`), `Needs input` (`waiting`, with
  `waitingFor` as the row's activity), and `Idle` (`idle`, `shell`). An `idle`
  record after a turn ran is that turn finishing, so it reads as `Completed`
  until the next `busy`; `Failed` likewise persists until the next turn.
- Hooks are exact edges applied immediately: `UserPromptSubmit` starts the
  turn, `Stop` completes it (and triggers the PR refresh), `StopFailure` fails
  it, `Elicitation` blocks it, `SessionStart` resets it, `SessionEnd` removes
  it. `PermissionRequest` and `SubagentStart` are advisory while a record is
  matched: the first fires before another hook or auto mode may resolve the
  request without a dialog, and the second fires for background subagents
  after the turn has stopped; the record already answers both. A record whose
  `status` this build cannot read leaves the screen in charge. A `Notification` counts only when it names
  `permission_prompt` or an elicitation dialog; the harness sends those after a
  dialog has waited about six seconds, so the record has long since said so.
- A record stamped earlier than the last hook edge cannot regress it: the
  prompt hook can land a few milliseconds before the harness rewrites `busy`.
  Both are stamped from the same wall clock.
- A pane whose record disappears or whose process dies falls back to hook
  edges and the screen classifier until a record matches again.

The hook client forwards the payload intact (event name, session, cwd, tool,
notification type, permission mode, message) as a typed `ClaudeHook` request
instead of a pre-decided state, and adds its own parent PID: the hook process
is still alive waiting for the reply, so the app can read the ancestry up to
the harness and match the record by PID even when the pane's process tree is
hidden. A hook client from before this contract is folded into the same
pipeline from its coarse event name.

Every prior Claude signal is now demoted: the OSC title spinner (which the
harness makes static under a multiplexer anyway), the `esc to interrupt`
footer, the progress line, and the composer are identification and last-resort
fallbacks. `Needs input` no longer depends on recognising a dialog's text.

## Benefits

- Automatic approvals never flash or accumulate false human attention.
- Manual approvals still turn amber from the UI the user can actually act on.
- Claude's row is whatever Claude Code itself says it is, including every
  blocking dialog the harness can raise, within one watcher tick or one hook.
- State clears from newer screen evidence (Codex) or the harness's own record
  (Claude); Pi's active lifecycle remains the exception.
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
- Claude's session record is an internal file whose shape is confirmed on
  2.1.246, not a documented contract. An unreadable directory or a changed
  schema degrades to hooks plus the screen classifier, never to an invented
  state. Ambiguous session/PID/cwd matches are ignored rather than guessed.
- Outside Linux the record's process liveness is unknown; a stale file for a
  finished session can only be excluded by hook identity or cwd uniqueness.
- A Windows host reads a WSL distribution's records over `\\wsl.localhost`
  once hook discovery has resolved that distribution's home.
- Rollout parsing could later add tool names, subagent activity, transcript
  recovery, and richer summaries. It should remain descriptive metadata unless
  records are request-correlated and prove that a person currently has an
  actionable prompt.
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
5. a Pi idle title cannot override an active lifecycle, while the same title
   can still clear a stale screen- or process-detected `Running` state.

Claude fixtures pin the record contract: a live `busy` record decides the pane
over its painted idle composer and the screen stays silent while matched; a
`waiting` record raises attention that the next `busy` clears; a lost record
returns authority to the screen; a hook edge leads and a record stamped before
it cannot regress it; `SessionEnd` removes the pane; and ambiguous cwd matches
are rejected while exact PID and hook-ancestry matching succeed. Parser
fixtures use a verbatim 2.1.246 record and a verbatim `/proc` stat line.
