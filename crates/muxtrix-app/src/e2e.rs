use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::Task;
use muxtrix_domain::{PaneId, SplitAxis, TabId};
use serde_json::json;

use muxtrix_control::AgentState;

use super::{
    ActiveView, Agent, AgentPaneStatus, CommandAction, GitHubDiffSource, GitHubDiffState,
    GitHubPanelState, GitHubPanelTab, HookScope, HookStatus, Message, Muxtrix, PaneRepository,
    TerminalMouseButton, WorktreeManagerEntry, WorktreeManagerMode, WorktreeManagerState, github,
};
use crate::settings::{FleetScope, FleetView};

const REPORT_ENV: &str = "MUXTRIX_E2E_REPORT";
const SCREENSHOT_ENV: &str = "MUXTRIX_E2E_SCREENSHOT_RGBA";
const EXTERNAL_MARKER: &str = "alpha beta";
const TERMINAL_URL_MARKER: &str = "https://example.com/docs";
const PANE_MENU_CLICK_AWAY_MARKER: &str = "pane-menu-click-away-ready";
const MOUSE_REPORT_MARKER: &str = "mouse-report-ok";
// Both markers have to fit on one row of a split pane at the minimum
// supported window: at 720px wide each half is around a dozen columns, and a
// marker that wraps can never be found in a single row of the snapshot.
const SECOND_MARKER: &str = "p2-mark";
const THIRD_MARKER: &str = "p3-mark";
const TERMINAL_RULE_CONTINUITY_PIXELS: usize = 300;
const TERMINAL_BLOCK_CONTINUITY_PIXELS: usize = 120;
const TERMINAL_BLOCK_CONTINUITY_ROWS: usize = 8;
const TERMINAL_ROUNDED_BOX_WIDTH_PIXELS: usize = 200;
const TERMINAL_ROUNDED_BOX_HEIGHT_PIXELS: usize = 30;
const TERMINAL_HEAVY_BOX_WIDTH_PIXELS: usize = 120;
const TERMINAL_HEAVY_BOX_HEIGHT_PIXELS: usize = 30;

/// A bare Down press, shaped exactly as the window delivers one.
fn arrow_down() -> iced::keyboard::Event {
    let key = iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown);
    iced::keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::ArrowDown),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::default(),
        text: None,
        repeat: false,
    }
}

/// Printed by the `terminal-palette` capture so a terminal theme is legible
/// from one frame: both ANSI ramps, the attribute set, truecolor, and a short
/// stretch of ordinary build output for body text.
///
/// The readiness marker is assembled by `printf` rather than written out, so
/// the shell's echo of this command line cannot satisfy the wait.
const TERMINAL_PALETTE_SCRIPT: &str = concat!(
    "printf '\\033[2J\\033[HANSI palette\\n\\n",
    "\\033[30m  0 \\033[31m  1 \\033[32m  2 \\033[33m  3 ",
    "\\033[34m  4 \\033[35m  5 \\033[36m  6 \\033[37m  7 \\033[0m\\n",
    "\\033[90m  8 \\033[91m  9 \\033[92m 10 \\033[93m 11 ",
    "\\033[94m 12 \\033[95m 13 \\033[96m 14 \\033[97m 15 \\033[0m\\n",
    "\\033[40m    \\033[41m    \\033[42m    \\033[43m    ",
    "\\033[44m    \\033[45m    \\033[46m    \\033[47m    \\033[0m\\n",
    "\\033[100m    \\033[101m    \\033[102m    \\033[103m    ",
    "\\033[104m    \\033[105m    \\033[106m    \\033[107m    \\033[0m\\n\\n",
    "regular  \\033[1mbold\\033[0m  \\033[2mdim\\033[0m  \\033[3mitalic\\033[0m  ",
    "\\033[4munderline\\033[0m  \\033[7mreverse\\033[0m\\n\\n",
    "\\033[38;2;235;110;110mtruecolor\\033[0m  \\033[38;2;110;220;150mgradient\\033[0m  ",
    "\\033[38;2;110;170;240msamples\\033[0m\\n\\n",
    "$ cargo test --workspace\\n",
    "   Compiling muxtrix v0.1.46\\n",
    "    Finished `test` profile in 41.2s\\n",
    "test result: ok. 312 passed; 0 failed\\n\\n'; ",
    "printf 'palette-%s\\n' ready\r"
);
const TERMINAL_PALETTE_MARKER: &str = "palette-ready";
const SELECTION_FOLLOW_MARKER: &str = "selection-follow-target";
const STAGED_GITHUB_PATCH: &str = concat!(
    "@@ -312,10 +312,18 @@ pub(crate) fn load(repository: &Repository) -> Result<PanelData, String> {\n",
    "     })\n",
    " }\n",
    " \n",
    "+/// Fetch lightweight summaries only when the PR tab opens.\n",
    "+pub(crate) fn list_pull_requests(\n",
    "+    repository: &Repository,\n",
    "+) -> Result<Vec<PullRequestSummary>, String> {\n",
    "+    load_pull_request_summaries(repository)\n",
    "+}\n",
    "+\n",
    " pub(crate) fn merge(\n",
    "     repository: &Repository,\n",
    "     number: u64,\n",
    "@@ -328,7 +336,6 @@ pub(crate) fn merge(\n",
    "-    let branch = repository.branch.clone();\n",
    "     let owner_and_name = github_repository(repository)?;\n",
    "     let number = number.to_string();\n",
    "     let output = console_command(\"gh\")\n",
    "@@ -344,6 +351,9 @@ pub(crate) fn merge(\n",
    "         .output()\n",
    "         .map_err(|error| format!(\"GitHub merge could not start: {error}\"))?;\n",
    "+    // The branch remains intact so the user can decide when to remove it after reviewing the complete merge result.\n",
    "+    // This deliberately keeps the command asynchronous from the renderer.\n",
    "+\n",
    "     if output.status.success() {\n",
    "         Ok(format!(\"Merged pull request #{number}\"))\n",
    "     } else {\n",
);

pub(super) struct Scenario {
    report_path: PathBuf,
    started: Instant,
    stage: Stage,
    initial_pane: PaneId,
    second_pane: Option<PaneId>,
    third_pane: Option<PaneId>,
    original_tab: Option<TabId>,
    tab_pane: Option<PaneId>,
    initial_cols: Option<u16>,
    initial_rows: Option<u16>,
    settle_ticks: u8,
    pane_grid_settle_ticks: u16,
    pane_grids_match_panes: bool,
    selection_observed: bool,
    selection_dragged: bool,
    selection_settle_ticks: u16,
    capture_selection_armed: bool,
    capture_selection_redraw_sent: bool,
    capture_selection_follow_observed: bool,
    palette_open_observed: bool,
    palette_navigation_observed: bool,
    settings_open_observed: bool,
    fleet_collapse_observed: bool,
    pane_maximize_observed: bool,
    pane_menu_observed: bool,
    pane_menu_click_open_observed: bool,
    pane_menu_click_away_observed: bool,
    terminal_scroll_observed: bool,
    terminal_scrollbar_observed: bool,
    terminal_mouse_reporting_observed: bool,
    tab_lifecycle_observed: bool,
    /// The `MUXTRIX_E2E_CAPTURE` state this run ends on, empty for the plain
    /// workspace frame. One name per staged surface; `capturing` is the only
    /// reader, so a new state costs one match arm rather than a new field.
    capture: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    PaneMenuClickAway,
    ExternalInput,
    SecondReady,
    SecondOutput,
    ThirdReady,
    ThirdOutput,
    AgentAttention,
    ChromeControls,
    TerminalExit,
    TerminalRestart,
    TabReady,
    Settle,
    Screenshot,
}

enum TickAction {
    Wait,
    ScrollSettingsToEnd,
    ScrollSettingsToGitHub,
    ScrollGitHubToEnd,
    ScrollGitHubPullRequestsToEnd,
    Capture,
}

impl Scenario {
    pub(super) fn from_environment(initial_pane: PaneId) -> Option<Self> {
        let report_path = std::env::var_os(REPORT_ENV).map(PathBuf::from)?;
        Some(Self {
            report_path,
            started: Instant::now(),
            stage: Stage::PaneMenuClickAway,
            initial_pane,
            second_pane: None,
            third_pane: None,
            original_tab: None,
            tab_pane: None,
            initial_cols: None,
            initial_rows: None,
            settle_ticks: 0,
            pane_grid_settle_ticks: 0,
            pane_grids_match_panes: false,
            selection_observed: false,
            selection_dragged: false,
            selection_settle_ticks: 0,
            capture_selection_armed: false,
            capture_selection_redraw_sent: false,
            capture_selection_follow_observed: false,
            palette_open_observed: false,
            palette_navigation_observed: false,
            settings_open_observed: false,
            fleet_collapse_observed: false,
            pane_maximize_observed: false,
            pane_menu_observed: false,
            pane_menu_click_open_observed: false,
            pane_menu_click_away_observed: false,
            terminal_scroll_observed: false,
            terminal_scrollbar_observed: false,
            terminal_mouse_reporting_observed: false,
            tab_lifecycle_observed: false,
            capture: std::env::var_os("MUXTRIX_E2E_CAPTURE")
                .map_or_else(String::new, |value| value.to_string_lossy().into_owned()),
        })
    }

    fn capturing(&self, state: &str) -> bool {
        self.capture == state
    }

