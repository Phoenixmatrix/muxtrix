//! Claude Code pane state from the harness's own session record.
//!
//! Claude Code writes `~/.claude/sessions/<pid>.json` from its live UI
//! state — every change to whether it is loading, showing a blocking dialog,
//! or sitting at its composer rewrites the file. `claude agents --json` reads
//! the same files, but strips the fields that matter most here (`waitingFor`,
//! `procStart`, `statusUpdatedAt`) and costs a Node process per read. Reading
//! the files directly is exact, immediate, and free.
//!
//! The record is the authority for a matched pane. Hooks add the exact turn
//! edges (`UserPromptSubmit`, `Stop`, `StopFailure`) and pane identity, and
//! can lead the record by a few milliseconds; a record older than the last
//! hook edge never regresses it. The terminal screen is only consulted for a
//! pane no record could be matched to.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use muxtrix_control::{AgentState, ClaudeHook};

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
/// still be mid-replace or belong to a newer format.
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
        .map(|mut record| {
            record.liveness = process_liveness(record.pid, record.proc_start.as_deref());
            record
        })
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

/// Linux: the process exists and `/proc/<pid>/stat` reports the start time
/// the record was written with, so a reused PID cannot vouch for a dead
/// session. Other platforms cannot check without a process table walk.
#[cfg(target_os = "linux")]
pub(crate) fn process_liveness(pid: Option<u32>, proc_start: Option<&str>) -> Liveness {
    let Some(pid) = pid else {
        return Liveness::Unknown;
    };
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return Liveness::Dead;
    };
    match (proc_start, process_start_time(&stat)) {
        (Some(expected), Some(actual)) if expected == actual => Liveness::Alive,
        (Some(_), Some(_)) => Liveness::Dead,
        _ => Liveness::Alive,
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn process_liveness(_pid: Option<u32>, _proc_start: Option<&str>) -> Liveness {
    Liveness::Unknown
}

/// Field 22 of `/proc/<pid>/stat`, after the parenthesised command name that
/// may itself contain spaces and parentheses.
pub(crate) fn process_start_time(stat: &str) -> Option<&str> {
    let after_name = &stat[stat.rfind(')')? + 1..];
    // The fields after the name start at field 3 (`state`).
    after_name.split_whitespace().nth(22 - 3)
}

/// Linux: the PID chain above a process, nearest first, bounded. A hook
/// command's parent is the shell Claude Code runs it through, whose parent is
/// the harness itself.
#[cfg(target_os = "linux")]
pub(crate) fn process_ancestry(pid: u32) -> Vec<u32> {
    let mut ancestry = Vec::new();
    let mut current = pid;
    for _ in 0..6 {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{current}/stat")) else {
            break;
        };
        let Some(parent) = process_parent(&stat) else {
            break;
        };
        if parent <= 1 {
            break;
        }
        ancestry.push(parent);
        current = parent;
    }
    ancestry
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn process_ancestry(_pid: u32) -> Vec<u32> {
    Vec::new()
}

/// Field 4 of `/proc/<pid>/stat`: the parent PID.
pub(crate) fn process_parent(stat: &str) -> Option<u32> {
    let after_name = &stat[stat.rfind(')')? + 1..];
    after_name.split_whitespace().nth(4 - 3)?.parse().ok()
}

/// What a hook or record asks the pane to become.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Decision {
    pub(crate) state: Option<(AgentState, String)>,
    /// A `Stop` hook: the turn ended and its consequences (PR refresh) apply.
    pub(crate) turn_completed: bool,
    /// A `SessionEnd` hook: the pane's agent status should be removed.
    pub(crate) session_ended: bool,
}

impl Decision {
    fn to(state: AgentState, activity: impl Into<String>) -> Self {
        Self {
            state: Some((state, activity.into())),
            ..Self::default()
        }
    }
}

