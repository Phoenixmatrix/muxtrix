//! Operating-system notifications for agents that need the user.
//!
//! Muxtrix already marks panes that want attention inside its own window.
//! These notices reach a user who has moved to another application: they are
//! opt-in, raised only while the window is in the background, and clicking
//! one brings the window forward on the pane that raised it.
//!
//! Deciding *whether* to notify is pure and lives here beside the copy, so it
//! is tested without a notification server. Delivery is the only part that
//! touches the platform: the freedesktop notification service on Linux, toast
//! notifications on Windows, and the notification center on macOS.

use std::time::{Duration, Instant};

use muxtrix_control::AgentState;
use muxtrix_domain::PaneId;

/// A pane that raised a notice of the same kind this recently stays quiet.
/// Screen classification can bounce between working and idle for a moment at
/// the edge of a turn; one notice per real edge is the goal.
pub(crate) const REPEAT_COOLDOWN: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    /// The agent stopped to ask a question or for permission.
    NeedsInput,
    /// The agent finished its turn and is back at its prompt.
    Finished,
    /// The agent's turn ended in an error.
    Failed,
}

/// The state edge that earns a notice, if any.
///
/// A pane Muxtrix has never seen before has no edge, except that an agent
/// first seen already waiting still needs someone. Leaving work — not merely
/// being idle — is what "finished" means, so an agent that was never running
/// finishes nothing.
pub(crate) fn notice_for(previous: Option<AgentState>, current: AgentState) -> Option<NoticeKind> {
    match (previous, current) {
        (Some(AgentState::Waiting), AgentState::Waiting) => None,
        (_, AgentState::Waiting) => Some(NoticeKind::NeedsInput),
        (Some(AgentState::Running), AgentState::Idle | AgentState::Completed) => {
            Some(NoticeKind::Finished)
        }
        (Some(AgentState::Running | AgentState::Waiting), AgentState::Failed) => {
            Some(NoticeKind::Failed)
        }
        _ => None,
    }
}

/// Whether the user asked to hear about this kind of edge.
///
/// An error stops the agent until someone looks, so it rides with the
/// requests for input rather than with ordinary turn ends.
pub(crate) fn wanted(kind: NoticeKind, settings: &crate::settings::AppSettings) -> bool {
    match kind {
        NoticeKind::NeedsInput | NoticeKind::Failed => settings.notify_when_waiting,
        NoticeKind::Finished => settings.notify_when_idle,
    }
}

/// One notice, as data, ready to hand to the platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    /// The pane a click should bring forward. `None` for the settings test.
    pub(crate) pane_id: Option<PaneId>,
    pub(crate) title: String,
    pub(crate) body: String,
}

/// Where the pane is, as the user named it: the pane title and, when it is
/// not the one in view, the workspace that holds it.
pub(crate) struct NoticeSubject<'a> {
    pub(crate) agent: &'a str,
    pub(crate) pane: &'a str,
    pub(crate) workspace: Option<&'a str>,
    /// What the agent said about itself, such as the permission it wants.
    pub(crate) activity: Option<&'a str>,
}

