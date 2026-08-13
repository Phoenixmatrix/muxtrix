# Terminal host resilience plan

Status: planned

Last updated: 2026-08-12

## Decision summary

Muxtrix must remain a usable workspace manager when a terminal process backend
is unavailable, stalled, or under severe resource pressure. Creating or
selecting a workspace, tab, pane, settings section, session-management view, or
diagnostic action must never wait for WSL, ConPTY, a PTY, Git, a session daemon,
or a blocking IPC write.

The workspace model is authoritative. A terminal process is an asynchronous
capability attached to a pane, not a prerequisite for the pane or application
shell to exist. A pane whose process cannot start remains navigable and offers
recovery actions in place.

Muxtrix will not automatically run `wsl --shutdown`, kill unrelated WSL
processes, or repeatedly retry a failing backend. Those actions can disrupt
work outside Muxtrix and are not safe recovery defaults.

## Incident and problem statement

The motivating incident occurred while WSL was out of memory and paging heavily.
Opening a workspace, tab, or pane appeared to freeze Muxtrix. A new Muxtrix
instance was not a viable escape hatch because startup selected the same sick
backend and could stall before the application became usable.

The failure is backend-agnostic. WSL made it visible, but the same contract must
cover a slow native shell, ConPTY or PTY creation, a stuck session daemon, a
blocked local socket or named pipe, an inaccessible working directory, slow
Git discovery, and future SSH or remote process hosts.

The required outcome is graceful degradation, even when it is literally
impossible to open a terminal:

- Muxtrix renders and accepts navigation immediately.
- Creating a pane, tab, or workspace produces visible UI immediately.
- Healthy panes and application sections remain interactive.
- The affected pane communicates truthful progress or failure.
- The user can cancel, retry, change backend, collect diagnostics, or close the
  pane without restarting Muxtrix.
- Repeated failures do not create an unbounded number of processes, threads,
  requests, or buffered output chunks.

## Goals

1. No external process, filesystem, or IPC operation may block Iced's update or
   rendering thread.
2. The first usable application frame must not depend on a session daemon or
   terminal process being ready.
3. Workspace, tab, and pane creation must commit local UI state before process
   launch begins.
4. One stuck launch must not delay input, resize, close, persistence, or output
   for another pane.
5. Launch state and errors must be explicit, correlated to one attempt, and
   visible without enabling the optional status bar.
6. Cancellation, timeout, and retry must be race-safe: a late result may not
   attach to a cancelled pane or replace a newer attempt.
7. Backpressure, concurrency limits, and a circuit breaker must prevent a sick
   host from making memory pressure worse.
8. All behavior must be testable headlessly with deterministic fake backends.

## Non-goals

- Repairing WSL, reclaiming host memory, or diagnosing the user's Linux
  workload automatically.
- Guaranteeing that an operating-system process creation call can be forcibly
  cancelled from a Rust thread.
- Killing a distribution or unrelated processes as an automatic recovery step.
- Replacing session persistence or changing the workspace -> tabs -> panes
  hierarchy.
- Treating a resource heuristic as authoritative proof that a backend will or
  will not launch.
- Building SSH transport as part of this work.

## Current failure chain

### Startup waits on backend work

`Muxtrix::boot` starts and connects to a session daemon before returning the
application. `Muxtrix::new` then launches the initial terminal before the first
normal frame. Session-host startup includes process creation, a readiness loop,
and a connection handshake. If daemon startup fails, terminal launch falls back
to an in-process PTY, putting the potentially stuck operation back into the GUI
startup path.

Relevant implementation:

- [`Muxtrix::boot` and `Muxtrix::new`](../crates/muxtrix-app/src/main.rs)
- [`start_session_host`](../crates/muxtrix-app/src/main.rs)
- [`wait_until_ready`](../crates/muxtrix-sessions/src/daemon.rs)
- [`start_live_session`](../crates/muxtrix-app/src/main.rs)

### User actions launch synchronously

Split, new-workspace, new-tab, worktree-terminal, and restart paths call
`TerminalRuntime::launch` directly from Iced message handling. Their model
mutation order is inconsistent:

