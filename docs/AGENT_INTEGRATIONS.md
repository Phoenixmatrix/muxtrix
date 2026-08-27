# Agent integrations

Muxtrix can launch Codex, Claude Code, and Oh My Pi in independent terminal
panes, track their lifecycle state, and surface waiting/completed events directly in the
originating fleet entry.
Integration is opt-in and reversible.

## Reversible hook contract

Every installed command carries the marker `muxtrix-hook-v1`. Muxtrix removes
only handlers containing that marker; unrelated settings, hooks from other
tools, and configuration added after installation are preserved. Repeated
`add` is idempotent, and `re-add` performs selective removal followed by a
fresh install.

Installation identity includes the executable path. If managed entries still
point at an older build or installation, status reports them as not installed
for the current client and Add/Re-add replaces only those stale Muxtrix
entries. Status also compares every managed event command with the current
lifecycle contract, so an upgrade that changes state semantics is repairable
even when the executable path remains stable.

A normal installed instance may re-point path-only stale hooks during
background discovery. Isolated instances are read-only during that discovery:
an app using `MUXTRIX_NO_SESSIOND`, `MUXTRIX_E2E_REPORT`, or a custom
`MUXTRIX_CONTROL_ENDPOINT` never rewrites the user's hooks for its temporary
executable. Explicit Add, Re-add, and Repair actions remain available in those
instances.

Status also reads the executable back out of each managed command and checks it
is on disk. Hooks naming a `muxtrixctl` that is gone match their own text
perfectly while delivering nothing — the agent's spawn simply fails and the pane
stops changing state — so they are reported as needing repair rather than as
installed.

A build that has no `muxtrixctl` beside it therefore never claims the user's
hooks: it neither re-points them during discovery nor installs its own on
demand, and Add and Re-add fail with the path they would have written instead of
removing what works. Muxtrix derives that path from its own location, so this is
a guess worth checking; a path the caller names — `--hook-command`, or a WSL
distribution's own translation of `muxtrixctl.exe`, which no Windows stat can
see — is taken on trust.

The Settings card calls all of this **Needs repair** and offers a one-click
**Repair** action, naming whether the hooks point at another Muxtrix binary or
at one that is missing. Muxtrix also raises a global alert so stale hooks cannot
look like a healthy integration. This is especially important after moving from
a development checkout to Scoop, because the hook command must follow the active
`muxtrixctl.exe` installation.

Before the first edit Muxtrix keeps a private pre-change backup and installation
record in its platform state directory. For Codex and Claude Code, cleanup is
selective rather than a whole-file restore so later edits cannot be overwritten.
For Oh My Pi, Muxtrix installs one auto-discovered extension module and removes
or restores that file as a unit. If Muxtrix created the extension file and its
parent directories, uninstall removes them. Invalid JSON in JSON-backed hook
files is reported and never overwritten or backed up.

Existing file permissions survive rewrites. New configuration and backup files
are private to the current user on Unix.

## UI controls

Open Settings, then use the Agent lifecycle card:

- **Add** installs user-level lifecycle hooks.
- **Remove** uninstalls only Muxtrix-managed handlers.
- **Re-add** refreshes the managed commands without disturbing other tools.
- **Launch** opens an independent pane and starts the configured agent command.

The Codex, Claude Code, and Oh My Pi command fields are persistent settings.
Once an agent's user-level integration is installed, it can also be selected as
the **Default worktree agent** in the same card. The command palette then offers
matching worktree actions that open a pane right or down, restart the current
pane in a new worktree, or restart it in an existing registered worktree, and
start that agent as soon as the terminal is ready. The default stops being
eligible if its hooks are removed or need repair. Selecting one of these
commands without an eligible default explains the requirement and links
directly to the agent settings. Muxtrix keeps that exact command pending while
Settings is open; choosing **Apply and continue** resumes it after setup.

The launch and worktree-agent actions are available from Ctrl+P on Windows/Linux
and Cmd+P on macOS.

Adding or re-adding user-level Codex hooks also marks the shared
`~/.muxtrix/worktrees` parent as trusted in `~/.codex/config.toml`. Codex can
then trust every linked checkout Muxtrix creates without accumulating a
separate project entry for each worktree. Existing configuration, comments,
and file permissions are preserved. When Windows Muxtrix manages agents in
WSL2, the trust entry uses the Linux-side home path that Codex sees.

## CLI controls

The CLI supports user and project scope:

```sh
muxtrixctl hooks status all
muxtrixctl hooks add codex --scope user
muxtrixctl hooks add claude --scope project --project /path/to/repository
muxtrixctl hooks add pi --scope user
muxtrixctl hooks remove all --scope user
muxtrixctl hooks re-add codex --scope user
```

`reinstall` is accepted as an alias for `re-add`. Status reports the target,
managed entry count, and whether the recovery backup is present.

Targets are:

| Agent | User scope | Project scope |
| --- | --- | --- |
| Codex | `~/.codex/hooks.json` | `<project>/.codex/hooks.json` |
| Claude Code | `~/.claude/settings.json` | `<project>/.claude/settings.local.json` |
| Oh My Pi | `~/.omp/agent/extensions/muxtrix-lifecycle.ts` | `<project>/.omp/extensions/muxtrix-lifecycle.ts` |