/// Title and body for a notice.
///
/// The title leads with the agent and what it needs, because a banner is
/// often read from its first line alone. The body says where — the pane and
/// workspace — and, when the agent said something more specific than its
/// state, that too.
pub(crate) fn compose(kind: NoticeKind, pane_id: PaneId, subject: &NoticeSubject<'_>) -> Notice {
    let agent = crate::app::agent_display_name(subject.agent);
    let title = match kind {
        NoticeKind::NeedsInput => format!("{agent} needs your input"),
        NoticeKind::Finished => format!("{agent} finished"),
        NoticeKind::Failed => format!("{agent} hit an error"),
    };
    let mut place = subject.pane.trim().to_owned();
    if place.is_empty() || place == agent {
        place.clear();
    }
    if let Some(workspace) = subject
        .workspace
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if place.is_empty() {
            place = workspace.to_owned();
        } else {
            place = format!("{place} · {workspace}");
        }
    }
    // Generic state copy repeats the title; only specific copy earns a line.
    let detail = subject
        .activity
        .map(str::trim)
        .filter(|activity| !activity.is_empty() && !is_generic_activity(activity))
        .map(|activity| crate::app::single_line_ellipsize(activity, 160));
    let body = match (place.is_empty(), detail) {
        (false, Some(detail)) => format!("{place}\n{detail}"),
        (false, None) => place,
        (true, Some(detail)) => detail,
        (true, None) => match kind {
            NoticeKind::NeedsInput => "Waiting for you in Muxtrix".into(),
            NoticeKind::Finished => "Ready for your next prompt".into(),
            NoticeKind::Failed => "Open Muxtrix to see what happened".into(),
        },
    };
    Notice {
        pane_id: Some(pane_id),
        title,
        body,
    }
}

/// The notice the settings page sends so the user can see what one looks like
/// and grant the operating system's permission before an agent needs it.
/// It claims nothing about the settings: the page may hold unapplied edits.
pub(crate) fn test_notice() -> Notice {
    Notice {
        pane_id: None,
        title: "Test from Muxtrix".into(),
        body: "Agent notifications will look like this".into(),
    }
}

fn is_generic_activity(activity: &str) -> bool {
    matches!(
        activity,
        "Ready for input"
            | "Agent working"
            | "Waiting for you"
            | "Waiting for input"
            | "Turn complete"
            | "Agent failed"
            | "Agent stopped"
            | "Idle"
    )
}

/// Per-pane memory of what was last seen and last announced.
#[derive(Debug, Default)]
pub(crate) struct NoticeTracker {
    states: std::collections::BTreeMap<PaneId, AgentState>,
    sent: std::collections::BTreeMap<(PaneId, u8), Instant>,
    /// Panes whose notice may still be on screen.
    shown: std::collections::BTreeSet<PaneId>,
}

impl NoticeTracker {
    /// Records `current` for a pane and returns the notice its edge earns,
    /// before any user preference or window focus is considered.
    pub(crate) fn observe(
        &mut self,
        pane_id: PaneId,
        current: AgentState,
        now: Instant,
    ) -> Option<NoticeKind> {
        let previous = self.states.insert(pane_id, current);
        if previous == Some(current) {
            return None;
        }
        let kind = notice_for(previous, current)?;
        let key = (pane_id, kind as u8);
        if self
            .sent
            .get(&key)
            .is_some_and(|sent| now.saturating_duration_since(*sent) < REPEAT_COOLDOWN)
        {
            return None;
        }
        self.sent.insert(key, now);
        Some(kind)
    }

    /// Forgets panes that no longer carry an agent, so one that starts again
    /// begins with no edge rather than inheriting a stale one. Returns the
    /// forgotten panes whose notice may still be on screen.
    pub(crate) fn retain(&mut self, mut live: impl FnMut(&PaneId) -> bool) -> Vec<PaneId> {
        if self.states.keys().all(&mut live) {
            return Vec::new();
        }
        self.states.retain(|pane_id, _| live(pane_id));
        self.sent.retain(|(pane_id, _), _| live(pane_id));
        let gone = self
            .shown
            .iter()
            .copied()
            .filter(|pane_id| !live(pane_id))
            .collect::<Vec<_>>();
        for pane_id in &gone {
            self.shown.remove(pane_id);
        }
        gone
    }

    pub(crate) fn mark_shown(&mut self, pane_id: PaneId) {
        self.shown.insert(pane_id);
    }

    /// Whether a notice for this pane may still be on screen; forgets it, so
    /// the caller withdraws it exactly once.
    pub(crate) fn take_shown(&mut self, pane_id: PaneId) -> bool {
        self.shown.remove(&pane_id)
    }
}

