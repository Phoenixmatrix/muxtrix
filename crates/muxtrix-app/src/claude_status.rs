//! Claude Code pane state from the harness's own session record.
//!
//! Claude Code writes `~/.claude/sessions/<pid>.json` from its live UI
//! state — every change to whether it is loading, showing a blocking dialog,
//! or sitting at its composer rewrites the file. `claude agents --json` reads
//! the same files, but strips the fields that matter most here (`waitingFor`,
//! `procStart`, `statusUpdatedAt`) and costs a Node process per read. Reading
//! the files directly is exact, immediate, and free.
//!
//! Records describe only the parent harness. Hook inventories and exact task
//! terminals track background executions independently, with per-task ordering
//! and terminal tombstones. Waiting wins; parent idleness cannot prove aggregate
//! completion. Missing, malformed and recovered evidence is explicitly unknown
//! rather than silently empty. Positive tasks do not expire with time; only a
//! detected observer gap weakens their confidence. Inventory validity is separate
//! from completion-observer availability. Cron schedules are not active executions.
//! Inventory loss has a causal watermark: only a strictly newer inventory can
//! repair it. Session exit retires identity independently of aggregate completion.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use muxtrix_control::{AgentState, ClaudeHook};

use crate::process::console_command;

/// `status` as Claude Code writes it. `shell` is its `!` shell mode: the
/// harness is idle while the user runs a command through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordStatus {
    Busy,
    Idle,
    Waiting,
    Shell,
}

impl RecordStatus {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "busy" => Some(Self::Busy),
            "idle" => Some(Self::Idle),
            "waiting" => Some(Self::Waiting),
            "shell" => Some(Self::Shell),
            _ => None,
        }
    }
}

/// Whether the process a record names is still the process that wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Liveness {
    /// The PID exists and its start time matches the record.
    Alive,
    /// The PID is gone or now belongs to a different process.
    Dead,
    /// This platform cannot tell; hook identity or uniqueness must vouch.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionRecord {
    pub(crate) pid: Option<u32>,
    pub(crate) proc_start: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) status: Option<RecordStatus>,
    pub(crate) waiting_for: Option<String>,
    pub(crate) status_updated_at_ms: Option<u64>,
    pub(crate) updated_at_ms: Option<u64>,
    pub(crate) liveness: Liveness,
}

impl SessionRecord {
    pub(crate) fn parse(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let object = value.as_object()?;
        let text = |key: &str| object.get(key).and_then(serde_json::Value::as_str);
        let number = |key: &str| object.get(key).and_then(serde_json::Value::as_u64);
        Some(Self {
            pid: number("pid").and_then(|pid| u32::try_from(pid).ok()),
            proc_start: text("procStart").map(str::to_owned),
            session_id: text("sessionId").map(str::to_owned),
            cwd: text("cwd").map(str::to_owned),
            kind: text("kind").map(str::to_owned),
            name: text("name").map(str::to_owned),
            status: text("status").and_then(RecordStatus::parse),
            waiting_for: text("waitingFor").map(str::to_owned),
            status_updated_at_ms: number("statusUpdatedAt"),
            updated_at_ms: number("updatedAt"),
            liveness: Liveness::Unknown,
        })
    }

    pub(crate) fn is_interactive(&self) -> bool {
        self.kind.as_deref() == Some("interactive")
    }

    fn freshness(&self) -> u64 {
        self.updated_at_ms
            .or(self.status_updated_at_ms)
            .unwrap_or_default()
    }
}

/// The directory Claude Code keeps its session records in, under the config
/// home it uses (`CLAUDE_CONFIG_DIR` when set, else `~/.claude`).
pub(crate) fn sessions_directory(home: &Path, config_dir: Option<&Path>) -> PathBuf {
    config_dir
        .map_or_else(|| home.join(".claude"), Path::to_path_buf)
        .join("sessions")
}

/// Reads every session record in the directory. Unreadable or unparseable
/// files are skipped: the harness writes them atomically, but a file may
/// still be mid-replace or belong to a newer format. Liveness is filled in
/// by the watcher's prober.
pub(crate) fn read_session_records(directory: &Path) -> Vec<SessionRecord> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|json| SessionRecord::parse(&json))
        .collect()
}

/// Keeps one record per live session: dead processes are dropped, and when a
/// resumed session left an older file behind with the same session ID, the
/// newest write wins.
pub(crate) fn live_records(records: Vec<SessionRecord>) -> Vec<SessionRecord> {
    let mut by_session: BTreeMap<String, SessionRecord> = BTreeMap::new();
    let mut anonymous = Vec::new();
    for record in records
        .into_iter()
        .filter(|record| record.liveness != Liveness::Dead)
    {
        match record.session_id.clone() {
            Some(session_id) => {
                let replace = by_session
                    .get(&session_id)
                    .is_none_or(|existing| record.freshness() >= existing.freshness());
                if replace {
                    by_session.insert(session_id, record);
                }
            }
            None => anonymous.push(record),
        }
    }
    by_session.into_values().chain(anonymous).collect()
}

/// Where the liveness prober runs: the shell on this host, or one inside the
/// WSL distribution whose records are being read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProbeHost {
    Local,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Wsl {
        distribution: String,
    },
}

/// One `sh` that answers, for each PID it is sent, that process's kernel
/// start time from `/proc/<pid>/stat` — or `-` when it is gone. The same
/// script serves Linux and WSL, so both platforms check liveness through one
/// code path; a host without `/proc` says so once and the prober retires.
const PROBE_SCRIPT: &str = r#"[ -r /proc/self/stat ] || { echo NOPROC; exit 0; }
while IFS= read -r line; do
  for pid in $line; do
    if s=$(cat "/proc/$pid/stat" 2>/dev/null); then
      rest=${s##*)}; set -- $rest; printf '%s %s
' "$pid" "${20}"
    else
      printf '%s -
' "$pid"
    fi
  done
  printf 'END
'
done"#;

/// What a prober sweep learned: each PID's start time, or `None` when gone.
pub(crate) type ProbeResult = BTreeMap<u32, Option<String>>;

struct Prober {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: std::sync::mpsc::Receiver<String>,
}

impl Prober {
    fn spawn(host: &ProbeHost) -> Option<Self> {
        let mut command = match host {
            ProbeHost::Local => console_command("sh"),
            ProbeHost::Wsl { distribution } => {
                let mut command = console_command("wsl.exe");
                if !distribution.trim().is_empty() {
                    command.args(["--distribution", distribution.trim()]);
                }
                command.args(["--exec", "sh"]);
                command
            }
        };
        let mut child = command
            .args(["-c", PROBE_SCRIPT])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let (sender, lines) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("claude-liveness".into())
            .spawn(move || {
                use std::io::BufRead as _;
                for line in std::io::BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if sender.send(line).is_err() {
                        break;
                    }
                }
            });
        Some(Self {
            child,
            stdin,
            lines,
        })
    }

    /// `Err(true)` means the host has no `/proc`: do not respawn.
    fn probe(&mut self, pids: &[u32], timeout: std::time::Duration) -> Result<ProbeResult, bool> {
        use std::io::Write as _;
        let mut line = pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).map_err(|_| false)?;
        self.stdin.flush().map_err(|_| false)?;
        let deadline = std::time::Instant::now() + timeout;
        let mut result = ProbeResult::new();
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let reply = self.lines.recv_timeout(remaining).map_err(|_| false)?;
            let reply = reply.trim();
            if reply == "END" {
                return Ok(result);
            }
            if reply == "NOPROC" {
                return Err(true);
            }
            if let Some((pid, start)) = reply.split_once(' ')
                && let Ok(pid) = pid.parse::<u32>()
            {
                result.insert(pid, (start != "-").then(|| start.to_owned()));
            }
        }
    }
}