- A split is inserted before launch.
- A new workspace or tab is launched before insertion.
- A restart replaces the runtime only after the replacement launch returns.

All three should instead commit a pending pane state first and request launch
afterward.

Relevant implementation:

- [`split_terminal`](../crates/muxtrix-app/src/main.rs)
- [`create_workspace`](../crates/muxtrix-app/src/main.rs)
- [`new_tab`](../crates/muxtrix-app/src/main.rs)
- [`restart_pane`](../crates/muxtrix-app/src/main.rs)

### Live-session readiness and teardown can wait forever

`LiveSession::spawn_source` creates a worker and then calls a blocking
`ready_receiver.recv()` without a deadline. In the local-backend path, the worker
does not report ready until PTY and child-process creation complete.

`LiveSession::Drop` sends shutdown and joins the worker. If that worker is
blocked in process creation, IPC, input, resize, or another backend operation,
closing or replacing a pane can freeze the caller too.

Relevant implementation:

- [`LiveSession::spawn_source`](../crates/muxtrix-terminal/src/lib.rs)
- [`run_live_session`](../crates/muxtrix-terminal/src/lib.rs)
- [`LiveSession::stop` and `Drop`](../crates/muxtrix-terminal/src/lib.rs)

### The session daemon serializes process creation with all control traffic

The daemon accepts and processes requests on one loop. `Request::Spawn` calls
`PtySession::spawn` inline. A stuck spawn prevents that loop from reading later
input, resize, layout, rename, kill, detach, or shutdown requests.

The client writes requests with blocking `write_all` while holding the writer
mutex. Some calls, including layout synchronization, originate on the UI thread.
If the daemon is not reading, the application can eventually block behind the
stalled request loop.

Relevant implementation:

- [Daemon request loop](../crates/muxtrix-sessions/src/daemon.rs)
- [Inline daemon spawn](../crates/muxtrix-sessions/src/daemon.rs)
- [`SessionClient::send`](../crates/muxtrix-sessions/src/lib.rs)
- [`sync_session_layout`](../crates/muxtrix-app/src/main.rs)

### Launch acknowledgement is not truthful enough

The GUI treats a remote `LiveSession` actor as successful as soon as the local
actor thread has attached to its byte channel. That does not mean the daemon
created the requested child. `Spawned` is retained only as process metadata;
`SpawnFailed` is reduced to an unclean exit and its useful error text is lost.
There is no attempt identifier, accepted acknowledgement, timeout event, or
cancel request.

Relevant implementation:

- [`TerminalRuntime::launch`](../crates/muxtrix-app/src/main.rs)
- [Session event handling](../crates/muxtrix-sessions/src/lib.rs)

### Other sections can invoke the same sick host

Session discovery and resume synchronously probe local IPC. Worktree creation
and management synchronously inspect repositories and can execute `wsl.exe` or
Git while opening the surface. Settings integration discovery already uses a
background task and is a useful incumbent pattern, though it still needs
deadlines and cancellation.

Relevant implementation:

- [`open_session_picker`](../crates/muxtrix-app/src/main.rs)
- [`resume_session`](../crates/muxtrix-app/src/main.rs)
- [`open_worktree_manager`](../crates/muxtrix-app/src/main.rs)
- [`git_repository_root`](../crates/muxtrix-app/src/main.rs)
- [`perform_blocking`](../crates/muxtrix-app/src/main.rs)

## Product and interaction contract

### Pane launch state

Terminal runtime state should be explicit and separate from the serializable
pane model. A representative state machine is:

```text
Pending
  -> Starting(attempt_id, backend, started_at)
       -> Running(session)
       -> Failed(attempt_id, kind, detail, retry_after)
       -> Cancelled(attempt_id)
```

`Taking longer than expected` is a presentation derived from the age of a
`Starting` state rather than a separate durable state. Each request receives a
monotonic attempt ID. Completion is applied only when both pane identity and
attempt ID still match.

