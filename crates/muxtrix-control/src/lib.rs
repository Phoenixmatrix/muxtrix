//! Typed local control protocol and reversible agent lifecycle integrations.

pub mod claude_journal;
mod hooks;
mod transport;

use serde::{Deserialize, Serialize};

pub use hooks::{Agent, HookAction, HookManager, HookScope, HookStatus, ManagedHookResult};
pub use transport::{
    ControlError, ControlNotifier, ControlServer, Endpoint, IncomingRequest, send_request,
};

/// Version of the control service and its `muxtrixctl` client.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum ControlRequest {
    Ping,
    Notify {
        title: String,
        body: String,
        pane_id: Option<String>,
    },
    AgentEvent {
        agent: String,
        state: AgentState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event: Option<String>,
        title: String,
        body: String,
        pane_id: Option<String>,
        session_id: Option<String>,
        cwd: Option<String>,
    },
    /// A Claude Code hook callback with its payload intact. Unlike
    /// [`ControlRequest::AgentEvent`], the state is not pre-decided by the
    /// installed command: the app derives it from the event name and fields,
    /// and merges it with Claude Code's own session record.
    ClaudeHook {
        pane_id: Option<String>,
        hook: ClaudeHook,
    },
    LaunchAgent {
        agent: Agent,
    },
    Split {
        direction: SplitDirection,
    },
    Focus {
        pane_id: String,
    },
    Close {
        pane_id: Option<String>,
    },
    SendText {
        text: String,
        pane_id: Option<String>,
    },
    Capture {
        pane_id: Option<String>,
    },
    ListPanes,
    /// Whether the e2e scenario has reached its capture point.
    ///
    /// GPUI can only render a window to an image on its test platform, so a
    /// headless capture is taken from outside the process. The harness polls
    /// this, grabs the frame itself, and then asks the app to quit.
    E2eStatus,
    /// End the process. Only the e2e harness sends this.
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Running,
    Waiting,
    Completed,
    Failed,
    Stopped,
    /// Activity cannot be established from the available evidence.
    Unknown,
}

