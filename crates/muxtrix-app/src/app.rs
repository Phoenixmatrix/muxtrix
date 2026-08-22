//! The application: state, messages, and what every message means.
//!
//! `Muxtrix` is the whole application state and `update` is the only thing
//! that changes it. Both are deliberately free of rendering concerns — the
//! view layer reads this state in [`crate::views`], and side effects leave as
//! [`crate::effect::Effect`] values for the runtime in `main.rs` to carry out.

#[cfg(target_os = "windows")]
use std::collections::HashMap;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{Hash, Hasher};
#[cfg(target_os = "windows")]
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use iced::advanced::image::Handle as ImageHandle;
use iced::futures::StreamExt as _;
use iced::mouse;
use iced::widget::{
    button, canvas, column, container, mouse_area, rich_text, row, span, stack, svg, text, tooltip,
};
use iced::{
    Alignment, Border, Color, Element, Fill, Font, Length, Padding, Pixels, Shadow, Subscription,
    Theme, Vector, font,
};
use libghostty_vt::TerminalOptions;
use muxtrix_control::{
    Agent, AgentState, ControlRequest, ControlResponse, ControlServer, Endpoint, HookAction,
    HookManager, HookScope, HookStatus, PaneSummary, SplitDirection,
};
use muxtrix_domain::{
    LaunchProfile, Pane, PaneAgent, PaneId, PaneTree, ProcessBackend, ProfileId, SessionState,
    SplitAxis, SplitRatio, Surface, TabId, TerminalSurface, Workspace, WorkspaceId, WorkspaceTab,
};
use muxtrix_platform::{LaunchPlan, PtySize};
use muxtrix_terminal::{
    EventNotifier, GridSnapshot, ImageLayer, LiveSession, LiveSessionEvent, ScrollbarSnapshot,
    TerminalActor, TerminalMouseAction, TerminalMouseButton, TerminalMouseEvent,
    TerminalNotification, TerminalTheme,
};

#[cfg(feature = "e2e")]
use crate::e2e;
use crate::{
    agent_screen, agents_roster, box_drawing, commands, effect, github, input, metrics, settings,
    terminal_image,
};

use crate::commands::CommandAction;
use crate::effect::{Effect, FocusTarget, ScrollTarget};
use crate::geom::{Point, ScrollDelta, Size};
use crate::input::{Key, KeyEvent, KeyInput, Modifiers, Named};
#[cfg(test)]
use crate::layout::expanded_stack_pane;
use crate::layout::{
    enlarge_focused_tree, enlarge_focused_tree_toward, neighbor_pane, pane_ids_for_layout,
    pane_ids_in_layout, pane_layout_tree, pane_rects, same_panes, stacked_neighbor,
    zellij_resize_direction,
};
use crate::process::{
    HELPER_COMMAND_TIMEOUT, ProcessCancellation, command_output, console_command,
};
#[cfg(any(target_os = "windows", test))]
use crate::settings::WindowsShellBackend;
use crate::settings::{
    AppSettings, Appearance, DEFAULT_TERMINAL_SCROLLBACK_LINES, FleetScope, FleetView, FontWeight,
    InstalledFontCatalog, TerminalFont, UiFont, font_with_style,
};
#[cfg(test)]
use crate::terminal::runs::terminal_style_runs;
use crate::terminal::runs::{
    TerminalRunGeometry, TerminalRunKind, TerminalUnderlineDecoration, bold_size_scale, rgb,
    terminal_row_style_runs, terminal_run_geometry, terminal_underline_decoration,
};
use crate::theme::DesignTokens;
use crate::themes::{TerminalThemeId, TerminalThemePreset};

pub(crate) static NO_TERMINAL_STARTUP: AtomicBool = AtomicBool::new(false);

pub(crate) const TERMINAL_PADDING: f32 = 16.0;

/// Ignore the tiny pointer movement desktop toolkits can report during an
/// ordinary click. Crossing this distance turns the gesture into selection.
pub(crate) const TERMINAL_SELECTION_DRAG_THRESHOLD: f32 = 3.0;

/// How often the Claude Code roster is re-read while a pane projects it. The
/// read spawns a short-lived process, so it is paced well below the frame rate;
/// entering the view bypasses this for an immediate first read.
pub(crate) const AGENTS_ROSTER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Repository and linked-PR metadata is external state. Recheck it even when
/// the pane stays on the same directory and branch so new commits and PRs land.
pub(crate) const PANE_REPOSITORY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

pub(crate) const SIDEBAR_WIDTH: f32 = 272.0;

/// Width available to fleet entry copy: the rail less its 1px border, the
/// entry's own 16px horizontal padding, and a reserve so the ellipsis fires
/// before copy can sit flush against the edge.
pub(crate) const FLEET_ENTRY_TEXT_WIDTH: f32 = SIDEBAR_WIDTH - 1.0 - 16.0 - 12.0;

/// Advance of one character of mixed-case UI copy relative to its type size.
///
/// The UI face is proportional, so this is an average rather than a measurable
/// cell: wide copy runs past it and narrow copy stops short. Anywhere it sizes
/// something that must not displace its neighbours, back it with a real bound
/// (see the pane header's clipped title).
pub(crate) const UI_TEXT_ADVANCE_RATIO: f32 = 0.55;

pub(crate) const COLLAPSED_SIDEBAR_WIDTH: f32 = 46.0;

/// Anatomy of the rail footer's GitHub status. The mark and its login are sized
/// to read as a pair at the same weight as the rest of the rail rather than as
/// a hairline afterthought, so the icon matches the one the GitHub panel's own
/// header wears.
pub(crate) const GITHUB_STATUS_ICON_SIZE: f32 = 19.0;

pub(crate) const GITHUB_STATUS_DOT_SIZE: f32 = 7.0;

pub(crate) const GITHUB_STATUS_ROW_SPACING: f32 = 7.0;

/// Where the footer's login stops: the rail less its 1px border and the
/// footer's 16px of side padding, less the 31px collapse button, the status
/// button's own 16px padding, its icon, its two internal gaps and its dot, and
/// a reserve so the ellipsis lands before the two controls could sit flush
/// against each other. The login is measured against this cap rather than
/// estimated into it, so it is a real bound at every interface size.
pub(crate) const GITHUB_STATUS_LABEL_WIDTH: f32 = SIDEBAR_WIDTH
    - 1.0
    - 16.0
    - 31.0
    - 16.0
    - GITHUB_STATUS_ICON_SIZE
    - GITHUB_STATUS_ROW_SPACING * 2.0
    - GITHUB_STATUS_DOT_SIZE
    - 12.0;

pub(crate) const GITHUB_PANEL_WIDTH: f32 = 372.0;

pub(crate) const GITHUB_FILE_ROW_HEIGHT: f32 = 42.0;

pub(crate) const GITHUB_PULL_REQUEST_ROW_HEIGHT: f32 = 58.0;

pub(crate) const GITHUB_PULL_REQUEST_SEARCH_HEIGHT: f32 = 66.0;

pub(crate) const GITHUB_PULL_REQUEST_SUMMARY_HEIGHT: f32 = 34.0;

/// Keep the list viewport fixed while refresh hides search and summary chrome.
/// Otherwise centered loading copy jumps upward by half the removed height.
pub(crate) const GITHUB_PULL_REQUEST_LIST_CHROME_HEIGHT: f32 =
    GITHUB_PULL_REQUEST_SEARCH_HEIGHT + GITHUB_PULL_REQUEST_SUMMARY_HEIGHT + 1.0;

pub(crate) const GITHUB_FILE_OVERSCAN: usize = 5;

/// Keep installed-font menus scannable instead of letting a large system font
/// catalog consume the window. Iced's menu overlay uses the resulting height
/// when choosing the side with more available viewport space, and scrolls any
/// remaining options.
pub(crate) const FONT_FAMILY_MENU_MAX_HEIGHT: f32 = 320.0;

pub(crate) const SPLIT_HANDLE_SIZE: f32 = 8.0;

pub(crate) const PALETTE_INPUT_ID: &str = "muxtrix-command-palette-input";

pub(crate) const SETTINGS_SCROLL_ID: &str = "muxtrix-settings-scroll";

pub(crate) const PALETTE_SCROLL_ID: &str = "muxtrix-command-palette-scroll";

pub(crate) const GITHUB_FILE_SCROLL_ID: &str = "muxtrix-github-file-scroll";

pub(crate) const GITHUB_PULL_REQUEST_SCROLL_ID: &str = "muxtrix-github-pull-request-scroll";

pub(crate) const GITHUB_PULL_REQUEST_QUERY_ID: &str = "muxtrix-github-pull-request-query";

/// Deliberately not attached to any widget. Focusing it takes focus away from
/// the search field so list keys reach the panel handler.
pub(crate) const GITHUB_KEYBOARD_SINK_ID: &str = "muxtrix-github-keyboard-sink";

pub(crate) const GITHUB_DIFF_LINE_HEIGHT: f32 = 24.0;

pub(crate) const GITHUB_DIFF_OVERSCAN: usize = 16;

pub(crate) const GITHUB_DIFF_CHROME_WIDTH: f32 = 122.0;

pub(crate) const GITHUB_DIFF_MIN_WRAP_COLUMNS: usize = 80;

pub(crate) const GITHUB_LOADING_DOT_COUNT: u8 = 9;

pub(crate) const WORKSPACE_CREATE_INPUT_ID: &str = "muxtrix-workspace-create-input";

pub(crate) const RENAME_INPUT_ID: &str = "muxtrix-rename-input";

pub(crate) const WORKTREE_INPUT_ID: &str = "muxtrix-worktree-input";

pub(crate) const NO_REPO_GROUP: &str = "No Repo";

/// Worktrees live under a hidden app folder rather than littering the
/// visible home directory. The dot hides it on Linux/WSL; native Windows
/// additionally gets the Hidden attribute set at creation.
pub(crate) const WORKTREE_HOME_FOLDER: &str = ".muxtrix/worktrees";

/// Rungs in the keyboard cursor's leading bar. Odd, so the ladder opens and
/// closes on a filled rung and reads as a broken bar rather than a fade.
pub(crate) const RAIL_CURSOR_RUNGS: usize = 7;

/// Horizontal inset held clear inside every ellipsized lane, so truncated copy
/// stops short of the next lane instead of touching it.
pub(crate) const WORKTREE_LANE_INSET: f32 = 10.0;

pub(crate) const WORKTREE_LANE_SPACING: f32 = 16.0;

pub(crate) const WORKTREE_ROW_PADDING_X: f32 = 14.0;

/// The one horizontal margin the settings window keeps: its top bar, both page
/// bodies, and both footers share it, so every leading and trailing edge in
/// the surface lines up on the same two rules.
pub(crate) const SETTINGS_PAGE_PADDING_X: f32 = 28.0;

/// Horizontal padding inside the top bar's quiet return button. The bar's own
/// leading inset is short by exactly this, so the button's glyph rather than
/// its hit area lands on `SETTINGS_PAGE_PADDING_X`.
pub(crate) const SETTINGS_NAV_QUIET_PADDING_X: f32 = 10.0;

/// The type size shared by the settings top bar's return label and its title.
/// The two sit on one line either side of a rule, so they are read against each
/// other and must share a baseline; one size is the only way to hold that at
/// every interface family, weight, and type-size setting.
pub(crate) const SETTINGS_NAV_LABEL_POINTS: f32 = 11.0;

/// How many of those labels wide the settings window has to be before its top
/// bar keeps the long form of the return label. Below it the sentence would
/// close the gap between the title and the page switch, so the label shortens
/// to its noun and hands the sentence to a tooltip. Expressed in label widths
/// rather than pixels because every item on the bar grows with the interface
/// type size, so the width the bar needs grows with it too.
pub(crate) const SETTINGS_NAV_LABEL_WIDTHS: f32 = 38.0;

/// The gap the top bar's rule keeps from the words on either side of it. The
/// rule sits between a padded button and bare text, so the two sides cannot
/// share one row spacing: the leading gap is this minus the button's own
/// padding, and only then does the rule read as centred between the words.
pub(crate) const SETTINGS_NAV_RULE_GAP: f32 = 14.0;

/// The inventory reads as a table, so it takes the window's width rather than
/// a column measure. The cap only stops the action lane from drifting a whole
/// screen away from the identity it acts on.
pub(crate) const WORKTREE_PAGE_MAX_WIDTH: f32 = 1480.0;

/// Reserve for the scrollable's overlay scrollbar, which draws over the
/// content's right edge rather than displacing it.
pub(crate) const WORKTREE_SCROLLBAR_RESERVE: f32 = 12.0;

/// Header chrome the title can never have: the band's 12/6 padding, the signal
/// dot, the five 8px gaps in the row, and a reserve so proportional copy stops
/// short of the state label instead of crowding it.
pub(crate) const PANE_HEADER_FIXED_WIDTH: f32 = 12.0 + 6.0 + 6.0 + 8.0 * 5.0 + 12.0;

pub(crate) const PANE_HEADER_ICON_BUTTON: f32 = 24.0;

pub(crate) const PANE_HEADER_DIVIDER: f32 = 1.0;

pub(crate) const PANE_HEADER_CONTROL_SPACING: f32 = 2.0;

/// Horizontal padding of the command chip and of a labelled header button.
pub(crate) const PANE_HEADER_CHIP_PADDING: f32 = 12.0;

pub(crate) const PANE_HEADER_LABEL_PADDING: f32 = 16.0;

/// Even a pane too narrow to hold its own chrome shows some title; a header
/// that tight is already compact and has shed its chip and state label.
pub(crate) const PANE_TITLE_MIN_WIDTH: f32 = 48.0;

/// Budget for the frames before a pane has reported its size — wide enough not
/// to bind, since the character budget still truncates.
pub(crate) const PANE_TITLE_UNMEASURED_WIDTH: f32 = 4_096.0;

pub(crate) static SESSION_HOST: std::sync::Mutex<Option<SessionHost>> = std::sync::Mutex::new(None);

pub(crate) const SHELL_INTEGRATION_ZSH_DIR: &str = ".local/share/muxtrix/shell-integration/zsh";

pub(crate) struct Muxtrix {
    pub(crate) session: SessionState,
    pub(crate) terminals: BTreeMap<PaneId, TerminalRuntime>,
    pub(crate) status: String,
    pub(crate) cursor_phase_visible: bool,
    pub(crate) active_view: ActiveView,
    pub(crate) settings_page: SettingsPage,
    pub(crate) palette: CommandPalette,
    pub(crate) settings: AppSettings,
    pub(crate) settings_draft: AppSettings,
    pub(crate) settings_scrollback_lines_input: String,
    pub(crate) installed_versions: InstalledVersionsState,
    pub(crate) installed_muxtrix_path: Result<std::path::PathBuf, String>,
    pub(crate) installed_fonts: InstalledFontCatalog,
    pub(crate) available_terminal_fonts: Vec<TerminalFont>,
    pub(crate) available_terminal_font_weights: Vec<FontWeight>,
    pub(crate) available_ui_fonts: Vec<UiFont>,
    pub(crate) available_ui_font_weights: Vec<FontWeight>,
    pub(crate) available_wsl_distributions: Vec<WslDistributionChoice>,
    pub(crate) workspace_name_draft: String,
    pub(crate) rename_prompt: Option<RenameTarget>,
    pub(crate) rename_draft: String,
    pub(crate) default_agent_prompt: bool,
    pub(crate) pending_default_agent_command: Option<CommandAction>,
    pub(crate) worktree_prompt: Option<WorktreePrompt>,
    pub(crate) worktree_name_draft: String,
    pub(crate) worktree_manager: Option<WorktreeManagerState>,
    pub(crate) worktree_manager_generation: u64,
    pub(crate) session_picker: Option<SessionPickerState>,
    pub(crate) last_layout_hash: u64,
    /// Transient confirmation pill (bottom-center, iTerm2/Ghostty style);
    /// cleared by the blink tick once it has been visible ~2s. Keyboard-mode
    /// guidance is derived from `prefix_armed`/`rail_nav` instead, so it stays
    /// visible for the full lifetime of those modes.
    pub(crate) toast: Option<(String, std::time::Instant)>,
    /// Ctrl+G was pressed; a recognized follow-up picks an unlocked action
    /// and Escape cancels (Zellij-style prefix).
    pub(crate) prefix_armed: bool,
    /// Keyboard cursor walking the rail (workspaces, then fleet).
    pub(crate) rail_nav: Option<RailTarget>,
    pub(crate) workspace_create_visible: bool,
    pub(crate) close_workspace_prompt: Option<WorkspaceId>,
    pub(crate) tab_drag: Option<TabDrag>,
    pub(crate) notifications: Vec<AgentNotification>,
    pub(crate) global_alerts: Vec<GlobalAlert>,
    pub(crate) github_auth: github::AuthStatus,
    pub(crate) github_auth_busy: bool,
    pub(crate) github_auth_generation: u64,
    pub(crate) github_auth_cancellation: ProcessCancellation,
    pub(crate) github_request_generation: u64,
    pub(crate) github_panel: Option<GitHubPanelState>,
    pub(crate) github_diff: Option<GitHubDiffState>,
    pub(crate) github_pane_refresh_pending: bool,
    pub(crate) github_pull_requests_refresh_pending: bool,
    pub(crate) github_context_generation: u64,
    pub(crate) github_context_cancellation: ProcessCancellation,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) maximized_pane: Option<PaneId>,
    pub(crate) pane_menu: Option<PaneId>,
    /// Whether a window exists yet. Some effects — constraining resizes to
    /// whole cells — are meaningless before one does.
    pub(crate) window_open: bool,
    pub(crate) window_size: Size,
    pub(crate) window_focused: bool,
    pub(crate) cursor_position: Point,
    pub(crate) keyboard_modifiers: Modifiers,
    pub(crate) split_sizes: BTreeMap<SplitKey, Size>,
    pub(crate) split_drag: Option<SplitDrag>,
    pub(crate) pane_layouts: BTreeMap<TabId, PaneLayout>,
    pub(crate) base_pane_layouts: BTreeMap<TabId, PaneTree>,
    pub(crate) pane_resize_history: BTreeMap<TabId, PaneResizeHistory>,
    pub(crate) hovered_terminal: Option<PaneId>,
    pub(crate) terminal_pointer_positions: BTreeMap<PaneId, Point>,
    pub(crate) terminal_scrollbar_positions: BTreeMap<PaneId, Point>,
    pub(crate) terminal_scroll_drag: Option<TerminalScrollDrag>,
    /// A button press currently owned by a mouse-reporting terminal program.
    /// Keeping this at the host level lets a release outside the pane still
    /// reach the program that accepted the press.
    pub(crate) terminal_mouse_capture: Option<TerminalMouseCapture>,
    /// A possible terminal selection gesture. The emulator owns an actual
    /// selection only after this crosses the drag threshold; an ordinary
    /// click merely clears the previous selection and focuses the pane.
    pub(crate) terminal_selection_drag: Option<TerminalSelectionDrag>,
    pub(crate) terminal_command_buffers: BTreeMap<PaneId, String>,
    pub(crate) event_notifier: EventNotifier,
    pub(crate) event_receiver: async_channel::Receiver<()>,
    pub(crate) terminal_launcher: Arc<dyn TerminalLauncher>,
    pub(crate) terminal_launch_completions: Arc<Mutex<VecDeque<TerminalLaunchCompletion>>>,
    pub(crate) next_terminal_launch_attempt: u64,
    pub(crate) launch_in_background: bool,
    pub(crate) startup_terminal_pending: Option<PaneId>,
    /// A restart requested while this pane's first launch is still in flight.
    /// Daemon launches reuse the durable pane ID, so replacements must wait
    /// for the current worker to hand its session back before starting.
    pub(crate) queued_terminal_restarts: BTreeSet<PaneId>,
    pub(crate) pending_terminal_input: BTreeMap<PaneId, Vec<Vec<u8>>>,
    pub(crate) control: Option<ControlServer>,
    pub(crate) control_endpoint: Option<String>,
    pub(crate) agent_statuses: BTreeMap<PaneId, AgentPaneStatus>,
    /// Terminal-frame revision that was current when a pane most recently
    /// entered Running. Outside Pi's exact active lifecycle, an Idle
    /// classification may only demote it after a newer frame arrives; this
    /// preserves the hook/frame race guard without making Running sticky.
    pub(crate) agent_running_frame_revisions: BTreeMap<PaneId, u64>,
    /// Pi panes between an exact `agent_start` and terminal `agent_end`.
    /// Pi's lifecycle owns this interval: an erroneously idle OSC title must
    /// not demote work that the harness still reports as active.
    pub(crate) pi_active_lifecycles: BTreeSet<PaneId>,
    /// Panes where process-tree detection has observed a live agent, with the
    /// instant it last saw that process. Lifecycle hooks enrich the same
    /// status but do not disable exit observation; the entry self-cleans after
    /// the harness process remains absent.
    pub(crate) detected_agents: BTreeMap<PaneId, std::time::Instant>,
    /// Panes currently showing Claude Code's Agents view. Their rows project
    /// `agents_roster` instead of one conversation's lifecycle.
    pub(crate) agents_view_panes: BTreeSet<PaneId>,
    /// Latest machine-wide roster, or `None` until the first read lands. An
    /// error leaves the previous roster in place rather than inventing counts.
    pub(crate) agents_roster: Option<agents_roster::AgentsRoster>,
    /// One roster read at a time: polls must not stack up behind a slow harness.
    pub(crate) agents_roster_pending: bool,
    /// Why the last read failed, while no roster has ever landed. A roll-up
    /// that cannot run — the configured Claude Code is not on this process's
    /// `PATH`, say — otherwise leaves the row saying `Agents` forever with
    /// nothing to act on.
    pub(crate) agents_roster_error: Option<String>,
    /// When the roster was last read. Cleared on entering the view so the first
    /// read is immediate rather than waiting out the cadence.
    pub(crate) agents_roster_checked: Option<std::time::Instant>,
    /// Repository labels resolved from each pane's live working directory.
    /// Resolution runs off the UI thread because WSL panes require git inside
    /// the selected distribution. Empty results are cached too, keeping
    /// ordinary terminal repaints free of filesystem and process work.
    pub(crate) pane_repositories: BTreeMap<PaneId, PaneRepository>,
    pub(crate) pending_repository_directories: BTreeMap<PaneId, std::path::PathBuf>,
    pub(crate) pane_repository_generation: u64,
    pub(crate) pane_repository_cancellation: ProcessCancellation,
    pub(crate) hook_statuses: Vec<HookStatus>,
    pub(crate) integration_generation: u64,
    pub(crate) integration_refreshing: bool,
    #[cfg(feature = "e2e")]
    pub(crate) e2e: Option<e2e::Scenario>,
}

pub(crate) struct TerminalRuntime {
    pub(crate) preview: String,
    pub(crate) snapshot: Option<GridSnapshot>,
    pub(crate) snapshot_revision: u64,
    pub(crate) image_handles: BTreeMap<u64, ImageHandle>,
    pub(crate) session: Option<LiveSession>,
    pub(crate) fallback_title: String,
    pub(crate) display_title: String,
    pub(crate) size: PtySize,
    pub(crate) viewport: Option<Size>,
    pub(crate) launch_state: TerminalLaunchState,
    /// Whether this pane has a selection worth offering to copy. The emulator
    /// holds the selection itself; this only spares the view a round trip to
    /// the session thread on every frame.
    pub(crate) has_selection: bool,
}

impl Drop for TerminalRuntime {
    fn drop(&mut self) {
        // A session actor can be inside a blocked host/IPC operation when the
        // window or pane is closed. `LiveSession::drop` joins that actor, so
        // doing it here would make Iced's UI teardown wait indefinitely — and
        // one busy pane would hold up every runtime dropped after it. Detach on
        // a disposal thread instead; explicit pane close has already queued
        // `terminate`, while whole-app shutdown preserves daemon-owned PTYs.
        if let Some(session) = self.session.take() {
            dispose_live_session(session);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalLaunchState {
    PreparingHost,
    Starting { attempt_id: u64 },
    Running,
    Failed(String),
    Suppressed,
    Exited,
}

pub(crate) struct TerminalLaunchRequest {
    pub(crate) profile: LaunchProfile,
    pub(crate) directory_policy: CreationDirectoryPolicy,
    pub(crate) wsl_distribution: String,
    pub(crate) pane_id: PaneId,
    pub(crate) theme: TerminalTheme,
    pub(crate) max_scrollback: usize,
    pub(crate) notifier: EventNotifier,
    pub(crate) control_endpoint: Option<String>,
    pub(crate) target_size: PtySize,
    pub(crate) cell_width_px: f32,
    pub(crate) cell_height_px: f32,
    pub(crate) previous_session: Option<LiveSession>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreationDirectoryPolicy {
    /// Use the profile directory exactly. Explicit worktree creation and pane
    /// restarts must never be redirected away from their requested target.
    Exact,
    /// Keep an ordinary directory, but leave a linked worktree for the
    /// repository's GitHub-default checkout before starting the shell.
    Regular,
}

pub(crate) trait TerminalLauncher: Send + Sync {
    fn launch(&self, request: TerminalLaunchRequest) -> Result<LaunchedTerminal, String>;

    /// Why the terminal host refused to start this pane's process, when it
    /// did. A daemon spawn is a request written to a socket: it reports
    /// success as soon as the host accepts it, and a refusal arrives later.
    fn spawn_failure(&self, _pane_id: PaneId) -> Option<String> {
        None
    }
}

#[derive(Default)]
pub(crate) struct SystemTerminalLauncher {
    /// Tests can inject a daemon client without publishing it through the
    /// process-global application host used by unrelated parallel tests.
    pub(crate) client: Option<Arc<muxtrix_sessions::SessionClient>>,
}

impl TerminalLauncher for SystemTerminalLauncher {
    fn launch(&self, request: TerminalLaunchRequest) -> Result<LaunchedTerminal, String> {
        let pane_id = request.pane_id;
        self.launch_session(request).map_err(|error| {
            // A refused spawn takes the pane's byte channel down with it, so
            // what surfaces here is usually the channel closing rather than
            // the refusal. Prefer the host's own reason, allowing for it
            // arriving just after the symptom did.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
            loop {
                if let Some(reason) = self.spawn_failure(pane_id) {
                    return reason;
                }
                if std::time::Instant::now() >= deadline {
                    return error;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        })
    }

    fn spawn_failure(&self, pane_id: PaneId) -> Option<String> {
        self.client
            .clone()
            .or_else(|| session_host().map(|(_, client)| client))?
            .pane_spawn_failure(pane_id.as_uuid())
    }
}

impl SystemTerminalLauncher {
    pub(crate) fn launch_session(
        &self,
        request: TerminalLaunchRequest,
    ) -> Result<LaunchedTerminal, String> {
        let mut profile = request.profile.clone();
        if request.directory_policy == CreationDirectoryPolicy::Regular
            && let Some(directory) = profile.working_directory.as_deref()
        {
            profile.working_directory = Some(resolve_regular_creation_directory(
                directory,
                &request.wsl_distribution,
            ));
        }
        if let Some(previous_session) = request.previous_session {
            previous_session.terminate();
            drop(previous_session);
        }
        let session = start_live_session_with_client(
            &profile,
            request.pane_id,
            request.max_scrollback,
            request.theme,
            request.notifier,
            request.control_endpoint.as_deref(),
            self.client
                .clone()
                .or_else(|| session_host().map(|(_, client)| client)),
        )?;
        if terminal_grid_changed(initial_pty_size(), request.target_size) {
            session
                .resize(
                    request.target_size,
                    request.cell_width_px,
                    request.cell_height_px,
                )
                .map_err(|error| error.to_string())?;
        }
        let snapshot = session.snapshot().map_err(|error| error.to_string())?;
        Ok(LaunchedTerminal {
            session,
            snapshot,
            size: request.target_size,
            working_directory: profile.working_directory,
        })
    }
}

pub(crate) struct LaunchedTerminal {
    pub(crate) session: LiveSession,
    pub(crate) snapshot: GridSnapshot,
    pub(crate) size: PtySize,
    pub(crate) working_directory: Option<std::path::PathBuf>,
}

pub(crate) struct TerminalLaunchCompletion {
    pub(crate) pane_id: PaneId,
    pub(crate) attempt_id: u64,
    pub(crate) result: Result<LaunchedTerminal, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActiveView {
    Workspace,
    Settings,
    /// Full-screen theme browsing, entered from Settings; every preset
    /// renders as a live terminal preview and Esc/Back returns.
    ThemeGallery,
    /// Full-window unified diff with the GitHub file panel retained at right.
    GitHubDiff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsPage {
    Preferences,
    Worktrees,
}

#[derive(Debug, Clone)]
pub(crate) struct InstalledVersions {
    pub(crate) muxtrix: Result<String, String>,
    pub(crate) muxtrixctl: Result<String, String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) enum InstalledVersionsState {
    #[default]
    Unchecked,
    Checking,
    Ready(InstalledVersions),
    Unavailable,
}

#[derive(Debug, Clone)]
pub(crate) struct GitHubPanelState {
    pub(crate) repository: github::Repository,
    pub(crate) active_tab: GitHubPanelTab,
    pub(crate) context_loading: bool,
    pub(crate) data: Option<github::PanelData>,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) pull_requests: Option<Vec<github::PullRequestSummary>>,
    pub(crate) pull_requests_loading: bool,
    pub(crate) pull_requests_error: Option<String>,
    pub(crate) pull_request_query: String,
    pub(crate) pull_request_scroll_offset: f32,
    pub(crate) pull_request_keyboard_cursor: Option<usize>,
    pub(crate) keyboard_focus: Option<GitHubPanelKeyboardFocus>,
    pub(crate) selected_pull_request_number: Option<u64>,
    pub(crate) selected_pull_request: Option<github::PullRequestDetails>,
    pub(crate) selected_pull_request_loading: bool,
    pub(crate) selected_pull_request_error: Option<String>,
    pub(crate) selected_pull_request_file_scroll_offset: f32,
    pub(crate) file_keyboard_cursor: Option<usize>,
    pub(crate) merge_confirmation: bool,
    pub(crate) pull_request_action_error: Option<String>,
    pub(crate) draft_state_updating: bool,
    pub(crate) merging: bool,
    pub(crate) file_scroll_offset: f32,
    pub(crate) pull_request_generation: u64,
    pub(crate) pull_request_detail_generation: u64,
    pub(crate) action_generation: u64,
    pub(crate) pull_requests_cancellation: ProcessCancellation,
    pub(crate) pull_request_detail_cancellation: ProcessCancellation,
    pub(crate) action_cancellation: ProcessCancellation,
    pub(crate) loading_phase: u8,
}

impl GitHubPanelState {
    pub(crate) fn loading(repository: github::Repository) -> Self {
        Self {
            repository,
            active_tab: GitHubPanelTab::Local,
            context_loading: false,
            data: None,
            loading: true,
            error: None,
            pull_requests: None,
            pull_requests_loading: false,
            pull_requests_error: None,
            pull_request_query: String::new(),
            pull_request_scroll_offset: 0.0,
            pull_request_keyboard_cursor: None,
            keyboard_focus: None,
            selected_pull_request_number: None,
            selected_pull_request: None,
            selected_pull_request_loading: false,
            selected_pull_request_error: None,
            selected_pull_request_file_scroll_offset: 0.0,
            file_keyboard_cursor: None,
            pull_request_action_error: None,
            draft_state_updating: false,
            merge_confirmation: false,
            merging: false,
            file_scroll_offset: 0.0,
            pull_request_generation: 0,
            pull_request_detail_generation: 0,
            action_generation: 0,
            pull_requests_cancellation: ProcessCancellation::default(),
            pull_request_detail_cancellation: ProcessCancellation::default(),
            action_cancellation: ProcessCancellation::default(),
            loading_phase: 0,
        }
    }

    pub(crate) fn active_loading(&self) -> bool {
        if self.context_loading || self.merging || self.draft_state_updating {
            return true;
        }
        match self.active_tab {
            GitHubPanelTab::Local => self.loading,
            GitHubPanelTab::PullRequests if self.selected_pull_request_number.is_some() => {
                self.selected_pull_request_loading
            }
            GitHubPanelTab::PullRequests => self.pull_requests_loading,
        }
    }

    pub(crate) fn cancel_requests(&self) {
        self.pull_requests_cancellation.cancel();
        self.pull_request_detail_cancellation.cancel();
        self.action_cancellation.cancel();
    }

    pub(crate) fn close_selected_pull_request(&mut self) {
        self.pull_request_detail_cancellation.cancel();
        self.action_cancellation.cancel();
        self.selected_pull_request_number = None;
        self.selected_pull_request = None;
        self.selected_pull_request_loading = false;
        self.selected_pull_request_error = None;
        self.selected_pull_request_file_scroll_offset = 0.0;
        self.pull_request_action_error = None;
        self.draft_state_updating = false;
        self.file_keyboard_cursor = None;
        self.keyboard_focus = Some(GitHubPanelKeyboardFocus::PullRequestList);
        self.merge_confirmation = false;
    }

    pub(crate) fn mark_pull_request_merged(&mut self, number: u64) {
        let Some(pull_requests) = self.pull_requests.as_mut() else {
            return;
        };
        let Some(pull_request) = pull_requests
            .iter_mut()
            .find(|pull_request| pull_request.number == number)
        else {
            return;
        };
        pull_request.status = github::PullRequestSummaryStatus::Merged;
    }

    pub(crate) fn mark_pull_request_draft(&mut self, number: u64, draft: bool) {
        let readiness = self
            .selected_pull_request
            .as_mut()
            .filter(|details| details.pull_request.number == number)
            .map(|details| {
                details.pull_request.draft = draft;
                details.pull_request.readiness()
            });
        let Some(pull_request) = self.pull_requests.as_mut().and_then(|pull_requests| {
            pull_requests
                .iter_mut()
                .find(|pull_request| pull_request.number == number)
        }) else {
            return;
        };
        pull_request.status = if draft {
            github::PullRequestSummaryStatus::Draft
        } else {
            github::PullRequestSummaryStatus::Open
        };
        pull_request.readiness = readiness.unwrap_or(if draft {
            github::MergeReadiness::Draft
        } else {
            github::MergeReadiness::Unknown
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitHubContextLoad {
    Open,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitHubPanelTab {
    Local,
    PullRequests,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitHubPanelKeyboardFocus {
    Tabs,
    Search,
    PullRequestList,
    Back,
    DraftAction,
    MergeAction,
    Files,
}

#[derive(Debug, Clone)]
pub(crate) struct GitHubDiffState {
    pub(crate) source: GitHubDiffSource,
    pub(crate) path: String,
    pub(crate) status: String,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
    pub(crate) document: Option<github::DiffDocument>,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) generation: u64,
    pub(crate) cancellation: ProcessCancellation,
    pub(crate) scroll_offset: f32,
    pub(crate) wrap_columns: Option<usize>,
    pub(crate) line_starts: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitHubDiffSource {
    Local,
    PullRequest(u64),
}

#[derive(Debug, Clone)]
pub(crate) struct GlobalAlert {
    pub(crate) title: String,
    pub(crate) body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameTarget {
    Workspace(WorkspaceId),
    Tab(WorkspaceId, TabId),
    Pane(PaneId),
}

#[derive(Debug, Clone)]
pub(crate) struct WorktreePrompt {
    pub(crate) target: WorktreePromptTarget,
    /// None when the focused pane is not inside a git repository — the
    /// dialog still opens and says so, because the status bar is hidden by
    /// default and a status-line message would be invisible.
    pub(crate) repo_root: Option<std::path::PathBuf>,
    /// Why creation is impossible, diagnosed when the dialog opened: no
    /// shell-reported directory, unreachable WSL distribution, vanished
    /// directory, missing git, or genuinely not a repository.
    pub(crate) failure: Option<String>,
    /// Where new worktrees for this repository land, resolved when the
    /// dialog opens (on Windows+WSL this costs a wsl.exe launch).
    pub(crate) base_directory: Option<std::path::PathBuf>,
    /// Directory names already present under `base_directory`, listed once
    /// at open so the per-keystroke conflict check never touches the
    /// filesystem.
    pub(crate) taken_names: BTreeSet<String>,
    /// Inline error shown in the dialog (name conflicts, git failures).
    pub(crate) error: Option<String>,
    /// A creation is in flight; the dialog stays open until it resolves.
    pub(crate) busy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreePromptTarget {
    Open(commands::WorktreeKind),
    OpenWithAgent(commands::WorktreeKind, Agent),
    RestartPane(PaneId),
    RestartPaneWithAgent(PaneId, Agent),
}

/// A rail entry the prefix-key navigation cursor can rest on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RailTarget {
    Workspace(WorkspaceId),
    FleetWorkspace(WorkspaceId),
    FleetTab(WorkspaceId, TabId),
    FleetGroup(WorkspaceId, PaneId),
    FleetPane(WorkspaceId, PaneId),
}

#[derive(Debug, Clone)]
pub(crate) struct WorktreeManagerState {
    pub(crate) mode: WorktreeManagerMode,
    pub(crate) generation: u64,
    /// None when the focused pane is not inside a repository; `failure`
    /// carries the explanation instead.
    pub(crate) repo_root: Option<std::path::PathBuf>,
    pub(crate) failure: Option<String>,
    pub(crate) entries: Vec<WorktreeManagerEntry>,
    /// Repository and per-checkout Git metadata are loaded off the UI thread.
    pub(crate) loading: bool,
    pub(crate) selected: usize,
    pub(crate) busy: bool,
    pub(crate) error: Option<String>,
    /// The selected row while the destructive restart confirmation is open.
    pub(crate) restart_target: Option<usize>,
}

impl WorktreeManagerState {
    pub(crate) fn loading(mode: WorktreeManagerMode, generation: u64) -> Self {
        Self {
            mode,
            generation,
            repo_root: None,
            failure: None,
            entries: Vec::new(),
            loading: true,
            selected: 0,
            busy: false,
            error: None,
            restart_target: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorktreeManagerDiscovery {
    pub(crate) repo_root: Option<std::path::PathBuf>,
    pub(crate) failure: Option<String>,
    pub(crate) entries: Vec<WorktreeManagerEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeManagerMode {
    Manage,
    RestartPane(PaneId),
    RestartPaneWithAgent(PaneId, Agent),
}

pub(crate) struct SessionPickerState {
    pub(crate) entries: Vec<SessionPickerEntry>,
    pub(crate) selected: usize,
    pub(crate) error: Option<String>,
    /// Opened before any new daemon is created because unattached sessions
    /// exist; declining it explicitly starts a fresh session.
    pub(crate) startup: bool,
}

pub(crate) struct SessionPickerEntry {
    pub(crate) record: muxtrix_sessions::SessionRecord,
    pub(crate) alive: bool,
    pub(crate) pane_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct WorktreeManagerEntry {
    pub(crate) path: std::path::PathBuf,
    pub(crate) branch: Option<String>,
    /// Commits reachable from this checkout's HEAD but from no configured
    /// remote. This is local-only status: opening the manager never fetches.
    pub(crate) unpushed_commits: usize,
    /// Why deletion is forbidden. The primary worktree is always protected;
    /// removing a linked worktree leaves its branch intact.
    pub(crate) deletion_blocker: Option<String>,
    /// Title of a pane currently working inside this worktree, if any —
    /// such worktrees cannot be deleted.
    pub(crate) used_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipboardAction {
    Copy,
    Paste,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneLayout {
    Base,
    Vertical,
    Horizontal,
    Stacked,
    HalfStacked,
}

impl PaneLayout {
    const ALL: [Self; 5] = [
        Self::Base,
        Self::Vertical,
        Self::Horizontal,
        Self::Stacked,
        Self::HalfStacked,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Base => "Base",
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
            Self::Stacked => "Stacked",
            Self::HalfStacked => "Half-stacked",
        }
    }

    pub(crate) const fn supports(self, pane_count: usize) -> bool {
        match self {
            Self::Base | Self::Vertical | Self::Horizontal => pane_count >= 2,
            // Muxtrix applies constraints to terminal panes only and keeps
            // the useful two-pane stack available.
            Self::Stacked => pane_count >= 2,
            Self::HalfStacked => pane_count >= 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutCycle {
    Previous,
    Next,
}

#[derive(Debug, Clone)]
pub(crate) struct PaneResizeSnapshot {
    pub(crate) root: PaneTree,
    pub(crate) maximized_pane: Option<PaneId>,
}

#[derive(Debug, Clone)]
pub(crate) struct PaneResizeHistory {
    pub(crate) pane_id: PaneId,
    pub(crate) snapshots: Vec<PaneResizeSnapshot>,
}

/// A pane's normalized rectangle within its tab, derived from split ratios.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PaneRect {
    pub(crate) pane_id: PaneId,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentNotification {
    pub(crate) pane_id: PaneId,
    pub(crate) unread: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentPaneStatus {
    pub(crate) agent: String,
    /// Best pane-local identity below an explicit user rename: a title emitted
    /// by the harness, or the linked-worktree directory while it starts.
    pub(crate) display_name: Option<String>,
    pub(crate) state: AgentState,
    pub(crate) activity: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) git_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneRepository {
    pub(crate) directory: std::path::PathBuf,
    pub(crate) root: Option<std::path::PathBuf>,
    pub(crate) name: Option<String>,
    pub(crate) worktree_name: Option<String>,
    pub(crate) branch: Option<String>,
    /// The agent-reported branch this probe was answering, so a
    /// later report of a different branch can invalidate it.
    pub(crate) reported_branch: Option<String>,
    pub(crate) head_oid: Option<String>,
    pub(crate) pull_request: Option<github::CurrentPullRequest>,
    pub(crate) checked_at: std::time::Instant,
}

pub(crate) fn pane_repository_pull_request_is_relevant(
    repository: &PaneRepository,
    directory: &std::path::Path,
    reported_branch: Option<&str>,
) -> bool {
    let directory_matches = repository.directory == directory
        || repository
            .root
            .as_deref()
            .is_some_and(|root| directory.starts_with(root));
    directory_matches
        && reported_branch.is_none_or(|reported_branch| {
            repository.reported_branch.as_deref() == Some(reported_branch)
        })
}

pub(crate) fn current_pull_request_after_refresh(
    cached: Option<github::CurrentPullRequest>,
    refresh: Result<Option<github::CurrentPullRequest>, String>,
) -> Option<github::CurrentPullRequest> {
    refresh.unwrap_or(cached)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FleetRepositoryGroup {
    pub(crate) name: String,
    pub(crate) entries: Vec<(WorkspaceId, PaneId)>,
}

pub(crate) fn should_accept_agent_state(
    current: Option<&AgentPaneStatus>,
    incoming_state: AgentState,
    incoming_session_id: Option<&str>,
) -> bool {
    if incoming_state != AgentState::Idle {
        return true;
    }

    let Some(current) = current else {
        return true;
    };
    if current.state != AgentState::Running {
        return true;
    }

    matches!(
        (current.session_id.as_deref(), incoming_session_id),
        (Some(current), Some(incoming)) if current != incoming
    )
}

pub(crate) fn agent_event_completes_turn(state: AgentState, event: Option<&str>) -> bool {
    state == AgentState::Completed && matches!(event, Some("Stop" | "agent_end"))
}

#[derive(Debug, Clone)]
pub(crate) struct IntegrationDiscovery {
    pub(crate) wsl_distributions: Vec<WslDistributionChoice>,
    pub(crate) hook_statuses: Result<Vec<HookStatus>, String>,
}

#[derive(Debug, Clone)]
pub(crate) struct HookOperationResult {
    pub(crate) message: String,
    pub(crate) statuses: Vec<HookStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TerminalCellPosition {
    pub(crate) row: u64,
    pub(crate) column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalLink {
    pub(crate) uri: String,
    pub(crate) row: u64,
    pub(crate) start_column: usize,
    pub(crate) end_column: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TerminalScrollDrag {
    pub(crate) pane_id: PaneId,
    pub(crate) pane_top: f32,
    pub(crate) grab_offset: f32,
    pub(crate) track_height: f32,
    pub(crate) last_offset: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TerminalSelectionDrag {
    pub(crate) pane_id: PaneId,
    pub(crate) origin: Point,
    pub(crate) anchor: (u16, u16),
    pub(crate) active: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TerminalMouseCapture {
    pub(crate) pane_id: PaneId,
    pub(crate) button: TerminalMouseButton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum SplitBranch {
    First,
    Second,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SplitKey {
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) tab_id: TabId,
    pub(crate) path: Vec<SplitBranch>,
}

#[derive(Debug, Clone)]
pub(crate) struct SplitDrag {
    pub(crate) key: SplitKey,
    pub(crate) axis: SplitAxis,
    pub(crate) start_coordinate: f32,
    pub(crate) start_ratio: u16,
    pub(crate) extent: f32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TabDrag {
    pub(crate) tab_id: TabId,
    pub(crate) target_workspace_id: WorkspaceId,
    pub(crate) target_index: usize,
}

#[derive(Clone)]
pub(crate) struct EventSubscription(async_channel::Receiver<()>);

impl Hash for EventSubscription {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "muxtrix-event-subscription".hash(state);
    }
}

#[derive(Debug, Default)]
pub(crate) struct CommandPalette {
    pub(crate) visible: bool,
    pub(crate) query: String,
    pub(crate) selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WslDistributionChoice(Option<String>);

impl WslDistributionChoice {
    pub(crate) const fn default_distribution() -> Self {
        Self(None)
    }
}

impl std::fmt::Display for WslDistributionChoice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0.as_deref().unwrap_or("Windows default"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DefaultAgentChoice {
    None,
    Agent(Agent),
}

impl std::fmt::Display for DefaultAgentChoice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::None => "Not configured",
            Self::Agent(Agent::Codex) => "Codex",
            Self::Agent(Agent::Claude) => "Claude Code",
            Self::Agent(Agent::Pi) => "Oh My Pi",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteMove {
    Next,
    Previous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneSignalKind {
    Subtle,
    Neutral,
    Warning,
    Active,
    Danger,
}

impl PaneSignalKind {
    pub(crate) const fn color(self, tokens: DesignTokens) -> Color {
        match self {
            Self::Subtle => tokens.faint,
            Self::Neutral => tokens.muted,
            Self::Warning => tokens.warning,
            Self::Active => tokens.success,
            Self::Danger => tokens.danger,
        }
    }

    pub(crate) const fn label_color(self, tokens: DesignTokens) -> Color {
        match self {
            Self::Warning => tokens.warning,
            Self::Active => tokens.success,
            Self::Danger => tokens.danger,
            Self::Subtle | Self::Neutral => tokens.muted,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
    #[cfg(test)]
    Split(SplitAxis),
    SplitFrom(PaneId, SplitAxis),
    Focus(PaneId),
    FocusFleetPane(WorkspaceId, PaneId),
    ClosePane(PaneId),
    RestartPane(PaneId),
    StartTerminal(PaneId),
    CancelTerminalLaunch(PaneId),
    SessionHostInitialized(PaneId, Result<Vec<muxtrix_sessions::SessionRecord>, String>),
    PollTerminal,
    AgentsRosterLoaded(Result<agents_roster::AgentsRoster, String>),
    PaneRepositoriesLoaded(u64, Result<Vec<(PaneId, PaneRepository)>, String>),
    BlinkCursor,
    AnimateGitHubLoading,
    Keyboard(KeyEvent),
    ResizePane(PaneId, Size),
    ResizeSplit(SplitKey, Size),
    BeginSplitDrag(SplitKey, SplitAxis),
    PointerMoved(Point),
    EndPointerInteraction,
    EnterTerminal(PaneId),
    LeaveTerminal(PaneId),
    TerminalPointerMoved(PaneId, Point),
    TerminalScrollbarMoved(PaneId, Point),
    BeginTerminalScroll(PaneId),
    TerminalMousePressed(PaneId, TerminalMouseButton),
    TerminalMouseReleased(PaneId, TerminalMouseButton),
    OpenPaneContextMenu(PaneId),
    CopyTerminalSelection(PaneId),
    PastePane(PaneId),
    ClipboardPasted(PaneId, Option<String>),
    TerminalLinkOpened(String, Result<(), String>),
    ScrollTerminal(PaneId, ScrollDelta),
    ScrollHoveredTerminal(ScrollDelta),
    WindowOpened(Size),
    WindowResized(Size),
    WindowFocusChanged(bool),
    CloseCommandPalette,
    ToggleCommandPalette,
    CommandQueryChanged(String),
    CommandSelected(usize),
    RunCommand(CommandAction),
    NewWorkspace,
    CreateWorkspace,
    CancelWorkspaceCreate,
    SwitchWorkspace(WorkspaceId),
    WorkspaceNameChanged(String),
    RenameDraftChanged(String),
    ConfirmRename,
    CancelRename,
    WorktreeNameChanged(String),
    ConfirmWorktree,
    CancelWorktree,
    WorktreeCreated(WorktreePromptTarget, Result<std::path::PathBuf, String>),
    CloseWorktreeManager,
    WorktreeManagerLoaded(u64, Result<WorktreeManagerDiscovery, String>),
    RefreshWorktreeManager,
    WorktreeManagerDelete(usize),
    WorktreeManagerDeleteUnused,
    WorktreeManagerDeleted(Vec<std::path::PathBuf>, Result<(), String>),
    OpenPaneWorktreePrompt(PaneId),
    WorktreeManagerRestart(usize),
    ConfirmWorktreeManagerRestart,
    CancelWorktreeManagerRestart,
    CloseSessionPicker,
    SessionPickerResume(usize),
    SessionPickerKill(usize),
    SessionPickerKillAll,
    NewTab,
    CloseTab(WorkspaceId, TabId),
    ConfirmCloseWorkspace(WorkspaceId),
    CancelCloseWorkspace,
    BeginTabDrag(WorkspaceId, TabId, usize),
    TabDragOver(WorkspaceId, usize),
    OpenSettings,
    OpenSettingsPage(SettingsPage),
    InstalledVersionsLoaded(Result<InstalledVersions, String>),
    GitHubStatusPressed,
    BeginGitHubAuth,
    GitHubAuthChecked(u64, github::AuthStatus),
    GitHubAuthFinished(u64, Result<github::AuthStatus, String>),
    CloseGitHubPanel,
    RefreshGitHubPanel,
    RefreshGitHubPullRequestsAfterAgentTurn,
    GitHubFocusedPaneLoaded(
        PaneId,
        u64,
        GitHubContextLoad,
        Box<Result<(github::Repository, github::PanelData), String>>,
    ),
    SelectGitHubPanelTab(GitHubPanelTab),
    GitHubPullRequestsLoaded(
        std::path::PathBuf,
        u64,
        Box<Result<Vec<github::PullRequestSummary>, String>>,
    ),
    GitHubPullRequestQueryChanged(String),
    GitHubPullRequestScrolled(f32),
    SelectGitHubPullRequest(u64),
    CloseGitHubPullRequest,
    GitHubPullRequestLoaded(
        std::path::PathBuf,
        u64,
        u64,
        Box<Result<github::PullRequestDetails, String>>,
    ),
    GitHubFileScrolled(f32),
    OpenGitHubDiff(String),
    RetryGitHubDiff,
    CloseGitHubDiff,
    GitHubDiffLoaded(
        std::path::PathBuf,
        String,
        u64,
        Box<Result<github::DiffDocument, String>>,
    ),
    GitHubDiffScrolled(f32),
    OpenGitHubPullRequest(String),
    ToggleGitHubPullRequestDraft,
    GitHubPullRequestDraftChanged(std::path::PathBuf, u64, u64, bool, Result<String, String>),
    RequestGitHubMerge,
    CancelGitHubMerge,
    ConfirmGitHubMerge,
    GitHubMergeFinished(std::path::PathBuf, u64, u64, Result<String, String>),
    ToggleSidebar,
    ToggleMaximize(PaneId),
    ToggleMaximizeFromPaneMenu(PaneId),
    TogglePaneMenu(PaneId),
    DismissPaneMenu,
    DismissGlobalAlert(usize),
    ManageHooks(Agent, HookAction),
    RefreshHookStatus,
    IntegrationDiscoveryFinished(u64, Result<IntegrationDiscovery, String>),
    HookOperationFinished(u64, Result<HookOperationResult, String>),
    SettingsTerminalFont(TerminalFont),
    SettingsTerminalFontWeight(FontWeight),
    SettingsTerminalTheme(TerminalThemeId),
    SettingsAppearance(Appearance),
    SettingsShowStatusBar(bool),
    SettingsUiFont(UiFont),
    SettingsUiFontWeight(FontWeight),
    SettingsTerminalFontSize(f32),
    SettingsLineHeight(f32),
    SettingsScrollbackLimit(String),
    SettingsUiFontSize(f32),
    SettingsShowAllWorkspaces(bool),
    SetFleetView(FleetView),
    SettingsDefaultAgent(DefaultAgentChoice),
    SettingsGitHubHost(String),
    SettingsCodexCommand(String),
    SettingsClaudeCommand(String),
    SettingsPiCommand(String),
    #[cfg(target_os = "windows")]
    SettingsWindowsShellBackend(WindowsShellBackend),
    #[cfg(target_os = "windows")]
    SettingsWslDistribution(WslDistributionChoice),
    #[cfg(target_os = "windows")]
    RefreshWslDistributions,
    SaveSettings,
    CancelSettings,
    CloseDefaultAgentPrompt,
    OpenDefaultAgentSettings,
    OpenThemeGallery,
    CloseThemeGallery,
    GalleryThemeChosen(TerminalThemeId),
    #[cfg(feature = "e2e")]
    E2eTick,
    #[cfg(feature = "e2e")]
    E2eScreenshot(iced::window::Screenshot),
    #[cfg(feature = "e2e")]
    E2eWindowMissing,
}

impl Muxtrix {
    pub(crate) fn boot() -> (Self, Vec<Effect>) {
        let mut app = Self::new();
        let discovery = app.refresh_integrations();
        let github_host = app.settings.github_host.clone();
        let github_auth_cancellation = app.github_auth_cancellation.clone();
        let github_auth = perform_blocking(
            move || github::auth_status(&github_host, &github_auth_cancellation),
            |result| {
                Message::GitHubAuthChecked(
                    0,
                    result.unwrap_or(github::AuthStatus::Unavailable {
                        reason: "GitHub authentication could not be checked.".into(),
                    }),
                )
            },
        );
        (app, effect::batch([discovery, github_auth]))
    }

    pub(crate) fn prepare_session_host(&mut self, pane_id: PaneId) -> Vec<Effect> {
        if let Some(runtime) = self.terminals.get_mut(&pane_id) {
            runtime.launch_state = TerminalLaunchState::PreparingHost;
            runtime.preview =
                "Preparing terminal host…\n\nThe workspace remains usable while this runs.".into();
        }
        if session_host().is_some() || local_pty_allowed() {
            if let Err(error) = self.launch_terminal_for_pane(pane_id) {
                self.mark_terminal_launch_failed(pane_id, error);
            }
            return Vec::new();
        }
        self.initialize_session_host(pane_id, true)
    }

    pub(crate) fn start_new_session_host(&mut self, pane_id: PaneId) -> Vec<Effect> {
        self.initialize_session_host(pane_id, false)
    }

    pub(crate) fn initialize_session_host(
        &self,
        pane_id: PaneId,
        offer_resume: bool,
    ) -> Vec<Effect> {
        perform_blocking(
            move || {
                let candidates = if offer_resume {
                    muxtrix_sessions::resumable_sessions(None)
                } else {
                    Vec::new()
                };
                start_host_unless_resumable(candidates, || {
                    let host = start_session_host().ok_or_else(|| {
                        "The terminal host did not become ready. Check WSL or the selected shell, then retry."
                            .to_owned()
                    })?;
                    if let Ok(mut active) = SESSION_HOST.lock() {
                        *active = Some(host);
                    }
                    Ok(())
                })
            },
            move |result| {
                Message::SessionHostInitialized(pane_id, result.and_then(std::convert::identity))
            },
        )
    }

    pub(crate) fn launch_terminal_for_pane(&mut self, pane_id: PaneId) -> Result<(), String> {
        let (profile_id, surface_directory) = self
            .session
            .workspaces
            .iter()
            .find_map(|workspace| workspace.pane(pane_id))
            .and_then(|pane| pane.active_surface())
            .and_then(|surface| match &surface.kind {
                muxtrix_domain::SurfaceKind::Terminal(terminal) => {
                    Some((terminal.profile_id, terminal.working_directory.clone()))
                }
                _ => None,
            })
            .ok_or_else(|| format!("pane {pane_id:?} has no terminal profile"))?;
        let mut profile = self
            .session
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
            .ok_or_else(|| "terminal launch profile is missing".to_owned())?;
        if let Some(directory) = surface_directory {
            profile.working_directory = Some(directory);
        }
        let fallback_title = self.terminals.get(&pane_id).map_or_else(
            || "terminal".to_owned(),
            |runtime| runtime.fallback_title.clone(),
        );
        self.request_terminal_launch(profile, pane_id, fallback_title)
    }

    pub(crate) fn request_terminal_launch(
        &mut self,
        profile: LaunchProfile,
        pane_id: PaneId,
        fallback_title: String,
    ) -> Result<(), String> {
        self.request_terminal_launch_with_policy(
            profile,
            pane_id,
            fallback_title,
            CreationDirectoryPolicy::Exact,
        )
    }

    pub(crate) fn request_regular_terminal_launch(
        &mut self,
        profile: LaunchProfile,
        pane_id: PaneId,
        fallback_title: String,
    ) -> Result<(), String> {
        self.request_terminal_launch_with_policy(
            profile,
            pane_id,
            fallback_title,
            CreationDirectoryPolicy::Regular,
        )
    }

    pub(crate) fn request_terminal_launch_with_policy(
        &mut self,
        profile: LaunchProfile,
        pane_id: PaneId,
        fallback_title: String,
        directory_policy: CreationDirectoryPolicy,
    ) -> Result<(), String> {
        self.next_terminal_launch_attempt = self.next_terminal_launch_attempt.wrapping_add(1);
        let attempt_id = self.next_terminal_launch_attempt;
        let viewport = self
            .terminals
            .get(&pane_id)
            .and_then(|runtime| runtime.viewport);
        let previous_session = self
            .terminals
            .get_mut(&pane_id)
            .and_then(|runtime| runtime.session.take());
        let target_size = viewport.map_or_else(initial_pty_size, |viewport| {
            pty_size_for_pane(viewport, &self.settings)
        });
        self.terminals.insert(
            pane_id,
            TerminalRuntime::starting(&fallback_title, attempt_id, viewport),
        );
        let request = TerminalLaunchRequest {
            profile,
            directory_policy,
            wsl_distribution: self.settings.wsl_distribution.clone(),
            pane_id,
            theme: self.settings.terminal_theme.preset().terminal_theme(),
            max_scrollback: self.settings.terminal_scrollback_lines,
            notifier: Arc::clone(&self.event_notifier),
            control_endpoint: self.control_endpoint.clone(),
            target_size,
            cell_width_px: self.settings.terminal_cell_width(),
            cell_height_px: self.settings.terminal_cell_height(),
            previous_session,
        };
        if !self.launch_in_background {
            let result = self.terminal_launcher.launch(request);
            self.finish_terminal_launch(TerminalLaunchCompletion {
                pane_id,
                attempt_id,
                result,
            });
            return self
                .terminals
                .get(&pane_id)
                .and_then(|runtime| match &runtime.launch_state {
                    TerminalLaunchState::Failed(error) => Some(Err(error.clone())),
                    _ => None,
                })
                .unwrap_or(Ok(()));
        }

        let launcher = Arc::clone(&self.terminal_launcher);
        let completions = Arc::clone(&self.terminal_launch_completions);
        let notifier = Arc::clone(&self.event_notifier);
        if let Err(error) = std::thread::Builder::new()
            .name(format!("muxtrix-terminal-launch-{attempt_id}"))
            .spawn(move || {
                let result = launcher.launch(request);
                if let Ok(mut queue) = completions.lock() {
                    queue.push_back(TerminalLaunchCompletion {
                        pane_id,
                        attempt_id,
                        result,
                    });
                }
                notifier();
            })
        {
            let error = format!("Could not start terminal launch worker: {error}");
            self.mark_terminal_launch_failed(pane_id, error.clone());
            return Err(error);
        }
        self.status = "Starting terminal in the background…".into();
        Ok(())
    }

    pub(crate) fn drain_terminal_launches(&mut self) {
        let completions: Vec<_> = self
            .terminal_launch_completions
            .lock()
            .map(|mut queue| queue.drain(..).collect())
            .unwrap_or_default();
        for completion in completions {
            self.finish_terminal_launch(completion);
        }
    }

    pub(crate) fn finish_terminal_launch(&mut self, completion: TerminalLaunchCompletion) {
        let current = self
            .terminals
            .get(&completion.pane_id)
            .is_some_and(|runtime| {
                matches!(
                    runtime.launch_state,
                    TerminalLaunchState::Starting { attempt_id }
                        if attempt_id == completion.attempt_id
                )
            });
        if !current {
            if let Ok(launched) = completion.result {
                if self.terminals.contains_key(&completion.pane_id) {
                    // A newer attempt for this same pane is in flight, and it
                    // shares the pane's identity with the daemon: killing this
                    // session would kill the pane that replaced it.
                    dispose_live_session(launched.session);
                } else {
                    // The pane was closed while its launch was still running,
                    // so the close never had a session to end. Nothing will
                    // attach to this one again.
                    terminate_live_session(launched.session, completion.pane_id);
                }
            }
            return;
        }
        let restart_queued = self.queued_terminal_restarts.remove(&completion.pane_id);
        match completion.result {
            Ok(launched) => {
                let working_directory = launched.working_directory.clone();
                let pending_viewport = self
                    .terminals
                    .get(&completion.pane_id)
                    .and_then(|runtime| runtime.viewport);
                if let Some(runtime) = self.terminals.get_mut(&completion.pane_id) {
                    runtime.session = Some(launched.session);
                    runtime.set_snapshot(launched.snapshot);
                    runtime.size = launched.size;
                    runtime.launch_state = TerminalLaunchState::Running;
                    runtime.preview = "Starting terminal…".into();
                }
                // A queued restart already wrote the directory it was chosen
                // for onto the surface, and that is where the replacement
                // this hand-off is about to start must land. Recording where
                // the launch that is being replaced ended up would send it
                // back to the directory the user just left.
                if !restart_queued
                    && let Some(pane) = self
                        .session
                        .workspaces
                        .iter_mut()
                        .find_map(|workspace| workspace.pane_mut(completion.pane_id))
                    && let Some(surface) = pane
                        .surfaces
                        .iter_mut()
                        .find(|surface| surface.id == pane.active_surface_id)
                    && let muxtrix_domain::SurfaceKind::Terminal(terminal) = &mut surface.kind
                {
                    terminal.working_directory = working_directory;
                }
                // Input typed after a restart was queued belongs to the
                // replacement, not the briefly completed original launch.
                if !restart_queued
                    && let Some(inputs) = self.pending_terminal_input.remove(&completion.pane_id)
                {
                    for input in inputs {
                        let _ = self.send_terminal_input_to(completion.pane_id, input);
                    }
                }
                self.status = "Live terminal — GPU compositor: Iced/wgpu".into();
                // A pane measured while its launch was still in flight had no
                // session to resize, and the launch just restored the size it
                // was requested with. Replay the pane's own viewport so the
                // PTY matches what the pane draws; the sensor only reports
                // size *changes*, so nothing else would correct it until the
                // pane happens to be resized by hand.
                if let Some(viewport) = pending_viewport
                    && let Err(error) = self.resize_terminal(completion.pane_id, viewport)
                {
                    self.status = format!("Terminal resize failed: {error}");
                }
            }
            Err(error) => self.mark_terminal_launch_failed(completion.pane_id, error),
        }
        if restart_queued {
            self.status = "Restarting terminal after its current launch finished…".into();
            if let Err(error) = self.launch_terminal_for_pane(completion.pane_id) {
                self.mark_terminal_launch_failed(completion.pane_id, error);
            }
        }
    }

    pub(crate) fn mark_terminal_launch_failed(&mut self, pane_id: PaneId, error: String) {
        if let Some(runtime) = self.terminals.get_mut(&pane_id) {
            runtime.preview = format!(
                "Terminal unavailable\n\n{error}\n\nRetry, change the terminal backend in Settings, or close this pane."
            );
            runtime.launch_state = TerminalLaunchState::Failed(error.clone());
            runtime.session = None;
        }
        self.pending_terminal_input.remove(&pane_id);
        self.agent_running_frame_revisions.remove(&pane_id);
        self.pi_active_lifecycles.remove(&pane_id);
        if let Some(agent) = self.agent_statuses.get_mut(&pane_id) {
            agent.state = AgentState::Failed;
            agent.activity = Some("Terminal failed before the agent could start".into());
        }
        self.status = format!("Terminal unavailable: {error}");
    }

    pub(crate) fn new() -> Self {
        let (mut settings, mut settings_warning) = AppSettings::load();
        // Preserve the path used to launch this process. Package managers can
        // replace the executable while the window stays open; `current_exe`
        // may continue to identify the old inode after that replacement.
        let installed_muxtrix_path = startup_muxtrix_path();
        let installed_fonts = InstalledFontCatalog::discover();
        // Cell width must come from the measured face before any pane is sized.
        installed_fonts.install_metrics();
        let available_ui_fonts = installed_fonts.ui_fonts();
        if !available_ui_fonts.contains(&settings.ui_font) {
            let unavailable = format!(
                "{} is not installed; interface font reset to System sans serif",
                settings.ui_font
            );
            settings.ui_font = UiFont::SystemSans;
            settings_warning = Some(settings_warning.map_or(unavailable.clone(), |warning| {
                format!("{warning}; {unavailable}")
            }));
        }
        let mut available_ui_font_weights = installed_fonts.ui_weights(&settings.ui_font);
        if available_ui_font_weights.is_empty() {
            available_ui_font_weights.push(FontWeight::Normal);
        }
        if !available_ui_font_weights.contains(&settings.ui_font_weight) {
            let replacement = available_ui_font_weights[0];
            let unavailable = format!(
                "{} is not installed for {}; interface font weight reset to {}",
                settings.ui_font_weight, settings.ui_font, replacement
            );
            settings.ui_font_weight = replacement;
            settings_warning = Some(settings_warning.map_or(unavailable.clone(), |warning| {
                format!("{warning}; {unavailable}")
            }));
        }
        let available_terminal_fonts = installed_fonts.terminal_fonts();
        if !available_terminal_fonts.contains(&settings.terminal_font) {
            let unavailable = format!(
                "{} is not installed; terminal font reset to System monospace",
                settings.terminal_font
            );
            settings.terminal_font = TerminalFont::SystemMonospace;
            settings_warning = Some(settings_warning.map_or(unavailable.clone(), |warning| {
                format!("{warning}; {unavailable}")
            }));
        }
        let mut available_terminal_font_weights =
            installed_fonts.terminal_weights(&settings.terminal_font);
        if available_terminal_font_weights.is_empty() {
            available_terminal_font_weights.push(FontWeight::Normal);
        }
        if !available_terminal_font_weights.contains(&settings.terminal_font_weight) {
            let replacement = available_terminal_font_weights[0];
            let unavailable = format!(
                "{} is not installed for {}; terminal font weight reset to {}",
                settings.terminal_font_weight, settings.terminal_font, replacement
            );
            settings.terminal_font_weight = replacement;
            settings_warning = Some(settings_warning.map_or(unavailable.clone(), |warning| {
                format!("{warning}; {unavailable}")
            }));
        }
        let profile = default_profile(&settings);
        let surface = terminal_surface(profile.id, "shell 1");
        let workspace = Workspace::new("muxtrix", surface);
        let workspace_name_draft = workspace.name.clone();
        let initial_pane_id = workspace
            .active_tab()
            .expect("new workspace should contain its default tab")
            .focused_pane_id;
        let (event_sender, event_receiver) = async_channel::bounded(1);
        let event_notifier: EventNotifier = Arc::new(move || {
            let _ = event_sender.try_send(());
        });
        let initial_control_endpoint = if std::env::var_os("MUXTRIX_CONTROL_ENDPOINT").is_some() {
            Endpoint::discover()
        } else {
            Endpoint::for_instance(&format!("window-{}", uuid::Uuid::new_v4()))
        };
        let (control, control_status) =
            start_control_server(initial_control_endpoint, Arc::clone(&event_notifier));
        let control_endpoint = control
            .as_ref()
            .map(|server| server.endpoint_environment_value().to_owned());
        let launch_in_background = !cfg!(test);
        let no_terminal = NO_TERMINAL_STARTUP.load(Ordering::Relaxed);
        let (runtime, terminal_status) = if launch_in_background {
            let runtime = if no_terminal {
                TerminalRuntime::suppressed("shell 1")
            } else {
                TerminalRuntime::preparing_host("shell 1")
            };
            let status = if no_terminal {
                "Opened without starting a terminal".into()
            } else {
                "Preparing terminal host…".into()
            };
            (runtime, status)
        } else {
            TerminalRuntime::launch(
                &profile,
                initial_pane_id,
                "shell 1",
                settings.terminal_scrollback_lines,
                settings.terminal_theme.preset().terminal_theme(),
                Arc::clone(&event_notifier),
                control_endpoint.as_deref(),
            )
        };
        let mut global_alerts = Vec::new();
        if let Some(body) = settings_warning.as_ref() {
            global_alerts.push(GlobalAlert {
                title: "Settings need review".into(),
                body: body.clone(),
            });
        }
        if let Some(body) = control_status.as_ref() {
            global_alerts.push(GlobalAlert {
                title: "Local control unavailable".into(),
                body: body.clone(),
            });
        }
        #[cfg(feature = "e2e")]
        let e2e = e2e::Scenario::from_environment(initial_pane_id);
        let mut available_wsl_distributions = vec![WslDistributionChoice::default_distribution()];
        if !settings.wsl_distribution.trim().is_empty()
            && !available_wsl_distributions
                .iter()
                .any(|choice| choice.0.as_deref() == Some(settings.wsl_distribution.trim()))
        {
            available_wsl_distributions.push(WslDistributionChoice(Some(
                settings.wsl_distribution.trim().to_owned(),
            )));
        }

        let settings_scrollback_lines_input = settings.terminal_scrollback_lines.to_string();
        let app = Self {
            session: SessionState::new(workspace, vec![profile]),
            terminals: BTreeMap::from([(initial_pane_id, runtime)]),
            status: settings_warning
                .or(control_status)
                .unwrap_or(terminal_status),
            cursor_phase_visible: true,
            active_view: ActiveView::Workspace,
            settings_page: SettingsPage::Preferences,
            palette: CommandPalette::default(),
            settings_draft: settings.clone(),
            settings_scrollback_lines_input,
            settings,
            installed_versions: InstalledVersionsState::default(),
            installed_muxtrix_path,
            installed_fonts,
            available_terminal_fonts,
            available_terminal_font_weights,
            available_ui_fonts,
            available_ui_font_weights,
            available_wsl_distributions,
            workspace_name_draft,
            rename_prompt: None,
            rename_draft: String::new(),
            default_agent_prompt: false,
            pending_default_agent_command: None,
            worktree_prompt: None,
            worktree_name_draft: String::new(),
            worktree_manager: None,
            worktree_manager_generation: 0,
            session_picker: None,
            last_layout_hash: 0,
            toast: None,
            prefix_armed: false,
            rail_nav: None,
            workspace_create_visible: false,
            close_workspace_prompt: None,
            tab_drag: None,
            notifications: Vec::new(),
            global_alerts,
            github_auth: github::AuthStatus::Checking,
            github_auth_busy: false,
            github_auth_generation: 0,
            github_auth_cancellation: ProcessCancellation::default(),
            github_request_generation: 0,
            github_panel: None,
            github_diff: None,
            github_pane_refresh_pending: false,
            github_pull_requests_refresh_pending: false,
            github_context_generation: 0,
            github_context_cancellation: ProcessCancellation::default(),
            sidebar_collapsed: false,
            maximized_pane: None,
            pane_menu: None,
            window_open: false,
            window_size: Size::new(1_280.0, 800.0),
            window_focused: true,
            cursor_position: Point::ORIGIN,
            keyboard_modifiers: Modifiers::empty(),
            split_sizes: BTreeMap::new(),
            split_drag: None,
            pane_layouts: BTreeMap::new(),
            base_pane_layouts: BTreeMap::new(),
            pane_resize_history: BTreeMap::new(),
            hovered_terminal: None,
            terminal_pointer_positions: BTreeMap::new(),
            terminal_scrollbar_positions: BTreeMap::new(),
            terminal_scroll_drag: None,
            terminal_mouse_capture: None,
            terminal_selection_drag: None,
            terminal_command_buffers: BTreeMap::new(),
            event_notifier,
            event_receiver,
            terminal_launcher: Arc::new(SystemTerminalLauncher::default()),
            terminal_launch_completions: Arc::new(Mutex::new(VecDeque::new())),
            next_terminal_launch_attempt: 0,
            launch_in_background,
            startup_terminal_pending: (launch_in_background && !no_terminal)
                .then_some(initial_pane_id),
            queued_terminal_restarts: BTreeSet::new(),
            pending_terminal_input: BTreeMap::new(),
            control,
            control_endpoint,
            agent_statuses: BTreeMap::new(),
            agent_running_frame_revisions: BTreeMap::new(),
            pi_active_lifecycles: BTreeSet::new(),
            detected_agents: BTreeMap::new(),
            agents_view_panes: BTreeSet::new(),
            agents_roster: None,
            agents_roster_pending: false,
            agents_roster_error: None,
            agents_roster_checked: None,
            pane_repositories: BTreeMap::new(),
            pending_repository_directories: BTreeMap::new(),
            pane_repository_generation: 0,
            pane_repository_cancellation: ProcessCancellation::default(),
            hook_statuses: Vec::new(),
            integration_generation: 0,
            integration_refreshing: false,
            #[cfg(feature = "e2e")]
            e2e,
        };
        let _ = app.publish_control_panes();
        app
    }

    pub(crate) fn title(&self) -> String {
        self.active_workspace().map_or_else(
            |_| "Muxtrix".into(),
            |workspace| {
                workspace.active_tab().map_or_else(
                    || format!("{} — Muxtrix", workspace.name),
                    |tab| {
                        format!(
                            "{} — {} — Muxtrix",
                            self.pane_title(workspace, tab.focused_pane_id),
                            tab.name
                        )
                    },
                )
            },
        )
    }

    pub(crate) fn theme(&self) -> Theme {
        match self.settings.appearance {
            Appearance::Light => Theme::Light,
            Appearance::System | Appearance::Dark => Theme::TokyoNight,
        }
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            Subscription::run_with(
                EventSubscription(self.event_receiver.clone()),
                event_subscription,
            ),
            iced::time::every(std::time::Duration::from_millis(500)).map(|_| Message::BlinkCursor),
            iced::event::listen_with(app_event),
        ];
        if self.github_loading_animating() {
            subscriptions.push(
                iced::time::every(std::time::Duration::from_millis(90))
                    .map(|_| Message::AnimateGitHubLoading),
            );
        }
        if self.github_pull_requests_refresh_pending {
            subscriptions.push(
                iced::time::every(std::time::Duration::from_millis(1))
                    .map(|_| Message::RefreshGitHubPullRequestsAfterAgentTurn),
            );
        }
        #[cfg(feature = "e2e")]
        let subscriptions = if self.has_e2e_scenario() {
            let mut with_e2e = subscriptions;
            with_e2e.push(
                iced::time::every(std::time::Duration::from_millis(50)).map(|_| Message::E2eTick),
            );
            with_e2e
        } else {
            subscriptions
        };
        Subscription::batch(subscriptions)
    }

    /// The pane the keyboard is currently aimed at, if a workspace is open.
    pub(crate) fn focused_pane_id(&self) -> Option<PaneId> {
        self.active_workspace()
            .ok()
            .and_then(Workspace::active_tab)
            .map(|tab| tab.focused_pane_id)
    }

    /// Whether the GitHub panel's loading animation should be stepping.
    ///
    /// Gated on window focus so an unattended window is not repainting for an
    /// animation nobody can see.
    pub(crate) fn github_loading_animating(&self) -> bool {
        let animating = self.window_focused
            && self
                .github_panel
                .as_ref()
                .is_some_and(GitHubPanelState::active_loading);
        #[cfg(feature = "e2e")]
        let animating = animating && self.e2e.is_none();
        animating
    }

    /// Whether an e2e scenario is driving this run.
    #[cfg(feature = "e2e")]
    pub(crate) fn has_e2e_scenario(&self) -> bool {
        self.e2e.is_some()
    }

    pub(crate) fn update(&mut self, message: Message) -> Vec<Effect> {
        let result = match message {
            #[cfg(test)]
            Message::Split(axis) => self.split_terminal(axis),
            Message::SplitFrom(pane_id, axis) => self
                .focus_pane(pane_id)
                .and_then(|()| self.split_terminal(axis)),
            Message::Focus(pane_id) => {
                self.cursor_phase_visible = true;
                self.active_view = ActiveView::Workspace;
                self.focus_pane(pane_id)
            }
            Message::FocusFleetPane(workspace_id, pane_id) => self
                .switch_workspace(workspace_id)
                .and_then(|()| self.focus_pane(pane_id)),
            Message::ClosePane(pane_id) => self.close_pane(pane_id),
            Message::RestartPane(pane_id) if session_host().is_none() && !local_pty_allowed() => {
                return self.prepare_session_host(pane_id);
            }
            Message::RestartPane(pane_id) => self.restart_pane(pane_id),
            Message::StartTerminal(pane_id) => {
                return self.prepare_session_host(pane_id);
            }
            Message::CancelTerminalLaunch(pane_id) => {
                self.queued_terminal_restarts.remove(&pane_id);
                if let Some(runtime) = self.terminals.get_mut(&pane_id)
                    && matches!(
                        runtime.launch_state,
                        TerminalLaunchState::PreparingHost | TerminalLaunchState::Starting { .. }
                    )
                {
                    runtime.launch_state = TerminalLaunchState::Suppressed;
                    runtime.preview =
                        "Terminal launch cancelled. The workspace is still usable.".into();
                    self.status = "Terminal launch cancelled".into();
                }
                return Vec::new();
            }
            Message::SessionHostInitialized(pane_id, result) => {
                let still_waiting = self.terminals.get(&pane_id).is_some_and(|runtime| {
                    matches!(runtime.launch_state, TerminalLaunchState::PreparingHost)
                });
                if !still_waiting {
                    return Vec::new();
                }
                match result {
                    Ok(candidates) if !candidates.is_empty() => {
                        self.open_session_picker_from_records(candidates, true);
                    }
                    Ok(_) => {
                        let panes = Self::control_pane_ids(&self.session);
                        let launch = session_host()
                            .ok_or_else(|| "Terminal session host is unavailable".to_owned())
                            .and_then(|(session_id, _)| {
                                self.bind_control_to_session(session_id, &panes)
                            })
                            .and_then(|()| self.launch_terminal_for_pane(pane_id));
                        if let Err(error) = launch {
                            self.mark_terminal_launch_failed(pane_id, error);
                        }
                    }
                    Err(error) => self.mark_terminal_launch_failed(pane_id, error),
                }
                return Vec::new();
            }
            Message::PollTerminal => {
                self.poll_terminal();
                self.poll_control();
                let metadata = self.refresh_background_metadata();
                let github = if self.github_pane_refresh_pending {
                    self.github_pane_refresh_pending = false;
                    self.refresh_github_focused_pane()
                } else {
                    Vec::new()
                };
                return effect::batch([metadata, github]);
            }
            Message::AgentsRosterLoaded(result) => {
                self.agents_roster_pending = false;
                // A failed read keeps the last roster rather than inventing
                // counts or blanking a row that is still showing the view. It
                // is still recorded: with no roster to fall back on, the reason
                // is the only thing the row can honestly report.
                match result {
                    Ok(roster) => {
                        self.agents_roster = Some(roster);
                        self.agents_roster_error = None;
                    }
                    Err(error) => self.agents_roster_error = Some(error),
                }
                return Vec::new();
            }
            Message::PaneRepositoriesLoaded(generation, result) => {
                if generation != self.pane_repository_generation {
                    return Vec::new();
                }
                let repositories = match result {
                    Ok(repositories) => repositories,
                    Err(error) => {
                        self.pending_repository_directories.clear();
                        self.status = format!("Repository grouping unavailable: {error}");
                        return Vec::new();
                    }
                };
                for (pane_id, repository) in repositories {
                    if self.pending_repository_directories.get(&pane_id)
                        == Some(&repository.directory)
                    {
                        self.pending_repository_directories.remove(&pane_id);
                        if self.pane_working_directory(pane_id).as_ref()
                            == Some(&repository.directory)
                        {
                            self.pane_repositories.insert(pane_id, repository);
                        }
                    }
                }
                return Vec::new();
            }
            Message::BlinkCursor => {
                self.cursor_phase_visible = !self.cursor_phase_visible;
                // The notifier channel deliberately coalesces wakeups. A
                // periodic drain is the safety net when a launch completion
                // and its first resized frame arrive under one wake token.
                self.poll_terminal();
                self.detect_agent_processes();
                if self
                    .toast
                    .as_ref()
                    .is_some_and(|(_, shown)| shown.elapsed() >= std::time::Duration::from_secs(2))
                {
                    self.toast = None;
                }
                self.sync_session_layout();
                return self.refresh_background_metadata();
            }
            Message::AnimateGitHubLoading => {
                if let Some(panel) = self
                    .github_panel
                    .as_mut()
                    .filter(|panel| panel.active_loading())
                {
                    panel.loading_phase = (panel.loading_phase + 1) % GITHUB_LOADING_DOT_COUNT;
                }
                return Vec::new();
            }
            Message::Keyboard(event) => {
                return self.handle_keyboard(event);
            }
            Message::ResizePane(pane_id, size) => {
                if let Err(error) = self.resize_terminal(pane_id, size) {
                    self.status = format!("Terminal resize failed: {error}");
                }
                return Vec::new();
            }
            Message::ResizeSplit(key, size) => {
                self.split_sizes.insert(key, size);
                return Vec::new();
            }
            Message::BeginSplitDrag(key, axis) => {
                if let Err(error) = self.begin_split_drag(key, axis) {
                    self.status = error;
                }
                return Vec::new();
            }
            Message::PointerMoved(position) => {
                self.cursor_position = position;
                if let Err(error) = self.update_split_drag(position) {
                    self.status = error;
                    self.split_drag = None;
                }
                if let Err(error) = self.update_terminal_scroll_drag(position) {
                    self.status = error;
                    self.terminal_scroll_drag = None;
                }
                return Vec::new();
            }
            Message::EndPointerInteraction => {
                let selected_text = self
                    .finish_terminal_selection(None)
                    .and_then(|pane_id| self.selected_terminal_text(pane_id));
                if let Err(error) = self.finish_tab_drag() {
                    self.status = error;
                }
                if let Err(error) = self.release_terminal_mouse_capture() {
                    self.status = format!("Terminal mouse release failed: {error}");
                }
                self.split_drag = None;
                self.terminal_scroll_drag = None;
                return selected_text
                    .map_or_else(Vec::new, |text| self.copy_terminal_selection(text));
            }
            Message::EnterTerminal(pane_id) => {
                self.hovered_terminal = Some(pane_id);
                return Vec::new();
            }
            Message::LeaveTerminal(pane_id) => {
                if self.hovered_terminal == Some(pane_id) {
                    self.hovered_terminal = None;
                }
                return Vec::new();
            }
            Message::TerminalPointerMoved(pane_id, position) => {
                self.terminal_pointer_positions.insert(pane_id, position);
                if let Some(mut drag) = self
                    .terminal_selection_drag
                    .filter(|drag| drag.pane_id == pane_id)
                {
                    let cell = self.terminal_grid_cell_at(pane_id, position);
                    let result = if drag.active {
                        self.terminals
                            .get_mut(&pane_id)
                            .map(|runtime| runtime.selection_extend(cell))
                    } else if terminal_selection_drag_started(drag.origin, position) {
                        drag.active = true;
                        self.terminals.get_mut(&pane_id).map(|runtime| {
                            runtime
                                .selection_start(drag.anchor)
                                .and_then(|()| runtime.selection_extend(cell))
                        })
                    } else {
                        None
                    };
                    self.terminal_selection_drag = Some(drag);
                    if let Some(Err(error)) = result {
                        self.status = format!("Selection failed: {error}");
                    }
                } else if self
                    .terminals
                    .get(&pane_id)
                    .and_then(|runtime| runtime.snapshot.as_ref())
                    .is_some_and(|snapshot| snapshot.mouse_reporting)
                {
                    let button = self
                        .terminal_mouse_capture
                        .filter(|capture| capture.pane_id == pane_id)
                        .map(|capture| capture.button);
                    let event = terminal_mouse_event(
                        position,
                        TerminalMouseAction::Motion,
                        button,
                        self.keyboard_modifiers,
                    );
                    if let Some(runtime) = self.terminals.get(&pane_id)
                        && let Err(error) = runtime.mouse(event)
                    {
                        self.status = format!("Terminal mouse motion failed: {error}");
                    }
                }
                return Vec::new();
            }
            Message::TerminalScrollbarMoved(pane_id, position) => {
                self.terminal_scrollbar_positions.insert(pane_id, position);
                return Vec::new();
            }
            Message::BeginTerminalScroll(pane_id) => {
                if let Err(error) = self.begin_terminal_scroll(pane_id) {
                    self.status = error;
                } else {
                    #[cfg(feature = "e2e")]
                    if let Some(scenario) = &mut self.e2e {
                        scenario.observe_terminal_scrollbar();
                    }
                }
                return Vec::new();
            }
            Message::TerminalMousePressed(pane_id, button) => {
                return self.begin_terminal_mouse(pane_id, button);
            }
            Message::TerminalMouseReleased(pane_id, button) => {
                let selected_text = if button == TerminalMouseButton::Left {
                    self.finish_terminal_selection(Some(pane_id))
                        .and_then(|pane_id| self.selected_terminal_text(pane_id))
                } else {
                    None
                };
                if let Err(error) = self.end_terminal_mouse(pane_id, button) {
                    self.status = format!("Terminal mouse release failed: {error}");
                }
                return selected_text
                    .map_or_else(Vec::new, |text| self.copy_terminal_selection(text));
            }
            Message::OpenPaneContextMenu(pane_id) => {
                let _ = self.focus_pane(pane_id);
                self.pane_menu = Some(pane_id);
                return Vec::new();
            }
            Message::CopyTerminalSelection(pane_id) => {
                self.pane_menu = None;
                let Some(text) = self.selected_terminal_text(pane_id) else {
                    return Vec::new();
                };
                return self.copy_terminal_selection(text);
            }
            Message::PastePane(pane_id) => {
                self.pane_menu = None;
                let _ = self.focus_pane(pane_id);
                return vec![Effect::ClipboardRead(Arc::new(move |contents| {
                    Message::ClipboardPasted(pane_id, contents)
                }))];
            }
            Message::ClipboardPasted(pane_id, contents) => {
                if let Some(text) = contents.filter(|text| !text.is_empty()) {
                    self.cursor_phase_visible = true;
                    if let Err(error) = self.paste_into_pane(pane_id, &text) {
                        self.status = format!("Paste failed: {error}");
                    }
                }
                return Vec::new();
            }
            Message::TerminalLinkOpened(uri, result) => {
                self.status = match result {
                    Ok(()) => format!("Opened {uri}"),
                    Err(error) => format!("Could not open {uri}: {error}"),
                };
                return Vec::new();
            }
            Message::ScrollTerminal(pane_id, delta) => {
                match self.scroll_terminal(pane_id, delta) {
                    Ok(()) =>
                    {
                        #[cfg(feature = "e2e")]
                        if let Some(scenario) = &mut self.e2e {
                            scenario.observe_terminal_scroll();
                        }
                    }
                    Err(error) => self.status = format!("Terminal scroll failed: {error}"),
                }
                return Vec::new();
            }
            Message::ScrollHoveredTerminal(delta) => {
                let Some(pane_id) = self.hovered_terminal else {
                    return Vec::new();
                };
                let result = self.scroll_terminal(pane_id, delta);
                match result {
                    Ok(()) =>
                    {
                        #[cfg(feature = "e2e")]
                        if let Some(scenario) = &mut self.e2e {
                            scenario.observe_terminal_scroll();
                        }
                    }
                    Err(error) => self.status = format!("Terminal scroll failed: {error}"),
                }
                return Vec::new();
            }
            Message::WindowOpened(size) => {
                self.window_open = true;
                self.window_size = size;
                let resize = self.window_resize_increment_task();
                let terminal = self
                    .startup_terminal_pending
                    .take()
                    .map_or_else(Vec::new, |pane_id| self.prepare_session_host(pane_id));
                return effect::batch([resize, terminal]);
            }
            Message::WindowResized(size) => {
                self.window_size = size;
                self.reflow_github_diff();
                return Vec::new();
            }
            Message::WindowFocusChanged(focused) => {
                self.window_focused = focused;
                return Vec::new();
            }
            Message::CloseCommandPalette => {
                self.close_command_palette();
                return Vec::new();
            }
            Message::ToggleCommandPalette => return self.toggle_command_palette(),
            Message::CommandQueryChanged(query) => {
                self.palette.query = query;
                let commands = commands::filtered(&self.palette.query);
                let enabled: Vec<_> = commands
                    .iter()
                    .map(|command| self.command_enabled(command.action))
                    .collect();
                self.palette.selected = first_enabled_palette_command(&enabled);
                return Vec::new();
            }
            Message::CommandSelected(index) => {
                let commands = commands::filtered(&self.palette.query);
                if let Some(command) = commands
                    .get(index)
                    .filter(|command| self.command_enabled(command.action))
                {
                    return self.run_command(command.action);
                }
                return Vec::new();
            }
            Message::RunCommand(action) => return self.run_command(action),
            Message::NewWorkspace => return self.open_workspace_create(),
            Message::CreateWorkspace => self.create_workspace(),
            Message::CancelWorkspaceCreate => {
                self.workspace_create_visible = false;
                return Vec::new();
            }
            Message::SwitchWorkspace(workspace_id) => self.switch_workspace(workspace_id),
            Message::WorkspaceNameChanged(name) => {
                self.workspace_name_draft = name;
                return Vec::new();
            }
            Message::RenameDraftChanged(name) => {
                self.rename_draft = name;
                return Vec::new();
            }
            Message::ConfirmRename => {
                let result = self.apply_rename();
                if result.is_ok() {
                    self.rename_prompt = None;
                }
                result
            }
            Message::CancelRename => {
                self.rename_prompt = None;
                return Vec::new();
            }
            Message::WorktreeNameChanged(name) => {
                self.worktree_name_draft = name;
                return Vec::new();
            }
            Message::ConfirmWorktree => return self.confirm_worktree(),
            Message::CancelWorktree => {
                self.worktree_prompt = None;
                return Vec::new();
            }
            Message::WorktreeCreated(target, result) => {
                match result.and_then(|path| {
                    self.open_created_worktree(target, path.clone())
                        .map(|()| path)
                }) {
                    Ok(path) => {
                        self.worktree_prompt = None;
                        self.status = match target {
                            WorktreePromptTarget::Open(commands::WorktreeKind::Pane(_)) => {
                                format!("Opened worktree {} in a new pane", path.display())
                            }
                            WorktreePromptTarget::OpenWithAgent(
                                commands::WorktreeKind::Pane(_),
                                agent,
                            ) => format!(
                                "Opened worktree {} in a new pane and launched {}",
                                path.display(),
                                agent_display_name(&agent.to_string())
                            ),
                            WorktreePromptTarget::Open(commands::WorktreeKind::Tab) => {
                                format!("Opened worktree {} in a new tab", path.display())
                            }
                            WorktreePromptTarget::OpenWithAgent(
                                commands::WorktreeKind::Tab,
                                agent,
                            ) => format!(
                                "Opened worktree {} in a new tab and launched {}",
                                path.display(),
                                agent_display_name(&agent.to_string())
                            ),
                            WorktreePromptTarget::RestartPane(_) => {
                                format!("Restarted pane in new worktree {}", path.display())
                            }
                            WorktreePromptTarget::RestartPaneWithAgent(_, agent) => format!(
                                "Restarted pane in new worktree {} and launched {}",
                                path.display(),
                                agent_display_name(&agent.to_string())
                            ),
                        };
                    }
                    Err(error) => {
                        // The dialog stays open with the failure inline, so
                        // the user can fix the name and retry.
                        if let Some(prompt) = self.worktree_prompt.as_mut() {
                            prompt.busy = false;
                            prompt.error = Some(error);
                        }
                    }
                }
                return Vec::new();
            }
            Message::NewTab => self.new_tab(),
            Message::CloseTab(workspace_id, tab_id) => self.close_tab(workspace_id, tab_id),
            Message::ConfirmCloseWorkspace(workspace_id) => {
                self.close_workspace_prompt = None;
                self.close_workspace_by_id(workspace_id)
            }
            Message::CloseSessionPicker => {
                let startup = self
                    .session_picker
                    .as_ref()
                    .is_some_and(|picker| picker.startup);
                self.session_picker = None;
                let pane_id = self.focused_pane_id();
                if startup && let Some(pane_id) = pane_id {
                    return self.start_new_session_host(pane_id);
                }
                return Vec::new();
            }
            Message::SessionPickerResume(index) => {
                self.resume_session(index);
                return Vec::new();
            }
            Message::SessionPickerKill(index) => {
                self.kill_picked_session(index);
                return Vec::new();
            }
            Message::SessionPickerKillAll => {
                let count = self
                    .session_picker
                    .as_ref()
                    .map_or(0, |picker| picker.entries.len());
                for index in (0..count).rev() {
                    self.kill_picked_session(index);
                }
                return Vec::new();
            }
            Message::CloseWorktreeManager => {
                self.worktree_manager = None;
                if self.active_view == ActiveView::Settings
                    && self.settings_page == SettingsPage::Worktrees
                {
                    self.active_view = ActiveView::Workspace;
                }
                return Vec::new();
            }
            Message::WorktreeManagerLoaded(generation, result) => {
                let current = self
                    .worktree_manager
                    .as_ref()
                    .is_some_and(|manager| manager.generation == generation);
                if !current {
                    return Vec::new();
                }
                match result {
                    Ok(mut discovery) => {
                        for entry in &mut discovery.entries {
                            entry.used_by = self.pane_using_directory(&entry.path);
                        }
                        if let Some(manager) = self.worktree_manager.as_mut() {
                            manager.loading = false;
                            manager.repo_root = discovery.repo_root;
                            manager.failure = discovery.failure;
                            manager.entries = discovery.entries;
                            manager.selected = 0;
                            manager.error = None;
                        }
                    }
                    Err(error) => {
                        if let Some(manager) = self.worktree_manager.as_mut() {
                            manager.loading = false;
                            manager.failure = None;
                            manager.entries.clear();
                            manager.error = Some(format!(
                                "Could not load worktrees. Check that Git is available, then try again. {error}"
                            ));
                        }
                    }
                }
                return Vec::new();
            }
            Message::RefreshWorktreeManager => {
                let Some(pane_id) = self.focused_pane_id() else {
                    return Vec::new();
                };
                return self.open_worktree_list(WorktreeManagerMode::Manage, pane_id);
            }
            Message::WorktreeManagerDelete(index) => return self.delete_worktree_entry(index),
            Message::WorktreeManagerDeleteUnused => return self.delete_unused_worktrees(),
            Message::OpenPaneWorktreePrompt(pane_id) => {
                return self.open_worktree_prompt(WorktreePromptTarget::RestartPane(pane_id));
            }
            Message::WorktreeManagerRestart(index) => {
                if let Some(manager) = self.worktree_manager.as_mut()
                    && matches!(
                        manager.mode,
                        WorktreeManagerMode::RestartPane(_)
                            | WorktreeManagerMode::RestartPaneWithAgent(_, _)
                    )
                    && index < manager.entries.len()
                {
                    manager.restart_target = Some(index);
                    manager.error = None;
                }
                return Vec::new();
            }
            Message::ConfirmWorktreeManagerRestart => {
                self.confirm_worktree_restart();
                return Vec::new();
            }
            Message::CancelWorktreeManagerRestart => {
                if let Some(manager) = self.worktree_manager.as_mut() {
                    manager.restart_target = None;
                }
                return Vec::new();
            }
            Message::WorktreeManagerDeleted(removed, result) => {
                if let Some(manager) = self.worktree_manager.as_mut() {
                    manager.busy = false;
                    manager
                        .entries
                        .retain(|entry| !removed.contains(&entry.path));
                    if manager.selected >= manager.entries.len() {
                        manager.selected = manager.entries.len().saturating_sub(1);
                    }
                    manager.error = result.err();
                    if !removed.is_empty() {
                        self.status = if removed.len() == 1 {
                            format!("Removed worktree {}", worktree_display_name(&removed[0]))
                        } else {
                            format!("Removed {} worktrees", removed.len())
                        };
                    }
                }
                return Vec::new();
            }
            Message::CancelCloseWorkspace => {
                self.close_workspace_prompt = None;
                return Vec::new();
            }
            Message::BeginTabDrag(workspace_id, tab_id, index) => {
                let _ = self.switch_workspace(workspace_id);
                let _ = self.switch_tab(tab_id);
                self.tab_drag = Some(TabDrag {
                    tab_id,
                    target_workspace_id: workspace_id,
                    target_index: index,
                });
                return Vec::new();
            }
            Message::TabDragOver(workspace_id, index) => {
                if let Some(drag) = &mut self.tab_drag {
                    drag.target_workspace_id = workspace_id;
                    drag.target_index = index;
                }
                return Vec::new();
            }
            Message::OpenSettings => return self.open_settings(),
            Message::InstalledVersionsLoaded(result) => {
                self.installed_versions = match result {
                    Ok(versions) => InstalledVersionsState::Ready(versions),
                    Err(_) => InstalledVersionsState::Unavailable,
                };
                return Vec::new();
            }
            Message::GitHubStatusPressed => {
                return self.open_github_panel();
            }
            Message::BeginGitHubAuth => return self.begin_github_auth(),
            Message::GitHubAuthChecked(generation, status) => {
                if generation != self.github_auth_generation {
                    return Vec::new();
                }
                let authenticated = matches!(status, github::AuthStatus::Authenticated { .. });
                self.github_auth = status;
                if authenticated {
                    self.pane_repositories.clear();
                    self.pending_repository_directories.clear();
                    let repositories = self.refresh_pane_repositories();
                    let panel = if self.github_panel_visible() {
                        self.refresh_github_focused_pane()
                    } else {
                        Vec::new()
                    };
                    let pull_requests = if self
                        .github_panel
                        .as_ref()
                        .is_some_and(|panel| panel.active_tab == GitHubPanelTab::PullRequests)
                    {
                        self.refresh_github_pull_requests()
                    } else {
                        Vec::new()
                    };
                    return effect::batch([repositories, panel, pull_requests]);
                }
                return Vec::new();
            }
            Message::GitHubAuthFinished(generation, result) => {
                if generation != self.github_auth_generation {
                    return Vec::new();
                }
                self.github_auth_busy = false;
                match result {
                    Ok(status) => {
                        self.github_auth = status;
                        self.status = format!("Connected to {}", self.settings.github_host);
                        self.pane_repositories.clear();
                        self.pending_repository_directories.clear();
                        let repositories = self.refresh_pane_repositories();
                        let context = if self.github_panel_visible() {
                            self.refresh_github_focused_pane()
                        } else {
                            Vec::new()
                        };
                        let pull_requests =
                            if self.github_panel.as_ref().is_some_and(|panel| {
                                panel.active_tab == GitHubPanelTab::PullRequests
                            }) {
                                self.refresh_github_pull_requests()
                            } else {
                                Vec::new()
                            };
                        return effect::batch([repositories, context, pull_requests]);
                    }
                    Err(error) => {
                        self.github_auth = github::AuthStatus::NeedsAuthentication;
                        if let Some(panel) = self.github_panel.as_mut() {
                            panel.pull_requests_error = Some(error.clone());
                        }
                        self.status = error;
                    }
                }
                return Vec::new();
            }
            Message::CloseGitHubPanel => {
                self.github_context_cancellation.cancel();
                self.github_context_generation = self.github_context_generation.wrapping_add(1);
                if let Some(panel) = self.github_panel.take() {
                    panel.cancel_requests();
                }
                if let Some(diff) = self.github_diff.take() {
                    diff.cancellation.cancel();
                }
                if self.active_view == ActiveView::GitHubDiff {
                    self.active_view = ActiveView::Workspace;
                }
                return Vec::new();
            }
            Message::RefreshGitHubPanel => return self.refresh_github_panel(),
            Message::RefreshGitHubPullRequestsAfterAgentTurn => {
                self.github_pull_requests_refresh_pending = false;
                let selected_pull_request_visible =
                    self.github_panel.as_ref().is_some_and(|panel| {
                        panel.active_tab == GitHubPanelTab::PullRequests
                            && panel.selected_pull_request_number.is_some()
                    });
                let pull_requests = self.refresh_github_pull_requests();
                return if selected_pull_request_visible {
                    effect::batch([pull_requests, self.refresh_selected_github_pull_request()])
                } else {
                    pull_requests
                };
            }
            Message::GitHubFocusedPaneLoaded(pane_id, generation, purpose, result) => {
                if generation != self.github_context_generation {
                    return Vec::new();
                }
                if purpose == GitHubContextLoad::Open && self.github_panel.is_none() {
                    return Vec::new();
                }
                if self.control_pane_id(None).ok() != Some(pane_id) {
                    if purpose == GitHubContextLoad::Open {
                        self.github_panel = None;
                        return self.open_github_panel();
                    }
                    self.queue_github_pane_refresh();
                    return Vec::new();
                }
                if self.github_panel.is_none() {
                    return Vec::new();
                }
                let (repository, data) = match *result {
                    Ok(loaded) => loaded,
                    Err(error) => {
                        if let Some(panel) = self.github_panel.take() {
                            panel.cancel_requests();
                        }
                        if let Some(diff) = self.github_diff.take() {
                            diff.cancellation.cancel();
                        }
                        self.active_view = ActiveView::Workspace;
                        self.status = error.clone();
                        self.show_toast(&error);
                        return Vec::new();
                    }
                };
                let same_repository = self
                    .github_panel
                    .as_ref()
                    .is_some_and(|panel| panel.repository.root == repository.root);
                let active_tab = self
                    .github_panel
                    .as_ref()
                    .map_or(GitHubPanelTab::Local, |panel| panel.active_tab);

                if same_repository {
                    let mut reload_diff = None;
                    if let Some(diff) = self
                        .github_diff
                        .as_ref()
                        .filter(|diff| diff.source == GitHubDiffSource::Local)
                    {
                        reload_diff = data
                            .files
                            .iter()
                            .find(|file| file.path == diff.path)
                            .map(|file| file.path.clone());
                        if reload_diff.is_none() {
                            if let Some(diff) = self.github_diff.take() {
                                diff.cancellation.cancel();
                            }
                            self.active_view = ActiveView::Workspace;
                            self.status =
                                "The selected file is no longer in the local change set.".into();
                        }
                    }
                    let file_count = data.files.len();
                    let viewport_height = github_file_viewport_height(self.window_size, false);
                    let panel = self.github_panel.as_mut().expect("panel checked above");
                    let clamped = github_clamped_scroll_offset(
                        file_count,
                        panel.file_scroll_offset,
                        viewport_height,
                        GITHUB_FILE_ROW_HEIGHT,
                    );
                    panel.file_scroll_offset = clamped;
                    panel.file_keyboard_cursor = panel
                        .file_keyboard_cursor
                        .map(|cursor| cursor.min(file_count.saturating_sub(1)))
                        .filter(|_| file_count > 0);
                    panel.repository = repository;
                    panel.data = Some(data);
                    panel.context_loading = false;
                    panel.loading = false;
                    panel.error = None;
                    let diff_task =
                        reload_diff.map_or_else(Vec::new, |path| self.open_github_diff(path));
                    return effect::batch([
                        diff_task,
                        github_scroll_to(ScrollTarget::GitHubFiles, clamped),
                    ]);
                }
                if let Some(panel) = self.github_panel.take() {
                    panel.cancel_requests();
                }
                let mut panel = GitHubPanelState::loading(repository);
                panel.active_tab = active_tab;
                panel.data = Some(data);
                panel.loading = false;
                self.github_panel = Some(panel);
                if let Some(diff) = self.github_diff.take() {
                    diff.cancellation.cancel();
                }
                self.active_view = ActiveView::Workspace;
                return if active_tab == GitHubPanelTab::PullRequests {
                    self.refresh_github_pull_requests()
                } else {
                    Vec::new()
                };
            }
            Message::SelectGitHubPanelTab(tab) => {
                let Some(panel) = self.github_panel.as_mut() else {
                    return Vec::new();
                };
                if panel.active_loading() {
                    return Vec::new();
                }
                panel.keyboard_focus = Some(GitHubPanelKeyboardFocus::Tabs);
                if panel.active_tab == tab {
                    return Vec::new();
                }
                panel.active_tab = tab;
                panel.merge_confirmation = false;
                panel.pull_request_keyboard_cursor = None;
                panel.file_keyboard_cursor = None;
                panel.loading_phase = 0;
                if let Some(diff) = self.github_diff.take() {
                    diff.cancellation.cancel();
                }
                self.active_view = ActiveView::Workspace;
                return match tab {
                    GitHubPanelTab::Local if panel.data.is_none() => {
                        self.refresh_github_focused_pane()
                    }
                    GitHubPanelTab::PullRequests if panel.pull_requests.is_none() => {
                        self.refresh_github_pull_requests()
                    }
                    _ => Vec::new(),
                };
            }
            Message::GitHubPullRequestsLoaded(root, generation, result) => {
                let Some(panel) = self.github_panel.as_mut().filter(|panel| {
                    panel.repository.root == root && panel.pull_request_generation == generation
                }) else {
                    return Vec::new();
                };
                panel.pull_requests_loading = false;
                let mut scroll_offset = None;
                match *result {
                    Ok(pull_requests) => {
                        panel.pull_requests = Some(pull_requests);
                        panel.pull_requests_error = None;
                        let count = panel.pull_requests.as_ref().map_or(0, |pull_requests| {
                            pull_requests
                                .iter()
                                .filter(|pull_request| {
                                    pull_request.matches(&panel.pull_request_query)
                                })
                                .count()
                        });
                        let viewport_height = github_pull_request_viewport_height(self.window_size);
                        let clamped = github_clamped_scroll_offset(
                            count,
                            panel.pull_request_scroll_offset,
                            viewport_height,
                            GITHUB_PULL_REQUEST_ROW_HEIGHT,
                        );
                        panel.pull_request_scroll_offset = clamped;
                        panel.pull_request_keyboard_cursor = panel
                            .pull_request_keyboard_cursor
                            .map(|cursor| cursor.min(count.saturating_sub(1)))
                            .filter(|_| count > 0);
                        scroll_offset = Some(clamped);
                    }
                    Err(error) => panel.pull_requests_error = Some(error),
                }
                return scroll_offset.map_or_else(Vec::new, |offset| {
                    github_scroll_to(ScrollTarget::GitHubPullRequests, offset)
                });
            }
            Message::GitHubPullRequestQueryChanged(query) => {
                if let Some(panel) = self.github_panel.as_mut() {
                    panel.pull_request_query = query;
                    panel.pull_request_scroll_offset = 0.0;
                    panel.pull_request_keyboard_cursor = None;
                    panel.keyboard_focus = Some(GitHubPanelKeyboardFocus::Search);
                }
                return github_scroll_to(ScrollTarget::GitHubPullRequests, 0.0);
            }
            Message::GitHubPullRequestScrolled(offset) => {
                if let Some(panel) = self.github_panel.as_mut() {
                    panel.pull_request_scroll_offset = offset.max(0.0);
                }
                return Vec::new();
            }
            Message::SelectGitHubPullRequest(number) => {
                return self.select_github_pull_request(number);
            }
            Message::CloseGitHubPullRequest => {
                if let Some(panel) = self.github_panel.as_mut() {
                    panel.close_selected_pull_request();
                }
                if self
                    .github_diff
                    .as_ref()
                    .is_some_and(|diff| matches!(diff.source, GitHubDiffSource::PullRequest(_)))
                {
                    if let Some(diff) = self.github_diff.take() {
                        diff.cancellation.cancel();
                    }
                    self.active_view = ActiveView::Workspace;
                }
                return Vec::new();
            }
            Message::GitHubPullRequestLoaded(root, number, generation, result) => {
                let Some(panel) = self.github_panel.as_mut().filter(|panel| {
                    panel.repository.root == root
                        && panel.selected_pull_request_number == Some(number)
                        && panel.pull_request_detail_generation == generation
                }) else {
                    return Vec::new();
                };
                panel.selected_pull_request_loading = false;
                let mut reload_diff = None;
                let mut scroll_offset = None;
                match *result {
                    Ok(details) => {
                        if let Some(diff) = self
                            .github_diff
                            .as_ref()
                            .filter(|diff| diff.source == GitHubDiffSource::PullRequest(number))
                        {
                            reload_diff = details
                                .files
                                .iter()
                                .find(|file| file.path == diff.path)
                                .map(|file| file.path.clone());
                            if reload_diff.is_none() {
                                if let Some(diff) = self.github_diff.take() {
                                    diff.cancellation.cancel();
                                }
                                self.active_view = ActiveView::Workspace;
                                self.status =
                                    "The selected file is no longer in the pull request.".into();
                            }
                        }
                        let viewport_height = github_file_viewport_height(self.window_size, true);
                        let clamped = github_clamped_scroll_offset(
                            details.files.len(),
                            panel.selected_pull_request_file_scroll_offset,
                            viewport_height,
                            GITHUB_FILE_ROW_HEIGHT,
                        );
                        panel.selected_pull_request_file_scroll_offset = clamped;
                        panel.file_keyboard_cursor = panel
                            .file_keyboard_cursor
                            .map(|cursor| cursor.min(details.files.len().saturating_sub(1)))
                            .filter(|_| !details.files.is_empty());
                        scroll_offset = Some(clamped);
                        panel.selected_pull_request = Some(details);
                        panel.selected_pull_request_error = None;
                    }
                    Err(error) => panel.selected_pull_request_error = Some(error),
                }
                let diff_task =
                    reload_diff.map_or_else(Vec::new, |path| self.open_github_diff(path));
                let scroll_task = scroll_offset.map_or_else(Vec::new, |offset| {
                    github_scroll_to(ScrollTarget::GitHubFiles, offset)
                });
                return effect::batch([diff_task, scroll_task]);
            }
            Message::GitHubFileScrolled(offset) => {
                if let Some(panel) = self.github_panel.as_mut() {
                    if panel.active_tab == GitHubPanelTab::PullRequests
                        && panel.selected_pull_request_number.is_some()
                    {
                        panel.selected_pull_request_file_scroll_offset = offset.max(0.0);
                    } else {
                        panel.file_scroll_offset = offset.max(0.0);
                    }
                }
                return Vec::new();
            }
            Message::OpenGitHubDiff(path) => {
                if let Some(panel) = self.github_panel.as_mut() {
                    panel.keyboard_focus = Some(GitHubPanelKeyboardFocus::Files);
                }
                return self.open_github_diff(path);
            }
            Message::RetryGitHubDiff => {
                let Some(path) = self.github_diff.as_ref().map(|diff| diff.path.clone()) else {
                    return Vec::new();
                };
                return self.open_github_diff(path);
            }
            Message::CloseGitHubDiff => {
                if let Some(diff) = self.github_diff.take() {
                    diff.cancellation.cancel();
                }
                self.active_view = ActiveView::Workspace;
                return Vec::new();
            }
            Message::GitHubDiffLoaded(root, path, generation, result) => {
                let wrap_columns = github_diff_wrap_columns(
                    self.window_size.width,
                    self.settings.terminal_cell_width(),
                );
                let Some(diff) = self
                    .github_diff
                    .as_mut()
                    .filter(|diff| diff.path == path && diff.generation == generation)
                else {
                    return Vec::new();
                };
                if self
                    .github_panel
                    .as_ref()
                    .is_none_or(|panel| panel.repository.root != root)
                {
                    return Vec::new();
                }
                diff.loading = false;
                match *result {
                    Ok(document) => {
                        diff.line_starts = github_diff_line_starts(&document, wrap_columns);
                        diff.wrap_columns = wrap_columns;
                        diff.document = Some(document);
                        diff.error = None;
                    }
                    Err(error) => diff.error = Some(error),
                }
                return Vec::new();
            }
            Message::GitHubDiffScrolled(offset) => {
                if let Some(diff) = self.github_diff.as_mut() {
                    diff.scroll_offset = offset.max(0.0);
                }
                return Vec::new();
            }
            Message::OpenGitHubPullRequest(url) => {
                let target = url.clone();
                return vec![Effect::Perform(Box::new(move || {
                    let result = open_web_url(&target).map_err(|error| error.to_string());
                    Message::TerminalLinkOpened(url, result)
                }))];
            }
            Message::ToggleGitHubPullRequestDraft => {
                return self.toggle_github_pull_request_draft();
            }
            Message::GitHubPullRequestDraftChanged(root, number, generation, draft, result) => {
                let Some(panel) = self.github_panel.as_mut().filter(|panel| {
                    panel.repository.root == root && panel.action_generation == generation
                }) else {
                    return Vec::new();
                };
                panel.draft_state_updating = false;
                match result {
                    Ok(status) => {
                        panel.mark_pull_request_draft(number, draft);
                        panel.pull_request_action_error = None;
                        let state = if draft {
                            github::CurrentPullRequestState::Draft
                        } else {
                            github::CurrentPullRequestState::Open
                        };
                        self.set_cached_pull_request_state(&root, number, state);
                        self.status = status;
                    }
                    Err(error) => {
                        if panel.selected_pull_request_number == Some(number) {
                            panel.pull_request_action_error = Some(error.clone());
                        }
                        self.status = error;
                    }
                }
                return Vec::new();
            }
            Message::RequestGitHubMerge => {
                if let Some(panel) = self.github_panel.as_mut()
                    && !panel.merging
                    && !panel.draft_state_updating
                    && panel.selected_pull_request.as_ref().is_some_and(|details| {
                        details.pull_request.readiness() == github::MergeReadiness::Ready
                    })
                {
                    panel.merge_confirmation = true;
                    panel.pull_request_action_error = None;
                }
                return Vec::new();
            }
            Message::CancelGitHubMerge => {
                if let Some(panel) = self.github_panel.as_mut() {
                    panel.merge_confirmation = false;
                }
                return Vec::new();
            }
            Message::ConfirmGitHubMerge => return self.confirm_github_merge(),
            Message::GitHubMergeFinished(root, number, generation, result) => {
                let Some(panel) = self.github_panel.as_mut().filter(|panel| {
                    panel.repository.root == root && panel.action_generation == generation
                }) else {
                    return Vec::new();
                };
                panel.merging = false;
                panel.merge_confirmation = false;
                match result {
                    Ok(status) => {
                        panel.mark_pull_request_merged(number);
                        panel.close_selected_pull_request();
                        if self.github_diff.as_ref().is_some_and(|diff| {
                            diff.source == GitHubDiffSource::PullRequest(number)
                        }) {
                            if let Some(diff) = self.github_diff.take() {
                                diff.cancellation.cancel();
                            }
                            self.active_view = ActiveView::Workspace;
                        }
                        self.set_cached_pull_request_state(
                            &root,
                            number,
                            github::CurrentPullRequestState::Merged,
                        );
                        self.status = status;
                        return self.refresh_github_pull_requests();
                    }
                    Err(error) => {
                        if panel.selected_pull_request_number == Some(number) {
                            panel.selected_pull_request_error = Some(error.clone());
                        }
                        self.status = error;
                    }
                }
                return Vec::new();
            }
            Message::OpenSettingsPage(SettingsPage::Preferences) => {
                self.settings_page = SettingsPage::Preferences;
                return Vec::new();
            }
            Message::OpenSettingsPage(SettingsPage::Worktrees) => {
                self.settings_page = SettingsPage::Worktrees;
                let Some(pane_id) = self.focused_pane_id() else {
                    return Vec::new();
                };
                return self.open_worktree_list(WorktreeManagerMode::Manage, pane_id);
            }
            Message::ToggleSidebar => {
                self.sidebar_collapsed = !self.sidebar_collapsed;
                return Vec::new();
            }
            Message::ToggleMaximize(pane_id) => {
                self.maximized_pane = if self.maximized_pane == Some(pane_id) {
                    None
                } else {
                    Some(pane_id)
                };
                let _ = self.focus_pane(pane_id);
                return Vec::new();
            }
            Message::ToggleMaximizeFromPaneMenu(pane_id) => {
                self.pane_menu = None;
                return self.update(Message::ToggleMaximize(pane_id));
            }
            Message::TogglePaneMenu(pane_id) => {
                if self.pane_menu == Some(pane_id) {
                    self.pane_menu = None;
                } else {
                    // Focus follows the menu so its actions and the clipboard
                    // chords always target the same pane.
                    let _ = self.focus_pane(pane_id);
                    self.pane_menu = Some(pane_id);
                }
                return Vec::new();
            }
            Message::DismissPaneMenu => {
                self.pane_menu = None;
                return Vec::new();
            }
            Message::DismissGlobalAlert(index) => {
                if index < self.global_alerts.len() {
                    self.global_alerts.remove(index);
                }
                return Vec::new();
            }
            Message::ManageHooks(agent, action) => {
                return self.manage_hooks(agent, action);
            }
            Message::RefreshHookStatus => {
                return self.refresh_integrations();
            }
            Message::IntegrationDiscoveryFinished(generation, result) => {
                if generation != self.integration_generation {
                    return Vec::new();
                }
                self.integration_refreshing = false;
                match result {
                    Ok(discovery) => {
                        self.available_wsl_distributions = discovery.wsl_distributions;
                        match discovery.hook_statuses {
                            Ok(statuses) => {
                                self.hook_statuses = statuses;
                                self.update_stale_hook_alert();
                            }
                            Err(error) => {
                                self.hook_statuses.clear();
                                self.status = format!("Could not refresh agent hooks: {error}");
                            }
                        }
                    }
                    Err(error) => {
                        self.hook_statuses.clear();
                        self.status = format!("Could not inspect agent integrations: {error}");
                    }
                }
                return Vec::new();
            }
            Message::HookOperationFinished(generation, result) => {
                if generation != self.integration_generation {
                    return Vec::new();
                }
                self.integration_refreshing = false;
                match result {
                    Ok(result) => {
                        self.status = result.message;
                        self.hook_statuses = result.statuses;
                        self.update_stale_hook_alert();
                    }
                    Err(error) => self.status = format!("Could not update agent hooks: {error}"),
                }
                return Vec::new();
            }
            Message::SettingsTerminalFont(font) => {
                self.settings_draft.terminal_font = font;
                self.available_terminal_font_weights = self
                    .installed_fonts
                    .terminal_weights(&self.settings_draft.terminal_font);
                if self.available_terminal_font_weights.is_empty() {
                    self.available_terminal_font_weights
                        .push(FontWeight::Normal);
                }
                if !self
                    .available_terminal_font_weights
                    .contains(&self.settings_draft.terminal_font_weight)
                {
                    self.settings_draft.terminal_font_weight =
                        self.available_terminal_font_weights[0];
                }
                return Vec::new();
            }
            Message::SettingsTerminalFontWeight(weight) => {
                self.settings_draft.terminal_font_weight = weight;
                return Vec::new();
            }
            Message::SettingsTerminalTheme(theme) => {
                self.settings_draft.terminal_theme = theme;
                return Vec::new();
            }
            Message::SettingsAppearance(appearance) => {
                self.settings_draft.appearance = appearance;
                return Vec::new();
            }
            Message::SettingsShowStatusBar(show) => {
                self.settings_draft.show_status_bar = show;
                return Vec::new();
            }
            Message::SettingsUiFont(font) => {
                self.settings_draft.ui_font = font;
                self.available_ui_font_weights = self
                    .installed_fonts
                    .ui_weights(&self.settings_draft.ui_font);
                if self.available_ui_font_weights.is_empty() {
                    self.available_ui_font_weights.push(FontWeight::Normal);
                }
                if !self
                    .available_ui_font_weights
                    .contains(&self.settings_draft.ui_font_weight)
                {
                    self.settings_draft.ui_font_weight = self.available_ui_font_weights[0];
                }
                return Vec::new();
            }
            Message::SettingsUiFontWeight(weight) => {
                self.settings_draft.ui_font_weight = weight;
                return Vec::new();
            }
            Message::SettingsTerminalFontSize(size) => {
                self.settings_draft.terminal_font_size = size;
                return Vec::new();
            }
            Message::SettingsLineHeight(height) => {
                self.settings_draft.terminal_line_height = height;
                return Vec::new();
            }
            Message::SettingsScrollbackLimit(limit) => {
                if limit
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == ',')
                {
                    self.settings_scrollback_lines_input = limit;
                    if let Ok(lines) = settings::parse_terminal_scrollback_lines(
                        &self.settings_scrollback_lines_input,
                    ) {
                        self.settings_draft.terminal_scrollback_lines = lines;
                    }
                }
                return Vec::new();
            }
            Message::SettingsUiFontSize(size) => {
                self.settings_draft.ui_font_size = size;
                return Vec::new();
            }
            Message::SettingsShowAllWorkspaces(show_all) => {
                self.settings_draft.fleet_scope = if show_all {
                    FleetScope::AllWorkspaces
                } else {
                    FleetScope::CurrentWorkspace
                };
                return Vec::new();
            }
            Message::SettingsDefaultAgent(choice) => {
                self.settings_draft.default_agent = match choice {
                    DefaultAgentChoice::None => None,
                    DefaultAgentChoice::Agent(agent) => Some(agent),
                };
                return Vec::new();
            }
            Message::SetFleetView(view) => {
                self.set_fleet_view(view);
                return self.refresh_pane_repositories();
            }
            Message::SettingsGitHubHost(host) => {
                self.settings_draft.github_host = host;
                return Vec::new();
            }
            Message::SettingsCodexCommand(command) => {
                self.settings_draft.codex_command = command;
                return Vec::new();
            }
            Message::SettingsClaudeCommand(command) => {
                self.settings_draft.claude_command = command;
                return Vec::new();
            }
            Message::SettingsPiCommand(command) => {
                self.settings_draft.pi_command = command;
                return Vec::new();
            }
            #[cfg(target_os = "windows")]
            Message::SettingsWindowsShellBackend(backend) => {
                self.settings_draft.windows_shell_backend = backend;
                return self.refresh_integrations();
            }
            #[cfg(target_os = "windows")]
            Message::SettingsWslDistribution(distribution) => {
                self.settings_draft.wsl_distribution = distribution.0.unwrap_or_default();
                return self.refresh_integrations();
            }
            #[cfg(target_os = "windows")]
            Message::RefreshWslDistributions => {
                return self.refresh_integrations();
            }
            Message::SaveSettings => {
                return self.save_settings();
            }
            Message::OpenThemeGallery => {
                self.active_view = ActiveView::ThemeGallery;
                return Vec::new();
            }
            Message::CloseThemeGallery => {
                self.active_view = ActiveView::Settings;
                return Vec::new();
            }
            Message::GalleryThemeChosen(theme) => {
                // The gallery's accent ring reads as a commitment, so the
                // click IS the commit: draft, settings, and live panes all
                // move together.
                self.settings_draft.terminal_theme = theme;
                self.settings.terminal_theme = theme;
                let terminal_theme = self.settings.terminal_theme.preset().terminal_theme();
                for runtime in self.terminals.values() {
                    if let Some(session) = &runtime.session {
                        let _ = session.apply_theme(terminal_theme);
                    }
                }
                return Vec::new();
            }
            Message::CancelSettings => {
                self.reset_settings_draft();
                self.pending_default_agent_command = None;
                self.active_view = ActiveView::Workspace;
                self.status = "Settings changes discarded".into();
                return Vec::new();
            }
            Message::CloseDefaultAgentPrompt => {
                self.default_agent_prompt = false;
                self.pending_default_agent_command = None;
                return Vec::new();
            }
            Message::OpenDefaultAgentSettings => {
                self.default_agent_prompt = false;
                let version_task = self.open_settings();
                let scroll_task = vec![Effect::ScrollToRatio(ScrollTarget::Settings, 1.0)];
                return effect::batch([version_task, scroll_task]);
            }
            #[cfg(feature = "e2e")]
            Message::E2eTick => return self.drive_e2e(),
            #[cfg(feature = "e2e")]
            Message::E2eScreenshot(screenshot) => return self.finish_e2e(screenshot),
            #[cfg(feature = "e2e")]
            Message::E2eWindowMissing => return self.fail_e2e("Iced window was not available"),
        };

        self.status = match result {
            Ok(()) => "Workspace updated".into(),
            Err(error) => error,
        };
        Vec::new()
    }

    pub(crate) fn window_resize_increment_task(&self) -> Vec<Effect> {
        // Before the window exists there is nothing to constrain; the effect
        // is re-issued once it opens.
        if !self.window_open {
            return Vec::new();
        }
        let Some(increments) = wsl_wayland_resize_increments(
            muxtrix::gpu::is_wsl(),
            std::env::var_os("WAYLAND_DISPLAY").is_some(),
            std::env::var_os("WINIT_UNIX_BACKEND")
                .is_some_and(|backend| backend.to_string_lossy().eq_ignore_ascii_case("x11")),
            &self.settings,
        ) else {
            return Vec::new();
        };
        vec![Effect::SetResizeIncrements(increments)]
    }

    pub(crate) fn default_terminal_profile(&self) -> Result<LaunchProfile, String> {
        self.session
            .profiles
            .first()
            .cloned()
            .ok_or_else(|| "terminal launch profile is missing".to_owned())
    }

    pub(crate) fn regular_terminal_profile(&self) -> Result<LaunchProfile, String> {
        let mut profile = self.default_terminal_profile()?;
        if let Some(directory) = self.regular_creation_directory() {
            profile.working_directory = Some(directory);
        }
        Ok(profile)
    }

    pub(crate) fn split_terminal(&mut self, axis: SplitAxis) -> Result<(), String> {
        if self.maximized_pane.is_some() {
            return Err("Restore panes before splitting the focused pane".into());
        }
        let profile = self.regular_terminal_profile()?;
        let pane_count = self
            .active_workspace()?
            .active_tab()
            .ok_or_else(|| "active tab is missing".to_owned())?
            .panes
            .len();
        let tab_id = self
            .active_workspace()?
            .active_tab()
            .ok_or_else(|| "active tab is missing".to_owned())?
            .id;
        self.clear_manual_layout_history(tab_id);
        let title = format!("shell {}", pane_count + 1);
        let surface = Surface::terminal(
            &title,
            TerminalSurface {
                profile_id: profile.id,
                working_directory: profile.working_directory.clone(),
            },
        );
        let pane_id = self
            .active_workspace_mut()?
            .split_focused(axis, SplitRatio::EQUAL, surface)
            .map_err(|error| error.to_string())?;
        self.request_regular_terminal_launch(profile, pane_id, title)
    }

    pub(crate) fn clear_manual_layout_history(&mut self, tab_id: TabId) {
        self.pane_layouts.remove(&tab_id);
        self.base_pane_layouts.remove(&tab_id);
        self.pane_resize_history.remove(&tab_id);
        self.split_sizes.retain(|key, _| key.tab_id != tab_id);
        if self
            .split_drag
            .as_ref()
            .is_some_and(|drag| drag.key.tab_id == tab_id)
        {
            self.split_drag = None;
        }
    }

    pub(crate) fn cycle_pane_layout(&mut self, cycle: LayoutCycle) -> Result<&'static str, String> {
        if self.maximized_pane.is_some() {
            return Err("Restore panes before changing their layout".into());
        }
        let (tab_id, pane_ids, current_root) = {
            let tab = self
                .active_workspace()?
                .active_tab()
                .ok_or_else(|| "active tab is missing".to_owned())?;
            (tab.id, pane_ids_for_layout(tab), tab.root.clone())
        };
        if pane_ids.len() < 2 {
            return Err("Open another pane before changing the layout".into());
        }

        let layouts: Vec<_> = PaneLayout::ALL
            .into_iter()
            .filter(|layout| layout.supports(pane_ids.len()))
            .collect();
        let current = self
            .pane_layouts
            .get(&tab_id)
            .copied()
            .unwrap_or(PaneLayout::Base);
        let index = layouts
            .iter()
            .position(|layout| *layout == current)
            .unwrap_or(0);
        let next_index = match cycle {
            LayoutCycle::Previous => (index + layouts.len() - 1) % layouts.len(),
            LayoutCycle::Next => (index + 1) % layouts.len(),
        };
        let next = layouts[next_index];

        if current == PaneLayout::Base {
            if same_panes(&current_root.pane_ids(), &pane_ids) {
                self.base_pane_layouts.insert(tab_id, current_root);
            } else {
                // A pane can still be present in the tab and Fleet even if an
                // older or interrupted layout projection omitted it. Do not
                // preserve that broken projection as Base: the generated
                // layout below restores every live pane to the workspace.
                self.base_pane_layouts.remove(&tab_id);
            }
        }
        let next_root = if next == PaneLayout::Base {
            self.base_pane_layouts
                .remove(&tab_id)
                .filter(|root| same_panes(&root.pane_ids(), &pane_ids))
                .unwrap_or_else(|| pane_layout_tree(PaneLayout::Vertical, &pane_ids))
        } else {
            pane_layout_tree(next, &pane_ids)
        };
        self.active_workspace_mut()?
            .tab_mut(tab_id)
            .ok_or_else(|| "active tab is missing".to_owned())?
            .root = next_root;
        self.pane_layouts.insert(tab_id, next);
        self.pane_resize_history.remove(&tab_id);
        self.split_sizes.retain(|key, _| key.tab_id != tab_id);
        self.split_drag = None;
        self.maximized_pane = None;
        self.pane_menu = None;
        Ok(next.label())
    }

    pub(crate) fn resize_focused_pane(&mut self, increase: bool) -> Result<&'static str, String> {
        if self.maximized_pane.is_some() {
            return Err("Restore panes before resizing the focused pane".into());
        }
        let (tab_id, pane_id, root) = {
            let tab = self
                .active_workspace()?
                .active_tab()
                .ok_or_else(|| "active tab is missing".to_owned())?;
            (tab.id, tab.focused_pane_id, tab.root.clone())
        };

        if !increase {
            let Some(history) = self.pane_resize_history.get(&tab_id) else {
                return Err("No focused-pane resize to restore".into());
            };
            if history.pane_id != pane_id || history.snapshots.is_empty() {
                self.pane_resize_history.remove(&tab_id);
                return Err("No focused-pane resize to restore".into());
            }
            let (snapshot, history_empty) = {
                let history = self
                    .pane_resize_history
                    .get_mut(&tab_id)
                    .expect("the resize history was just checked");
                let snapshot = history
                    .snapshots
                    .pop()
                    .expect("the resize history was just checked as non-empty");
                (snapshot, history.snapshots.is_empty())
            };
            self.active_workspace_mut()?
                .tab_mut(tab_id)
                .ok_or_else(|| "active tab is missing".to_owned())?
                .root = snapshot.root;
            self.maximized_pane = snapshot.maximized_pane;
            if history_empty {
                self.pane_resize_history.remove(&tab_id);
            }
            self.split_sizes.retain(|key, _| key.tab_id != tab_id);
            self.split_drag = None;
            return Ok("Restored previous pane size");
        }

        let mut next_root = root.clone();
        let preferred_direction = zellij_resize_direction(&pane_rects(&root), pane_id);
        let grew = preferred_direction.is_some_and(|direction| {
            enlarge_focused_tree_toward(&mut next_root, pane_id, direction)
        }) || enlarge_focused_tree(&mut next_root, pane_id);
        if !grew && self.maximized_pane == Some(pane_id) {
            return Err("The focused pane already fills the workspace".into());
        }
        let snapshot = PaneResizeSnapshot {
            root,
            maximized_pane: self.maximized_pane,
        };
        let history = self
            .pane_resize_history
            .entry(tab_id)
            .or_insert_with(|| PaneResizeHistory {
                pane_id,
                snapshots: Vec::new(),
            });
        if history.pane_id != pane_id {
            *history = PaneResizeHistory {
                pane_id,
                snapshots: Vec::new(),
            };
        }
        history.snapshots.push(snapshot);

        self.pane_layouts.remove(&tab_id);
        self.base_pane_layouts.remove(&tab_id);
        self.split_sizes.retain(|key, _| key.tab_id != tab_id);
        self.split_drag = None;
        if grew {
            self.active_workspace_mut()?
                .tab_mut(tab_id)
                .ok_or_else(|| "active tab is missing".to_owned())?
                .root = next_root;
            self.maximized_pane = None;
            Ok("Grew focused pane")
        } else {
            self.maximized_pane = Some(pane_id);
            Ok("Focused pane fills the workspace")
        }
    }

    pub(crate) fn open_workspace_create(&mut self) -> Vec<Effect> {
        self.workspace_name_draft = format!("Workspace {}", self.session.workspaces.len() + 1);
        self.workspace_create_visible = true;
        self.rename_prompt = None;
        self.close_command_palette();
        vec![Effect::Focus(FocusTarget::WorkspaceCreate)]
    }

    pub(crate) fn create_workspace(&mut self) -> Result<(), String> {
        let profile = self.regular_terminal_profile()?;
        let workspace_name = self.workspace_name_draft.trim().to_owned();
        if workspace_name.is_empty() {
            return Err("Workspace names cannot be empty".into());
        }
        let title = "shell 1";
        let workspace = Workspace::new(
            &workspace_name,
            Surface::terminal(
                title,
                TerminalSurface {
                    profile_id: profile.id,
                    working_directory: profile.working_directory.clone(),
                },
            ),
        );
        let pane_id = workspace
            .active_tab()
            .ok_or_else(|| "new workspace is missing its default tab".to_owned())?
            .focused_pane_id;
        self.session
            .add_workspace(workspace)
            .map_err(|error| error.to_string())?;
        self.request_regular_terminal_launch(profile, pane_id, title.into())?;
        self.workspace_name_draft = workspace_name;
        self.rename_prompt = None;
        self.workspace_create_visible = false;
        self.active_view = ActiveView::Workspace;
        self.maximized_pane = None;
        self.pane_menu = None;
        Ok(())
    }

    pub(crate) fn switch_workspace(&mut self, workspace_id: WorkspaceId) -> Result<(), String> {
        let changed = self.session.active_workspace_id != workspace_id;
        self.session
            .switch_workspace(workspace_id)
            .map_err(|error| error.to_string())?;
        self.workspace_name_draft = self.active_workspace()?.name.clone();
        self.rename_prompt = None;
        self.active_view = ActiveView::Workspace;
        self.maximized_pane = None;
        self.pane_menu = None;
        self.hovered_terminal = None;
        if changed {
            self.queue_github_pane_refresh();
        }
        Ok(())
    }

    pub(crate) fn apply_rename(&mut self) -> Result<(), String> {
        let target = self
            .rename_prompt
            .ok_or_else(|| "Nothing is being renamed".to_owned())?;
        match target {
            RenameTarget::Workspace(workspace_id) => {
                self.session
                    .rename_workspace(workspace_id, &self.rename_draft)
                    .map_err(|error| error.to_string())?;
                self.workspace_name_draft = self.active_workspace()?.name.clone();
            }
            RenameTarget::Tab(workspace_id, tab_id) => {
                self.session
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.id == workspace_id)
                    .ok_or_else(|| "workspace is missing".to_owned())?
                    .rename_tab(tab_id, &self.rename_draft)
                    .map_err(|error| error.to_string())?;
            }
            RenameTarget::Pane(pane_id) => {
                let name = self.rename_draft.trim().to_owned();
                let pane = self
                    .session
                    .workspaces
                    .iter_mut()
                    .find_map(|workspace| workspace.pane_mut(pane_id))
                    .ok_or_else(|| format!("pane {pane_id:?} is missing"))?;
                pane.custom_name = (!name.is_empty()).then_some(name);
            }
        }
        Ok(())
    }

    pub(crate) fn open_rename_prompt(
        &mut self,
        target: RenameTarget,
        current_name: String,
    ) -> Vec<Effect> {
        self.rename_prompt = Some(target);
        self.rename_draft = current_name;
        self.active_view = ActiveView::Workspace;
        vec![Effect::Focus(FocusTarget::Rename)]
    }

    /// The directory the focused pane is working in, best effort: agent
    /// lifecycle reports first, then terminal-owned sources.
    pub(crate) fn pane_working_directory(&self, pane_id: PaneId) -> Option<std::path::PathBuf> {
        if let Some(cwd) = self
            .agent_statuses
            .get(&pane_id)
            .and_then(|status| status.cwd.as_deref())
            .map(str::trim)
            .filter(|cwd| !cwd.is_empty())
        {
            return Some(std::path::PathBuf::from(cwd));
        }
        self.pane_terminal_directory(pane_id)
    }

    /// A pane directory derived without agent lifecycle state. Destructive
    /// terminal actions use this path so stale hooks cannot choose their
    /// target repository or worktree.
    pub(crate) fn pane_terminal_directory(&self, pane_id: PaneId) -> Option<std::path::PathBuf> {
        #[cfg(target_os = "linux")]
        if let Some(process_id) = self
            .terminals
            .get(&pane_id)
            .and_then(|runtime| runtime.session.as_ref())
            .and_then(LiveSession::process_id)
            && let Ok(cwd) = std::fs::read_link(format!("/proc/{process_id}/cwd"))
        {
            return Some(cwd);
        }
        // The shell's own report (OSC 7/9/1337). This is the only live source
        // on a Windows build with WSL panes, where the process and its
        // filesystem are on the other side of the VM boundary.
        if let Some(pwd) = self
            .terminals
            .get(&pane_id)
            .and_then(|runtime| runtime.snapshot.as_ref())
            .and_then(|snapshot| snapshot.pwd.as_deref())
            .and_then(decode_reported_pwd)
        {
            return Some(pwd);
        }
        self.session
            .workspaces
            .iter()
            .find_map(|workspace| workspace.pane(pane_id))
            .and_then(Pane::active_surface)
            .and_then(|surface| match &surface.kind {
                muxtrix_domain::SurfaceKind::Terminal(terminal) => {
                    terminal.working_directory.clone().or_else(|| {
                        self.session
                            .profiles
                            .iter()
                            .find(|profile| profile.id == terminal.profile_id)
                            .and_then(|profile| profile.working_directory.clone())
                    })
                }
                _ => None,
            })
    }

    /// The focused pane's working directory.
    pub(crate) fn focused_pane_directory(&self) -> Option<std::path::PathBuf> {
        let pane_id = self
            .active_workspace()
            .ok()?
            .active_tab()
            .map(|tab| tab.focused_pane_id)?;
        self.pane_working_directory(pane_id)
    }

    /// The source directory for regular pane, tab, and workspace creation.
    /// Worktree routing happens on the terminal-launch worker so reading Git
    /// or crossing into WSL can never block the UI action.
    pub(crate) fn regular_creation_directory(&self) -> Option<std::path::PathBuf> {
        self.focused_pane_directory()
    }

    pub(crate) fn confirm_worktree(&mut self) -> Vec<Effect> {
        let Some(prompt) = self.worktree_prompt.clone() else {
            return Vec::new();
        };
        if prompt.busy {
            return Vec::new();
        }
        let Some(repo_root) = prompt.repo_root else {
            return Vec::new();
        };
        let set_error = |this: &mut Self, message: String| {
            if let Some(prompt) = this.worktree_prompt.as_mut() {
                prompt.error = Some(message);
            }
        };
        let name = worktree_name(&self.worktree_name_draft);
        if name.is_empty() {
            set_error(self, "Worktree names cannot be empty".into());
            return Vec::new();
        }
        let Some(base) = prompt.base_directory.clone() else {
            set_error(self, "The home directory could not be discovered".into());
            return Vec::new();
        };
        if prompt.taken_names.contains(&name) {
            set_error(self, format!("{name} already exists for this repository"));
            return Vec::new();
        }
        let destination = worktree_destination(&base, &name);
        if let Some(prompt) = self.worktree_prompt.as_mut() {
            prompt.error = None;
            prompt.busy = true;
        }
        self.status = format!("Creating worktree {name}…");
        let target = prompt.target;
        let wsl_distribution = self.settings.wsl_distribution.clone();
        perform_blocking(
            move || create_git_worktree(&repo_root, &destination, &name, &wsl_distribution),
            move |result| Message::WorktreeCreated(target, result.and_then(|inner| inner)),
        )
    }

    pub(crate) fn open_worktree_prompt(&mut self, target: WorktreePromptTarget) -> Vec<Effect> {
        self.active_view = ActiveView::Workspace;
        let pane_id = match target {
            WorktreePromptTarget::Open(_) | WorktreePromptTarget::OpenWithAgent(_, _) => self
                .active_workspace()
                .ok()
                .and_then(Workspace::active_tab)
                .map(|tab| tab.focused_pane_id),
            WorktreePromptTarget::RestartPane(pane_id)
            | WorktreePromptTarget::RestartPaneWithAgent(pane_id, _) => Some(pane_id),
        };
        let probed_directory = pane_id
            .and_then(|pane_id| match target {
                WorktreePromptTarget::Open(_) | WorktreePromptTarget::OpenWithAgent(_, _) => {
                    self.pane_working_directory(pane_id)
                }
                WorktreePromptTarget::RestartPane(_)
                | WorktreePromptTarget::RestartPaneWithAgent(_, _) => {
                    self.pane_terminal_directory(pane_id)
                }
            })
            .filter(|directory| reported_path_is_concrete(directory));
        let wsl_distribution = self.settings.wsl_distribution.clone();
        let repo_root = probed_directory
            .as_deref()
            .and_then(|directory| git_repository_root(directory, &wsl_distribution));
        let base_directory = repo_root
            .as_deref()
            .and_then(|root| worktree_base_directory(root, &wsl_distribution));
        let taken_names = base_directory
            .as_deref()
            .map(|base| worktree_taken_names(base, &wsl_distribution))
            .unwrap_or_default();
        let failure = repo_root
            .is_none()
            .then(|| worktree_failure_message(probed_directory.as_deref(), &wsl_distribution));
        self.worktree_name_draft = if repo_root.is_some() {
            default_worktree_name(&taken_names)
        } else {
            String::new()
        };
        self.worktree_prompt = Some(WorktreePrompt {
            target,
            repo_root,
            failure,
            base_directory,
            taken_names,
            error: None,
            busy: false,
        });
        vec![Effect::Focus(FocusTarget::Worktree)]
    }

    /// Opens the worktree manager for the focused pane's repository,
    /// listing every linked worktree with who is using it.
    pub(crate) fn open_worktree_manager(&mut self) -> Vec<Effect> {
        let Some(pane_id) = self
            .active_workspace()
            .ok()
            .and_then(Workspace::active_tab)
            .map(|tab| tab.focused_pane_id)
        else {
            return Vec::new();
        };
        let version_task = self.open_settings();
        self.settings_page = SettingsPage::Worktrees;
        effect::batch([
            version_task,
            self.open_worktree_list(WorktreeManagerMode::Manage, pane_id),
        ])
    }

    pub(crate) fn open_worktree_switcher(&mut self, pane_id: PaneId) -> Vec<Effect> {
        if let Err(error) = self.focus_pane(pane_id) {
            self.status = error;
            return Vec::new();
        }
        self.open_worktree_list(WorktreeManagerMode::RestartPane(pane_id), pane_id)
    }

    pub(crate) fn open_worktree_list(
        &mut self,
        mode: WorktreeManagerMode,
        pane_id: PaneId,
    ) -> Vec<Effect> {
        if matches!(
            mode,
            WorktreeManagerMode::RestartPane(_) | WorktreeManagerMode::RestartPaneWithAgent(_, _)
        ) {
            self.active_view = ActiveView::Workspace;
        }
        let probed_directory = self
            .pane_terminal_directory(pane_id)
            .filter(|directory| reported_path_is_concrete(directory));
        let wsl_distribution = self.settings.wsl_distribution.clone();
        self.worktree_manager_generation = self.worktree_manager_generation.wrapping_add(1);
        let generation = self.worktree_manager_generation;
        self.worktree_manager = Some(WorktreeManagerState::loading(mode, generation));
        perform_blocking(
            move || discover_worktree_manager(mode, probed_directory, &wsl_distribution),
            move |result| {
                Message::WorktreeManagerLoaded(generation, result.and_then(|value| value))
            },
        )
    }

    /// The title of a pane currently working inside `directory`, if any.
    pub(crate) fn pane_using_directory(&self, directory: &std::path::Path) -> Option<String> {
        for workspace in &self.session.workspaces {
            for tab in &workspace.tabs {
                for pane_id in tab.panes.keys().copied() {
                    if self
                        .pane_terminal_directory(pane_id)
                        .is_some_and(|cwd| cwd.starts_with(directory))
                    {
                        return Some(self.pane_title(workspace, pane_id).to_owned());
                    }
                }
            }
        }
        None
    }

    pub(crate) fn delete_worktree_entry(&mut self, index: usize) -> Vec<Effect> {
        let Some(manager) = self.worktree_manager.as_mut() else {
            return Vec::new();
        };
        if manager.busy || !matches!(manager.mode, WorktreeManagerMode::Manage) {
            return Vec::new();
        }
        let Some(repo_root) = manager.repo_root.clone() else {
            return Vec::new();
        };
        let Some(entry) = manager.entries.get(index).cloned() else {
            return Vec::new();
        };
        let name = worktree_display_name(&entry.path);
        if let Some(blocker) = entry.deletion_blocker {
            manager.error = Some(format!("{name} is the {blocker} and cannot be deleted"));
            return Vec::new();
        }
        if entry.used_by.is_some() {
            manager.error = Some(format!("{name} is in use by a pane and cannot be deleted"));
            return Vec::new();
        }
        manager.busy = true;
        manager.error = None;
        manager.selected = index;
        self.status = format!("Removing worktree {name}…");
        self.remove_worktrees_task(repo_root, vec![entry.path])
    }

    pub(crate) fn delete_unused_worktrees(&mut self) -> Vec<Effect> {
        let Some(manager) = self.worktree_manager.as_mut() else {
            return Vec::new();
        };
        if manager.busy || !matches!(manager.mode, WorktreeManagerMode::Manage) {
            return Vec::new();
        }
        let Some(repo_root) = manager.repo_root.clone() else {
            return Vec::new();
        };
        let paths = unused_worktree_paths(&manager.entries);
        if paths.is_empty() {
            return Vec::new();
        }
        manager.busy = true;
        manager.error = None;
        if let Some(first) = manager
            .entries
            .iter()
            .position(|entry| entry.path == paths[0])
        {
            manager.selected = first;
        }
        self.status = format!("Removing {} unused worktrees…", paths.len());
        self.remove_worktrees_task(repo_root, paths)
    }

    pub(crate) fn remove_worktrees_task(
        &self,
        repo_root: std::path::PathBuf,
        paths: Vec<std::path::PathBuf>,
    ) -> Vec<Effect> {
        let wsl_distribution = self.settings.wsl_distribution.clone();
        perform_blocking(
            move || remove_git_worktrees(&repo_root, paths, &wsl_distribution),
            move |result| match result {
                Ok((removed, result)) => Message::WorktreeManagerDeleted(removed, result),
                Err(error) => Message::WorktreeManagerDeleted(Vec::new(), Err(error)),
            },
        )
    }

    pub(crate) fn confirm_worktree_restart(&mut self) {
        let target = self.worktree_manager.as_ref().and_then(|manager| {
            let (pane_id, agent) = match manager.mode {
                WorktreeManagerMode::RestartPane(pane_id) => (pane_id, None),
                WorktreeManagerMode::RestartPaneWithAgent(pane_id, agent) => (pane_id, Some(agent)),
                WorktreeManagerMode::Manage => return None,
            };
            let index = manager.restart_target?;
            manager
                .entries
                .get(index)
                .cloned()
                .map(|entry| (pane_id, agent, entry))
        });
        let Some((pane_id, agent, entry)) = target else {
            return;
        };
        let name = entry.branch.clone().unwrap_or_else(|| {
            entry.path.file_name().map_or_else(
                || entry.path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
        });
        match self
            .restart_pane_in_directory(pane_id, entry.path.clone())
            .and_then(|()| agent.map_or(Ok(()), |agent| self.start_agent_in_pane(agent, pane_id)))
        {
            Ok(()) => {
                self.worktree_manager = None;
                self.status = agent.map_or_else(
                    || format!("Restarted pane in {name}"),
                    |agent| {
                        format!(
                            "Restarted pane in {name} and launched {}",
                            agent_display_name(&agent.to_string())
                        )
                    },
                );
            }
            Err(error) => {
                if let Some(manager) = self.worktree_manager.as_mut() {
                    manager.restart_target = None;
                    manager.error = Some(error);
                }
            }
        }
    }

    pub(crate) fn show_toast(&mut self, message: &str) {
        self.toast = Some((message.into(), std::time::Instant::now()));
    }

    pub(crate) fn copy_terminal_selection(&mut self, text: String) -> Vec<Effect> {
        self.show_toast("Copied to clipboard");
        vec![Effect::ClipboardWrite(text)]
    }

    /// `weight` snapped to a face the configured interface family installs.
    ///
    /// Emphasis levels in the rail are derived, not configured, so nothing
    /// validates them against the chosen family the way the settings picker
    /// validates the base weight. A family without the requested face makes the
    /// shaper substitute a different family for that run, changing the typeface
    /// and the shape of glyphs like `…` between adjacent rows.
    pub(crate) fn ui_weight(&self, weight: FontWeight) -> font::Weight {
        self.installed_fonts
            .nearest_ui_weight(&self.settings.ui_font, weight)
            .iced()
    }

    /// `weight` snapped against the family `Font::DEFAULT` resolves to, for
    /// chrome that deliberately stays on the system sans.
    pub(crate) fn default_family_weight(&self, weight: FontWeight) -> font::Weight {
        self.installed_fonts
            .nearest_ui_weight(&UiFont::SystemSans, weight)
            .iced()
    }

    /// Bottom-center feedback. Prefix and rail-navigation hints are modal,
    /// not transient confirmations, so the ordinary two-second toast timer
    /// never controls their lifetime.
    ///
    /// The second element says whether the pill announces a live keyboard mode.
    /// A mode the user is standing in has to be findable at a glance; a toast
    /// that disappears on its own should not compete with the terminal, so the
    /// two are drawn differently.
    pub(crate) fn feedback_message(&self) -> Option<(&str, bool)> {
        if self.prefix_armed {
            Some(("Prefix — w workspaces · f fleet · Esc cancel", true))
        } else if self.rail_nav.is_some() {
            Some(("Navigate — ↑↓ move · Enter select · Esc exit", true))
        } else {
            self.toast
                .as_ref()
                .map(|(message, _)| (message.as_str(), false))
        }
    }

    /// Every rail entry the prefix navigation can land on, in visual
    /// order: workspaces first, then the fleet's tabs and panes.
    pub(crate) fn rail_targets(&self) -> Vec<RailTarget> {
        let mut targets: Vec<RailTarget> = self
            .session
            .workspaces
            .iter()
            .map(|workspace| RailTarget::Workspace(workspace.id))
            .collect();

        if self.sidebar_is_compact() {
            targets.extend(
                self.fleet_entries()
                    .into_iter()
                    .map(|(workspace_id, pane_id)| RailTarget::FleetPane(workspace_id, pane_id)),
            );
            return targets;
        }

        let show_workspace_groups = self.settings.fleet_scope == FleetScope::AllWorkspaces;
        match self.effective_fleet_view() {
            FleetView::Agents => {
                let entries = self.fleet_entries();
                for workspace in self.fleet_workspaces() {
                    let mut workspace_entries = entries
                        .iter()
                        .copied()
                        .filter(|(workspace_id, _)| *workspace_id == workspace.id)
                        .peekable();
                    if workspace_entries.peek().is_none() {
                        continue;
                    }
                    if show_workspace_groups {
                        targets.push(RailTarget::FleetWorkspace(workspace.id));
                    }
                    targets.extend(workspace_entries.map(|(workspace_id, pane_id)| {
                        RailTarget::FleetPane(workspace_id, pane_id)
                    }));
                }
            }
            FleetView::Repos => {
                for workspace in self.fleet_workspaces() {
                    let groups = self.fleet_repository_groups_for(workspace);
                    if show_workspace_groups && !groups.is_empty() {
                        targets.push(RailTarget::FleetWorkspace(workspace.id));
                    }
                    for group in groups {
                        if let Some((workspace_id, first_pane)) = group.entries.first().copied() {
                            targets.push(RailTarget::FleetGroup(workspace_id, first_pane));
                        }
                        targets.extend(group.entries.into_iter().map(|(workspace_id, pane_id)| {
                            RailTarget::FleetPane(workspace_id, pane_id)
                        }));
                    }
                }
            }
            FleetView::Tabs => {
                for workspace in self.fleet_workspaces() {
                    if show_workspace_groups {
                        targets.push(RailTarget::FleetWorkspace(workspace.id));
                    }
                    for tab in &workspace.tabs {
                        // Single-tab workspaces do not render a tab band in the
                        // fleet, so keyboard navigation must not land on one.
                        if workspace.tabs.len() > 1 {
                            targets.push(RailTarget::FleetTab(workspace.id, tab.id));
                        }
                        for pane_id in pane_ids_in_layout(&tab.root) {
                            targets.push(RailTarget::FleetPane(workspace.id, pane_id));
                        }
                    }
                }
            }
        }
        targets
    }

    /// Pushes the current layout to this session's daemon whenever it
    /// changes, so the session is resumable exactly as last seen. Runs on
    /// the blink tick; the hash gate keeps quiet ticks free.
    pub(crate) fn sync_session_layout(&mut self) {
        let Some((_, client)) = session_host() else {
            return;
        };
        let session = session_with_agent_identities(&self.session, &self.agent_statuses);
        let Ok(layout) = serde_json::to_string(&session) else {
            return;
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        layout.hash(&mut hasher);
        let hash = hasher.finish();
        if hash == self.last_layout_hash {
            return;
        }
        self.last_layout_hash = hash;
        let _ = client.send(&muxtrix_sessions::Request::Layout { data: layout });
        if let Some(workspace) = self
            .session
            .workspaces
            .iter()
            .find(|workspace| workspace.id == self.session.active_workspace_id)
        {
            let _ = client.send(&muxtrix_sessions::Request::Rename {
                name: workspace.name.clone(),
            });
        }
    }

    pub(crate) fn open_session_picker(&mut self, startup: bool) {
        self.active_view = ActiveView::Workspace;
        let own = session_host().map(|(id, _)| id);
        let candidates: Vec<muxtrix_sessions::SessionRecord> = if startup {
            muxtrix_sessions::resumable_sessions(own)
        } else {
            // The palette view also lists dead records so they can be
            // cleaned up, but never sessions another instance holds.
            muxtrix_sessions::list_sessions()
                .into_iter()
                .filter(|record| Some(record.id) != own)
                .filter(|record| !record.attached)
                .collect()
        };
        self.open_session_picker_from_records(candidates, startup);
    }

    pub(crate) fn open_session_picker_from_records(
        &mut self,
        candidates: Vec<muxtrix_sessions::SessionRecord>,
        startup: bool,
    ) {
        self.active_view = ActiveView::Workspace;
        let entries: Vec<SessionPickerEntry> = candidates
            .into_iter()
            .map(|record| {
                // Startup candidates were already probed by
                // `resumable_sessions` on the background host worker.
                let alive = startup || muxtrix_sessions::record_is_alive(&record);
                let pane_count = record
                    .layout
                    .as_deref()
                    .and_then(|layout| serde_json::from_str::<SessionState>(layout).ok())
                    .map_or(0, |state| {
                        state
                            .workspaces
                            .iter()
                            .flat_map(|workspace| &workspace.tabs)
                            .map(|tab| tab.panes.len())
                            .sum()
                    });
                SessionPickerEntry {
                    record,
                    alive,
                    pane_count,
                }
            })
            .collect();
        self.session_picker = Some(SessionPickerState {
            entries,
            selected: 0,
            error: None,
            startup,
        });
    }

    /// Kills one listed session: live daemons get a shutdown, dead records
    /// are registry residue and just get removed.
    pub(crate) fn kill_picked_session(&mut self, index: usize) {
        let Some(picker) = self.session_picker.as_mut() else {
            return;
        };
        let Some(entry) = picker.entries.get(index) else {
            return;
        };
        if entry.alive {
            match muxtrix_sessions::SessionClient::connect_endpoint(&entry.record.endpoint) {
                Ok((client, _, _)) => {
                    let _ = client.send(&muxtrix_sessions::Request::Shutdown);
                }
                Err(error) => {
                    picker.error = Some(format!("could not reach the session: {error}"));
                    return;
                }
            }
        }
        muxtrix_sessions::remove_session_record(entry.record.id);
        picker.entries.remove(index);
        if picker.selected >= picker.entries.len() {
            picker.selected = picker.entries.len().saturating_sub(1);
        }
        picker.error = None;
    }

    /// Switches this window onto a background session: its layout replaces
    /// the current one and every pane reattaches to the daemon-owned PTY
    /// (backlog replays into a fresh VT). The session this window was on
    /// stays alive in the background — that is the multiplexer contract.
    pub(crate) fn resume_session(&mut self, index: usize) {
        let Some(picker) = self.session_picker.as_mut() else {
            return;
        };
        let Some(entry) = picker.entries.get(index) else {
            return;
        };
        if !entry.alive {
            picker.error = Some("that session's daemon is no longer running".into());
            return;
        }
        let record = entry.record.clone();
        let (client, _, layout) =
            match muxtrix_sessions::SessionClient::connect_endpoint(&record.endpoint) {
                Ok(connection) => connection,
                Err(error) => {
                    picker.error = Some(format!("could not attach: {error}"));
                    return;
                }
            };
        let layout = layout.or(record.layout.clone());
        let Some(state) = layout
            .as_deref()
            .and_then(|layout| serde_json::from_str::<SessionState>(layout).ok())
        else {
            picker.error = Some("that session never reported a layout".into());
            return;
        };
        let control_panes = Self::control_pane_ids(&state);
        if let Err(error) = self.bind_control_to_session(record.id, &control_panes) {
            if let Some(picker) = self.session_picker.as_mut() {
                picker.error = Some(format!("could not route local control: {error}"));
            }
            return;
        }
        let client = Arc::new(client);
        // Register every pane, then re-attach so the daemon replays each
        // backlog into the channels that now exist to receive it.
        let mut receivers: std::collections::HashMap<PaneId, std::sync::mpsc::Receiver<Vec<u8>>> =
            std::collections::HashMap::new();
        for workspace in &state.workspaces {
            for tab in &workspace.tabs {
                for pane_id in tab.panes.keys() {
                    receivers.insert(*pane_id, client.register_pane(pane_id.as_uuid()));
                }
            }
        }
        let _ = client.send(&muxtrix_sessions::Request::Attach);
        if let Ok(mut host) = SESSION_HOST.lock() {
            *host = Some(SessionHost {
                id: record.id,
                client: Arc::clone(&client),
            });
        }
        let theme = self.settings.terminal_theme.preset().terminal_theme();
        let mut terminals = BTreeMap::new();
        for workspace in &state.workspaces {
            for tab in &workspace.tabs {
                for (pane_id, pane) in &tab.panes {
                    let title = pane
                        .custom_name
                        .clone()
                        .or_else(|| pane.active_surface().map(|surface| surface.title.clone()))
                        .unwrap_or_else(|| "shell".into());
                    let Some(receiver) = receivers.remove(pane_id) else {
                        continue;
                    };
                    let runtime = TerminalRuntime::attach(
                        *pane_id,
                        &title,
                        self.settings.terminal_scrollback_lines,
                        theme,
                        Arc::clone(&self.event_notifier),
                        Arc::clone(&client),
                        receiver,
                    );
                    terminals.insert(*pane_id, runtime);
                }
            }
        }
        let restored_agent_statuses = agent_statuses_from_session(&state);
        self.session = state;
        self.terminals = terminals;
        self.queued_terminal_restarts.clear();
        self.session_picker = None;
        self.maximized_pane = None;
        self.pane_menu = None;
        self.hovered_terminal = None;
        self.agent_statuses = restored_agent_statuses;
        self.agent_running_frame_revisions.clear();
        self.pi_active_lifecycles.clear();
        self.detected_agents.clear();
        self.agents_view_panes.clear();
        self.pane_layouts.clear();
        self.base_pane_layouts.clear();
        self.pane_resize_history.clear();
        self.split_sizes.clear();
        self.split_drag = None;
        self.last_layout_hash = 0;
        self.status = format!("Resumed session {}", record.name);
    }

    pub(crate) fn open_created_worktree(
        &mut self,
        target: WorktreePromptTarget,
        directory: std::path::PathBuf,
    ) -> Result<(), String> {
        match target {
            WorktreePromptTarget::Open(kind) => self.open_worktree(kind, directory).map(|_| ()),
            WorktreePromptTarget::OpenWithAgent(kind, agent) => {
                let pane_id = self.open_worktree(kind, directory)?;
                self.start_agent_in_pane(agent, pane_id)
            }
            WorktreePromptTarget::RestartPane(pane_id) => {
                self.restart_pane_in_directory(pane_id, directory)
            }
            WorktreePromptTarget::RestartPaneWithAgent(pane_id, agent) => {
                self.restart_pane_in_directory(pane_id, directory)?;
                self.start_agent_in_pane(agent, pane_id)
            }
        }
    }

    /// Opens a terminal in `directory` as a sibling pane or a new tab.
    pub(crate) fn open_worktree(
        &mut self,
        kind: commands::WorktreeKind,
        directory: std::path::PathBuf,
    ) -> Result<PaneId, String> {
        if self.maximized_pane.is_some() && matches!(kind, commands::WorktreeKind::Pane(_)) {
            return Err("Restore panes before opening a worktree beside them".into());
        }
        let mut profile = self.default_terminal_profile()?;
        profile.working_directory = Some(directory.clone());
        let title = directory.file_name().map_or_else(
            || "worktree".into(),
            |name| name.to_string_lossy().into_owned(),
        );
        let surface = Surface::terminal(
            &title,
            TerminalSurface {
                profile_id: profile.id,
                working_directory: Some(directory),
            },
        );
        if matches!(kind, commands::WorktreeKind::Pane(_)) {
            let tab_id = self
                .active_workspace()?
                .active_tab()
                .ok_or_else(|| "active tab is missing".to_owned())?
                .id;
            self.clear_manual_layout_history(tab_id);
        }
        let pane_id = match kind {
            commands::WorktreeKind::Pane(axis) => self
                .active_workspace_mut()?
                .split_focused(axis, SplitRatio::EQUAL, surface)
                .map_err(|error| error.to_string())?,
            commands::WorktreeKind::Tab => {
                let tab = WorkspaceTab::new(&title, surface);
                let pane_id = tab.focused_pane_id;
                self.active_workspace_mut()?
                    .add_tab(tab)
                    .map_err(|error| error.to_string())?;
                self.maximized_pane = None;
                self.pane_menu = None;
                pane_id
            }
        };
        self.request_terminal_launch(profile, pane_id, title)?;
        Ok(pane_id)
    }

    /// Moves pane focus in `direction`, spilling across tabs at the layout
    /// edge: right of the last pane wraps into the next tab's first pane, left
    /// of the first pane into the previous tab's last pane.
    pub(crate) fn focus_neighbor_pane(&mut self, direction: NavDirection) -> Result<(), String> {
        let workspace = self.active_workspace()?;
        let tab = workspace
            .active_tab()
            .ok_or_else(|| "active tab is missing".to_owned())?;
        if let Some(next) = stacked_neighbor(&tab.root, tab.focused_pane_id, direction) {
            return self.focus_pane(next);
        }
        let rects = pane_rects(&tab.root);
        if let Some(next) = neighbor_pane(&rects, tab.focused_pane_id, direction) {
            return self.focus_pane(next);
        }
        let tab_count = workspace.tabs.len();
        let tab_index = workspace
            .tabs
            .iter()
            .position(|candidate| candidate.id == tab.id)
            .ok_or_else(|| "active tab is missing".to_owned())?;
        let target = match direction {
            NavDirection::Right => {
                let next_tab = &workspace.tabs[(tab_index + 1) % tab_count];
                pane_ids_in_layout(&next_tab.root).first().copied()
            }
            NavDirection::Left => {
                let previous_tab = &workspace.tabs[(tab_index + tab_count - 1) % tab_count];
                pane_ids_in_layout(&previous_tab.root).last().copied()
            }
            NavDirection::Up | NavDirection::Down => None,
        };
        match target {
            Some(pane_id) if pane_id != tab.focused_pane_id => self.focus_pane(pane_id),
            _ => Ok(()),
        }
    }

    pub(crate) fn new_tab(&mut self) -> Result<(), String> {
        let profile = self.regular_terminal_profile()?;
        let tab_name = format!("Tab {}", self.active_workspace()?.tabs.len() + 1);
        let title = "shell 1";
        let tab = WorkspaceTab::new(
            &tab_name,
            Surface::terminal(
                title,
                TerminalSurface {
                    profile_id: profile.id,
                    working_directory: profile.working_directory.clone(),
                },
            ),
        );
        let pane_id = tab.focused_pane_id;
        self.active_workspace_mut()?
            .add_tab(tab)
            .map_err(|error| error.to_string())?;
        self.request_regular_terminal_launch(profile, pane_id, title.into())?;
        self.maximized_pane = None;
        self.pane_menu = None;
        Ok(())
    }

    pub(crate) fn switch_tab(&mut self, tab_id: TabId) -> Result<(), String> {
        let changed = self
            .active_workspace()?
            .active_tab()
            .is_none_or(|tab| tab.id != tab_id);
        self.active_workspace_mut()?
            .switch_tab(tab_id)
            .map_err(|error| error.to_string())?;
        self.maximized_pane = None;
        self.pane_menu = None;
        self.hovered_terminal = None;
        if changed {
            self.queue_github_pane_refresh();
        }
        Ok(())
    }

    pub(crate) fn close_workspace(&mut self) -> Result<(), String> {
        self.close_workspace_by_id(self.session.active_workspace_id)
    }

    pub(crate) fn close_workspace_by_id(
        &mut self,
        workspace_id: WorkspaceId,
    ) -> Result<(), String> {
        let (pane_ids, tab_ids) = self
            .session
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .map(|workspace| {
                (
                    workspace.all_pane_ids(),
                    workspace.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
                )
            })
            .ok_or_else(|| "workspace is missing".to_owned())?;
        self.session
            .close_workspace(workspace_id)
            .map_err(|error| error.to_string())?;
        for pane_id in pane_ids {
            self.cleanup_pane_state(pane_id);
        }
        for tab_id in tab_ids {
            self.pane_layouts.remove(&tab_id);
            self.base_pane_layouts.remove(&tab_id);
            self.pane_resize_history.remove(&tab_id);
        }
        self.split_sizes
            .retain(|key, _| key.workspace_id != workspace_id);
        self.terminal_scroll_drag = None;
        if self
            .split_drag
            .as_ref()
            .is_some_and(|drag| drag.key.workspace_id == workspace_id)
        {
            self.split_drag = None;
        }
        self.workspace_name_draft = self.active_workspace()?.name.clone();
        self.rename_prompt = None;
        self.maximized_pane = None;
        self.pane_menu = None;
        self.hovered_terminal = None;
        Ok(())
    }

    pub(crate) fn close_focused(&mut self) -> Result<(), String> {
        let pane_id = self
            .active_workspace()?
            .active_tab()
            .ok_or_else(|| "active tab is missing".to_owned())?
            .focused_pane_id;
        self.close_pane(pane_id)
    }

    pub(crate) fn close_pane(&mut self, pane_id: PaneId) -> Result<(), String> {
        let (workspace_id, tab_id, pane_count, tab_count) = self
            .session
            .workspaces
            .iter()
            .find_map(|workspace| {
                workspace
                    .tab_containing_pane(pane_id)
                    .map(|tab| (workspace.id, tab.id, tab.panes.len(), workspace.tabs.len()))
            })
            .ok_or_else(|| format!("pane {pane_id:?} is missing"))?;
        if pane_count == 1 {
            if tab_count == 1 {
                self.close_workspace_prompt = Some(workspace_id);
                return Ok(());
            }
            return self.close_tab(workspace_id, tab_id);
        }
        self.clear_manual_layout_history(tab_id);
        self.session
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
            .ok_or_else(|| "workspace is missing".to_owned())?
            .close_pane(pane_id)
            .map_err(|error| error.to_string())?;
        self.cleanup_pane_state(pane_id);
        Ok(())
    }

    pub(crate) fn close_tab(
        &mut self,
        workspace_id: WorkspaceId,
        tab_id: TabId,
    ) -> Result<(), String> {
        let workspace = self
            .session
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
            .ok_or_else(|| "workspace is missing".to_owned())?;
        if workspace.tabs.len() == 1 {
            self.close_workspace_prompt = Some(workspace_id);
            return Ok(());
        }
        let removed = workspace
            .close_tab(tab_id)
            .map_err(|error| error.to_string())?;
        self.pane_layouts.remove(&tab_id);
        self.base_pane_layouts.remove(&tab_id);
        self.pane_resize_history.remove(&tab_id);
        self.split_sizes.retain(|key, _| key.tab_id != tab_id);
        for pane_id in removed.root.pane_ids() {
            self.cleanup_pane_state(pane_id);
        }
        self.maximized_pane = None;
        self.pane_menu = None;
        Ok(())
    }

    pub(crate) fn cleanup_pane_state(&mut self, pane_id: PaneId) {
        // Closing a pane is a decision to end its process — unlike app
        // exit, where daemon-owned panes must keep running.
        if let Some(runtime) = self.terminals.remove(&pane_id)
            && let Some(session) = &runtime.session
        {
            session.terminate();
        }
        forget_host_pane(pane_id);
        self.clear_pane_activity_state(pane_id);
        self.queued_terminal_restarts.remove(&pane_id);
        if self.maximized_pane == Some(pane_id) {
            self.maximized_pane = None;
        }
        if self.pane_menu == Some(pane_id) {
            self.pane_menu = None;
        }
    }

    pub(crate) fn clear_pane_activity_state(&mut self, pane_id: PaneId) {
        self.notifications
            .retain(|notification| notification.pane_id != pane_id);
        self.agent_statuses.remove(&pane_id);
        self.agent_running_frame_revisions.remove(&pane_id);
        self.pi_active_lifecycles.remove(&pane_id);
        self.detected_agents.remove(&pane_id);
        self.agents_view_panes.remove(&pane_id);
        self.terminal_pointer_positions.remove(&pane_id);
        self.terminal_scrollbar_positions.remove(&pane_id);
        self.terminal_command_buffers.remove(&pane_id);
        self.pending_terminal_input.remove(&pane_id);
        if self.hovered_terminal == Some(pane_id) {
            self.hovered_terminal = None;
        }
        if self
            .terminal_scroll_drag
            .is_some_and(|drag| drag.pane_id == pane_id)
        {
            self.terminal_scroll_drag = None;
        }
    }

    pub(crate) fn finish_tab_drag(&mut self) -> Result<(), String> {
        let Some(drag) = self.tab_drag.take() else {
            return Ok(());
        };
        self.session
            .move_tab(drag.tab_id, drag.target_workspace_id, drag.target_index)
            .map_err(|error| error.to_string())?;
        self.active_view = ActiveView::Workspace;
        self.maximized_pane = None;
        self.pane_menu = None;
        Ok(())
    }

    pub(crate) fn restart_pane(&mut self, pane_id: PaneId) -> Result<(), String> {
        let surface_directory = self
            .session
            .workspaces
            .iter()
            .find_map(|workspace| workspace.pane(pane_id))
            .and_then(|pane| pane.active_surface())
            .and_then(|surface| match &surface.kind {
                muxtrix_domain::SurfaceKind::Terminal(terminal) => {
                    terminal.working_directory.clone()
                }
                _ => None,
            });
        self.restart_pane_with_directory(pane_id, surface_directory)
    }

    pub(crate) fn restart_pane_in_directory(
        &mut self,
        pane_id: PaneId,
        directory: std::path::PathBuf,
    ) -> Result<(), String> {
        self.restart_pane_with_directory(pane_id, Some(directory))
    }

    pub(crate) fn restart_pane_with_directory(
        &mut self,
        pane_id: PaneId,
        directory: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        let profile_id = self
            .session
            .workspaces
            .iter()
            .find_map(|workspace| workspace.pane(pane_id))
            .and_then(Pane::active_surface)
            .and_then(|surface| match &surface.kind {
                muxtrix_domain::SurfaceKind::Terminal(terminal) => Some(terminal.profile_id),
                _ => None,
            })
            .ok_or_else(|| format!("pane {pane_id:?} has no terminal profile"))?;
        let mut profile = self
            .session
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
            .ok_or_else(|| "terminal launch profile is missing".to_owned())?;
        profile.working_directory = directory.clone().or(profile.working_directory);
        let fallback_title = self.terminals.get(&pane_id).map_or_else(
            || "terminal".to_owned(),
            |runtime| runtime.fallback_title.clone(),
        );
        self.clear_pane_activity_state(pane_id);

        let pane = self
            .session
            .workspaces
            .iter_mut()
            .find_map(|workspace| workspace.pane_mut(pane_id))
            .ok_or_else(|| format!("pane {pane_id:?} is missing"))?;
        pane.attention.unread_count = 0;
        pane.attention.message = None;
        let surface = pane
            .surfaces
            .iter_mut()
            .find(|surface| surface.id == pane.active_surface_id)
            .ok_or_else(|| format!("pane {pane_id:?} has no active surface"))?;
        let muxtrix_domain::SurfaceKind::Terminal(terminal) = &mut surface.kind else {
            return Err(format!("pane {pane_id:?} has no terminal profile"));
        };
        terminal.working_directory = directory;

        let launch_in_flight = self.terminals.get(&pane_id).is_some_and(|runtime| {
            matches!(runtime.launch_state, TerminalLaunchState::Starting { .. })
        });
        if launch_in_flight {
            // Starting another daemon session now would reuse the same pane
            // ID while the first worker still owns it. Let that worker finish,
            // then replace the session through the ordinary previous-session
            // handoff. Repeated restart requests collapse to the latest
            // working directory already stored on the terminal surface.
            self.queued_terminal_restarts.insert(pane_id);
            self.status = "Waiting for the current terminal launch before restarting…".into();
            return Ok(());
        }

        // The previous session is moved into the background launch request.
        // The worker terminates and drops it before spawning the replacement,
        // preserving daemon Kill-before-Spawn ordering without joining a PTY
        // owner thread on the UI path.
        self.request_terminal_launch(profile, pane_id, fallback_title)
    }

    pub(crate) fn handle_keyboard(&mut self, event: KeyEvent) -> Vec<Effect> {
        self.keyboard_modifiers = event.modifiers();
        let KeyEvent::Pressed(KeyInput {
            key,
            modified_key,
            modifiers,
            text,
            ..
        }) = event
        else {
            return Vec::new();
        };

        if self.pane_menu.is_some() && matches!(modified_key.as_ref(), Key::Named(Named::Escape)) {
            self.pane_menu = None;
            return Vec::new();
        }

        // Zellij-style prefix: Ctrl+G unlocks an action layer that persists
        // until the user picks a recognized follow-up or presses Escape.
        if modifiers.control()
            && !modifiers.shift()
            && !modifiers.alt()
            && !modifiers.logo()
            && character_key_is(modified_key.as_ref(), "g")
            && self.active_view == ActiveView::Workspace
            && !self.palette.visible
        {
            self.prefix_armed = true;
            self.rail_nav = None;
            return Vec::new();
        }
        if self.prefix_armed {
            if character_key_is(modified_key.as_ref(), "w") {
                self.prefix_armed = false;
                let targets = self.rail_targets();
                self.rail_nav = targets
                    .iter()
                    .find(|target| matches!(target, RailTarget::Workspace(_)))
                    .copied();
            } else if character_key_is(modified_key.as_ref(), "f") {
                self.prefix_armed = false;
                let targets = self.rail_targets();
                self.rail_nav = targets
                    .iter()
                    .find(|target| !matches!(target, RailTarget::Workspace(_)))
                    .copied();
            } else if matches!(modified_key.as_ref(), Key::Named(Named::Escape)) {
                self.prefix_armed = false;
            }
            // Unrecognized keys leave the prefix armed and are consumed so
            // partial chords never leak into panes.
            return Vec::new();
        }
        if let Some(current) = self.rail_nav {
            match modified_key.as_ref() {
                Key::Named(Named::ArrowUp) | Key::Named(Named::ArrowDown) => {
                    let targets = self.rail_targets();
                    if let Some(index) = targets.iter().position(|target| *target == current) {
                        let next = if matches!(modified_key.as_ref(), Key::Named(Named::ArrowDown))
                        {
                            (index + 1).min(targets.len().saturating_sub(1))
                        } else {
                            index.saturating_sub(1)
                        };
                        self.rail_nav = targets.get(next).copied();
                    } else {
                        self.rail_nav = targets.first().copied();
                    }
                }
                Key::Named(Named::Enter) => {
                    self.rail_nav = None;
                    match current {
                        RailTarget::Workspace(workspace_id) => {
                            if let Err(error) = self.switch_workspace(workspace_id) {
                                self.status = error;
                            }
                        }
                        RailTarget::FleetTab(workspace_id, tab_id) => {
                            let first_pane = self
                                .session
                                .workspaces
                                .iter()
                                .find(|workspace| workspace.id == workspace_id)
                                .and_then(|workspace| {
                                    workspace.tabs.iter().find(|tab| tab.id == tab_id)
                                })
                                .map(|tab| tab.focused_pane_id);
                            if let Some(pane_id) = first_pane
                                && let Err(error) = self
                                    .switch_workspace(workspace_id)
                                    .and_then(|()| self.switch_tab(tab_id))
                                    .and_then(|()| self.focus_pane(pane_id))
                            {
                                self.status = error;
                            }
                        }
                        RailTarget::FleetWorkspace(workspace_id) => {
                            if let Err(error) = self.switch_workspace(workspace_id) {
                                self.status = error;
                            }
                        }
                        RailTarget::FleetGroup(workspace_id, pane_id)
                        | RailTarget::FleetPane(workspace_id, pane_id) => {
                            if let Err(error) = self
                                .switch_workspace(workspace_id)
                                .and_then(|()| self.focus_pane(pane_id))
                            {
                                self.status = error;
                            }
                        }
                    }
                }
                Key::Named(Named::Escape) => self.rail_nav = None,
                // Rail navigation is a mode. Unrelated keys are consumed but
                // cannot silently dismiss it or leak into the terminal.
                _ => {}
            }
            return Vec::new();
        }

        if modifiers == Modifiers::COMMAND | Modifiers::SHIFT
            && let Some(number) = number_shortcut(key.as_ref())
        {
            if let Some(workspace_id) = self
                .session
                .workspaces
                .get(number - 1)
                .map(|workspace| workspace.id)
                && let Err(error) = self.switch_workspace(workspace_id)
            {
                self.status = error;
            }
            return Vec::new();
        }
        if modifiers.command() && character_key_is(modified_key.as_ref(), "p") {
            return self.toggle_command_palette();
        }
        if modifiers.command() && character_key_is(modified_key.as_ref(), ",") {
            return self.open_settings();
        }
        if modifiers == Modifiers::COMMAND
            && let Some(number) = number_shortcut(key.as_ref())
        {
            if let Some((workspace_id, pane_id)) = self.fleet_entries().get(number - 1).copied() {
                self.active_view = ActiveView::Workspace;
                let _ = self.switch_workspace(workspace_id);
                let _ = self.focus_pane(pane_id);
            }
            return Vec::new();
        }
        if let Some(action) = clipboard_shortcut(modified_key.as_ref(), modifiers)
            && self.active_view == ActiveView::Workspace
            && !self.palette.visible
            && !self.workspace_create_visible
            && self.close_workspace_prompt.is_none()
            && self.rename_prompt.is_none()
            && !self.default_agent_prompt
            && self.worktree_prompt.is_none()
            && let Ok(workspace) = self.active_workspace()
            && let Some(tab) = workspace.active_tab()
        {
            let pane_id = tab.focused_pane_id;
            self.pane_menu = None;
            // Both shortcuts are consumed even when they have nothing to do,
            // matching Ghostty: an empty copy never reaches the shell as ^C.
            return match action {
                ClipboardAction::Copy => self
                    .selected_terminal_text(pane_id)
                    .map_or_else(Vec::new, |text| self.copy_terminal_selection(text)),
                ClipboardAction::Paste => vec![Effect::ClipboardRead(Arc::new(move |contents| {
                    Message::ClipboardPasted(pane_id, contents)
                }))],
            };
        }
        // Workspace chords advertised in the pane menu and tab bar. Consumed
        // under the same guards as the clipboard chords so they never leak
        // into the terminal as control bytes.
        if modifiers.control()
            && modifiers.shift()
            && !modifiers.alt()
            && !modifiers.logo()
            && self.active_view == ActiveView::Workspace
            && !self.palette.visible
            && !self.workspace_create_visible
            && self.close_workspace_prompt.is_none()
            && self.rename_prompt.is_none()
            && !self.default_agent_prompt
            && self.worktree_prompt.is_none()
        {
            if character_key_is(modified_key.as_ref(), "e") {
                self.status = match self.split_terminal(SplitAxis::Horizontal) {
                    Ok(()) => "Opened a new terminal pane".into(),
                    Err(error) => error,
                };
                return Vec::new();
            }
            if character_key_is(modified_key.as_ref(), "o") {
                self.status = match self.split_terminal(SplitAxis::Vertical) {
                    Ok(()) => "Opened a new terminal pane".into(),
                    Err(error) => error,
                };
                return Vec::new();
            }
            if character_key_is(modified_key.as_ref(), "m")
                && let Ok(workspace) = self.active_workspace()
                && let Some(tab) = workspace.active_tab()
            {
                let pane_id = tab.focused_pane_id;
                return self.update(Message::ToggleMaximize(pane_id));
            }
            if character_key_is(modified_key.as_ref(), "t") {
                self.status = match self.new_tab() {
                    Ok(()) => "Created a new tab".into(),
                    Err(error) => error,
                };
                return Vec::new();
            }
        }
        if self.palette.visible {
            match modified_key.as_ref() {
                Key::Named(Named::Escape) => self.close_command_palette(),
                Key::Named(Named::ArrowDown) => {
                    return self.move_palette_selection(PaletteMove::Next);
                }
                Key::Named(Named::ArrowUp) => {
                    return self.move_palette_selection(PaletteMove::Previous);
                }
                Key::Named(Named::Tab) => {
                    return self.move_palette_selection(if modifiers.shift() {
                        PaletteMove::Previous
                    } else {
                        PaletteMove::Next
                    });
                }
                Key::Named(Named::Enter) => {
                    let commands = commands::filtered(&self.palette.query);
                    if let Some(command) = commands
                        .get(self.palette.selected)
                        .filter(|command| self.command_enabled(command.action))
                    {
                        return self.run_command(command.action);
                    }
                }
                _ => {}
            }
            return Vec::new();
        }

        if let Some(task) = self.handle_github_panel_keyboard(modified_key.as_ref(), modifiers) {
            return task;
        }

        // Gallery must precede the generic non-workspace branch, which
        // would otherwise swallow Esc, discard the draft, and jump to the
        // workspace instead of back to Settings.
        if self.active_view == ActiveView::ThemeGallery {
            if matches!(modified_key.as_ref(), Key::Named(Named::Escape)) {
                self.active_view = ActiveView::Settings;
            }
            return Vec::new();
        }

        if self.active_view == ActiveView::GitHubDiff {
            if matches!(modified_key.as_ref(), Key::Named(Named::Escape)) {
                if let Some(diff) = self.github_diff.take() {
                    diff.cancellation.cancel();
                }
                self.active_view = ActiveView::Workspace;
            }
            return Vec::new();
        }

        // The Worktrees settings page is a keyboard-operable inventory, not a
        // static form: it owns Up/Down/Delete and advertises them in its
        // footer. Without this exemption the generic non-workspace branch
        // below consumes every key before the worktree handler is reached,
        // leaving the advertised navigation dead.
        let worktree_settings_owns_keys = self.worktree_manager.is_some()
            && self.active_view == ActiveView::Settings
            && self.settings_page == SettingsPage::Worktrees;
        if self.active_view != ActiveView::Workspace && !worktree_settings_owns_keys {
            if matches!(modified_key.as_ref(), Key::Named(Named::Escape)) {
                self.reset_settings_draft();
                self.active_view = ActiveView::Workspace;
            }
            return Vec::new();
        }

        if self.workspace_create_visible {
            if matches!(modified_key.as_ref(), Key::Named(Named::Escape)) {
                self.workspace_create_visible = false;
            }
            return Vec::new();
        }

        if let Some(workspace_id) = self.close_workspace_prompt {
            match modified_key.as_ref() {
                Key::Named(Named::Escape) => self.close_workspace_prompt = None,
                Key::Named(Named::Enter) => {
                    // Enter confirms when closing is possible; the dialog for
                    // the last workspace only offers dismissal.
                    self.close_workspace_prompt = None;
                    if self.session.workspaces.len() > 1
                        && let Err(error) = self.close_workspace_by_id(workspace_id)
                    {
                        self.status = error;
                    }
                }
                _ => {}
            }
            return Vec::new();
        }

        if self.rename_prompt.is_some() {
            if matches!(modified_key.as_ref(), Key::Named(Named::Escape)) {
                self.rename_prompt = None;
            }
            return Vec::new();
        }

        if self.default_agent_prompt {
            if matches!(modified_key.as_ref(), Key::Named(Named::Escape)) {
                self.default_agent_prompt = false;
                self.pending_default_agent_command = None;
            }
            return Vec::new();
        }

        if let Some(picker) = &mut self.session_picker {
            let entry_count = picker.entries.len();
            let selected = picker.selected;
            match modified_key.as_ref() {
                Key::Named(Named::Escape) => self.session_picker = None,
                Key::Named(Named::Enter) if entry_count > 0 => {
                    // A dead row cannot resume; Enter does the one thing it
                    // can — clean it up — instead of printing a refusal.
                    if self
                        .session_picker
                        .as_ref()
                        .and_then(|picker| picker.entries.get(selected))
                        .is_some_and(|entry| entry.alive)
                    {
                        self.resume_session(selected);
                    } else {
                        self.kill_picked_session(selected);
                    }
                }
                Key::Named(Named::Enter) => self.session_picker = None,
                Key::Named(Named::ArrowUp) if entry_count > 0 => {
                    picker.selected = selected.saturating_sub(1);
                }
                Key::Named(Named::ArrowDown) if entry_count > 0 => {
                    picker.selected = (selected + 1).min(entry_count - 1);
                }
                Key::Named(Named::Delete | Named::Backspace) if entry_count > 0 => {
                    self.kill_picked_session(selected);
                }
                _ => {}
            }
            return Vec::new();
        }

        let worktree_manager_is_active = self.worktree_manager.as_ref().is_some_and(|manager| {
            matches!(
                manager.mode,
                WorktreeManagerMode::RestartPane(_)
                    | WorktreeManagerMode::RestartPaneWithAgent(_, _)
            ) || (self.active_view == ActiveView::Settings
                && self.settings_page == SettingsPage::Worktrees)
        });
        if worktree_manager_is_active && let Some(manager) = &mut self.worktree_manager {
            let entry_count = manager.entries.len();
            if manager.restart_target.is_some() {
                match modified_key.as_ref() {
                    Key::Named(Named::Escape) => manager.restart_target = None,
                    Key::Named(Named::Enter) => self.confirm_worktree_restart(),
                    _ => {}
                }
                return Vec::new();
            }
            match modified_key.as_ref() {
                Key::Named(Named::Escape) => {
                    self.worktree_manager = None;
                    if self.active_view == ActiveView::Settings {
                        // Leaving settings by keyboard discards the draft
                        // wherever it happens, so returning from Worktrees
                        // cannot strand an edited Preferences draft.
                        self.reset_settings_draft();
                        self.active_view = ActiveView::Workspace;
                    }
                }
                Key::Named(Named::Enter)
                    if entry_count > 0
                        && matches!(
                            manager.mode,
                            WorktreeManagerMode::RestartPane(_)
                                | WorktreeManagerMode::RestartPaneWithAgent(_, _)
                        ) =>
                {
                    manager.restart_target = Some(manager.selected);
                }
                // Manage renders only as the settings page, never as a
                // dismissible dialog. Enter there has nothing to confirm, and
                // discarding the inventory would strand the page on its
                // "not loaded" notice.
                Key::Named(Named::ArrowUp) if entry_count > 0 => {
                    manager.selected = manager.selected.saturating_sub(1);
                }
                Key::Named(Named::ArrowDown) if entry_count > 0 => {
                    manager.selected = (manager.selected + 1).min(entry_count - 1);
                }
                // Delete only. Backspace reads as "go back" on a full-window
                // page, and the footer advertises Del — an unannounced second
                // chord that removes a checkout is a trap, not a shortcut.
                Key::Named(Named::Delete)
                    if matches!(manager.mode, WorktreeManagerMode::Manage) =>
                {
                    let index = manager.selected;
                    return self.delete_worktree_entry(index);
                }
                _ => {}
            }
            return Vec::new();
        }

        if let Some(prompt) = &self.worktree_prompt {
            // Enter drives the primary action so the dialog is fully keyboard
            // operable: confirm when creation is possible, dismiss the
            // not-a-repo notice otherwise. A double fire from the input's
            // on_submit is absorbed by the busy guard in confirm_worktree.
            let can_confirm = prompt.repo_root.is_some();
            match modified_key.as_ref() {
                Key::Named(Named::Escape) => self.worktree_prompt = None,
                Key::Named(Named::Enter) if can_confirm => return self.confirm_worktree(),
                Key::Named(Named::Enter) => self.worktree_prompt = None,
                _ => {}
            }
            return Vec::new();
        }

        if modifiers.control() && !modifiers.alt() && !modifiers.logo() {
            let resize = if character_key_is(modified_key.as_ref(), "+")
                || character_key_is(modified_key.as_ref(), "=")
            {
                Some(true)
            } else if character_key_is(modified_key.as_ref(), "-") {
                Some(false)
            } else {
                None
            };
            if let Some(increase) = resize {
                self.status = match self.resize_focused_pane(increase) {
                    Ok(status) => status.into(),
                    Err(error) => error,
                };
                return Vec::new();
            }
        }

        if modifiers.alt() && !modifiers.control() && !modifiers.shift() && !modifiers.logo() {
            let cycle = if character_key_is(modified_key.as_ref(), "[") {
                Some(LayoutCycle::Previous)
            } else if character_key_is(modified_key.as_ref(), "]") {
                Some(LayoutCycle::Next)
            } else {
                None
            };
            if let Some(cycle) = cycle {
                self.status = match self.cycle_pane_layout(cycle) {
                    Ok(layout) => format!("Pane layout: {layout}"),
                    Err(error) => error,
                };
                let toast = self.status.clone();
                self.show_toast(&toast);
                return Vec::new();
            }
        }

        if modifiers.alt() && !modifiers.control() && !modifiers.shift() {
            let direction = match modified_key.as_ref() {
                Key::Named(Named::ArrowLeft) => Some(NavDirection::Left),
                Key::Named(Named::ArrowRight) => Some(NavDirection::Right),
                Key::Named(Named::ArrowUp) => Some(NavDirection::Up),
                Key::Named(Named::ArrowDown) => Some(NavDirection::Down),
                _ => None,
            };
            if let Some(direction) = direction {
                if let Err(error) = self.focus_neighbor_pane(direction) {
                    self.status = error;
                }
                return Vec::new();
            }
        }

        let Some(bytes) = encode_terminal_key(modified_key.as_ref(), modifiers, text.as_deref())
        else {
            return Vec::new();
        };

        self.cursor_phase_visible = true;
        if let Err(error) = self.send_terminal_input(bytes) {
            self.status = format!("Terminal input failed: {error}");
        }
        Vec::new()
    }

    /// Keyboard ownership for the GitHub ledger. The panel opts in through a
    /// panel interaction; focusing a terminal opts out. This keeps an open
    /// ledger from ever stealing shell navigation or Enter.
    pub(crate) fn handle_github_panel_keyboard(
        &mut self,
        key: Key<&str>,
        modifiers: Modifiers,
    ) -> Option<Vec<Effect>> {
        let panel = self.github_panel.as_ref()?;
        if self.active_view != ActiveView::Workspace
            || self.palette.visible
            || self.workspace_create_visible
            || self.close_workspace_prompt.is_some()
            || self.rename_prompt.is_some()
            || self.default_agent_prompt
            || self.worktree_prompt.is_some()
        {
            return None;
        }

        let focus = panel.keyboard_focus?;

        if matches!(key, Key::Named(Named::Tab))
            && !modifiers.control()
            && !modifiers.alt()
            && !modifiers.logo()
        {
            let next = github_keyboard_focus_step(panel, focus, !modifiers.shift());
            let panel = self.github_panel.as_mut().expect("panel checked above");
            panel.keyboard_focus = Some(next);
            return Some(vec![Effect::Focus(match next {
                GitHubPanelKeyboardFocus::Search => FocusTarget::GitHubPullRequestQuery,
                _ => FocusTarget::GitHubKeyboardSink,
            })]);
        }
        if !modifiers.is_empty() {
            return None;
        }
        if matches!(key, Key::Named(Named::Escape)) {
            if panel.merge_confirmation {
                return Some(self.update(Message::CancelGitHubMerge));
            }
            if panel.merging || panel.draft_state_updating {
                return Some(Vec::new());
            }
            if panel.selected_pull_request_number.is_some() {
                return Some(self.update(Message::CloseGitHubPullRequest));
            }
            if let Some(panel) = self.github_panel.as_mut() {
                panel.keyboard_focus = None;
                panel.pull_request_keyboard_cursor = None;
                panel.file_keyboard_cursor = None;
            }
            return Some(Vec::new());
        }
        if panel.active_loading() {
            return Some(Vec::new());
        }
        if panel.active_tab == GitHubPanelTab::PullRequests
            && panel.selected_pull_request_number.is_none()
            && character_key_is(key, "/")
        {
            return Some(vec![Effect::Focus(FocusTarget::GitHubPullRequestQuery)]);
        }

        if focus == GitHubPanelKeyboardFocus::Tabs {
            let target = match key {
                Key::Named(Named::ArrowLeft) => Some(GitHubPanelTab::Local),
                Key::Named(Named::ArrowRight) => Some(GitHubPanelTab::PullRequests),
                Key::Named(Named::Enter | Named::Space) => Some(panel.active_tab),
                _ => None,
            };
            return target.map(|tab| self.update(Message::SelectGitHubPanelTab(tab)));
        }
        if focus == GitHubPanelKeyboardFocus::Back
            && matches!(key, Key::Named(Named::Enter | Named::Space))
        {
            return Some(self.update(Message::CloseGitHubPullRequest));
        }
        if focus == GitHubPanelKeyboardFocus::DraftAction
            && matches!(key, Key::Named(Named::Enter | Named::Space))
        {
            return Some(self.update(Message::ToggleGitHubPullRequestDraft));
        }
        if focus == GitHubPanelKeyboardFocus::MergeAction
            && matches!(key, Key::Named(Named::Enter | Named::Space))
        {
            return Some(self.update(if panel.merge_confirmation {
                Message::ConfirmGitHubMerge
            } else {
                Message::RequestGitHubMerge
            }));
        }

        let direction = match key {
            Key::Named(Named::ArrowUp) => Some(false),
            Key::Named(Named::ArrowDown) => Some(true),
            _ => None,
        };

        if panel.active_tab == GitHubPanelTab::PullRequests
            && panel.selected_pull_request_number.is_none()
            && focus == GitHubPanelKeyboardFocus::PullRequestList
        {
            let matching_numbers = panel
                .pull_requests
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter(|pull_request| pull_request.matches(&panel.pull_request_query))
                .map(|pull_request| pull_request.number)
                .collect::<Vec<_>>();
            if let Some(move_down) = direction {
                if matching_numbers.is_empty() {
                    return Some(Vec::new());
                }
                let panel = self.github_panel.as_mut().expect("panel checked above");
                let current = panel.pull_request_keyboard_cursor.unwrap_or(0);
                let next = if move_down {
                    (current + usize::from(panel.pull_request_keyboard_cursor.is_some()))
                        .min(matching_numbers.len() - 1)
                } else {
                    current.saturating_sub(1)
                };
                panel.pull_request_keyboard_cursor = Some(next);
                let viewport = github_pull_request_viewport_height(self.window_size);
                let offset = github_scroll_offset_for_cursor(
                    matching_numbers.len(),
                    next,
                    panel.pull_request_scroll_offset,
                    viewport,
                    GITHUB_PULL_REQUEST_ROW_HEIGHT,
                );
                panel.pull_request_scroll_offset = offset;
                return Some(github_scroll_to(ScrollTarget::GitHubPullRequests, offset));
            }
            if matches!(key, Key::Named(Named::Enter))
                && let Some(number) = panel
                    .pull_request_keyboard_cursor
                    .and_then(|cursor| matching_numbers.get(cursor))
                    .copied()
            {
                return Some(self.select_github_pull_request(number));
            }
            return matches!(key, Key::Named(Named::Enter)).then_some(Vec::new());
        }

        if focus != GitHubPanelKeyboardFocus::Files {
            return None;
        }
        let files = if panel.active_tab == GitHubPanelTab::PullRequests {
            panel
                .selected_pull_request
                .as_ref()
                .map(|details| details.files.as_slice())
                .unwrap_or_default()
        } else {
            panel
                .data
                .as_ref()
                .map(|data| data.files.as_slice())
                .unwrap_or_default()
        };
        if let Some(move_down) = direction {
            if files.is_empty() {
                return Some(Vec::new());
            }
            let count = files.len();
            let pull_request_detail = panel.selected_pull_request_number.is_some();
            let panel = self.github_panel.as_mut().expect("panel checked above");
            let current = panel.file_keyboard_cursor.unwrap_or(0);
            let next = if move_down {
                (current + usize::from(panel.file_keyboard_cursor.is_some())).min(count - 1)
            } else {
                current.saturating_sub(1)
            };
            panel.file_keyboard_cursor = Some(next);
            let viewport = github_file_viewport_height(self.window_size, pull_request_detail);
            let current_offset = if pull_request_detail {
                panel.selected_pull_request_file_scroll_offset
            } else {
                panel.file_scroll_offset
            };
            let offset = github_scroll_offset_for_cursor(
                count,
                next,
                current_offset,
                viewport,
                GITHUB_FILE_ROW_HEIGHT,
            );
            if pull_request_detail {
                panel.selected_pull_request_file_scroll_offset = offset;
            } else {
                panel.file_scroll_offset = offset;
            }
            return Some(github_scroll_to(ScrollTarget::GitHubFiles, offset));
        }
        if matches!(key, Key::Named(Named::Enter))
            && let Some(path) = panel
                .file_keyboard_cursor
                .and_then(|cursor| files.get(cursor))
                .map(|file| file.path.clone())
        {
            return Some(self.open_github_diff(path));
        }
        matches!(key, Key::Named(Named::Enter)).then_some(Vec::new())
    }

    pub(crate) fn move_palette_selection(&mut self, direction: PaletteMove) -> Vec<Effect> {
        let commands = commands::filtered(&self.palette.query);
        let enabled: Vec<_> = commands
            .iter()
            .map(|command| self.command_enabled(command.action))
            .collect();
        self.palette.selected =
            enabled_palette_selection(self.palette.selected, &enabled, direction);
        let count = commands.len();
        let offset = if count > 1 {
            self.palette.selected as f32 / (count - 1) as f32
        } else {
            0.0
        };
        vec![
            Effect::Focus(FocusTarget::CommandPalette),
            Effect::ScrollToRatio(ScrollTarget::CommandPalette, offset),
        ]
    }

    pub(crate) fn toggle_command_palette(&mut self) -> Vec<Effect> {
        if self.palette.visible {
            self.close_command_palette();
            Vec::new()
        } else {
            self.palette.visible = true;
            self.palette.query.clear();
            let commands = commands::filtered("");
            let enabled: Vec<_> = commands
                .iter()
                .map(|command| self.command_enabled(command.action))
                .collect();
            self.palette.selected = first_enabled_palette_command(&enabled);
            vec![Effect::Focus(FocusTarget::CommandPalette)]
        }
    }

    pub(crate) fn command_enabled(&self, action: CommandAction) -> bool {
        self.maximized_pane.is_none() || !action.requires_tiled_panes()
    }

    pub(crate) fn close_command_palette(&mut self) {
        self.palette.visible = false;
        self.palette.query.clear();
        self.palette.selected = 0;
    }

    pub(crate) fn set_cached_pull_request_state(
        &mut self,
        root: &std::path::Path,
        number: u64,
        state: github::CurrentPullRequestState,
    ) {
        if let Some(current) = self
            .github_panel
            .as_mut()
            .and_then(|panel| panel.data.as_mut())
            .and_then(|data| data.current_pull_request.as_mut())
            .filter(|pull_request| pull_request.number == number)
        {
            current.state = state;
        }
        for repository in self.pane_repositories.values_mut().filter(|repository| {
            repository.root.as_deref() == Some(root)
                && repository
                    .pull_request
                    .as_ref()
                    .is_some_and(|pull_request| pull_request.number == number)
        }) {
            if let Some(pull_request) = repository.pull_request.as_mut() {
                pull_request.state = state;
            }
            repository.checked_at = std::time::Instant::now();
        }
    }

    pub(crate) fn next_github_request_generation(&mut self) -> u64 {
        self.github_request_generation = self.github_request_generation.wrapping_add(1);
        self.github_request_generation
    }

    pub(crate) fn open_github_panel(&mut self) -> Vec<Effect> {
        self.close_command_palette();
        let Ok(pane_id) = self.control_pane_id(None) else {
            self.status = "The focused pane is unavailable.".into();
            return Vec::new();
        };
        let Some(directory) = self.pane_working_directory(pane_id) else {
            self.status = "The focused pane has no working directory.".into();
            self.show_toast("The focused pane has no working directory");
            return Vec::new();
        };
        if let Some(panel) = self.github_panel.as_ref() {
            return if panel.active_loading() {
                Vec::new()
            } else {
                self.refresh_github_focused_pane()
            };
        }
        let mut panel = GitHubPanelState::loading(github::Repository {
            root: directory,
            name: String::new(),
            owner_and_name: None,
            host: self.settings.github_host.clone(),
            branch: String::new(),
            head_oid: String::new(),
            wsl_distribution: self.settings.wsl_distribution.clone(),
        });
        panel.context_loading = true;
        self.github_panel = Some(panel);
        self.load_github_focused_pane(GitHubContextLoad::Open)
    }

    pub(crate) fn begin_github_auth(&mut self) -> Vec<Effect> {
        if self.github_auth_busy {
            return Vec::new();
        }
        self.github_auth_busy = true;
        self.github_auth_generation = self.github_auth_generation.wrapping_add(1);
        let generation = self.github_auth_generation;
        self.github_auth_cancellation.cancel();
        self.github_auth_cancellation = ProcessCancellation::default();
        let cancellation = self.github_auth_cancellation.clone();
        let github_host = self.settings.github_host.clone();
        self.status = format!("Finish connecting {github_host} in your browser");
        perform_blocking(
            move || github::authenticate(&github_host, &cancellation),
            move |result| {
                Message::GitHubAuthFinished(generation, result.and_then(std::convert::identity))
            },
        )
    }

    pub(crate) fn queue_github_pane_refresh(&mut self) {
        let Some(panel) = self.github_panel.as_ref() else {
            return;
        };
        self.github_context_generation = self.github_context_generation.wrapping_add(1);
        self.github_context_cancellation.cancel();
        let repository_may_change = self
            .focused_pane_directory()
            .is_none_or(|directory| !directory.starts_with(&panel.repository.root));
        let context_loading = panel.active_tab == GitHubPanelTab::Local || repository_may_change;
        if context_loading {
            let panel = self.github_panel.as_mut().expect("panel checked above");
            panel.context_loading = true;
            panel.loading_phase = 0;
        }
        self.github_pane_refresh_pending = true;
        (self.event_notifier)();
    }

    pub(crate) fn queue_github_pull_request_refresh(&mut self, pane_id: PaneId) {
        if let Some(repository) = self.pane_repositories.get_mut(&pane_id) {
            // Keep truthful cached metadata on screen while the replacement
            // probe runs. Removing the row here made the PR marker blink after
            // every completed agent turn.
            repository.checked_at = std::time::Instant::now() - PANE_REPOSITORY_INTERVAL;
        }
        self.pending_repository_directories.remove(&pane_id);
        if self.control_pane_id(None).ok() != Some(pane_id) {
            return;
        }
        let authenticated = matches!(self.github_auth, github::AuthStatus::Authenticated { .. });
        let Some(panel) = self.github_panel.as_mut() else {
            return;
        };
        let pull_requests_visible = panel.active_tab == GitHubPanelTab::PullRequests;
        if panel.pull_requests.is_none() && !pull_requests_visible {
            return;
        }

        // A completed turn may have created its pull request. Discard the
        // cached list even while another tab is open; selecting Pull requests
        // will then load current data instead of presenting the stale cache.
        panel.pull_requests = None;
        panel.pull_requests_error = None;
        if pull_requests_visible && authenticated {
            panel.pull_requests_loading = true;
            if panel.selected_pull_request_number.is_some() {
                panel.selected_pull_request_loading = true;
            }
            self.github_pull_requests_refresh_pending = true;
        }
    }

    pub(crate) fn refresh_github_panel(&mut self) -> Vec<Effect> {
        let Some(panel) = self.github_panel.as_ref() else {
            return Vec::new();
        };
        match panel.active_tab {
            GitHubPanelTab::Local => self.refresh_github_focused_pane(),
            GitHubPanelTab::PullRequests if panel.selected_pull_request_number.is_some() => {
                self.refresh_selected_github_pull_request()
            }
            GitHubPanelTab::PullRequests => self.refresh_github_pull_requests(),
        }
    }

    pub(crate) fn refresh_github_pull_requests(&mut self) -> Vec<Effect> {
        if !matches!(self.github_auth, github::AuthStatus::Authenticated { .. }) {
            return Vec::new();
        }
        let generation = self.next_github_request_generation();
        let Some(panel) = self.github_panel.as_mut() else {
            return Vec::new();
        };
        panel.pull_requests_loading = true;
        panel.pull_requests_error = None;
        panel.loading_phase = 0;
        panel.pull_request_generation = generation;
        panel.pull_requests_cancellation.cancel();
        panel.pull_requests_cancellation = ProcessCancellation::default();
        let cancellation = panel.pull_requests_cancellation.clone();
        let repository = panel.repository.clone();
        let root = repository.root.clone();
        perform_blocking(
            move || github::list_pull_requests(&repository, &cancellation),
            move |result| {
                Message::GitHubPullRequestsLoaded(
                    root,
                    generation,
                    Box::new(result.and_then(std::convert::identity)),
                )
            },
        )
    }

    pub(crate) fn select_github_pull_request(&mut self, number: u64) -> Vec<Effect> {
        let Some(panel) = self.github_panel.as_mut() else {
            return Vec::new();
        };
        panel.selected_pull_request_number = Some(number);
        panel.selected_pull_request = None;
        panel.selected_pull_request_file_scroll_offset = 0.0;
        panel.file_keyboard_cursor = None;
        panel.keyboard_focus = Some(GitHubPanelKeyboardFocus::Back);
        self.load_github_pull_request(number)
    }

    pub(crate) fn load_github_pull_request(&mut self, number: u64) -> Vec<Effect> {
        let generation = self.next_github_request_generation();
        let Some(panel) = self.github_panel.as_mut() else {
            return Vec::new();
        };
        panel.selected_pull_request_loading = true;
        panel.selected_pull_request_error = None;
        panel.pull_request_action_error = None;
        panel.draft_state_updating = false;
        panel.merge_confirmation = false;
        panel.loading_phase = 0;
        panel.pull_request_detail_generation = generation;
        panel.pull_request_detail_cancellation.cancel();
        panel.pull_request_detail_cancellation = ProcessCancellation::default();
        let cancellation = panel.pull_request_detail_cancellation.clone();
        let repository = panel.repository.clone();
        let root = repository.root.clone();
        perform_blocking(
            move || github::load_pull_request_details(&repository, number, &cancellation),
            move |result| {
                Message::GitHubPullRequestLoaded(
                    root,
                    number,
                    generation,
                    Box::new(result.and_then(std::convert::identity)),
                )
            },
        )
    }

    pub(crate) fn refresh_selected_github_pull_request(&mut self) -> Vec<Effect> {
        let Some(number) = self
            .github_panel
            .as_ref()
            .and_then(|panel| panel.selected_pull_request_number)
        else {
            return Vec::new();
        };
        self.load_github_pull_request(number)
    }

    pub(crate) fn refresh_github_focused_pane(&mut self) -> Vec<Effect> {
        self.load_github_focused_pane(GitHubContextLoad::Refresh)
    }

    pub(crate) fn load_github_focused_pane(&mut self, purpose: GitHubContextLoad) -> Vec<Effect> {
        let Ok(pane_id) = self.control_pane_id(None) else {
            return Vec::new();
        };
        let Some(directory) = self.pane_working_directory(pane_id) else {
            return Vec::new();
        };
        let load_current_pull_request =
            matches!(self.github_auth, github::AuthStatus::Authenticated { .. });
        let wsl_distribution = self.settings.wsl_distribution.clone();
        let github_host = self.settings.github_host.clone();
        self.github_context_generation = self.github_context_generation.wrapping_add(1);
        let generation = self.github_context_generation;
        self.github_context_cancellation.cancel();
        self.github_context_cancellation = ProcessCancellation::default();
        let cancellation = self.github_context_cancellation.clone();
        if let Some(panel) = self.github_panel.as_mut() {
            let repository_may_change = !directory.starts_with(&panel.repository.root);
            panel.context_loading =
                panel.active_tab == GitHubPanelTab::Local || repository_may_change;
            if panel.context_loading {
                panel.loading_phase = 0;
            }
        }
        perform_blocking(
            move || {
                let repository = github::repository_from(
                    &directory,
                    &wsl_distribution,
                    &github_host,
                    &cancellation,
                )?;
                let mut data = github::load_local(&repository, &cancellation)?;
                if load_current_pull_request {
                    data.current_pull_request =
                        github::current_pull_request(&repository, &cancellation)
                            .ok()
                            .flatten();
                }
                Ok((repository, data))
            },
            move |result| {
                Message::GitHubFocusedPaneLoaded(
                    pane_id,
                    generation,
                    purpose,
                    Box::new(result.and_then(std::convert::identity)),
                )
            },
        )
    }

    pub(crate) fn open_github_diff(&mut self, path: String) -> Vec<Effect> {
        let Some((file, source, pull_request, repository)) =
            self.github_panel.as_ref().and_then(|panel| {
                let selection = match panel.active_tab {
                    GitHubPanelTab::Local => panel
                        .data
                        .as_ref()
                        .and_then(|data| data.files.iter().find(|file| file.path == path))
                        .cloned()
                        .map(|file| (file, GitHubDiffSource::Local, None)),
                    GitHubPanelTab::PullRequests => {
                        panel.selected_pull_request.as_ref().and_then(|details| {
                            details
                                .files
                                .iter()
                                .find(|file| file.path == path)
                                .cloned()
                                .map(|file| {
                                    (
                                        file,
                                        GitHubDiffSource::PullRequest(details.pull_request.number),
                                        Some(details.pull_request.clone()),
                                    )
                                })
                        })
                    }
                };
                selection.map(|(file, source, pull_request)| {
                    (file, source, pull_request, panel.repository.clone())
                })
            })
        else {
            self.status = "The selected file is no longer in the change set.".into();
            return Vec::new();
        };
        let generation = self.next_github_request_generation();
        if let Some(diff) = self.github_diff.take() {
            diff.cancellation.cancel();
        }
        let cancellation = ProcessCancellation::default();
        let request_cancellation = cancellation.clone();
        let root = repository.root.clone();
        self.github_diff = Some(GitHubDiffState {
            source,
            path: file.path.clone(),
            status: file.status.clone(),
            additions: file.additions,
            deletions: file.deletions,
            document: None,
            loading: true,
            error: None,
            generation,
            cancellation,
            scroll_offset: 0.0,
            wrap_columns: None,
            line_starts: vec![0],
        });
        self.active_view = ActiveView::GitHubDiff;
        let path = file.path.clone();
        perform_blocking(
            move || match pull_request {
                Some(pull_request) => github::load_pull_request_diff(
                    &repository,
                    &pull_request,
                    &file,
                    &request_cancellation,
                ),
                None => github::load_diff(&repository, &file, false, &request_cancellation),
            },
            move |result| {
                Message::GitHubDiffLoaded(
                    root,
                    path,
                    generation,
                    Box::new(result.and_then(std::convert::identity)),
                )
            },
        )
    }

    pub(crate) fn reflow_github_diff(&mut self) {
        let wrap_columns =
            github_diff_wrap_columns(self.window_size.width, self.settings.terminal_cell_width());
        let Some(diff) = self
            .github_diff
            .as_mut()
            .filter(|diff| diff.wrap_columns != wrap_columns)
        else {
            return;
        };
        let Some(document) = diff.document.as_ref() else {
            diff.wrap_columns = wrap_columns;
            return;
        };
        let old_visual_row = (diff.scroll_offset / GITHUB_DIFF_LINE_HEIGHT).floor() as usize;
        let anchor_line = github_diff_line_for_visual_row(&diff.line_starts, old_visual_row);
        let line_starts = github_diff_line_starts(document, wrap_columns);
        diff.scroll_offset = line_starts.get(anchor_line).copied().unwrap_or_default() as f32
            * GITHUB_DIFF_LINE_HEIGHT;
        diff.line_starts = line_starts;
        diff.wrap_columns = wrap_columns;
    }

    pub(crate) fn toggle_github_pull_request_draft(&mut self) -> Vec<Effect> {
        let generation = self.next_github_request_generation();
        let Some(panel) = self.github_panel.as_mut() else {
            return Vec::new();
        };
        if panel.draft_state_updating || panel.merging {
            return Vec::new();
        }
        let Some(pull_request) = panel
            .selected_pull_request
            .as_ref()
            .map(|details| &details.pull_request)
        else {
            return Vec::new();
        };
        let number = pull_request.number;
        let draft = !pull_request.draft;
        panel.draft_state_updating = true;
        panel.pull_request_action_error = None;
        panel.merge_confirmation = false;
        panel.loading_phase = 0;
        panel.action_generation = generation;
        panel.action_cancellation.cancel();
        panel.action_cancellation = ProcessCancellation::default();
        let cancellation = panel.action_cancellation.clone();
        let repository = panel.repository.clone();
        let root = repository.root.clone();
        perform_blocking(
            move || github::set_draft(&repository, number, draft, &cancellation),
            move |result| {
                Message::GitHubPullRequestDraftChanged(
                    root,
                    number,
                    generation,
                    draft,
                    result.and_then(std::convert::identity),
                )
            },
        )
    }

    pub(crate) fn confirm_github_merge(&mut self) -> Vec<Effect> {
        let generation = self.next_github_request_generation();
        let Some(panel) = self.github_panel.as_mut() else {
            return Vec::new();
        };
        if panel.merging {
            panel.merge_confirmation = false;
            return Vec::new();
        }
        let Some(pull_request) = panel
            .selected_pull_request
            .as_ref()
            .map(|details| &details.pull_request)
        else {
            panel.merge_confirmation = false;
            return Vec::new();
        };
        if pull_request.readiness() != github::MergeReadiness::Ready {
            panel.merge_confirmation = false;
            panel.selected_pull_request_error =
                Some("This pull request is not ready to merge yet.".into());
            return Vec::new();
        }
        let number = pull_request.number;
        let head_oid = pull_request.head_oid.clone();
        panel.merging = true;
        panel.merge_confirmation = false;
        panel.selected_pull_request_error = None;
        panel.pull_request_action_error = None;
        panel.loading_phase = 0;
        panel.action_generation = generation;
        panel.action_cancellation.cancel();
        panel.action_cancellation = ProcessCancellation::default();
        let cancellation = panel.action_cancellation.clone();
        let repository = panel.repository.clone();
        let root = repository.root.clone();
        perform_blocking(
            move || github::merge(&repository, number, &head_oid, &cancellation),
            move |result| {
                Message::GitHubMergeFinished(
                    root,
                    number,
                    generation,
                    result.and_then(std::convert::identity),
                )
            },
        )
    }

    pub(crate) fn run_command(&mut self, action: CommandAction) -> Vec<Effect> {
        if !self.command_enabled(action) {
            self.status = "Restore panes before changing their layout".into();
            return Vec::new();
        }
        self.close_command_palette();
        match action {
            CommandAction::Split(axis) => {
                self.active_view = ActiveView::Workspace;
                self.status = match self.split_terminal(axis) {
                    Ok(()) => "Opened a new terminal pane".into(),
                    Err(error) => error,
                };
            }
            CommandAction::GrowPane => {
                self.active_view = ActiveView::Workspace;
                self.status = match self.resize_focused_pane(true) {
                    Ok(status) => status.into(),
                    Err(error) => error,
                };
            }
            CommandAction::RestorePaneSize => {
                self.active_view = ActiveView::Workspace;
                self.status = match self.resize_focused_pane(false) {
                    Ok(status) => status.into(),
                    Err(error) => error,
                };
            }
            CommandAction::PreviousPaneLayout | CommandAction::NextPaneLayout => {
                self.active_view = ActiveView::Workspace;
                let cycle = if action == CommandAction::PreviousPaneLayout {
                    LayoutCycle::Previous
                } else {
                    LayoutCycle::Next
                };
                self.status = match self.cycle_pane_layout(cycle) {
                    Ok(layout) => format!("Pane layout: {layout}"),
                    Err(error) => error,
                };
                let toast = self.status.clone();
                self.show_toast(&toast);
            }
            CommandAction::ClosePane => {
                self.active_view = ActiveView::Workspace;
                self.status = match self.close_focused() {
                    Ok(()) if self.close_workspace_prompt.is_some() => {
                        "Confirm closing the workspace".into()
                    }
                    Ok(()) => "Closed the focused pane".into(),
                    Err(error) => error,
                };
            }
            CommandAction::NewTab => {
                self.status = match self.new_tab() {
                    Ok(()) => "Created a new tab".into(),
                    Err(error) => error,
                };
            }
            CommandAction::CloseTab => {
                let target = self
                    .active_workspace()
                    .map(|workspace| (workspace.id, workspace.active_tab_id));
                self.status = match target
                    .and_then(|(workspace_id, tab_id)| self.close_tab(workspace_id, tab_id))
                {
                    Ok(()) if self.close_workspace_prompt.is_some() => {
                        "Confirm closing the workspace".into()
                    }
                    Ok(()) => "Closed the tab".into(),
                    Err(error) => error,
                };
            }
            CommandAction::NewWorkspace => {
                return self.open_workspace_create();
            }
            CommandAction::CloseWorkspace => {
                self.status = match self.close_workspace() {
                    Ok(()) => "Closed the workspace".into(),
                    Err(error) => error,
                };
            }
            CommandAction::CopySelection => {
                self.active_view = ActiveView::Workspace;
                if let Ok(workspace) = self.active_workspace()
                    && let Some(tab) = workspace.active_tab()
                {
                    if let Some(text) = self.selected_terminal_text(tab.focused_pane_id) {
                        return self.copy_terminal_selection(text);
                    }
                    self.status = "Nothing is selected in the focused terminal".into();
                }
            }
            CommandAction::PasteClipboard => {
                self.active_view = ActiveView::Workspace;
                if let Ok(workspace) = self.active_workspace()
                    && let Some(tab) = workspace.active_tab()
                {
                    let pane_id = tab.focused_pane_id;
                    return vec![Effect::ClipboardRead(Arc::new(move |contents| {
                        Message::ClipboardPasted(pane_id, contents)
                    }))];
                }
            }
            CommandAction::RenameWorkspace => {
                if let Ok(workspace) = self.active_workspace() {
                    let target = RenameTarget::Workspace(workspace.id);
                    let name = workspace.name.clone();
                    return self.open_rename_prompt(target, name);
                }
            }
            CommandAction::RenameTab => {
                if let Ok(workspace) = self.active_workspace()
                    && let Some(tab) = workspace.active_tab()
                {
                    let target = RenameTarget::Tab(workspace.id, tab.id);
                    let name = tab.name.clone();
                    return self.open_rename_prompt(target, name);
                }
            }
            CommandAction::RenamePane => {
                if let Ok(workspace) = self.active_workspace()
                    && let Some(tab) = workspace.active_tab()
                {
                    let pane_id = tab.focused_pane_id;
                    let name = workspace
                        .pane(pane_id)
                        .and_then(|pane| pane.custom_name.clone())
                        .unwrap_or_default();
                    return self.open_rename_prompt(RenameTarget::Pane(pane_id), name);
                }
            }
            CommandAction::NewWorktree(kind) => {
                return self.open_worktree_prompt(WorktreePromptTarget::Open(kind));
            }
            CommandAction::NewWorktreeWithAgent(kind) => {
                let action = CommandAction::NewWorktreeWithAgent(kind);
                let Some(agent) = self.default_agent_for_worktree_command(action) else {
                    return Vec::new();
                };
                return self.open_worktree_prompt(WorktreePromptTarget::OpenWithAgent(kind, agent));
            }
            CommandAction::RestartPaneInWorktree => {
                if let Ok(workspace) = self.active_workspace()
                    && let Some(tab) = workspace.active_tab()
                {
                    return self.open_worktree_prompt(WorktreePromptTarget::RestartPane(
                        tab.focused_pane_id,
                    ));
                }
            }
            CommandAction::RestartPaneInExistingWorktree => {
                if let Ok(workspace) = self.active_workspace()
                    && let Some(tab) = workspace.active_tab()
                {
                    return self.open_worktree_switcher(tab.focused_pane_id);
                }
            }
            CommandAction::RestartPaneInWorktreeWithAgent => {
                let action = CommandAction::RestartPaneInWorktreeWithAgent;
                let Some(agent) = self.default_agent_for_worktree_command(action) else {
                    return Vec::new();
                };
                if let Ok(workspace) = self.active_workspace()
                    && let Some(tab) = workspace.active_tab()
                {
                    return self.open_worktree_prompt(WorktreePromptTarget::RestartPaneWithAgent(
                        tab.focused_pane_id,
                        agent,
                    ));
                }
            }
            CommandAction::RestartPaneInExistingWorktreeWithAgent => {
                let action = CommandAction::RestartPaneInExistingWorktreeWithAgent;
                let Some(agent) = self.default_agent_for_worktree_command(action) else {
                    return Vec::new();
                };
                if let Ok(workspace) = self.active_workspace()
                    && let Some(tab) = workspace.active_tab()
                {
                    let pane_id = tab.focused_pane_id;
                    return self.open_worktree_list(
                        WorktreeManagerMode::RestartPaneWithAgent(pane_id, agent),
                        pane_id,
                    );
                }
            }
            CommandAction::ManageWorktrees => return self.open_worktree_manager(),
            CommandAction::ManageSessions => {
                self.open_session_picker(false);
                return Vec::new();
            }
            CommandAction::FleetToggleAllWorkspaces => {
                let scope = if self.settings.fleet_scope == FleetScope::AllWorkspaces {
                    FleetScope::CurrentWorkspace
                } else {
                    FleetScope::AllWorkspaces
                };
                self.set_fleet_scope(scope);
                self.status = match scope {
                    FleetScope::CurrentWorkspace => "Fleet shows only the current workspace",
                    FleetScope::AllWorkspaces => "Fleet shows all workspaces",
                }
                .into();
                return self.refresh_pane_repositories();
            }
            CommandAction::FleetTabs => {
                self.set_fleet_view(FleetView::Tabs);
                self.status = "Fleet lists every pane".into();
            }
            CommandAction::FleetAgents => {
                self.set_fleet_view(FleetView::Agents);
                self.status = "Fleet filters to agent panes".into();
            }
            CommandAction::FleetRepos => {
                self.set_fleet_view(FleetView::Repos);
                self.status = "Fleet groups panes by repository".into();
                return self.refresh_pane_repositories();
            }
            CommandAction::OpenGitHubPanel => return self.open_github_panel(),
            CommandAction::OpenSettings => return self.open_settings(),
            CommandAction::LaunchAgent(agent) => {
                self.active_view = ActiveView::Workspace;
                self.status = match self.launch_agent(agent) {
                    Ok(()) => format!("Launched {agent} in a new pane"),
                    Err(error) => error,
                };
            }
            CommandAction::ReturnToWorkspace => self.active_view = ActiveView::Workspace,
        }
        Vec::new()
    }

    pub(crate) fn launch_agent(&mut self, agent: Agent) -> Result<(), String> {
        self.agent_launch_command(agent)?;
        self.split_terminal(SplitAxis::Horizontal)?;
        let pane_id = self
            .active_workspace()?
            .active_tab()
            .ok_or_else(|| "active tab is missing".to_owned())?
            .focused_pane_id;
        self.start_agent_in_pane(agent, pane_id)
    }

    pub(crate) fn start_agent_in_pane(
        &mut self,
        agent: Agent,
        pane_id: PaneId,
    ) -> Result<(), String> {
        let command = self.agent_launch_command(agent)?;
        let input = format!("{command}\r").into_bytes();
        if self
            .terminals
            .get(&pane_id)
            .is_some_and(|runtime| runtime.session.is_some())
        {
            self.send_terminal_input_to(pane_id, input)?;
        } else {
            self.pending_terminal_input
                .entry(pane_id)
                .or_default()
                .push(input);
        }
        let agent = agent.to_string();
        let activity = Some(format!("Starting {}", agent_display_name(&agent)));
        self.record_running_agent(pane_id, agent, activity);
        Ok(())
    }

    pub(crate) fn record_running_agent(
        &mut self,
        pane_id: PaneId,
        agent: String,
        activity: Option<String>,
    ) {
        let display_name = self.agent_worktree_name(pane_id);
        self.agent_statuses.insert(
            pane_id,
            AgentPaneStatus {
                agent,
                display_name,
                state: AgentState::Running,
                activity,
                session_id: None,
                cwd: None,
                git_branch: None,
            },
        );
        self.agent_running_frame_revisions.insert(
            pane_id,
            self.terminals
                .get(&pane_id)
                .map_or(0, |runtime| runtime.snapshot_revision),
        );
    }

    pub(crate) fn agent_launch_command(&self, agent: Agent) -> Result<String, String> {
        let command = agent_command_setting(&self.settings, agent).trim();
        if command.is_empty() {
            return Err(format!(
                "Set a launch command for {} in Settings first",
                agent_display_name(&agent.to_string())
            ));
        }
        Ok(command.to_owned())
    }

    pub(crate) fn default_agent_for_worktree_command(
        &mut self,
        action: CommandAction,
    ) -> Option<Agent> {
        let agent = self.configured_default_agent();
        if agent.is_some() {
            self.pending_default_agent_command = None;
        } else {
            self.active_view = ActiveView::Workspace;
            self.default_agent_prompt = true;
            self.pending_default_agent_command = Some(action);
            self.status =
                "Choose a configured default agent before opening a worktree with an agent".into();
        }
        agent
    }

    pub(crate) fn reset_settings_draft(&mut self) {
        self.settings_draft = self.settings.clone();
        self.settings_scrollback_lines_input = self.settings.terminal_scrollback_lines.to_string();
    }

    pub(crate) fn open_settings(&mut self) -> Vec<Effect> {
        self.reset_settings_draft();
        self.workspace_name_draft = self
            .active_workspace()
            .map_or_else(|_| String::new(), |workspace| workspace.name.clone());
        self.available_terminal_font_weights = self
            .installed_fonts
            .terminal_weights(&self.settings_draft.terminal_font);
        self.available_ui_font_weights = self
            .installed_fonts
            .ui_weights(&self.settings_draft.ui_font);
        if self.available_ui_font_weights.is_empty() {
            self.available_ui_font_weights.push(FontWeight::Normal);
        }
        if !self.settings_draft.wsl_distribution.trim().is_empty()
            && !self.available_wsl_distributions.iter().any(|choice| {
                choice.0.as_deref() == Some(self.settings_draft.wsl_distribution.trim())
            })
        {
            self.available_wsl_distributions
                .push(WslDistributionChoice(Some(
                    self.settings_draft.wsl_distribution.trim().to_owned(),
                )));
        }
        self.settings_page = SettingsPage::Preferences;
        self.active_view = ActiveView::Settings;
        self.close_command_palette();
        self.installed_versions = InstalledVersionsState::Checking;
        let installed_muxtrix_path = self.installed_muxtrix_path.clone();
        perform_blocking(
            move || probe_installed_versions(installed_muxtrix_path),
            Message::InstalledVersionsLoaded,
        )
    }

    pub(crate) fn refresh_integrations(&mut self) -> Vec<Effect> {
        self.integration_generation = self.integration_generation.wrapping_add(1);
        let generation = self.integration_generation;
        self.integration_refreshing = true;
        let settings = self.settings_draft.clone();
        perform_blocking(
            move || {
                let mut wsl_distributions = discover_wsl_distributions();
                let selected = settings.wsl_distribution.trim();
                if !selected.is_empty()
                    && !wsl_distributions
                        .iter()
                        .any(|choice| choice.0.as_deref() == Some(selected))
                {
                    wsl_distributions.push(WslDistributionChoice(Some(selected.to_owned())));
                }
                IntegrationDiscovery {
                    wsl_distributions,
                    hook_statuses: load_hook_statuses(&settings),
                }
            },
            move |result| Message::IntegrationDiscoveryFinished(generation, result),
        )
    }

    pub(crate) fn manage_hooks(&mut self, agent: Agent, action: HookAction) -> Vec<Effect> {
        self.integration_generation = self.integration_generation.wrapping_add(1);
        let generation = self.integration_generation;
        self.integration_refreshing = true;
        self.status = format!("Updating {agent} hooks…");
        let settings = self.settings_draft.clone();
        perform_blocking(
            move || {
                let manager = hook_manager(&settings)?;
                let result = manager
                    .apply(agent, HookScope::User, action)
                    .map_err(|error| error.to_string())?;
                let statuses = Agent::ALL
                    .into_iter()
                    .map(|candidate| {
                        manager
                            .status(candidate, HookScope::User)
                            .map_err(|error| error.to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(HookOperationResult {
                    message: result.message,
                    statuses,
                })
            },
            move |result| {
                Message::HookOperationFinished(generation, result.and_then(|result| result))
            },
        )
    }

    pub(crate) fn update_stale_hook_alert(&mut self) {
        const TITLE: &str = "Agent hooks need repair";
        self.global_alerts.retain(|alert| alert.title != TITLE);
        let stale: Vec<_> = self
            .hook_statuses
            .iter()
            .filter(|status| !status.installed && status.managed_entries > 0)
            .map(|status| agent_display_name(&status.agent.to_string()).to_owned())
            .collect();
        if !stale.is_empty() {
            self.global_alerts.push(GlobalAlert {
                title: TITLE.into(),
                body: format!(
                    "{} hooks point to another Muxtrix installation. Open Settings and choose Repair.",
                    stale.join(" and ")
                ),
            });
        }
    }

    pub(crate) fn agent_is_configured_for(&self, agent: Agent, settings: &AppSettings) -> bool {
        !agent_command_setting(settings, agent).trim().is_empty()
            && self.hook_statuses.iter().any(|status| {
                status.agent == agent && status.scope == HookScope::User && status.installed
            })
    }

    pub(crate) fn agent_is_configured(&self, agent: Agent) -> bool {
        self.agent_is_configured_for(agent, &self.settings)
    }

    pub(crate) fn configured_default_agent(&self) -> Option<Agent> {
        let agent = self.settings.default_agent?;
        // The initial discovery starts with the app itself. Keep a previously
        // verified choice usable during that brief refresh; once statuses land,
        // external removal or semantic drift closes the gate immediately.
        if self.integration_refreshing && self.hook_statuses.is_empty() {
            return Some(agent);
        }
        self.agent_is_configured(agent).then_some(agent)
    }

    pub(crate) fn resume_pending_default_agent_command(&mut self) -> Vec<Effect> {
        let Some(action) = self.pending_default_agent_command.take() else {
            return Vec::new();
        };
        if self.configured_default_agent().is_some() {
            return self.run_command(action);
        }
        self.pending_default_agent_command = Some(action);
        self.default_agent_prompt = true;
        self.status = "Choose a configured default agent to continue the worktree command".into();
        Vec::new()
    }

    pub(crate) fn save_settings(&mut self) -> Vec<Effect> {
        let scrollback_lines = match settings::parse_terminal_scrollback_lines(
            &self.settings_scrollback_lines_input,
        ) {
            Ok(lines) => lines,
            Err(error) => {
                self.status = format!("Could not save settings: {error}");
                return Vec::new();
            }
        };
        self.settings_draft.terminal_scrollback_lines = scrollback_lines;
        self.settings_scrollback_lines_input = scrollback_lines.to_string();
        let github_host = match settings::normalize_github_host(&self.settings_draft.github_host) {
            Ok(host) => host,
            Err(error) => {
                self.status = format!("Could not save settings: {error}");
                return Vec::new();
            }
        };
        self.settings_draft.github_host = github_host;
        let github_host_changed = self.settings_draft.github_host != self.settings.github_host;
        let fleet_scope_changed = self.settings_draft.fleet_scope != self.settings.fleet_scope;
        self.settings = self.settings_draft.clone();
        let terminal_theme = self.settings.terminal_theme.preset().terminal_theme();
        let mut theme_error = None;
        for runtime in self.terminals.values() {
            if let Some(session) = &runtime.session
                && let Err(error) = session.apply_theme(terminal_theme)
            {
                theme_error = Some(error.to_string());
            }
        }
        if let Some(profile) = self.session.profiles.first_mut() {
            let profile_id = profile.id;
            *profile = default_profile_with_id(&self.settings, profile_id);
        }
        self.active_view = ActiveView::Workspace;
        let viewports: Vec<(PaneId, Size)> = self
            .terminals
            .iter()
            .filter_map(|(pane_id, runtime)| runtime.viewport.map(|size| (*pane_id, size)))
            .collect();
        for (pane_id, viewport) in viewports {
            if let Err(error) = self.resize_terminal(pane_id, viewport) {
                self.status = format!("Font saved, but terminal resize failed: {error}");
                return Vec::new();
            }
        }
        let save_result = self.settings.save();
        let saved = save_result.is_ok();
        self.status = match (save_result, theme_error) {
            (Ok(path), None) => format!("Settings saved to {}", path.display()),
            (Ok(path), Some(error)) => format!(
                "Settings saved to {}, but a terminal could not update its theme: {error}",
                path.display()
            ),
            (Err(error), _) => format!("Could not save settings: {error}"),
        };
        let mut tasks = vec![self.window_resize_increment_task()];
        if github_host_changed {
            self.github_auth_generation = self.github_auth_generation.wrapping_add(1);
            let generation = self.github_auth_generation;
            self.github_auth_cancellation.cancel();
            self.github_auth_cancellation = ProcessCancellation::default();
            let cancellation = self.github_auth_cancellation.clone();
            let github_host = self.settings.github_host.clone();
            self.github_auth_busy = false;
            self.github_auth = github::AuthStatus::Checking;
            self.github_context_cancellation.cancel();
            self.github_context_generation = self.github_context_generation.wrapping_add(1);
            if let Some(panel) = self.github_panel.take() {
                panel.cancel_requests();
            }
            if let Some(diff) = self.github_diff.take() {
                diff.cancellation.cancel();
            }
            self.pane_repositories.clear();
            self.pending_repository_directories.clear();
            tasks.push(self.refresh_pane_repositories());
            tasks.push(perform_blocking(
                move || github::auth_status(&github_host, &cancellation),
                move |result| {
                    Message::GitHubAuthChecked(
                        generation,
                        result.unwrap_or(github::AuthStatus::Unavailable {
                            reason: "GitHub authentication could not be checked.".into(),
                        }),
                    )
                },
            ));
        }
        if fleet_scope_changed {
            tasks.push(self.refresh_pane_repositories());
        }
        if saved && self.pending_default_agent_command.is_some() {
            tasks.push(self.resume_pending_default_agent_command());
        }
        effect::batch(tasks)
    }

    pub(crate) fn send_terminal_input(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        let focused_pane_id = self
            .active_workspace()?
            .active_tab()
            .ok_or_else(|| "active tab is missing".to_owned())?
            .focused_pane_id;
        self.send_terminal_input_to(focused_pane_id, bytes)
    }

    pub(crate) fn send_terminal_input_to(
        &mut self,
        pane_id: PaneId,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        {
            let session = self
                .terminals
                .get(&pane_id)
                .and_then(|runtime| runtime.session.as_ref())
                .ok_or_else(|| format!("pane {pane_id:?} has no live terminal"))?;
            session
                .input(bytes.clone())
                .map_err(|error| error.to_string())?;
        }
        self.observe_agent_interrupt(pane_id, &bytes);
        self.observe_terminal_command(pane_id, &bytes);
        Ok(())
    }

    /// Detection beneath the hooks: an agent process running inside a pane is
    /// an agent even before (or without) any lifecycle hook firing. Hook
    /// events enrich that status, while process observation remains in place
    /// so returning to the shell clears stale lifecycle metadata.
    pub(crate) fn detect_agent_processes(&mut self) {
        let pane_ids: Vec<PaneId> = self.terminals.keys().copied().collect();
        for pane_id in pane_ids {
            match self.pane_agent_process(pane_id) {
                Some(agent) => {
                    self.detected_agents
                        .insert(pane_id, std::time::Instant::now());
                    if !self.agent_statuses.contains_key(&pane_id) {
                        self.record_running_agent(pane_id, agent, None);
                    }
                }
                None if self.detected_agents.get(&pane_id).is_some_and(|last_seen| {
                    last_seen.elapsed() > std::time::Duration::from_secs(2)
                }) =>
                {
                    self.agent_statuses.remove(&pane_id);
                    self.agent_running_frame_revisions.remove(&pane_id);
                    self.pi_active_lifecycles.remove(&pane_id);
                    self.detected_agents.remove(&pane_id);
                }
                None => {}
            }
        }
    }

    /// Walks the pane's process tree via /proc looking for a known agent
    /// executable. Bounded breadth-first walk; the shell itself is skipped.
    #[cfg(target_os = "linux")]
    pub(crate) fn pane_agent_process(&self, pane_id: PaneId) -> Option<String> {
        let root = self
            .terminals
            .get(&pane_id)?
            .session
            .as_ref()?
            .process_id()?;
        let codex = command_executable(&self.settings.codex_command)
            .unwrap_or("codex")
            .to_ascii_lowercase();
        let claude = command_executable(&self.settings.claude_command)
            .unwrap_or("claude")
            .to_ascii_lowercase();
        let pi = command_executable(&self.settings.pi_command)
            .unwrap_or("omp")
            .to_ascii_lowercase();
        let mut queue = std::collections::VecDeque::from([root]);
        let mut inspected = 0;
        while let Some(pid) = queue.pop_front() {
            inspected += 1;
            if inspected > 32 {
                break;
            }
            if pid != root
                && let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm"))
            {
                let comm = comm.trim().to_ascii_lowercase();
                if comm == "codex" || comm == codex {
                    return Some("codex".into());
                }
                if comm == "claude" || comm == "claude-code" || comm == claude {
                    return Some("claude".into());
                }
                if comm == "omp" || comm == "pi" || comm == pi {
                    return Some("pi".into());
                }
            }
            if let Ok(children) =
                std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
            {
                queue.extend(
                    children
                        .split_whitespace()
                        .filter_map(|pid| pid.parse::<u32>().ok()),
                );
            }
        }
        None
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn pane_agent_process(&self, _pane_id: PaneId) -> Option<String> {
        None
    }

    pub(crate) fn paste_into_pane(&mut self, pane_id: PaneId, text: &str) -> Result<(), String> {
        self.terminals
            .get(&pane_id)
            .and_then(|runtime| runtime.session.as_ref())
            .ok_or_else(|| "the pane has no live terminal".to_owned())?
            .paste(text)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn observe_agent_interrupt(&mut self, pane_id: PaneId, bytes: &[u8]) {
        if !bytes.contains(&0x03) {
            return;
        }
        self.pi_active_lifecycles.remove(&pane_id);
        let Some(status) = self.agent_statuses.get_mut(&pane_id) else {
            return;
        };
        if status.state == AgentState::Running {
            status.state = AgentState::Idle;
            status.activity = Some("Prompt interrupted".into());
        }
    }

    pub(crate) fn observe_terminal_command(&mut self, pane_id: PaneId, bytes: &[u8]) {
        if self.agent_statuses.contains_key(&pane_id) {
            return;
        }
        let buffer = self.terminal_command_buffers.entry(pane_id).or_default();
        let mut submitted = None;
        for byte in bytes {
            match byte {
                b'\r' | b'\n' => {
                    if !buffer.trim().is_empty() {
                        submitted = Some(std::mem::take(buffer));
                    } else {
                        buffer.clear();
                    }
                }
                0x08 | 0x7f => {
                    buffer.pop();
                }
                0x03 | 0x1b => buffer.clear(),
                byte if byte.is_ascii_graphic() || *byte == b' ' => buffer.push(char::from(*byte)),
                _ => {}
            }
        }
        let Some(command) = submitted else {
            return;
        };
        let Some(agent) = agent_command(&command, &self.settings) else {
            return;
        };
        let agent = agent.to_string();
        let activity = Some(format!("Starting {}", agent_display_name(&agent)));
        self.record_running_agent(pane_id, agent, activity);
    }

    pub(crate) fn agent_worktree_name(&self, pane_id: PaneId) -> Option<String> {
        let directory = self.pane_working_directory(pane_id)?;
        linked_worktree_name(&directory)
    }

    pub(crate) fn resize_terminal(&mut self, pane_id: PaneId, size: Size) -> Result<(), String> {
        self.terminals
            .get_mut(&pane_id)
            .ok_or_else(|| format!("pane {pane_id:?} has no terminal runtime"))?
            .resize(size, &self.settings)
    }

    pub(crate) fn scroll_terminal(
        &mut self,
        pane_id: PaneId,
        delta: ScrollDelta,
    ) -> Result<(), String> {
        self.focus_pane(pane_id)?;
        let lines = terminal_scroll_lines(delta, self.settings.terminal_cell_height());
        if lines == 0 {
            return Ok(());
        }
        let runtime = self
            .terminals
            .get(&pane_id)
            .ok_or_else(|| format!("pane {pane_id:?} has no terminal runtime"))?;
        // The 1-based pointer cell, for applications that report the mouse.
        let cell = self
            .terminal_pointer_positions
            .get(&pane_id)
            .map(|position| {
                let column =
                    ((position.x - 8.0).max(0.0) / self.settings.terminal_cell_width()) as u16;
                let row =
                    ((position.y - 8.0).max(0.0) / self.settings.terminal_cell_height()) as u16;
                (
                    (column + 1).min(runtime.size.cols.max(1)),
                    (row + 1).min(runtime.size.rows.max(1)),
                )
            });
        // Selection remains emulator-owned. If this wheel is answered by a
        // repainting application, the terminal session preserves the selected
        // text and re-anchors it after the new frame arrives.
        runtime.wheel(lines, cell)
    }

    pub(crate) fn begin_terminal_mouse(
        &mut self,
        pane_id: PaneId,
        button: TerminalMouseButton,
    ) -> Vec<Effect> {
        let _ = self.focus_pane(pane_id);
        let position = self
            .terminal_pointer_positions
            .get(&pane_id)
            .copied()
            .unwrap_or(Point::ORIGIN);
        let mouse_reporting = self
            .terminals
            .get(&pane_id)
            .and_then(|runtime| runtime.snapshot.as_ref())
            .is_some_and(|snapshot| snapshot.mouse_reporting);

        // Match Ghostty's default host policy: a mouse-reporting program owns
        // unmodified pointer input, while Shift escapes capture so terminal
        // text can still be selected locally.
        if mouse_reporting && !self.keyboard_modifiers.shift() {
            let event = terminal_mouse_event(
                position,
                TerminalMouseAction::Press,
                Some(button),
                self.keyboard_modifiers,
            );
            let result = self
                .terminals
                .get_mut(&pane_id)
                .ok_or_else(|| format!("pane {pane_id:?} has no terminal runtime"))
                .and_then(|runtime| {
                    runtime
                        .selection_clear()
                        .and_then(|()| runtime.mouse(event))
                });
            match result {
                Ok(()) => {
                    self.terminal_mouse_capture = Some(TerminalMouseCapture { pane_id, button });
                }
                Err(error) => self.status = format!("Terminal mouse press failed: {error}"),
            }
            return Vec::new();
        }

        match button {
            TerminalMouseButton::Left => {
                if terminal_link_modifiers(self.keyboard_modifiers)
                    && let Some(link) = self.hovered_terminal_link(pane_id)
                {
                    let uri = link.uri;
                    let target = uri.clone();
                    return vec![Effect::Perform(Box::new(move || {
                        let result = open_web_url(&target).map_err(|error| error.to_string());
                        Message::TerminalLinkOpened(uri, result)
                    }))];
                }
                let cell = self.terminal_grid_cell_at(pane_id, position);
                if let Some(runtime) = self.terminals.get_mut(&pane_id)
                    && let Err(error) = runtime.selection_clear()
                {
                    self.status = format!("Selection failed: {error}");
                }
                self.terminal_selection_drag = Some(TerminalSelectionDrag {
                    pane_id,
                    origin: position,
                    anchor: cell,
                    active: false,
                });
            }
            TerminalMouseButton::Right => self.pane_menu = Some(pane_id),
            TerminalMouseButton::Middle => {}
        }
        Vec::new()
    }

    /// Commits a genuine text-selection drag and returns its pane exactly once.
    ///
    /// Ghostty copies only when the button is released rather than on every
    /// pointer move. Accepting `None` lets the window-level release finish a
    /// drag that ended outside the terminal; the pane-level release normally
    /// wins, and taking the gesture prevents a duplicate clipboard write.
    pub(crate) fn finish_terminal_selection(&mut self, pane_id: Option<PaneId>) -> Option<PaneId> {
        let drag = self.terminal_selection_drag?;
        if pane_id.is_some_and(|pane_id| pane_id != drag.pane_id) {
            return None;
        }
        self.terminal_selection_drag = None;
        drag.active.then_some(drag.pane_id)
    }

    pub(crate) fn end_terminal_mouse(
        &mut self,
        pane_id: PaneId,
        button: TerminalMouseButton,
    ) -> Result<(), String> {
        if !self
            .terminal_mouse_capture
            .is_some_and(|capture| capture.pane_id == pane_id && capture.button == button)
        {
            return Ok(());
        }
        self.terminal_mouse_capture = None;
        let position = self
            .terminal_pointer_positions
            .get(&pane_id)
            .copied()
            .unwrap_or(Point::ORIGIN);
        let event = terminal_mouse_event(
            position,
            TerminalMouseAction::Release,
            Some(button),
            self.keyboard_modifiers,
        );
        self.terminals
            .get(&pane_id)
            .ok_or_else(|| format!("pane {pane_id:?} has no terminal runtime"))?
            .mouse(event)
    }

    pub(crate) fn release_terminal_mouse_capture(&mut self) -> Result<(), String> {
        let Some(capture) = self.terminal_mouse_capture else {
            return Ok(());
        };
        self.end_terminal_mouse(capture.pane_id, capture.button)
    }

    pub(crate) fn scroll_terminal_to(
        &mut self,
        pane_id: PaneId,
        offset: u64,
    ) -> Result<(), String> {
        self.focus_pane(pane_id)?;
        let row = usize::try_from(offset)
            .map_err(|_| "terminal scroll position exceeds this platform's range".to_owned())?;
        self.terminals
            .get(&pane_id)
            .ok_or_else(|| format!("pane {pane_id:?} has no terminal runtime"))?
            .scroll_to(row)
    }

    pub(crate) fn begin_terminal_scroll(&mut self, pane_id: PaneId) -> Result<(), String> {
        let runtime = self
            .terminals
            .get(&pane_id)
            .ok_or_else(|| format!("pane {pane_id:?} has no terminal runtime"))?;
        let snapshot = runtime
            .snapshot
            .as_ref()
            .ok_or_else(|| "terminal scrollback is not ready yet".to_owned())?;
        let viewport_height = runtime.viewport.map_or(0.0, |viewport| viewport.height);
        let geometry = terminal_scrollbar_geometry(snapshot.scrollbar, viewport_height);
        if geometry.max_offset == 0 {
            return Ok(());
        }
        let local = self
            .terminal_scrollbar_positions
            .get(&pane_id)
            .copied()
            .unwrap_or(Point::new(0.0, geometry.track_top + geometry.thumb_top));
        let thumb_start = geometry.track_top + geometry.thumb_top;
        let thumb_end = thumb_start + geometry.thumb_height;
        let grab_offset = if (thumb_start..=thumb_end).contains(&local.y) {
            local.y - thumb_start
        } else {
            geometry.thumb_height / 2.0
        };
        self.terminal_scroll_drag = Some(TerminalScrollDrag {
            pane_id,
            pane_top: self.cursor_position.y - local.y,
            grab_offset,
            track_height: viewport_height,
            last_offset: snapshot.scrollbar.offset,
        });
        self.update_terminal_scroll_drag(self.cursor_position)
    }

    pub(crate) fn update_terminal_scroll_drag(&mut self, position: Point) -> Result<(), String> {
        let Some(drag) = self.terminal_scroll_drag else {
            return Ok(());
        };
        let scrollbar = self
            .terminals
            .get(&drag.pane_id)
            .and_then(|runtime| runtime.snapshot.as_ref())
            .map(|snapshot| snapshot.scrollbar)
            .ok_or_else(|| "terminal scrollback is no longer available".to_owned())?;
        let geometry = terminal_scrollbar_geometry(scrollbar, drag.track_height);
        let thumb_top = position.y - drag.pane_top - geometry.track_top - drag.grab_offset;
        let target = geometry.offset_for_thumb_top(thumb_top);
        if target == drag.last_offset {
            return Ok(());
        }
        self.scroll_terminal_to(drag.pane_id, target)?;
        if let Some(active) = &mut self.terminal_scroll_drag {
            active.last_offset = target;
        }
        Ok(())
    }

    pub(crate) fn begin_split_drag(
        &mut self,
        key: SplitKey,
        axis: SplitAxis,
    ) -> Result<(), String> {
        let tab_id = key.tab_id;
        let size = self
            .split_sizes
            .get(&key)
            .copied()
            .ok_or_else(|| "Split handle is not ready yet".to_owned())?;
        let workspace = self
            .session
            .workspaces
            .iter()
            .find(|workspace| workspace.id == key.workspace_id)
            .ok_or_else(|| "Split workspace is no longer available".to_owned())?;
        let tab = workspace
            .tab(key.tab_id)
            .ok_or_else(|| "Split tab is no longer available".to_owned())?;
        let start_ratio = split_ratio_at(&tab.root, &key.path)
            .ok_or_else(|| "Split is no longer available".to_owned())?
            .permille();
        let extent = match axis {
            SplitAxis::Horizontal => size.width,
            SplitAxis::Vertical => size.height,
        };
        if extent <= SPLIT_HANDLE_SIZE {
            return Err("Split is too small to resize".into());
        }
        let start_coordinate = match axis {
            SplitAxis::Horizontal => self.cursor_position.x,
            SplitAxis::Vertical => self.cursor_position.y,
        };
        self.split_drag = Some(SplitDrag {
            key,
            axis,
            start_coordinate,
            start_ratio,
            extent,
        });
        self.pane_layouts.remove(&tab_id);
        self.base_pane_layouts.remove(&tab_id);
        self.pane_resize_history.remove(&tab_id);
        Ok(())
    }

    pub(crate) fn update_split_drag(&mut self, position: Point) -> Result<(), String> {
        let Some(drag) = self.split_drag.clone() else {
            return Ok(());
        };
        let coordinate = match drag.axis {
            SplitAxis::Horizontal => position.x,
            SplitAxis::Vertical => position.y,
        };
        let delta = coordinate - drag.start_coordinate;
        let permille = (f32::from(drag.start_ratio) + delta / drag.extent * 1_000.0)
            .round()
            .clamp(f32::from(SplitRatio::MIN), f32::from(SplitRatio::MAX))
            as u16;
        let ratio = SplitRatio::new(permille).map_err(|error| error.to_string())?;
        let workspace = self
            .session
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == drag.key.workspace_id)
            .ok_or_else(|| "Split workspace is no longer available".to_owned())?;
        let tab = workspace
            .tab_mut(drag.key.tab_id)
            .ok_or_else(|| "Split tab is no longer available".to_owned())?;
        if set_split_ratio_at(&mut tab.root, &drag.key.path, ratio) {
            Ok(())
        } else {
            Err("Split is no longer available".into())
        }
    }

    pub(crate) fn poll_terminal(&mut self) {
        self.drain_terminal_launches();
        let mut notifications = Vec::new();
        let mut exited = Vec::new();
        let mut titles = Vec::new();
        for (pane_id, runtime) in &mut self.terminals {
            let poll = runtime.poll();
            if let Some(status) = poll.status {
                self.status = status;
            }
            notifications.extend(
                poll.notifications
                    .into_iter()
                    .map(|notification| (*pane_id, notification)),
            );
            if poll.exited {
                exited.push((*pane_id, poll.exited_clean));
            }
            if let Some(title) = poll.title {
                titles.push((*pane_id, title));
            }
        }
        for (pane_id, title) in titles {
            if let Some(status) = self.agent_statuses.get_mut(&pane_id)
                && let Some(title) = harness_terminal_title(&title, &status.agent)
            {
                status.display_name = Some(title);
            }
            if let Some(surface) = self
                .session
                .workspaces
                .iter_mut()
                .find_map(|workspace| workspace.pane_mut(pane_id))
                .and_then(|pane| {
                    pane.surfaces
                        .iter_mut()
                        .find(|surface| surface.id == pane.active_surface_id)
                })
            {
                surface.title = title;
            }
        }
        // Schema-v3 sessions created before durable pane identity (and any
        // pane whose hook never reached this process) can still recover from
        // agent-specific chrome in the replayed live screen. This is the
        // fallback beneath persisted identity and platform process scanning.
        let recoveries = self
            .terminals
            .iter()
            .filter(|(pane_id, _)| !self.agent_statuses.contains_key(pane_id))
            .filter_map(|(pane_id, runtime)| {
                let snapshot = runtime.snapshot.as_ref()?;
                let identification = agent_screen::identify(snapshot)?;
                let display_name = snapshot
                    .title
                    .as_deref()
                    .and_then(|title| harness_terminal_title(title, identification.agent))
                    .or_else(|| self.agent_worktree_name(*pane_id));
                Some((*pane_id, identification, display_name))
            })
            .collect::<Vec<_>>();
        for (pane_id, identification, display_name) in recoveries {
            let screen = identification
                .classification
                .map_or(agent_screen::ScreenState::Idle, |classification| {
                    classification.state
                });
            self.agent_statuses.insert(
                pane_id,
                AgentPaneStatus {
                    agent: identification.agent.into(),
                    display_name,
                    state: screen_state(screen),
                    activity: Some(agent_state_activity(screen).into()),
                    session_id: None,
                    cwd: None,
                    git_branch: None,
                },
            );
        }
        // A pane changes hands when a different agent starts in it — Pi
        // launched into a worktree and later replaced by Claude Code, or an
        // identity restored from a session that predates the switch. Nothing
        // downstream can recover on its own: the old agent's rules cannot
        // read the new agent's frames, and the new agent's hooks are refused
        // as strays from a descendant shell. Only positive evidence moves
        // identity — the frame must carry the new agent's chrome and none of
        // the current agent's, so a nested tool run inside a live agent
        // (whose own chrome stays on screen) never re-labels the pane.
        let handovers = self
            .agent_statuses
            .iter()
            .filter_map(|(pane_id, status)| {
                let snapshot = self.terminals.get(pane_id)?.snapshot.as_ref()?;
                let identification = agent_screen::identify(snapshot)?;
                if pane_agent(identification.agent) == pane_agent(&status.agent)
                    || agent_screen::carries_signature(&status.agent, snapshot)
                {
                    return None;
                }
                let display_name = snapshot
                    .title
                    .as_deref()
                    .and_then(|title| harness_terminal_title(title, identification.agent))
                    .or_else(|| self.agent_worktree_name(*pane_id));
                Some((*pane_id, identification, display_name))
            })
            .collect::<Vec<_>>();
        for (pane_id, identification, display_name) in handovers {
            self.hand_over_agent_pane(pane_id, identification, display_name);
        }
        // Re-evaluate the retained latest frame as well as newly received
        // frames. Agent identity may arrive from a hook just after the TUI
        // painted a stable prompt, with no reason for another repaint.
        let classifications = self
            .agent_statuses
            .iter()
            .filter_map(|(pane_id, status)| {
                self.terminals
                    .get(pane_id)
                    .and_then(|runtime| {
                        runtime
                            .snapshot
                            .as_ref()
                            .and_then(|frame| agent_screen::classify(&status.agent, frame))
                            .map(|classification| (runtime.snapshot_revision, classification))
                    })
                    .map(|(revision, classification)| {
                        (*pane_id, status.agent.clone(), revision, classification)
                    })
            })
            .collect::<Vec<_>>();
        for (pane_id, agent, revision, classification) in classifications {
            self.apply_agent_screen_classification(pane_id, &agent, revision, classification);
        }
        // Which panes are projecting Claude Code's roster. Derived from the
        // same frames as the classification above, so entering or leaving the
        // view is reflected on the very next paint rather than on a poll.
        let showing_roster = self
            .agent_statuses
            .iter()
            .filter(|(pane_id, status)| {
                self.terminals.get(pane_id).is_some_and(|runtime| {
                    runtime
                        .snapshot
                        .as_ref()
                        .is_some_and(|frame| agent_screen::agents_view(&status.agent, frame))
                })
            })
            .map(|(pane_id, _)| *pane_id)
            .collect::<BTreeSet<_>>();
        if showing_roster != self.agents_view_panes {
            self.agents_view_panes = showing_roster;
            // A view change is exactly when the roll-up must be re-read, so
            // the row never shows the previous view's counts.
            self.agents_roster_checked = None;
        }
        for (pane_id, notification) in notifications {
            self.record_notification(pane_id, notification);
        }
        for (pane_id, clean) in exited {
            // A pane whose process the host never started did not exit: it
            // never ran. Saying so in place beats an empty terminal that
            // looks live and answers nothing.
            if let Some(error) = self.terminal_launcher.spawn_failure(pane_id) {
                self.mark_terminal_launch_failed(pane_id, error);
                continue;
            }
            self.agent_statuses.remove(&pane_id);
            self.agent_running_frame_revisions.remove(&pane_id);
            self.pi_active_lifecycles.remove(&pane_id);
            self.detected_agents.remove(&pane_id);
            self.agents_view_panes.remove(&pane_id);
            self.terminal_command_buffers.remove(&pane_id);
            // A clean exit closes its pane, cascading exactly like a manual
            // close: last pane closes the tab, the workspace's final tab
            // raises the close-workspace confirmation instead of vanishing.
            // Unclean or unknown exits keep the pane so its output — likely
            // an error — stays readable, with Restart available.
            if clean {
                let _ = self.close_pane(pane_id);
            }
        }
    }

    /// Reads the machine-wide roster while at least one pane projects it.
    ///
    /// The read costs a short-lived subprocess, so it is paced rather than run
    /// per frame, and only one may be in flight. Entering the view clears the
    /// timestamp, making that first read immediate.
    pub(crate) fn refresh_agents_roster(&mut self) -> Vec<Effect> {
        if self.agents_view_panes.is_empty() {
            // Nothing projects it any more; drop it so a later view cannot
            // paint a stale count before its own read lands.
            self.agents_roster = None;
            self.agents_roster_error = None;
            self.agents_roster_checked = None;
            return Vec::new();
        }
        let due = self
            .agents_roster_checked
            .is_none_or(|read| read.elapsed() >= AGENTS_ROSTER_INTERVAL);
        if self.agents_roster_pending || !due {
            return Vec::new();
        }
        self.agents_roster_pending = true;
        self.agents_roster_checked = Some(std::time::Instant::now());
        let command = self.settings.claude_command.clone();
        perform_blocking(
            move || agents_roster::load(&command),
            |result| Message::AgentsRosterLoaded(result.and_then(|roster| roster)),
        )
    }

    pub(crate) fn refresh_background_metadata(&mut self) -> Vec<Effect> {
        let roster = self.refresh_agents_roster();
        let repositories = self.refresh_pane_repositories();
        effect::batch([roster, repositories])
    }

    /// Resolves repository, branch, HEAD, and linked pull-request identity for
    /// each fleet pane. Every subprocess runs off the UI thread and each pane
    /// probes independently so one slow remote cannot hold the fleet hostage.
    pub(crate) fn refresh_pane_repositories(&mut self) -> Vec<Effect> {
        let live_panes = self
            .session
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.tabs.iter())
            .flat_map(|tab| tab.root.pane_ids())
            .collect::<BTreeSet<_>>();
        self.pane_repositories
            .retain(|pane_id, _| live_panes.contains(pane_id));
        self.pending_repository_directories
            .retain(|pane_id, _| live_panes.contains(pane_id));

        let pane_ids = self
            .fleet_entries_in_tab_order()
            .into_iter()
            .map(|(_, pane_id)| pane_id)
            .collect::<Vec<_>>();
        let mut probes = Vec::new();
        for pane_id in pane_ids {
            let Some(directory) = self
                .pane_working_directory(pane_id)
                .filter(|directory| reported_path_is_concrete(directory))
            else {
                self.pane_repositories.remove(&pane_id);
                self.pending_repository_directories.remove(&pane_id);
                continue;
            };
            let reported_branch = self
                .agent_statuses
                .get(&pane_id)
                .and_then(|status| status.git_branch.as_deref());
            if self
                .pane_repositories
                .get(&pane_id)
                .is_some_and(|repository| {
                    !pane_repository_pull_request_is_relevant(
                        repository,
                        &directory,
                        reported_branch,
                    )
                })
                && let Some(repository) = self.pane_repositories.get_mut(&pane_id)
            {
                // Branch and worktree changes make the old PR false
                // immediately. Repository grouping may stay until the
                // replacement probe lands, but its stale action may not.
                repository.pull_request = None;
            }
            let cached = self
                .pane_repositories
                .get(&pane_id)
                .is_some_and(|repository| {
                    repository.directory == directory
                        && repository.checked_at.elapsed() < PANE_REPOSITORY_INTERVAL
                        // A hook that reports a *different* branch than the one
                        // the cached probe answered means the pane moved, so the
                        // entry is stale ahead of its interval. Comparing the
                        // report against git's own answer instead would never
                        // settle when the two disagree — an unreachable branch,
                        // a probe that failed, a detached HEAD — and this runs
                        // on every terminal event, so a permanent disagreement
                        // becomes an unbounded probe loop, one that costs six
                        // console subprocesses per pane per turn of it.
                        && repository.reported_branch.as_deref()
                            == self
                                .agent_statuses
                                .get(&pane_id)
                                .and_then(|status| status.git_branch.as_deref())
                });
            let pending = self.pending_repository_directories.get(&pane_id) == Some(&directory);
            if !cached && !pending {
                self.pending_repository_directories
                    .insert(pane_id, directory.clone());
                probes.push((pane_id, directory));
            }
        }
        if probes.is_empty() {
            return Vec::new();
        }
        for (pane_id, directory) in self.pending_repository_directories.clone() {
            if live_panes.contains(&pane_id)
                && !probes.iter().any(|(candidate, _)| *candidate == pane_id)
                && self
                    .pane_repositories
                    .get(&pane_id)
                    .is_none_or(|repository| repository.directory != directory)
            {
                probes.push((pane_id, directory));
            }
        }
        self.pending_repository_directories.clear();
        self.pending_repository_directories
            .extend(probes.iter().cloned());

        self.pane_repository_generation = self.pane_repository_generation.wrapping_add(1);
        let generation = self.pane_repository_generation;
        self.pane_repository_cancellation.cancel();
        self.pane_repository_cancellation = ProcessCancellation::default();
        let authenticated = matches!(self.github_auth, github::AuthStatus::Authenticated { .. });
        let wsl_distribution = self.settings.wsl_distribution.clone();
        let github_host = self.settings.github_host.clone();
        effect::batch(probes.into_iter().map(|(pane_id, directory)| {
            // Recorded with the answer so the next pass can tell "the hook has
            // since reported a move" apart from "git and the hook disagree".
            let reported_branch = self
                .agent_statuses
                .get(&pane_id)
                .and_then(|status| status.git_branch.clone());
            let cached_pull_request = self
                .pane_repositories
                .get(&pane_id)
                .and_then(|repository| repository.pull_request.clone());
            let cancellation = self.pane_repository_cancellation.clone();
            let wsl_distribution = wsl_distribution.clone();
            let github_host = github_host.clone();
            perform_blocking(
                move || {
                    let worktree_name = linked_worktree_name(&directory);
                    let repository = github::repository_from(
                        &directory,
                        &wsl_distribution,
                        &github_host,
                        &cancellation,
                    )
                    .ok();
                    let name = repository.as_ref().and_then(|repository| {
                        git_repository_name_from_root_cancellable(
                            &repository.root,
                            &wsl_distribution,
                            &cancellation,
                        )
                    });
                    let pull_request = if authenticated {
                        match repository.as_ref() {
                            Some(repository) => current_pull_request_after_refresh(
                                cached_pull_request,
                                github::current_pull_request(repository, &cancellation),
                            ),
                            None => cached_pull_request,
                        }
                    } else {
                        None
                    };
                    vec![(
                        pane_id,
                        PaneRepository {
                            directory,
                            root: repository
                                .as_ref()
                                .map(|repository| repository.root.clone()),
                            name,
                            worktree_name,
                            branch: repository
                                .as_ref()
                                .map(|repository| repository.branch.clone()),
                            reported_branch,
                            head_oid: repository
                                .as_ref()
                                .map(|repository| repository.head_oid.clone()),
                            pull_request,
                            checked_at: std::time::Instant::now(),
                        },
                    )]
                },
                move |result| Message::PaneRepositoriesLoaded(generation, result),
            )
        }))
    }

    pub(crate) fn control_pane_ids(session: &SessionState) -> Vec<String> {
        session
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.tabs.iter())
            .flat_map(|tab| tab.panes.keys())
            .map(|pane_id| pane_id.as_uuid().to_string())
            .collect()
    }

    pub(crate) fn publish_control_panes(&self) -> Result<(), String> {
        if let Some(control) = &self.control {
            control
                .publish_panes(Self::control_pane_ids(&self.session))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(crate) fn bind_control_to_session(
        &mut self,
        session_id: uuid::Uuid,
        panes: &[String],
    ) -> Result<(), String> {
        if std::env::var_os("MUXTRIX_CONTROL_ENDPOINT").is_none() {
            let endpoint = Endpoint::for_instance(&format!("session-{session_id}"))
                .map_err(|error| error.to_string())?;
            let server =
                ControlServer::bind_with_notifier(endpoint, Arc::clone(&self.event_notifier))
                    .map_err(|error| error.to_string())?;
            server
                .publish_panes(panes.iter().cloned())
                .map_err(|error| error.to_string())?;
            self.control_endpoint = Some(server.endpoint_environment_value().to_owned());
            self.control = Some(server);
        } else {
            self.control
                .as_ref()
                .ok_or_else(|| "The configured local control endpoint is unavailable".to_owned())?
                .publish_panes(panes.iter().cloned())
                .map_err(|error| error.to_string())?;
        }
        self.global_alerts
            .retain(|alert| alert.title != "Local control unavailable");
        Ok(())
    }

    pub(crate) fn poll_control(&mut self) {
        let _ = self.publish_control_panes();
        let mut incoming = Vec::new();
        if let Some(control) = &self.control {
            while let Ok(request) = control.try_recv() {
                incoming.push(request);
            }
        }
        for request in incoming {
            let response = self.handle_control_request(request.request.clone());
            request.respond(response);
        }
    }

    pub(crate) fn handle_control_request(&mut self, request: ControlRequest) -> ControlResponse {
        match request {
            ControlRequest::Ping => ControlResponse::success("pong"),
            ControlRequest::Notify {
                title,
                body,
                pane_id,
            } => match self.control_pane_id(pane_id.as_deref()) {
                Ok(pane_id) => {
                    self.record_notification(pane_id, TerminalNotification { title, body });
                    ControlResponse::success("notification recorded")
                }
                Err(error) => ControlResponse::error(error),
            },
            ControlRequest::AgentEvent {
                agent,
                state,
                event,
                title,
                body,
                pane_id,
                session_id,
                cwd,
            } => match self.control_agent_pane_id(pane_id.as_deref()) {
                Ok(pane_id) => {
                    // A pane actively running agent X cannot be the origin
                    // of agent Y's lifecycle events — that is a stray
                    // MUXTRIX_PANE_ID inherited by a process outside the
                    // pane (an agent launched from a descendant shell).
                    // Applying it would overwrite or demote a live agent
                    // mid-run.
                    if let Some(current) = self.agent_statuses.get(&pane_id)
                        && current.agent != agent
                        && !matches!(
                            current.state,
                            AgentState::Completed | AgentState::Failed | AgentState::Stopped
                        )
                    {
                        return ControlResponse::success(
                            "event names a different agent than the pane is running; ignored",
                        );
                    }
                    if !should_accept_agent_state(
                        self.agent_statuses.get(&pane_id),
                        state,
                        session_id.as_deref(),
                    ) {
                        return ControlResponse::success("stale agent lifecycle state ignored");
                    }
                    let is_pi = pane_agent(&agent) == Some(PaneAgent::OhMyPi);
                    // Pi's managed extension brackets active work with
                    // `agent_start` and terminal `agent_end`. Maintenance
                    // completion used to be reported as Completed by older
                    // extension versions even when it ran inside that bracket;
                    // preserve the active run for those already-installed
                    // modules while automatic migration replaces them.
                    let state = if is_pi
                        && self.pi_active_lifecycles.contains(&pane_id)
                        && state == AgentState::Completed
                        && matches!(
                            event.as_deref(),
                            Some("session_compact" | "auto_compaction_end")
                        ) {
                        AgentState::Running
                    } else {
                        state
                    };
                    if is_pi {
                        match (event.as_deref(), state) {
                            (
                                Some(
                                    "agent_start"
                                    | "tool_approval_requested"
                                    | "tool_approval_resolved",
                                ),
                                _,
                            )
                            | (Some("agent_end"), AgentState::Running) => {
                                self.pi_active_lifecycles.insert(pane_id);
                            }
                            (
                                Some(
                                    "agent_end" | "session_start" | "session_switch"
                                    | "session_branch" | "session_shutdown",
                                ),
                                _,
                            ) => {
                                self.pi_active_lifecycles.remove(&pane_id);
                            }
                            _ => {}
                        }
                    }
                    // Codex PermissionRequest runs before its automatic
                    // reviewer, and Claude notifications are similarly not
                    // proof that a person is required. Those two agents may
                    // enter Waiting only from positive evidence in a live
                    // terminal frame. PostToolUse is also too late and can be
                    // unrelated under parallel tool use, so it cannot clear a
                    // screen-confirmed prompt. Pi approval events are exact.
                    let screen_confirmed_wait =
                        agent_screen::requires_screen_confirmed_wait(&agent);
                    let advisory_wait = screen_confirmed_wait && state == AgentState::Waiting;
                    let post_tool_cannot_clear_wait = screen_confirmed_wait
                        && state == AgentState::Running
                        && event.as_deref() == Some("PostToolUse")
                        && self
                            .agent_statuses
                            .get(&pane_id)
                            .is_some_and(|current| current.state == AgentState::Waiting);
                    let git_branch = git_branch_for_directory(cwd.as_deref());
                    let display_name = self
                        .agent_statuses
                        .get(&pane_id)
                        .and_then(|status| status.display_name.clone())
                        .or_else(|| {
                            cwd.as_deref()
                                .and_then(|cwd| linked_worktree_name(std::path::Path::new(cwd)))
                        });
                    let frame_revision = self
                        .terminals
                        .get(&pane_id)
                        .map_or(0, |runtime| runtime.snapshot_revision);
                    if advisory_wait || post_tool_cannot_clear_wait {
                        if let Some(current) = self.agent_statuses.get_mut(&pane_id) {
                            if session_id.is_some() {
                                current.session_id = session_id;
                            }
                            if cwd.is_some() {
                                current.cwd = cwd;
                                current.git_branch = git_branch;
                            }
                            if current.state != AgentState::Waiting && !body.trim().is_empty() {
                                current.activity = Some(body.clone());
                            }
                        } else {
                            self.agent_statuses.insert(
                                pane_id,
                                AgentPaneStatus {
                                    agent: agent.clone(),
                                    display_name,
                                    state: AgentState::Running,
                                    activity: (!body.trim().is_empty()).then(|| body.clone()),
                                    session_id,
                                    cwd,
                                    git_branch,
                                },
                            );
                            self.agent_running_frame_revisions
                                .insert(pane_id, frame_revision);
                        }
                        return ControlResponse::success(
                            "agent lifecycle metadata updated; live screen retains state authority",
                        );
                    }
                    if state == AgentState::Stopped {
                        self.agent_statuses.remove(&pane_id);
                        self.agent_running_frame_revisions.remove(&pane_id);
                        self.pi_active_lifecycles.remove(&pane_id);
                        self.terminal_command_buffers.remove(&pane_id);
                        self.detected_agents.remove(&pane_id);
                    } else {
                        self.agent_statuses.insert(
                            pane_id,
                            AgentPaneStatus {
                                agent: agent.clone(),
                                display_name,
                                state,
                                activity: (!body.trim().is_empty()).then(|| body.clone()),
                                session_id,
                                cwd,
                                git_branch,
                            },
                        );
                        if state == AgentState::Running {
                            self.agent_running_frame_revisions
                                .insert(pane_id, frame_revision);
                        } else {
                            self.agent_running_frame_revisions.remove(&pane_id);
                        }
                    }
                    match state {
                        AgentState::Waiting | AgentState::Failed => {
                            self.record_notification(pane_id, TerminalNotification { title, body });
                        }
                        AgentState::Completed => {
                            // Completion is useful lifecycle information, not
                            // a request for the user. It also resolves any
                            // attention left by an earlier waiting state.
                            self.clear_pane_attention(pane_id);
                            if agent_event_completes_turn(state, event.as_deref()) {
                                self.queue_github_pull_request_refresh(pane_id);
                            }
                        }
                        AgentState::Idle | AgentState::Running | AgentState::Stopped => {}
                    }
                    ControlResponse::success("agent lifecycle state updated")
                }
                Err(error) => ControlResponse::error(error),
            },
            ControlRequest::LaunchAgent { agent } => match self.launch_agent(agent) {
                Ok(()) => ControlResponse::success(format!("launched {agent}")),
                Err(error) => ControlResponse::error(error),
            },
            ControlRequest::Split { direction } => {
                let axis = match direction {
                    SplitDirection::Right => SplitAxis::Horizontal,
                    SplitDirection::Down => SplitAxis::Vertical,
                };
                match self.split_terminal(axis) {
                    Ok(()) => ControlResponse::success("terminal pane created"),
                    Err(error) => ControlResponse::error(error),
                }
            }
            ControlRequest::Focus { pane_id } => match self.control_pane_id(Some(&pane_id)) {
                Ok(pane_id) => match self.focus_pane(pane_id) {
                    Ok(()) => ControlResponse::success("pane focused"),
                    Err(error) => ControlResponse::error(error),
                },
                Err(error) => ControlResponse::error(error),
            },
            ControlRequest::Close { pane_id } => match self.control_pane_id(pane_id.as_deref()) {
                Ok(pane_id) => match self.close_pane(pane_id) {
                    Ok(()) if self.close_workspace_prompt.is_some() => ControlResponse::error(
                        "closing the final tab requires workspace confirmation",
                    ),
                    Ok(()) => ControlResponse::success("terminal pane closed"),
                    Err(error) => ControlResponse::error(error),
                },
                Err(error) => ControlResponse::error(error),
            },
            ControlRequest::SendText { text, pane_id } => {
                match self.control_pane_id(pane_id.as_deref()) {
                    Ok(pane_id) => match self.send_terminal_input_to(pane_id, text.into_bytes()) {
                        Ok(()) => ControlResponse::success("text sent"),
                        Err(error) => ControlResponse::error(error),
                    },
                    Err(error) => ControlResponse::error(error),
                }
            }
            ControlRequest::Capture { pane_id } => match self.control_pane_id(pane_id.as_deref()) {
                Ok(pane_id) => self
                    .terminals
                    .get(&pane_id)
                    .and_then(|runtime| runtime.snapshot.as_ref())
                    .map_or_else(
                        || ControlResponse::error("pane has no terminal snapshot"),
                        |snapshot| ControlResponse {
                            ok: true,
                            message: None,
                            text: Some(snapshot.text()),
                            panes: Vec::new(),
                        },
                    ),
                Err(error) => ControlResponse::error(error),
            },
            ControlRequest::ListPanes => {
                let panes = self.active_workspace().map_or_else(
                    |_| Vec::new(),
                    |workspace| {
                        workspace
                            .tabs
                            .iter()
                            .flat_map(|tab| {
                                tab.panes.values().map(move |pane| PaneSummary {
                                    pane_id: pane.id.as_uuid().to_string(),
                                    title: pane.active_surface().map_or_else(
                                        || "terminal".into(),
                                        |surface| surface.title.clone(),
                                    ),
                                    focused: workspace.active_tab_id == tab.id
                                        && tab.focused_pane_id == pane.id,
                                    unread_count: pane.attention.unread_count,
                                })
                            })
                            .collect()
                    },
                );
                ControlResponse {
                    ok: true,
                    message: Some(format!("{} panes", panes.len())),
                    text: None,
                    panes,
                }
            }
        }
    }

    pub(crate) fn apply_agent_screen_classification(
        &mut self,
        pane_id: PaneId,
        agent: &str,
        frame_revision: u64,
        classification: agent_screen::Classification,
    ) {
        let Some(current) = self.agent_statuses.get(&pane_id) else {
            return;
        };
        let state = screen_state(classification.state);
        // Keep a completed turn visible while its composer is merely idle, but
        // let positive working evidence start the next turn even if its prompt
        // hook was delayed or unavailable. Failed and stopped sessions remain
        // lifecycle-owned so stale terminal chrome cannot revive them.
        if matches!(current.state, AgentState::Failed | AgentState::Stopped)
            || (current.state == AgentState::Completed && state != AgentState::Running)
        {
            return;
        }
        // Pi's exact lifecycle bracket is stronger than its correction-layer
        // title. Older Pi releases could briefly publish `π >` while an async
        // job or scheduled continuation still owned the turn; accepting that
        // title made the fleet row stay Idle for the rest of the task.
        if pane_agent(agent) == Some(PaneAgent::OhMyPi)
            && state == AgentState::Idle
            && self.pi_active_lifecycles.contains(&pane_id)
        {
            return;
        }
        // A retained idle title may predate a just-received prompt hook. Guard
        // only that exact frame: once the terminal publishes a newer idle
        // frame, it is positive evidence that the turn really ended.
        if current.state == AgentState::Running
            && state == AgentState::Idle
            && self
                .agent_running_frame_revisions
                .get(&pane_id)
                .is_some_and(|running_revision| frame_revision == *running_revision)
        {
            return;
        }
        if current.state == state {
            return;
        }
        let resolved_waiting = current.state == AgentState::Waiting && state != AgentState::Waiting;
        let activity = agent_state_activity(classification.state);
        if let Some(current) = self.agent_statuses.get_mut(&pane_id) {
            current.state = state;
            current.activity = Some(activity.into());
        }
        if state == AgentState::Running {
            self.agent_running_frame_revisions
                .insert(pane_id, frame_revision);
        } else {
            self.agent_running_frame_revisions.remove(&pane_id);
        }
        if resolved_waiting {
            self.clear_pane_attention(pane_id);
        }
        if state == AgentState::Waiting {
            self.record_notification(
                pane_id,
                TerminalNotification {
                    title: agent_display_name(agent).into(),
                    body: activity.into(),
                },
            );
        }
    }

    /// Re-labels a pane whose live frame belongs to a different agent than
    /// its status names. The previous agent's lifecycle bookkeeping goes with
    /// it; the pane keeps its directory context, which describes the pane
    /// rather than the agent, until the new agent's own hooks refresh it.
    pub(crate) fn hand_over_agent_pane(
        &mut self,
        pane_id: PaneId,
        identification: agent_screen::Identification,
        display_name: Option<String>,
    ) {
        let Some(previous) = self.agent_statuses.get(&pane_id) else {
            return;
        };
        let cwd = previous.cwd.clone();
        let git_branch = previous.git_branch.clone();
        let screen = identification
            .classification
            .map_or(agent_screen::ScreenState::Idle, |classification| {
                classification.state
            });
        let state = screen_state(screen);
        let frame_revision = self
            .terminals
            .get(&pane_id)
            .map_or(0, |runtime| runtime.snapshot_revision);
        self.agent_statuses.insert(
            pane_id,
            AgentPaneStatus {
                agent: identification.agent.into(),
                display_name,
                state,
                activity: Some(agent_state_activity(screen).into()),
                session_id: None,
                cwd,
                git_branch,
            },
        );
        self.pi_active_lifecycles.remove(&pane_id);
        if state == AgentState::Running {
            self.agent_running_frame_revisions
                .insert(pane_id, frame_revision);
        } else {
            self.agent_running_frame_revisions.remove(&pane_id);
        }
        if state != AgentState::Waiting {
            self.clear_pane_attention(pane_id);
        }
    }

    pub(crate) fn control_pane_id(&self, pane_id: Option<&str>) -> Result<PaneId, String> {
        let workspace = self.active_workspace()?;
        match pane_id {
            None => workspace
                .active_tab()
                .map(|tab| tab.focused_pane_id)
                .ok_or_else(|| "active tab is missing".to_owned()),
            Some(value) => self
                .session
                .workspaces
                .iter()
                .flat_map(Workspace::all_pane_ids)
                .find(|pane_id| pane_id.as_uuid().to_string() == value)
                .ok_or_else(|| format!("pane {value} was not found")),
        }
    }

    pub(crate) fn control_agent_pane_id(&self, pane_id: Option<&str>) -> Result<PaneId, String> {
        let pane_id = pane_id.ok_or_else(|| {
            "agent lifecycle event has no Muxtrix pane identity; event ignored".to_owned()
        })?;
        self.control_pane_id(Some(pane_id))
    }

    pub(crate) fn focus_pane(&mut self, pane_id: PaneId) -> Result<(), String> {
        if let Some(panel) = self.github_panel.as_mut() {
            panel.keyboard_focus = None;
        }
        let Some((workspace_id, tab_id)) = self.session.workspaces.iter().find_map(|workspace| {
            workspace
                .tab_containing_pane(pane_id)
                .map(|tab| (workspace.id, tab.id))
        }) else {
            return Err(format!("pane {pane_id:?} was not found"));
        };
        if workspace_id != self.session.active_workspace_id {
            self.switch_workspace(workspace_id)?;
        }
        let focus_changed = {
            let workspace = self.active_workspace_mut()?;
            workspace
                .switch_tab(tab_id)
                .map_err(|error| error.to_string())?;
            let tab = workspace
                .tab_mut(tab_id)
                .ok_or_else(|| format!("tab {tab_id:?} was not found"))?;
            if !tab.panes.contains_key(&pane_id) {
                return Err(format!("pane {pane_id:?} was not found"));
            }
            let focus_changed = tab.focused_pane_id != pane_id;
            tab.focused_pane_id = pane_id;
            focus_changed
        };
        if focus_changed {
            self.pane_resize_history.remove(&tab_id);
            if self.pane_working_directory(pane_id).is_some() {
                self.queue_github_pane_refresh();
            }
        }
        self.clear_pane_attention(pane_id);
        Ok(())
    }

    pub(crate) fn clear_pane_attention(&mut self, pane_id: PaneId) {
        if let Some(pane) = self
            .session
            .workspaces
            .iter_mut()
            .find_map(|workspace| workspace.pane_mut(pane_id))
        {
            pane.attention = Default::default();
        }
        for notification in &mut self.notifications {
            if notification.pane_id == pane_id {
                notification.unread = false;
            }
        }
    }

    pub(crate) fn record_notification(
        &mut self,
        pane_id: PaneId,
        notification: TerminalNotification,
    ) {
        let focused = self.active_workspace().is_ok_and(|workspace| {
            workspace
                .active_tab()
                .is_some_and(|tab| tab.focused_pane_id == pane_id)
        }) && self.active_view == ActiveView::Workspace;
        if !focused
            && let Some(pane) = self
                .session
                .workspaces
                .iter_mut()
                .find_map(|workspace| workspace.pane_mut(pane_id))
        {
            pane.attention.unread_count = pane.attention.unread_count.saturating_add(1);
            pane.attention.message = Some(notification.body.clone());
        }
        self.notifications.push(AgentNotification {
            pane_id,
            unread: !focused,
        });
        if self.notifications.len() > 100 {
            self.notifications.remove(0);
        }
    }

    /// Applies the fleet projection everywhere it is read and persists it as
    /// a durable preference without routing through the settings screen.
    pub(crate) fn set_fleet_view(&mut self, view: FleetView) {
        self.settings.fleet_view = view;
        self.settings_draft.fleet_view = view;
        // Unit tests run without a config-path override and must never touch
        // the user's real settings file.
        #[cfg(not(test))]
        {
            let _ = self.settings.save();
        }
    }

    /// Applies and persists which workspaces feed the fleet.
    pub(crate) fn set_fleet_scope(&mut self, scope: FleetScope) {
        self.settings.fleet_scope = scope;
        self.settings_draft.fleet_scope = scope;
        #[cfg(not(test))]
        {
            let _ = self.settings.save();
        }
    }

    /// The fleet projection the rail actually renders. The collapsed rail is
    /// pure navigation with no reachable toggle, so it always lists every
    /// pane without projection grouping.
    pub(crate) fn effective_fleet_view(&self) -> FleetView {
        if self.sidebar_is_compact() {
            FleetView::Tabs
        } else {
            self.settings.fleet_view
        }
    }

    /// The launch profile behind a plain terminal pane.
    pub(crate) fn pane_profile(&self, pane_id: PaneId) -> Option<&LaunchProfile> {
        self.session
            .workspaces
            .iter()
            .find_map(|workspace| workspace.pane(pane_id))
            .and_then(Pane::active_surface)
            .and_then(|surface| match &surface.kind {
                muxtrix_domain::SurfaceKind::Terminal(terminal) => self
                    .session
                    .profiles
                    .iter()
                    .find(|profile| profile.id == terminal.profile_id),
                _ => None,
            })
    }

    /// The program a plain terminal pane was launched with, as a basename.
    ///
    /// `None` when the backend supplies its own shell rather than being handed
    /// a program: a WSL profile runs the distribution's login shell, so naming
    /// it in the header would only repeat the profile ("WSL shell") in a place
    /// that is meant to say what is running.
    pub(crate) fn pane_program(&self, pane_id: PaneId) -> Option<String> {
        let profile = self.pane_profile(pane_id)?;
        let program = profile
            .program
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default();
        (!program.is_empty()).then(|| program.to_owned())
    }

    /// The command a plain terminal pane was launched with — the program's
    /// basename, or the profile name when the backend has no program. Copy
    /// with room for it (tooltips) still wants the profile as a last resort.
    pub(crate) fn pane_command(&self, pane_id: PaneId) -> String {
        self.pane_program(pane_id)
            .or_else(|| {
                self.pane_profile(pane_id)
                    .map(|profile| profile.name.clone())
            })
            .unwrap_or_default()
    }

    pub(crate) fn pane_location_label(&self, pane_id: PaneId) -> String {
        let Some(directory) = self.pane_working_directory(pane_id) else {
            return "No directory".into();
        };
        if let Some(repository) = self
            .pane_repositories
            .get(&pane_id)
            .filter(|repository| repository.directory == directory)
        {
            return repository
                .worktree_name
                .as_deref()
                .or(repository.name.as_deref())
                .map(str::to_owned)
                .unwrap_or_else(|| directory.display().to_string());
        }
        let path = directory.display().to_string();
        if path.is_empty() {
            "No directory".into()
        } else {
            path
        }
    }

    pub(crate) fn pane_title<'a>(&'a self, workspace: &'a Workspace, pane_id: PaneId) -> &'a str {
        if let Some(name) = workspace
            .pane(pane_id)
            .and_then(|pane| pane.custom_name.as_deref())
        {
            return name;
        }
        self.agent_statuses.get(&pane_id).map_or_else(
            || {
                workspace
                    .pane(pane_id)
                    .and_then(|pane| pane.active_surface())
                    .map_or("terminal", |surface| surface.title.as_str())
            },
            |status| {
                status
                    .display_name
                    .as_deref()
                    .unwrap_or_else(|| agent_display_name(&status.agent))
            },
        )
    }

    pub(crate) fn fleet_pane_identity_label(
        &self,
        workspace: &Workspace,
        pane_id: PaneId,
        location: &str,
    ) -> String {
        let title = self.pane_title(workspace, pane_id);
        let has_custom_name = workspace
            .pane(pane_id)
            .is_some_and(|pane| pane.custom_name.is_some());
        if !has_custom_name && same_fleet_label(title, location) {
            if self.agent_statuses.contains_key(&pane_id) || self.shows_agents_roster(pane_id) {
                return self.pane_activity(pane_id, None);
            }
            let command = self.pane_command(pane_id);
            if command.is_empty() {
                self.pane_activity(pane_id, None)
            } else {
                command
            }
        } else {
            title.to_owned()
        }
    }

    /// True while this pane is showing Claude Code's Agents view.
    pub(crate) fn shows_agents_roster(&self, pane_id: PaneId) -> bool {
        self.agents_view_panes.contains(&pane_id)
    }

    pub(crate) fn pane_state_label(&self, pane_id: PaneId) -> String {
        if self.shows_agents_roster(pane_id) {
            // Until the first read lands the roster is genuinely unknown, so
            // the row names the surface instead of guessing a count — and says
            // so plainly when the read is what failed.
            return self.agents_roster.map_or_else(
                || {
                    if self.agents_roster_error.is_some() {
                        // The same word a pane whose terminal cannot be reached
                        // uses: the surface is there, the reading of it is not.
                        "Unavailable".into()
                    } else {
                        "Agents".into()
                    }
                },
                agents_roster::AgentsRoster::label,
            );
        }
        if let Some(status) = self.agent_statuses.get(&pane_id) {
            return agent_state_label(status.state).into();
        }
        self.terminals.get(&pane_id).map_or_else(
            || "Unavailable".into(),
            |runtime| match runtime.launch_state {
                TerminalLaunchState::PreparingHost => "Preparing".into(),
                TerminalLaunchState::Starting { .. } => "Starting".into(),
                TerminalLaunchState::Running => "Shell".into(),
                TerminalLaunchState::Failed(_) => "Unavailable".into(),
                TerminalLaunchState::Suppressed => "Not started".into(),
                TerminalLaunchState::Exited => "Exited".into(),
            },
        )
    }

    pub(crate) fn pane_activity(&self, pane_id: PaneId, notification: Option<&str>) -> String {
        if self.shows_agents_roster(pane_id) {
            return self.agents_roster.map_or_else(
                || {
                    self.agents_roster_error.clone().map_or_else(
                        || "Showing the agent roster".into(),
                        |error| format!("Agent roster unreadable — {error}"),
                    )
                },
                agents_roster::AgentsRoster::activity,
            );
        }
        if let Some(status) = self.agent_statuses.get(&pane_id) {
            if let Some(activity) = status.activity.as_deref().filter(|value| !value.is_empty()) {
                return activity.to_owned();
            }
            return match status.state {
                AgentState::Idle => "Ready for input".into(),
                AgentState::Running => "Agent working".into(),
                AgentState::Waiting => "Waiting for you".into(),
                AgentState::Completed => "Turn complete".into(),
                AgentState::Failed => "Agent failed".into(),
                AgentState::Stopped => "Agent stopped".into(),
            };
        }
        if let Some(notification) = notification.filter(|value| !value.is_empty()) {
            return notification.to_owned();
        }
        self.terminals.get(&pane_id).map_or_else(
            || "Terminal unavailable".into(),
            |runtime| match runtime.launch_state {
                TerminalLaunchState::PreparingHost => "Preparing terminal host".into(),
                TerminalLaunchState::Starting { .. } => "Starting terminal".into(),
                TerminalLaunchState::Running => "Ready for input".into(),
                TerminalLaunchState::Failed(_) => "Terminal unavailable".into(),
                TerminalLaunchState::Suppressed => "Terminal not started".into(),
                TerminalLaunchState::Exited => "Process exited".into(),
            },
        )
    }

    /// Raw pane context — callers apply their own display budgets, so this
    /// never pre-truncates (double truncation reads as a two-ended ellipsis).
    pub(crate) fn pane_context(&self, pane_id: PaneId) -> String {
        if let Some(status) = self.agent_statuses.get(&pane_id) {
            return match (status.git_branch.as_deref(), status.cwd.as_deref()) {
                (Some(branch), Some(cwd)) => format!("{branch} · {cwd}"),
                (None, Some(cwd)) => cwd.to_owned(),
                (_, None) => status.session_id.clone().unwrap_or_default(),
            };
        }
        // The live process's directory (via /proc on Linux) beats the static
        // launch configuration, so shell folder lines follow `cd`.
        self.pane_working_directory(pane_id)
            .map_or_else(String::new, |cwd| cwd.display().to_string())
    }

    pub(crate) fn pane_signal_kind(&self, pane_id: PaneId, attention: bool) -> PaneSignalKind {
        // A pane projecting the roster reports the roster's worst state: the
        // conversation behind it is backgrounded and has no visible state of
        // its own. Blocked and failed come from each session's own reported
        // state, never from the harness's "awaiting input" tally, which counts
        // merely-idle sessions as well.
        if self.shows_agents_roster(pane_id) {
            return self
                .agents_roster
                .and_then(agents_roster::AgentsRoster::signal)
                .map_or(PaneSignalKind::Neutral, |signal| match signal {
                    agents_roster::RosterSignal::Failed => PaneSignalKind::Danger,
                    agents_roster::RosterSignal::Blocked => PaneSignalKind::Warning,
                    agents_roster::RosterSignal::Working => PaneSignalKind::Active,
                    // A finished fleet reads exactly like a finished agent, and
                    // a roster of sessions that never started reads like a
                    // stopped one.
                    agents_roster::RosterSignal::Completed => PaneSignalKind::Neutral,
                    agents_roster::RosterSignal::Idle => PaneSignalKind::Subtle,
                });
        }
        match self.agent_statuses.get(&pane_id).map(|status| status.state) {
            Some(AgentState::Idle | AgentState::Stopped) => PaneSignalKind::Subtle,
            Some(AgentState::Running) => PaneSignalKind::Active,
            Some(AgentState::Waiting) => PaneSignalKind::Warning,
            Some(AgentState::Completed) => PaneSignalKind::Neutral,
            Some(AgentState::Failed) => PaneSignalKind::Danger,
            _ if self.terminals.get(&pane_id).is_some_and(|runtime| {
                matches!(runtime.launch_state, TerminalLaunchState::Failed(_))
            }) =>
            {
                PaneSignalKind::Danger
            }
            _ if attention => PaneSignalKind::Warning,
            _ if self.terminals.get(&pane_id).is_some_and(|runtime| {
                matches!(
                    runtime.launch_state,
                    TerminalLaunchState::PreparingHost | TerminalLaunchState::Starting { .. }
                )
            }) =>
            {
                // A slow WSL launch is progress, not a request for the user.
                // Keep the signal neutral while the nearby label says exactly
                // what is happening; amber is reserved for actionable input.
                PaneSignalKind::Neutral
            }
            _ if self.terminals.get(&pane_id).is_some_and(|runtime| {
                matches!(
                    runtime.launch_state,
                    TerminalLaunchState::Exited | TerminalLaunchState::Suppressed
                )
            }) =>
            {
                PaneSignalKind::Subtle
            }
            _ => PaneSignalKind::Neutral,
        }
    }

    pub(crate) fn pane_needs_attention(&self, pane_id: PaneId, unread_count: u32) -> bool {
        unread_count > 0
            && !self
                .agent_statuses
                .get(&pane_id)
                .is_some_and(|status| status.state == AgentState::Completed)
    }

    pub(crate) fn pane_signal_color(
        &self,
        pane_id: PaneId,
        attention: bool,
        tokens: DesignTokens,
    ) -> Color {
        self.pane_signal_kind(pane_id, attention).color(tokens)
    }

    pub(crate) fn tab_signal_kind(&self, tab: &WorkspaceTab) -> PaneSignalKind {
        tab.root
            .pane_ids()
            .into_iter()
            .filter_map(|pane_id| {
                tab.panes.get(&pane_id).map(|pane| {
                    self.pane_signal_kind(
                        pane_id,
                        self.pane_needs_attention(pane_id, pane.attention.unread_count),
                    )
                })
            })
            .max_by_key(|kind| pane_signal_priority(*kind))
            .unwrap_or(PaneSignalKind::Neutral)
    }

    pub(crate) fn workspace_signal_kind(&self, workspace: &Workspace) -> PaneSignalKind {
        workspace
            .tabs
            .iter()
            .map(|tab| self.tab_signal_kind(tab))
            .max_by_key(|kind| pane_signal_priority(*kind))
            .unwrap_or(PaneSignalKind::Neutral)
    }

    pub(crate) fn workspace_state_label(&self, workspace: &Workspace) -> &'static str {
        match self.workspace_signal_kind(workspace) {
            PaneSignalKind::Danger => "Failed",
            PaneSignalKind::Warning => "Needs input",
            PaneSignalKind::Active => "Working",
            PaneSignalKind::Subtle | PaneSignalKind::Neutral
                if self.workspace_has_pending_terminal(workspace) =>
            {
                "Starting"
            }
            PaneSignalKind::Subtle | PaneSignalKind::Neutral => "Ready",
        }
    }

    pub(crate) fn workspace_has_pending_terminal(&self, workspace: &Workspace) -> bool {
        workspace.tabs.iter().any(|tab| {
            tab.root.pane_ids().into_iter().any(|pane_id| {
                self.terminals.get(&pane_id).is_some_and(|runtime| {
                    matches!(
                        runtime.launch_state,
                        TerminalLaunchState::PreparingHost | TerminalLaunchState::Starting { .. }
                    )
                })
            })
        })
    }

    pub(crate) fn workspace_context(&self, workspace: &Workspace) -> String {
        workspace
            .active_tab()
            .into_iter()
            .flat_map(|tab| {
                std::iter::once(tab.focused_pane_id).chain(
                    tab.root
                        .pane_ids()
                        .into_iter()
                        .filter(move |pane_id| *pane_id != tab.focused_pane_id),
                )
            })
            .chain(
                workspace
                    .tabs
                    .iter()
                    .filter(|tab| tab.id != workspace.active_tab_id)
                    .flat_map(|tab| tab.root.pane_ids()),
            )
            .map(|pane_id| self.pane_context(pane_id))
            .find(|context| !context.is_empty())
            .unwrap_or_default()
    }

    pub(crate) fn fleet_workspaces(&self) -> impl Iterator<Item = &Workspace> {
        let active_workspace_id = self.session.active_workspace_id;
        let scope = self.settings.fleet_scope;
        self.session.workspaces.iter().filter(move |workspace| {
            scope == FleetScope::AllWorkspaces || workspace.id == active_workspace_id
        })
    }

    pub(crate) fn fleet_entries_in_tab_order(&self) -> Vec<(WorkspaceId, PaneId)> {
        self.fleet_workspaces()
            .flat_map(|workspace| {
                workspace.tabs.iter().flat_map(move |tab| {
                    pane_ids_in_layout(&tab.root)
                        .into_iter()
                        .map(move |pane_id| (workspace.id, pane_id))
                })
            })
            .collect()
    }

    pub(crate) fn fleet_repository_groups_for(
        &self,
        workspace: &Workspace,
    ) -> Vec<FleetRepositoryGroup> {
        let mut groups: Vec<FleetRepositoryGroup> = Vec::new();
        let mut no_repo = Vec::new();
        for pane_id in workspace
            .tabs
            .iter()
            .flat_map(|tab| pane_ids_in_layout(&tab.root))
        {
            let entry = (workspace.id, pane_id);
            let current_directory = self.pane_working_directory(pane_id);
            let repository_name = self
                .pane_repositories
                .get(&pane_id)
                .filter(|repository| current_directory.as_ref() == Some(&repository.directory))
                .and_then(|repository| repository.name.as_deref());
            let Some(repository_name) = repository_name else {
                no_repo.push(entry);
                continue;
            };
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group.name == repository_name)
            {
                group.entries.push(entry);
            } else {
                groups.push(FleetRepositoryGroup {
                    name: repository_name.to_owned(),
                    entries: vec![entry],
                });
            }
        }
        if !no_repo.is_empty() {
            groups.push(FleetRepositoryGroup {
                name: NO_REPO_GROUP.into(),
                entries: no_repo,
            });
        }
        groups
    }

    pub(crate) fn fleet_repository_groups(&self) -> Vec<FleetRepositoryGroup> {
        let mut groups = Vec::new();
        for workspace in self.fleet_workspaces() {
            groups.extend(self.fleet_repository_groups_for(workspace));
        }
        groups
    }

    pub(crate) fn fleet_entries(&self) -> Vec<(WorkspaceId, PaneId)> {
        let entries = self.fleet_entries_in_tab_order();
        match self.effective_fleet_view() {
            FleetView::Tabs => entries,
            // Agents is a flat filter over the same tab-and-pane order.
            FleetView::Agents => entries
                .into_iter()
                .filter(|(_, pane_id)| self.agent_statuses.contains_key(pane_id))
                .collect(),
            FleetView::Repos => self
                .fleet_repository_groups()
                .into_iter()
                .flat_map(|group| group.entries)
                .collect(),
        }
    }

    pub(crate) fn selected_terminal_text(&self, pane_id: PaneId) -> Option<String> {
        self.terminals.get(&pane_id)?.selection_text()
    }

    pub(crate) fn terminal_grid_cell_at(&self, pane_id: PaneId, position: Point) -> (u16, u16) {
        let size = self
            .terminals
            .get(&pane_id)
            .map_or_else(initial_pty_size, |runtime| runtime.size);
        terminal_grid_cell_at(position, &self.settings, size)
    }

    pub(crate) fn hovered_terminal_link(&self, pane_id: PaneId) -> Option<TerminalLink> {
        if self.hovered_terminal != Some(pane_id) {
            return None;
        }
        let snapshot = self.terminals.get(&pane_id)?.snapshot.as_ref()?;
        let position = self.terminal_pointer_positions.get(&pane_id).copied()?;
        let cell = terminal_cell_at(position, &self.settings, snapshot.scrollbar.offset);
        terminal_link_at(snapshot, cell)
    }

    pub(crate) fn sidebar_is_compact(&self) -> bool {
        self.sidebar_collapsed
    }

    pub(crate) fn default_agent_choices(&self) -> Vec<DefaultAgentChoice> {
        std::iter::once(DefaultAgentChoice::None)
            .chain(
                Agent::ALL
                    .into_iter()
                    .filter(|agent| self.agent_is_configured_for(*agent, &self.settings_draft))
                    .map(DefaultAgentChoice::Agent),
            )
            .collect()
    }

    pub(crate) fn default_agent_choice(&self) -> DefaultAgentChoice {
        self.settings_draft
            .default_agent
            .filter(|agent| self.agent_is_configured_for(*agent, &self.settings_draft))
            .map_or(DefaultAgentChoice::None, DefaultAgentChoice::Agent)
    }

    pub(crate) fn active_workspace(&self) -> Result<&Workspace, String> {
        self.session
            .workspaces
            .iter()
            .find(|workspace| workspace.id == self.session.active_workspace_id)
            .ok_or_else(|| "active workspace is missing".into())
    }

    pub(crate) fn active_workspace_mut(&mut self) -> Result<&mut Workspace, String> {
        self.session
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == self.session.active_workspace_id)
            .ok_or_else(|| "active workspace is missing".into())
    }
}

pub(crate) fn github_virtual_window(
    item_count: usize,
    offset: f32,
    viewport_height: f32,
    row_height: f32,
) -> (usize, usize) {
    let visible = (viewport_height / row_height.max(1.0)).ceil() as usize;
    let raw_first = (offset.max(0.0) / row_height.max(1.0)).floor() as usize;
    // A filter or refresh may make a formerly valid deep scroll offset point
    // past the new end. Anchor to the last full viewport before overscanning
    // so the virtualized body can never render as a blank list.
    let anchored_first = raw_first.min(item_count.saturating_sub(visible));
    let first = anchored_first.saturating_sub(GITHUB_FILE_OVERSCAN);
    let last = (first + visible + GITHUB_FILE_OVERSCAN * 2).min(item_count);
    (first, last)
}

pub(crate) fn github_clamped_scroll_offset(
    item_count: usize,
    offset: f32,
    viewport_height: f32,
    row_height: f32,
) -> f32 {
    let content_height = item_count as f32 * row_height.max(1.0);
    offset
        .max(0.0)
        .min((content_height - viewport_height).max(0.0))
}

pub(crate) fn github_scroll_offset_for_cursor(
    item_count: usize,
    cursor: usize,
    offset: f32,
    viewport_height: f32,
    row_height: f32,
) -> f32 {
    let row_height = row_height.max(1.0);
    let row_top = cursor as f32 * row_height;
    let row_bottom = row_top + row_height;
    let next = if row_top < offset {
        row_top
    } else if row_bottom > offset + viewport_height {
        row_bottom - viewport_height
    } else {
        offset
    };
    github_clamped_scroll_offset(item_count, next, viewport_height, row_height)
}

pub(crate) fn github_pull_request_viewport_height(window_size: Size) -> f32 {
    // Header, tabs, labelled search, count header, and their separators.
    (window_size.height - 198.0).max(140.0)
}

pub(crate) fn github_file_viewport_height(window_size: Size, pull_request_detail: bool) -> f32 {
    (window_size.height - if pull_request_detail { 380.0 } else { 142.0 }).max(140.0)
}

pub(crate) fn github_scroll_to(target: ScrollTarget, offset: f32) -> Vec<Effect> {
    vec![Effect::ScrollToOffset(target, offset)]
}

pub(crate) fn github_keyboard_focus_step(
    panel: &GitHubPanelState,
    current: GitHubPanelKeyboardFocus,
    forward: bool,
) -> GitHubPanelKeyboardFocus {
    let selected = panel.selected_pull_request_number.is_some();
    let pull_request = panel
        .selected_pull_request
        .as_ref()
        .map(|details| &details.pull_request);
    let merge_ready = pull_request
        .is_some_and(|pull_request| pull_request.readiness() == github::MergeReadiness::Ready);
    let order: &[GitHubPanelKeyboardFocus] = if panel.active_tab == GitHubPanelTab::Local {
        &[
            GitHubPanelKeyboardFocus::Tabs,
            GitHubPanelKeyboardFocus::Files,
        ]
    } else if !selected {
        &[
            GitHubPanelKeyboardFocus::Tabs,
            GitHubPanelKeyboardFocus::Search,
            GitHubPanelKeyboardFocus::PullRequestList,
        ]
    } else if pull_request.is_none() {
        &[
            GitHubPanelKeyboardFocus::Tabs,
            GitHubPanelKeyboardFocus::Back,
            GitHubPanelKeyboardFocus::Files,
        ]
    } else if panel.merge_confirmation {
        &[
            GitHubPanelKeyboardFocus::Tabs,
            GitHubPanelKeyboardFocus::Back,
            GitHubPanelKeyboardFocus::MergeAction,
            GitHubPanelKeyboardFocus::Files,
        ]
    } else if merge_ready {
        &[
            GitHubPanelKeyboardFocus::Tabs,
            GitHubPanelKeyboardFocus::Back,
            GitHubPanelKeyboardFocus::DraftAction,
            GitHubPanelKeyboardFocus::MergeAction,
            GitHubPanelKeyboardFocus::Files,
        ]
    } else {
        &[
            GitHubPanelKeyboardFocus::Tabs,
            GitHubPanelKeyboardFocus::Back,
            GitHubPanelKeyboardFocus::DraftAction,
            GitHubPanelKeyboardFocus::Files,
        ]
    };
    let index = order
        .iter()
        .position(|candidate| *candidate == current)
        .unwrap_or(0);
    if forward {
        order[(index + 1) % order.len()]
    } else {
        order[(index + order.len() - 1) % order.len()]
    }
}

pub(crate) fn github_diff_wrap_columns(window_width: f32, cell_width: f32) -> Option<usize> {
    let text_width = (window_width - GITHUB_PANEL_WIDTH - GITHUB_DIFF_CHROME_WIDTH).max(0.0);
    let columns = (text_width / cell_width.max(1.0)).floor() as usize;
    (columns >= GITHUB_DIFF_MIN_WRAP_COLUMNS).then_some(columns)
}

pub(crate) fn github_diff_line_starts(
    document: &github::DiffDocument,
    wrap_columns: Option<usize>,
) -> Vec<usize> {
    let mut starts = Vec::with_capacity(document.lines.len() + 1);
    starts.push(0);
    let mut visual_rows = 0usize;
    for line in &document.lines {
        let rows = wrap_columns.map_or(1, |columns| {
            line.text.chars().count().max(1).div_ceil(columns.max(1))
        });
        visual_rows = visual_rows.saturating_add(rows);
        starts.push(visual_rows);
    }
    starts
}

pub(crate) fn github_diff_line_for_visual_row(line_starts: &[usize], visual_row: usize) -> usize {
    line_starts
        .partition_point(|start| *start <= visual_row)
        .saturating_sub(1)
        .min(line_starts.len().saturating_sub(1))
}

pub(crate) fn github_diff_window(
    line_starts: &[usize],
    offset: f32,
    viewport_height: f32,
) -> (usize, usize, usize, usize) {
    let line_count = line_starts.len().saturating_sub(1);
    let total_rows = line_starts.last().copied().unwrap_or_default();
    let visible = (viewport_height / GITHUB_DIFF_LINE_HEIGHT).ceil() as usize;
    let raw_first = (offset.max(0.0) / GITHUB_DIFF_LINE_HEIGHT).floor() as usize;
    let first_row = raw_first.saturating_sub(GITHUB_DIFF_OVERSCAN);
    let last_row = raw_first
        .saturating_add(visible)
        .saturating_add(GITHUB_DIFF_OVERSCAN)
        .min(total_rows);
    let first = github_diff_line_for_visual_row(line_starts, first_row).min(line_count);
    let mut last = line_starts
        .partition_point(|start| *start < last_row)
        .min(line_count);
    if last < line_count && last <= first {
        last = first + 1;
    }
    let top_rows = line_starts.get(first).copied().unwrap_or(total_rows);
    let bottom_rows =
        total_rows.saturating_sub(line_starts.get(last).copied().unwrap_or(total_rows));
    (first, last, top_rows, bottom_rows)
}

pub(crate) fn github_diff_header_height(window_width: f32) -> f32 {
    if window_width - GITHUB_PANEL_WIDTH < 540.0 {
        76.0
    } else {
        52.0
    }
}

pub(crate) fn character_key_is(key: Key<&str>, expected: &str) -> bool {
    matches!(key, Key::Character(character) if character.eq_ignore_ascii_case(expected))
}

pub(crate) fn number_shortcut(key: Key<&str>) -> Option<usize> {
    let Key::Character(character) = key else {
        return None;
    };
    character
        .parse::<usize>()
        .ok()
        .filter(|number| (1..=9).contains(number))
}

pub(crate) fn clipboard_shortcut(key: Key<&str>, modifiers: Modifiers) -> Option<ClipboardAction> {
    clipboard_shortcut_for(key, modifiers, cfg!(target_os = "macos"))
}

/// Ghostty's default clipboard bindings: Super+C/Super+V on macOS and
/// Ctrl+Shift+C/Ctrl+Shift+V everywhere else. Everything else — including
/// bare Ctrl+C and Ctrl+V — still belongs to the terminal.
pub(crate) fn clipboard_shortcut_for(
    key: Key<&str>,
    modifiers: Modifiers,
    macos: bool,
) -> Option<ClipboardAction> {
    let chord = if macos {
        modifiers.logo() && !modifiers.control() && !modifiers.shift() && !modifiers.alt()
    } else {
        modifiers.control() && modifiers.shift() && !modifiers.logo() && !modifiers.alt()
    };
    if !chord {
        return None;
    }
    if character_key_is(key, "c") {
        Some(ClipboardAction::Copy)
    } else if character_key_is(key, "v") {
        Some(ClipboardAction::Paste)
    } else {
        None
    }
}

pub(crate) fn palette_selection(current: usize, count: usize, direction: PaletteMove) -> usize {
    if count == 0 {
        return 0;
    }
    match direction {
        PaletteMove::Next => (current + 1) % count,
        PaletteMove::Previous => (current + count - 1) % count,
    }
}

pub(crate) fn first_enabled_palette_command(enabled: &[bool]) -> usize {
    enabled
        .iter()
        .position(|command_enabled| *command_enabled)
        .unwrap_or(0)
}

pub(crate) fn enabled_palette_selection(
    current: usize,
    enabled: &[bool],
    direction: PaletteMove,
) -> usize {
    let enabled_count = enabled
        .iter()
        .filter(|command_enabled| **command_enabled)
        .count();
    if enabled_count == 0 {
        return 0;
    }
    let current_position = enabled.get(current).copied().unwrap_or(false).then(|| {
        enabled[..current]
            .iter()
            .filter(|command_enabled| **command_enabled)
            .count()
    });
    let next_position = current_position.map_or_else(
        || match direction {
            PaletteMove::Next => 0,
            PaletteMove::Previous => enabled_count - 1,
        },
        |position| palette_selection(position, enabled_count, direction),
    );
    enabled
        .iter()
        .enumerate()
        .filter_map(|(index, command_enabled)| command_enabled.then_some(index))
        .nth(next_position)
        .unwrap_or(0)
}

pub(crate) fn home_directory() -> Option<std::path::PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from)
}

/// Walks up from `directory` to the closest directory containing `.git`.
/// Whether a path is a real observed location rather than a launch
/// placeholder like `~`. On Windows a Linux-side path never counts as
/// absolute, so the leading-slash check carries that case.
pub(crate) fn reported_path_is_concrete(path: &std::path::Path) -> bool {
    path.is_absolute() || path.to_string_lossy().starts_with('/')
}

/// True when this process can only reach `path` through WSL: a Windows
/// build handed a Linux-side absolute path (as WSL panes report via OSC 7).
pub(crate) fn path_is_wsl_side(path: &std::path::Path) -> bool {
    cfg!(target_os = "windows") && path.to_string_lossy().starts_with('/')
}

/// Writes `content` to `$HOME/<relative_dir>/<file_name>` inside the WSL
/// distribution, returning the absolute directory it landed in.
#[cfg(target_os = "windows")]
pub(crate) fn wsl_stage_file(
    distribution: &str,
    relative_dir: &str,
    file_name: &str,
    content: &str,
) -> Option<String> {
    use std::io::Write as _;
    let mut child = wsl_command(distribution)
        .args([
            "--exec",
            "sh",
            "-c",
            &format!(
                "dir=\"$HOME/{relative_dir}\" && mkdir -p \"$dir\" && cat > \"$dir/{file_name}\" && printf %s \"$dir\""
            ),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(content.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let dir = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!dir.is_empty()).then_some(dir)
}

/// Explains why worktree creation is impossible, probing the actual failure
/// point instead of shrugging. Runs once, from the dialog-open path, and
/// only after repository detection has already failed.
pub(crate) fn worktree_failure_message(
    probed: Option<&std::path::Path>,
    wsl_distribution: &str,
) -> String {
    let Some(directory) = probed else {
        return "This pane's shell has not reported its working directory (OSC 7). Bash, \
                zsh, and fish panes report automatically when Muxtrix opens them: try a \
                freshly opened pane. Other shells need an OSC 7 prompt hook."
            .into();
    };
    #[cfg(target_os = "windows")]
    if path_is_wsl_side(directory) {
        let probe = |args: &[&str]| {
            let mut command = wsl_command(wsl_distribution);
            command.args(args);
            command_output(
                &mut command,
                HELPER_COMMAND_TIMEOUT,
                &ProcessCancellation::default(),
            )
            .is_ok_and(|output| output.status.success())
        };
        if !probe(&["--exec", "test", "-d", "/"]) {
            let distribution = wsl_distribution.trim();
            return if distribution.is_empty() {
                "The default WSL distribution could not be reached, so the repository \
                 cannot be inspected. Check that WSL is installed and running."
                    .into()
            } else {
                format!(
                    "WSL distribution \"{distribution}\" could not be reached — check the \
                     distribution selected in Settings."
                )
            };
        }
        let directory_str = directory.to_string_lossy();
        if !probe(&["--exec", "test", "-d", &directory_str]) {
            return format!(
                "{directory_str} does not exist inside WSL. The shell may have reported a \
                 stale directory, or a different distribution is selected in Settings."
            );
        }
        if !probe(&["--exec", "sh", "-c", "command -v git >/dev/null"]) {
            return "git is not installed inside the WSL distribution, so worktrees cannot \
                    be created there."
                .into();
        }
        return format!(
            "{directory_str} is not inside a git repository, so a worktree cannot be \
             created from it."
        );
    }
    let _ = wsl_distribution;
    if !directory.is_dir() {
        return format!(
            "{} no longer exists, so a worktree cannot be created from it.",
            directory.display()
        );
    }
    format!(
        "{} is not inside a git repository, so a worktree cannot be created from it.",
        directory.display()
    )
}

/// A hidden wsl.exe invocation targeting the configured distribution, or
/// the default distribution when the setting is empty.
#[cfg(target_os = "windows")]
pub(crate) fn wsl_command(wsl_distribution: &str) -> std::process::Command {
    let mut command = console_command("wsl.exe");
    let distribution = wsl_distribution.trim();
    if !distribution.is_empty() {
        command.args(["--distribution", distribution]);
    }
    command
}

/// The Linux-side home directory, asked of the distribution itself.
#[cfg(target_os = "windows")]
pub(crate) fn wsl_home_directory(wsl_distribution: &str) -> Option<String> {
    let mut command = wsl_command(wsl_distribution);
    command.args(["--exec", "sh", "-c", "printf %s \"$HOME\""]);
    let output = command_output(
        &mut command,
        HELPER_COMMAND_TIMEOUT,
        &ProcessCancellation::default(),
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let home = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!home.is_empty()).then_some(home)
}

/// Decodes a shell-reported working directory. OSC 7 carries a
/// `file://[host]/path` URI with percent-encoding; OSC 9/1337 carry a bare
/// path. libghostty stores the raw value; decoding is the embedder's job.
pub(crate) fn decode_reported_pwd(raw: &str) -> Option<std::path::PathBuf> {
    let raw = raw.trim();
    let path = if let Some(rest) = raw.strip_prefix("file://") {
        let path_start = rest.find('/')?;
        percent_decode_path(&rest[path_start..])
    } else {
        raw.to_owned()
    };
    if path.is_empty() {
        return None;
    }
    Some(std::path::PathBuf::from(path))
}

pub(crate) fn percent_decode_path(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Some(high) = (bytes[index + 1] as char).to_digit(16)
            && let Some(low) = (bytes[index + 2] as char).to_digit(16)
        {
            decoded.push((high * 16 + low) as u8);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The repository root that owns `directory`. git itself answers (it also
/// understands worktrees, and runs inside WSL when the directory lives
/// there); walking for `.git` remains as a fallback when git is missing.
pub(crate) fn git_repository_root(
    directory: &std::path::Path,
    wsl_distribution: &str,
) -> Option<std::path::PathBuf> {
    match git_in(
        directory,
        wsl_distribution,
        &["rev-parse", "--show-toplevel"],
    ) {
        Ok(output) if output.status.success() => {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            return (!root.is_empty()).then(|| std::path::PathBuf::from(root));
        }
        // Git ran and authoritatively said this is not a repository. Walking
        // upward here can mistake an unrelated or unreadable `.git` entry in
        // an ancestor for the pane's repository.
        Ok(_) => return None,
        Err(_) => {}
    }
    let mut directory = directory.to_path_buf();
    if !directory.is_dir() {
        return None;
    }
    loop {
        if directory.join(".git").exists() {
            return Some(directory);
        }
        if !directory.pop() {
            return None;
        }
    }
}

/// The primary checkout's leaf name for the repository containing
/// `directory`. Linked worktrees have their own top-level directory names, so
/// grouping from `--show-toplevel` would split one repository into many fake
/// repos. Git's common directory points every linked worktree back to the
/// primary checkout's `.git` directory.
#[cfg(test)]
pub(crate) fn git_repository_name(
    directory: &std::path::Path,
    wsl_distribution: &str,
) -> Option<String> {
    let root = git_repository_root(directory, wsl_distribution)?;
    git_repository_name_from_root(&root, wsl_distribution)
}

pub(crate) fn git_repository_name_from_root(
    repo_root: &std::path::Path,
    wsl_distribution: &str,
) -> Option<String> {
    git_repository_name_from_root_cancellable(
        repo_root,
        wsl_distribution,
        &ProcessCancellation::default(),
    )
}

pub(crate) fn git_repository_name_from_root_cancellable(
    repo_root: &std::path::Path,
    wsl_distribution: &str,
    cancellation: &ProcessCancellation,
) -> Option<String> {
    let common = git_in_cancellable(
        repo_root,
        wsl_distribution,
        &["rev-parse", "--git-common-dir"],
        cancellation,
    )
    .ok()
    .filter(|output| output.status.success())
    .and_then(|output| {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        (!value.is_empty()).then(|| std::path::PathBuf::from(value))
    });
    common
        .map(|common| {
            if reported_path_is_concrete(&common) {
                common
            } else {
                repo_root.join(common)
            }
        })
        .and_then(|common| common.parent().and_then(path_leaf_name))
        .or_else(|| path_leaf_name(repo_root))
}

/// The leaf name of a linked worktree, excluding the repository's primary
/// checkout. A linked checkout has a `.git` pointer file where the primary
/// checkout has a directory. This stays process-free on the UI path; the
/// convention fallback covers Linux-side paths a native Windows build cannot
/// inspect directly.
pub(crate) fn linked_worktree_name(directory: &std::path::Path) -> Option<String> {
    let mut root = directory.to_path_buf();
    while !root.as_os_str().is_empty() {
        let metadata = root.join(".git");
        if metadata.is_file() {
            return path_leaf_name(&root);
        }
        if metadata.is_dir() {
            return None;
        }
        if !root.pop() {
            break;
        }
    }
    linked_worktree_name_from_convention(directory)
}

pub(crate) fn path_leaf_name(path: &std::path::Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

pub(crate) fn linked_worktree_name_from_convention(directory: &std::path::Path) -> Option<String> {
    let components: Vec<_> = directory
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect();
    for (index, component) in components.iter().enumerate() {
        if !component.eq_ignore_ascii_case("worktrees") {
            continue;
        }
        let parent = index.checked_sub(1).and_then(|index| components.get(index));
        let name_index = if parent.is_some_and(|parent| {
            parent.eq_ignore_ascii_case(".claude") || parent.eq_ignore_ascii_case(".codex")
        }) {
            index + 1
        } else if parent.is_some_and(|parent| {
            parent.eq_ignore_ascii_case(".muxtrix") || parent.eq_ignore_ascii_case("codex-fleet")
        }) {
            index + 2
        } else {
            continue;
        };
        if let Some(name) = components.get(name_index).map(|name| name.trim())
            && !name.is_empty()
        {
            return Some(name.to_owned());
        }
    }
    None
}

/// Where a repository's worktrees live: `<home>/.muxtrix/worktrees/<repo>`,
/// with `<repo>` taken from the primary checkout so creating a worktree from a
/// linked checkout stays in the same repository namespace. `<home>` is on
/// whichever side of the WSL boundary owns the repository.
pub(crate) fn worktree_base_directory(
    repo_root: &std::path::Path,
    wsl_distribution: &str,
) -> Option<std::path::PathBuf> {
    let repo_name = git_repository_name_from_root(repo_root, wsl_distribution)?;
    #[cfg(target_os = "windows")]
    if path_is_wsl_side(repo_root) {
        let home = wsl_home_directory(wsl_distribution)?;
        return Some(std::path::PathBuf::from(format!(
            "{home}/{WORKTREE_HOME_FOLDER}/{repo_name}"
        )));
    }
    Some(home_directory()?.join(WORKTREE_HOME_FOLDER).join(repo_name))
}

/// Joins the worktree name onto the base without introducing backslashes —
/// the base may be a Linux-side path a Windows build reaches through WSL.
pub(crate) fn worktree_destination(base: &std::path::Path, name: &str) -> std::path::PathBuf {
    if path_is_wsl_side(base) {
        std::path::PathBuf::from(format!("{}/{name}", base.to_string_lossy()))
    } else {
        base.join(name)
    }
}

/// Directory names already present under the worktree base.
pub(crate) fn worktree_taken_names(
    base: &std::path::Path,
    wsl_distribution: &str,
) -> BTreeSet<String> {
    #[cfg(target_os = "windows")]
    if path_is_wsl_side(base) {
        let mut command = wsl_command(wsl_distribution);
        command.args(["--exec", "ls", "-1"]).arg(base);
        return command_output(
            &mut command,
            HELPER_COMMAND_TIMEOUT,
            &ProcessCancellation::default(),
        )
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| line.trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect()
        })
        .unwrap_or_default();
    }
    let _ = wsl_distribution;
    std::fs::read_dir(base)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Parses `git worktree list --porcelain` output into (path, branch) pairs.
/// The first entry is the main worktree — callers filter it out by path.
pub(crate) fn parse_worktree_list(output: &str) -> Vec<(std::path::PathBuf, Option<String>)> {
    let mut entries = Vec::new();
    let mut current: Option<(std::path::PathBuf, Option<String>)> = None;
    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some((std::path::PathBuf::from(path), None));
        } else if let Some(branch) = line.strip_prefix("branch ")
            && let Some((_, slot)) = current.as_mut()
        {
            *slot = Some(
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(branch)
                    .to_owned(),
            );
        }
    }
    if let Some(entry) = current {
        entries.push(entry);
    }
    entries
}

/// All worktrees in git's stable porcelain order. Git documents the primary
/// worktree first, followed by linked worktrees.
pub(crate) fn git_worktrees(
    repo_root: &std::path::Path,
    wsl_distribution: &str,
) -> Option<Vec<(std::path::PathBuf, Option<String>)>> {
    let output = git_in(
        repo_root,
        wsl_distribution,
        &["worktree", "list", "--porcelain"],
    )
    .ok()?;
    output
        .status
        .success()
        .then(|| parse_worktree_list(&String::from_utf8_lossy(&output.stdout)))
}

/// Performs every Git subprocess needed by the worktree screens away from the
/// Iced update thread. The screen is already visible with a loading state while
/// this runs, so repositories with many checkouts never stall navigation.
pub(crate) fn discover_worktree_manager(
    mode: WorktreeManagerMode,
    probed_directory: Option<std::path::PathBuf>,
    wsl_distribution: &str,
) -> Result<WorktreeManagerDiscovery, String> {
    let repo_root = probed_directory
        .as_deref()
        .and_then(|directory| git_repository_root(directory, wsl_distribution));
    let Some(repo_root) = repo_root else {
        return Ok(WorktreeManagerDiscovery {
            repo_root: None,
            failure: Some(worktree_failure_message(
                probed_directory.as_deref(),
                wsl_distribution,
            )),
            entries: Vec::new(),
        });
    };
    let worktrees = git_worktrees(&repo_root, wsl_distribution)
        .ok_or_else(|| format!("Git could not list worktrees for {}", repo_root.display()))?;
    let entries = worktrees
        .iter()
        .enumerate()
        .filter(|(_, (path, _))| match mode {
            WorktreeManagerMode::Manage => true,
            WorktreeManagerMode::RestartPane(_)
            | WorktreeManagerMode::RestartPaneWithAgent(_, _) => probed_directory
                .as_deref()
                .is_none_or(|directory| !directory.starts_with(path)),
        })
        .map(|(index, (path, branch))| WorktreeManagerEntry {
            used_by: None,
            deletion_blocker: worktree_deletion_blocker(index == 0),
            unpushed_commits: unpushed_commit_count(path, wsl_distribution),
            path: path.clone(),
            branch: branch.clone(),
        })
        .collect();
    Ok(WorktreeManagerDiscovery {
        repo_root: Some(repo_root),
        failure: None,
        entries,
    })
}

/// Counts commits in a checkout that are not reachable from any configured
/// remote ref. This catches both ahead-of-upstream work and branches that have
/// never been published without requiring a network fetch.
pub(crate) fn unpushed_commit_count(worktree: &std::path::Path, wsl_distribution: &str) -> usize {
    let Ok(output) = git_in(
        worktree,
        wsl_distribution,
        &["rev-list", "--count", "HEAD", "--not", "--remotes"],
    ) else {
        return 0;
    };
    if !output.status.success() {
        return 0;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

pub(crate) fn worktree_display_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

pub(crate) fn unused_worktree_paths(entries: &[WorktreeManagerEntry]) -> Vec<std::path::PathBuf> {
    entries
        .iter()
        .filter(|entry| entry.deletion_blocker.is_none() && entry.used_by.is_none())
        .map(|entry| entry.path.clone())
        .collect()
}

pub(crate) fn remove_git_worktrees(
    repo_root: &std::path::Path,
    paths: Vec<std::path::PathBuf>,
    wsl_distribution: &str,
) -> (Vec<std::path::PathBuf>, Result<(), String>) {
    let mut removed = Vec::new();
    let mut failures = Vec::new();
    for path in paths {
        let path_str = path.to_string_lossy().into_owned();
        match git_in(
            repo_root,
            wsl_distribution,
            &["worktree", "remove", &path_str],
        ) {
            Ok(output) if output.status.success() => removed.push(path),
            Ok(output) => failures.push(format!(
                "{}: {}",
                worktree_display_name(&path),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => failures.push(format!("{}: {error}", worktree_display_name(&path))),
        }
    }
    let result = if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("Could not remove {}", failures.join("; ")))
    };
    (removed, result)
}

/// The default branch advertised by a configured GitHub remote. A normal
/// GitHub clone records this as refs/remotes/<remote>/HEAD, so this stays
/// local and responsive while still following repository metadata instead of
/// assuming a branch name.
pub(crate) fn github_default_branch(
    repo_root: &std::path::Path,
    wsl_distribution: &str,
) -> Option<String> {
    let output = git_in(repo_root, wsl_distribution, &["remote"]).ok()?;
    if !output.status.success() {
        return None;
    }
    let mut remotes: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|remote| !remote.is_empty())
        .map(str::to_owned)
        .collect();
    remotes.sort_by_key(|remote| remote != "origin");
    for remote in remotes {
        let Ok(url) = git_in(repo_root, wsl_distribution, &["remote", "get-url", &remote]) else {
            continue;
        };
        if !url.status.success()
            || !String::from_utf8_lossy(&url.stdout)
                .to_ascii_lowercase()
                .contains("github.com")
        {
            continue;
        }
        let reference = format!("refs/remotes/{remote}/HEAD");
        let Ok(head) = git_in(
            repo_root,
            wsl_distribution,
            &["symbolic-ref", "--quiet", "--short", &reference],
        ) else {
            continue;
        };
        if !head.status.success() {
            continue;
        }
        let head = String::from_utf8_lossy(&head.stdout).trim().to_owned();
        let prefix = format!("{remote}/");
        if let Some(branch) = head
            .strip_prefix(&prefix)
            .filter(|branch| !branch.is_empty())
        {
            return Some(branch.to_owned());
        }
    }
    None
}

/// Picks the worktree for the GitHub default branch. If remote HEAD metadata
/// is missing, main/master are conservative fallbacks; the primary worktree
/// remains the final safe destination.
pub(crate) fn preferred_default_worktree(
    worktrees: &[(std::path::PathBuf, Option<String>)],
    github_default_branch: Option<&str>,
) -> std::path::PathBuf {
    let branch = github_default_branch.and_then(|default_branch| {
        worktrees
            .iter()
            .find(|(_, branch)| branch.as_deref() == Some(default_branch))
    });
    let fallback = github_default_branch.is_none().then(|| {
        ["main", "master"].into_iter().find_map(|fallback_branch| {
            worktrees
                .iter()
                .find(|(_, branch)| branch.as_deref() == Some(fallback_branch))
        })
    });
    branch
        .or_else(|| fallback.flatten())
        .or_else(|| worktrees.first())
        .map_or_else(std::path::PathBuf::new, |(path, _)| path.clone())
}

pub(crate) fn regular_creation_directory_from_worktrees(
    focused_directory: &std::path::Path,
    focused_repo_root: &std::path::Path,
    worktrees: &[(std::path::PathBuf, Option<String>)],
    github_default_branch: Option<&str>,
) -> std::path::PathBuf {
    if worktrees
        .first()
        .is_none_or(|(primary_path, _)| primary_path == focused_repo_root)
    {
        return focused_directory.to_path_buf();
    }
    preferred_default_worktree(worktrees, github_default_branch)
}

/// Resolves the launch directory for an ordinary pane, tab, or workspace.
/// Any subprocess work happens on the terminal-launch worker. Failures are
/// intentionally conservative: outside a linked worktree, or when Git cannot
/// answer, preserve the exact focused directory.
pub(crate) fn resolve_regular_creation_directory(
    focused_directory: &std::path::Path,
    wsl_distribution: &str,
) -> std::path::PathBuf {
    let Some(repo_root) = git_repository_root(focused_directory, wsl_distribution) else {
        return focused_directory.to_path_buf();
    };
    let Some(worktrees) = git_worktrees(&repo_root, wsl_distribution) else {
        return focused_directory.to_path_buf();
    };
    let default_branch = github_default_branch(&repo_root, wsl_distribution);
    regular_creation_directory_from_worktrees(
        focused_directory,
        &repo_root,
        &worktrees,
        default_branch.as_deref(),
    )
}

pub(crate) fn worktree_deletion_blocker(is_primary: bool) -> Option<String> {
    is_primary.then(|| "Primary worktree".into())
}

/// The first `worktree-N` not already taken.
pub(crate) fn default_worktree_name(taken: &BTreeSet<String>) -> String {
    (1..1000)
        .map(|index| format!("worktree-{index}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| "worktree".into())
}

/// A worktree name usable as both a directory name and a branch name.
pub(crate) fn worktree_name(raw: &str) -> String {
    let mut name: String = raw
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '/') {
                character
            } else {
                '-'
            }
        })
        .collect();
    while name.starts_with(['-', '.', '/']) {
        name.remove(0);
    }
    while name.ends_with(['/', '.']) {
        name.pop();
    }
    name.replace('/', "-")
}

/// Runs git where the repository actually lives: natively, or inside WSL
/// when a Windows build is pointed at a Linux-side path.
pub(crate) fn git_in(
    repo_root: &std::path::Path,
    wsl_distribution: &str,
    args: &[&str],
) -> Result<std::process::Output, String> {
    git_in_cancellable(
        repo_root,
        wsl_distribution,
        args,
        &ProcessCancellation::default(),
    )
}

pub(crate) fn git_in_cancellable(
    repo_root: &std::path::Path,
    wsl_distribution: &str,
    args: &[&str],
    cancellation: &ProcessCancellation,
) -> Result<std::process::Output, String> {
    #[cfg(target_os = "windows")]
    let mut command = if path_is_wsl_side(repo_root) {
        let mut command = wsl_command(wsl_distribution);
        command.args(["--exec", "env", "GIT_OPTIONAL_LOCKS=0", "git"]);
        command
    } else {
        console_command("git")
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = console_command("git");
    let _ = wsl_distribution;
    command.env("GIT_OPTIONAL_LOCKS", "0");
    command.arg("-C").arg(repo_root).args(args);
    command_output(
        &mut command,
        std::time::Duration::from_secs(2 * 60),
        cancellation,
    )
    .map_err(|error| format!("could not run git: {error}"))
}

pub(crate) fn create_git_worktree(
    repo_root: &std::path::Path,
    destination: &std::path::Path,
    branch: &str,
    wsl_distribution: &str,
) -> Result<std::path::PathBuf, String> {
    if path_is_wsl_side(destination) {
        // The destination is on the Linux side, unreachable via std::fs.
        #[cfg(target_os = "windows")]
        if let Some(parent) = destination.parent() {
            let mut command = wsl_command(wsl_distribution);
            command.args(["--exec", "mkdir", "-p"]).arg(parent);
            let _ = command_output(
                &mut command,
                HELPER_COMMAND_TIMEOUT,
                &ProcessCancellation::default(),
            );
        }
    } else if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        // Dot-prefixed names are not hidden on Windows by themselves; the
        // attribute is what Explorer respects.
        #[cfg(target_os = "windows")]
        if let Some(app_folder) = home_directory().map(|home| home.join(".muxtrix"))
            && app_folder.is_dir()
        {
            let mut command = console_command("attrib");
            command.arg("+h").arg(&app_folder);
            let _ = command_output(
                &mut command,
                HELPER_COMMAND_TIMEOUT,
                &ProcessCancellation::default(),
            );
        }
    }
    // A hand-deleted worktree folder leaves a stale registration behind that
    // blocks every later attempt; prune is cheap and makes retries work.
    let _ = git_in(repo_root, wsl_distribution, &["worktree", "prune"]);
    // A branch left over from an earlier worktree is reused rather than
    // treated as a fatal conflict — unless a live worktree still holds it,
    // which git reports clearly below.
    let branch_exists = git_in(
        repo_root,
        wsl_distribution,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok_and(|output| output.status.success());
    let output = if branch_exists {
        let mut args = vec!["worktree", "add"];
        let destination_str = destination.to_string_lossy();
        args.push(&destination_str);
        args.push(branch);
        git_in(repo_root, wsl_distribution, &args)?
    } else {
        let destination_str = destination.to_string_lossy();
        git_in(
            repo_root,
            wsl_distribution,
            &["worktree", "add", "-b", branch, &destination_str],
        )?
    };
    if output.status.success() {
        Ok(destination.to_path_buf())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr.trim().to_owned())
    }
}

/// Coarse "3h ago"-style label from a unix timestamp.
pub(crate) fn age_label(created_unix: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let elapsed = now.saturating_sub(created_unix);
    if created_unix == 0 {
        return "unknown age".into();
    }
    match elapsed {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", elapsed / 60),
        3600..=86_399 => format!("{}h ago", elapsed / 3600),
        86_400..=31_535_999 => format!("{}d ago", elapsed / 86_400),
        _ => "a long time ago".into(),
    }
}

pub(crate) fn split_ratio_at(tree: &PaneTree, path: &[SplitBranch]) -> Option<SplitRatio> {
    if path.is_empty() {
        return match tree {
            PaneTree::Split { ratio, .. } => Some(*ratio),
            PaneTree::Leaf { .. } | PaneTree::Stack { .. } => None,
        };
    }
    let PaneTree::Split { first, second, .. } = tree else {
        return None;
    };
    match path[0] {
        SplitBranch::First => split_ratio_at(first, &path[1..]),
        SplitBranch::Second => split_ratio_at(second, &path[1..]),
    }
}

pub(crate) fn set_split_ratio_at(
    tree: &mut PaneTree,
    path: &[SplitBranch],
    next: SplitRatio,
) -> bool {
    if path.is_empty() {
        if let PaneTree::Split { ratio, .. } = tree {
            *ratio = next;
            return true;
        }
        return false;
    }
    let PaneTree::Split { first, second, .. } = tree else {
        return false;
    };
    match path[0] {
        SplitBranch::First => set_split_ratio_at(first, &path[1..], next),
        SplitBranch::Second => set_split_ratio_at(second, &path[1..], next),
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum IconKind {
    Back,
    Add,
    Collapse,
    Expand,
    SplitRight,
    SplitDown,
    Maximize,
    Restore,
    Settings,
    Command,
    GitHub,
    Refresh,
    Branch,
    File,
    Close,
    Overflow,
    StatusReady,
    StatusWarning,
    StatusError,
    StatusInfo,
    PullRequestOpen,
    PullRequestDraft,
    PullRequestClosed,
    PullRequestMerged,
}

pub(crate) fn icon<'a>(kind: IconKind, color: Color, size: f32) -> svg::Svg<'a> {
    let bytes: &'static [u8] = match kind {
        IconKind::Back => include_bytes!("../assets/icons/back.svg"),
        IconKind::Add => include_bytes!("../assets/icons/add.svg"),
        IconKind::Collapse => include_bytes!("../assets/icons/collapse.svg"),
        IconKind::Expand => include_bytes!("../assets/icons/expand.svg"),
        IconKind::SplitRight => include_bytes!("../assets/icons/split-right.svg"),
        IconKind::SplitDown => include_bytes!("../assets/icons/split-down.svg"),
        IconKind::Maximize => include_bytes!("../assets/icons/maximize.svg"),
        IconKind::Restore => include_bytes!("../assets/icons/restore.svg"),
        IconKind::Settings => include_bytes!("../assets/icons/settings.svg"),
        IconKind::Command => include_bytes!("../assets/icons/command.svg"),
        IconKind::GitHub => include_bytes!("../assets/icons/github.svg"),
        IconKind::Refresh => include_bytes!("../assets/icons/refresh.svg"),
        IconKind::Branch => include_bytes!("../assets/icons/branch.svg"),
        IconKind::File => include_bytes!("../assets/icons/file.svg"),
        IconKind::Close => include_bytes!("../assets/icons/close.svg"),
        IconKind::Overflow => include_bytes!("../assets/icons/overflow.svg"),
        IconKind::StatusReady => include_bytes!("../assets/icons/status-ready.svg"),
        IconKind::StatusWarning => include_bytes!("../assets/icons/status-warning.svg"),
        IconKind::StatusError => include_bytes!("../assets/icons/status-error.svg"),
        IconKind::StatusInfo => include_bytes!("../assets/icons/status-info.svg"),
        IconKind::PullRequestOpen => include_bytes!("../assets/icons/pull-request-open.svg"),
        IconKind::PullRequestDraft => include_bytes!("../assets/icons/pull-request-draft.svg"),
        IconKind::PullRequestClosed => include_bytes!("../assets/icons/pull-request-closed.svg"),
        IconKind::PullRequestMerged => include_bytes!("../assets/icons/pull-request-merged.svg"),
    };
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
}

/// The 2px accent bar that marks the selected row in a ruled list — always
/// present so selection never shifts layout, transparent when inactive.
/// Small tinted state pill sitting beside an identity line in list rows —
/// keeps state out of the meta column's lane so long meta never collides
/// with it.
pub(crate) fn status_pill(
    label: &str,
    hue: Color,
    settings: &AppSettings,
) -> Element<'static, Message> {
    container(
        text(label.to_owned())
            .size(settings.ui_pixels(7.5))
            .font(Font {
                weight: font::Weight::Semibold,
                ..Font::DEFAULT
            })
            .color(hue)
            .wrapping(iced::widget::text::Wrapping::None),
    )
    .padding([2, 8])
    .style(move |_| {
        container::Style::default()
            .background(Color { a: 0.12, ..hue })
            .border(Border {
                color: Color { a: 0.3, ..hue },
                width: 1.0,
                radius: 999.0.into(),
            })
    })
    .into()
}

pub(crate) fn selection_bar(selected: bool, tokens: DesignTokens) -> Element<'static, Message> {
    container("")
        .width(3)
        .height(Length::Fill)
        .style(move |_| {
            container::Style::default().background(if selected {
                tokens.accent
            } else {
                Color::TRANSPARENT
            })
        })
        .into()
}

/// The leading mark in a rail row's gutter. Where you already are is one
/// unbroken accent bar; where the keyboard cursor would land is that same bar
/// cut into rungs. Solid reads as committed and broken reads as proposed, and
/// the pair stays legible in a 3px gutter where two shades of one accent would
/// collapse into each other. The cursor wins the gutter when it sits on the
/// focused row, matching the row fill, which resolves the overlap the same way.
pub(crate) fn rail_marker(
    selected: bool,
    targeted: bool,
    tokens: DesignTokens,
) -> Element<'static, Message> {
    if !targeted {
        return selection_bar(selected, tokens);
    }
    let mut ladder = column![].width(3).height(Length::Fill);
    for rung in 0..RAIL_CURSOR_RUNGS {
        let filled = rung % 2 == 0;
        ladder = ladder.push(container("").width(3).height(Length::Fill).style(move |_| {
            container::Style::default().background(if filled {
                tokens.accent
            } else {
                Color::TRANSPARENT
            })
        }));
    }
    ladder.into()
}

pub(crate) fn signal_dot(color: Color, size: f32) -> Element<'static, Message> {
    container("")
        .width(size)
        .height(size)
        .style(move |_| {
            container::Style::default()
                .background(color)
                .border(Border::default().rounded(size / 2.0))
        })
        .into()
}

/// The pip for a pane projecting Claude Code's roster: the same footprint and
/// the same rolled-up signal colour as a lifecycle dot, drawn as a core inside
/// a ring so the row reads as a container of agents rather than as one agent's
/// own state. It keeps the shared left edge every fleet row aligns to.
///
/// The ring alone was not enough. At the quiet end of the palette a hairline
/// outline of `faint` or `muted` on the rail's own background reads as no pip
/// at all — which is the state a healthy fleet spends most of its time in — so
/// the mark keeps a solid centre at every signal.
pub(crate) fn roster_ring(color: Color, size: f32) -> Element<'static, Message> {
    container(signal_dot(color, (size * 0.45).round().max(3.0)))
        .width(size)
        .height(size)
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .style(move |_| {
            container::Style::default().border(Border {
                color: Color {
                    a: color.a * 0.7,
                    ..color
                },
                width: 1.0,
                radius: (size / 2.0).into(),
            })
        })
        .into()
}

pub(crate) fn section_label(
    label: &'static str,
    settings: &AppSettings,
    tokens: DesignTokens,
) -> Element<'static, Message> {
    container(
        text(label)
            .size(settings.ui_pixels(9.0))
            .color(tokens.faint)
            .font(Font {
                weight: font::Weight::Semibold,
                ..Font::DEFAULT
            }),
    )
    .padding([6, 6])
    .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FleetGroupLevel {
    Workspace,
    Nested,
}

/// A fleet group band with an amber rollup dot when any pane inside needs a
/// person. Workspace bands carry stronger type and the rail surface; nested
/// tab and repository bands stay smaller and recessed on the app surface.
pub(crate) fn fleet_group_label(
    label: String,
    level: FleetGroupLevel,
    warning: bool,
    targeted: bool,
    on_press: Option<Message>,
    settings: &AppSettings,
    tokens: DesignTokens,
) -> Element<'static, Message> {
    let workspace = level == FleetGroupLevel::Workspace;
    let content = row![
        text(ellipsize(
            &label.to_uppercase(),
            settings.ui_char_budget(if workspace { 24 } else { 26 })
        ))
        .size(settings.ui_pixels(if workspace { 9.0 } else { 8.0 }))
        .font(Font {
            weight: if workspace {
                font::Weight::Bold
            } else {
                font::Weight::Semibold
            },
            ..Font::DEFAULT
        })
        .color(if targeted {
            tokens.accent
        } else if workspace {
            tokens.muted
        } else {
            tokens.faint
        })
        .width(Fill)
        .wrapping(iced::widget::text::Wrapping::None),
        if warning {
            signal_dot(tokens.warning, 6.0)
        } else {
            signal_dot(Color::TRANSPARENT, 6.0)
        },
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let mut band = button(centered_button_content(content))
        .height(if workspace { 32 } else { 30 })
        .padding([0, if workspace { 12 } else { 16 }])
        .width(Fill)
        .style(move |_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(iced::Background::Color(if targeted {
                    Color {
                        a: 0.12,
                        ..tokens.accent
                    }
                } else if hovered {
                    Color {
                        a: 0.04,
                        ..tokens.text
                    }
                } else if workspace {
                    tokens.rail
                } else {
                    tokens.app
                })),
                text_color: if targeted { tokens.text } else { tokens.faint },
                border: Border {
                    color: if targeted {
                        tokens.accent
                    } else {
                        Color::TRANSPARENT
                    },
                    width: if targeted { 1.0 } else { 0.0 },
                    radius: 0.0.into(),
                },
                shadow: Shadow::default(),
                snap: true,
            }
        });
    if let Some(message) = on_press {
        band = band.on_press(message);
    }
    band.into()
}

pub(crate) const fn pane_signal_priority(kind: PaneSignalKind) -> u8 {
    match kind {
        PaneSignalKind::Danger => 4,
        PaneSignalKind::Warning => 3,
        PaneSignalKind::Active => 2,
        PaneSignalKind::Neutral => 1,
        PaneSignalKind::Subtle => 0,
    }
}

pub(crate) fn ellipsize(value: &str, max_characters: usize) -> String {
    if value.chars().count() <= max_characters {
        return value.to_owned();
    }
    let mut truncated: String = value
        .chars()
        .take(max_characters.saturating_sub(1))
        .collect();
    truncated.push('…');
    truncated
}

/// Truncates from the front, keeping the tail — for paths, the leaf is the
/// informative end.
pub(crate) fn ellipsize_start(value: &str, max_characters: usize) -> String {
    let count = value.chars().count();
    if count <= max_characters {
        return value.to_owned();
    }
    let keep = max_characters.saturating_sub(1).max(1);
    let mut truncated = String::from("\u{2026}");
    truncated.extend(value.chars().skip(count - keep));
    truncated
}

pub(crate) fn single_line_ellipsize(value: &str, max_characters: usize) -> String {
    let single_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    ellipsize(&single_line, max_characters)
}

pub(crate) fn same_fleet_label(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    !left.is_empty() && left.eq_ignore_ascii_case(right)
}

pub(crate) fn git_branch_for_directory(cwd: Option<&str>) -> Option<String> {
    let mut directory = std::path::PathBuf::from(cwd?.trim());
    if !directory.is_dir() {
        return None;
    }
    loop {
        let metadata = directory.join(".git");
        let head = if metadata.is_dir() {
            std::fs::read_to_string(metadata.join("HEAD")).ok()
        } else if metadata.is_file() {
            let pointer = std::fs::read_to_string(&metadata).ok()?;
            let git_dir = pointer.trim().strip_prefix("gitdir:")?.trim();
            let git_dir = std::path::Path::new(git_dir);
            let git_dir = if git_dir.is_absolute() {
                git_dir.to_owned()
            } else {
                directory.join(git_dir)
            };
            std::fs::read_to_string(git_dir.join("HEAD")).ok()
        } else {
            None
        };
        if let Some(head) = head
            && let Some(branch) = head.trim().strip_prefix("ref: refs/heads/")
            && !branch.is_empty()
        {
            return Some(branch.to_owned());
        }
        if !directory.pop() {
            return None;
        }
    }
}

/// Rail rows are square and full-bleed. Real selection keeps the familiar
/// neutral fill plus its solid 3px leading bar; the keyboard cursor takes an
/// accent tint, a complete accent perimeter, accent headline text, and a rung
/// bar, so current location and proposed destination remain unmistakably
/// different, even when they overlap. The cursor is transient and answers a
/// question the eye is actively asking, so it is allowed to shout where
/// selection stays quiet.
pub(crate) fn rail_row_style(
    tokens: DesignTokens,
    selected: bool,
    targeted: bool,
    status: button::Status,
) -> button::Style {
    if targeted {
        return button::Style {
            background: Some(
                Color {
                    a: 0.18,
                    ..tokens.accent
                }
                .into(),
            ),
            text_color: tokens.text,
            border: Border {
                color: tokens.accent,
                width: 1.0,
                radius: 0.0.into(),
            },
            shadow: Shadow::default(),
            snap: true,
        };
    }

    button::Style {
        border: Border::default(),
        ..quiet_button_style(tokens, selected, status)
    }
}

pub(crate) fn quiet_button_style(
    tokens: DesignTokens,
    selected: bool,
    status: button::Status,
) -> button::Style {
    // Text-derived translucent fills read correctly on every surface in both
    // appearances: light overlay on dark text-on-dark, dark overlay on light.
    let background = if selected {
        Some(Color {
            a: 0.07,
            ..tokens.text
        })
    } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        Some(Color {
            a: 0.04,
            ..tokens.text
        })
    } else {
        None
    };
    button::Style {
        background: background.map(iced::Background::Color),
        text_color: tokens.text,
        border: Border::default().rounded(6.0),
        shadow: Shadow::default(),
        snap: true,
    }
}

/// A fleet-view segment inside the recessed toggle track. The selected thumb
/// is a raised, bordered chip with its own shadow so the control reads as a
/// physical toggle: dark well, light thumb.
pub(crate) fn fleet_toggle_style(
    tokens: DesignTokens,
    selected: bool,
    status: button::Status,
) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let background = if selected {
        Some(tokens.panel_raised)
    } else if hovered {
        Some(Color {
            a: 0.05,
            ..tokens.text
        })
    } else {
        None
    };
    button::Style {
        background: background.map(iced::Background::Color),
        text_color: if selected { tokens.text } else { tokens.muted },
        border: Border {
            color: if selected {
                tokens.line_strong
            } else {
                Color::TRANSPARENT
            },
            width: 1.0,
            radius: 5.0.into(),
        },
        shadow: if selected {
            Shadow {
                color: Color::from_rgba8(0, 0, 0, 0.35),
                offset: Vector::new(0.0, 1.0),
                blur_radius: 3.0,
            }
        } else {
            Shadow::default()
        },
        snap: true,
    }
}

pub(crate) fn add_tab_button_style(tokens: DesignTokens, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then_some(iced::Background::Color(tokens.panel_raised)),
        text_color: tokens.text,
        border: Border {
            color: if hovered {
                tokens.line_strong
            } else {
                Color::TRANSPARENT
            },
            width: 1.0,
            radius: 5.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub(crate) fn palette_button_style(
    tokens: DesignTokens,
    selected: bool,
    enabled: bool,
    status: button::Status,
) -> button::Style {
    let hovered = enabled && matches!(status, button::Status::Hovered | button::Status::Pressed);
    let background = if selected && enabled {
        Color {
            a: 0.14,
            ..tokens.accent
        }
    } else if hovered {
        tokens.panel
    } else {
        Color::TRANSPARENT
    };
    button::Style {
        background: Some(iced::Background::Color(background)),
        text_color: if enabled { tokens.text } else { tokens.faint },
        border: Border::default().rounded(5.0),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub(crate) fn ruled_surface(background: Color, line: Color) -> container::Style {
    container::Style::default()
        .background(background)
        .border(Border {
            color: line,
            width: 1.0,
            radius: 0.0.into(),
        })
}

pub(crate) fn modal_surface(tokens: DesignTokens) -> container::Style {
    container::Style::default()
        .background(tokens.overlay)
        .border(Border {
            color: tokens.line_strong,
            width: 1.0,
            radius: 8.0.into(),
        })
        .shadow(Shadow {
            color: Color::from_rgba8(0, 0, 0, 0.45),
            offset: Vector::new(0.0, 10.0),
            blur_radius: 28.0,
        })
}

pub(crate) fn centered_button_content<'a>(
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(content)
        .height(Fill)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

pub(crate) fn centered_button_label(label: &'static str, size: f32) -> Element<'static, Message> {
    centered_button_content(text(label).size(size))
}

pub(crate) fn app_tooltip<'a>(
    content: impl Into<Element<'a, Message>>,
    label: impl Into<String>,
    position: tooltip::Position,
    tokens: DesignTokens,
    font_size: f32,
) -> Element<'a, Message> {
    tooltip(
        content,
        text(label.into()).size(font_size).color(tokens.text),
        position,
    )
    .gap(6)
    .padding(8)
    .style(move |_| ruled_surface(tokens.overlay, tokens.line_strong))
    .into()
}

pub(crate) fn pane_icon_button(
    kind: IconKind,
    label: &'static str,
    message: Message,
    tokens: DesignTokens,
) -> Element<'static, Message> {
    app_tooltip(
        button(
            container(icon(kind, tokens.muted, 14.0))
                .width(Fill)
                .height(Fill)
                .center_x(Fill)
                .align_y(iced::alignment::Vertical::Center),
        )
        .on_press(message)
        .width(30)
        .height(28)
        .padding(0)
        .style(move |_, status| quiet_button_style(tokens, false, status)),
        label,
        tooltip::Position::Bottom,
        tokens,
        12.0,
    )
}

/// A context-menu row: label, optional trailing shortcut hint, and an
/// optional message — `None` renders the row disabled in place so menu
/// positions never shift with selection or process state.
pub(crate) fn pane_menu_entry(
    label: &'static str,
    hint: &'static str,
    message: Option<Message>,
    danger: bool,
    tokens: DesignTokens,
    settings: &AppSettings,
) -> Element<'static, Message> {
    let label_color = if message.is_none() {
        tokens.faint
    } else if danger {
        tokens.danger
    } else {
        tokens.text
    };
    let mut entry = button(centered_button_content(
        row![
            text(label)
                .size(settings.ui_pixels(9.0))
                .color(label_color)
                .width(Fill)
                .wrapping(iced::widget::text::Wrapping::None),
            text(hint)
                .font(settings.terminal_font.iced())
                .size(settings.ui_pixels(7.5))
                .color(tokens.faint)
                .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    ))
    .width(Fill)
    .height(30)
    .padding([0, 9])
    .style(move |_, status| pane_menu_entry_style(tokens, danger, status));
    if let Some(message) = message {
        entry = entry.on_press(message);
    }
    entry.into()
}

/// Menu rows sit on the raised panel, so the hover fill must come from a
/// different token than the panel itself or hovering is invisible.
pub(crate) fn pane_menu_entry_style(
    tokens: DesignTokens,
    danger: bool,
    status: button::Status,
) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let background = if !hovered {
        None
    } else if danger {
        Some(Color {
            a: 0.14,
            ..tokens.danger
        })
    } else {
        // A text-derived tint stays visible on the overlay surface, where
        // the line token would disappear into the fill.
        Some(Color {
            a: 0.08,
            ..tokens.text
        })
    };
    button::Style {
        background: background.map(iced::Background::Color),
        text_color: tokens.text,
        border: Border::default().rounded(4.0),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub(crate) fn pane_menu_divider(tokens: DesignTokens) -> Element<'static, Message> {
    container(
        container("")
            .height(1)
            .width(Fill)
            .style(move |_| container::Style::default().background(tokens.line)),
    )
    .padding([3, 4])
    .into()
}

#[derive(Clone, Copy)]
pub(crate) enum SettingsButtonKind {
    Primary,
    Secondary,
    Danger,
    /// Navigation that happens to be a button. It carries no fill or border at
    /// rest so it cannot compete with the page's real actions, and gains a
    /// surface only under the pointer.
    Quiet,
}

pub(crate) fn github_pull_request_summary_copy(
    pull_request: &github::PullRequestSummary,
    tokens: DesignTokens,
) -> (&'static str, &'static str, Color) {
    if pull_request.status == github::PullRequestSummaryStatus::Merged {
        (
            "Merged pull request",
            "This pull request has been merged",
            tokens.github_merged,
        )
    } else {
        github_readiness_copy(pull_request.readiness, tokens)
    }
}

pub(crate) fn github_readiness_icon(readiness: github::MergeReadiness) -> IconKind {
    match readiness {
        github::MergeReadiness::Ready => IconKind::StatusReady,
        github::MergeReadiness::Conflicts | github::MergeReadiness::ChecksFailed => {
            IconKind::StatusError
        }
        github::MergeReadiness::Behind
        | github::MergeReadiness::ChecksPending
        | github::MergeReadiness::ReviewRequired
        | github::MergeReadiness::Blocked => IconKind::StatusWarning,
        github::MergeReadiness::Draft | github::MergeReadiness::Unknown => IconKind::StatusInfo,
    }
}

pub(crate) fn github_readiness_copy(
    readiness: github::MergeReadiness,
    tokens: DesignTokens,
) -> (&'static str, &'static str, Color) {
    match readiness {
        github::MergeReadiness::Ready => (
            "Ready to merge",
            "Checks and review requirements are satisfied",
            tokens.success,
        ),
        github::MergeReadiness::Draft => (
            "Draft pull request",
            "Mark it ready for review on GitHub first",
            tokens.muted,
        ),
        github::MergeReadiness::Conflicts => (
            "Merge conflicts",
            "Resolve conflicts before merging",
            tokens.danger,
        ),
        github::MergeReadiness::Behind => (
            "Branch is behind",
            "Update the branch before merging",
            tokens.warning,
        ),
        github::MergeReadiness::ChecksPending => (
            "Checks are running",
            "Wait for required checks to finish",
            tokens.warning,
        ),
        github::MergeReadiness::ChecksFailed => (
            "Checks failed",
            "Fix the failing checks before merging",
            tokens.danger,
        ),
        github::MergeReadiness::ReviewRequired => (
            "Review required",
            "Approval is still required",
            tokens.warning,
        ),
        github::MergeReadiness::Blocked => (
            "Merge blocked",
            "GitHub reports an unmet branch rule",
            tokens.warning,
        ),
        github::MergeReadiness::Unknown => (
            "Readiness unknown",
            "Refresh after GitHub finishes calculating mergeability",
            tokens.muted,
        ),
    }
}

pub(crate) fn github_action_button_style(
    tokens: DesignTokens,
    keyboard_selected: bool,
    status: button::Status,
) -> button::Style {
    let mut style = settings_button_style(tokens, SettingsButtonKind::Secondary, status);
    if keyboard_selected && !matches!(status, button::Status::Disabled) {
        style.border.color = tokens.accent;
    }
    style
}

pub(crate) fn github_merge_button_style(
    tokens: DesignTokens,
    keyboard_selected: bool,
    status: button::Status,
) -> button::Style {
    if matches!(status, button::Status::Disabled) {
        return button::Style {
            background: Some(tokens.panel.into()),
            text_color: tokens.faint,
            border: Border {
                color: tokens.line,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..button::Style::default()
        };
    }
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(
            Color {
                a: if hovered { 0.88 } else { 1.0 },
                ..tokens.success
            }
            .into(),
        ),
        text_color: tokens.app,
        border: Border {
            color: if keyboard_selected {
                tokens.accent
            } else {
                tokens.success
            },
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub(crate) fn settings_button_style(
    tokens: DesignTokens,
    kind: SettingsButtonKind,
    status: button::Status,
) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    // An un-pressable button must not impersonate a live one.
    if matches!(status, button::Status::Disabled) {
        return button::Style {
            background: Some(tokens.panel.into()),
            text_color: tokens.faint,
            border: Border {
                color: tokens.line,
                width: 1.0,
                radius: 7.0.into(),
            },
            ..button::Style::default()
        };
    }
    let (background, text_color, border_color) = match kind {
        SettingsButtonKind::Primary => (
            if hovered {
                Color {
                    a: 0.86,
                    ..tokens.accent
                }
            } else {
                tokens.accent
            },
            tokens.app,
            tokens.accent,
        ),
        SettingsButtonKind::Secondary => (
            if hovered {
                tokens.panel_raised
            } else {
                tokens.panel
            },
            tokens.text,
            tokens.line_strong,
        ),
        SettingsButtonKind::Danger => (
            if hovered {
                Color {
                    a: 0.12,
                    ..tokens.danger
                }
            } else {
                Color {
                    a: 0.05,
                    ..tokens.danger
                }
            },
            tokens.danger,
            // tokens.line (6% white) rendered these borders invisible.
            if hovered {
                tokens.danger
            } else {
                Color {
                    a: 0.45,
                    ..tokens.danger
                }
            },
        ),
        SettingsButtonKind::Quiet => (
            if hovered {
                tokens.panel_raised
            } else {
                Color::TRANSPARENT
            },
            if hovered { tokens.text } else { tokens.muted },
            if hovered {
                tokens.line_strong
            } else {
                Color::TRANSPARENT
            },
        ),
    };
    button::Style {
        background: Some(iced::Background::Color(background)),
        text_color,
        border: Border {
            color: border_color,
            width: 1.0,
            radius: 5.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub(crate) fn settings_action_button(
    label: &'static str,
    message: Message,
    kind: SettingsButtonKind,
    settings: &AppSettings,
) -> Element<'static, Message> {
    settings_action_button_maybe(label, Some(message), kind, settings)
}

pub(crate) fn settings_have_changes(saved: &AppSettings, draft: &AppSettings) -> bool {
    draft != saved
}

pub(crate) fn settings_action_button_maybe(
    label: &'static str,
    message: Option<Message>,
    kind: SettingsButtonKind,
    settings: &AppSettings,
) -> Element<'static, Message> {
    let tokens = DesignTokens::for_appearance(settings.appearance);
    button(centered_button_label(label, settings.ui_pixels(9.0)))
        .on_press_maybe(message)
        .height(30)
        .padding([0, 11])
        .style(move |_, status| settings_button_style(tokens, kind, status))
        .into()
}

/// Whether the settings top bar has to shorten its return label.
///
/// The bar's label, title, and page switch are all typeset from one size, so
/// the width the bar needs scales with the interface type size: a window wide
/// enough at the default size can be too narrow once the type is scaled up.
pub(crate) fn settings_nav_is_crowded(window_width: f32, settings: &AppSettings) -> bool {
    window_width < SETTINGS_NAV_LABEL_WIDTHS * settings.ui_pixels(SETTINGS_NAV_LABEL_POINTS)
}

/// The Preferences/Worktrees page switcher, built on the same recessed-well
/// toggle the fleet heading uses: a dark track with one raised thumb, so the
/// current page is unambiguous without inventing a second selection idiom.
pub(crate) fn settings_page_toggle(
    current: SettingsPage,
    settings: &AppSettings,
) -> Element<'static, Message> {
    let tokens = DesignTokens::for_appearance(settings.appearance);
    let segment = |label: &'static str, page: SettingsPage| {
        let selected = current == page;
        button(centered_button_content(
            text(label)
                .size(settings.ui_pixels(9.5))
                .wrapping(iced::widget::text::Wrapping::None),
        ))
        .on_press(Message::OpenSettingsPage(page))
        .height(26)
        .padding([0, 13])
        .style(move |_, status| fleet_toggle_style(tokens, selected, status))
    };
    container(
        row![
            segment("Preferences", SettingsPage::Preferences),
            segment("Worktrees", SettingsPage::Worktrees),
        ]
        .spacing(2),
    )
    .padding(2)
    .style(move |_| {
        container::Style::default()
            .background(tokens.app)
            .border(Border {
                color: tokens.line,
                width: 1.0,
                radius: 7.0.into(),
            })
    })
    .into()
}

pub(crate) fn settings_notice<'a>(
    title: impl Into<String>,
    body: impl Into<String>,
    recovery: impl Into<String>,
    hue: Color,
    settings: &AppSettings,
) -> Element<'a, Message> {
    let tokens = DesignTokens::for_appearance(settings.appearance);
    let title_size = settings.ui_pixels(11.0);
    container(
        row![
            // Centred against the title's line box rather than the row's top
            // edge: aligned to Start the dot floated above the cap height of
            // the words it belongs to.
            container(signal_dot(hue, 7.0))
                .height(title_size * 1.3)
                .align_y(iced::alignment::Vertical::Center),
            column![
                text(title.into())
                    .size(title_size)
                    .font(Font {
                        weight: font::Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    .color(tokens.text),
                text(body.into())
                    .size(settings.ui_pixels(9.5))
                    .color(tokens.muted),
                text(recovery.into())
                    .size(settings.ui_pixels(8.5))
                    .color(hue),
            ]
            .spacing(3)
            .width(Fill),
        ]
        .spacing(11)
        .align_y(Alignment::Start),
    )
    .padding([12, 14])
    .width(Fill)
    .style(move |_| {
        container::Style::default()
            .background(Color { a: 0.06, ..hue })
            .border(Border {
                color: Color { a: 0.35, ..hue },
                width: 1.0,
                radius: 6.0.into(),
            })
    })
    .into()
}

pub(crate) fn worktree_status_tag(
    label: &str,
    hue: Color,
    settings: &AppSettings,
) -> Element<'static, Message> {
    container(
        row![
            signal_dot(hue, 5.0),
            text(label.to_owned())
                .size(settings.ui_pixels(8.0))
                .font(Font {
                    weight: font::Weight::Semibold,
                    ..Font::DEFAULT
                })
                .color(hue),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([3, 7])
    .style(move |_| {
        container::Style::default()
            .background(Color { a: 0.08, ..hue })
            .border(Border {
                color: Color { a: 0.28, ..hue },
                width: 1.0,
                radius: 4.0.into(),
            })
    })
    .into()
}

/// Lane widths for the worktree inventory, derived once from the table's real
/// width so the header, every row, and the ellipsis budgets cannot drift apart.
///
/// The three trailing lanes hold bounded copy — a tag, a count, one button —
/// so they stay fixed. Identity and branch are the two unbounded strings, and
/// they split whatever the window leaves rather than living in fixed boxes
/// that force a long branch to wrap while the window has room to spare.
#[derive(Clone, Copy)]
pub(crate) struct WorktreeLanes {
    pub(crate) identity: f32,
    pub(crate) branch: f32,
    pub(crate) status: f32,
    pub(crate) commits: f32,
    pub(crate) action: f32,
}

impl WorktreeLanes {
    const STATUS: f32 = 200.0;
    const COMMITS: f32 = 176.0;
    const ACTION: f32 = 104.0;
    /// Gap between the two lanes that share a line in the compact layout.
    pub(crate) const STACKED_GAP: f32 = 14.0;

    /// Width a row's content actually gets, once the page margins, the table
    /// border, the selection-bar gutter, the row padding, and the overlay
    /// scrollbar are all taken out.
    pub(crate) fn content_width(window_width: f32) -> f32 {
        let table =
            (window_width - 2.0 * SETTINGS_PAGE_PADDING_X).clamp(320.0, WORKTREE_PAGE_MAX_WIDTH);
        (table
            - 2.0 // the table's own 1px border
            - 3.0 // the row's leading selection bar
            - 2.0 * WORKTREE_ROW_PADDING_X
            - WORKTREE_SCROLLBAR_RESERVE)
            .max(240.0)
    }

    pub(crate) fn for_window(window_width: f32, compact: bool) -> Self {
        let content = Self::content_width(window_width);
        if compact {
            // Stacked layout: identity and the commit block each own a full
            // line, while branch and status share one.
            let half = ((content - Self::STACKED_GAP) / 2.0).max(120.0);
            return Self {
                identity: content,
                branch: half,
                status: half,
                commits: content,
                action: Self::ACTION,
            };
        }
        let flexible =
            (content - 4.0 * WORKTREE_LANE_SPACING - (Self::STATUS + Self::COMMITS + Self::ACTION))
                .max(240.0);
        // The path under the name is the longer of the two strings, so
        // identity keeps the larger share of the slack.
        let branch = (flexible * 0.4).floor();
        Self {
            identity: flexible - branch,
            branch,
            status: Self::STATUS,
            commits: Self::COMMITS,
            action: Self::ACTION,
        }
    }
}

/// Characters that fit in `width` of proportional UI copy at `size` pixels.
pub(crate) fn worktree_ui_budget(width: f32, size: f32) -> usize {
    pane_title_char_budget(width - WORKTREE_LANE_INSET, size * UI_TEXT_ADVANCE_RATIO)
}

/// Characters that fit in `width` of the configured monospace face.
pub(crate) fn worktree_mono_budget(width: f32, size: f32, settings: &AppSettings) -> usize {
    pane_title_char_budget(
        width - WORKTREE_LANE_INSET,
        size * settings.terminal_advance_ratio(),
    )
}

/// One keyboard affordance in the inventory footer: the real key drawn as a
/// keycap in the terminal face, then what it does. Same grammar as the command
/// palette's keycap, so the two teach the same thing the same way.
pub(crate) fn worktree_footer_hint(
    keys: &'static str,
    label: &'static str,
    settings: &AppSettings,
) -> Element<'static, Message> {
    let tokens = DesignTokens::for_appearance(settings.appearance);
    row![
        container(
            text(keys)
                .font(settings.terminal_font.iced())
                .size(settings.ui_pixels(8.5))
                .color(tokens.text)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .padding([1.0, 5.0])
        .style(move |_| {
            container::Style::default()
                .background(tokens.panel_raised)
                .border(Border {
                    color: tokens.line_strong,
                    width: 1.0,
                    radius: 4.0.into(),
                })
        }),
        text(label)
            .size(settings.ui_pixels(9.0))
            .color(tokens.muted)
            .wrapping(iced::widget::text::Wrapping::None),
    ]
    .spacing(7)
    .align_y(Alignment::Center)
    .into()
}

pub(crate) fn worktree_table_header(
    settings: &AppSettings,
    lanes: WorktreeLanes,
) -> Element<'static, Message> {
    let tokens = DesignTokens::for_appearance(settings.appearance);
    let label = |copy: &'static str| {
        text(copy)
            .size(settings.ui_pixels(8.0))
            .font(Font {
                weight: font::Weight::Semibold,
                ..Font::DEFAULT
            })
            .color(tokens.faint)
            .wrapping(iced::widget::text::Wrapping::None)
    };
    container(
        row![
            label("WORKTREE").width(lanes.identity),
            label("BRANCH").width(lanes.branch),
            label("STATUS").width(lanes.status),
            label("LOCAL COMMITS").width(lanes.commits),
            label("ACTION").width(lanes.action),
        ]
        .spacing(WORKTREE_LANE_SPACING),
    )
    // The extra 3px on the left absorbs the selection-bar gutter every row
    // reserves, so each label sits exactly over the copy it names.
    .padding(Padding {
        top: 9.0,
        bottom: 9.0,
        left: WORKTREE_ROW_PADDING_X + 3.0,
        right: WORKTREE_ROW_PADDING_X,
    })
    .into()
}

pub(crate) fn terminal_theme_preview(
    preset: TerminalThemePreset,
    settings: &AppSettings,
) -> Element<'static, Message> {
    terminal_theme_preview_with_caption(preset, settings, true)
}

pub(crate) fn terminal_theme_preview_with_caption(
    preset: TerminalThemePreset,
    settings: &AppSettings,
    caption: bool,
) -> Element<'static, Message> {
    let tokens = DesignTokens::for_appearance(settings.appearance);
    let terminal_font = settings.terminal_font.iced();
    let preview_size = settings.terminal_font_pixels().clamp(13.0, 18.0);
    let mode = if preset.is_light { "Light" } else { "Dark" };
    let sample_spans: Vec<iced::widget::text::Span<'static, (), Font>> = vec![
        span("❯ ").color(rgb(preset.ansi[10])),
        span("cargo test ").color(rgb(preset.foreground)),
        span("--workspace\n").color(rgb(preset.ansi[12])),
        span("   Compiling ").color(rgb(preset.ansi[3])),
        span("muxtrix\n").color(rgb(preset.foreground)),
        span("   Finished ").color(rgb(preset.ansi[2])),
        span("95 tests passed  ").color(rgb(preset.foreground)),
        span(" selected ")
            .color(rgb(preset.selection_foreground))
            .background(rgb(preset.selection_background)),
        span("  ").color(rgb(preset.foreground)),
        span(" C ")
            .color(rgb(preset.cursor_text))
            .background(rgb(preset.cursor)),
    ];
    let sample = rich_text(sample_spans)
        .font(terminal_font)
        .size(preview_size)
        .line_height(Pixels(preview_size * 1.35));

    let mut normal = row![].spacing(5);
    let mut bright = row![].spacing(5);
    for (index, color) in preset.ansi.into_iter().enumerate() {
        let swatch = container("")
            .width(Fill)
            .max_width(24)
            .height(12)
            .style(move |_| {
                container::Style::default()
                    .background(rgb(color))
                    .border(Border::default().rounded(2.0))
            });
        if index < 8 {
            normal = normal.push(swatch);
        } else {
            bright = bright.push(swatch);
        }
    }

    container(
        column![
            row![
                text(preset.name)
                    .size(settings.ui_pixels(11.0))
                    .font(Font {
                        weight: font::Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    .color(rgb(preset.foreground))
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None),
                if caption {
                    // In the gallery, section headings already say the mode;
                    // repeating a pill on every card is noise, and long
                    // names were clipping it against the card edge.
                    Element::from(
                        container(
                            text(mode)
                                .size(settings.ui_pixels(8.0))
                                .color(rgb(preset.background)),
                        )
                        .padding([2, 7])
                        .style(move |_| {
                            container::Style::default()
                                .background(rgb(preset.foreground))
                                .border(Border::default().rounded(4.0))
                        }),
                    )
                } else {
                    container("").width(0).into()
                },
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            sample,
            column![normal, bright].spacing(5),
            if caption {
                Element::from(
                    text("Theme colors set defaults · direct RGB and application OSC colors stay intact")
                        .size(settings.ui_pixels(8.0))
                        .color(rgb(preset.ansi[8])),
                )
            } else {
                container("").width(0).height(0).into()
            },
        ]
        .spacing(12),
    )
    .padding([14, 16])
    .width(Fill)
    .style(move |_| {
        container::Style::default()
            .background(rgb(preset.background))
            .border(Border {
                color: tokens.line_strong,
                width: 1.0,
                radius: 6.0.into(),
            })
    })
    .into()
}

pub(crate) fn settings_hook_button(
    label: &'static str,
    agent: Agent,
    action: HookAction,
    kind: SettingsButtonKind,
    settings: &AppSettings,
) -> Element<'static, Message> {
    settings_action_button(label, Message::ManageHooks(agent, action), kind, settings)
}

pub(crate) fn settings_divider(tokens: DesignTokens) -> Element<'static, Message> {
    container("")
        .width(Fill)
        .height(1)
        .style(move |_| container::Style::default().background(tokens.line))
        .into()
}

pub(crate) fn installed_version_restart_copy(state: &InstalledVersionsState) -> Option<String> {
    let InstalledVersionsState::Ready(versions) = state else {
        return None;
    };
    if let Ok(installed) = &versions.muxtrix
        && installed != env!("CARGO_PKG_VERSION")
    {
        return Some(format!(
            "Muxtrix v{installed} is installed; this window is still running v{}.",
            env!("CARGO_PKG_VERSION")
        ));
    }
    if let Ok(installed) = &versions.muxtrixctl
        && installed != muxtrix_control::VERSION
    {
        return Some(format!(
            "muxtrixctl v{installed} is installed; this window's control service is still running v{}.",
            muxtrix_control::VERSION
        ));
    }
    None
}

pub(crate) fn settings_version_value(
    running: &'static str,
    installed: Option<&Result<String, String>>,
    fallback: &'static str,
    settings: &AppSettings,
) -> Element<'static, Message> {
    let tokens = DesignTokens::for_appearance(settings.appearance);
    let (detail, detail_color) = match installed {
        Some(Ok(version)) if version == running => {
            ("Running · matches installed".to_owned(), tokens.muted)
        }
        Some(Ok(version)) => (format!("Running · v{version} installed"), tokens.warning),
        Some(Err(_)) => (
            "Running · installed binary unavailable".into(),
            tokens.faint,
        ),
        None => (fallback.to_owned(), tokens.muted),
    };
    column![
        text(format!("v{running}"))
            .font(settings.terminal_font.iced())
            .size(settings.ui_pixels(10.0))
            .color(tokens.text),
        text(detail)
            .size(settings.ui_pixels(8.5))
            .color(detail_color),
    ]
    .spacing(2)
    .align_x(Alignment::End)
    .into()
}

pub(crate) fn settings_row<'a>(
    label: &'static str,
    description: &'static str,
    control: impl Into<Element<'a, Message>>,
    settings: &AppSettings,
) -> Element<'a, Message> {
    let tokens = DesignTokens::for_appearance(settings.appearance);
    container(
        row![
            column![
                text(label)
                    .size(settings.ui_pixels(11.0))
                    .font(Font {
                        weight: font::Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    .color(tokens.text),
                text(description)
                    .size(settings.ui_pixels(9.0))
                    .color(tokens.muted),
            ]
            .spacing(2)
            .width(Fill),
            container(control).align_y(iced::alignment::Vertical::Center),
        ]
        .spacing(18)
        .align_y(Alignment::Center),
    )
    .padding([12, 14])
    .width(Fill)
    .into()
}

pub(crate) fn settings_section<'a>(
    title: &'static str,
    description: &'static str,
    content: iced::widget::Column<'a, Message>,
    settings: &AppSettings,
) -> Element<'a, Message> {
    let tokens = DesignTokens::for_appearance(settings.appearance);
    column![
        column![
            text(title)
                .size(settings.ui_pixels(13.0))
                .font(Font {
                    weight: font::Weight::Semibold,
                    ..Font::DEFAULT
                })
                .color(tokens.text),
            text(description)
                .size(settings.ui_pixels(9.0))
                .color(tokens.muted),
        ]
        .spacing(2),
        container(content).width(Fill).style(move |_| {
            container::Style::default()
                .background(tokens.panel)
                .border(Border {
                    color: tokens.line,
                    width: 1.0,
                    radius: 10.0.into(),
                })
        }),
    ]
    .spacing(8)
    .into()
}

#[derive(Default)]
pub(crate) struct RuntimePoll {
    pub(crate) status: Option<String>,
    pub(crate) notifications: Vec<TerminalNotification>,
    pub(crate) title: Option<String>,
    pub(crate) exited: bool,
    pub(crate) exited_clean: bool,
}

impl TerminalRuntime {
    pub(crate) fn with_state(
        preview: impl Into<String>,
        fallback_title: &str,
        session: Option<LiveSession>,
        viewport: Option<Size>,
        launch_state: TerminalLaunchState,
    ) -> Self {
        Self {
            preview: preview.into(),
            snapshot: None,
            snapshot_revision: 0,
            image_handles: BTreeMap::new(),
            session,
            fallback_title: fallback_title.into(),
            display_title: fallback_title.into(),
            size: initial_pty_size(),
            viewport,
            launch_state,
            has_selection: false,
        }
    }

    pub(crate) fn preparing_host(fallback_title: &str) -> Self {
        Self::with_state(
            "Preparing terminal host…\n\nThe workspace remains usable while this runs.",
            fallback_title,
            None,
            None,
            TerminalLaunchState::PreparingHost,
        )
    }

    pub(crate) fn suppressed(fallback_title: &str) -> Self {
        Self::with_state(
            "No terminal was started.\n\nYou can browse the workspace and start a terminal when the host is healthy.",
            fallback_title,
            None,
            None,
            TerminalLaunchState::Suppressed,
        )
    }

    pub(crate) fn starting(fallback_title: &str, attempt_id: u64, viewport: Option<Size>) -> Self {
        Self::with_state(
            "Starting terminal…\n\nThis pane can be cancelled without blocking the workspace.",
            fallback_title,
            None,
            viewport,
            TerminalLaunchState::Starting { attempt_id },
        )
    }

    pub(crate) fn launch(
        profile: &LaunchProfile,
        pane_id: PaneId,
        fallback_title: &str,
        max_scrollback: usize,
        theme: TerminalTheme,
        notifier: EventNotifier,
        control_endpoint: Option<&str>,
    ) -> (Self, String) {
        match start_live_session(
            profile,
            pane_id,
            max_scrollback,
            theme,
            notifier,
            control_endpoint,
        ) {
            Ok(session) => (
                Self::with_state(
                    "Starting local terminal…",
                    fallback_title,
                    Some(session),
                    None,
                    TerminalLaunchState::Running,
                ),
                "Live terminal — GPU compositor: Iced/wgpu".into(),
            ),
            Err(error) => {
                let preview = ghostty_preview().unwrap_or_else(|preview_error| {
                    format!(
                        "Live terminal failed: {error}\nGhostty preview failed: {preview_error}"
                    )
                });
                (
                    Self::with_state(
                        preview,
                        fallback_title,
                        None,
                        None,
                        TerminalLaunchState::Failed(error.clone()),
                    ),
                    format!("Terminal launch failed: {error}"),
                )
            }
        }
    }

    /// Attaches to a pane whose PTY already lives in a session daemon:
    /// no spawn request, just the byte stream (backlog first) into a
    /// fresh VT.
    pub(crate) fn attach(
        pane_id: PaneId,
        title: &str,
        max_scrollback: usize,
        theme: TerminalTheme,
        notifier: EventNotifier,
        client: Arc<muxtrix_sessions::SessionClient>,
        output: std::sync::mpsc::Receiver<Vec<u8>>,
    ) -> Self {
        let size = initial_pty_size();
        // Knock the PTY one row off before attaching: the first real layout
        // resize is then a guaranteed change, and the SIGWINCH it raises
        // makes full-screen applications repaint — clearing any artifacts
        // the backlog replay left behind.
        let _ = client.send(&muxtrix_sessions::Request::Resize {
            pane: pane_id.as_uuid(),
            rows: size.rows.saturating_sub(1),
            cols: size.cols,
        });
        let session = LiveSession::spawn_remote(
            Box::new(RemotePaneBackend {
                pane: pane_id.as_uuid(),
                client,
                output: Some(output),
            }),
            size,
            TerminalOptions {
                cols: size.cols,
                rows: size.rows,
                max_scrollback,
            },
            theme,
            Some(notifier),
        )
        .ok();
        let launch_state = if session.is_some() {
            TerminalLaunchState::Running
        } else {
            TerminalLaunchState::Failed("Could not attach to the terminal session".into())
        };
        Self::with_state("Reattaching…", title, session, None, launch_state)
    }

    pub(crate) fn set_snapshot(&mut self, snapshot: GridSnapshot) {
        let generations = snapshot
            .images
            .iter()
            .map(|placement| placement.image.generation)
            .collect::<BTreeSet<_>>();
        self.image_handles
            .retain(|generation, _| generations.contains(generation));
        for placement in &snapshot.images {
            self.image_handles
                .entry(placement.image.generation)
                .or_insert_with(|| {
                    ImageHandle::from_rgba(
                        placement.image.width,
                        placement.image.height,
                        Bytes::from_owner(Arc::clone(&placement.image.rgba)),
                    )
                });
        }
        self.snapshot_revision = self.snapshot_revision.wrapping_add(1);
        self.snapshot = Some(snapshot);
    }

    pub(crate) fn poll(&mut self) -> RuntimePoll {
        let mut poll = RuntimePoll::default();
        loop {
            let event = self
                .session
                .as_ref()
                .and_then(|session| session.try_recv().ok());
            match event {
                Some(LiveSessionEvent::Frame(snapshot)) => {
                    if !snapshot_matches_grid(&snapshot, self.size) {
                        continue;
                    }
                    let title = snapshot
                        .title
                        .clone()
                        .unwrap_or_else(|| self.fallback_title.clone());
                    if title != self.display_title {
                        self.display_title.clone_from(&title);
                        poll.title = Some(title);
                    }
                    self.set_snapshot(snapshot);
                }
                Some(LiveSessionEvent::Notification(notification)) => {
                    poll.notifications.push(notification);
                }
                Some(LiveSessionEvent::Exited { clean }) => {
                    poll.status = Some("Terminal process exited".into());
                    poll.exited = true;
                    poll.exited_clean = clean;
                    self.session.take();
                    self.launch_state = TerminalLaunchState::Exited;
                    break;
                }
                Some(LiveSessionEvent::Error(error)) => {
                    poll.status = Some(format!("Terminal error: {error}"));
                }
                None => break,
            }
        }
        poll
    }

    /// Anchors a selection at a grid cell. The emulator owns and tracks it
    /// from here, so nothing on this side records where it sits.
    pub(crate) fn selection_start(&mut self, cell: (u16, u16)) -> Result<(), String> {
        self.has_selection = false;
        self.session
            .as_ref()
            .ok_or_else(|| "terminal process has exited".to_owned())?
            .selection_start(cell.0, cell.1)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn selection_extend(&mut self, cell: (u16, u16)) -> Result<(), String> {
        self.has_selection = true;
        self.session
            .as_ref()
            .ok_or_else(|| "terminal process has exited".to_owned())?
            .selection_extend(cell.0, cell.1)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn selection_clear(&mut self) -> Result<(), String> {
        self.has_selection = false;
        let Some(session) = self.session.as_ref() else {
            return Ok(());
        };
        session.selection_clear().map_err(|error| error.to_string())
    }

    pub(crate) fn selection_text(&self) -> Option<String> {
        self.session
            .as_ref()?
            .selection_text()
            .ok()
            .flatten()
            .filter(|text| !text.is_empty())
    }

    pub(crate) fn wheel(&self, lines: isize, cell: Option<(u16, u16)>) -> Result<(), String> {
        self.session
            .as_ref()
            .ok_or_else(|| "terminal process has exited".to_owned())?
            .wheel(lines, cell)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn mouse(&self, event: TerminalMouseEvent) -> Result<(), String> {
        self.session
            .as_ref()
            .ok_or_else(|| "terminal process has exited".to_owned())?
            .mouse(event)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn scroll_to(&self, row: usize) -> Result<(), String> {
        self.session
            .as_ref()
            .ok_or_else(|| "terminal process has exited".to_owned())?
            .scroll_viewport_to(row)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn resize(&mut self, pane_size: Size, settings: &AppSettings) -> Result<(), String> {
        self.viewport = Some(pane_size);
        let size = pty_size_for_pane(pane_size, settings);
        if !terminal_grid_changed(self.size, size) {
            self.size = size;
            return Ok(());
        }
        let Some(session) = self.session.as_ref() else {
            self.size = size;
            return Ok(());
        };
        session
            .resize(
                size,
                settings.terminal_cell_width(),
                settings.terminal_cell_height(),
            )
            .map_err(|error| error.to_string())?;
        self.size = size;
        Ok(())
    }
}

pub(crate) fn snapshot_matches_grid(snapshot: &GridSnapshot, size: PtySize) -> bool {
    snapshot.cells.len() == usize::from(size.rows)
        && snapshot
            .cells
            .iter()
            .all(|row| row.len() == usize::from(size.cols))
}

pub(crate) fn terminal_grid_changed(previous: PtySize, next: PtySize) -> bool {
    previous.rows != next.rows || previous.cols != next.cols
}

pub(crate) fn terminal_scroll_lines(delta: ScrollDelta, cell_height: f32) -> isize {
    let lines = match delta {
        ScrollDelta::Lines { y, .. } => y * 3.0,
        ScrollDelta::Pixels { y, .. } => y / cell_height.max(1.0),
    };
    if lines > 0.0 {
        -lines.ceil() as isize
    } else if lines < 0.0 {
        (-lines).ceil() as isize
    } else {
        0
    }
}

pub(crate) fn terminal_cell_at(
    position: Point,
    settings: &AppSettings,
    scroll_offset: u64,
) -> TerminalCellPosition {
    TerminalCellPosition {
        row: scroll_offset.saturating_add(
            ((position.y - 8.0).max(0.0) / settings.terminal_cell_height()).floor() as u64,
        ),
        column: ((position.x - 8.0).max(0.0) / settings.terminal_cell_width()).floor() as usize,
    }
}

/// The visible grid cell under a pointer, clamped to the grid. Selection is
/// expressed to the emulator in viewport coordinates, which is the only space
/// a pointer position can speak to.
pub(crate) fn terminal_grid_cell_at(
    position: Point,
    settings: &AppSettings,
    size: PtySize,
) -> (u16, u16) {
    let column = ((position.x - TERMINAL_PADDING / 2.0).max(0.0) / settings.terminal_cell_width())
        .floor()
        .clamp(0.0, f32::from(size.cols.saturating_sub(1))) as u16;
    let row = ((position.y - TERMINAL_PADDING / 2.0).max(0.0) / settings.terminal_cell_height())
        .floor()
        .clamp(0.0, f32::from(size.rows.saturating_sub(1))) as u16;
    (column, row)
}

pub(crate) fn terminal_mouse_event(
    position: Point,
    action: TerminalMouseAction,
    button: Option<TerminalMouseButton>,
    modifiers: Modifiers,
) -> TerminalMouseEvent {
    TerminalMouseEvent {
        action,
        button,
        x: position.x - TERMINAL_PADDING / 2.0,
        y: position.y - TERMINAL_PADDING / 2.0,
        shift: modifiers.shift(),
        alt: modifiers.alt(),
        control: modifiers.control(),
    }
}

pub(crate) fn terminal_selection_drag_started(origin: Point, current: Point) -> bool {
    let x = current.x - origin.x;
    let y = current.y - origin.y;
    x * x + y * y >= TERMINAL_SELECTION_DRAG_THRESHOLD * TERMINAL_SELECTION_DRAG_THRESHOLD
}

pub(crate) fn terminal_link_modifiers(modifiers: Modifiers) -> bool {
    modifiers.control() && modifiers.shift() && !modifiers.alt() && !modifiers.logo()
}

pub(crate) fn terminal_mouse_interaction(link_hovered: bool) -> mouse::Interaction {
    if link_hovered {
        mouse::Interaction::Pointer
    } else {
        mouse::Interaction::Idle
    }
}

pub(crate) fn terminal_link_at(
    snapshot: &GridSnapshot,
    position: TerminalCellPosition,
) -> Option<TerminalLink> {
    let viewport_row =
        usize::try_from(position.row.checked_sub(snapshot.scrollbar.offset)?).ok()?;
    let cells = snapshot.cells.get(viewport_row)?;
    let cell = cells.get(position.column)?;

    if let Some(uri) = cell.hyperlink.as_deref().and_then(valid_web_url) {
        let mut start_column = position.column;
        while start_column > 0 && cells[start_column - 1].hyperlink.as_deref() == Some(uri.as_str())
        {
            start_column -= 1;
        }
        let mut end_column = position.column + 1;
        while end_column < cells.len()
            && cells[end_column].hyperlink.as_deref() == Some(uri.as_str())
        {
            end_column += 1;
        }
        return Some(TerminalLink {
            uri,
            row: position.row,
            start_column,
            end_column,
        });
    }

    let ascii_row = cells
        .iter()
        .map(|cell| {
            let bytes = cell.text.as_bytes();
            if cell.columns == 1 && bytes.len() == 1 && bytes[0].is_ascii() {
                bytes[0]
            } else {
                b' '
            }
        })
        .collect::<Vec<_>>();
    detected_web_links(&ascii_row)
        .into_iter()
        .find(|(_, start, end)| position.column >= *start && position.column < *end)
        .map(|(uri, start_column, end_column)| TerminalLink {
            uri,
            row: position.row,
            start_column,
            end_column,
        })
}

pub(crate) fn detected_web_links(row: &[u8]) -> Vec<(String, usize, usize)> {
    detected_web_link_ranges(row)
        .into_iter()
        .filter_map(|(start, end)| {
            std::str::from_utf8(&row[start..end])
                .ok()
                .map(|uri| (uri.to_owned(), start, end))
        })
        .collect()
}

pub(crate) fn detected_web_link_ranges(row: &[u8]) -> Vec<(usize, usize)> {
    let mut links = Vec::new();
    let mut index = 0;
    while index < row.len() {
        let scheme_length = if row[index..].starts_with(b"https://") {
            8
        } else if row[index..].starts_with(b"http://") {
            7
        } else {
            index += 1;
            continue;
        };
        if index > 0 && row[index - 1].is_ascii_alphanumeric() {
            index += scheme_length;
            continue;
        }
        let mut end = index + scheme_length;
        while end < row.len() && !is_url_boundary(row[end]) {
            end += 1;
        }
        end = trim_url_end(row, index, end);
        if end > index + scheme_length
            && let Ok(candidate) = std::str::from_utf8(&row[index..end])
            && is_valid_web_url(candidate)
        {
            links.push((index, end));
        }
        index = end.max(index + scheme_length);
    }
    links
}

pub(crate) fn is_url_boundary(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'<' | b'>' | b'"' | b'\'' | b'`')
}

pub(crate) fn trim_url_end(row: &[u8], start: usize, mut end: usize) -> usize {
    while end > start && matches!(row[end - 1], b'.' | b',' | b';' | b':' | b'!' | b'?') {
        end -= 1;
    }
    for (open, close) in [(b'(', b')'), (b'[', b']'), (b'{', b'}')] {
        while end > start
            && row[end - 1] == close
            && row[start..end]
                .iter()
                .filter(|byte| **byte == close)
                .count()
                > row[start..end].iter().filter(|byte| **byte == open).count()
        {
            end -= 1;
        }
    }
    end
}

pub(crate) fn valid_web_url(candidate: &str) -> Option<String> {
    is_valid_web_url(candidate).then(|| candidate.to_owned())
}

pub(crate) fn is_valid_web_url(candidate: &str) -> bool {
    let Some(rest) = candidate
        .strip_prefix("https://")
        .or_else(|| candidate.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !authority.is_empty()
        && authority.bytes().any(|byte| byte.is_ascii_alphanumeric())
        && candidate
            .bytes()
            .all(|byte| !byte.is_ascii_control() && !byte.is_ascii_whitespace())
}

pub(crate) fn open_web_url(uri: &str) -> std::io::Result<()> {
    if valid_web_url(uri).is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "only http and https URLs can be opened",
        ));
    }

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = console_command("rundll32.exe");
        command.args(["url.dll,FileProtocolHandler", uri]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = console_command("open");
        command.arg(uri);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = console_command("xdg-open");
        command.arg(uri);
        command
    };

    command.spawn().map(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TerminalScrollbarGeometry {
    pub(crate) track_top: f32,
    pub(crate) track_height: f32,
    pub(crate) thumb_top: f32,
    pub(crate) thumb_height: f32,
    pub(crate) max_offset: u64,
}

impl TerminalScrollbarGeometry {
    pub(crate) fn offset_for_thumb_top(self, thumb_top: f32) -> u64 {
        let travel = (self.track_height - self.thumb_height).max(0.0);
        if self.max_offset == 0 || travel == 0.0 {
            return 0;
        }
        ((thumb_top.clamp(0.0, travel) / travel) * self.max_offset as f32).round() as u64
    }
}

pub(crate) fn terminal_scrollbar_geometry(
    scrollbar: ScrollbarSnapshot,
    viewport_height: f32,
) -> TerminalScrollbarGeometry {
    const TRACK_INSET: f32 = 5.0;
    const MIN_THUMB_HEIGHT: f32 = 24.0;

    let total = scrollbar.total.max(1);
    let visible = scrollbar.visible.min(total);
    let track_height = (viewport_height - TRACK_INSET * 2.0).max(1.0);
    let thumb_height = ((visible as f32 / total as f32) * track_height)
        .clamp(MIN_THUMB_HEIGHT.min(track_height), track_height);
    let travel = (track_height - thumb_height).max(0.0);
    let max_offset = total.saturating_sub(visible);
    let thumb_top = if max_offset == 0 {
        0.0
    } else {
        (scrollbar.offset.min(max_offset) as f32 / max_offset as f32) * travel
    };
    TerminalScrollbarGeometry {
        track_top: TRACK_INSET,
        track_height,
        thumb_top,
        thumb_height,
        max_offset,
    }
}

pub(crate) fn terminal_scrollbar(
    pane_id: PaneId,
    scrollbar: ScrollbarSnapshot,
    viewport_height: f32,
    tokens: DesignTokens,
) -> Element<'static, Message> {
    let geometry = terminal_scrollbar_geometry(scrollbar, viewport_height);
    let track = column![
        container("").height(Length::Fixed(geometry.thumb_top)),
        container("")
            .height(Length::Fixed(geometry.thumb_height))
            .width(3)
            .style(move |_| {
                container::Style::default()
                    .background(tokens.line_strong)
                    .border(Border::default().rounded(2.0))
            }),
        container("").height(Fill),
    ]
    .height(Fill)
    .align_x(Alignment::End);
    let hit_target = mouse_area(
        container(track)
            .width(12)
            .height(Fill)
            .align_x(iced::alignment::Horizontal::Right)
            .padding(Padding {
                top: geometry.track_top,
                right: 3.0,
                bottom: geometry.track_top,
                left: 0.0,
            }),
    )
    .on_move(move |position| Message::TerminalScrollbarMoved(pane_id, position.into()))
    .on_press(Message::BeginTerminalScroll(pane_id))
    // A scrollbar is not a draggable object: the grab interaction renders as
    // the four-direction move cross on some platforms. Plain arrow.
    .interaction(mouse::Interaction::Idle);
    container(hit_target)
        .width(Fill)
        .height(Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .into()
}

pub(crate) fn pane_header_is_compact(window_width: f32, pane_count: usize) -> bool {
    window_width < 1_080.0 || pane_count > 2
}

/// Characters the pane header title may render in `available` pixels.
///
/// The row lays every element out at its natural width, so a title measured
/// too generously does not truncate — it pushes the state label and controls
/// toward and past the card's right edge.
pub(crate) fn pane_title_char_budget(available: f32, character_width: f32) -> usize {
    ((available.max(0.0) / character_width.max(1.0)).floor() as usize).max(1)
}

pub(crate) fn event_subscription(
    subscription: &EventSubscription,
) -> impl iced::futures::Stream<Item = Message> + use<> {
    subscription.0.clone().map(|()| Message::PollTerminal)
}

pub(crate) fn app_event(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        // Text inputs and focused buttons own captured keys. Forwarding them
        // to the global keyboard handler as well would type into the terminal
        // underneath the GitHub search field or double-trigger controls.
        iced::Event::Keyboard(event) => {
            let event = input::from_iced(&event);
            // Tab and Escape are claimed even when a widget captured them:
            // they close the palette and move pane focus, and a focused text
            // input would otherwise swallow both.
            let always_ours = matches!(
                &event,
                KeyEvent::Pressed(KeyInput {
                    modified_key: Key::Named(Named::Tab | Named::Escape),
                    ..
                })
            );
            (status == iced::event::Status::Ignored || always_ours)
                .then_some(Message::Keyboard(event))
        }
        iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::PointerMoved(position.into()))
        }
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            Some(Message::EndPointerInteraction)
        }
        iced::Event::Mouse(mouse::Event::WheelScrolled { delta })
            if status == iced::event::Status::Ignored =>
        {
            Some(Message::ScrollHoveredTerminal(delta.into()))
        }
        iced::Event::Window(iced::window::Event::Opened { size, .. }) => {
            Some(Message::WindowOpened(size.into()))
        }
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            Some(Message::WindowResized(size.into()))
        }
        iced::Event::Window(iced::window::Event::Focused) => {
            Some(Message::WindowFocusChanged(true))
        }
        iced::Event::Window(iced::window::Event::Unfocused) => {
            Some(Message::WindowFocusChanged(false))
        }
        _ => None,
    }
}

pub(crate) fn terminal_empty_state_copy(runtime: Option<&TerminalRuntime>) -> Option<&str> {
    runtime.and_then(|runtime| {
        matches!(
            runtime.launch_state,
            TerminalLaunchState::Failed(_) | TerminalLaunchState::Suppressed
        )
        .then_some(runtime.preview.as_str())
    })
}

pub(crate) fn terminal_surface_background(
    snapshot: Option<&GridSnapshot>,
    theme: TerminalThemePreset,
) -> muxtrix_terminal::Rgb {
    snapshot.map_or(theme.background, |snapshot| snapshot.default_background)
}

pub(crate) fn styled_terminal(
    snapshot: &GridSnapshot,
    image_handles: &BTreeMap<u64, ImageHandle>,
    focused: bool,
    cursor_phase_visible: bool,
    hovered_link: Option<&TerminalLink>,
    settings: &AppSettings,
) -> Element<'static, Message> {
    let theme = settings.terminal_theme.preset();
    let cell_width = settings.terminal_cell_width();
    let cell_height = settings.terminal_cell_height();
    let font_size = settings.terminal_font_pixels();
    let terminal_font = settings.terminal_font.iced();
    let family = settings.terminal_font.family_name();
    let cell_ratio = settings.terminal_advance_ratio();
    // A bold face may be wider than the regular one it shares a grid with. The
    // grid stays uniform, so shrink bold text instead of letting it overrun.
    let bold_scale = bold_size_scale(settings, cell_ratio);
    let mut backgrounds = column![].spacing(0);
    let mut overlays = column![].spacing(0);
    let mut text_grid = column![].spacing(0);
    for runs in
        terminal_row_style_runs(snapshot, focused, cursor_phase_visible, hovered_link, theme)
    {
        let mut background_line = row![].spacing(0).height(Length::Fixed(cell_height));
        let mut overlay_line = row![].spacing(0).height(Length::Fixed(cell_height));
        let mut text_line = row![].spacing(0).height(Length::Fixed(cell_height));
        for run in runs.into_iter().filter(|run| run.columns > 0) {
            let alpha = if run.style.faint { 0.6 } else { 1.0 };
            let foreground = if run.style.selected {
                theme.selection_foreground
            } else {
                run.style.foreground
            };
            let foreground_color =
                Color::from_rgba8(foreground.red, foreground.green, foreground.blue, alpha);
            let background = run.style.background;
            let overlay_background = run.style.overlay_background;
            let run_width = cell_width * run.columns as f32;
            background_line = background_line.push(
                container("")
                    .width(Length::Fixed(run_width))
                    .height(Length::Fixed(cell_height))
                    .style(move |_| {
                        background.map_or_else(container::Style::default, |background| {
                            container::Style::default().background(rgb(background))
                        })
                    }),
            );
            overlay_line = overlay_line.push(
                container("")
                    .width(Length::Fixed(run_width))
                    .height(Length::Fixed(cell_height))
                    .style(move |_| {
                        overlay_background.map_or_else(container::Style::default, |background| {
                            container::Style::default().background(rgb(background))
                        })
                    }),
            );
            if run.kind == TerminalRunKind::BoxDrawing {
                let content = canvas(box_drawing::BoxDrawingRun::new(
                    run.text,
                    cell_width,
                    cell_height,
                    foreground_color,
                ))
                .width(Length::Fixed(run_width))
                .height(Length::Fixed(cell_height));
                text_line = text_line.push(
                    container(content)
                        .width(Length::Fixed(run_width))
                        .height(Length::Fixed(cell_height))
                        .clip(true),
                );
                continue;
            }
            let weight = if run.style.bold {
                settings.terminal_font_weight.bold_variant()
            } else {
                settings.terminal_font_weight.iced()
            };
            // Only Unicode runs can miss the configured face; ASCII runs are
            // guaranteed present and skip the lookup entirely.
            let fallback = run.kind.needs_fallback().then_some(()).and_then(|()| {
                metrics::glyph_fallback(
                    family,
                    settings::weight_numeric(weight),
                    &run.text,
                    cell_ratio,
                    settings.terminal_line_height,
                )
            });
            // A substitute is requested at the weight and style its own face
            // ships. Shaping drops the family rather than relaxing either, so a
            // single-weight face silently falls through to another family.
            let (base_font, run_weight, run_style) = match fallback {
                Some(fallback) => (
                    Font::with_name(fallback.family),
                    settings::weight_from_numeric(fallback.weight),
                    font::Style::Normal,
                ),
                None => (
                    terminal_font,
                    weight,
                    if run.style.italic {
                        font::Style::Italic
                    } else {
                        font::Style::Normal
                    },
                ),
            };
            let size_scale = fallback.map_or_else(
                || if run.style.bold { bold_scale } else { 1.0 },
                |fallback| fallback.size_scale,
            );
            // A substituted glyph is placed by its run's line height, which only
            // moves it while the paragraph is top-aligned; centering it in the
            // cell cancels the term out.
            let line_height =
                fallback.map_or(cell_height, |fallback| font_size * fallback.line_height_em);
            let geometry = terminal_run_geometry(&run);
            let underline_decoration = terminal_underline_decoration(run.style);
            let run_span: iced::widget::text::Span<'static, (), Font> = span(run.text)
                .color(foreground_color)
                .size(font_size * size_scale)
                .font(font_with_style(base_font, run_weight, run_style))
                .underline(underline_decoration == TerminalUnderlineDecoration::Solid)
                .strikethrough(run.style.strikethrough);
            let mut content = rich_text(vec![run_span])
                .size(font_size)
                .line_height(Pixels(line_height))
                .font(terminal_font)
                .wrapping(iced::widget::text::Wrapping::None);
            if fallback.is_some_and(|fallback| fallback.color) {
                // A colour glyph is drawn wider than its cell, and a container
                // cannot offset content it does not fit. Centring at shaping
                // time is what produces the negative offset it needs.
                content = content
                    .width(Length::Fixed(cell_width * run.columns as f32))
                    .align_x(iced::alignment::Horizontal::Center);
            }
            let vertical = if fallback.is_some() {
                iced::alignment::Vertical::Top
            } else {
                iced::alignment::Vertical::Center
            };
            let horizontal = if fallback.is_some() {
                iced::alignment::Horizontal::Center
            } else {
                iced::alignment::Horizontal::Left
            };
            let content: Element<'static, Message> = match geometry {
                Some(TerminalRunGeometry::FullBlock) => {
                    // U+2588 means the whole terminal cell, while many fonts
                    // leave a fractional side bearing around its outline.
                    // Drawing that semantic cell area directly keeps progress
                    // bars solid across fractional GPU clip boundaries.
                    container("")
                        .width(Length::Fixed(run_width))
                        .height(Length::Fixed(cell_height))
                        .style(move |_| container::Style::default().background(foreground_color))
                        .into()
                }
                None if underline_decoration == TerminalUnderlineDecoration::Dotted => {
                    const LINK_DOT_SIZE: f32 = 5.0;
                    let dot_advance = (cell_ratio * LINK_DOT_SIZE).max(1.0);
                    let dot_count = (run_width / dot_advance).ceil() as usize;
                    let dots = container(
                        text(".".repeat(dot_count))
                            .font(terminal_font)
                            .size(LINK_DOT_SIZE)
                            .line_height(Pixels(LINK_DOT_SIZE))
                            .color(Color::from_rgba8(
                                foreground.red,
                                foreground.green,
                                foreground.blue,
                                alpha,
                            ))
                            .wrapping(iced::widget::text::Wrapping::None),
                    )
                    .width(Length::Fixed(run_width))
                    .height(Length::Fixed(cell_height))
                    .align_x(iced::alignment::Horizontal::Left)
                    .align_y(iced::alignment::Vertical::Bottom)
                    .clip(true);
                    stack([content.into(), dots.into()]).into()
                }
                _ => content.into(),
            };
            text_line = text_line.push(
                container(content)
                    .width(Length::Fixed(run_width))
                    .height(Length::Fixed(cell_height))
                    .align_x(horizontal)
                    .align_y(vertical)
                    // A colour glyph is sized to match the text beside it, which
                    // needs marginally more than one cell on a square canvas.
                    .clip(!fallback.is_some_and(|fallback| fallback.color)),
            );
        }
        backgrounds = backgrounds.push(background_line);
        overlays = overlays.push(overlay_line);
        text_grid = text_grid.push(text_line);
    }
    stack([
        terminal_image::layer(
            snapshot,
            image_handles,
            ImageLayer::BelowBackground,
            cell_width,
            cell_height,
        ),
        backgrounds.into(),
        terminal_image::layer(
            snapshot,
            image_handles,
            ImageLayer::BelowText,
            cell_width,
            cell_height,
        ),
        overlays.into(),
        text_grid.into(),
        terminal_image::layer(
            snapshot,
            image_handles,
            ImageLayer::AboveText,
            cell_width,
            cell_height,
        ),
    ])
    .into()
}

pub(crate) fn pty_size_for_pane(size: Size, settings: &AppSettings) -> PtySize {
    let cell_width = settings.terminal_cell_width();
    let cell_height = settings.terminal_cell_height();
    let width = (size.width - TERMINAL_PADDING).max(cell_width * 2.0);
    let height = (size.height - TERMINAL_PADDING).max(cell_height * 2.0);
    PtySize {
        rows: (height / cell_height)
            .floor()
            .clamp(2.0, f32::from(u16::MAX)) as u16,
        cols: (width / cell_width).floor().clamp(2.0, f32::from(u16::MAX)) as u16,
        pixel_width: width.round().clamp(0.0, f32::from(u16::MAX)) as u16,
        pixel_height: height.round().clamp(0.0, f32::from(u16::MAX)) as u16,
    }
}

pub(crate) fn wsl_wayland_resize_increments(
    is_wsl: bool,
    wayland_is_available: bool,
    x11_is_forced: bool,
    settings: &AppSettings,
) -> Option<Size> {
    if !is_wsl || !wayland_is_available || x11_is_forced {
        return None;
    }

    Some(Size::new(
        settings.terminal_cell_width().round().clamp(4.0, 32.0),
        settings.terminal_cell_height().round().clamp(4.0, 32.0),
    ))
}

pub(crate) fn initial_pty_size() -> PtySize {
    PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 800,
        pixel_height: 480,
    }
}

pub(crate) fn encode_terminal_key(
    key: Key<&str>,
    modifiers: Modifiers,
    text: Option<&str>,
) -> Option<Vec<u8>> {
    if modifiers.logo() {
        return None;
    }

    // Ctrl+Alt commonly represents AltGr. Prefer the composed text when the
    // platform supplies it instead of turning it into a control sequence.
    if modifiers.control()
        && modifiers.alt()
        && let Some(text) = text.filter(|text| !text.is_empty())
    {
        return Some(text.as_bytes().to_vec());
    }

    let mut bytes = match key {
        Key::Character(character) if modifiers.control() => {
            vec![control_byte(character)?]
        }
        Key::Character(character) => text
            .filter(|text| !text.is_empty())
            .unwrap_or(character)
            .as_bytes()
            .to_vec(),
        // Agent prompts treat a line feed as "insert newline" (Ctrl+J), so
        // Ctrl+Enter extends the prompt instead of submitting it.
        Key::Named(Named::Enter) if modifiers.control() => vec![b'\n'],
        Key::Named(Named::Enter) => vec![b'\r'],
        Key::Named(Named::Space) if modifiers.control() => vec![0x00],
        Key::Named(Named::Space) => text
            .filter(|text| !text.is_empty())
            .unwrap_or(" ")
            .as_bytes()
            .to_vec(),
        Key::Named(Named::Tab) if modifiers.shift() => b"\x1b[Z".to_vec(),
        Key::Named(Named::Tab) => vec![b'\t'],
        Key::Named(Named::Backspace) if modifiers.control() => vec![0x08],
        Key::Named(Named::Backspace) => vec![0x7f],
        Key::Named(Named::Escape) => vec![0x1b],
        Key::Named(Named::ArrowUp) => modified_csi('A', modifiers),
        Key::Named(Named::ArrowDown) => modified_csi('B', modifiers),
        Key::Named(Named::ArrowRight) => modified_csi('C', modifiers),
        Key::Named(Named::ArrowLeft) => modified_csi('D', modifiers),
        Key::Named(Named::Home) => modified_csi('H', modifiers),
        Key::Named(Named::End) => modified_csi('F', modifiers),
        Key::Named(Named::Insert) => b"\x1b[2~".to_vec(),
        Key::Named(Named::Delete) => b"\x1b[3~".to_vec(),
        Key::Named(Named::PageUp) => b"\x1b[5~".to_vec(),
        Key::Named(Named::PageDown) => b"\x1b[6~".to_vec(),
        Key::Named(Named::F1) => b"\x1bOP".to_vec(),
        Key::Named(Named::F2) => b"\x1bOQ".to_vec(),
        Key::Named(Named::F3) => b"\x1bOR".to_vec(),
        Key::Named(Named::F4) => b"\x1bOS".to_vec(),
        Key::Named(Named::F5) => b"\x1b[15~".to_vec(),
        Key::Named(Named::F6) => b"\x1b[17~".to_vec(),
        Key::Named(Named::F7) => b"\x1b[18~".to_vec(),
        Key::Named(Named::F8) => b"\x1b[19~".to_vec(),
        Key::Named(Named::F9) => b"\x1b[20~".to_vec(),
        Key::Named(Named::F10) => b"\x1b[21~".to_vec(),
        Key::Named(Named::F11) => b"\x1b[23~".to_vec(),
        Key::Named(Named::F12) => b"\x1b[24~".to_vec(),
        _ => return None,
    };

    if modifiers.alt() {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

pub(crate) fn control_byte(character: &str) -> Option<u8> {
    let character = character.chars().next()?;
    match character.to_ascii_lowercase() {
        '@' | ' ' => Some(0x00),
        'a'..='z' => Some((character.to_ascii_lowercase() as u8) - b'a' + 1),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

pub(crate) fn modified_csi(final_byte: char, modifiers: Modifiers) -> Vec<u8> {
    let modifier = 1
        + u8::from(modifiers.shift())
        + 2 * u8::from(modifiers.alt())
        + 4 * u8::from(modifiers.control());
    if modifier == 1 {
        format!("\x1b[{final_byte}").into_bytes()
    } else {
        format!("\x1b[1;{modifier}{final_byte}").into_bytes()
    }
}

pub(crate) fn agent_state_label(state: AgentState) -> &'static str {
    match state {
        // A finished turn and a never-started one are the same thing to the
        // person reading the row — an agent sitting at its composer waiting for
        // them. The two stay distinct internally, where the difference decides
        // attention, signal colour, and which evidence may transition the pane.
        AgentState::Idle | AgentState::Completed => "Idle",
        AgentState::Running => "Running",
        AgentState::Waiting => "Needs input",
        AgentState::Failed => "Failed",
        AgentState::Stopped => "Stopped",
    }
}

pub(crate) fn screen_state(state: agent_screen::ScreenState) -> AgentState {
    match state {
        agent_screen::ScreenState::Waiting => AgentState::Waiting,
        agent_screen::ScreenState::Running => AgentState::Running,
        agent_screen::ScreenState::Idle => AgentState::Idle,
    }
}

/// What a screen-classified pane is doing. Only the live states have an answer
/// here: completion, failure, and stopping are lifecycle-owned and arrive with
/// the hook's own body.
pub(crate) fn agent_state_activity(state: agent_screen::ScreenState) -> &'static str {
    match state {
        agent_screen::ScreenState::Waiting => "Visible approval or answer required",
        agent_screen::ScreenState::Running => "Agent is working",
        agent_screen::ScreenState::Idle => "Ready for input",
    }
}

pub(crate) fn pane_agent(agent: &str) -> Option<PaneAgent> {
    match agent.to_ascii_lowercase().as_str() {
        "codex" => Some(PaneAgent::Codex),
        "claude" | "claude-code" => Some(PaneAgent::ClaudeCode),
        "pi" | "omp" | "oh-my-pi" => Some(PaneAgent::OhMyPi),
        _ => None,
    }
}

pub(crate) fn pane_agent_name(agent: PaneAgent) -> &'static str {
    match agent {
        PaneAgent::Codex => "codex",
        PaneAgent::ClaudeCode => "claude",
        PaneAgent::OhMyPi => "pi",
    }
}

pub(crate) fn session_with_agent_identities(
    session: &SessionState,
    statuses: &BTreeMap<PaneId, AgentPaneStatus>,
) -> SessionState {
    let mut persisted = session.clone();
    for workspace in &mut persisted.workspaces {
        for tab in &mut workspace.tabs {
            for pane in tab.panes.values_mut() {
                pane.agent = statuses
                    .get(&pane.id)
                    .and_then(|status| pane_agent(&status.agent));
            }
        }
    }
    persisted
}

pub(crate) fn agent_statuses_from_session(
    session: &SessionState,
) -> BTreeMap<PaneId, AgentPaneStatus> {
    session
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.tabs)
        .flat_map(|tab| tab.panes.values())
        .filter_map(|pane| {
            let agent = pane_agent_name(pane.agent?).to_owned();
            let display_name = pane
                .active_surface()
                .and_then(|surface| harness_terminal_title(&surface.title, &agent));
            Some((
                pane.id,
                AgentPaneStatus {
                    agent,
                    display_name,
                    state: AgentState::Idle,
                    activity: Some(agent_state_activity(agent_screen::ScreenState::Idle).into()),
                    session_id: None,
                    cwd: None,
                    git_branch: None,
                },
            ))
        })
        .collect()
}

pub(crate) fn agent_display_name(agent: &str) -> &str {
    match agent {
        "codex" => "Codex",
        "claude" | "claude-code" => "Claude Code",
        "pi" | "omp" | "oh-my-pi" => "Oh My Pi",
        _ => agent,
    }
}

pub(crate) fn agent_command_setting(settings: &AppSettings, agent: Agent) -> &str {
    match agent {
        Agent::Codex => &settings.codex_command,
        Agent::Claude => &settings.claude_command,
        Agent::Pi => &settings.pi_command,
    }
}

/// Harnesses publish their own session/thread names through OSC terminal-title
/// metadata. Exact brand-only titles add no identity, so retain the worktree
/// fallback until the harness emits something more useful.
pub(crate) fn harness_terminal_title(title: &str, agent: &str) -> Option<String> {
    // Titles that name the harness's current view rather than its work. The
    // roster and the `current session` label it leaves behind on the way out
    // would otherwise rename a fleet row on every toggle and strand it there.
    if agent_screen::is_view_chrome_title(agent, title) {
        return None;
    }
    let title = agent_screen::stable_title(agent, title);
    if title.is_empty()
        || title.eq_ignore_ascii_case(agent)
        || title.eq_ignore_ascii_case(agent_display_name(agent))
        || (agent == "codex" && title.eq_ignore_ascii_case("Codex CLI"))
        || ((agent == "claude" || agent == "claude-code") && title.eq_ignore_ascii_case("Claude"))
        || ((agent == "pi" || agent == "omp" || agent == "oh-my-pi")
            && (title == "π" || title.eq_ignore_ascii_case("omp")))
    {
        None
    } else {
        Some(title)
    }
}

pub(crate) fn agent_command(command: &str, settings: &AppSettings) -> Option<Agent> {
    let executable = command_executable(command)?;
    let codex = command_executable(&settings.codex_command).unwrap_or("codex");
    let claude = command_executable(&settings.claude_command).unwrap_or("claude");
    let pi = command_executable(&settings.pi_command).unwrap_or("omp");
    if executable.eq_ignore_ascii_case(codex) || executable.eq_ignore_ascii_case("codex") {
        Some(Agent::Codex)
    } else if executable.eq_ignore_ascii_case(claude) || executable.eq_ignore_ascii_case("claude") {
        Some(Agent::Claude)
    } else if executable.eq_ignore_ascii_case(pi)
        || executable.eq_ignore_ascii_case("omp")
        || executable.eq_ignore_ascii_case("pi")
    {
        Some(Agent::Pi)
    } else {
        None
    }
}

pub(crate) fn command_executable(command: &str) -> Option<&str> {
    let mut words = command.split_whitespace();
    let mut executable = words.next()?;
    if executable.eq_ignore_ascii_case("env") || executable.eq_ignore_ascii_case("sudo") {
        executable = words.find(|word| !word.contains('='))?;
    } else if executable.contains('=') {
        executable = words.find(|word| !word.contains('='))?;
    }
    executable = executable.trim_matches(['\'', '"']);
    let executable = executable.rsplit(['/', '\\']).next()?;
    Some(executable.strip_suffix(".exe").unwrap_or(executable))
}

#[cfg(not(test))]
pub(crate) fn start_control_server(
    endpoint: Result<Endpoint, muxtrix_control::ControlError>,
    notifier: EventNotifier,
) -> (Option<ControlServer>, Option<String>) {
    match endpoint.and_then(|endpoint| ControlServer::bind_with_notifier(endpoint, notifier)) {
        Ok(server) => (Some(server), None),
        Err(error) => (
            None,
            Some(format!("Local control service unavailable: {error}")),
        ),
    }
}

#[cfg(test)]
pub(crate) fn start_control_server(
    _endpoint: Result<Endpoint, muxtrix_control::ControlError>,
    _notifier: EventNotifier,
) -> (Option<ControlServer>, Option<String>) {
    (None, None)
}

pub(crate) fn muxtrixctl_path() -> Result<std::path::PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    Ok(muxtrixctl_path_beside(&executable))
}

pub(crate) fn muxtrixctl_path_beside(executable: &std::path::Path) -> std::path::PathBuf {
    let file_name = if cfg!(windows) {
        "muxtrixctl.exe"
    } else {
        "muxtrixctl"
    };
    executable.with_file_name(file_name)
}

pub(crate) fn startup_muxtrix_path() -> Result<std::path::PathBuf, String> {
    let invoked = std::env::args_os()
        .next()
        .ok_or_else(|| "the startup executable path is unavailable".to_owned())?;
    let current_directory = std::env::current_dir().map_err(|error| error.to_string())?;
    let search_path = std::env::var_os("PATH");
    resolve_startup_executable(&invoked, &current_directory, search_path.as_deref())
}

pub(crate) fn resolve_startup_executable(
    invoked: &std::ffi::OsStr,
    current_directory: &std::path::Path,
    search_path: Option<&std::ffi::OsStr>,
) -> Result<std::path::PathBuf, String> {
    let invoked = std::path::PathBuf::from(invoked);
    if invoked.is_absolute() {
        return Ok(invoked);
    }
    if invoked.components().count() > 1 {
        return Ok(current_directory.join(invoked));
    }
    if let Some(search_path) = search_path {
        for directory in std::env::split_paths(search_path) {
            let candidate = directory.join(&invoked);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(format!(
        "{} was not found on the startup PATH",
        invoked.display()
    ))
}

pub(crate) fn probe_installed_versions(
    installed_muxtrix_path: Result<std::path::PathBuf, String>,
) -> InstalledVersions {
    let muxtrix_path = match installed_muxtrix_path {
        Ok(path) => path,
        Err(error) => {
            return InstalledVersions {
                muxtrix: Err(error.clone()),
                muxtrixctl: Err(error),
            };
        }
    };
    // Release packages install both binaries from one workspace version.
    // Probe the CLI for that package version: an older CLI rejects the flag
    // and exits, while launching an older GUI binary could open another window
    // and leave this background check blocked.
    let installed = probe_binary_version(&muxtrixctl_path_beside(&muxtrix_path), "muxtrixctl");
    InstalledVersions {
        muxtrix: installed.clone(),
        muxtrixctl: installed,
    }
}

pub(crate) fn probe_binary_version(
    path: &std::path::Path,
    binary_name: &str,
) -> Result<String, String> {
    let mut command = console_command(path);
    command.arg("--version");
    let output = command_output(
        &mut command,
        HELPER_COMMAND_TIMEOUT,
        &ProcessCancellation::default(),
    )
    .map_err(|error| format!("could not run {}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} --version exited with {}",
            path.display(),
            output.status
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("{} returned invalid text: {error}", path.display()))?;
    parse_binary_version(&stdout, binary_name)
}

pub(crate) fn parse_binary_version(output: &str, binary_name: &str) -> Result<String, String> {
    let mut fields = output.split_whitespace();
    if fields.next() != Some(binary_name) {
        return Err(format!(
            "{binary_name} returned an unexpected version response"
        ));
    }
    let version = fields
        .next()
        .filter(|version| !version.is_empty())
        .ok_or_else(|| format!("{binary_name} did not report a version"))?;
    if fields.next().is_some() {
        return Err(format!(
            "{binary_name} returned an unexpected version response"
        ));
    }
    Ok(version.to_owned())
}

/// Run `work` off the UI thread and turn its result into a message.
///
/// The runtime owns the thread, so this is now just the shape of the request:
/// the work runs to completion somewhere else, and `map` decides what the
/// answer means. `Result` is kept in the signature because callers already
/// phrase failure that way, even though the effect itself cannot fail.
pub(crate) fn perform_blocking<T>(
    work: impl FnOnce() -> T + Send + 'static,
    map: impl FnOnce(Result<T, String>) -> Message + Send + 'static,
) -> Vec<Effect>
where
    T: Send + 'static,
{
    vec![Effect::Perform(Box::new(move || map(Ok(work()))))]
}

pub(crate) fn hook_manager(settings: &AppSettings) -> Result<HookManager, String> {
    #[cfg(target_os = "windows")]
    if settings.windows_shell_backend == WindowsShellBackend::Wsl {
        return wsl_hook_manager(settings);
    }
    let _ = settings;
    HookManager::discover(muxtrixctl_path()?).map_err(|error| error.to_string())
}

pub(crate) fn hook_discovery_may_migrate_paths(
    no_session_daemon: bool,
    e2e_instance: bool,
    custom_control_endpoint: bool,
) -> bool {
    !no_session_daemon && !e2e_instance && !custom_control_endpoint
}

#[cfg(not(test))]
pub(crate) fn load_hook_statuses(settings: &AppSettings) -> Result<Vec<HookStatus>, String> {
    let manager = hook_manager(settings)?;
    // An isolated/headless instance must never claim the user's real hooks for
    // its temporary executable. Explicit Add/Repair actions still write; only
    // background discovery becomes read-only in these environments.
    let migrate_paths = hook_discovery_may_migrate_paths(
        std::env::var_os("MUXTRIX_NO_SESSIOND").is_some(),
        std::env::var_os("MUXTRIX_E2E_REPORT").is_some(),
        std::env::var_os("MUXTRIX_CONTROL_ENDPOINT").is_some(),
    );
    Agent::ALL
        .into_iter()
        .map(|agent| {
            let status = if migrate_paths {
                // A normal installed instance may transparently re-point hooks
                // after its executable moves. Semantic changes still require
                // the explicit Repair action.
                manager.synced_status(agent, HookScope::User)
            } else {
                manager.status(agent, HookScope::User)
            };
            status.map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn load_hook_statuses(_settings: &AppSettings) -> Result<Vec<HookStatus>, String> {
    Ok(Vec::new())
}

#[cfg(target_os = "windows")]
pub(crate) fn wsl_hook_manager(settings: &AppSettings) -> Result<HookManager, String> {
    let cache_key = settings.wsl_distribution.trim().to_ascii_lowercase();
    static CONTEXTS: OnceLock<Mutex<HashMap<String, WslHookContext>>> = OnceLock::new();
    let contexts = CONTEXTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(context) = contexts
        .lock()
        .map_err(|_| "WSL hook context cache is unavailable".to_owned())?
        .get(&cache_key)
        .cloned()
    {
        return Ok(context.manager());
    }

    let windows_executable = muxtrixctl_path()?;
    let mut identity = console_command("wsl.exe");
    if !settings.wsl_distribution.trim().is_empty() {
        identity.args(["--distribution", settings.wsl_distribution.trim()]);
    }
    identity
        .args([
            "--exec",
            "sh",
            "-lc",
            "printf '%s\\n%s\\n' \"$WSL_DISTRO_NAME\" \"$HOME\"; wslpath -u \"$1\"",
            "muxtrix-hook-context",
        ])
        .arg(&windows_executable);
    let identity = command_output(
        &mut identity,
        HELPER_COMMAND_TIMEOUT,
        &ProcessCancellation::default(),
    )
    .map_err(|error| format!("could not query the selected WSL integration context: {error}"))?;
    if !identity.status.success() {
        return Err(format!(
            "could not query the selected WSL integration context: {}",
            String::from_utf8_lossy(&identity.stderr).trim()
        ));
    }
    let identity = String::from_utf8(identity.stdout)
        .map_err(|_| "the selected WSL distribution returned invalid UTF-8".to_owned())?;
    let mut lines = identity.lines();
    let distribution = lines
        .next()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "the selected WSL distribution did not report its name".to_owned())?;
    let linux_home = lines
        .next()
        .filter(|value| value.starts_with('/'))
        .ok_or_else(|| "the selected WSL distribution did not report its home".to_owned())?;
    let linux_executable = lines
        .next()
        .ok_or_else(|| "the selected WSL distribution did not translate muxtrixctl.exe".to_owned())?
        .trim()
        .to_owned();
    if !linux_executable.starts_with('/') {
        return Err("the selected WSL distribution returned an invalid muxtrixctl path".into());
    }

    let unc_home = PathBuf::from(format!(
        r"\\wsl.localhost\{}\{}",
        distribution,
        linux_home.trim_start_matches('/').replace('/', "\\")
    ));
    let state_root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .ok_or_else(|| "Windows application data directory is unavailable".to_owned())?;
    let state_name: String = distribution
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let context = WslHookContext {
        home: unc_home,
        state_dir: state_root
            .join("Muxtrix")
            .join("hooks")
            .join(format!("wsl-{state_name}")),
        executable: PathBuf::from(linux_executable),
        worktree_root: PathBuf::from(linux_home).join(WORKTREE_HOME_FOLDER),
    };
    contexts
        .lock()
        .map_err(|_| "WSL hook context cache is unavailable".to_owned())?
        .insert(cache_key, context.clone());
    Ok(context.manager())
}

#[cfg(target_os = "windows")]
#[derive(Clone)]
pub(crate) struct WslHookContext {
    pub(crate) home: PathBuf,
    pub(crate) state_dir: PathBuf,
    pub(crate) executable: PathBuf,
    pub(crate) worktree_root: PathBuf,
}

#[cfg(target_os = "windows")]
impl WslHookContext {
    pub(crate) fn manager(&self) -> HookManager {
        // The executable is the distribution's own translation of
        // muxtrixctl.exe, so it only resolves inside WSL — this Windows
        // process cannot stat it, and `wslpath` has already vouched for it.
        HookManager::with_paths(&self.home, &self.home, &self.state_dir, &self.executable)
            .with_named_executable()
            .worktree_root(&self.worktree_root)
    }
}

pub(crate) fn default_profile(settings: &AppSettings) -> LaunchProfile {
    default_profile_with_id(settings, ProfileId::new())
}

pub(crate) fn default_profile_with_id(settings: &AppSettings, id: ProfileId) -> LaunchProfile {
    #[cfg(target_os = "windows")]
    {
        windows_profile(settings, id)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = settings;
        let (name, program, arguments) = (
            "Local shell",
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
            vec!["-l".to_owned()],
        );

        LaunchProfile {
            id,
            name: name.into(),
            backend: ProcessBackend::Local,
            program,
            arguments,
            working_directory: None,
        }
    }
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_profile(settings: &AppSettings, id: ProfileId) -> LaunchProfile {
    match settings.windows_shell_backend {
        WindowsShellBackend::Native => LaunchProfile {
            id,
            name: "PowerShell".into(),
            backend: ProcessBackend::Local,
            program: "powershell.exe".into(),
            arguments: vec!["-NoLogo".into()],
            working_directory: None,
        },
        WindowsShellBackend::Wsl => LaunchProfile {
            id,
            name: "WSL shell".into(),
            backend: ProcessBackend::Wsl {
                distribution: (!settings.wsl_distribution.trim().is_empty())
                    .then(|| settings.wsl_distribution.trim().to_owned()),
            },
            program: String::new(),
            arguments: Vec::new(),
            working_directory: Some("~".into()),
        },
    }
}

pub(crate) fn terminal_surface(profile_id: ProfileId, title: &str) -> Surface {
    Surface::terminal(
        title,
        TerminalSurface {
            profile_id,
            working_directory: None,
        },
    )
}

pub(crate) fn ghostty_preview() -> Result<String, String> {
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: 80,
        rows: 24,
        max_scrollback: DEFAULT_TERMINAL_SCROLLBACK_LINES,
    })
    .map_err(|error| error.to_string())?;
    actor
        .feed(
            b"\x1b[1;36mMuxtrix\x1b[0m terminal surface\r\n\r\nGhostty VT is parsing this grid.\r\nIced/wgpu will render terminal snapshots on the GPU.\r\n\r\n$ ".to_vec(),
        )
        .map_err(|error| error.to_string())?;
    let snapshot = actor.snapshot().map_err(|error| error.to_string())?;
    actor.shutdown().map_err(|error| error.to_string())?;
    Ok(snapshot.text())
}

/// The GUI's connection to the session daemon that owns its PTYs. Absent
/// in tests and when the daemon fails to start, in which case panes fall
/// back to in-process PTYs (no persistence). Mutable so resuming another
/// session can swap the connection.
pub(crate) struct SessionHost {
    pub(crate) id: uuid::Uuid,
    pub(crate) client: Arc<muxtrix_sessions::SessionClient>,
}

pub(crate) fn session_host() -> Option<(uuid::Uuid, Arc<muxtrix_sessions::SessionClient>)> {
    SESSION_HOST
        .lock()
        .ok()?
        .as_ref()
        .map(|host| (host.id, Arc::clone(&host.client)))
}

pub(crate) fn start_host_unless_resumable(
    candidates: Vec<muxtrix_sessions::SessionRecord>,
    start: impl FnOnce() -> Result<(), String>,
) -> Result<Vec<muxtrix_sessions::SessionRecord>, String> {
    if !candidates.is_empty() {
        return Ok(candidates);
    }
    start()?;
    Ok(Vec::new())
}

/// Spawns and connects a new session daemon after the user has declined every
/// resumable session. Startup discovery must happen before this function runs.
pub(crate) fn start_session_host() -> Option<SessionHost> {
    if std::env::var_os("MUXTRIX_NO_SESSIOND").is_some()
        || std::env::var_os("MUXTRIX_E2E_REPORT").is_some()
    {
        return None;
    }
    let id = uuid::Uuid::new_v4();
    let endpoint = muxtrix_sessions::session_endpoint(id);
    muxtrix_sessions::daemon::spawn_detached(id, "Workspace", &endpoint).ok()?;
    if !muxtrix_sessions::daemon::wait_until_ready(&endpoint) {
        return None;
    }
    let (client, _, _) = muxtrix_sessions::SessionClient::connect_endpoint(&endpoint).ok()?;
    Some(SessionHost {
        id,
        client: Arc::new(client),
    })
}

/// Blocking reader over a pane's daemon-fed byte channel; channel close is
/// EOF, exactly like a PTY reader hitting the end of stream.
pub(crate) struct ReceiverReader(std::sync::mpsc::Receiver<Vec<u8>>, Vec<u8>);

impl std::io::Read for ReceiverReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.1.is_empty() {
            match self.0.recv() {
                Ok(bytes) => self.1 = bytes,
                Err(_) => return Ok(0),
            }
        }
        let count = self.1.len().min(buffer.len());
        buffer[..count].copy_from_slice(&self.1[..count]);
        self.1.drain(..count);
        Ok(count)
    }
}

pub(crate) struct RemotePaneBackend {
    pub(crate) pane: uuid::Uuid,
    pub(crate) client: Arc<muxtrix_sessions::SessionClient>,
    pub(crate) output: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
}

impl muxtrix_terminal::SessionBackend for RemotePaneBackend {
    fn take_reader(&mut self) -> Result<Box<dyn std::io::Read + Send>, String> {
        self.output
            .take()
            .map(|receiver| {
                Box::new(ReceiverReader(receiver, Vec::new())) as Box<dyn std::io::Read + Send>
            })
            .ok_or_else(|| "pane reader already taken".to_owned())
    }
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.client
            .send(&muxtrix_sessions::Request::Input {
                pane: self.pane,
                data: muxtrix_sessions::encode_bytes(bytes),
            })
            .map_err(|error| error.to_string())
    }
    fn resize(&self, size: PtySize) -> Result<(), String> {
        self.client
            .send(&muxtrix_sessions::Request::Resize {
                pane: self.pane,
                rows: size.rows,
                cols: size.cols,
            })
            .map_err(|error| error.to_string())
    }
    fn kill(&mut self) -> Result<(), String> {
        let result = self
            .client
            .send(&muxtrix_sessions::Request::Kill { pane: self.pane })
            .map_err(|error| error.to_string());
        // A killed pane reports no exit, so nothing else will ever close its
        // byte channel — and its reader thread blocks until something does.
        self.client.unregister_pane(self.pane);
        result
    }
    fn process_id(&self) -> Option<u32> {
        self.client.pane_process_id(self.pane)
    }
    fn poll_exit(&mut self) -> Result<Option<bool>, String> {
        Ok(self.client.pane_exit(self.pane))
    }
    fn exit_clean(&mut self) -> bool {
        self.client.pane_exit(self.pane).unwrap_or(false)
    }
    fn kill_on_detach(&self) -> bool {
        false
    }
    fn discard_pty_responses(&self) -> bool {
        self.client.pane_replaying(self.pane)
    }
}

pub(crate) fn start_live_session(
    profile: &LaunchProfile,
    pane_id: PaneId,
    max_scrollback: usize,
    theme: TerminalTheme,
    notifier: EventNotifier,
    control_endpoint: Option<&str>,
) -> Result<LiveSession, String> {
    start_live_session_with_client(
        profile,
        pane_id,
        max_scrollback,
        theme,
        notifier,
        control_endpoint,
        session_host().map(|(_, client)| client),
    )
}

pub(crate) fn start_live_session_with_client(
    profile: &LaunchProfile,
    pane_id: PaneId,
    max_scrollback: usize,
    theme: TerminalTheme,
    notifier: EventNotifier,
    control_endpoint: Option<&str>,
    session_client: Option<Arc<muxtrix_sessions::SessionClient>>,
) -> Result<LiveSession, String> {
    let mut plan = LaunchPlan::from_profile(profile).map_err(|error| error.to_string())?;
    let wslenv = std::env::var("WSLENV").ok();
    let inherited_endpoint = std::env::var("MUXTRIX_CONTROL_ENDPOINT").ok();
    let endpoint = control_endpoint.or(inherited_endpoint.as_deref());
    let integration_zdotdir = shell_integration_zdotdir(&profile.backend);
    add_muxtrix_environment(
        &mut plan,
        &profile.backend,
        pane_id,
        wslenv.as_deref(),
        endpoint,
        integration_zdotdir.as_deref(),
    );
    let size = initial_pty_size();
    let options = TerminalOptions {
        cols: size.cols,
        rows: size.rows,
        max_scrollback,
    };
    // Daemon-owned PTYs survive this GUI closing. Production never silently
    // falls back to an in-process PTY: doing so can repeat the same blocked
    // host operation on the UI's behalf and hides which backend failed.
    if let Some(client) = session_client {
        let pane = pane_id.as_uuid();
        // Pane ids are durable, so this spawn may be reclaiming the id of a
        // pane that is being replaced. Releasing it explicitly — rather than
        // relying on the outgoing session's thread having done so — is what
        // keeps the host from serving two incarnations of one id, where the
        // outgoing one's reader and exit report land on this one and leave a
        // live-looking terminal that never receives a byte.
        let _ = client.send(&muxtrix_sessions::Request::Kill { pane });
        let output = client.register_pane(pane);
        let spawned = client.send(&muxtrix_sessions::Request::Spawn {
            pane,
            executable: plan.executable.clone(),
            arguments: plan.arguments.clone(),
            working_directory: plan.working_directory.clone(),
            environment: plan.environment.clone(),
            rows: size.rows,
            cols: size.cols,
        });
        spawned.map_err(|error| format!("Terminal host rejected the launch: {error}"))?;
        return LiveSession::spawn_remote(
            Box::new(RemotePaneBackend {
                pane,
                client,
                output: Some(output),
            }),
            size,
            options,
            theme,
            Some(notifier),
        )
        .map_err(|error| error.to_string());
    }
    if local_pty_allowed() {
        return LiveSession::spawn_with_notifier_and_theme(
            plan,
            size,
            options,
            theme,
            Some(notifier),
        )
        .map_err(|error| error.to_string());
    }
    Err("Terminal session host is unavailable; local fallback was not attempted".into())
}

pub(crate) fn local_pty_allowed() -> bool {
    should_allow_local_pty(
        cfg!(test),
        std::env::var_os("MUXTRIX_NO_SESSIOND").is_some(),
        std::env::var_os("MUXTRIX_E2E_REPORT").is_some(),
    )
}

pub(crate) const fn should_allow_local_pty(testing: bool, no_sessiond: bool, e2e: bool) -> bool {
    testing || no_sessiond || e2e
}

pub(crate) fn dispose_live_session(session: LiveSession) {
    let _ = std::thread::Builder::new()
        .name("muxtrix-stale-terminal-disposal".into())
        .spawn(move || drop(session));
}

/// Ends a session no pane can reach any more. Dropping alone would only
/// detach it — daemon-owned panes are built to survive that — so this kills
/// the process outright, off the UI thread because it waits on the daemon
/// round trip and the session thread's join.
pub(crate) fn terminate_live_session(session: LiveSession, pane_id: PaneId) {
    let _ = std::thread::Builder::new()
        .name("muxtrix-stale-terminal-disposal".into())
        .spawn(move || {
            session.terminate();
            // Joining the session thread is what guarantees the kill reached
            // the daemon before the pane is forgotten.
            drop(session);
            forget_host_pane(pane_id);
        });
}

/// Drops the session host's half of a closed pane. `LiveSession::terminate`
/// covers a pane whose session thread is still running, but a pane whose
/// process already exited has no thread left to carry the message — without
/// this its PTY, backlog and reader linger in the daemon, and the client
/// keeps a byte channel open that nothing will ever close.
pub(crate) fn forget_host_pane(pane_id: PaneId) {
    let Some((_, client)) = session_host() else {
        return;
    };
    let pane = pane_id.as_uuid();
    let _ = client.send(&muxtrix_sessions::Request::Kill { pane });
    client.unregister_pane(pane);
}

pub(crate) fn add_muxtrix_environment(
    plan: &mut LaunchPlan,
    backend: &ProcessBackend,
    pane_id: PaneId,
    inherited_wslenv: Option<&str>,
    endpoint: Option<&str>,
    shell_integration_zdotdir: Option<&str>,
) {
    plan.environment
        .push(("MUXTRIX_PANE_ID".into(), pane_id.as_uuid().to_string()));
    if let Some(endpoint) = endpoint {
        plan.environment
            .push(("MUXTRIX_CONTROL_ENDPOINT".into(), endpoint.into()));
    }
    // OSC 7 working-directory reporting for shells that do not emit it
    // themselves: bash reads PROMPT_COMMAND straight from the environment,
    // zsh picks up a precmd hook through the redirected ZDOTDIR (whose
    // .zshenv restores the user's real dotfiles). Fish needs neither.
    plan.environment.push((
        "PROMPT_COMMAND".into(),
        muxtrix_platform::shell_integration::BASH_PROMPT_COMMAND.into(),
    ));
    if let Some(zdotdir) = shell_integration_zdotdir {
        if let Ok(original) = std::env::var("ZDOTDIR")
            && !original.is_empty()
        {
            plan.environment
                .push(("MUXTRIX_ORIG_ZDOTDIR".into(), original));
        }
        plan.environment.push(("ZDOTDIR".into(), zdotdir.into()));
    }
    if matches!(backend, ProcessBackend::Wsl { .. }) {
        let mut shared: Vec<String> = inherited_wslenv
            .map(|value| {
                value
                    .split(':')
                    .filter(|entry| !entry.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let mut names = vec![
            "MUXTRIX_PANE_ID",
            "MUXTRIX_CONTROL_ENDPOINT",
            "PROMPT_COMMAND",
        ];
        if shell_integration_zdotdir.is_some() {
            names.push("ZDOTDIR");
        }
        for name in names {
            if !shared
                .iter()
                .any(|entry| entry.split('/').next() == Some(name))
            {
                shared.push(name.into());
            }
        }
        plan.environment.push(("WSLENV".into(), shared.join(":")));
    }
}

/// Where the zsh integration bridge for `backend` lives, staging it on
/// first use — one filesystem write (or wsl.exe round trip) per backend
/// per app run, cached after that.
pub(crate) fn shell_integration_zdotdir(backend: &ProcessBackend) -> Option<String> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Option<String>>>,
    > = std::sync::OnceLock::new();
    let key = match backend {
        ProcessBackend::Local => "local".to_owned(),
        ProcessBackend::Wsl { distribution } => {
            format!("wsl:{}", distribution.as_deref().unwrap_or(""))
        }
    };
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(cached) = cache.lock().expect("shell integration cache").get(&key) {
        return cached.clone();
    }
    let prepared = stage_shell_integration(backend);
    cache
        .lock()
        .expect("shell integration cache")
        .insert(key, prepared.clone());
    prepared
}

pub(crate) fn stage_shell_integration(backend: &ProcessBackend) -> Option<String> {
    match backend {
        ProcessBackend::Local => {
            #[cfg(target_os = "windows")]
            {
                // Native Windows shells are not zsh; nothing to stage.
                None
            }
            #[cfg(not(target_os = "windows"))]
            {
                let config_home = std::env::var("XDG_CONFIG_HOME")
                    .ok()
                    .filter(|value| !value.is_empty())
                    .map(std::path::PathBuf::from)
                    .or_else(|| Some(home_directory()?.join(".config")))?;
                let fish_dir = config_home.join("fish/conf.d");
                if std::fs::create_dir_all(&fish_dir).is_ok() {
                    let _ = std::fs::write(
                        fish_dir.join("muxtrix.fish"),
                        muxtrix_platform::shell_integration::FISH_CONF_D,
                    );
                }
                let dir = home_directory()?.join(SHELL_INTEGRATION_ZSH_DIR);
                std::fs::create_dir_all(&dir).ok()?;
                std::fs::write(
                    dir.join(".zshenv"),
                    muxtrix_platform::shell_integration::ZSH_ZSHENV,
                )
                .ok()?;
                Some(dir.to_string_lossy().into_owned())
            }
        }
        ProcessBackend::Wsl { distribution } => {
            // The bridge files must live on the Linux side; content travels
            // on stdin because command lines and newlines do not mix well
            // across the boundary.
            #[cfg(target_os = "windows")]
            {
                let distribution = distribution.as_deref().unwrap_or("");
                // fish stays silent under plain TERMs unless taught; a
                // failure here must not cost zsh its bridge.
                let _ = wsl_stage_file(
                    distribution,
                    ".config/fish/conf.d",
                    "muxtrix.fish",
                    muxtrix_platform::shell_integration::FISH_CONF_D,
                );
                wsl_stage_file(
                    distribution,
                    SHELL_INTEGRATION_ZSH_DIR,
                    ".zshenv",
                    muxtrix_platform::shell_integration::ZSH_ZSHENV,
                )
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = distribution;
                None
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn discover_wsl_distributions() -> Vec<WslDistributionChoice> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let mut choices = vec![WslDistributionChoice::default_distribution()];
    let Ok(root) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Lxss")
    else {
        return choices;
    };
    let names = root.enum_keys().filter_map(Result::ok).filter_map(|id| {
        let distribution = root.open_subkey(id).ok()?;
        let modern = distribution.get_value::<u32, _>("Modern").unwrap_or(0);
        if modern == 1 {
            return None;
        }
        distribution.get_value::<String, _>("DistributionName").ok()
    });
    for distribution in visible_wsl_distribution_names(names) {
        choices.push(WslDistributionChoice(Some(distribution)));
    }
    choices
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn discover_wsl_distributions() -> Vec<WslDistributionChoice> {
    vec![WslDistributionChoice::default_distribution()]
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn visible_wsl_distribution_names(
    candidates: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut names = Vec::new();
    for name in candidates {
        let name = name.trim();
        let lower = name.to_ascii_lowercase();
        if name.is_empty()
            || lower.starts_with("docker-desktop")
            || lower.starts_with("rancher-desktop")
        {
            continue;
        }
        if !names
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(name))
        {
            names.push(name.to_owned());
        }
    }
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names
}

#[cfg(test)]
mod tests;
