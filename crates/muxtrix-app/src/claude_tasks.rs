//! Task completion evidence from Claude's system-generated transcript notifications.
//!
//! Observed Claude records use `origin.kind=task-notification`, `promptSource=system`,
//! `queueSkipAttachments=true` and a user-role XML task envelope. Ordinary user or
//! assistant text is not evidence. EOF, output silence and event-only monitor
//! notifications never complete a task. Exact hook-provided paths also preserve
//! custom CLAUDE_CONFIG_DIR homes; we never guess a session from a directory scan.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::claude_status::ProbeHost;

const READ_BUDGET: usize = 256 * 1024;
const MAX_LINE: usize = 1024 * 1024;
const MAX_UPDATES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskRequest {
    pub(crate) pane_id: String,
    pub(crate) session_id: String,
    pub(crate) transcript_path: PathBuf,
    pub(crate) host: ProbeHost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskEvent {
    Finished {
        task_id: String,
        status: String,
        at_ms: u64,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskObservation {
    pub(crate) pane_id: String,
    pub(crate) session_id: String,
    pub(crate) transcript_path: PathBuf,
    pub(crate) event: TaskEvent,
}

fn unavailable(reason: &str) -> TaskEvent {
    TaskEvent::Unavailable {
        reason: reason.into(),
    }
}

fn source_path(request: &TaskRequest) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    if let ProbeHost::Wsl { distribution } = &request.host {
        let path = request.transcript_path.to_str()?;
        if path.starts_with('/') {
            if distribution.is_empty() || distribution.contains(['/', '\\']) {
                return None;
            }
            return Some(PathBuf::from(format!(
                "\\\\wsl.localhost\\{}{}",
                distribution,
                path.replace('/', "\\")
            )));
        }
    }
    request
        .transcript_path
        .is_absolute()
        .then(|| request.transcript_path.clone())
}

/// Parse only the UTC millisecond timestamps emitted by the observed transcript
/// schema. Unknown timestamp formats are unavailable, never wall-clock guesses.
fn timestamp_ms(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.len() != 24
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'.'
        || b[23] != b'Z'
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u64> {
        b[range].iter().try_fold(0, |n, c| {
            c.is_ascii_digit().then(|| n * 10 + u64::from(c - b'0'))
        })
    };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;
    let millis = number(20..23)?;
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let leap = |y: u64| y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let lengths = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day == 0 || day > lengths[(month - 1) as usize] {
        return None;
    }
    let leaps_before = |y: u64| (y - 1) / 4 - (y - 1) / 100 + (y - 1) / 400;
    let days = (year - 1970) * 365 + leaps_before(year) - leaps_before(1970)
        + lengths[..(month - 1) as usize].iter().sum::<u64>()
        + day
        - 1;
    Some((((days * 24 + hour) * 60 + minute) * 60 + second) * 1000 + millis)
}

fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let (before, rest) = text.split_once(&open)?;
    let (value, after) = rest.split_once(&close)?;
    if before.contains(&close)
        || after.contains(&open)
        || after.contains(&close)
        || value.contains(['<', '>'])
    {
        return None;
    }
    Some(value)
}

fn parse_line(line: &[u8], session_id: &str) -> Option<TaskEvent> {
    let value: serde_json::Value = match serde_json::from_slice(line) {
        Ok(value) => value,
        Err(_) => return Some(unavailable("corrupt transcript record")),
    };
    if value.get("sessionId").and_then(|v| v.as_str()) != Some(session_id) {
        return None;
    }
    if value.pointer("/origin/kind").and_then(|v| v.as_str()) != Some("task-notification") {
        return None;
    }
    if value.get("type").and_then(|v| v.as_str()) != Some("user")
        || value.get("promptSource").and_then(|v| v.as_str()) != Some("system")
        || value.get("queueSkipAttachments").and_then(|v| v.as_bool()) != Some(true)
        || value.get("isSidechain").and_then(|v| v.as_bool()) != Some(false)
        || value.pointer("/message/role").and_then(|v| v.as_str()) != Some("user")
    {
        return Some(unavailable("unsupported task notification provenance"));
    }
    let Some(text) = value.pointer("/message/content").and_then(|v| v.as_str()) else {
        return Some(unavailable("unsupported task notification content"));
    };
    let Some(body) = text.strip_prefix("<task-notification>\n").and_then(|text| {
        text.split_once("</task-notification>")
            .map(|(body, _)| body)
    }) else {
        return Some(unavailable("unsupported task notification envelope"));
    };
    // The task ID and status precede free-form summary/result text. Never find
    // tags in those untrusted payloads, even inside a genuine notification.
    let header = body.split("<summary>").next().unwrap_or(body);
    let Some(task_id) = field(header, "task-id") else {
        return Some(unavailable("missing task notification identity"));
    };
    if task_id.is_empty()
        || !task_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Some(unavailable("unsupported task notification identity"));
    }
    let Some(status) = field(header, "status") else {
        // Monitor event deliveries are not terminal status records.
        return Some(unavailable("task notification has no terminal status"));
    };
    if !matches!(status, "completed" | "failed" | "stopped" | "killed") {
        return Some(unavailable("unsupported task notification status"));
    }
    let Some(at_ms) = value
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(timestamp_ms)
    else {
        return Some(unavailable("unsupported task notification timestamp"));
    };
    Some(TaskEvent::Finished {
        task_id: task_id.into(),
        status: status.into(),
        at_ms,
    })
}

