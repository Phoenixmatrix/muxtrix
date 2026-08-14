# Agent state detection

## Decision

For Codex and Claude Code, the live terminal screen is authoritative for
`Running`, `Idle`, and `Needs input`. Oh My Pi supplies lifecycle state through
its managed extension; Muxtrix also recognizes its `π` terminal title as pane
identity. Oh My Pi approval events are observability-only signals emitted after
the agent has decided a tool really needs approval, so they may create and clear
`Needs input`. Other lifecycle hooks still identify the agent, session, working
directory, prompt submission, completion, and shutdown, but a permission or
notification hook is not allowed to create human attention.

The invariant is intentionally strict:

> `Needs input` requires positive, currently visible approval, question, or
> permission UI in that pane.

An unrecognized screen preserves the last trusted state. This favors a missed
new prompt over repeatedly telling the user that an automatic reviewer needs
them.

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

The implemented change adopts a screen-authoritative, conservative matching
model: session-integration hooks are separated from state authority, and
`Needs input` requires visible evidence on the live screen.

## Implemented model

On each terminal poll, the application evaluates each pane's latest Ghostty
grid snapshot. Retained frames are re-evaluated so an identity hook arriving
just after a stable prompt paint cannot miss it on the next poll. The screen
classifier runs for panes already identified as Codex or Claude Code. Oh My Pi
has no screen-state classifier yet; ordinary Pi frames preserve the last hook or
process state, while its managed extension reports exact lifecycle,
session switch/branch, approval-request transitions, and context
compaction/handoff maintenance.

- Codex `Action Required` OSC titles and strong live confirmation/answer forms
  create `Needs input`.
- Codex spinner titles or its bottom `Working (... esc to interrupt)` footer
  create `Running`; a plain nonempty Codex title supplies idle evidence.
- Claude spinner titles and its active `/btw` overlay create `Running`.
- Claude confirmation/navigation forms and dynamic-workflow prompts create
  `Needs input`; its idle OSC title creates `Idle`.
- A Claude frame showing the Agents view returns no classification at all, and
  is evaluated before every rule below it. The roster draws its own composer and
  its own spinner-free title, either of which a later rule would otherwise read
  as this conversation's state.
- Claude's rendered composer — a `❯` line inside the last pair of horizontal
  rules, with no menu over it — creates `Idle`. It is ranked below every
  blocking rule so it can never clear a wait that is still painted, and it
  ignores a `❯ 1. Yes` answer line.
- Transcript viewers return no classification so historical text cannot
  repaint the pane.
- Loose prose such as "do you want to proceed?" is insufficient without the
  accompanying form controls.
- Retained idle evidence may initialize a detected agent or resolve a visible
  wait, but the exact frame retained when `UserPromptSubmit` arrives cannot
  regress that newer running state. Muxtrix records the frame revision at the
  transition; a subsequently rendered idle frame can resolve `Running`. This
  guards the hook/frame race without leaving a visibly idle pane stuck green.
- A completed turn remains `Done` while its idle composer is visible, preserving
  the useful completion signal. Strong working evidence starts the next turn
  even if `UserPromptSubmit` was lost, so `Done` cannot become a permanent latch
  when hook delivery is unavailable.

The typed control event now carries the original hook event name. This is a
backward-compatible optional field. Muxtrix treats supported-agent waiting
hooks as metadata only. A `PostToolUse` can update metadata or ordinary running
state, but cannot clear a screen-confirmed wait. Completion, failure, stop, and
new prompt lifecycle events retain their coarse roles.

Codex and Claude Code keep their existing hook set and wire state names. Pi
installations whose managed extension lacks maintenance events are repairable
through the normal hook status/re-add flow.

## Recovery across session reattach

Agent identity is part of the daemon-owned serialized pane layout. A new
Muxtrix instance restores that identity before attaching the pane's byte
stream, then applies the same screen-authoritative classifier to the terminal
grid rebuilt from backlog replay. Current `Running`, `Idle`, or `Needs input`
therefore does not depend on an already-running agent emitting a new hook into
the new application instance.

Layouts created before durable identity was added remain recoverable. Once the
replayed grid arrives, Muxtrix accepts only agent-specific signatures: Codex's
composer, working footer, approval forms, or branded title; Claude Code's
prompt box, Agents view, or branded title; and Oh My Pi's `π`/`π: <title>`
terminal title. A generic title or spinner is not enough to invent an agent.
The recovered identity is written into the next layout update, so this fallback
is normally needed only once.

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
`blocked` and `failed` from `working` and `done`. That read costs a short-lived
subprocess (~0.25 s), so it runs off the UI thread, at most one at a time, at
most every two seconds, and only while a pane is actually projecting the
roster — immediately on entering it. Leaving the roster needs no read at all:
the conversation's own screen evidence is available on the next frame.

`--json` reports more than the view does: every interactive Claude Code on the
machine appears in it with `"kind": "interactive"`, while the Agents view lists
background sessions only. Counting the interactive ones made the row disagree
with the screen beside it — a machine whose view read `1 working · 3 completed`
rolled up as `1 working · 5 idle` — and counted the very session doing the
viewing. The roll-up therefore skips `interactive` entries, and keeps entries
whose `kind` it does not recognise so a field that disappears degrades to
over-reporting rather than to an empty roster. `done` is counted and named
`completed`, matching the harness's own vocabulary for a finished session.

## Benefits

- Automatic approvals never flash or accumulate false human attention.
- Manual approvals still turn amber from the UI the user can actually act on.
- State clears as soon as a working/idle frame is rendered instead of waiting
  for tool completion.
- Historical transcript questions and parallel tool completions cannot own the
  current attention state.
- Hooks remain useful for pane/session attribution and terminal-independent
  completion events.
- The classifier is deterministic, offline, and covered by headless unit and
  native application tests.

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
- Rollout parsing could later add tool names, subagent activity, transcript
  recovery, and richer summaries. It should remain descriptive metadata unless
  records are request-correlated and prove that a person currently has an
  actionable prompt.
- Direct Codex app-server events could distinguish automatic review from an
  explicit approval request exactly. Adopting them would require a supported
  ownership/transport boundary for sessions launched as ordinary terminal
  programs and an equivalent strategy for Claude Code.

## Validation contract

Regression coverage pins four cases:

1. repeated Codex `PermissionRequest` / `PostToolUse` automatic-review cycles
   never create unread attention;
2. a recognized visible prompt creates `Needs input`;
3. a late `PostToolUse` cannot clear that visible prompt;
4. a subsequent working screen frame clears it.

Classifier fixtures also cover OSC precedence, transcript-viewer suppression,
Claude blocked/working/idle forms, and rejection of narrative questions.