Muxtrix subscribes to session start/end, prompt submission, permission, stop,
and sub-agent events for Codex and Claude Code. Claude Code also contributes
stop-failure, elicitation, and notification events, and its hook client
forwards the whole payload rather than a pre-decided state. Oh My Pi does not use Codex/Claude-style
JSON hook arrays; its managed module is a native `.omp` extension that listens
to session lifecycle, agent lifecycle, approval, compaction, and handoff
maintenance events. Hooks identify the pane, session, working directory, and
coarse turn boundaries. Completion and failure still create attention on the
originating fleet entry.

For Codex, hooks are not the authority for live interactive state: it emits
`PermissionRequest` before its automatic reviewer decides, and `PostToolUse`
arrives only after a successful tool finishes. Muxtrix treats those callbacks
as metadata and derives `Running`, `Idle`, or `Needs input` from the current
terminal screen and OSC title. Only a recognized, visible approval or answer
surface can create `Needs input`. `Done` preserves the previous completion
while the composer is idle, but positive working evidence starts the next turn
even when its prompt hook was delayed or unavailable.

Claude Code is read from the session record the harness itself writes
(`~/.claude/sessions/<pid>.json`): `busy`, `idle`, `waiting` with the reason,
or `shell`. Hooks add the exact turn edges and session identity; the screen is
only a fallback for a pane no live record matches. See
[Agent state detection](AGENT_STATE_DETECTION.md#claude-code-session-records).

Oh My Pi keeps exact approval and active-turn lifecycle events. From
`agent_start` through terminal `agent_end`, an idle title cannot demote the
fleet row; this covers Pi releases that briefly paint `π >` while async work or
a scheduled continuation still owns the turn. Outside that bracket, Pi's
documented OSC title state (`π >` idle, `π ⠋` working, `π !` attention, with
all spinner frames accepted) remains the live correction and reattach layer.
Maintenance completion stays `Running` and does not masquerade as a completed
turn. Outdated managed Pi extensions migrate automatically when Muxtrix
synchronizes hook status; re-add remains the explicit repair path. See
[Agent state detection](AGENT_STATE_DETECTION.md) for precedence and limits.

Typing a configured `codex`, `claude`, or `omp` launch command assigns that
pane's agent before any managed hook or extension callback can arrive, which
makes the fleet identity useful before the first callback. It is deliberately
pane-local. Linux process-tree detection can also identify configured agent
executables while hooks are absent. Claude's structured records associate by
hook session ID, exact Linux process PID, or a cwd that is unique on both sides;
ambiguous records never repaint a pane. Oh My Pi's approval extension events
are exact observability events, so they can report `Needs input` immediately
without waiting for the matching title paint.

Agent pane names follow the same pane-local rule. A user rename wins first,
then a meaningful terminal title emitted by the harness (including a named
Claude Code session, Codex thread, or the label after Oh My Pi's `π <state>`
prefix), then the linked-worktree directory name, and finally `Codex`,
`Claude Code`, or `Oh My Pi`. Brand-only terminal titles do not replace the
worktree fallback. Animated Pi state separators never become pane identity.
Long names stay on one line and are ellipsized before the separate lifecycle-state column.

Codex reviews newly discovered hook commands by exact command hash. After an
install or re-add, use Codex `/hooks` to review or trust the generated commands
when prompted. Claude Code exposes its active configuration through `/hooks`.
Oh My Pi auto-discovers the managed extension from its native extension
directory and reports session start/switch/branch, prompt execution, completion,
shutdown, and tool-approval request/resolution events to Muxtrix. The extension
does not write a Muxtrix status into Pi's own footer.

## Windows Muxtrix with agents in WSL2

Select WSL and its distribution in Settings before using the lifecycle hook
controls. Muxtrix discovers that distribution's default Linux user/home, writes
the Linux-side Codex and Claude configuration through `\\wsl.localhost`, and
embeds the current Windows `muxtrixctl.exe` as a WSL-visible `/mnt/...` command.
Add, Remove, Re-add, and status therefore operate on the same configuration as
the agents running in that pane. Re-add is the safest first action after moving
or upgrading a development binary.

Distribution enumeration is native and does not launch `wsl.exe`. The selected
distribution's name, Linux home, and WSL-visible executable path are resolved
in one hidden background WSL call and cached for the rest of the application
process. Opening Settings uses the cached state and never waits for discovery.

The lifecycle process must call the Windows control client so it can reach the
Windows app's named pipe. For manual or project-scoped setup, a Linux
`muxtrixctl` can still edit the Linux-side agent config while overriding the
command embedded in each hook:

```sh
./target/debug/muxtrixctl hooks add all --scope user \
  --hook-command "/mnt/c/Program Files/Muxtrix/muxtrixctl.exe"
```

Use the actual WSL-visible path to the installed Windows executable. Windows
executables require the `.exe` suffix under WSL. Muxtrix's WSL launch plan
shares pane identity through `WSLENV`, so the callback is attributed to the
correct pane. A custom control endpoint is shared the same way.

Removal does not need the old executable path because it matches the Muxtrix
marker:

```sh
muxtrixctl hooks remove all --scope user
```

To move or replace the Windows binary, refresh commands explicitly:

```sh
muxtrixctl hooks re-add all --scope user \
  --hook-command "/mnt/c/new/path/muxtrixctl.exe"
```

This bridge uses standard WSL process interoperability. It does not install a
daemon, edit shell startup files, or create a cross-VM TCP listener.

## Failure behavior

Hook callbacks have a short timeout, ignore an unavailable Muxtrix app, and do
not block the agent. The internal `hook-event` command always emits an empty
JSON response required by hook consumers that inspect stdout. Lifecycle state
is advisory; terminal operation remains independent if hooks are removed.