A completion that no longer matches still produces a session, and the two ways
it can mismatch call for opposite handling. If the pane is gone, the close it
missed never had a session to end, so the completion ends the session itself —
a daemon-owned one survives a plain drop, which is the whole point of the
session daemon. If the pane is still open under a newer attempt, the completion
only detaches: attempts for one pane share that pane's identity with the daemon,
so killing here would take down the session that replaced this one.

Closing a pending or failed pane is a local model operation. It must not wait for
the backend. Cancelling a launch changes UI state immediately and sends a
best-effort cancellation request afterward.

A closed pane is released on both sides of the socket. The daemon drops the
pane on `Kill` — its PTY, its reader and its backlog — and emits no `Exited`
event for it, because a relaunch reuses the pane's identity and a late death
notice would land on the session that replaced it. The client therefore
releases the pane's byte channel itself; closing that channel is the EOF the
pane's reader thread is waiting for, and a pane dropped without it leaves that
thread blocked for the life of the process.

### Timing policy

Exact thresholds should remain constants so tests can substitute shorter
durations. The initial policy is:

- Immediately: show `Starting <backend> terminal...` in the new pane.
- After 5 seconds: show `Taking longer than expected` with **Cancel** and
  **Keep waiting**.
- After 20 seconds without a confirmed child: mark the attempt failed in the UI
  and offer recovery. The underlying operation may still be abandoned if the
  operating system cannot cancel it safely.
- After two timeouts for the same backend within 60 seconds: open the backend
  circuit breaker. Do not auto-launch more terminals for that backend until the
  user explicitly selects **Retry backend** or changes profile.
- A successful confirmed launch closes the circuit and resets its recent
  failure count.

The 20-second threshold is a UI deadline, not a claim that a blocked OS thread
was forcibly terminated.

### Pane-owned recovery UI

The status bar is optional and defaults off, so terminal launch state belongs
inside the affected pane. The pane remains part of navigation, can be renamed or
closed, and preserves its intended profile and directory.

Recommended copy and actions:

| State | Primary copy | Actions |
| --- | --- | --- |
| Starting | `Starting WSL terminal...` | Cancel |
| Slow | `WSL is taking longer than expected.` | Keep waiting, Cancel |
| Failed | `WSL did not respond. Your workspace is still available, but this terminal could not start.` | Retry, Use Windows shell, Change terminal profile, Copy diagnostics, Close pane |
| Backend paused | `New WSL terminals are paused after repeated failures.` | Retry backend, Change terminal profile |
| No-terminal startup | `Muxtrix started without opening a terminal.` | Start terminal, Change terminal profile |

Actions must use native button semantics, visible keyboard focus, and text
labels. State cannot rely on color or animation. The existing terminal card and
header remain stable; the failure surface replaces only terminal content.

### Global versus pane ownership

A single launch failure belongs to its pane. A circuit-open backend affects
future launches across panes, so it also receives one nonmodal global alert.
The alert must not steal focus and should link to the relevant backend setting
or retry action. Existing healthy terminal panes remain visually and
functionally unchanged.

### Startup recovery

Muxtrix first renders local application state, then begins session discovery and
default terminal launch. Startup must offer a terminal-free recovery route even
if no process host can start:

- `muxtrix --no-terminal` starts the complete application shell with a pending
  placeholder and no automatic host request.
- A discoverable startup modifier, preferably holding Shift, provides the same
  recovery route for packaged desktop launches.
- A failed initial launch remains a pane rather than closing the application.
- Settings, session cleanup, profile selection, diagnostics, and application
  exit remain usable.

The no-terminal route should not disable the session manager or hide existing
background sessions; it only suppresses automatic creation of a new terminal.

## Technical plan

### 1. Commit UI state before requesting a terminal

Introduce an explicit launch-state field in `TerminalRuntime` or a small
runtime wrapper keyed by `PaneId`. Construct a pending runtime without a
`LiveSession`. Normalize split, workspace, tab, worktree, restart, and startup
flows to:

1. Validate local input.
2. Insert or update the pane model and pending runtime.
3. Return control to Iced.
4. Submit a launch request as a task.
5. Apply success or failure only if the attempt ID is current.

A restart should keep the old terminal visible until the user confirms ending
it, or replace an already-exited runtime with a pending state. It must not
silently terminate healthy work while probing a replacement backend.

### 2. Move bootstrap behind the first frame

`Muxtrix::new` should build settings, local workspace state, controls, and a
pending initial pane without launching a process. `boot` should return an Iced
task that initializes the session host and then requests the first terminal.

If the session host cannot initialize, report `Session host unavailable` and
leave the pane recoverable. Do not automatically fall back to an in-process PTY
from the GUI startup path. A deliberate nonpersistent fallback can be offered as
a user action later, but it must also launch asynchronously.

Font discovery and other potentially expensive but local startup work should be
measured separately. This milestone is specifically about removing process-host
and IPC dependencies from first-frame availability.

### 3. Add a launch supervisor

The first implementation may reuse the existing background-task bridge to move
launch work off Iced, but the durable owner should be a launch supervisor with:

- a bounded command queue;
- at most one active launch per backend while that backend is unhealthy;
- attempt IDs and pane IDs on every command and result;
- soft and hard deadlines;
- best-effort cancellation;
- late-result disposal;
- recent failure tracking and a circuit breaker;
- structured diagnostic phases such as daemon start, daemon attach, PTY create,
  child create, and first output.

An abandoned operating-system call may leave a worker blocked until the host
recovers. Concurrency limits are therefore mandatory before adding automatic
timeouts or retries.

### 4. Make session IPC nonblocking from the GUI's perspective

Replace direct `SessionClient::send` calls with a dedicated writer and bounded
queue. Producers enqueue without waiting for the socket. Queue-full and
disconnected states become structured errors.

Classify messages by delivery behavior:

- Input and lifecycle commands preserve order and report backpressure.
- Resize may coalesce to the latest grid per pane.
- Layout and rename may coalesce to the latest session value.
- Shutdown, detach, and cancel must not wait on the UI thread.

Do not drop arbitrary terminal output bytes to relieve pressure because doing
so can corrupt VT state. Prefer bounded buffering and PTY backpressure, with a
future explicit snapshot/resynchronization protocol if live-stream recovery is
needed.

### 5. Decouple daemon spawning from its request loop

Extend the session protocol with correlation and truthful lifecycle events:

```text
Spawn { pane, attempt_id, ... }
SpawnAccepted { pane, attempt_id }
Spawned { pane, attempt_id, process_id }
SpawnFailed { pane, attempt_id, phase, error }
CancelSpawn { pane, attempt_id }
SpawnCancelled { pane, attempt_id }
```

The daemon request loop acknowledges and dispatches spawn work without calling
`PtySession::spawn` inline. Existing panes must continue receiving input,
resize, kill, and persistence traffic while a new backend launch is stalled.

A background thread alone provides UI isolation but cannot guarantee hard
cancellation. If field evidence shows process creation can remain blocked
indefinitely, introduce a killable per-pane launch-helper or pane-host process.
That helper owns the PTY and child, allowing Muxtrix to abandon one stuck pane
without killing the session daemon or other panes.

### 6. Make teardown nonblocking

`Drop` on a GUI-owned runtime must never join a potentially blocked session
thread. Split teardown into:

- an immediate handle release used by UI state transitions;
- best-effort detach or terminate dispatched to the supervisor;
- an explicit bounded join used by tests or controlled application shutdown.

Preserve the existing multiplexer contract: dropping the GUI detaches
daemon-owned panes, while an explicit pane close requests termination. The
difference is that neither operation waits on Iced.

### 7. Move adjacent probes behind loading states

Audit every action that opens or selects a section. The following become
asynchronous, deadline-bound operations with generation IDs:

- session liveness checks, picker refresh, attach, kill, and kill-all;
- worktree repository discovery, WSL home discovery, list, create, and delete;
- integration and hook inspection;
- future remote-host discovery.