    fn tick(&mut self, app: &mut Muxtrix) -> Result<TickAction, String> {
        self.palette_open_observed |= app.palette.visible;
        self.palette_navigation_observed |= app.palette.visible && app.palette.selected > 0;
        self.settings_open_observed |= app.active_view == ActiveView::Settings;
        if app.pane_menu.is_some() {
            self.pane_menu_click_open_observed = true;
        } else if self.pane_menu_click_open_observed {
            self.pane_menu_click_away_observed = true;
        }
        self.terminal_mouse_reporting_observed |=
            pane_contains(app, self.initial_pane, MOUSE_REPORT_MARKER);
        if self.started.elapsed() > Duration::from_secs(20) {
            let terminal = self.third_pane.and_then(|pane_id| {
                app.terminals.get(&pane_id).map(|runtime| {
                    format!(
                        "third pane: state={:?}, session={}, snapshot={}, preview={:?}",
                        runtime.launch_state,
                        runtime.session.is_some(),
                        runtime.snapshot.is_some(),
                        runtime.preview,
                    )
                })
            });
            // A stage can stall in several places; without its own progress
            // flags the report says only which stage, not which wait.
            return Err(format!(
                "E2E scenario timed out during {:?}; {}; app status={:?}; \
                 selection dragged={} observed={} settle={}; second pane marker={}",
                self.stage,
                terminal.as_deref().unwrap_or("third pane unavailable"),
                app.status,
                self.selection_dragged,
                self.selection_observed,
                self.selection_settle_ticks,
                self.second_pane
                    .is_some_and(|pane_id| pane_contains(app, pane_id, SECOND_MARKER)),
            ));
        }

        match self.stage {
            Stage::PaneMenuClickAway => {
                let marker_visible = app
                    .terminals
                    .get(&self.initial_pane)
                    .and_then(|runtime| runtime.snapshot.as_ref())
                    .is_some_and(|snapshot| snapshot.text().contains(PANE_MENU_CLICK_AWAY_MARKER));
                if !marker_visible {
                    return Ok(TickAction::Wait);
                }
                if !self.pane_menu_click_open_observed {
                    app.pane_menu = Some(self.initial_pane);
                    return Ok(TickAction::Wait);
                }
                if app.pane_menu.is_some() {
                    return Ok(TickAction::Wait);
                }
                if !self.pane_menu_click_away_observed {
                    return Err("the pane menu closed without recording its outside click".into());
                }
                self.stage = Stage::ExternalInput;
            }
            Stage::ExternalInput => {
                let Some(runtime) = app.terminals.get(&self.initial_pane) else {
                    return Err("initial terminal runtime disappeared".into());
                };
                let has_external_input = runtime.snapshot.as_ref().is_some_and(|snapshot| {
                    let text = snapshot.text();
                    text.contains(EXTERNAL_MARKER) && text.contains(TERMINAL_URL_MARKER)
                });
                if !has_external_input {
                    return Ok(TickAction::Wait);
                }
                let cursor_visible = runtime
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.cursor.is_some_and(|cursor| cursor.visible));
                if !cursor_visible {
                    return Err("focused initial terminal did not expose a visible cursor".into());
                }
                self.initial_cols = Some(runtime.size.cols);
                self.initial_rows = Some(runtime.size.rows);
                app.split_terminal(SplitAxis::Horizontal)?;
                self.second_pane = Some(
                    app.active_workspace()?
                        .active_tab()
                        .ok_or_else(|| "active tab is missing".to_owned())?
                        .focused_pane_id,
                );
                self.stage = Stage::SecondReady;
            }
            Stage::SecondReady => {
                let second = self.second_pane()?;
                let Some(runtime) = app.terminals.get(&second) else {
                    return Ok(TickAction::Wait);
                };
                if runtime.snapshot.is_none()
                    || runtime.size.cols >= self.initial_cols.unwrap_or(u16::MAX)
                {
                    return Ok(TickAction::Wait);
                }
                app.send_terminal_input(format!("printf '{SECOND_MARKER}\\n'\r").into_bytes())?;
                self.stage = Stage::SecondOutput;
            }
            Stage::SecondOutput => {
                let second = self.second_pane()?;
                if !pane_contains(app, second, SECOND_MARKER) {
                    return Ok(TickAction::Wait);
                }
                if pane_contains(app, self.initial_pane, SECOND_MARKER) {
                    return Err("second pane output leaked into the initial pane".into());
                }
                // Drag across the marker the way a pointer does, then ask for
                // the text: this crosses the pointer path, the session
                // thread, the emulator that owns the selection, and the
                // snapshot that paints it.
                if !self.selection_dragged {
                    let Some((row, column)) = pane_text_position(app, second, SECOND_MARKER) else {
                        return Err("the second pane's marker was not locatable".into());
                    };
                    let cell_width = app.settings.terminal_cell_width();
                    let cell_height = app.settings.terminal_cell_height();
                    let point = |column: usize, row: usize| {
                        iced::Point::new(
                            8.0 + cell_width * (column as f32 + 0.5),
                            8.0 + cell_height * (row as f32 + 0.5),
                        )
                    };
                    let last = column + SECOND_MARKER.chars().count() - 1;
                    let _ = app.update(Message::TerminalPointerMoved(second, point(column, row)));
                    let _ = app.update(Message::TerminalMousePressed(
                        second,
                        TerminalMouseButton::Left,
                    ));
                    let _ = app.update(Message::TerminalPointerMoved(second, point(last, row)));
                    let _ = app.update(Message::TerminalMouseReleased(
                        second,
                        TerminalMouseButton::Left,
                    ));
                    self.selection_dragged = true;
                    return Ok(TickAction::Wait);
                }
                if !self.selection_observed {
                    // The emulator answers the drag on its own thread; the
                    // frame carrying the highlight arrives on a later poll.
                    let painted = app
                        .terminals
                        .get(&second)
                        .and_then(|runtime| runtime.snapshot.as_ref())
                        .is_some_and(|snapshot| {
                            snapshot.selection.iter().any(std::option::Option::is_some)
                        });
                    if !painted {
                        self.selection_settle_ticks += 1;
                        if self.selection_settle_ticks > 240 {
                            return Err("the selected text was never painted as selected".into());
                        }
                        return Ok(TickAction::Wait);
                    }
                    let selected = app.selected_terminal_text(second);
                    if selected.as_deref() != Some(SECOND_MARKER) {
                        return Err(format!(
                            "dragging over {SECOND_MARKER:?} selected {selected:?}"
                        ));
                    }
                    self.selection_observed = true;
                }
                app.split_terminal(SplitAxis::Vertical)?;
                self.third_pane = Some(
                    app.active_workspace()?
                        .active_tab()
                        .ok_or_else(|| "active tab is missing".to_owned())?
                        .focused_pane_id,
                );
                self.stage = Stage::ThirdReady;
            }
            Stage::ThirdReady => {
                let third = self.third_pane()?;
                let Some(runtime) = app.terminals.get(&third) else {
                    return Ok(TickAction::Wait);
                };
                if runtime.snapshot.is_none()
                    || runtime.size.rows >= self.initial_rows.unwrap_or(u16::MAX)
                {
                    return Ok(TickAction::Wait);
                }
                app.send_terminal_input(format!("printf '{THIRD_MARKER}\\n'\r").into_bytes())?;
                self.stage = Stage::ThirdOutput;
            }
            Stage::ThirdOutput => {
                let second = self.second_pane()?;
                let third = self.third_pane()?;
                if !pane_contains(app, third, THIRD_MARKER) {
                    return Ok(TickAction::Wait);
                }
                if pane_contains(app, self.initial_pane, THIRD_MARKER)
                    || pane_contains(app, second, THIRD_MARKER)
                {
                    return Err("third pane output leaked into another pane".into());
                }
                app.active_workspace()?
                    .validate()
                    .map_err(|error| error.to_string())?;
                if app.terminals.len() != 3 {
                    return Err("nested splits did not create three terminal runtimes".into());
                }
                // Every pane's PTY must match the pane it is drawn into. A
                // resize measured while a launch was still in flight used to
                // be dropped, leaving a fresh pane rendering an 80-column
                // grid clipped by the pane's right edge.
                if let Some(mismatch) = pane_grid_mismatch(app) {
                    self.pane_grid_settle_ticks += 1;
                    if self.pane_grid_settle_ticks > 240 {
                        return Err(mismatch);
                    }
                    return Ok(TickAction::Wait);
                }
                self.pane_grids_match_panes = true;
                app.terminals
                    .get(&second)
                    .and_then(|runtime| runtime.session.as_ref())
                    .ok_or_else(|| "second pane lost its live session".to_owned())?
                    .input(b"printf '\\033]777;notify;Codex;Needs input\\007'\r".to_vec())
                    .map_err(|error| error.to_string())?;
                self.stage = Stage::AgentAttention;
            }
            Stage::AgentAttention => {
                let second = self.second_pane()?;
                let attention = app
                    .active_workspace()?
                    .pane(second)
                    .ok_or_else(|| "second pane disappeared".to_owned())?
                    .attention
                    .clone();
                if attention.unread_count == 0 {
                    return Ok(TickAction::Wait);
                }
                if attention.message.as_deref() != Some("Needs input") {
                    return Err("OSC notification body did not reach pane attention state".into());
                }
                if !app
                    .notifications
                    .iter()
                    .any(|notification| notification.pane_id == second && notification.unread)
                {
                    return Err("agent notification was not retained as pane activity".into());
                }
                if !app.global_alerts.is_empty() {
                    // Naming the alerts matters: a seeded settings profile that
                    // fails to parse also lands here, and the two causes need
                    // different fixes.
                    return Err(format!(
                        "pane activity was incorrectly duplicated into global Attention: {:?}",
                        app.global_alerts
                            .iter()
                            .map(|alert| format!("{}: {}", alert.title, alert.body))
                            .collect::<Vec<_>>()
                    ));
                }
                app.focus_pane(second)?;
                if app
                    .active_workspace()?
                    .pane(second)
                    .is_some_and(|pane| pane.attention.unread_count > 0)
                {
                    return Err("focusing an agent pane did not clear its fleet attention".into());
                }
                let _ = app.update(Message::ToggleSidebar);
                let _ = app.update(Message::ToggleMaximize(second));
                let _ = app.update(Message::TogglePaneMenu(second));
                self.stage = Stage::ChromeControls;
            }
            Stage::ChromeControls => {
                let second = self.second_pane()?;
                let third = self.third_pane()?;
                self.fleet_collapse_observed |= app.sidebar_collapsed;
                self.pane_maximize_observed |= app.maximized_pane == Some(second);
                self.pane_menu_observed |= app.pane_menu == Some(second);
                if !self.fleet_collapse_observed
                    || !self.pane_maximize_observed
                    || !self.pane_menu_observed
                {
                    return Err(
                        "fleet collapse, pane maximize, or overflow state did not render".into(),
                    );
                }
                let _ = app.update(Message::ToggleSidebar);
                let _ = app.update(Message::ToggleMaximize(second));
                let _ = app.update(Message::TogglePaneMenu(second));
                app.focus_pane(third)?;
                app.send_terminal_input(b"exit 1\r".to_vec())?;
                self.stage = Stage::TerminalExit;
            }
            Stage::TerminalExit => {
                let third = self.third_pane()?;
                if app
                    .terminals
                    .get(&third)
                    .is_some_and(|runtime| runtime.session.is_some())
                {
                    return Ok(TickAction::Wait);
                }
                app.restart_pane(third)?;
                self.stage = Stage::TerminalRestart;
            }
            Stage::TerminalRestart => {
                let second = self.second_pane()?;
                let third = self.third_pane()?;
                if app
                    .terminals
                    .get(&third)
                    .is_none_or(|runtime| runtime.session.is_none() || runtime.snapshot.is_none())
                {
                    return Ok(TickAction::Wait);
                }
                app.close_focused()?;
                if app.terminals.len() != 2 || app.terminals.contains_key(&third) {
                    return Err(
                        "closing the focused pane did not clean up exactly one runtime".into(),
                    );
                }
                if !pane_contains(app, second, SECOND_MARKER) {
                    return Err("surviving second pane lost its independent snapshot".into());
                }
                self.original_tab = Some(app.active_workspace()?.active_tab_id);
                app.new_tab()?;
                self.tab_pane = Some(
                    app.active_workspace()?
                        .active_tab()
                        .ok_or_else(|| "new tab is missing".to_owned())?
                        .focused_pane_id,
                );
                self.stage = Stage::TabReady;
            }
            Stage::TabReady => {
                let tab_pane = self
                    .tab_pane
                    .ok_or_else(|| "new tab pane was not recorded".to_owned())?;
                if app
                    .terminals
                    .get(&tab_pane)
                    .is_none_or(|runtime| runtime.snapshot.is_none())
                {
                    return Ok(TickAction::Wait);
                }
                let workspace = app.active_workspace()?;
                if workspace.tabs.len() != 2
                    || workspace
                        .active_tab()
                        .is_none_or(|tab| tab.panes.len() != 1)
                {
                    return Err("new tab did not start with exactly one pane".into());
                }
                app.switch_tab(
                    self.original_tab
                        .ok_or_else(|| "original tab was not recorded".to_owned())?,
                )?;
                self.tab_lifecycle_observed = true;
                self.stage = Stage::Settle;
            }
            Stage::Settle => {
                self.settle_ticks += 1;
                if self.settle_ticks == 1 {
                    self.stage_capture(app)?;
                }
                if self.capturing("terminal-glyphs")
                    && !pane_contains(app, self.initial_pane, "Weekly limit")
                {
                    self.settle_ticks = 1;
                    return Ok(TickAction::Wait);
                }
                if self.capturing("terminal-palette")
                    && !pane_contains(app, self.initial_pane, TERMINAL_PALETTE_MARKER)
                {
                    self.settle_ticks = 1;
                    return Ok(TickAction::Wait);
                }
                if self.capturing("selection-follow")
                    && !self.stage_selection_follow_capture(app)?
                {
                    self.settle_ticks = 1;
                    return Ok(TickAction::Wait);
                }
                if self.capturing("github-scrolled") && self.settle_ticks == 2 {
                    return Ok(TickAction::ScrollGitHubToEnd);
                }
                if self.capturing("worktree-agent-settings") && self.settle_ticks == 2 {
                    return Ok(TickAction::ScrollSettingsToEnd);
                }
                if (self.capturing("settings-github-enterprise")
                    || self.capturing("settings-github-invalid"))
                    && self.settle_ticks == 2
                {
                    return Ok(TickAction::ScrollSettingsToGitHub);
                }
                if self.capturing("github-pull-requests-scrolled") && self.settle_ticks == 2 {
                    return Ok(TickAction::ScrollGitHubPullRequestsToEnd);
                }
                if self.capturing("settings-version-mismatch") && self.settle_ticks == 2 {
                    return Ok(TickAction::ScrollSettingsToEnd);
                }
                if self.capturing("github-scrolled")
                    && self.settle_ticks >= 5
                    && app
                        .github_panel
                        .as_ref()
                        .is_none_or(|panel| panel.selected_pull_request_file_scroll_offset <= 0.0)
                {
                    return Err("GitHub file list did not report a scrolled offset".into());
                }
                if self.capturing("github-pull-requests-scrolled")
                    && self.settle_ticks >= 5
                    && app
                        .github_panel
                        .as_ref()
                        .is_none_or(|panel| panel.pull_request_scroll_offset <= 0.0)
                {
                    return Err("GitHub pull request list did not report a scrolled offset".into());
                }
                if self.settle_ticks
                    >= if self.capturing("github-scrolled")
                        || self.capturing("github-pull-requests-scrolled")
                        || self.capturing("settings-version-mismatch")
                    {
                        5
                    } else {
                        4
                    }
                {
                    self.stage = Stage::Screenshot;
                    return Ok(TickAction::Capture);
                }
            }
            Stage::Screenshot => return Ok(TickAction::Wait),
        }
        Ok(TickAction::Wait)
    }

    /// Selects content in a generic alternate-screen application, then moves
    /// that content with autonomous output rather than a wheel gesture. The
    /// final capture is admitted only after both the copied text and painted
    /// selection have followed the content to its new row.
    fn stage_selection_follow_capture(&mut self, app: &mut Muxtrix) -> Result<bool, String> {
        let Some((row, column)) =
            pane_text_position(app, self.initial_pane, SELECTION_FOLLOW_MARKER)
        else {
            return Ok(false);
        };
        if !self.capture_selection_armed {
            let last = column + SELECTION_FOLLOW_MARKER.chars().count() - 1;
            let runtime = app
                .terminals
                .get_mut(&self.initial_pane)
                .ok_or_else(|| "selection capture pane disappeared".to_owned())?;
            runtime.selection_start((column as u16, row as u16))?;
            runtime.selection_extend((last as u16, row as u16))?;
            self.capture_selection_armed = true;
            return Ok(false);
        }
        if !self.capture_selection_redraw_sent {
            if app.selected_terminal_text(self.initial_pane).as_deref()
                != Some(SELECTION_FOLLOW_MARKER)
            {
                return Ok(false);
            }
            app.terminals
                .get(&self.initial_pane)
                .and_then(|runtime| runtime.session.as_ref())
                .ok_or_else(|| "selection capture pane lost its live session".to_owned())?
                .input(
                    format!(
                        "printf '\\033[2J\\033[H{SELECTION_FOLLOW_MARKER}\\ncharlie\\ndelta\\necho\\n'\r"
                    )
                    .into_bytes(),
                )
                .map_err(|error| error.to_string())?;
            self.capture_selection_redraw_sent = true;
            return Ok(false);
        }
        if row != 0 {
            return Ok(false);
        }
        let snapshot = app
            .terminals
            .get(&self.initial_pane)
            .and_then(|runtime| runtime.snapshot.as_ref())
            .ok_or_else(|| "selection capture pane has no frame".to_owned())?;
        let selected_row = snapshot.selection.iter().position(Option::is_some);
        let selected_text = app.selected_terminal_text(self.initial_pane);
        if selected_row != Some(row) || selected_text.as_deref() != Some(SELECTION_FOLLOW_MARKER) {
            return Err(format!(
                "autonomous output moved the target to row {row}, but selection row/text are {selected_row:?}/{selected_text:?}"
            ));
        }
        self.capture_selection_follow_observed = true;
        Ok(true)
    }

    /// Puts the app into the surface named by `MUXTRIX_E2E_CAPTURE`, one
    /// tick before the frame is grabbed. Every state is staged from real
    /// application state, so a capture fails visibly when the surface it
    /// names stops rendering.
    #[allow(clippy::too_many_lines)]
    fn stage_capture(&mut self, app: &mut Muxtrix) -> Result<(), String> {
        if self.capturing("terminal-glyphs") {
            app.focus_pane(self.initial_pane)?;
            app.maximized_pane = Some(self.initial_pane);
            app.terminals
                .get(&self.initial_pane)
                .and_then(|runtime| runtime.session.as_ref())
                .ok_or_else(|| "glyph capture pane lost its live session".to_owned())?
                .input(
                    "printf '\\033[2J\\033[HClaude prompt rule\\nDesktop  Documents  Downloads  Pictures\\n────────────────────────────────────────\\nWeekly limit\\n████████████████░░░░ 80%%\\n\\033[38;2;255;0;255m╭──────────────────────────────╮\\n│ Codex · model · directory    │\\n╰──────────────────────────────╯\\033[0m\\n\\033[38;2;0;255;255m┏━━━━━━━━━━━━━━━━━━┓\\n┃ Heavy box        ┃\\n┗━━━━━━━━━━━━━━━━━━┛\\033[0m\\n\\033[38;2;0;255;0m╔══════╗ ┄┅┆┇┈┉┊┋ ╱╲╳\\n╚══════╝ ═║╬\\033[0m\\n'\r"
                        .as_bytes()
                        .to_vec(),
                )
                .map_err(|error| error.to_string())?;
        } else if self.capturing("selection-follow") {
            app.focus_pane(self.initial_pane)?;
            app.maximized_pane = Some(self.initial_pane);
            app.terminals
                .get(&self.initial_pane)
                .and_then(|runtime| runtime.session.as_ref())
                .ok_or_else(|| "selection capture pane lost its live session".to_owned())?
                .input(
                    format!(
                        "printf '\\033[?1049h\\033[2J\\033[Halpha\\n{SELECTION_FOLLOW_MARKER}\\ncharlie\\ndelta\\n'\r"
                    )
                    .into_bytes(),
                )
                .map_err(|error| error.to_string())?;
        } else if self.capturing("settings") {
            drop(app.open_settings());
        } else if self.capturing("settings-github-enterprise") {
            drop(app.open_settings());
            app.settings_draft.github_host = "github.corp.example.com".into();
        } else if self.capturing("settings-github-invalid") {
            drop(app.open_settings());
            app.settings_draft.github_host = "github.corp.example.com/api/v3".into();
        } else if self.capturing("settings-version-mismatch") {
            drop(app.open_settings());
            let installed = next_patch_version(env!("CARGO_PKG_VERSION"));
            app.installed_versions =
                super::InstalledVersionsState::Ready(super::InstalledVersions {
                    muxtrix: Ok(installed.clone()),
                    muxtrixctl: Ok(installed),
                });
        } else if self.capturing("worktree-agent-settings") {
            drop(app.open_settings());
            app.settings.default_agent = Some(Agent::Codex);
            app.settings_draft.default_agent = Some(Agent::Codex);
            app.hook_statuses = Agent::ALL
                .into_iter()
                .map(|agent| HookStatus {
                    agent,
                    scope: HookScope::User,
                    target: format!("/home/user/.config/{agent}/muxtrix-hooks").into(),
                    installed: true,
                    managed_entries: match agent {
                        Agent::Codex => 8,
                        Agent::Claude => 9,
                        Agent::Pi => 4,
                    },
                    backup_available: true,
                    unreachable_entries: 0,
                })
                .collect();
        } else if self.capturing("worktree-agent-setup") {
            app.default_agent_prompt = true;
            app.pending_default_agent_command = Some(CommandAction::NewWorktreeWithAgent(
                super::commands::WorktreeKind::Pane(SplitAxis::Horizontal),
            ));
        } else if self.capturing("worktree-agent-palette") {
            app.settings.default_agent = Some(Agent::Codex);
            app.settings_draft.default_agent = Some(Agent::Codex);
            app.hook_statuses = vec![HookStatus {
                agent: Agent::Codex,
                scope: HookScope::User,
                target: "/home/user/.codex/config.toml".into(),
                installed: true,
                managed_entries: 8,
                backup_available: true,
                unreachable_entries: 0,
            }];
            app.palette.visible = true;
            app.palette.query = "with agent".into();
            app.palette.selected = 0;
        } else if self.capturing("hook-repair") {
            // Hooks whose muxtrixctl was removed under them: they still
            // read as Muxtrix's own by their text, so the row has to
            // say why it cannot work rather than call itself installed.
            drop(app.open_settings());
            app.hook_statuses = Agent::ALL
                .into_iter()
                .map(|agent| {
                    let managed_entries = match agent {
                        Agent::Codex => 8,
                        Agent::Claude => 9,
                        Agent::Pi => 4,
                    };
                    HookStatus {
                        agent,
                        scope: HookScope::User,
                        target: match agent {
                            Agent::Codex => "/home/user/.codex/hooks.json",
                            Agent::Claude => "/home/user/.claude/settings.json",
                            Agent::Pi => "/home/user/.omp/agent/extensions/muxtrix-lifecycle.ts",
                        }
                        .into(),
                        installed: agent == Agent::Codex,
                        managed_entries,
                        backup_available: true,
                        unreachable_entries: if agent == Agent::Claude {
                            managed_entries
                        } else {
                            0
                        },
                    }
                })
                .collect();
        } else if self.settle_ticks == 1
            && (self.capturing("github-panel")
                || self.capturing("github-blocked")
                || self.capturing("github-merge-confirmation")
                || self.capturing("github-no-pr")
                || self.capturing("github-scrolled")
                || self.capturing("github-pull-requests")
                || self.capturing("github-merged-pr")
                || self.capturing("github-pull-request-search")
                || self.capturing("github-pull-requests-scrolled")
                || self.capturing("github-diff")
                || self.capturing("github-diff-binary")
                || self.capturing("github-diff-loading")
                || self.capturing("github-diff-error"))
        {
            app.github_auth = github::AuthStatus::Authenticated {
                login: "phoenixmatrix".into(),
            };
            let mut panel = staged_github_panel();
            if self.capturing("github-no-pr") {
                panel.active_tab = GitHubPanelTab::PullRequests;
                panel.pull_requests = Some(Vec::new());
            } else if self.capturing("github-pull-requests")
                || self.capturing("github-merged-pr")
                || self.capturing("github-pull-request-search")
                || self.capturing("github-pull-requests-scrolled")
            {
                panel.active_tab = GitHubPanelTab::PullRequests;
                if self.capturing("github-pull-request-search") {
                    panel.pull_request_query = "mouse".into();
                    // Reproduce filtering from a deep virtualized scroll. The
                    // final matching row must remain visible, never a blank
                    // viewport while the widget catches up to the new offset.
                    panel.pull_request_scroll_offset = 9_999.0;
                }
                if self.capturing("github-merged-pr")
                    && let Some(pull_request) = panel
                        .pull_requests
                        .as_mut()
                        .and_then(|pull_requests| pull_requests.first_mut())
                {
                    pull_request.status = github::PullRequestSummaryStatus::Merged;
                }
            } else if !self.capturing("github-panel") {
                panel.active_tab = GitHubPanelTab::PullRequests;
                panel.selected_pull_request_number = Some(391);
                let files = panel
                    .data
                    .as_ref()
                    .map_or_else(Vec::new, |data| data.files.clone());
                panel.selected_pull_request = Some(staged_github_pull_request_details(files));
            }
            panel.merge_confirmation = self.capturing("github-merge-confirmation");
            if self.capturing("github-blocked")
                && let Some(pull_request) = panel
                    .selected_pull_request
                    .as_mut()
                    .map(|details| &mut details.pull_request)
            {
                pull_request.checks.passed = 5;
                pull_request.checks.pending = 0;
                pull_request.checks.failed = 2;
                pull_request.merge_state = "BLOCKED".into();
            }
            app.github_panel = Some(panel);
            if self.capturing("github-diff")
                || self.capturing("github-diff-binary")
                || self.capturing("github-diff-loading")
                || self.capturing("github-diff-error")
            {
                app.active_view = ActiveView::GitHubDiff;
                let document = if self.capturing("github-diff") {
                    Some(github::parse_diff(STAGED_GITHUB_PATCH.as_bytes()))
                } else if self.capturing("github-diff-binary") {
                    Some(github::DiffDocument {
                        lines: Vec::new(),
                        notice: Some(
                            "This is a binary file, so there is no textual diff to display.".into(),
                        ),
                        truncated: false,
                        max_columns: 0,
                    })
                } else {
                    None
                };
                let wrap_columns = super::github_diff_wrap_columns(
                    app.window_size.width,
                    app.settings.terminal_cell_width(),
                );
                let line_starts = document.as_ref().map_or_else(
                    || vec![0],
                    |document| super::github_diff_line_starts(document, wrap_columns),
                );
                app.github_diff = Some(GitHubDiffState {
                    source: GitHubDiffSource::PullRequest(391),
                    path: "crates/muxtrix-app/src/github.rs".into(),
                    status: "Modified".into(),
                    additions: 14,
                    deletions: 1,
                    document,
                    loading: self.capturing("github-diff-loading"),
                    error: self
                        .capturing("github-diff-error")
                        .then(|| "Git could not read this file's diff.".into()),
                    generation: 1,
                    scroll_offset: 0.0,
                    wrap_columns,
                    line_starts,
                });
            }
        } else if self.settle_ticks == 1
            && (self.capturing("github-auth") || self.capturing("github-auth-collapsed"))
        {
            app.github_auth = github::AuthStatus::NeedsAuthentication;
            let mut panel = GitHubPanelState::loading(staged_repository());
            panel.active_tab = GitHubPanelTab::PullRequests;
            panel.loading = false;
            app.github_panel = Some(panel);
            app.sidebar_collapsed = self.capturing("github-auth-collapsed");
        } else if self.capturing("github-long-login") {
            // GitHub's own 39-character ceiling on a login: the rail
            // footer has to ellipsize it rather than let it run under
            // the signal dot or shove the collapse control off the
            // rail's edge.
            app.github_auth = github::AuthStatus::Authenticated {
                login: "a-very-long-github-login-name-abcdefghi".into(),
            };
        } else if self.capturing("palette") {
            app.palette.visible = true;
            app.palette.selected = 3;
        } else if self.capturing("pane-menu") {
            let focused = app
                .active_workspace()?
                .active_tab()
                .ok_or_else(|| "active tab is missing".to_owned())?
                .focused_pane_id;
            app.pane_menu = Some(focused);
        } else if self.settle_ticks == 1 && self.capturing("fleet-all-workspaces") {
            let active_workspace_id = app.session.active_workspace_id;
            if let Some(workspace) = app
                .session
                .workspaces
                .iter_mut()
                .find(|workspace| workspace.id == active_workspace_id)
            {
                workspace.name = "muxtrix-core".into();
            }
            app.agent_statuses.insert(
                self.initial_pane,
                AgentPaneStatus {
                    agent: "codex".into(),
                    display_name: Some("fleet-scope".into()),
                    state: AgentState::Waiting,
                    activity: Some("Needs approval".into()),
                    session_id: None,
                    cwd: Some("/home/user/dev/muxtrix".into()),
                    git_branch: Some("main".into()),
                },
            );
            app.workspace_name_draft = "release-audit".into();
            app.create_workspace()?;
            let release_pane = app
                .active_workspace()?
                .active_tab()
                .ok_or_else(|| "capture workspace has no active tab".to_owned())?
                .focused_pane_id;
            app.agent_statuses.insert(
                release_pane,
                AgentPaneStatus {
                    agent: "claude".into(),
                    display_name: Some("release-audit".into()),
                    state: AgentState::Running,
                    activity: Some("Running release checks".into()),
                    session_id: None,
                    cwd: Some("/home/user/dev/muxtrix".into()),
                    git_branch: Some("release".into()),
                },
            );
            app.settings.fleet_scope = FleetScope::AllWorkspaces;
            app.settings.fleet_view = FleetView::Tabs;
        } else if self.capturing("fleet-agents") {
            // Stage different harnesses across both tabs so the capture
            // proves Agents is one flat selected-workspace projection.
            app.agent_statuses.insert(
                self.initial_pane,
                AgentPaneStatus {
                    agent: "codex".into(),
                    display_name: Some("review-terminal-host".into()),
                    state: AgentState::Waiting,
                    activity: Some("Needs approval".into()),
                    session_id: None,
                    cwd: Some("/home/user/dev/muxtrix".into()),
                    git_branch: Some("main".into()),
                },
            );
            app.agent_statuses.insert(
                self.second_pane()?,
                AgentPaneStatus {
                    agent: "claude".into(),
                    display_name: Some(
                        "agent-title-sidebar-layout-hardening-with-a-very-long-name".into(),
                    ),
                    state: AgentState::Running,
                    activity: Some("Refactoring the fleet rail".into()),
                    session_id: None,
                    cwd: Some("/home/user/dev/muxtrix".into()),
                    git_branch: Some("main".into()),
                },
            );
            app.agent_statuses.insert(
                self.tab_pane
                    .ok_or_else(|| "new-tab pane was not recorded".to_owned())?,
                AgentPaneStatus {
                    agent: "pi".into(),
                    display_name: Some("oh-my-pi-release-audit".into()),
                    state: AgentState::Completed,
                    activity: Some("Oh My Pi checks passed".into()),
                    session_id: None,
                    cwd: Some("/home/user/dev/muxtrix".into()),
                    git_branch: Some("release".into()),
                },
            );
            for (pane_id, worktree_name) in [
                (self.initial_pane, Some("terminal-host-review")),
                (
                    self.second_pane()?,
                    Some("fleet-two-line-rows-with-an-extra-long-worktree-name"),
                ),
                (
                    self.tab_pane
                        .ok_or_else(|| "new-tab pane was not recorded".to_owned())?,
                    Some("oh-my-pi-release-audit"),
                ),
            ] {
                let directory = app
                    .pane_working_directory(pane_id)
                    .ok_or_else(|| "capture pane has no working directory".to_owned())?;
                app.pane_repositories.insert(
                    pane_id,
                    PaneRepository {
                        directory,
                        name: Some("muxtrix".into()),
                        worktree_name: worktree_name.map(str::to_owned),
                    },
                );
            }
            app.settings.fleet_view = FleetView::Agents;
        } else if self.capturing("fleet-agents-empty") {
            app.settings.fleet_view = FleetView::Agents;
        } else if self.settle_ticks == 1
            && (self.capturing("worktree-manager") || self.capturing("worktree-switcher"))
        {
            // Stage a synthetic manager so the dialog renders
            // populated; the state exists only for the frame.
            app.worktree_manager = Some(WorktreeManagerState {
                mode: if self.capturing("worktree-switcher") {
                    WorktreeManagerMode::RestartPane(self.initial_pane)
                } else {
                    WorktreeManagerMode::Manage
                },
                generation: 1,
                repo_root: Some("/home/user/dev/muxtrix".into()),
                failure: None,
                entries: vec![
                    WorktreeManagerEntry {
                        path: "/home/user/dev/muxtrix".into(),
                        branch: Some("release".into()),
                        unpushed_commits: 0,
                        deletion_blocker: Some("Primary worktree".into()),
                        used_by: None,
                    },
                    WorktreeManagerEntry {
                        path: "/home/user/.muxtrix/worktrees/muxtrix/agent-title-sidebar-layout-hardening-with-a-very-long-name".into(),
                        branch: Some(
                            "agent-title-sidebar-layout-hardening-with-a-very-long-name"
                                .into(),
                        ),
                        unpushed_commits: 3,
                        deletion_blocker: None,
                        used_by: Some(
                            "shell with an exceptionally long descriptive pane title"
                                .into(),
                        ),
                    },
                    WorktreeManagerEntry {
                        path: "/home/user/.muxtrix/worktrees/muxtrix/main".into(),
                        branch: Some("main".into()),
                        unpushed_commits: 1,
                        deletion_blocker: None,
                        used_by: None,
                    },
                ],
                loading: false,
                // Starts at the top so the settings capture can walk
                // the selection down with real key events instead of
                // staging the result it wants to prove.
                selected: 0,
                busy: false,
                error: None,
                restart_target: None,
            });
            if self.capturing("worktree-manager") {
                app.active_view = ActiveView::Settings;
                app.settings_page = super::SettingsPage::Worktrees;
            } else if let Some(manager) = app.worktree_manager.as_mut() {
                // The restart dialog has no keyboard step of its own
                // here; keep its previous framing.
                manager.selected = 2;
            }
        } else if self.settle_ticks == 2 && self.capturing("worktree-manager") {
            // Drive the advertised navigation through the real key
            // path, so the captured frame is evidence that Down
            // reaches the inventory rather than a staged selection.
            let _ = app.handle_keyboard(arrow_down());
            let _ = app.handle_keyboard(arrow_down());
            let selected = app
                .worktree_manager
                .as_ref()
                .map(|manager| manager.selected);
            if selected != Some(2) {
                return Err(format!(
                    "worktree settings ignored Down: selection is {selected:?}, expected Some(2)"
                ));
            }
        } else if self.capturing("worktree-restart-confirmation") {
            app.worktree_manager = Some(WorktreeManagerState {
                mode: WorktreeManagerMode::RestartPane(self.initial_pane),
                generation: 1,
                repo_root: Some("/home/user/dev/muxtrix".into()),
                failure: None,
                entries: vec![WorktreeManagerEntry {
                    path: "/home/user/.muxtrix/worktrees/muxtrix/feature-ui".into(),
                    branch: Some("feature-ui".into()),
                    unpushed_commits: 0,
                    deletion_blocker: None,
                    used_by: None,
                }],
                loading: false,
                selected: 0,
                busy: false,
                error: None,
                restart_target: Some(0),
            });
        } else if self.capturing("toast") {
            app.toast = Some(("Copied to clipboard".into(), std::time::Instant::now()));
        } else if self.capturing("theme-gallery") {
            app.active_view = ActiveView::ThemeGallery;
        } else if self.capturing("session-picker") {
            // Synthetic startup picker; exists only for the frame.
            app.session_picker = Some(super::SessionPickerState {
                entries: vec![
                    super::SessionPickerEntry {
                        record: muxtrix_sessions::SessionRecord {
                            id: uuid::Uuid::new_v4(),
                            name: "muxtrix".into(),
                            endpoint: String::new(),
                            process_id: 0,
                            created_unix: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |elapsed| elapsed.as_secs())
                                .saturating_sub(7_200),
                            layout: None,
                            attached: false,
                            version: "0.1.31".into(),
                        },
                        alive: true,
                        pane_count: 3,
                    },
                    super::SessionPickerEntry {
                        record: muxtrix_sessions::SessionRecord {
                            id: uuid::Uuid::new_v4(),
                            name: "experiments".into(),
                            endpoint: String::new(),
                            process_id: 0,
                            created_unix: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |elapsed| elapsed.as_secs())
                                .saturating_sub(7_200),
                            layout: None,
                            attached: false,
                            version: env!("CARGO_PKG_VERSION").into(),
                        },
                        alive: false,
                        pane_count: 1,
                    },
                ],
                selected: 0,
                error: None,
                startup: true,
            });
        } else if self.capturing("worktree-dialog") {
            let _ = app.run_command(super::CommandAction::RestartPaneInWorktree);
            app.worktree_name_draft = "worktree-2".into();
            if let Some(prompt) = app.worktree_prompt.as_mut() {
                prompt.repo_root = Some("/home/user/dev/muxtrix".into());
                prompt.failure = None;
                prompt.base_directory =
                    Some("/home/user/an-extraordinarily-long-home-directory-name/.muxtrix/worktrees/muxtrix".into());
            }
        } else if self.capturing("close-workspace") {
            app.close_workspace_prompt = Some(app.session.active_workspace_id);
        } else if self.capturing("rail-nav") || self.capturing("rail-nav-collapsed") {
            // Park the prefix-navigation cursor on the second fleet pane so the
            // frame carries the cursor and the really focused pane at once —
            // the one comparison this capture exists to make. Group bands are
            // targets too, and landing on one would show the cursor with
            // nothing to contrast it against.
            let targets = app.rail_targets();
            app.rail_nav = targets
                .iter()
                .filter(|target| matches!(target, super::RailTarget::FleetPane(..)))
                .nth(1)
                .copied();
            // The collapsed rail draws the same cursor with none of the room,
            // so it gets its own frame rather than an assumption.
            app.sidebar_collapsed = self.capturing("rail-nav-collapsed");
        } else if self.capturing("stacked-layout") {
            app.split_terminal(SplitAxis::Vertical)?;
            app.cycle_pane_layout(super::LayoutCycle::Next)?;
            app.cycle_pane_layout(super::LayoutCycle::Next)?;
            app.cycle_pane_layout(super::LayoutCycle::Next)?;
        } else if self.capturing("needs-input") {
            // Stage a waiting agent so the whole-pane amber treatment
            // is capturable; this state exists only for the frame.
            app.agent_statuses.insert(
                self.second_pane()?,
                AgentPaneStatus {
                    agent: "claude".into(),
                    display_name: Some("permission-flow".into()),
                    state: AgentState::Waiting,
                    activity: Some("Permission required".into()),
                    session_id: None,
                    cwd: None,
                    git_branch: None,
                },
            );
        } else if self.capturing("agents-roster") {
            // A pane projecting Claude Code's roster beside an ordinary
            // agent pane, so the hollow roll-up pip and the solid
            // lifecycle pip are comparable in one rail.
            let roster_pane = self.second_pane()?;
            app.agent_statuses.insert(
                roster_pane,
                AgentPaneStatus {
                    agent: "claude".into(),
                    display_name: Some("fleet-supervisor".into()),
                    state: AgentState::Idle,
                    activity: None,
                    session_id: None,
                    cwd: None,
                    git_branch: None,
                },
            );
            // Emit the harness's real Agents-view title so the capture
            // exercises detection rather than a staged flag.
            app.terminals
                .get(&roster_pane)
                .and_then(|runtime| runtime.session.as_ref())
                .ok_or_else(|| "roster pane lost its live session".to_owned())?
                .input(
                    b"printf '\\033]0;1 awaiting input \\302\\267 claude agents\\007'\r".to_vec(),
                )
                .map_err(|error| error.to_string())?;
            // Counts are staged, and the in-flight guard is held down so
            // a real `claude agents --json` on this machine cannot make
            // the captured frame depend on the host's own sessions.
            //
            // A finished fleet is staged deliberately: it is both the
            // state a healthy fleet spends most of its time in and the
            // quietest colour the roll-up can paint, so the capture
            // fails visibly if the roster pip ever stops reading.
            app.agents_roster = Some(super::agents_roster::AgentsRoster {
                working: 0,
                blocked: 0,
                failed: 0,
                completed: 4,
                idle: 2,
            });
            app.agents_roster_pending = true;
            // The neighbouring row keeps its ordinary lifecycle pip, so
            // the two treatments sit adjacent in the same tab band.
            let neighbour = app
                .fleet_entries()
                .into_iter()
                .map(|(_, pane_id)| pane_id)
                .find(|pane_id| *pane_id != roster_pane)
                .ok_or("no neighbouring pane to contrast the roster row")?;
            app.agent_statuses.insert(
                neighbour,
                AgentPaneStatus {
                    agent: "claude".into(),
                    display_name: Some("port-idle-rule".into()),
                    state: AgentState::Running,
                    activity: Some("Editing agent_screen.rs".into()),
                    session_id: None,
                    cwd: None,
                    git_branch: None,
                },
            );
        } else if self.capturing("fleet-repos") {
            // Repos groups the fleet by the checkout each pane reports over
            // OSC 7. The harness shell runs outside any repository, so the
            // mapping is staged to give the grouping something to group.
            let second = self.second_pane()?;
            let tab_pane = self
                .tab_pane
                .ok_or_else(|| "new-tab pane was not recorded".to_owned())?;
            app.pane_repositories.insert(
                self.initial_pane,
                PaneRepository {
                    directory: "/home/user/dev/muxtrix".into(),
                    name: Some("muxtrix".into()),
                    worktree_name: None,
                },
            );
            app.pane_repositories.insert(
                second,
                PaneRepository {
                    directory: "/home/user/.muxtrix/worktrees/muxtrix/feature-ui".into(),
                    name: Some("muxtrix".into()),
                    worktree_name: Some("feature-ui".into()),
                },
            );
            app.pane_repositories.insert(
                tab_pane,
                PaneRepository {
                    directory: "/home/user/dev/impeccable".into(),
                    name: Some("impeccable".into()),
                    worktree_name: None,
                },
            );
            app.agent_statuses.insert(
                second,
                AgentPaneStatus {
                    agent: "claude".into(),
                    display_name: Some("feature-ui".into()),
                    state: AgentState::Running,
                    activity: Some("Rewriting the rail".into()),
                    session_id: None,
                    cwd: Some("/home/user/.muxtrix/worktrees/muxtrix/feature-ui".into()),
                    git_branch: Some("feature-ui".into()),
                },
            );
            app.settings.fleet_view = FleetView::Repos;
        } else if self.capturing("fleet-tabs-duplicates") {
            // Tabs view should spend the pane row on pane-specific activity
            // when the worktree/repository line already names the checkout.
            // The renamed tab bands stay visible as the grouping context.
            let second = self.second_pane()?;
            let tab_pane = self
                .tab_pane
                .ok_or_else(|| "new-tab pane was not recorded".to_owned())?;
            let original_tab = self
                .original_tab
                .ok_or_else(|| "original tab was not recorded".to_owned())?;
            for tab in &mut app.active_workspace_mut()?.tabs {
                if tab.id == original_tab {
                    tab.name = "backend-review".into();
                } else if tab.panes.contains_key(&tab_pane) {
                    tab.name = "feature-ui".into();
                }
            }
            let stage_pane =
                |app: &mut Muxtrix, pane_id: PaneId, worktree: &str| -> Result<(), String> {
                    let directory = app
                        .pane_working_directory(pane_id)
                        .ok_or_else(|| format!("pane {pane_id:?} has no working directory"))?;
                    app.pane_repositories.insert(
                        pane_id,
                        PaneRepository {
                            directory,
                            name: Some("muxtrix".into()),
                            worktree_name: Some(worktree.into()),
                        },
                    );
                    let pane = app
                        .session
                        .workspaces
                        .iter_mut()
                        .find_map(|workspace| workspace.pane_mut(pane_id))
                        .ok_or_else(|| format!("pane {pane_id:?} is missing"))?;
                    let surface = pane
                        .surfaces
                        .iter_mut()
                        .find(|surface| surface.id == pane.active_surface_id)
                        .ok_or_else(|| format!("pane {pane_id:?} has no active surface"))?;
                    surface.title = worktree.into();
                    Ok(())
                };
            stage_pane(app, self.initial_pane, "muxtrix")?;
            stage_pane(app, second, "feature-ui")?;
            stage_pane(app, tab_pane, "feature-ui")?;
            app.settings.fleet_view = FleetView::Tabs;
        } else if self.capturing("fleet-tabs-collapsed") {
            app.sidebar_collapsed = true;
        } else if self.capturing("fleet-agents-collapsed") {
            app.agent_statuses.insert(
                self.second_pane()?,
                AgentPaneStatus {
                    agent: "claude".into(),
                    display_name: Some("feature-ui".into()),
                    state: AgentState::Waiting,
                    activity: Some("Needs approval".into()),
                    session_id: None,
                    cwd: None,
                    git_branch: None,
                },
            );
            app.settings.fleet_view = FleetView::Agents;
            app.sidebar_collapsed = true;
        } else if self.capturing("agent-lifecycle-states") {
            // One pane per lifecycle colour, so the whole vocabulary of the
            // Agents rail is comparable in a single frame.
            let second = self.second_pane()?;
            let tab_pane = self
                .tab_pane
                .ok_or_else(|| "new-tab pane was not recorded".to_owned())?;
            for (pane_id, agent, name, state, activity) in [
                (
                    self.initial_pane,
                    "codex",
                    "migrate-control-service",
                    AgentState::Failed,
                    Some("cargo test exited 101"),
                ),
                (
                    second,
                    "claude",
                    "harden-pty-resize",
                    AgentState::Stopped,
                    Some("Interrupted"),
                ),
                (tab_pane, "pi", "idle-scout", AgentState::Idle, None),
            ] {
                app.agent_statuses.insert(
                    pane_id,
                    AgentPaneStatus {
                        agent: agent.into(),
                        display_name: Some(name.into()),
                        state,
                        activity: activity.map(Into::into),
                        session_id: None,
                        cwd: Some("/home/user/dev/muxtrix".into()),
                        git_branch: Some("main".into()),
                    },
                );
            }
            app.settings.fleet_view = FleetView::Agents;
        } else if self.capturing("pane-attention") {
            // Unread agent activity on a pane the user is not looking at, so
            // the rail paints its attention affordance rather than clearing it.
            let second = self.second_pane()?;
            app.focus_pane(self.initial_pane)?;
            if let Some(pane) = app.active_workspace_mut()?.pane_mut(second) {
                pane.attention.unread_count = 3;
                pane.attention.message = Some("Needs approval to run cargo test".into());
            }
            app.notifications.push(super::AgentNotification {
                pane_id: second,
                unread: true,
            });
            app.agent_statuses.insert(
                second,
                AgentPaneStatus {
                    agent: "codex".into(),
                    display_name: Some("release-audit".into()),
                    state: AgentState::Waiting,
                    activity: Some("Needs approval".into()),
                    session_id: None,
                    cwd: None,
                    git_branch: None,
                },
            );
        } else if self.capturing("global-alert") {
            app.global_alerts.push(super::GlobalAlert {
                title: "codex · release-audit".into(),
                body: "Sandbox denied a write outside the workspace".into(),
            });
            app.global_alerts.push(super::GlobalAlert {
                title: "claude · feature-ui".into(),
                body: "Waiting on approval to run cargo test".into(),
            });
        } else if self.capturing("maximized-pane") {
            app.focus_pane(self.second_pane()?)?;
            app.maximized_pane = Some(self.second_pane()?);
        } else if self.capturing("prefix-armed") {
            app.prefix_armed = true;
        } else if self.capturing("workspace-create") {
            app.workspace_create_visible = true;
            app.workspace_name_draft = "release-audit".into();
        } else if self.capturing("rename-workspace") {
            app.rename_prompt = Some(super::RenameTarget::Workspace(
                app.session.active_workspace_id,
            ));
            app.rename_draft = "Release audit".into();
        } else if self.capturing("rename-tab") {
            let workspace_id = app.session.active_workspace_id;
            let tab_id = app.active_workspace()?.active_tab_id;
            app.rename_prompt = Some(super::RenameTarget::Tab(workspace_id, tab_id));
            app.rename_draft = "review".into();
        } else if self.capturing("rename-pane") {
            app.rename_prompt = Some(super::RenameTarget::Pane(self.initial_pane));
            app.rename_draft = "build watcher".into();
        } else if self.capturing("many-tabs") {
            for _ in 0..6 {
                app.new_tab()?;
            }
        } else if self.capturing("many-workspaces") {
            for name in ["release-audit", "spike", "docs", "incident-4821"] {
                app.workspace_name_draft = name.into();
                app.create_workspace()?;
            }
        } else if self.capturing("palette-query") {
            app.palette.visible = true;
            app.palette.query = "work".into();
            app.palette.selected = 1;
        } else if self.capturing("palette-empty-query") {
            app.palette.visible = true;
            app.palette.query = "zzzz-no-such-command".into();
            app.palette.selected = 0;
        } else if self.capturing("settings-worktrees-loading") {
            app.active_view = ActiveView::Settings;
            app.settings_page = super::SettingsPage::Worktrees;
            app.worktree_manager = Some(WorktreeManagerState {
                mode: WorktreeManagerMode::Manage,
                generation: 1,
                repo_root: Some("/home/user/dev/muxtrix".into()),
                failure: None,
                entries: Vec::new(),
                loading: true,
                selected: 0,
                busy: false,
                error: None,
                restart_target: None,
            });
        } else if self.capturing("worktree-manager-error") {
            app.active_view = ActiveView::Settings;
            app.settings_page = super::SettingsPage::Worktrees;
            app.worktree_manager = Some(WorktreeManagerState {
                mode: WorktreeManagerMode::Manage,
                generation: 1,
                repo_root: Some("/home/user/dev/muxtrix".into()),
                failure: None,
                entries: vec![WorktreeManagerEntry {
                    path: "/home/user/.muxtrix/worktrees/muxtrix/feature-ui".into(),
                    branch: Some("feature-ui".into()),
                    unpushed_commits: 2,
                    deletion_blocker: None,
                    used_by: None,
                }],
                loading: false,
                selected: 0,
                busy: false,
                error: Some(
                    "git worktree remove failed: feature-ui contains modified or untracked files"
                        .into(),
                ),
                restart_target: None,
            });
        } else if self.capturing("worktree-manager-no-repo") {
            app.active_view = ActiveView::Settings;
            app.settings_page = super::SettingsPage::Worktrees;
            app.worktree_manager = Some(WorktreeManagerState {
                mode: WorktreeManagerMode::Manage,
                generation: 1,
                repo_root: None,
                failure: Some("The focused pane is not inside a Git repository".into()),
                entries: Vec::new(),
                loading: false,
                selected: 0,
                busy: false,
                error: None,
                restart_target: None,
            });
        } else if self.capturing("session-picker-error") {
            app.session_picker = Some(super::SessionPickerState {
                entries: Vec::new(),
                selected: 0,
                error: Some("Could not read the session registry: permission denied".into()),
                startup: false,
            });
        } else if self.capturing("github-loading") {
            app.github_auth = github::AuthStatus::Authenticated {
                login: "phoenixmatrix".into(),
            };
            let mut panel = GitHubPanelState::loading(staged_repository());
            panel.loading_phase = 4;
            app.github_panel = Some(panel);
        } else if self.capturing("github-pull-requests-loading") {
            app.github_auth = github::AuthStatus::Authenticated {
                login: "phoenixmatrix".into(),
            };
            let mut panel = staged_github_panel();
            panel.active_tab = GitHubPanelTab::PullRequests;
            panel.pull_requests = None;
            panel.pull_requests_loading = true;
            panel.loading_phase = 4;
            app.github_panel = Some(panel);
        } else if self.capturing("github-refreshing") {
            app.github_auth = github::AuthStatus::Authenticated {
                login: "phoenixmatrix".into(),
            };
            let mut panel = staged_github_panel();
            panel.loading = true;
            panel.loading_phase = 4;
            app.github_panel = Some(panel);
        } else if self.capturing("github-error") {
            app.github_auth = github::AuthStatus::Authenticated {
                login: "phoenixmatrix".into(),
            };
            let mut panel = GitHubPanelState::loading(staged_repository());
            panel.loading = false;
            panel.error = Some("Git could not read the focused pane's working tree.".into());
            app.github_panel = Some(panel);
        } else if self.capturing("github-unavailable") {
            app.github_auth = github::AuthStatus::Unavailable {
                reason: "The GitHub CLI (gh) is not installed".into(),
            };
            let mut panel = GitHubPanelState::loading(staged_repository());
            panel.active_tab = GitHubPanelTab::PullRequests;
            panel.loading = false;
            app.github_panel = Some(panel);
        } else if self.capturing("github-merging") {
            app.github_auth = github::AuthStatus::Authenticated {
                login: "phoenixmatrix".into(),
            };
            let mut panel = staged_github_pull_request_panel();
            panel.merging = true;
            panel.loading_phase = 4;
            app.github_panel = Some(panel);
        } else if self.capturing("github-draft-pr") || self.capturing("github-draft-error") {
            app.github_auth = github::AuthStatus::Authenticated {
                login: "phoenixmatrix".into(),
            };
            let mut panel = staged_github_pull_request_panel();
            if let Some(pull_request) = panel
                .selected_pull_request
                .as_mut()
                .map(|details| &mut details.pull_request)
            {
                pull_request.draft = true;
                pull_request.review_decision = "REVIEW_REQUIRED".into();
                pull_request.merge_state = "DRAFT".into();
                pull_request.checks.passed = 4;
                pull_request.checks.pending = 3;
                pull_request.checks.failed = 0;
            }
            if self.capturing("github-draft-error") {
                panel.pull_request_action_error =
                    Some("GitHub could not mark this pull request ready for review.".into());
            }
            app.github_panel = Some(panel);
        } else if self.capturing("layout-vertical") {
            app.split_terminal(SplitAxis::Vertical)?;
            app.cycle_pane_layout(super::LayoutCycle::Next)?;
        } else if self.capturing("layout-horizontal") {
            app.split_terminal(SplitAxis::Vertical)?;
            app.cycle_pane_layout(super::LayoutCycle::Next)?;
            app.cycle_pane_layout(super::LayoutCycle::Next)?;
        } else if self.capturing("layout-half-stacked") {
            app.split_terminal(SplitAxis::Vertical)?;
            for _ in 0..4 {
                app.cycle_pane_layout(super::LayoutCycle::Next)?;
            }
        } else if self.capturing("four-panes") {
            app.split_terminal(SplitAxis::Vertical)?;
            app.split_terminal(SplitAxis::Horizontal)?;
        } else if self.capturing("terminal-palette") {
            // A colour chart makes a terminal theme legible in one frame; the
            // theme-variation captures all end on this surface.
            app.focus_pane(self.initial_pane)?;
            app.maximized_pane = Some(self.initial_pane);
            app.terminals
                .get(&self.initial_pane)
                .and_then(|runtime| runtime.session.as_ref())
                .ok_or_else(|| "palette capture pane lost its live session".to_owned())?
                .input(TERMINAL_PALETTE_SCRIPT.as_bytes().to_vec())
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn second_pane(&self) -> Result<PaneId, String> {
        self.second_pane
            .ok_or_else(|| "second pane was not recorded".into())
    }

    fn third_pane(&self) -> Result<PaneId, String> {
        self.third_pane
            .ok_or_else(|| "third pane was not recorded".into())
    }

    fn success_report(&self, screenshot: &iced::window::Screenshot) -> Result<(), String> {
        if !self.palette_open_observed {
            return Err("Ctrl+P did not open the command palette".into());
        }
        if !self.palette_navigation_observed {
            return Err("keyboard navigation did not move the command palette selection".into());
        }
        if !self.settings_open_observed {
            return Err("Ctrl+Comma did not open settings".into());
        }
        if !self.fleet_collapse_observed || !self.pane_maximize_observed || !self.pane_menu_observed
        {
            return Err("fleet and pane chrome controls were not all observed".into());
        }
        if !self.pane_menu_click_away_observed {
            return Err("a real outside click did not dismiss the pane overflow menu".into());
        }
        if !self.selection_observed {
            return Err("a pointer drag did not select the text it crossed".into());
        }
        if self.capturing("selection-follow") && !self.capture_selection_follow_observed {
            return Err("selection did not follow autonomous alternate-screen output".into());
        }
        if !self.pane_grids_match_panes {
            return Err("split panes did not all render a grid matching their size".into());
        }
        if !self.terminal_scroll_observed {
            return Err("mouse-wheel terminal scrolling was not observed".into());
        }
        if !self.terminal_scrollbar_observed {
            return Err("terminal scrollbar interaction was not observed".into());
        }
        if !self.terminal_mouse_reporting_observed {
            return Err("a mouse-reporting program did not receive pointer motion".into());
        }
        if !self.tab_lifecycle_observed {
            return Err("workspace tab creation was not observed".into());
        }
        let expected_bytes = screenshot.size.width as usize * screenshot.size.height as usize * 4;
        if screenshot.rgba.len() != expected_bytes {
            return Err("GPU screenshot byte length did not match its dimensions".into());
        }
        let mut colors = HashSet::new();
        let mut opaque_pixels = 0_usize;
        for pixel in screenshot.rgba.chunks_exact(4) {
            colors.insert([pixel[0], pixel[1], pixel[2]]);
            opaque_pixels += usize::from(pixel[3] == 255);
            if colors.len() > 64 && opaque_pixels > 10_000 {
                break;
            }
        }
        if colors.len() < 16 || opaque_pixels < 10_000 {
            return Err("GPU screenshot did not contain a populated application frame".into());
        }
        let terminal_glyph_continuity = self
            .capturing("terminal-glyphs")
            .then(|| light_horizontal_continuity(screenshot));
        let rounded_box_continuity = self
            .capturing("terminal-glyphs")
            .then(|| magenta_rounded_box_continuity(screenshot));
        let heavy_box_continuity = self
            .capturing("terminal-glyphs")
            .then(|| cyan_heavy_box_continuity(screenshot));
        if terminal_glyph_continuity.is_some_and(|(longest, solid_rows)| {
            longest < TERMINAL_RULE_CONTINUITY_PIXELS || solid_rows < TERMINAL_BLOCK_CONTINUITY_ROWS
        }) {
            return Err(format!(
                "terminal drawing glyphs did not produce a {TERMINAL_RULE_CONTINUITY_PIXELS}-pixel rule and at least {TERMINAL_BLOCK_CONTINUITY_ROWS} rows of a {TERMINAL_BLOCK_CONTINUITY_PIXELS}-pixel block"
            ));
        }
        if rounded_box_continuity.is_some_and(|(connected, width, height)| {
            !connected
                || width < TERMINAL_ROUNDED_BOX_WIDTH_PIXELS
                || height < TERMINAL_ROUNDED_BOX_HEIGHT_PIXELS
        }) {
            return Err(format!(
                "rounded terminal border was not one connected component spanning at least {TERMINAL_ROUNDED_BOX_WIDTH_PIXELS}x{TERMINAL_ROUNDED_BOX_HEIGHT_PIXELS} pixels"
            ));
        }
        if heavy_box_continuity.is_some_and(|(connected, width, height)| {
            !connected
                || width < TERMINAL_HEAVY_BOX_WIDTH_PIXELS
                || height < TERMINAL_HEAVY_BOX_HEIGHT_PIXELS
        }) {
            return Err(format!(
                "heavy terminal border was not one connected component spanning at least {TERMINAL_HEAVY_BOX_WIDTH_PIXELS}x{TERMINAL_HEAVY_BOX_HEIGHT_PIXELS} pixels"
            ));
        }
        if let Some(path) = std::env::var_os(SCREENSHOT_ENV) {
            std::fs::write(path, &screenshot.rgba).map_err(|error| error.to_string())?;
        }

        self.write_report(json!({
            "success": true,
            "checks": {
                "real_window_and_wgpu_frame": true,
                "command_palette_shortcut_and_render": true,
                "command_palette_keyboard_navigation": true,
                "settings_shortcut_and_render": true,
                "external_keyboard_input_with_spaces": true,
                "terminal_url_decoration_rendered": true,
                "focused_cursor_visible": true,
                "horizontal_split_resized": true,
                "vertical_split_resized": true,
                "split_pane_grids_match_their_panes": true,
                "pointer_drag_selects_terminal_text": true,
                "independent_terminal_sessions": true,
                "focused_pane_close_cleanup": true,
                "terminal_exit_detach_and_restart": true,
                "osc_agent_notification_fleet_attention_and_clear": true,
                "fleet_collapse_pane_maximize_and_overflow": true,
                "pane_overflow_click_away": true,
                "terminal_mouse_wheel_scrollback": true,
                "terminal_scrollbar_click_and_drag": true,
                "terminal_program_mouse_motion": true,
                "workspace_tab_default_pane_and_switch": true,
                "terminal_drawing_glyphs_are_pixel_continuous": terminal_glyph_continuity
                    .is_none_or(|(longest, solid_rows)| {
                        longest >= TERMINAL_RULE_CONTINUITY_PIXELS
                            && solid_rows >= TERMINAL_BLOCK_CONTINUITY_ROWS
                    }),
                "terminal_rounded_box_is_pixel_connected": rounded_box_continuity
                    .is_none_or(|(connected, width, height)| {
                        connected
                            && width >= TERMINAL_ROUNDED_BOX_WIDTH_PIXELS
                            && height >= TERMINAL_ROUNDED_BOX_HEIGHT_PIXELS
                    }),
                "terminal_heavy_box_is_pixel_connected": heavy_box_continuity
                    .is_none_or(|(connected, width, height)| {
                        connected
                            && width >= TERMINAL_HEAVY_BOX_WIDTH_PIXELS
                            && height >= TERMINAL_HEAVY_BOX_HEIGHT_PIXELS
                    })
            },
            "metrics": {
                "screenshot_width": screenshot.size.width,
                "screenshot_height": screenshot.size.height,
                "screenshot_unique_colors_at_least": colors.len(),
                "initial_columns": self.initial_cols,
                "initial_rows": self.initial_rows,
                "terminal_drawing_longest_continuous_run": terminal_glyph_continuity
                    .map(|(longest, _)| longest),
                "terminal_block_continuous_rows": terminal_glyph_continuity
                    .map(|(_, solid_rows)| solid_rows),
                "terminal_rounded_box_width": rounded_box_continuity
                    .map(|(_, width, _)| width),
                "terminal_rounded_box_height": rounded_box_continuity
                    .map(|(_, _, height)| height),
                "terminal_heavy_box_width": heavy_box_continuity
                    .map(|(_, width, _)| width),
                "terminal_heavy_box_height": heavy_box_continuity
                    .map(|(_, _, height)| height)
            }
        }))
    }

    fn failure_report(&self, error: &str) {
        let _ = self.write_report(json!({
            "success": false,
            "stage": format!("{:?}", self.stage),
            "error": error
        }));
    }

    fn write_report(&self, report: serde_json::Value) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
        std::fs::write(&self.report_path, bytes).map_err(|error| error.to_string())
    }

    pub(super) fn observe_terminal_scroll(&mut self) {
        self.terminal_scroll_observed = true;
    }

    pub(super) fn observe_terminal_scrollbar(&mut self) {
        self.terminal_scrollbar_observed = true;
    }
}