impl Drop for Prober {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Applies a sweep to a record: alive only when the PID still carries the
/// start time the record was written with, so a reused PID cannot vouch for
/// a finished session. A PID the sweep did not cover stays unknown.
pub(crate) fn liveness_from_probe(record: &SessionRecord, probe: &ProbeResult) -> Liveness {
    let Some(pid) = record.pid else {
        return Liveness::Unknown;
    };
    match (probe.get(&pid), record.proc_start.as_deref()) {
        (None, _) => Liveness::Unknown,
        (Some(None), _) => Liveness::Dead,
        (Some(Some(actual)), Some(expected)) if actual == expected => Liveness::Alive,
        (Some(Some(_)), Some(_)) => Liveness::Dead,
        (Some(Some(_)), None) => Liveness::Alive,
    }
}

/// Aggregate changes are separate from delivery acceptance: rejected sessions
/// must not update pane identity or transcript metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decision {
    pub(crate) state: Option<(AgentState, String)>,
    pub(crate) turn_completed: bool,
    pub(crate) session_ended: bool,
    pub(crate) accepted: bool,
}

impl Default for Decision {
    fn default() -> Self {
        Self {
            state: None,
            turn_completed: false,
            session_ended: false,
            accepted: true,
        }
    }
}

impl Decision {
    fn to(state: AgentState, activity: impl Into<String>) -> Self {
        Self {
            state: Some((state, activity.into())),
            ..Self::default()
        }
    }