Opening a surface only changes local UI state. The surface then shows loading,
ready, empty, or inline error content. Closing the surface invalidates its
generation so a late result cannot reopen or mutate it.

### 8. Add bounded diagnostics

Record structured, local diagnostics for each launch attempt:

- backend and selected distribution or profile;
- attempt ID and pane ID;
- phase transitions and elapsed durations;
- accepted, spawned, first-output, failed, cancelled, or timed-out result;
- queue saturation or IPC-disconnect events;
- cheap host-memory information when available without invoking the failing
  backend.

Diagnostics must redact environment values and shell content. Resource readings
are explanatory evidence, never a launch gate. **Copy diagnostics** should copy
a short user-reviewable report rather than silently uploading anything.

## Delivery stages

### Stage 0: deterministic failure harness

Add a fake launch backend or injected launcher that can block behind a test
barrier, fail at a named phase, succeed late, or emit no output. Add test-only
clock substitution for soft and hard launch deadlines.

Acceptance:

- Tests can hold launch forever without holding the test's simulated UI call.
- Tests can deterministically release a late result after cancel or retry.
- No real WSL, GUI window, or memory exhaustion is required.

### Stage 1: fail-open application and pending panes

Make first-frame bootstrap and all pane-creation paths model-first and
asynchronous. Add `Starting`, `Slow`, and `Failed` pane content, basic retry and
close actions, attempt correlation, and `--no-terminal`. Remove automatic
in-process fallback when the session host is unavailable.

Acceptance:

- A fake terminal launch that never returns does not stop app initialization.
- Split, new tab, new workspace, and restart handlers return without releasing
  the fake launch barrier.
- Navigation, Settings, command palette, rename, and close remain operable.
- A launch failure keeps the pane and exposes the backend error.
- Cancelling or retrying makes a late completion harmless.

### Stage 2: nonblocking session transport and daemon concurrency

Add the bounded client writer, protocol attempt IDs and acknowledgements, daemon
spawn dispatch, message coalescing, and nonblocking teardown.

Acceptance:

- A blocked spawn does not delay input or resize in an existing pane.
- Layout persistence cannot block Iced.
- Closing a stuck pane never waits for its worker.
- Spawn failures retain their structured error text.
- Writer and launch queues have tested bounds and saturation behavior.

### Stage 3: timeout policy, circuit breaker, and complete recovery UX

Add soft and hard deadlines, concurrency limits, backend failure memory,
fallback-profile actions, backend-wide alerts, safe startup modifier, and copyable
diagnostics.

Acceptance:

- Two recent timeouts pause automatic launches for only the affected backend.
- Existing healthy panes remain unaffected.
- Explicit retry or profile change closes or bypasses the circuit correctly.
- Recovery actions are fully keyboard operable and expose text state beyond
  color or animation.
- No automatic action shuts down WSL or unrelated processes.

### Stage 4: adjacent section hardening and optional process isolation

Move session and worktree probes off Iced, then decide from Windows evidence
whether a per-pane helper is necessary for hard cancellation.

Acceptance:

- Every surface-opening handler is free of process, Git, WSL, and blocking IPC
  calls.
- Session and worktree surfaces remain dismissible while their probes are
  stalled.
- If implemented, terminating a stuck pane helper leaves the session daemon and
  healthy panes intact.

## Headless verification matrix

| Scenario | Required proof |
| --- | --- |
| Initial backend never returns | Application state and first frame become available; recovery UI is navigable |
| New split never returns | Existing pane accepts input and the new pane shows slow/failed state |
| New tab or workspace never returns | New local model appears immediately and other tabs/workspaces switch normally |
| Daemon unavailable | No synchronous local fallback; pane explains session-host failure |
| Daemon accepts but child spawn stalls | Existing daemon panes still accept input, resize, and close |
| Client writer queue fills | UI receives a structured recoverable error without blocking |
| Cancel then late success | Late success is discarded and cannot resurrect the pane |
| Retry then old success | Only the newest attempt attaches |
| Two backend timeouts | Circuit opens for that backend; native or other backends remain available |
| Closing a stuck pane | UI close completes without waiting for a join |
| Session picker probe stalls | Picker can close and the rest of the app remains usable |
| Worktree probe stalls | Dialog remains dismissible and exposes loading/error state |
| Output pressure | Buffers stay bounded and VT bytes are not silently dropped |