/// Per-pane bookkeeping for one Claude Code conversation. State itself lives
/// on the pane's `AgentPaneStatus`; this tracks the evidence that decides it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ClaudeTracker {
    pub(crate) session_id: Option<String>,
    /// The harness PID, from a matched record or a hook's process ancestry.
    pub(crate) process_id: Option<u32>,
    /// PIDs above the last hook command: candidates for the harness PID.
    pub(crate) hook_ancestry: Vec<u32>,
    /// Wall-clock milliseconds of the last hook that set state. A record
    /// stamped earlier than this describes the moment before the edge.
    hook_edge_ms: Option<u64>,
    /// A live record is currently matched, so the screen has no authority.
    pub(crate) record_matched: bool,
    /// A turn has run in this session, so `idle` means a finished turn.
    saw_turn: bool,
}

impl ClaudeTracker {
    pub(crate) fn hook(&mut self, current: AgentState, hook: &ClaudeHook) -> Decision {
        if let Some(session_id) = hook.session_id.as_deref().filter(|id| !id.is_empty()) {
            if self.session_id.as_deref() != Some(session_id) {
                // A new conversation in the same pane (`/clear`, a resume):
                // its first turn has not run yet.
                self.saw_turn = false;
            }
            self.session_id = Some(session_id.to_owned());
        }
        let decision = match hook.event.as_str() {
            "SessionStart" => {
                self.saw_turn = false;
                Decision::to(AgentState::Idle, "Ready for input")
            }
            "UserPromptSubmit" => {
                self.saw_turn = true;
                Decision::to(AgentState::Running, "Agent is working")
            }
            "PermissionRequest" => Decision::to(
                AgentState::Waiting,
                hook.tool_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .map_or_else(
                        || "Approval required".to_owned(),
                        |tool| format!("Approve {tool}"),
                    ),
            ),
            "Elicitation" => Decision::to(AgentState::Waiting, "Answer required"),
            "Notification" => match hook.notification_type.as_deref() {
                Some("permission_prompt") => Decision::to(
                    AgentState::Waiting,
                    hook.message
                        .as_deref()
                        .filter(|message| !message.is_empty())
                        .unwrap_or("Approval required"),
                ),
                Some("elicitation_dialog" | "elicitation_url_dialog") => {
                    Decision::to(AgentState::Waiting, "Answer required")
                }
                // Idle reminders, auth, background-agent chatter: not state.
                _ => Decision::default(),
            },
            "Stop" => {
                self.saw_turn = true;
                Decision {
                    state: Some((
                        AgentState::Completed,
                        hook.last_assistant_message
                            .as_deref()
                            .map(short_body)
                            .filter(|body| !body.is_empty())
                            .unwrap_or_else(|| "Turn complete".to_owned()),
                    )),
                    turn_completed: true,
                    session_ended: false,
                }
            }
            "StopFailure" => Decision::to(
                AgentState::Failed,
                hook.message
                    .as_deref()
                    .map(short_body)
                    .filter(|body| !body.is_empty())
                    .unwrap_or_else(|| "Turn failed".to_owned()),
            ),
            "SubagentStart" if current != AgentState::Waiting => {
                self.saw_turn = true;
                Decision::to(AgentState::Running, "Agent is working")
            }
            "SessionEnd" => Decision {
                session_ended: true,
                ..Decision::default()
            },
            _ => Decision::default(),
        };
        if decision.state.is_some() {
            self.hook_edge_ms = Some(hook.sent_at_ms);
        }
        decision
    }