fn next_patch_version(version: &str) -> String {
    let Some((prefix, patch)) = version.rsplit_once('.') else {
        return format!("{version}+installed");
    };
    patch.parse::<u64>().map_or_else(
        |_| format!("{version}+installed"),
        |patch| format!("{prefix}.{}", patch + 1),
    )
}

impl Muxtrix {
    pub(super) fn drive_e2e(&mut self) -> Task<Message> {
        let Some(mut scenario) = self.e2e.take() else {
            return Task::none();
        };
        match scenario.tick(self) {
            Ok(TickAction::Wait) => {
                self.e2e = Some(scenario);
                Task::none()
            }
            Ok(TickAction::ScrollSettingsToEnd) => {
                self.e2e = Some(scenario);
                iced::widget::operation::snap_to_end(iced::widget::Id::new(
                    super::SETTINGS_SCROLL_ID,
                ))
            }
            Ok(TickAction::ScrollSettingsToGitHub) => {
                self.e2e = Some(scenario);
                iced::widget::operation::snap_to(
                    iced::widget::Id::new(super::SETTINGS_SCROLL_ID),
                    iced::widget::operation::RelativeOffset { x: 0.0, y: 0.65 },
                )
            }
            Ok(TickAction::ScrollGitHubToEnd) => {
                self.e2e = Some(scenario);
                iced::widget::operation::snap_to_end(iced::widget::Id::new(
                    super::GITHUB_FILE_SCROLL_ID,
                ))
            }
            Ok(TickAction::ScrollGitHubPullRequestsToEnd) => {
                self.e2e = Some(scenario);
                iced::widget::operation::snap_to_end(iced::widget::Id::new(
                    super::GITHUB_PULL_REQUEST_SCROLL_ID,
                ))
            }
            Ok(TickAction::Capture) => {
                self.e2e = Some(scenario);
                iced::window::latest().then(|window| match window {
                    Some(window) => iced::window::screenshot(window).map(Message::E2eScreenshot),
                    None => Task::done(Message::E2eWindowMissing),
                })
            }
            Err(error) => {
                scenario.failure_report(&error);
                iced::exit()
            }
        }
    }