Use unit tests in `muxtrix-terminal` and `muxtrix-sessions` for supervisor,
transport, and teardown behavior; application-state tests in `muxtrix-app` for
attempt correlation and recovery actions; and the existing private-display E2E
harness for the complete fail-open flow. Keep all automation headless.

## Low-hanging fruits and recommended starting slice

Implementation status (2026-08-12): items 1–4 below are complete. The app now
uses a barrier-testable launcher boundary, model-first background pane launches,
attempt-correlated late-result rejection, explicit daemon failure, window-event
deferred startup, and `--no-terminal`. Item 5 and the deeper bounded transport,
hard-cancellation, timeout, and circuit-breaker stages remain future work.

The highest-value low-risk starting slice is Stage 0 plus the narrowest part of
Stage 1. It improves the reported incident before changing the daemon protocol.

### 1. Add an injectable hung-launch test seam

Why first: the bug is otherwise difficult and unsafe to reproduce. A barrier-
controlled fake launcher lets every later change prove that the Iced handler
returns before the backend does.

Scope:

- Extract terminal launch behind a small app-facing launcher function or trait.
- Provide success, error, and never-completes test implementations.
- Add attempt IDs now so the seam does not need replacing for retry tests.

Risk: low. Production behavior remains unchanged until the next step.

### 2. Create the pane first and launch it in a background task

Why second: this directly prevents split, tab, workspace, and restart actions
from waiting on WSL. Reuse the existing background-task pattern initially; a
dedicated supervisor can replace it in Stage 2.

Scope:

- Add pending/starting/failed runtime states.
- Add `TerminalLaunchFinished(pane_id, attempt_id, result)` messages.
- Normalize every creation path to insert pending state before returning.
- Render a simple truthful starting or failed pane instead of an endless
  terminal preview.

Risk: medium but contained to app runtime construction. It does not require a
wire-protocol change.

### 3. Stop synchronously falling back when the session daemon fails

Why third: the current bounded daemon readiness failure can hand control to an
unbounded local PTY spawn. Once failed panes exist, the safer behavior is to
surface `Session host unavailable` and let the user retry.

Scope:

- Remove automatic local fallback from the GUI launch request.
- Keep explicit in-process operation for tests and any deliberate future
  nonpersistent action.

Risk: low after failed-pane UI exists. It trades hidden degraded persistence for
an honest recoverable state.

### 4. Start the initial pane after the first frame

Why fourth: this makes a fresh Muxtrix instance a reliable recovery surface.

Scope:

- Build `Muxtrix` with a pending initial pane.
- Start session-host initialization through an Iced task.
- Chain the initial launch only after host initialization resolves.
- Add `--no-terminal` while this path is already being separated.

Risk: medium. Startup/session-picker ordering needs focused regression coverage.

### 5. Move the easiest adjacent probes to existing background tasks

Why fifth: worktree and session views can reproduce the same visible freeze,
and most already return `Task<Message>` or have loading-state structures.

Start with worktree repository/list discovery and non-startup session-picker
refresh. Use generation IDs so dismissing the surface invalidates late results.

Risk: low to medium. This is largely relocation of existing synchronous work.

### First implementation milestone

Implement items 1 through 4 as one coherent milestone. It should end with this
user-visible guarantee:

> Even if terminal launch never returns, Muxtrix opens, the requested pane
> appears, navigation remains responsive, and the pane can be cancelled,
> retried, changed to another backend, or closed.

Do not begin with memory heuristics, a longer timeout, automatic WSL repair, or
the per-pane helper-process architecture. None of those removes the UI-thread
dependency by itself. Once the fail-open milestone is proven, Stage 2 makes the
same guarantee durable under blocked IPC and daemon contention.
