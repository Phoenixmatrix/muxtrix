//! Typed local control protocol and reversible agent lifecycle integrations.

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