    pub(super) fn finish_e2e(&mut self, screenshot: iced::window::Screenshot) -> Task<Message> {
        let Some(scenario) = self.e2e.take() else {
            return iced::exit();
        };
        if let Err(error) = scenario.success_report(&screenshot) {
            scenario.failure_report(&error);
        }
        iced::exit()
    }

    pub(super) fn fail_e2e(&mut self, error: &str) -> Task<Message> {
        if let Some(scenario) = self.e2e.take() {
            scenario.failure_report(error);
        }
        iced::exit()
    }
}

/// The repository every GitHub capture reports on, so the panel's header,
/// error, and authentication states describe the same checkout.
fn staged_repository() -> github::Repository {
    github::Repository {
        root: "/home/user/.muxtrix/worktrees/muxtrix/github-support".into(),
        name: "muxtrix".into(),
        owner_and_name: Some("Phoenixmatrix/muxtrix".into()),
        host: "github.com".into(),
        branch: "github-support".into(),
        wsl_distribution: "Ubuntu-24.04".into(),
    }
}

/// A loaded repository panel with local changes plus a large, independently
/// virtualized pull-request inventory.
fn staged_github_panel() -> GitHubPanelState {
    let names = [
        "crates/muxtrix-app/src/github.rs",
        "crates/muxtrix-app/src/main.rs",
        "crates/muxtrix-app/src/commands.rs",
        "crates/muxtrix-app/src/e2e.rs",
        "crates/muxtrix-app/assets/icons/github.svg",
        "crates/muxtrix-app/assets/icons/refresh.svg",
        "docs/ARCHITECTURE.md",
        "docs/TESTING.md",
        "Cargo.lock",
        "crates/muxtrix-app/Cargo.toml",
        "README.md",
        "DESIGN.md",
        "PRODUCT.md",
        "docs/DESIGN_SURFACE_APP_SHELL.md",
    ];
    let files = names
        .iter()
        .cycle()
        .take(74)
        .enumerate()
        .map(|(index, name)| github::FileChange {
            path: if index < names.len() {
                (*name).into()
            } else {
                format!("github/panel_row_{index:02}.rs")
            },
            status: if index != 0 && index % 11 == 0 {
                "Added".into()
            } else {
                "Modified".into()
            },
            additions: 3 + index * 2,
            deletions: index % 9,
            previous_path: None,
            patch: (index == 0).then(|| STAGED_GITHUB_PATCH.into()),
        })
        .collect::<Vec<_>>();
    let additions = files.iter().map(|file| file.additions).sum::<usize>();
    let deletions = files.iter().map(|file| file.deletions).sum::<usize>();
    let pull_requests = (0..120)
        .map(|index| github::PullRequestSummary {
            number: 391 - index as u64,
            title: if index == 0 {
                "Native GitHub review panel".into()
            } else if index == 1 {
                "Harden terminal mouse reporting over SSH".into()
            } else if index == 2 {
                "Keep worktree cleanup recoverable".into()
            } else {
                format!("Repository maintenance batch {}", 120 - index)
            },
            url: format!(
                "https://github.com/Phoenixmatrix/muxtrix/pull/{}",
                391 - index
            ),
            author: if index % 4 == 0 {
                "phoenixmatrix".into()
            } else {
                format!("contributor-{}", index % 11)
            },
            head: if index == 0 {
                "github-support".into()
            } else {
                format!("maintenance-{index}")
            },
            base: "main".into(),
            status: if index % 9 == 4 {
                github::PullRequestSummaryStatus::Draft
            } else {
                github::PullRequestSummaryStatus::Open
            },
        })
        .collect();

    GitHubPanelState {
        repository: staged_repository(),
        active_tab: GitHubPanelTab::Local,
        context_loading: false,
        data: Some(github::PanelData {
            branch: "github-support".into(),
            additions,
            deletions,
            files: files.clone(),
        }),
        loading: false,
        error: None,
        pull_requests: Some(pull_requests),
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
        request_generation: 0,
        pull_request_generation: 0,
        pull_request_detail_generation: 0,
        loading_phase: 0,
    }
}