#[derive(Default)]
struct Cursor {
    offset: u64,
    partial: Vec<u8>,
    buffer: Vec<u8>,
    oversized: bool,
    anchor: Vec<u8>,
    modified: Option<std::time::SystemTime>,
    identity: Option<(u64, u64)>,
    last_unavailable: Option<String>,
}

fn file_identity(metadata: &std::fs::Metadata) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        metadata
            .created()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|created| (created.as_secs(), u64::from(created.subsec_nanos())))
    }
}

impl Cursor {
    fn observe(&mut self, path: &Path, session_id: &str) -> Vec<TaskEvent> {
        match self.read(path, session_id) {
            Ok(events) => {
                let mut out = Vec::new();
                for event in events {
                    if let TaskEvent::Unavailable { reason } = &event {
                        if self.last_unavailable.as_ref() == Some(reason) {
                            continue;
                        }
                        self.last_unavailable = Some(reason.clone());
                    } else {
                        self.last_unavailable = None;
                    }
                    out.push(event);
                }
                out
            }
            Err(_) => {
                let reason = "task transcript cannot be read";
                if self.last_unavailable.as_deref() == Some(reason) {
                    return Vec::new();
                }
                self.last_unavailable = Some(reason.into());
                vec![unavailable(reason)]
            }
        }
    }

    fn read(&mut self, path: &Path, session_id: &str) -> std::io::Result<Vec<TaskEvent>> {
        let mut file = std::fs::File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(std::io::Error::other("not a regular transcript"));
        }
        let identity = file_identity(&metadata);
        let modified = metadata.modified().ok();
        let mut reset = metadata.len() < self.offset
            || (metadata.len() == self.offset
                && self.modified.is_some()
                && modified != self.modified)
            || (self.identity.is_some() && identity != self.identity);
        if !reset && !self.anchor.is_empty() {
            file.seek(SeekFrom::Start(self.offset - self.anchor.len() as u64))?;
            let mut check = [0; 64];
            let check = &mut check[..self.anchor.len()];
            reset = file.read_exact(check).is_err() || check != self.anchor.as_slice();
        }
        let mut events = Vec::new();
        if reset {
            self.offset = 0;
            self.partial.clear();
            self.oversized = false;
            self.anchor.clear();
            events.push(unavailable("task transcript source reset; rescanning"));
        }
        self.identity = identity;
        self.modified = modified;
        file.seek(SeekFrom::Start(self.offset))?;
        self.buffer.resize(READ_BUDGET, 0);
        let count = file.read(&mut self.buffer)?;
        let bytes = &self.buffer[..count];
        self.offset += count as u64;
        for byte in bytes {
            if *byte == b'\n' {
                if !self.oversized
                    && !self.partial.is_empty()
                    && let Some(event) = parse_line(&self.partial, session_id)
                {
                    events.push(event);
                }
                self.partial.clear();
                self.oversized = false;
            } else if !self.oversized {
                if self.partial.len() == MAX_LINE {
                    self.partial.clear();
                    self.oversized = true;
                    events.push(unavailable("task transcript record exceeds supported size"));
                } else {
                    self.partial.push(*byte);
                }
            }
        }
        if count > 0 {
            self.anchor
                .extend_from_slice(&bytes[bytes.len().saturating_sub(64)..]);
            if self.anchor.len() > 64 {
                self.anchor.drain(..self.anchor.len() - 64);
            }
        }
        Ok(events)
    }
}