/// Stable identity the platform files Muxtrix's notices under: the Windows
/// AppUserModelID, and the name every platform shows as the sender.
pub(crate) const APP_ID: &str = "Muxtrix.Muxtrix";
pub(crate) const APP_NAME: &str = "Muxtrix";

/// The platform tag for a pane's notice. A newer notice for the same pane
/// replaces the older one, and a click carries the tag back.
pub(crate) fn tag(pane_id: Option<PaneId>) -> String {
    match pane_id {
        Some(pane_id) => format!("agent-pane:{}", pane_id.as_uuid()),
        None => "muxtrix-test".into(),
    }
}

/// The pane among `panes` that a clicked notice's tag names.
pub(crate) fn pane_for_tag(
    notice_tag: &str,
    panes: impl IntoIterator<Item = PaneId>,
) -> Option<PaneId> {
    panes
        .into_iter()
        .find(|pane_id| tag(Some(*pane_id)) == notice_tag)
}

/// Shows `notice` through the freedesktop notification service and blocks
/// until it is clicked or goes away; returns true for a click.
///
/// GPUI's own Linux notifications omit the icon and desktop-entry hints that
/// let a notification server show Muxtrix's icon and group its banners, and
/// they cannot replace a pane's earlier banner. The same `notify-rust` it uses
/// can do all three, so Linux delivers here. Runs on a worker thread: the
/// D-Bus calls are synchronous and a click can come minutes later.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn deliver(notice: &Notice) -> Result<bool, String> {
    let mut notification = notify_rust::Notification::new();
    notification
        .appname(APP_NAME)
        .summary(&notice.title)
        .body(&notice.body)
        .icon("muxtrix")
        .hint(notify_rust::Hint::DesktopEntry("muxtrix".into()))
        .hint(notify_rust::Hint::Category("im.received".into()))
        // A click on the banner itself invokes the "default" action.
        .action("default", "Show pane");
    if let Some(pane_id) = notice.pane_id {
        notification.id(replace_id(pane_id));
    }
    let handle = notification.show().map_err(|error| error.to_string())?;
    let mut clicked = false;
    handle.wait_for_action(|action| clicked = action == "default");
    Ok(clicked)
}

/// What a failed delivery means to the user. Without a notification service
/// on the session bus — the usual state of WSLg and bare window managers —
/// the D-Bus error names an interface nobody implements.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn delivery_failure(error: &str) -> String {
    if error.contains("ServiceUnknown")
        || error.contains("org.freedesktop.Notifications")
        || error.contains("DBUS_SESSION_BUS_ADDRESS")
    {
        "No notification service is running. Install one such as dunst or mako, or use a desktop that provides it.".into()
    } else {
        format!("The notification service refused it: {error}")
    }
}

/// Whether this process runs from inside an `.app` bundle.
#[cfg(target_os = "macos")]
pub(crate) fn running_from_app_bundle() -> bool {
    std::env::current_exe()
        .is_ok_and(|path| path.to_string_lossy().contains(".app/Contents/MacOS/"))
}

/// A notification id derived from the pane. Zero asks the server for a fresh
/// id, so it is never produced.
#[cfg(all(unix, not(target_os = "macos")))]
fn replace_id(pane_id: PaneId) -> u32 {
    let bytes = pane_id.as_uuid().as_u128();
    let folded = (bytes ^ (bytes >> 64)) as u64;
    ((folded ^ (folded >> 32)) as u32).max(1)
}