fn staged_github_pull_request_panel() -> GitHubPanelState {
    let mut panel = staged_github_panel();
    panel.active_tab = GitHubPanelTab::PullRequests;
    panel.selected_pull_request_number = Some(391);
    let files = panel
        .data
        .as_ref()
        .map_or_else(Vec::new, |data| data.files.clone());
    panel.selected_pull_request = Some(staged_github_pull_request_details(files));
    panel
}

fn staged_github_pull_request_details(
    files: Vec<github::FileChange>,
) -> github::PullRequestDetails {
    let additions = files.iter().map(|file| file.additions).sum();
    let deletions = files.iter().map(|file| file.deletions).sum();
    github::PullRequestDetails {
        pull_request: github::PullRequest {
            number: 391,
            title: "Native GitHub review panel".into(),
            url: "https://github.com/Phoenixmatrix/muxtrix/pull/391".into(),
            author: "phoenixmatrix".into(),
            head: "github-support".into(),
            head_oid: "deadbeefcafebabe".into(),
            head_repository: "Phoenixmatrix/muxtrix".into(),
            base: "main".into(),
            base_oid: "feedfacecafebabe".into(),
            additions,
            deletions,
            changed_files: files.len(),
            draft: false,
            mergeable: "MERGEABLE".into(),
            merge_state: "CLEAN".into(),
            review_decision: "APPROVED".into(),
            checks: github::CheckSummary {
                passed: 7,
                pending: 0,
                failed: 0,
            },
        },
        files,
    }
}