/// Disk work and JSON decoding stay on a dedicated thread. Each pass reads at
/// most 256 KiB per source; partial/oversized lines cannot grow without bound.
/// Requests are replaced by the app, and output is drained by the app. Dropping
/// the app-owned slots stops this thread rather than retaining closed windows.
pub(crate) fn spawn_watcher(
    requests: Arc<Mutex<Vec<TaskRequest>>>,
    updates: Arc<Mutex<Vec<TaskObservation>>>,
    notify: Arc<dyn Fn() + Send + Sync>,
    interval: Duration,
) {
    let requests = Arc::downgrade(&requests);
    let updates = Arc::downgrade(&updates);
    let _ = std::thread::Builder::new()
        .name("claude-tasks".into())
        .spawn(move || {
            let mut cursors: BTreeMap<String, (TaskRequest, Cursor)> = BTreeMap::new();
            while let (Some(requests), Some(updates)) = (requests.upgrade(), updates.upgrade()) {
                let current = requests.lock().unwrap_or_else(|e| e.into_inner()).clone();
                cursors.retain(|pane, _| current.iter().any(|request| &request.pane_id == pane));
                let mut changed = false;
                for request in current {
                    if updates.lock().unwrap_or_else(|e| e.into_inner()).len() >= MAX_UPDATES {
                        break;
                    }
                    let entry = cursors
                        .entry(request.pane_id.clone())
                        .or_insert_with(|| (request.clone(), Cursor::default()));
                    if entry.0 != request {
                        *entry = (request.clone(), Cursor::default());
                    }
                    let events = match source_path(&request) {
                        Some(path) => entry.1.observe(&path, &request.session_id),
                        None => {
                            let reason = "task transcript path cannot be resolved on this host";
                            if entry.1.last_unavailable.as_deref() == Some(reason) {
                                Vec::new()
                            } else {
                                entry.1.last_unavailable = Some(reason.into());
                                vec![unavailable(reason)]
                            }
                        }
                    };
                    changed |= !events.is_empty();
                    updates.lock().unwrap_or_else(|e| e.into_inner()).extend(
                        events.into_iter().map(|event| TaskObservation {
                            pane_id: request.pane_id.clone(),
                            session_id: request.session_id.clone(),
                            transcript_path: request.transcript_path.clone(),
                            event,
                        }),
                    );
                }
                if changed {
                    notify();
                }
                drop(requests);
                drop(updates);
                std::thread::sleep(interval.max(Duration::from_millis(10)));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn record(session: &str, id: &str, status: &str) -> serde_json::Value {
        serde_json::json!({
            "type":"user", "sessionId":session, "isSidechain":false,
            "origin":{"kind":"task-notification","producer":"session-task"},
            "promptSource":"system", "queueSkipAttachments":true,
            "timestamp":"2026-09-23T15:23:33.171Z",
            "message":{"role":"user","content":format!("<task-notification>\n<task-id>{id}</task-id>\n<tool-use-id>toolu_test</tool-use-id>\n<output-file>/tmp/task.output</output-file>\n<status>{status}</status>\n<summary>Background command completed</summary>\n</task-notification>\nRead the output file to retrieve the result.")}
        })
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("muxtrix-task-test-{}", uuid::Uuid::new_v4())))
        }
        fn append(&self, bytes: &[u8]) {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.0)
                .expect("valid test fixture")
                .write_all(bytes)
                .expect("valid test fixture");
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn genuine_notification_preserves_exact_task_and_timestamp() {
        let line = serde_json::to_vec(&record("session", "b123", "completed"))
            .expect("valid test fixture");
        assert_eq!(
            parse_line(&line, "session"),
            Some(TaskEvent::Finished {
                task_id: "b123".into(),
                status: "completed".into(),
                at_ms: 1_790_177_013_171,
            })
        );
        assert_eq!(parse_line(&line, "other-session"), None);
        for status in ["failed", "stopped", "killed"] {
            assert!(
                matches!(parse_line(&serde_json::to_vec(&record("session", "other-id", status)).expect("valid test fixture"), "session"),
                Some(TaskEvent::Finished { task_id, .. }) if task_id == "other-id")
            );
        }
    }

    #[test]
    fn lookalikes_and_result_text_are_not_completion() {
        let mut value = record("session", "b123", "completed");
        value
            .as_object_mut()
            .expect("valid test fixture")
            .remove("origin");
        assert_eq!(
            parse_line(
                &serde_json::to_vec(&value).expect("valid test fixture"),
                "session"
            ),
            None
        );
        value["type"] = "assistant".into();
        assert_eq!(
            parse_line(
                &serde_json::to_vec(&value).expect("valid test fixture"),
                "session"
            ),
            None
        );
        let mut value = record("session", "b123", "completed");
        value["message"]["content"] = "<task-notification>\n<task-id>b123</task-id>\n<summary><status>completed</status></summary>\n<event>update</event>\n</task-notification>".into();
        assert!(matches!(
            parse_line(
                &serde_json::to_vec(&value).expect("valid test fixture"),
                "session"
            ),
            Some(TaskEvent::Unavailable { .. })
        ));
    }

    #[test]
    fn silent_finish_requires_notification_and_partial_append_recovers() {
        let fixture = Fixture::new();
        fixture.append(b"");
        let mut cursor = Cursor::default();
        assert!(cursor.observe(&fixture.0, "session").is_empty());
        assert!(cursor.observe(&fixture.0, "session").is_empty());
        let mut bytes = serde_json::to_vec(&record("session", "b123", "completed"))
            .expect("valid test fixture");
        bytes.push(b'\n');
        let split = bytes.len() / 2;
        fixture.append(&bytes[..split]);
        assert!(cursor.observe(&fixture.0, "session").is_empty());
        fixture.append(&bytes[split..]);
        assert!(matches!(
            cursor.observe(&fixture.0, "session").as_slice(),
            [TaskEvent::Finished { .. }]
        ));
        assert!(cursor.observe(&fixture.0, "session").is_empty());
        // Restart scans exact evidence again; reducer timestamps make replay idempotent.
        assert!(matches!(
            Cursor::default().observe(&fixture.0, "session").as_slice(),
            [TaskEvent::Finished { .. }]
        ));
        assert!(
            Cursor::default()
                .observe(&fixture.0, "new-session")
                .is_empty()
        );
    }

    #[test]
    fn read_failure_corruption_and_reset_never_invent_completion() {
        let fixture = Fixture::new();
        let mut cursor = Cursor::default();
        assert!(matches!(
            cursor.observe(&fixture.0, "session").as_slice(),
            [TaskEvent::Unavailable { .. }]
        ));
        fixture.append(b"not json\n");
        assert!(matches!(
            cursor.observe(&fixture.0, "session").as_slice(),
            [TaskEvent::Unavailable { .. }]
        ));
        std::fs::write(&fixture.0, b"{}\n").expect("valid test fixture");
        assert!(
            matches!(cursor.observe(&fixture.0, "session").as_slice(), [TaskEvent::Unavailable { reason }] if reason.contains("reset"))
        );
        let mut bytes =
            serde_json::to_vec(&record("session", "b123", "failed")).expect("valid test fixture");
        bytes.push(b'\n');
        fixture.append(&bytes);
        assert!(
            matches!(cursor.observe(&fixture.0, "session").as_slice(), [TaskEvent::Finished { status, .. }] if status == "failed")
        );
    }

    #[test]
    fn invalid_dates_and_unknown_status_are_unavailable() {
        assert_eq!(timestamp_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(timestamp_ms("2026-02-29T00:00:00.000Z"), None);
        assert_eq!(timestamp_ms("2026-01-01T00:00:00Z"), None);
        assert!(matches!(
            parse_line(
                &serde_json::to_vec(&record("session", "b123", "running"))
                    .expect("valid test fixture"),
                "session"
            ),
            Some(TaskEvent::Unavailable { .. })
        ));
    }

    #[test]
    fn oversized_line_is_bounded_and_next_record_recovers() {
        let fixture = Fixture::new();
        fixture.append(&vec![b'x'; MAX_LINE + 1]);
        fixture.append(b"\n");
        let mut bytes = serde_json::to_vec(&record("session", "b123", "completed"))
            .expect("valid test fixture");
        bytes.push(b'\n');
        fixture.append(&bytes);
        let mut cursor = Cursor::default();
        let mut events = Vec::new();
        for _ in 0..6 {
            events.extend(cursor.observe(&fixture.0, "session"));
            assert!(cursor.partial.len() <= MAX_LINE);
        }
        assert!(
            events
                .iter()
                .any(|event| matches!(event, TaskEvent::Unavailable { .. }))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, TaskEvent::Finished { .. }))
        );
    }

    #[test]
    fn same_size_rewrite_resets_partial_and_rescans() {
        let fixture = Fixture::new();
        let mut first = serde_json::to_vec(&record("session", "b123", "completed"))
            .expect("valid test fixture");
        first.push(b'\n');
        fixture.append(&first);
        let mut cursor = Cursor::default();
        assert!(matches!(
            cursor.observe(&fixture.0, "session").as_slice(),
            [TaskEvent::Finished { .. }]
        ));
        let mut replacement = serde_json::to_vec(&record("session", "b456", "completed"))
            .expect("valid test fixture");
        replacement.push(b'\n');
        assert_eq!(first.len(), replacement.len());
        std::fs::write(&fixture.0, &replacement).expect("valid test fixture");
        assert!(matches!(cursor.observe(&fixture.0, "session").as_slice(),
            [TaskEvent::Unavailable { .. }, TaskEvent::Finished { task_id, .. }] if task_id == "b456"));
    }

    #[test]
    fn configured_home_path_is_used_without_discovery() {
        let request = TaskRequest {
            pane_id: "pane".into(),
            session_id: "session".into(),
            transcript_path: std::env::temp_dir().join("custom-claude-home/projects/session.jsonl"),
            host: ProbeHost::Local,
        };
        assert_eq!(source_path(&request), Some(request.transcript_path.clone()));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn wsl_transcript_paths_use_the_selected_distribution() {
        let mut request = TaskRequest {
            pane_id: "pane".into(),
            session_id: "session".into(),
            transcript_path: "/home/user/custom-claude/projects/session.jsonl".into(),
            host: ProbeHost::Wsl {
                distribution: "Ubuntu".into(),
            },
        };
        assert_eq!(
            source_path(&request),
            Some(PathBuf::from(
                r"\\wsl.localhost\Ubuntu\home\user\custom-claude\projects\session.jsonl"
            ))
        );
        request.host = ProbeHost::Wsl {
            distribution: String::new(),
        };
        assert_eq!(source_path(&request), None);
    }
}