/// Gives Muxtrix's toast identity its icon.
///
/// GPUI registers the AppUserModelID with a display name only, which leaves
/// every toast with a blank tile. The icon is written beside the user's other
/// Muxtrix data because the registry can only point at a file.
#[cfg(target_os = "windows")]
pub(crate) fn register_windows_icon() {
    use std::sync::Once;

    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const ICON: &[u8] = include_bytes!("../assets/muxtrix-icon.png");
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {
        let register = || -> std::io::Result<()> {
            let directory = std::env::var_os("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .ok_or_else(|| std::io::Error::other("LOCALAPPDATA is not set"))?
                .join("Muxtrix");
            std::fs::create_dir_all(&directory)?;
            let icon = directory.join("notification-icon.png");
            if std::fs::read(&icon).ok().as_deref() != Some(ICON) {
                std::fs::write(&icon, ICON)?;
            }
            let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(format!(r"Software\Classes\AppUserModelId\{APP_ID}"))?;
            key.set_value("DisplayName", &APP_NAME)?;
            key.set_value("IconUri", &icon.display().to_string())?;
            Ok(())
        };
        if let Err(error) = register() {
            eprintln!("muxtrix: could not register the notification icon: {error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_edges_earn_notices() {
        use AgentState::*;
        assert_eq!(notice_for(Some(Running), Idle), Some(NoticeKind::Finished));
        assert_eq!(
            notice_for(Some(Running), Completed),
            Some(NoticeKind::Finished)
        );
        assert_eq!(
            notice_for(Some(Running), Waiting),
            Some(NoticeKind::NeedsInput)
        );
        assert_eq!(notice_for(None, Waiting), Some(NoticeKind::NeedsInput));
        assert_eq!(notice_for(Some(Running), Failed), Some(NoticeKind::Failed));
        // Being idle is not finishing; neither is answering a prompt.
        assert_eq!(notice_for(None, Idle), None);
        assert_eq!(notice_for(Some(Waiting), Idle), None);
        assert_eq!(notice_for(Some(Idle), Completed), None);
        assert_eq!(notice_for(Some(Waiting), Waiting), None);
        assert_eq!(notice_for(Some(Idle), Running), None);
    }

    #[test]
    fn a_bouncing_pane_is_announced_once() {
        let pane = PaneId::new();
        let start = Instant::now();
        let mut tracker = NoticeTracker::default();
        assert_eq!(tracker.observe(pane, AgentState::Running, start), None);
        assert_eq!(
            tracker.observe(pane, AgentState::Idle, start),
            Some(NoticeKind::Finished)
        );
        assert_eq!(tracker.observe(pane, AgentState::Running, start), None);
        assert_eq!(
            tracker.observe(pane, AgentState::Idle, start + Duration::from_secs(2)),
            None
        );
        assert_eq!(tracker.observe(pane, AgentState::Running, start), None);
        assert_eq!(
            tracker.observe(pane, AgentState::Idle, start + REPEAT_COOLDOWN),
            Some(NoticeKind::Finished)
        );
    }

    #[test]
    fn tags_round_trip_to_their_pane() {
        let pane = PaneId::new();
        let other = PaneId::new();
        assert_eq!(pane_for_tag(&tag(Some(pane)), [other, pane]), Some(pane));
        assert_eq!(pane_for_tag(&tag(None), [other, pane]), None);
    }

    #[test]
    fn notice_copy_names_the_agent_and_the_place() {
        let pane = PaneId::new();
        let notice = compose(
            NoticeKind::NeedsInput,
            pane,
            &NoticeSubject {
                agent: "claude",
                pane: "fix-login",
                workspace: Some("muxtrix"),
                activity: Some("Claude needs your permission to use Bash"),
            },
        );
        assert_eq!(notice.title, "Claude Code needs your input");
        assert_eq!(
            notice.body,
            "fix-login · muxtrix\nClaude needs your permission to use Bash"
        );
        assert_eq!(notice.pane_id, Some(pane));

        let generic = compose(
            NoticeKind::Finished,
            pane,
            &NoticeSubject {
                agent: "codex",
                pane: "Codex",
                workspace: None,
                activity: Some("Turn complete"),
            },
        );
        assert_eq!(generic.title, "Codex finished");
        assert_eq!(generic.body, "Ready for your next prompt");
    }
}