fn light_horizontal_continuity(screenshot: &iced::window::Screenshot) -> (usize, usize) {
    let row_bytes = screenshot.size.width as usize * 4;
    let row_runs = screenshot
        .rgba
        .chunks_exact(row_bytes)
        .map(|row| {
            row.chunks_exact(4)
                .fold((0_usize, 0_usize), |(current, longest), pixel| {
                    let light =
                        pixel[3] == 255 && pixel[0] >= 128 && pixel[1] >= 128 && pixel[2] >= 128;
                    let current = if light { current + 1 } else { 0 };
                    (current, longest.max(current))
                })
                .1
        })
        .collect::<Vec<_>>();
    (
        row_runs.iter().copied().max().unwrap_or(0),
        row_runs
            .iter()
            .filter(|run| **run >= TERMINAL_BLOCK_CONTINUITY_PIXELS)
            .count(),
    )
}

/// Finds the bright-magenta rounded-border fixture and verifies that its top,
/// bottom, left, and right edges all belong to one 8-connected component.
fn magenta_rounded_box_continuity(screenshot: &iced::window::Screenshot) -> (bool, usize, usize) {
    colored_box_continuity(screenshot, |pixel| {
        pixel[3] == 255 && pixel[0] >= 128 && pixel[1] <= 96 && pixel[2] >= 128
    })
}