    /// The pane's matched record changed. Returns no state when the record
    /// predates the last hook edge or repeats the current state.
    pub(crate) fn record(&mut self, current: AgentState, record: &SessionRecord) -> Decision {
        self.record_matched = true;
        if let Some(pid) = record.pid {
            self.process_id = Some(pid);
        }
        if self.session_id.is_none() {
            self.session_id = record.session_id.clone();
        }
        let stamped = record.status_updated_at_ms.or(record.updated_at_ms);
        if let (Some(stamped), Some(edge)) = (stamped, self.hook_edge_ms)
            && stamped < edge
        {
            return Decision::default();
        }
        match record.status {
            Some(RecordStatus::Busy) => {
                self.saw_turn = true;
                Decision::to(AgentState::Running, "Agent is working")
            }
            Some(RecordStatus::Waiting) => {
                self.saw_turn = true;
                Decision::to(
                    AgentState::Waiting,
                    waiting_activity(record.waiting_for.as_deref()),
                )
            }
            Some(RecordStatus::Idle) => match current {
                // A failed or completed turn stays reported while the
                // composer merely idles; the next busy record starts over.
                AgentState::Failed | AgentState::Completed => Decision::default(),
                _ if self.saw_turn => Decision::to(AgentState::Completed, "Turn complete"),
                _ => Decision::to(AgentState::Idle, "Ready for input"),
            },
            Some(RecordStatus::Shell) => Decision::to(AgentState::Idle, "In shell mode"),
            None => Decision::default(),
        }
    }

    /// The matched record disappeared or its process died. Screen evidence
    /// regains authority; hook edges remain valid.
    pub(crate) fn record_lost(&mut self) {
        self.record_matched = false;
    }