    fn rejected() -> Self {
        Self {
            accepted: false,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Parent {
    #[default]
    Idle,
    Running,
    Waiting(String),
    Stopped,
    Failed(String),
    Interrupted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TaskEvidence {
    at: u64,
    terminal: bool,
    uncertain: Option<String>,
}

/// Parent ordering and task ordering deliberately never share a watermark.
/// Tombstones survive snapshots, so a late start cannot resurrect a finished
/// execution. A complete inventory is stronger evidence than parent idleness.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct ClaudeTracker {
    pub(crate) session_id: Option<String>,
    pub(crate) transcript_path: Option<String>,
    pub(crate) process_id: Option<u32>,
    pub(crate) record_matched: bool,
    retired_sessions: BTreeSet<String>,
    session_started_at: u64,
    ended_at: Option<u64>,
    pending_end_at: Option<u64>,
    turn_started_at: u64,
    parent_at: u64,
    parent: Parent,
    tasks: BTreeMap<String, TaskEvidence>,
    snapshot_at: Option<u64>,
    snapshot_observed_at: u64,
    snapshot_problem: Option<String>,
    inventory_gap_at: Option<u64>,
    compacted_tasks_at: Option<u64>,
    crons: BTreeMap<String, (String, bool)>,
    cron_at: u64,
    saw_turn: bool,
    completion_reported: bool,
    summary: Option<String>,
    recovery_at: Option<u64>,
    parent_recovered: bool,
}

fn terminal_status(status: &str) -> bool {
    matches!(
        status,
        "completed" | "complete" | "failed" | "killed" | "cancelled" | "canceled" | "stopped"
    )
}

impl ClaudeTracker {
    /// Persist causal evidence without retaining assistant output or UI reasons.
    pub(crate) fn durable_snapshot(&self) -> Self {
        let mut snapshot = self.clone();
        snapshot.summary = None;
        snapshot.parent = match &self.parent {
            Parent::Failed(_) => Parent::Failed("Turn failed".into()),
            Parent::Waiting(_) => Parent::Waiting("Input required".into()),
            parent => parent.clone(),
        };
        if snapshot.snapshot_problem.is_some() {
            snapshot.snapshot_problem = Some("Background inventory is unavailable".into());
        }
        for task in snapshot.tasks.values_mut() {
            if task.uncertain.is_some() {
                task.uncertain = Some("Background task awaits reconciliation".into());
            }
        }
        snapshot
    }

    fn accept_session(&mut self, id: Option<&str>, start: bool, at: u64) -> bool {
        if self.ended_at.is_some_and(|ended| !start || at <= ended) {
            return false;
        }
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            return self.session_id.is_none();
        };
        if self.session_id.as_deref() == Some(id) {
            if let Some(ended) = self.ended_at {
                if !start || at <= ended {
                    return false;
                }
                self.ended_at = None;
            }
            if start && at > self.session_started_at {
                self.session_started_at = at;
                self.pending_end_at = None;
            }
            return at == 0 || at >= self.session_started_at;
        }
        let latest = self
            .tasks
            .values()
            .map(|task| task.at)
            .chain([
                self.session_started_at,
                self.parent_at,
                self.snapshot_observed_at,
            ])
            .max()
            .unwrap_or_default();
        if self.session_id.is_some() && (!start || at <= latest) {
            return false;
        }
        if self.retired_sessions.contains(id) && !start {
            return false;
        }
        let initial_gap = if self.session_id.is_none() {
            self.inventory_gap_at
        } else {
            None
        };
        let initial_problem = initial_gap.and(self.snapshot_problem.take());
        let mut retired = std::mem::take(&mut self.retired_sessions);
        if let Some(previous) = self.session_id.take() {
            retired.insert(previous);
        }
        retired.remove(id);
        *self = Self {
            session_id: Some(id.to_owned()),
            retired_sessions: retired,
            session_started_at: if start { at } else { 0 },
            inventory_gap_at: initial_gap,
            snapshot_observed_at: initial_gap.unwrap_or_default(),
            snapshot_problem: initial_problem,
            ..Self::default()
        };
        true
    }

    fn task_edge(
        &mut self,
        id: &str,
        at: u64,
        terminal: bool,
        uncertain: Option<String>,
        explicit_start: bool,
    ) {
        if id.is_empty() {
            self.snapshot_problem = Some("Background task has no identity".into());
            return;
        }
        if let Some(old) = self.tasks.get(id) {
            if at < old.at || (old.terminal && !terminal && (!explicit_start || at == old.at)) {
                return;
            }
            if !terminal && !old.terminal && at == old.at {
                return;
            }
        } else if self.compacted_tasks_at.is_some_and(|stamp| at <= stamp)
            || (!terminal
                && self
                    .snapshot_at
                    .is_some_and(|stamp| at < stamp || (at == stamp && !explicit_start)))
        {
            return;
        }
        self.tasks.insert(
            id.to_owned(),
            TaskEvidence {
                at,
                terminal,
                uncertain,
            },
        );
        if terminal && self.tasks.len() > 1024 {
            let boundary = self
                .tasks
                .values()
                .filter(|task| task.terminal)
                .map(|task| task.at)
                .max();
            if let Some(boundary) = boundary {
                self.compacted_tasks_at =
                    Some(self.compacted_tasks_at.unwrap_or_default().max(boundary));
                self.tasks.retain(|_, task| !task.terminal);
                if self.snapshot_at.is_none_or(|at| at <= boundary) {
                    self.inventory_gap_at =
                        Some(self.inventory_gap_at.unwrap_or_default().max(boundary));
                    self.snapshot_observed_at = self.snapshot_observed_at.max(boundary);
                    self.snapshot_problem = Some("Task history awaits fresh inventory".into());
                }
            }
        }
    }

    fn snapshot(&mut self, hook: &ClaudeHook) {
        let at = hook.sent_at_ms;
        if hook.background_tasks_invalid {
            if at >= self.snapshot_observed_at {
                self.snapshot_observed_at = at;
                self.snapshot_problem = Some("Malformed background task snapshot".into());
            }
        } else if let Some(tasks) = hook.background_tasks.as_ref()
            && (at > self.snapshot_observed_at
                || (self.snapshot_at.is_none() && self.snapshot_problem.is_none()))
            && self.inventory_gap_at.is_none_or(|gap| at > gap)
        {
            self.snapshot_observed_at = at;
            let ids: BTreeSet<&str> = tasks
                .iter()
                .filter(|task| task.ambient != Some(true))
                .map(|task| task.id.as_str())
                .collect();
            // The inventory watermark already rejects older missing tasks.
            // Do not manufacture equal-time terminal evidence from omission:
            // an explicit start at this same millisecond still proves work.
            self.tasks
                .retain(|id, task| task.at >= at || ids.contains(id.as_str()));
            self.snapshot_problem = None;
            self.inventory_gap_at = None;
            for task in tasks {
                if task.ambient == Some(true) {
                    continue;
                }
                if hook.event == "SubagentStop"
                    && hook.agent_id.as_deref() == Some(task.id.as_str())
                {
                    continue;
                }
                let terminal = terminal_status(&task.status)
                    && !(task.kind == "monitor" && task.ambient.is_none());
                let uncertain = if task.kind == "monitor" && task.ambient.is_none() {
                    Some("Monitor ambient classification is unavailable".into())
                } else if !terminal
                    && !matches!(
                        task.status.as_str(),
                        "running" | "pending" | "queued" | "in_progress"
                    )
                {
                    Some(format!(
                        "Unsupported background task status: {}",
                        task.status
                    ))
                } else {
                    None
                };
                self.task_edge(&task.id, at, terminal, uncertain, false);
            }
            self.snapshot_at = Some(at);
            if self.recovery_at.is_some_and(|stamp| at > stamp) {
                self.recovery_at = None;
            }
            self.tasks.retain(|_, task| !task.terminal || task.at >= at);
        }
        if at >= self.cron_at
            && !hook.session_crons_invalid
            && let Some(crons) = &hook.session_crons
        {
            self.crons = crons
                .iter()
                .map(|cron| (cron.id.clone(), (cron.schedule.clone(), cron.recurring)))
                .collect();
            self.cron_at = at;
        }
    }

    pub(crate) fn hook(&mut self, current: AgentState, hook: &ClaudeHook) -> Decision {
        let at = hook.sent_at_ms;
        if !self.accept_session(hook.session_id.as_deref(), hook.event == "SessionStart", at) {
            return Decision::rejected();
        }
        if hook.event == "SessionEnd" && at < self.session_started_at {
            return Decision::rejected();
        }
        if let Some(path) = &hook.transcript_path {
            self.transcript_path = Some(path.clone());
        }
        // A task completion carried on an old parent event is still new task
        // evidence; never discard it using the parent's watermark.
        match hook.event.as_str() {
            "SubagentStart" => {
                if let Some(id) = &hook.agent_id {
                    self.task_edge(id, at, false, None, true);
                }
            }
            "SubagentStop" => {
                if let Some(id) = &hook.agent_id {
                    self.task_edge(id, at, true, None, false);
                }
            }
            _ => {}
        }
        if at >= self.parent_at {
            let next = match hook.event.as_str() {
                "SessionStart" => Some(Parent::Idle),
                "UserPromptSubmit"
                    if at > self.parent_at
                        || self.parent_at == 0
                        || matches!(self.parent, Parent::Running) =>
                {
                    if !matches!(self.parent, Parent::Running) || at > self.parent_at {
                        self.completion_reported = false;
                        self.summary = None;
                        self.saw_turn = true;
                        self.turn_started_at = at;
                    }
                    Some(Parent::Running)
                }
                "PermissionRequest" if !self.record_matched => {
                    Some(Parent::Waiting(hook.tool_name.as_ref().map_or_else(
                        || "Approval required".into(),
                        |tool| format!("Approve {tool}"),
                    )))
                }
                "Elicitation" => Some(Parent::Waiting("Answer required".into())),
                "Notification" => match hook.notification_type.as_deref() {
                    Some("permission_prompt") => Some(Parent::Waiting("Approval required".into())),
                    Some("elicitation_dialog" | "elicitation_url_dialog") => {
                        Some(Parent::Waiting("Answer required".into()))
                    }
                    _ => None,
                },
                "Stop" => {
                    self.saw_turn = true;
                    self.summary = hook.last_assistant_message.as_deref().map(short_body);
                    Some(Parent::Stopped)
                }
                "StopFailure" => {
                    self.saw_turn = true;
                    Some(Parent::Failed(
                        hook.message
                            .as_deref()
                            .map(short_body)
                            .unwrap_or_else(|| "Turn failed".into()),
                    ))
                }
                "SessionEnd" => {
                    self.pending_end_at = Some(at);
                    Some(Parent::Interrupted)
                }
                _ => None,
            };
            if let Some(next) = next {
                if at > self.parent_at {
                    self.parent_recovered = false;
                }
                self.parent = next;
                self.parent_at = at;
            }
        }
        self.snapshot(hook);
        if matches!(
            hook.event.as_str(),
            "SubagentStart" | "SubagentStop" | "Notification"
        ) {
            self.derive_background(current)
        } else {
            self.derive(current)
        }
    }

    pub(crate) fn record(&mut self, current: AgentState, record: &SessionRecord) -> Decision {
        let at = record
            .status_updated_at_ms
            .or(record.updated_at_ms)
            .unwrap_or_default();
        if !self.accept_session(record.session_id.as_deref(), false, at) {
            return Decision::rejected();
        }
        self.record_matched = record.status.is_some();
        self.process_id = record.pid.or(self.process_id);
        if at < self.parent_at
            || (at == self.parent_at
                && self.parent_at != 0
                && record.status != Some(RecordStatus::Waiting))
        {
            return self.derive_background(current);
        }
        let Some(status) = record.status else {
            return Decision::default();
        };
        self.parent = match status {
            RecordStatus::Busy => {
                if !matches!(self.parent, Parent::Running) {
                    self.completion_reported = false;
                    self.turn_started_at = at;
                }
                self.saw_turn = true;
                Parent::Running
            }
            RecordStatus::Waiting => {
                Parent::Waiting(waiting_activity(record.waiting_for.as_deref()))
            }
            RecordStatus::Idle | RecordStatus::Shell => match &self.parent {
                Parent::Failed(_) | Parent::Stopped | Parent::Interrupted => self.parent.clone(),
                _ if self.saw_turn => Parent::Stopped,
                _ => Parent::Idle,
            },
        };
        if at > self.parent_at {
            self.parent_recovered = false;
        }
        self.parent_at = at;
        self.derive(current)
    }

    fn background_known(&self) -> bool {
        self.snapshot_at
            .is_some_and(|at| at >= self.turn_started_at)
            && self.snapshot_problem.is_none()
            && self.tasks.values().all(|task| task.terminal)
            && self.recovery_at.is_none()
    }

    fn derive(&mut self, _current: AgentState) -> Decision {
        if let Some(end) = self.pending_end_at
            && !self.has_background_work()
        {
            self.pending_end_at = None;
            self.ended_at = Some(end.max(self.parent_at));
            return Decision {
                session_ended: true,
                ..Decision::default()
            };
        }
        if !self.parent_recovered {
            if let Parent::Waiting(reason) = &self.parent {
                return Decision::to(AgentState::Waiting, reason.clone());
            }
            if matches!(self.parent, Parent::Running) {
                return Decision::to(AgentState::Running, "Agent is working");
            }
        }
        if self
            .tasks
            .values()
            .any(|task| !task.terminal && task.uncertain.is_none())
        {
            return Decision::to(AgentState::Running, "Background tasks are working");
        }
        if let Some(reason) = self
            .tasks
            .values()
            .filter(|task| !task.terminal)
            .find_map(|task| task.uncertain.as_ref())
        {
            return Decision::to(AgentState::Unknown, reason.clone());
        }
        if self.recovery_at.is_some() {
            return Decision::to(
                AgentState::Unknown,
                "Recovered activity awaits fresh evidence",
            );
        }
        if self.parent_recovered && matches!(self.parent, Parent::Running | Parent::Waiting(_)) {
            return Decision::to(AgentState::Unknown, "Parent activity awaits fresh evidence");
        }
        if !self.background_known() {
            return Decision::to(
                AgentState::Unknown,
                self.snapshot_problem
                    .as_deref()
                    .unwrap_or("Background task inventory is unavailable"),
            );
        }
        let mut decision = match &self.parent {
            Parent::Failed(reason) => Decision::to(AgentState::Failed, reason.clone()),
            Parent::Stopped if self.crons.is_empty() => Decision::to(
                AgentState::Completed,
                self.summary
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Turn complete"),
            ),
            Parent::Unknown => {
                Decision::to(AgentState::Unknown, "Parent activity awaits fresh evidence")
            }
            _ if !self.crons.is_empty() => {
                Decision::to(AgentState::Idle, "Scheduled work is armed")
            }
            _ => Decision::to(AgentState::Idle, "Ready for input"),
        };
        if self.saw_turn
            && matches!(self.parent, Parent::Stopped | Parent::Failed(_))
            && !self.completion_reported
        {
            self.completion_reported = true;
            decision.turn_completed = true;
        }
        decision
    }

    pub(crate) fn has_background_work(&self) -> bool {
        self.tasks.values().any(|task| !task.terminal)
    }

    pub(crate) fn session_ended(&self) -> bool {
        self.ended_at.is_some()
    }

    /// Retire a replaced process without reusing its task or parent evidence.
    /// Keep its identity fence so delayed deliveries cannot attach it again.
    pub(crate) fn retire_session(&mut self) {
        let ended_at = self
            .tasks
            .values()
            .map(|task| task.at)
            .chain([
                self.parent_at,
                self.session_started_at,
                self.snapshot_observed_at,
            ])
            .max()
            .unwrap_or_default();
        *self = Self {
            session_id: self.session_id.take(),
            retired_sessions: std::mem::take(&mut self.retired_sessions),
            ended_at: Some(ended_at),
            ..Self::default()
        };
    }

    pub(crate) fn record_lost(&mut self) {
        let was_matched = std::mem::replace(&mut self.record_matched, false);
        if was_matched && matches!(self.parent, Parent::Running | Parent::Waiting(_)) {
            self.parent = Parent::Unknown;
        }
    }

    pub(crate) fn interrupted(&mut self, current: AgentState) -> Decision {
        self.parent = Parent::Interrupted;
        self.derive(current)
    }

    pub(crate) fn restored(&mut self, current: AgentState) -> Decision {
        self.recovery_at = Some(
            self.tasks
                .values()
                .map(|task| task.at)
                .chain(self.snapshot_at)
                .chain([self.parent_at])
                .max()
                .unwrap_or_default(),
        );
        self.record_matched = false;
        self.parent_recovered = true;
        for task in self.tasks.values_mut().filter(|task| !task.terminal) {
            task.uncertain = Some("Recovered task awaits reconciliation".into());
        }
        self.derive(current)
    }

    fn derive_background(&mut self, current: AgentState) -> Decision {
        // The screen can establish a wait when no record is readable. Only a
        // parent event may resolve that wait, not background reconciliation.
        if current == AgentState::Waiting
            && !matches!(self.parent, Parent::Waiting(_))
            && !(self.pending_end_at.is_some() && !self.has_background_work())
        {
            return Decision::default();
        }
        self.derive(current)
    }

    pub(crate) fn background_source_unknown(
        &mut self,
        current: AgentState,
        reason: &str,
    ) -> Decision {
        // A completion observer only speaks for tasks already in the inventory.
        // Its absence cannot invalidate an authoritative empty inventory.
        for task in self.tasks.values_mut().filter(|task| !task.terminal) {
            task.uncertain = Some(reason.to_owned());
        }
        self.derive_background(current)
    }

    /// A lost inventory cannot be repaired by evidence at or before its boundary.
    /// An unorderable gap uses the current evidence frontier, never timestamp zero
    /// as permission to trust a snapshot already applied through live delivery.
    pub(crate) fn inventory_source_unknown_at(
        &mut self,
        current: AgentState,
        reason: &str,
        at_ms: u64,
    ) -> Decision {
        if at_ms != 0 && self.snapshot_at.is_some_and(|at| at > at_ms) {
            return self.derive_background(current);
        }
        let boundary = if at_ms == 0 {
            self.tasks
                .values()
                .map(|task| task.at)
                .chain([self.parent_at, self.snapshot_observed_at])
                .max()
                .unwrap_or_default()
        } else {
            at_ms
        };
        self.inventory_gap_at = Some(self.inventory_gap_at.unwrap_or_default().max(boundary));
        self.snapshot_observed_at = self.snapshot_observed_at.max(boundary);
        self.snapshot_problem = Some(reason.to_owned());
        for task in self.tasks.values_mut().filter(|task| !task.terminal) {
            if task.at <= boundary {
                task.uncertain = Some(reason.to_owned());
            }
        }
        self.derive_background(current)
    }

    pub(crate) fn reconcile(&mut self, current: AgentState) -> Decision {
        self.derive_background(current)
    }

    pub(crate) fn task_finished(
        &mut self,
        current: AgentState,
        session_id: &str,
        task_id: &str,
        status: &str,
        at_ms: u64,
    ) -> Decision {
        if self.session_id.as_deref() != Some(session_id)
            || !terminal_status(status)
            || task_id.is_empty()
            || self.tasks.get(task_id).is_some_and(|task| at_ms < task.at)
            || self.ended_at.is_some()
        {
            return Decision::rejected();
        }
        self.task_edge(task_id, at_ms, true, None, false);
        self.derive_background(current)
    }
}

/// `waitingFor` as Claude Code phrases it, in the sidebar's voice.
pub(crate) fn waiting_activity(waiting_for: Option<&str>) -> String {
    match waiting_for.map(str::trim) {
        Some("permission prompt") => "Approval required".into(),
        Some("input needed") => "Answer required".into(),
        Some("dialog open") => "Dialog needs an answer".into(),
        Some("sandbox request") => "Sandbox request needs approval".into(),
        Some("worker request") => "Worker request needs approval".into(),
        Some(other) if !other.is_empty() => {
            let mut label = other.to_owned();
            if let Some(first) = label.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            label
        }
        _ => "Waiting for you".into(),
    }
}

fn short_body(body: &str) -> String {
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated = body.chars().count() > 240;
    let mut body: String = body.chars().take(240).collect();
    if truncated {
        body.push('…');
    }
    body
}

/// The newest records the watcher has read and the app has not yet taken.
pub(crate) type RecordSlot = Arc<Mutex<Option<Vec<SessionRecord>>>>;

/// How often the prober sweeps every known PID.
pub(crate) const LIVENESS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Watches the sessions directory from a background thread. Each change to
/// the set of files, their sizes, or their modification times re-reads the
/// records; a long-lived prober sweeps their PIDs every few seconds (and as
/// soon as a new PID appears). Any change to the live set lands in the slot
/// and wakes the app through `notify`. Reading is a handful of small files,
/// so polling is cheap; a filesystem watcher would not reach a WSL
/// distribution's files from Windows anyway.
pub(crate) fn spawn_watcher(
    directory: PathBuf,
    host: ProbeHost,
    slot: RecordSlot,
    notify: Arc<dyn Fn() + Send + Sync>,
    interval: std::time::Duration,
) {
    let _ = std::thread::Builder::new()
        .name("claude-sessions".into())
        .spawn(move || {
            let mut fingerprint = None;
            let mut records: Vec<SessionRecord> = Vec::new();
            let mut probe = ProbeResult::new();
            let mut prober: Option<Prober> = None;
            let mut prober_retired = false;
            let mut probed_at: Option<std::time::Instant> = None;
            let mut published: Option<Vec<SessionRecord>> = None;
            loop {
                let next = directory_fingerprint(&directory);
                if next != fingerprint {
                    fingerprint = next;
                    records = read_session_records(&directory);
                }
                let pids = records
                    .iter()
                    .filter_map(|record| record.pid)
                    .collect::<BTreeSet<_>>();
                let unseen = pids.iter().any(|pid| !probe.contains_key(pid));
                let due = probed_at.is_none_or(|at| at.elapsed() >= LIVENESS_INTERVAL);
                if !prober_retired && !pids.is_empty() && (due || unseen) {
                    if prober.is_none() {
                        prober = Prober::spawn(&host);
                    }
                    let pids = pids.iter().copied().collect::<Vec<_>>();
                    match prober
                        .as_mut()
                        .map(|prober| prober.probe(&pids, PROBE_TIMEOUT))
                    {
                        Some(Ok(result)) => probe = result,
                        Some(Err(no_proc)) => {
                            // A wedged or exited prober is replaced on the
                            // next sweep; a host without /proc never is.
                            prober = None;
                            prober_retired = no_proc;
                        }
                        None => {}
                    }
                    probed_at = Some(std::time::Instant::now());
                }
                let live = live_records(
                    records
                        .iter()
                        .cloned()
                        .map(|mut record| {
                            record.liveness = liveness_from_probe(&record, &probe);
                            record
                        })
                        .collect(),
                );
                if published.as_ref() != Some(&live) {
                    published = Some(live.clone());
                    if let Ok(mut slot) = slot.lock() {
                        *slot = Some(live);
                    }
                    notify();
                }
                std::thread::sleep(interval);
            }
        });
}

fn directory_fingerprint(directory: &Path) -> Option<Vec<(String, u64, u128)>> {
    let entries = std::fs::read_dir(directory).ok()?;
    let mut fingerprint = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |elapsed| elapsed.as_nanos());
            Some((name, metadata.len(), modified))
        })
        .collect::<Vec<_>>();
    fingerprint.sort();
    Some(fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim shape of a session record on 2.1.246.
    const BUSY: &str = r#"{"pid":656783,"sessionId":"d2ef98dc-3a78-4387-896c-d88114a70b65","cwd":"/home/u/dev/muxtrix","startedAt":1787780101328,"procStart":"8139171","version":"2.1.246","kind":"interactive","entrypoint":"cli","name":"muxtrix-4d","status":"busy","updatedAt":1787780213922,"statusUpdatedAt":1787780213922}"#;
    const WAITING: &str = r#"{"pid":1,"sessionId":"s","cwd":"/w","kind":"interactive","status":"waiting","updatedAt":1781666105582,"statusUpdatedAt":1781666105582,"waitingFor":"dialog open"}"#;

    fn hook(event: &str) -> ClaudeHook {
        ClaudeHook {
            event: event.into(),
            session_id: Some("s".into()),
            sent_at_ms: 1_000,
            ..ClaudeHook::default()
        }
    }

    fn record(status: &str, stamped: u64) -> SessionRecord {
        SessionRecord {
            pid: Some(7),
            proc_start: None,
            session_id: Some("s".into()),
            cwd: Some("/w".into()),
            kind: Some("interactive".into()),
            name: None,
            status: RecordStatus::parse(status),
            waiting_for: None,
            status_updated_at_ms: Some(stamped),
            updated_at_ms: Some(stamped),
            liveness: Liveness::Alive,
        }
    }

    fn event(event: &str, at: u64, tasks: Option<serde_json::Value>) -> ClaudeHook {
        let mut hook = hook(event);
        hook.sent_at_ms = at;
        hook.background_tasks =
            tasks.map(|tasks| serde_json::from_value(tasks).expect("valid test fixture"));
        hook
    }

    fn active(id: &str, kind: &str) -> serde_json::Value {
        serde_json::json!([{"id":id, "type":kind, "status":"running", "ambient":false}])
    }

    fn state(decision: Decision) -> AgentState {
        decision.state.expect("aggregate state").0
    }

    #[test]
    fn replay_gap_survives_first_session_binding() {
        let mut tracker = ClaudeTracker::default();
        tracker.inventory_source_unknown_at(AgentState::Unknown, "lost hook", 40);
        let old = tracker.hook(
            AgentState::Unknown,
            &event("Stop", 30, Some(serde_json::json!([]))),
        );
        assert!(!old.turn_completed);
        assert_eq!(state(old), AgentState::Unknown);
        let repaired = tracker.hook(
            AgentState::Unknown,
            &event("Notification", 50, Some(serde_json::json!([]))),
        );
        assert!(repaired.turn_completed);
        assert_eq!(state(repaired), AgentState::Completed);
    }

    #[test]
    fn inventory_gaps_require_strictly_newer_snapshots_and_ignore_older_loss() {
        let mut tracker = ClaudeTracker::default();
        let snapshot = event("Stop", 30, Some(serde_json::json!([])));
        tracker.hook(AgentState::Running, &snapshot);
        assert_eq!(
            state(tracker.inventory_source_unknown_at(AgentState::Completed, "old loss", 20)),
            AgentState::Completed
        );
        assert_eq!(
            state(tracker.inventory_source_unknown_at(AgentState::Completed, "new loss", 40)),
            AgentState::Unknown
        );
        for at in [30, 40] {
            assert_eq!(
                state(tracker.hook(
                    AgentState::Unknown,
                    &event("Notification", at, Some(serde_json::json!([])))
                )),
                AgentState::Unknown
            );
        }
        assert_eq!(
            state(tracker.hook(
                AgentState::Unknown,
                &event("Notification", 50, Some(serde_json::json!([])))
            )),
            AgentState::Completed
        );
        assert_eq!(
            state(tracker.inventory_source_unknown_at(AgentState::Completed, "unorderable", 0)),
            AgentState::Unknown
        );
        assert_eq!(
            state(tracker.hook(
                AgentState::Unknown,
                &event("Notification", 50, Some(serde_json::json!([])))
            )),
            AgentState::Unknown
        );
        assert_eq!(
            state(tracker.hook(
                AgentState::Unknown,
                &event("Notification", 60, Some(serde_json::json!([])))
            )),
            AgentState::Completed
        );
    }

    #[test]
    fn session_exit_without_final_inventory_retires_without_completion() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(AgentState::Idle, &event("SessionStart", 10, None));
        tracker.hook(AgentState::Unknown, &event("UserPromptSubmit", 15, None));
        let end = tracker.hook(AgentState::Running, &event("SessionEnd", 20, None));
        assert!(end.session_ended);
        assert!(!end.turn_completed);
        assert!(tracker.session_ended());
        assert!(
            !tracker
                .hook(AgentState::Unknown, &event("Stop", 25, None))
                .accepted
        );
    }

    #[test]
    fn session_exit_waits_for_known_tasks_without_reporting_turn_completion() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("Stop", 10, Some(active("a", "shell"))),
        );
        let end = tracker.hook(AgentState::Running, &event("SessionEnd", 20, None));
        assert!(!end.session_ended);
        assert!(!end.turn_completed);
        assert_eq!(state(end), AgentState::Running);
        assert!(tracker.has_background_work());
        let finished = tracker.task_finished(AgentState::Running, "s", "a", "completed", 30);
        assert!(finished.session_ended);
        assert!(!finished.turn_completed);
        assert!(tracker.session_ended());
    }

    #[test]
    fn same_session_resume_rejects_old_end_including_unorderable_end() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(AgentState::Idle, &event("SessionStart", 10, None));
        tracker.hook(AgentState::Unknown, &event("SessionEnd", 20, None));
        tracker.hook(AgentState::Unknown, &event("SessionStart", 30, None));
        let before = tracker.clone();
        for at in [0, 20] {
            let end = tracker.hook(AgentState::Unknown, &event("SessionEnd", at, None));
            assert!(!end.accepted);
            assert!(!end.session_ended);
            assert_eq!(tracker, before);
        }
    }

    #[test]
    fn equal_time_empty_inventory_never_erases_explicit_start() {
        for (start_first, prior_start) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let mut tracker = ClaudeTracker::default();
            let empty = event("Stop", 20, Some(serde_json::json!([])));
            let mut start = event("SubagentStart", 20, None);
            start.agent_id = Some("a".into());
            if prior_start {
                let mut prior = start.clone();
                prior.sent_at_ms = 10;
                tracker.hook(AgentState::Running, &prior);
            }
            let events = if start_first {
                [&start, &empty]
            } else {
                [&empty, &start]
            };
            for hook in events {
                tracker.hook(AgentState::Running, hook);
            }
            assert!(tracker.has_background_work());
            assert_eq!(
                state(tracker.reconcile(AgentState::Running)),
                AgentState::Running
            );
            tracker.background_source_unknown(AgentState::Running, "observer gap");
            assert_eq!(
                state(tracker.hook(AgentState::Unknown, &start)),
                AgentState::Unknown
            );
        }
    }

    #[test]
    fn terminal_history_compacts_conservatively_then_fresh_inventory_repairs() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(AgentState::Idle, &event("SessionStart", 1, None));
        for at in 2..50_002 {
            tracker.task_finished(
                AgentState::Unknown,
                "s",
                &format!("task-{at}"),
                "completed",
                at,
            );
        }
        assert!(tracker.tasks.len() <= 1024);
        assert!(
            serde_json::to_vec(&tracker.durable_snapshot())
                .expect("valid test fixture")
                .len()
                < 2 * 1024 * 1024
        );
        let mut stale = event("SubagentStart", 2, None);
        stale.agent_id = Some("task-2".into());
        tracker.hook(AgentState::Unknown, &stale);
        assert!(!tracker.has_background_work());
        assert_eq!(
            state(tracker.reconcile(AgentState::Unknown)),
            AgentState::Unknown
        );
        assert_eq!(
            state(tracker.hook(
                AgentState::Unknown,
                &event("Stop", 60_000, Some(serde_json::json!([])))
            )),
            AgentState::Completed
        );
        assert!(tracker.tasks.is_empty());
    }

    #[test]
    fn process_retirement_blocks_old_delivery_until_fresh_session_start() {
        for known in [false, true] {
            let mut tracker = ClaudeTracker::default();
            if known {
                tracker.hook(AgentState::Idle, &event("SessionStart", 10, None));
            }
            tracker.retire_session();
            assert!(
                !tracker
                    .hook(AgentState::Idle, &event("Stop", 20, None))
                    .accepted
            );
            assert!(
                tracker
                    .hook(AgentState::Idle, &event("SessionStart", 30, None))
                    .accepted
            );
            assert!(!tracker.session_ended());
        }
    }

    #[test]
    fn every_positive_task_kind_outlives_parent_stop_idle_shell_failure_and_interrupt() {
        for kind in ["shell", "subagent", "workflow", "future-type"] {
            let mut tracker = ClaudeTracker::default();
            tracker.hook(
                AgentState::Idle,
                &event("UserPromptSubmit", 10, Some(active("a", kind))),
            );
            assert_eq!(
                state(tracker.hook(AgentState::Running, &event("Stop", 20, None))),
                AgentState::Running
            );
            assert_eq!(
                state(tracker.record(AgentState::Running, &record("idle", 30))),
                AgentState::Running
            );
            assert_eq!(
                state(tracker.record(AgentState::Running, &record("shell", 40))),
                AgentState::Running
            );
            assert_eq!(
                state(tracker.hook(AgentState::Running, &event("StopFailure", 50, None))),
                AgentState::Running
            );
            assert_eq!(
                state(tracker.interrupted(AgentState::Running)),
                AgentState::Running
            );
            assert!(tracker.has_background_work());
        }
    }

    #[test]
    fn waits_win_and_old_child_completion_is_not_filtered_by_parent_time() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("Stop", 10, Some(active("a", "subagent"))),
        );
        tracker.record(AgentState::Running, &record("waiting", 100));
        assert_eq!(
            state(tracker.task_finished(AgentState::Waiting, "s", "a", "completed", 20)),
            AgentState::Waiting
        );
        assert!(!tracker.has_background_work());
        let done = tracker.record(AgentState::Waiting, &record("idle", 110));
        assert_eq!(state(done.clone()), AgentState::Completed);
        assert!(done.turn_completed);
        assert!(
            !tracker
                .record(AgentState::Completed, &record("idle", 120))
                .turn_completed
        );
    }

    #[test]
    fn missing_malformed_and_previous_turn_empty_are_not_completion_evidence() {
        let mut tracker = ClaudeTracker::default();
        assert_eq!(
            state(tracker.hook(AgentState::Idle, &event("Stop", 10, None))),
            AgentState::Unknown
        );
        tracker.hook(
            AgentState::Unknown,
            &event("Notification", 20, Some(serde_json::json!([]))),
        );
        tracker.hook(AgentState::Completed, &event("UserPromptSubmit", 30, None));
        assert_eq!(
            state(tracker.hook(AgentState::Running, &event("Stop", 40, None))),
            AgentState::Unknown
        );
        let mut malformed = event("Stop", 50, None);
        malformed.background_tasks_invalid = true;
        assert_eq!(
            state(tracker.hook(AgentState::Unknown, &malformed)),
            AgentState::Unknown
        );
        let done = tracker.hook(
            AgentState::Unknown,
            &event("Notification", 60, Some(serde_json::json!([]))),
        );
        assert!(done.turn_completed);
        assert_eq!(state(done), AgentState::Completed);
        tracker.record(AgentState::Completed, &record("busy", 70));
        assert_eq!(
            state(tracker.record(AgentState::Running, &record("idle", 80))),
            AgentState::Unknown
        );
    }

    #[test]
    fn malformed_snapshot_preserves_positive_work() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("Stop", 10, Some(active("a", "shell"))),
        );
        let mut malformed = event("Stop", 20, Some(serde_json::json!([])));
        malformed.background_tasks_invalid = true;
        assert_eq!(
            state(tracker.hook(AgentState::Running, &malformed)),
            AgentState::Running
        );
        assert!(tracker.has_background_work());
        assert_eq!(
            state(tracker.task_finished(AgentState::Running, "s", "a", "completed", 30)),
            AgentState::Unknown
        );
    }

    #[test]
    fn empty_snapshot_cannot_erase_newer_start_and_terminal_tombstones_prevent_resurrection() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(AgentState::Idle, &event("Stop", 100, None));
        let mut start = event("SubagentStart", 80, None);
        start.agent_id = Some("a".into());
        tracker.hook(AgentState::Unknown, &start);
        tracker.hook(
            AgentState::Running,
            &event("Notification", 70, Some(serde_json::json!([]))),
        );
        assert!(tracker.has_background_work());
        let mut stop = event("SubagentStop", 90, Some(active("a", "subagent")));
        stop.agent_id = Some("a".into());
        let done = tracker.hook(AgentState::Running, &stop);
        assert!(done.turn_completed);
        tracker.hook(AgentState::Completed, &start);
        tracker.hook(
            AgentState::Completed,
            &event("Notification", 120, Some(active("a", "subagent"))),
        );
        assert!(!tracker.has_background_work());
        assert!(!tracker.hook(AgentState::Completed, &stop).turn_completed);
        start.sent_at_ms = 130;
        tracker.hook(AgentState::Completed, &start);
        assert!(tracker.has_background_work());
    }

    #[test]
    fn terminal_before_start_and_newer_empty_snapshot_are_tombstones() {
        let mut tracker = ClaudeTracker::default();
        let mut stop = event("SubagentStop", 30, None);
        stop.agent_id = Some("a".into());
        tracker.hook(AgentState::Idle, &stop);
        let mut start = event("SubagentStart", 20, None);
        start.agent_id = Some("a".into());
        tracker.hook(AgentState::Unknown, &start);
        assert!(!tracker.has_background_work());
        tracker.hook(
            AgentState::Unknown,
            &event("Stop", 40, Some(serde_json::json!([]))),
        );
        start.agent_id = Some("b".into());
        tracker.hook(AgentState::Completed, &start);
        assert!(!tracker.has_background_work());
    }

    #[test]
    fn last_task_does_not_complete_running_parent_and_failure_is_retained() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("UserPromptSubmit", 10, Some(active("a", "shell"))),
        );
        let done = tracker.task_finished(AgentState::Running, "s", "a", "completed", 20);
        assert!(!done.turn_completed);
        assert_eq!(state(done), AgentState::Running);
        tracker.hook(
            AgentState::Running,
            &event("StopFailure", 30, Some(active("b", "workflow"))),
        );
        let failed = tracker.task_finished(AgentState::Running, "s", "b", "failed", 40);
        assert_eq!(state(failed), AgentState::Failed);
        assert_eq!(
            state(tracker.record(AgentState::Failed, &record("idle", 50))),
            AgentState::Failed
        );
    }

    #[test]
    fn sessions_reject_cross_session_metadata_but_explicit_resume_is_allowed() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("SessionStart", 10, Some(active("a", "shell"))),
        );
        let mut next = event("SessionStart", 30, Some(serde_json::json!([])));
        next.session_id = Some("next".into());
        tracker.hook(AgentState::Running, &next);
        let before = tracker.clone();
        assert!(
            !tracker
                .hook(
                    AgentState::Idle,
                    &event("Stop", 40, Some(active("a", "shell")))
                )
                .accepted
        );
        assert!(
            !tracker
                .record(AgentState::Idle, &record("busy", 50))
                .accepted
        );
        assert_eq!(tracker, before);
        assert!(
            tracker
                .hook(
                    AgentState::Idle,
                    &event("SessionStart", 60, Some(serde_json::json!([])))
                )
                .accepted
        );
        let end = tracker.hook(
            AgentState::Idle,
            &event("SessionEnd", 70, Some(serde_json::json!([]))),
        );
        assert!(end.session_ended);
        assert!(
            !tracker
                .hook(AgentState::Idle, &event("Stop", 80, None))
                .accepted
        );
    }

    #[test]
    fn ambient_and_crons_are_not_running_but_unclassified_monitor_is_unknown() {
        let mut tracker = ClaudeTracker::default();
        let mut stop = event(
            "Stop",
            10,
            Some(serde_json::json!([
                {"id":"ambient","type":"monitor","status":"running","ambient":true}
            ])),
        );
        stop.session_crons = Some(
            serde_json::from_value(serde_json::json!([
                {"id":"cron","schedule":"* * * * *","recurring":true}
            ]))
            .expect("valid test fixture"),
        );
        assert_eq!(
            state(tracker.hook(AgentState::Running, &stop)),
            AgentState::Idle
        );
        assert!(!tracker.has_background_work());
        for status in ["running", "completed"] {
            let mut tracker = ClaudeTracker::default();
            let monitor = event(
                "Stop",
                10,
                Some(serde_json::json!([
                    {"id":"monitor","type":"monitor","status":status}
                ])),
            );
            let unknown = tracker.hook(AgentState::Running, &monitor);
            assert!(!unknown.turn_completed);
            assert_eq!(state(unknown), AgentState::Unknown);
        }
    }

    #[test]
    fn recovery_and_observer_loss_are_unknown_never_false_completion() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("Stop", 10, Some(active("a", "shell"))),
        );
        let json = serde_json::to_string(&tracker).expect("valid test fixture");
        let mut recovered: ClaudeTracker = serde_json::from_str(&json).expect("valid test fixture");
        assert_eq!(tracker, recovered);
        assert_eq!(
            state(recovered.restored(AgentState::Running)),
            AgentState::Unknown
        );
        assert_eq!(
            state(recovered.hook(
                AgentState::Unknown,
                &event("Notification", 20, Some(active("a", "shell")))
            )),
            AgentState::Running
        );
        let gap =
            recovered.background_source_unknown(AgentState::Running, "Transcript is unreadable");
        assert!(!gap.turn_completed);
        assert_eq!(state(gap), AgentState::Unknown);
        assert!(recovered.has_background_work());
        let done = recovered.task_finished(AgentState::Unknown, "s", "a", "completed", 30);
        assert_eq!(state(done.clone()), AgentState::Completed);
        assert!(done.turn_completed);
        assert!(
            !recovered
                .task_finished(AgentState::Completed, "s", "a", "completed", 30)
                .turn_completed
        );
        assert_eq!(
            state(
                tracker.background_source_unknown(AgentState::Running, "Transcript is unreadable")
            ),
            AgentState::Unknown
        );
    }

    #[test]
    fn transcript_terminals_require_matching_identity_status_and_task_order() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("Stop", 20, Some(active("a", "shell"))),
        );
        let before = tracker.clone();
        for (session, task, status, at) in [
            ("old", "a", "completed", 30),
            ("s", "", "completed", 30),
            ("s", "a", "running", 30),
            ("s", "a", "completed", 10),
        ] {
            assert!(
                !tracker
                    .task_finished(AgentState::Running, session, task, status, at)
                    .accepted
            );
            assert_eq!(tracker, before);
        }
    }

    #[test]
    fn lost_parent_and_incomplete_empty_recovery_do_not_invent_idle() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("UserPromptSubmit", 10, Some(serde_json::json!([]))),
        );
        tracker.record_lost();
        assert_eq!(
            state(tracker.derive(AgentState::Running)),
            AgentState::Running
        );
        tracker.record(AgentState::Running, &record("busy", 11));
        tracker.record_lost();
        assert_eq!(
            state(tracker.derive(AgentState::Running)),
            AgentState::Unknown
        );
        tracker.hook(
            AgentState::Unknown,
            &event("Stop", 20, Some(serde_json::json!([]))),
        );
        assert_eq!(
            state(tracker.inventory_source_unknown_at(
                AgentState::Completed,
                "Journal replay is incomplete",
                20,
            )),
            AgentState::Unknown
        );
        assert_eq!(
            state(tracker.hook(
                AgentState::Unknown,
                &event("Notification", 30, Some(serde_json::json!([])))
            )),
            AgentState::Completed
        );
    }

    #[test]
    fn newer_malformed_inventory_cannot_be_overruled_by_older_empty_inventory() {
        let mut tracker = ClaudeTracker::default();
        let mut malformed = event("Stop", 30, None);
        malformed.background_tasks_invalid = true;
        tracker.hook(AgentState::Running, &malformed);
        assert_eq!(
            state(tracker.hook(
                AgentState::Unknown,
                &event("Notification", 20, Some(serde_json::json!([])))
            )),
            AgentState::Unknown
        );
    }

    #[test]
    fn screen_wait_defers_completion_edge_until_parent_evidence_resolves_it() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("Stop", 10, Some(active("a", "shell"))),
        );
        let done = tracker.task_finished(AgentState::Waiting, "s", "a", "completed", 20);
        assert_eq!(done, Decision::default());
        assert!(!tracker.completion_reported);
        let resolved = tracker.record(AgentState::Waiting, &record("idle", 30));
        assert!(resolved.turn_completed);
        assert_eq!(state(resolved), AgentState::Completed);
    }

    #[test]
    fn replaying_identical_snapshot_does_not_clear_detected_observer_gap() {
        let mut tracker = ClaudeTracker::default();
        let snapshot = event("Stop", 10, Some(active("a", "shell")));
        tracker.hook(AgentState::Idle, &snapshot);
        tracker.background_source_unknown(AgentState::Running, "Transcript is unreadable");
        assert_eq!(
            state(tracker.hook(AgentState::Unknown, &snapshot)),
            AgentState::Unknown
        );
        assert!(tracker.has_background_work());
    }

    #[test]
    fn healthy_long_running_task_never_expires_and_completes_exactly_once() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("Stop", 10, Some(active("a", "shell"))),
        );
        // Historical timestamps deliberately put the task well beyond five minutes.
        for at in [3_600_010, 86_400_010] {
            assert_eq!(
                state(tracker.record(AgentState::Running, &record("idle", at))),
                AgentState::Running
            );
            let reconciled = tracker.reconcile(AgentState::Running);
            assert!(!reconciled.turn_completed);
            assert_eq!(state(reconciled), AgentState::Running);
        }
        let done = tracker.task_finished(AgentState::Running, "s", "a", "completed", 86_400_020);
        assert!(done.turn_completed);
        assert_eq!(state(done), AgentState::Completed);
        assert!(
            !tracker
                .task_finished(AgentState::Completed, "s", "a", "completed", 86_400_020)
                .turn_completed
        );
    }

    #[test]
    fn authoritative_empty_inventory_does_not_require_a_readable_transcript() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(
            AgentState::Idle,
            &event("Stop", 10, Some(active("a", "shell"))),
        );
        tracker.background_source_unknown(AgentState::Running, "Transcript is unreadable");
        let done = tracker.hook(
            AgentState::Unknown,
            &event("Stop", 20, Some(serde_json::json!([]))),
        );
        assert!(done.turn_completed);
        assert_eq!(state(done), AgentState::Completed);
        let missing =
            tracker.background_source_unknown(AgentState::Completed, "Transcript is unreadable");
        assert!(!missing.turn_completed);
        assert_eq!(state(missing), AgentState::Completed);
    }

    #[test]
    fn recovered_tasks_do_not_hide_fresh_parent_wait_or_busy_but_idle_reveals_gap() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(AgentState::Idle, &event("Stop", 10, None));
        tracker.hook(
            AgentState::Unknown,
            &event("Notification", 100, Some(active("a", "shell"))),
        );
        assert_eq!(
            state(tracker.restored(AgentState::Running)),
            AgentState::Unknown
        );
        assert_eq!(
            state(tracker.record(AgentState::Unknown, &record("waiting", 20))),
            AgentState::Waiting
        );
        assert_eq!(
            state(tracker.record(AgentState::Waiting, &record("busy", 30))),
            AgentState::Running
        );
        let idle = tracker.record(AgentState::Running, &record("idle", 40));
        assert!(!idle.turn_completed);
        assert_eq!(state(idle), AgentState::Unknown);
        assert!(tracker.has_background_work());
        assert_eq!(
            state(tracker.hook(
                AgentState::Unknown,
                &event("Notification", 110, Some(serde_json::json!([])))
            )),
            AgentState::Completed
        );
    }

    #[test]
    fn restoration_does_not_replay_old_parent_wait_or_busy_as_current_activity() {
        for status in ["waiting", "busy"] {
            let mut tracker = ClaudeTracker::default();
            tracker.record(AgentState::Idle, &record(status, 10));
            assert_eq!(
                state(tracker.restored(AgentState::Running)),
                AgentState::Unknown
            );
            assert_eq!(
                state(tracker.hook(
                    AgentState::Unknown,
                    &event("Notification", 20, Some(serde_json::json!([])))
                )),
                AgentState::Unknown
            );
        }
    }

    #[test]
    fn exact_terminal_before_inventory_is_retained_without_inventing_empty_inventory() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(AgentState::Idle, &event("SessionStart", 10, None));
        let terminal = tracker.task_finished(AgentState::Unknown, "s", "a", "completed", 30);
        assert!(terminal.accepted);
        assert!(!terminal.turn_completed);
        assert_eq!(state(terminal), AgentState::Unknown);
        let mut start = event("SubagentStart", 20, None);
        start.agent_id = Some("a".into());
        tracker.hook(AgentState::Unknown, &start);
        let done = tracker.hook(
            AgentState::Unknown,
            &event("Stop", 40, Some(active("a", "shell"))),
        );
        assert!(!tracker.has_background_work());
        assert!(done.turn_completed);
        assert_eq!(state(done), AgentState::Completed);
        assert!(
            !tracker
                .task_finished(AgentState::Completed, "s", "a", "completed", 30)
                .turn_completed
        );
    }

    #[test]
    fn durable_snapshot_excludes_sensitive_ui_text_without_changing_live_state() {
        let sensitive = "unique-sensitive-assistant-output";
        let mut tracker = ClaudeTracker::default();
        let mut stop = event("Stop", 10, Some(active("a", "shell")));
        stop.last_assistant_message = Some(sensitive.into());
        tracker.hook(AgentState::Running, &stop);
        let mut failed = event("StopFailure", 20, None);
        failed.message = Some(sensitive.into());
        tracker.hook(AgentState::Running, &failed);
        tracker.background_source_unknown(AgentState::Running, sensitive);
        tracker.inventory_source_unknown_at(AgentState::Unknown, sensitive, 20);
        let live = tracker.clone();
        for parent in [
            Parent::Failed(sensitive.into()),
            Parent::Waiting(sensitive.into()),
        ] {
            tracker.parent = parent;
            let durable = tracker.durable_snapshot();
            assert!(
                !serde_json::to_string(&durable)
                    .expect("valid test fixture")
                    .contains(sensitive)
            );
            assert_eq!(durable.session_id, tracker.session_id);
            assert_eq!(durable.parent_at, tracker.parent_at);
            assert_eq!(durable.tasks["a"].at, tracker.tasks["a"].at);
            assert!(durable.tasks["a"].uncertain.is_some());
            assert!(durable.snapshot_problem.is_some());
            assert_eq!(tracker.summary, live.summary);
            assert_eq!(tracker.tasks, live.tasks);
        }
    }

    #[test]
    fn a_real_record_parses_every_field_the_tracker_uses() {
        let record = SessionRecord::parse(BUSY).expect("record parses");
        assert_eq!(record.pid, Some(656_783));
        assert_eq!(record.proc_start.as_deref(), Some("8139171"));
        assert_eq!(
            record.session_id.as_deref(),
            Some("d2ef98dc-3a78-4387-896c-d88114a70b65")
        );
        assert_eq!(record.status, Some(RecordStatus::Busy));
        assert!(record.is_interactive());
        assert_eq!(record.status_updated_at_ms, Some(1_787_780_213_922));

        let waiting = SessionRecord::parse(WAITING).expect("record parses");
        assert_eq!(waiting.status, Some(RecordStatus::Waiting));
        assert_eq!(waiting.waiting_for.as_deref(), Some("dialog open"));
        assert_eq!(
            waiting_activity(waiting.waiting_for.as_deref()),
            "Dialog needs an answer"
        );
    }

    #[test]
    fn the_prober_reports_start_times_and_gone_pids_through_one_script() {
        let Some(mut prober) = Prober::spawn(&ProbeHost::Local) else {
            return;
        };
        let own = std::process::id();
        match prober.probe(&[own, u32::MAX], PROBE_TIMEOUT) {
            Ok(result) => {
                let start = result[&own].clone().expect("this process is alive");
                assert!(start.chars().all(|c| c.is_ascii_digit()));
                assert_eq!(result[&u32::MAX], None);
                let mut record = record("busy", 1);
                record.pid = Some(own);
                record.proc_start = Some(start);
                assert_eq!(liveness_from_probe(&record, &result), Liveness::Alive);
                record.proc_start = Some("1".into());
                assert_eq!(liveness_from_probe(&record, &result), Liveness::Dead);
                record.pid = Some(u32::MAX);
                assert_eq!(liveness_from_probe(&record, &result), Liveness::Dead);
                record.pid = Some(2);
                assert_eq!(liveness_from_probe(&record, &result), Liveness::Unknown);
            }
            // A host without /proc retires the prober rather than lying.
            Err(no_proc) => assert!(no_proc),
        }
    }

    #[test]
    fn dead_processes_and_superseded_resumes_are_dropped() {
        let mut stale = record("idle", 10);
        stale.pid = Some(1);
        let mut dead = record("busy", 50);
        dead.pid = Some(2);
        dead.session_id = Some("other".into());
        dead.liveness = Liveness::Dead;
        let fresh = record("busy", 20);
        let live = live_records(vec![stale, dead, fresh.clone()]);
        assert_eq!(live, vec![fresh]);
    }

    #[test]
    fn the_sessions_directory_follows_the_configured_claude_home() {
        assert_eq!(
            sessions_directory(Path::new("/home/u"), None),
            PathBuf::from("/home/u/.claude/sessions")
        );
        assert_eq!(
            sessions_directory(Path::new("/home/u"), Some(Path::new("/cfg"))),
            PathBuf::from("/cfg/sessions")
        );
    }

    #[test]
    fn records_are_read_from_a_directory_and_the_watcher_notices_changes() {
        let directory =
            std::env::temp_dir().join(format!("muxtrix-claude-sessions-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        std::fs::write(directory.join("1.json"), BUSY).expect("write");
        std::fs::write(directory.join("1.key"), "{}").expect("write");
        std::fs::write(directory.join("2.json"), "not json").expect("write");
        let records = read_session_records(&directory);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].pid, Some(656_783));
        assert_eq!(records[0].liveness, Liveness::Unknown);

        let before = directory_fingerprint(&directory);
        std::fs::write(directory.join("3.json"), WAITING).expect("write");
        assert_ne!(directory_fingerprint(&directory), before);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