/// Finds the bright-cyan heavy-border fixture and verifies all four edges.
fn cyan_heavy_box_continuity(screenshot: &iced::window::Screenshot) -> (bool, usize, usize) {
    colored_box_continuity(screenshot, |pixel| {
        pixel[3] == 255 && pixel[0] <= 96 && pixel[1] >= 128 && pixel[2] >= 128
    })
}

fn colored_box_continuity(
    screenshot: &iced::window::Screenshot,
    is_border_pixel: impl Fn(&[u8]) -> bool,
) -> (bool, usize, usize) {
    let width = screenshot.size.width as usize;
    let height = screenshot.size.height as usize;
    let is_border = screenshot
        .rgba
        .chunks_exact(4)
        .map(is_border_pixel)
        .collect::<Vec<_>>();
    let mut visited = vec![false; is_border.len()];
    let mut largest = Vec::new();

    for start in 0..is_border.len() {
        if !is_border[start] || visited[start] {
            continue;
        }
        visited[start] = true;
        let mut pending = VecDeque::from([start]);
        let mut component = Vec::new();
        while let Some(index) = pending.pop_front() {
            component.push(index);
            let x = index % width;
            let y = index / width;
            for next_y in y.saturating_sub(1)..=(y + 1).min(height.saturating_sub(1)) {
                for next_x in x.saturating_sub(1)..=(x + 1).min(width.saturating_sub(1)) {
                    let next = next_y * width + next_x;
                    if is_border[next] && !visited[next] {
                        visited[next] = true;
                        pending.push_back(next);
                    }
                }
            }
        }
        if component.len() > largest.len() {
            largest = component;
        }
    }

    let Some(min_x) = largest.iter().map(|index| index % width).min() else {
        return (false, 0, 0);
    };
    let max_x = largest
        .iter()
        .map(|index| index % width)
        .max()
        .unwrap_or(min_x);
    let min_y = largest.iter().map(|index| index / width).min().unwrap_or(0);
    let max_y = largest
        .iter()
        .map(|index| index / width)
        .max()
        .unwrap_or(min_y);
    let component_width = max_x - min_x + 1;
    let component_height = max_y - min_y + 1;
    let middle_x = (min_x + max_x) / 2;
    let middle_y = (min_y + max_y) / 2;
    let near = |index: &usize, target_x: usize, target_y: usize| {
        (index % width).abs_diff(target_x) <= 4 && (index / width).abs_diff(target_y) <= 4
    };
    let connected = [
        (middle_x, min_y),
        (middle_x, max_y),
        (min_x, middle_y),
        (max_x, middle_y),
    ]
    .into_iter()
    .all(|(x, y)| largest.iter().any(|index| near(index, x, y)));

    (connected, component_width, component_height)
}