    /// The user interrupted the turn from the keyboard.
    pub(crate) fn interrupted(&mut self) {
        self.saw_turn = false;
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

/// Watches the sessions directory from a background thread. Each change to
/// the set of files, their sizes, or their modification times re-reads the
/// records into the slot and wakes the app through `notify`. Reading is a
/// handful of small files, so polling is cheap; a filesystem watcher would
/// not reach a WSL distribution's files from Windows anyway.
pub(crate) fn spawn_watcher(
    directory: PathBuf,
    slot: RecordSlot,
    notify: Arc<dyn Fn() + Send + Sync>,
    interval: std::time::Duration,
) {
    let _ = std::thread::Builder::new()
        .name("claude-sessions".into())
        .spawn(move || {
            let mut fingerprint = None;
            loop {
                let next = directory_fingerprint(&directory);
                if next != fingerprint {
                    fingerprint = next;
                    let records = live_records(read_session_records(&directory));
                    if let Ok(mut slot) = slot.lock() {
                        *slot = Some(records);
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
    fn proc_stat_fields_survive_a_command_name_with_spaces_and_parentheses() {
        let stat =
            "42 (claude (x) y) S 41 42 42 0 -1 4194560 100 0 0 0 5 3 0 0 20 0 8 0 8139171 1 2 3";
        assert_eq!(process_start_time(stat), Some("8139171"));
        assert_eq!(process_parent(stat), Some(41));
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
    fn record_status_maps_to_pane_state_and_remembers_the_turn() {
        let mut tracker = ClaudeTracker::default();
        let idle = tracker.record(AgentState::Idle, &record("idle", 1));
        assert_eq!(
            idle.state,
            Some((AgentState::Idle, "Ready for input".into()))
        );
        assert!(tracker.record_matched);
        assert_eq!(tracker.process_id, Some(7));

        let busy = tracker.record(AgentState::Idle, &record("busy", 2));
        assert_eq!(
            busy.state,
            Some((AgentState::Running, "Agent is working".into()))
        );

        let mut waiting = record("waiting", 3);
        waiting.waiting_for = Some("permission prompt".into());
        let blocked = tracker.record(AgentState::Running, &waiting);
        assert_eq!(
            blocked.state,
            Some((AgentState::Waiting, "Approval required".into()))
        );

        // Idle after a turn is a finished turn, not a session that never ran.
        let done = tracker.record(AgentState::Running, &record("idle", 4));
        assert_eq!(
            done.state,
            Some((AgentState::Completed, "Turn complete".into()))
        );
        // And stays reported while the composer merely idles.
        let still = tracker.record(AgentState::Completed, &record("idle", 5));
        assert_eq!(still.state, None);

        let shell = tracker.record(AgentState::Completed, &record("shell", 6));
        assert_eq!(
            shell.state,
            Some((AgentState::Idle, "In shell mode".into()))
        );
    }

    #[test]
    fn a_record_older_than_the_hook_edge_cannot_regress_it() {
        let mut tracker = ClaudeTracker::default();
        let submitted = tracker.hook(AgentState::Idle, &hook("UserPromptSubmit"));
        assert_eq!(
            submitted.state,
            Some((AgentState::Running, "Agent is working".into()))
        );
        // The record still says idle from before the prompt.
        let stale = tracker.record(AgentState::Running, &record("idle", 999));
        assert_eq!(stale.state, None);
        // Once the harness rewrites it, the record decides again.
        let busy = tracker.record(AgentState::Running, &record("busy", 1_001));
        assert_eq!(
            busy.state,
            Some((AgentState::Running, "Agent is working".into()))
        );
        let done = tracker.record(AgentState::Running, &record("idle", 1_002));
        assert_eq!(
            done.state,
            Some((AgentState::Completed, "Turn complete".into()))
        );
    }

    #[test]
    fn hook_edges_are_exact_and_carry_their_reason() {
        let mut tracker = ClaudeTracker::default();
        let mut permission = hook("PermissionRequest");
        permission.tool_name = Some("Bash".into());
        assert_eq!(
            tracker.hook(AgentState::Running, &permission).state,
            Some((AgentState::Waiting, "Approve Bash".into()))
        );
        let mut notification = hook("Notification");
        notification.notification_type = Some("idle_prompt".into());
        assert_eq!(
            tracker.hook(AgentState::Completed, &notification),
            Decision::default()
        );
        notification.notification_type = Some("permission_prompt".into());
        notification.message = Some("Claude needs your permission to use Bash".into());
        assert_eq!(
            tracker.hook(AgentState::Running, &notification).state,
            Some((
                AgentState::Waiting,
                "Claude needs your permission to use Bash".into()
            ))
        );
        let mut stop = hook("Stop");
        stop.last_assistant_message = Some("Done.\n\nAll  tests pass.".into());
        let stopped = tracker.hook(AgentState::Running, &stop);
        assert_eq!(
            stopped.state,
            Some((AgentState::Completed, "Done. All tests pass.".into()))
        );
        assert!(stopped.turn_completed);
        let mut failure = hook("StopFailure");
        failure.message = Some("rate limited".into());
        assert_eq!(
            tracker.hook(AgentState::Running, &failure).state,
            Some((AgentState::Failed, "rate limited".into()))
        );
        // Idle records do not clear a failure; the next turn does.
        assert_eq!(
            tracker
                .record(AgentState::Failed, &record("idle", 2_000))
                .state,
            None
        );
        assert_eq!(
            tracker
                .record(AgentState::Failed, &record("busy", 2_001))
                .state,
            Some((AgentState::Running, "Agent is working".into()))
        );
        assert!(
            tracker
                .hook(AgentState::Idle, &hook("SessionEnd"))
                .session_ended
        );
        // A subagent starting inside a wait does not paint over the wait.
        assert_eq!(
            tracker.hook(AgentState::Waiting, &hook("SubagentStart")),
            Decision::default()
        );
    }

    #[test]
    fn a_new_session_in_the_pane_forgets_the_previous_turn() {
        let mut tracker = ClaudeTracker::default();
        tracker.hook(AgentState::Idle, &hook("UserPromptSubmit"));
        let mut restarted = hook("SessionStart");
        restarted.session_id = Some("next".into());
        restarted.sent_at_ms = 2_000;
        assert_eq!(
            tracker.hook(AgentState::Running, &restarted).state,
            Some((AgentState::Idle, "Ready for input".into()))
        );
        let mut idle = record("idle", 3_000);
        idle.session_id = Some("next".into());
        assert_eq!(
            tracker.record(AgentState::Idle, &idle).state,
            Some((AgentState::Idle, "Ready for input".into()))
        );
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

        let before = directory_fingerprint(&directory);
        std::fs::write(directory.join("3.json"), WAITING).expect("write");
        assert_ne!(directory_fingerprint(&directory), before);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