/// The fields of a Claude Code hook payload that decide pane state or
/// identity. Everything is optional: hook payloads differ per event and the
/// harness may add or drop fields between releases.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeHook {
    /// `hook_event_name`, e.g. `UserPromptSubmit`, `PermissionRequest`.
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    /// Subagent lifecycle identity, shared by `SubagentStart` and `SubagentStop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// `Notification` payloads name their kind: `permission_prompt`,
    /// `idle_prompt`, `elicitation_dialog`, `auth_success`, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_assistant_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    /// Wall-clock milliseconds when the hook client sent this, on the same
    /// clock Claude Code stamps its session record with. Lets the app order a
    /// hook edge against the record that may lag or lead it.
    #[serde(default)]
    pub sent_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_tasks: Option<Box<[ClaudeBackgroundTask]>>,
    #[serde(default)]
    pub background_tasks_invalid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_crons: Option<Box<[ClaudeSessionCron]>>,
    #[serde(default)]
    pub session_crons_invalid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeBackgroundTask {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeSessionCron {
    pub id: String,
    pub schedule: String,
    pub recurring: bool,
}

fn bounded_snapshot<T: serde::de::DeserializeOwned>(
    payload: &serde_json::Value,
    key: &str,
    valid: impl Fn(&T) -> bool,
) -> (Option<Box<[T]>>, bool) {
    let Some(value) = payload.get(key) else {
        return (None, false);
    };
    let Some(items) = value.as_array() else {
        return (None, true);
    };
    let mut invalid = items.len() > 256;
    let mut result = Vec::with_capacity(items.len().min(256));
    for item in items.iter().take(256) {
        // Limit even unknown fields before allocating a typed item.
        if item.to_string().len() > 16 * 1024 {
            invalid = true;
            continue;
        }
        match T::deserialize(item) {
            Ok(item) if valid(&item) => result.push(item),
            _ => invalid = true,
        }
    }
    (Some(result.into_boxed_slice()), invalid)
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096
}

impl ClaudeHook {
    /// Builds the request from a raw hook payload as Claude Code writes it to
    /// the hook command's stdin.
    #[must_use]
    pub fn from_payload(payload: &serde_json::Value, event: &str) -> Self {
        let text = |key: &str| {
            payload
                .get(key)
                .and_then(serde_json::Value::as_str)
                .filter(|value| value.len() <= 8192)
                .map(str::to_owned)
        };
        let (background_tasks, background_tasks_invalid) = bounded_snapshot(
            payload,
            "background_tasks",
            |task: &ClaudeBackgroundTask| {
                valid_identity(&task.id)
                    && valid_identity(&task.kind)
                    && valid_identity(&task.status)
            },
        );
        let (session_crons, session_crons_invalid) =
            bounded_snapshot(payload, "session_crons", |cron: &ClaudeSessionCron| {
                valid_identity(&cron.id) && valid_identity(&cron.schedule)
            });
        Self {
            event: event.chars().take(256).collect(),
            session_id: text("session_id"),
            cwd: text("cwd"),
            tool_name: text("tool_name"),
            tool_use_id: text("tool_use_id"),
            agent_id: text("agent_id"),
            notification_type: text("notification_type").or_else(|| text("matcher")),
            permission_mode: text("permission_mode"),
            message: text("message"),
            last_assistant_message: text("last_assistant_message"),
            transcript_path: text("transcript_path"),
            sent_at_ms: 0,
            background_tasks,
            background_tasks_invalid,
            session_crons,
            session_crons_invalid,
            delivery_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    Right,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneSummary {
    pub pane_id: String,
    pub title: String,
    pub focused: bool,
    pub unread_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<PaneSummary>,
    /// Set only in reply to [`ControlRequest::E2eStatus`]: the scenario has
    /// settled on the frame it wants captured.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub capture_ready: bool,
}

impl ControlResponse {
    #[must_use]
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: Some(message.into()),
            text: None,
            panes: Vec::new(),
            capture_ready: false,
        }
    }

    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: Some(message.into()),
            text: None,
            panes: Vec::new(),
            capture_ready: false,
        }
    }

    /// The reply to [`ControlRequest::E2eStatus`].
    #[must_use]
    pub fn e2e_status(capture_ready: bool) -> Self {
        Self {
            ok: true,
            message: None,
            text: None,
            panes: Vec::new(),
            capture_ready,
        }
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;

    #[test]
    fn background_shell_snapshot_survives_extraction() {
        let hook = ClaudeHook::from_payload(
            &serde_json::json!({
                "background_tasks": [{"id":"shell-1","type":"shell","status":"running","ambient":false}],
                "session_crons": [{"id":"cron-1","schedule":"*/5 * * * *","recurring":true}]
            }),
            "Stop",
        );
        let tasks = hook.background_tasks.expect("valid test fixture");
        assert_eq!(tasks[0].id, "shell-1");
        assert_eq!(tasks[0].kind, "shell");
        assert_eq!(tasks[0].status, "running");
        assert_eq!(tasks[0].ambient, Some(false));
        assert!(!hook.background_tasks_invalid);
        assert!(hook.session_crons.expect("valid test fixture")[0].recurring);
    }

    #[test]
    fn missing_malformed_and_empty_snapshots_are_distinct() {
        let absent = ClaudeHook::from_payload(&serde_json::json!({}), "Stop");
        assert_eq!(absent.background_tasks, None);
        assert!(!absent.background_tasks_invalid);
        let empty = ClaudeHook::from_payload(&serde_json::json!({"background_tasks":[]}), "Stop");
        assert_eq!(empty.background_tasks, Some(Box::default()));
        assert!(!empty.background_tasks_invalid);
        for value in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!([{}]),
        ] {
            let hook = ClaudeHook::from_payload(
                &serde_json::json!({"background_tasks":value, "session_crons":value}),
                "Stop",
            );
            assert!(hook.background_tasks_invalid);
            assert!(hook.session_crons_invalid);
        }
    }

    #[test]
    fn valid_unknown_values_are_preserved_amid_invalid_entries() {
        let hook = ClaudeHook::from_payload(
            &serde_json::json!({
                "background_tasks": [
                    {"id":"future","type":"new-kind","status":"new-status","ambient":true},
                    {"id":"bad","type":"shell","status":"running","ambient":"yes"}
                ]
            }),
            "Stop",
        );
        assert!(hook.background_tasks_invalid);
        let tasks = hook.background_tasks.expect("valid test fixture");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].kind, "new-kind");
        assert_eq!(tasks[0].status, "new-status");
        assert_eq!(tasks[0].ambient, Some(true));
    }

    #[test]
    fn bounded_snapshots_report_incomplete_evidence() {
        let task = serde_json::json!({"id":"task","type":"shell","status":"running"});
        let hook = ClaudeHook::from_payload(
            &serde_json::json!({"background_tasks":vec![task; 257]}),
            "Stop",
        );
        assert!(hook.background_tasks_invalid);
        assert_eq!(
            hook.background_tasks.expect("valid test fixture").len(),
            256
        );
    }
}