/// The first pane whose live grid does not match the pane it is drawn into,
/// described for the failure report.
fn pane_grid_mismatch(app: &Muxtrix) -> Option<String> {
    app.terminals.iter().find_map(|(pane_id, runtime)| {
        let Some(viewport) = runtime.viewport else {
            return Some(format!("pane {pane_id:?} was never measured by layout"));
        };
        let expected = super::pty_size_for_pane(viewport, &app.settings);
        let snapshot = runtime.snapshot.as_ref()?;
        if super::snapshot_matches_grid(snapshot, expected) {
            return None;
        }
        Some(format!(
            "pane {pane_id:?} renders a {}x{} grid inside a pane sized for {}x{}",
            snapshot.cells.first().map_or(0, |row| row.len()),
            snapshot.cells.len(),
            expected.cols,
            expected.rows,
        ))
    })
}

/// Where a string sits in a pane's visible grid, as (row, column).
fn pane_text_position(app: &Muxtrix, pane_id: PaneId, needle: &str) -> Option<(usize, usize)> {
    let snapshot = app.terminals.get(&pane_id)?.snapshot.as_ref()?;
    snapshot.rows.iter().enumerate().find_map(|(row, text)| {
        text.find(needle)
            .map(|byte| (row, text[..byte].chars().count()))
    })
}

fn pane_contains(app: &Muxtrix, pane_id: PaneId, needle: &str) -> bool {
    app.terminals
        .get(&pane_id)
        .and_then(|runtime| runtime.snapshot.as_ref())
        .is_some_and(|snapshot| snapshot.text().contains(needle))
}
