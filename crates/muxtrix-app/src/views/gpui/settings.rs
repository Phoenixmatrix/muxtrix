//! The settings screen.
//!
//! Every control edits the settings *draft*, never the saved settings, so the
//! page can be abandoned without effect until Save writes it through. That is
//! also what makes the Save button's enabled state meaningful: it compares the
//! two.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, px, svg,
};

use gpui_component::Sizable as _;
use gpui_component::input::{Input, InputState};
use gpui_component::select::Select;
use gpui_component::slider::{Slider, SliderState};
use gpui_component::switch::Switch;
use muxtrix_control::{Agent, HookAction, HookScope};

use crate::app::{
    FONT_FAMILY_MENU_MAX_HEIGHT, IconKind, InstalledVersionsState, Message,
    SETTINGS_NAV_LABEL_POINTS, SETTINGS_NAV_QUIET_PADDING_X, SETTINGS_NAV_RULE_GAP,
    SETTINGS_PAGE_PADDING_X, SettingsButtonKind, installed_version_restart_copy,
    settings_have_changes, settings_nav_is_crowded,
};
use crate::commands::CommandAction;
use crate::runtime::gpui::{Root, color};
use crate::settings::FleetScope;
use crate::theme::DesignTokens;
use crate::views::gpui::settings_widgets::Picker;
use crate::views::gpui::{icon_button, terminal_family};

impl Root {
    pub(crate) fn view_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        // The draft's appearance, not the saved one: flipping Light/Dark on
        // the page previews it on the page, as the iced screen does.
        let tokens = DesignTokens::for_appearance(app.settings_draft.appearance);
        if app.active_view == crate::app::ActiveView::ThemeGallery {
            return self.theme_gallery(tokens, cx);
        }
        let changed = settings_have_changes(&app.settings, &app.settings_draft)
            || app.settings_scrollback_lines_input
                != app.settings.terminal_scrollback_lines.to_string();
        let content = match app.settings_page {
            crate::app::SettingsPage::Preferences => self.preferences_page(changed, tokens, cx),
            crate::app::SettingsPage::Worktrees => self.worktree_manager(tokens, cx),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(color(tokens.app))
            .text_color(color(tokens.text))
            .child(self.settings_nav(changed, tokens, cx))
            .child(div().flex_grow(1.0).min_h(px(0.)).child(content))
            .into_any_element()
    }

    /// The settings top bar: the way back, the window's title while settings
    /// owns it, and the page switch. 52 px, on the rail surface, ruled below.
    fn settings_nav(
        &self,
        changed: bool,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let draft = &app.settings_draft;
        let full_terminal_label = if changed {
            "Discard changes and return"
        } else {
            "Back to terminal"
        };
        // Everything the bar holds is typeset from one size, so what it can
        // hold is a width measured in that size; a window that clears the
        // threshold at the default type size stops clearing it once the
        // interface type is scaled up, and the sentence gives way to the word.
        let crowded = settings_nav_is_crowded(app.window_size.width, draft);
        let terminal_label = match (crowded, changed) {
            (true, true) => "Discard changes",
            (true, false) => "Terminal",
            (false, _) => full_terminal_label,
        };
        let label_size = px(draft.ui_pixels(SETTINGS_NAV_LABEL_POINTS));
        // Returning to the terminal is navigation, not one of the page's
        // actions, so it wears the quiet role.
        let back = div()
            .id("settings-back")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.))
            .h(px(30.))
            .px(px(SETTINGS_NAV_QUIET_PADDING_X))
            .rounded(px(5.))
            .border_1()
            .border_color(color(iced::Color::TRANSPARENT))
            .cursor_pointer()
            .text_size(label_size)
            .text_color(color(tokens.muted))
            .whitespace_nowrap()
            .hover(move |style| {
                style
                    .bg(color(tokens.panel_raised))
                    .border_color(color(tokens.line_strong))
                    .text_color(color(tokens.text))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::CancelSettings, window, cx);
                }),
            )
            .child(
                svg()
                    .path(crate::assets::icon_path(IconKind::Back))
                    .size(px(12.))
                    .text_color(color(tokens.muted)),
            )
            .child(terminal_label);
        // The rule owns the gap on both of its sides: the button carries its
        // own trailing padding and the title carries none.
        let nav_rule = div()
            .pl(px(SETTINGS_NAV_RULE_GAP - SETTINGS_NAV_QUIET_PADDING_X))
            .pr(px(SETTINGS_NAV_RULE_GAP))
            .child(div().w(px(1.)).h(px(16.)).bg(color(tokens.line_strong)));

        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(52.))
            // The left inset is short by the back button's own padding so its
            // glyph, not its hit area, lands on the page's content margin.
            .pl(px(SETTINGS_PAGE_PADDING_X - SETTINGS_NAV_QUIET_PADDING_X))
            .pr(px(SETTINGS_PAGE_PADDING_X))
            .bg(color(tokens.rail))
            .border_b(px(1.))
            .border_color(color(tokens.line))
            .child(back)
            .child(nav_rule)
            .child(
                div()
                    .text_size(label_size)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(color(tokens.text))
                    .whitespace_nowrap()
                    .child("Settings"),
            )
            .child(div().flex_grow(1.0))
            .child(self.settings_page_toggle(tokens, cx))
            .into_any_element()
    }

    /// The Preferences/Worktrees page switcher, on the same recessed well the
    /// fleet heading uses.
    fn settings_page_toggle(&self, tokens: DesignTokens, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let current = app.settings_page;
        let mut well = div()
            .flex()
            .flex_row()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(7.))
            .bg(color(tokens.app))
            .border_1()
            .border_color(color(tokens.line));
        for (label, page) in [
            ("Preferences", crate::app::SettingsPage::Preferences),
            ("Worktrees", crate::app::SettingsPage::Worktrees),
        ] {
            let selected = current == page;
            let mut hover = color(tokens.text);
            hover.a = 0.05;
            let mut segment = div()
                .id(SharedString::from(format!("settings-page-{label}")))
                .h(px(26.))
                .px(px(13.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .border_1()
                .cursor_pointer()
                .text_size(px(app.settings_draft.ui_pixels(9.5)))
                .whitespace_nowrap()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::OpenSettingsPage(page), window, cx);
                    }),
                )
                .child(label);
            segment = if selected {
                segment
                    .bg(color(tokens.panel_raised))
                    .border_color(color(tokens.line_strong))
                    .text_color(color(tokens.text))
                    .shadow(vec![gpui::BoxShadow {
                        color: gpui::Rgba {
                            r: 0.,
                            g: 0.,
                            b: 0.,
                            a: 0.35,
                        }
                        .into(),
                        offset: gpui::point(px(0.), px(1.)),
                        blur_radius: px(2.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
            } else {
                segment
                    .border_color(color(iced::Color::TRANSPARENT))
                    .text_color(color(tokens.muted))
                    .hover(move |style| style.bg(hover))
            };
            well = well.child(segment);
        }
        well.into_any_element()
    }

    /// The preferences page: appearance, fonts, terminal, agents, integrations.
    fn preferences_page(
        &self,
        changed: bool,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let draft = &app.settings_draft;
        let can_continue_pending_command = app.pending_default_agent_command.is_some()
            && draft
                .default_agent
                .is_some_and(|agent| app.agent_is_configured_for(agent, draft));
        let ui = |points: f32| px(draft.ui_pixels(points));

        let title = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .text_size(ui(22.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("Preferences"),
            )
            .child(
                div()
                    .text_size(ui(11.0))
                    .text_color(color(tokens.muted))
                    .child("Tune the interface, terminal, and developer integrations."),
            );

        let widgets = &self.settings_widgets;
        let picker = |state: &Picker, width: f32| {
            div()
                .w(px(width))
                .child(
                    Select::new(&state.state)
                        .small()
                        .menu_max_h(px(FONT_FAMILY_MENU_MAX_HEIGHT)),
                )
                .into_any_element()
        };
        let slider_row = |state: &gpui::Entity<SliderState>, readout: String| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .child(div().w(px(220.)).child(Slider::new(state)))
                .child(
                    div()
                        .w(px(52.))
                        .text_size(ui(10.0))
                        .text_color(color(tokens.muted))
                        .child(readout),
                )
                .into_any_element()
        };
        let switch = |id: &'static str, on: bool, message: fn(bool) -> Message| {
            Switch::new(id)
                .checked(on)
                .on_click(cx.listener(move |root, checked: &bool, window, cx| {
                    root.dispatch(message(*checked), window, cx);
                }))
                .into_any_element()
        };
        let field = |state: &gpui::Entity<InputState>, width: f32| {
            div()
                .w(px(width))
                .h(px(30.))
                .child(Input::new(state).small())
                .into_any_element()
        };

        let interface = self.settings_section(
            "Appearance",
            "Theme and interface chrome",
            vec![
                self.settings_row(
                    "Theme",
                    "Color scheme for the application",
                    picker(&widgets.appearance, 220.),
                    tokens,
                ),
                self.settings_row(
                    "Interface font",
                    "Installed font used by application chrome",
                    picker(&widgets.ui_font, 280.),
                    tokens,
                ),
                self.settings_row(
                    "Interface font weight",
                    "Weights installed for the selected family",
                    picker(&widgets.ui_font_weight, 220.),
                    tokens,
                ),
                self.settings_row(
                    "Interface text size",
                    "Scales labels, controls, and workspace chrome",
                    slider_row(
                        &widgets.ui_font_size,
                        format!("{:.0} pt", draft.ui_font_size),
                    ),
                    tokens,
                ),
                self.settings_row(
                    "Workspace status bar",
                    "Show process messages and pane count at the bottom",
                    switch(
                        "status-bar",
                        draft.show_status_bar,
                        Message::SettingsShowStatusBar,
                    ),
                    tokens,
                ),
                self.settings_row(
                    "Show all workspaces in Fleet",
                    "Include panes from every workspace; when off, show only the current workspace",
                    switch(
                        "fleet-scope",
                        draft.fleet_scope == FleetScope::AllWorkspaces,
                        Message::SettingsShowAllWorkspaces,
                    ),
                    tokens,
                ),
            ],
            tokens,
        );

        let terminal_appearance = self.settings_section(
            "Terminal appearance",
            "Ghostty-compatible color presets and ANSI palette",
            vec![
                self.settings_row(
                    "Color theme",
                    "Sets terminal defaults while applications keep explicit colors",
                    picker(&widgets.terminal_theme, 280.),
                    tokens,
                ),
                self.settings_row(
                    "Theme gallery",
                    "Browse every preset with live terminal previews",
                    self.settings_action_button(
                        "browse-gallery",
                        "Browse gallery",
                        Message::OpenThemeGallery,
                        SettingsButtonKind::Secondary,
                        tokens,
                        cx,
                    ),
                    tokens,
                ),
                div()
                    .p(px(14.))
                    .child(theme_preview_card(
                        draft.terminal_theme.preset(),
                        draft,
                        true,
                    ))
                    .into_any_element(),
            ],
            tokens,
        );

        let scrollback_validation =
            crate::settings::parse_terminal_scrollback_lines(&app.settings_scrollback_lines_input);
        let scrollback_valid = scrollback_validation.is_ok();
        let mut scrollback_limit = div().flex().flex_col().gap(px(4.)).child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(field(&widgets.scrollback, 180.))
                .child(
                    div()
                        .text_size(ui(10.0))
                        .text_color(color(tokens.muted))
                        .child("lines"),
                ),
        );
        if let Err(error) = scrollback_validation {
            scrollback_limit = scrollback_limit.child(
                div()
                    .w(px(260.))
                    .text_size(ui(9.0))
                    .text_color(color(tokens.danger))
                    .child(error),
            );
        }
        let typography_preview = div()
            .w_full()
            .py(px(10.))
            .px(px(12.))
            .rounded(px(5.))
            .bg(color(tokens.app))
            .border_1()
            .border_color(color(tokens.line))
            .font_family(terminal_family(draft))
            .font_weight(gpui::FontWeight(f32::from(
                crate::settings::weight_numeric(draft.terminal_font_weight.iced()),
            )))
            .text_size(px(draft.terminal_font_pixels()))
            .line_height(px(draft.terminal_cell_height()))
            .child("$ cargo test --workspace\n✓ all checks passed");
        let terminal = self.settings_section(
            "Terminal text and history",
            "Fonts, grid metrics, and scrollback history",
            vec![
                self.settings_row(
                    "Font family",
                    "Only installed monospaced families are listed",
                    picker(&widgets.terminal_font, 340.),
                    tokens,
                ),
                self.settings_row(
                    "Font weight",
                    "Weights installed for the selected family",
                    picker(&widgets.terminal_font_weight, 220.),
                    tokens,
                ),
                self.settings_row(
                    "Font size",
                    "Point size used for terminal glyphs",
                    slider_row(
                        &widgets.terminal_font_size,
                        format!("{:.0} pt", draft.terminal_font_size),
                    ),
                    tokens,
                ),
                self.settings_row(
                    "Line height",
                    "Vertical spacing between terminal rows",
                    slider_row(
                        &widgets.terminal_line_height,
                        format!("{:.2}", draft.terminal_line_height),
                    ),
                    tokens,
                ),
                self.settings_row(
                    "Scrollback history",
                    "Lines kept by new and restarted panes (1,000–100,000)",
                    scrollback_limit.into_any_element(),
                    tokens,
                ),
                self.settings_row(
                    "Preview",
                    "Updates before changes are applied",
                    typography_preview.into_any_element(),
                    tokens,
                ),
            ],
            tokens,
        );

        let github_host_validation = crate::settings::normalize_github_host(&draft.github_host);
        let github_host_valid = github_host_validation.is_ok();
        let mut github_host_control = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(field(&widgets.github_host, 320.));
        if let Err(error) = github_host_validation {
            github_host_control = github_host_control.child(
                div()
                    .w(px(320.))
                    .text_size(ui(9.0))
                    .text_color(color(tokens.danger))
                    .child(error),
            );
        }
        let github = self.settings_section(
            "GitHub",
            "Public GitHub and Enterprise Server",
            vec![self.settings_row(
                "GitHub host",
                "Use github.com or your Enterprise Server hostname; no API path",
                github_host_control.into_any_element(),
                tokens,
            )],
            tokens,
        );

        let integrations = self.settings_section(
            "Agent lifecycle hooks",
            "Reversible Codex, Claude Code, and Oh My Pi integration",
            vec![
                self.settings_row("Default worktree agent", "Used when a worktree command opens or restarts a pane with an agent", picker(&widgets.default_agent, 220.), tokens),
                self.agent_hook_row(Agent::Codex, tokens, cx),
                self.agent_hook_row(Agent::Claude, tokens, cx),
                self.agent_hook_row(Agent::Pi, tokens, cx),
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(18.))
                    .p(px(14.))
                    .child(
                        div()
                            .flex_grow(1.0)
                            .text_size(ui(10.0))
                            .text_color(color(tokens.muted))
                            .child("Hook changes apply immediately. Muxtrix updates only its tagged entries. Project hooks remain available in muxtrixctl."),
                    )
                    .child(self.settings_action_button("refresh-hooks", "Refresh", Message::RefreshHookStatus, SettingsButtonKind::Secondary, tokens, cx))
                    .into_any_element(),
            ],
            tokens,
        );
        let versions = self.version_section(tokens);

        let content = div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .w_full()
            .max_w(px(860.))
            .child(title)
            .child(interface)
            .child(terminal_appearance)
            .child(terminal)
            .child(github)
            .child(integrations)
            .child(versions);

        let font_restart = if draft.ui_font != app.settings.ui_font
            || draft.ui_font_weight != app.settings.ui_font_weight
        {
            "Interface typography changes after restarting Muxtrix. "
        } else {
            ""
        };
        let can_apply =
            (changed || can_continue_pending_command) && github_host_valid && scrollback_valid;
        let apply_label = if can_continue_pending_command {
            "Apply and continue"
        } else {
            "Apply changes"
        };
        let mut apply = self.settings_action_button(
            "apply-settings",
            apply_label,
            Message::SaveSettings,
            SettingsButtonKind::Primary,
            tokens,
            cx,
        );
        if !can_apply {
            // An un-pressable button must not impersonate a live one.
            apply = div()
                .h(px(30.))
                .px(px(11.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .bg(color(tokens.panel))
                .border_1()
                .border_color(color(tokens.line))
                .text_size(ui(9.0))
                .text_color(color(tokens.faint))
                .whitespace_nowrap()
                .child(apply_label)
                .into_any_element();
        }
        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .h(px(58.))
            .py(px(10.))
            .px(px(SETTINGS_PAGE_PADDING_X))
            .bg(color(tokens.rail))
            .border_t(px(1.))
            .border_color(color(tokens.line))
            .child(
                div()
                    .flex_grow(1.0)
                    .text_size(ui(9.0))
                    .text_color(color(tokens.faint))
                    .child(format!("{font_restart}Preferences apply when saved; shell and scrollback affect new and restarted panes; hook actions apply immediately")),
            )
            .child(self.settings_action_button("cancel-settings", "Cancel", Message::CancelSettings, SettingsButtonKind::Secondary, tokens, cx))
            .child(apply);

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .id("settings-scroll")
                    .flex_grow(1.0)
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scrolls.settings)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_center()
                            .w_full()
                            .py(px(24.))
                            .px(px(SETTINGS_PAGE_PADDING_X))
                            .child(content),
                    ),
            )
            .child(footer)
            .into_any_element()
    }

    fn version_section(&self, tokens: DesignTokens) -> AnyElement {
        let app = self.app();
        let draft = &app.settings_draft;
        let ui = |points: f32| px(draft.ui_pixels(points));
        let fallback = match &app.installed_versions {
            InstalledVersionsState::Unchecked | InstalledVersionsState::Ready(_) => "Running",
            InstalledVersionsState::Checking => "Running · checking installed build…",
            InstalledVersionsState::Unavailable => "Running · installed check unavailable",
        };
        let (installed_muxtrix, installed_muxtrixctl) = match &app.installed_versions {
            InstalledVersionsState::Ready(versions) => {
                (Some(&versions.muxtrix), Some(&versions.muxtrixctl))
            }
            _ => (None, None),
        };
        let value = |running: &'static str, installed: Option<&Result<String, String>>| {
            let (detail, detail_color) = match installed {
                Some(Ok(version)) if version == running => {
                    ("Running · matches installed".to_owned(), tokens.muted)
                }
                Some(Ok(version)) => (format!("Running · v{version} installed"), tokens.warning),
                Some(Err(_)) => (
                    "Running · installed binary unavailable".to_owned(),
                    tokens.faint,
                ),
                None => (fallback.to_owned(), tokens.muted),
            };
            div()
                .flex()
                .flex_col()
                .items_end()
                .gap(px(2.))
                .child(
                    div()
                        .font_family(terminal_family(draft))
                        .text_size(ui(10.0))
                        .text_color(color(tokens.text))
                        .child(format!("v{running}")),
                )
                .child(
                    div()
                        .text_size(ui(8.5))
                        .text_color(color(detail_color))
                        .child(detail),
                )
                .into_any_element()
        };
        let mut rows = Vec::new();
        if let Some(copy) = installed_version_restart_copy(&app.installed_versions) {
            let mut fill = color(tokens.warning);
            fill.a = 0.07;
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .w_full()
                    .py(px(11.))
                    .px(px(14.))
                    .bg(fill)
                    .child(div().size(px(8.)).rounded_full().bg(color(tokens.warning)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(ui(10.5))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(color(tokens.text))
                                    .child("Restart to use the installed build"),
                            )
                            .child(
                                div()
                                    .text_size(ui(9.0))
                                    .text_color(color(tokens.warning))
                                    .child(copy),
                            ),
                    )
                    .into_any_element(),
            );
        }
        rows.push(self.settings_row(
            "Muxtrix",
            "Desktop application",
            value(env!("CARGO_PKG_VERSION"), installed_muxtrix),
            tokens,
        ));
        rows.push(self.settings_row(
            "Muxtrix Control",
            "Local control service and muxtrixctl command",
            value(muxtrix_control::VERSION, installed_muxtrixctl),
            tokens,
        ));
        self.settings_section(
            "Versions",
            "Builds active in this window and installed on disk",
            rows,
            tokens,
        )
    }

    fn agent_hook_row(
        &self,
        agent: Agent,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let draft = &app.settings_draft;
        let ui = |points: f32| px(draft.ui_pixels(points));
        let status = app
            .hook_statuses
            .iter()
            .find(|status| status.agent == agent && status.scope == HookScope::User);
        let installed = status.is_some_and(|status| status.installed);
        let repair_needed =
            status.is_some_and(|status| !status.installed && status.managed_entries > 0);
        let detail = status.map_or_else(
            || {
                if app.integration_refreshing {
                    "Checking…".to_owned()
                } else {
                    "Not installed".to_owned()
                }
            },
            |status| {
                if installed {
                    format!("Installed · {} managed entries", status.managed_entries)
                } else if status.unreachable_entries > 0 {
                    format!(
                        "Needs repair · {} hooks call a muxtrixctl that is missing",
                        status.unreachable_entries
                    )
                } else if repair_needed {
                    format!(
                        "Needs repair · {} hooks target another Muxtrix binary",
                        status.managed_entries
                    )
                } else {
                    "Not installed".to_owned()
                }
            },
        );
        let command = match agent {
            Agent::Codex => &self.settings_widgets.codex_command,
            Agent::Claude => &self.settings_widgets.claude_command,
            Agent::Pi => &self.settings_widgets.pi_command,
        };
        // Which actions the row offers, then the buttons for them — built in
        // two steps because each button borrows the context.
        let remove = (
            "remove-hooks",
            "Remove hooks",
            Message::ManageHooks(agent, HookAction::Remove),
            SettingsButtonKind::Danger,
        );
        let offered: Vec<(&'static str, &'static str, Message, SettingsButtonKind)> =
            if app.integration_refreshing {
                Vec::new()
            } else if installed {
                vec![
                    (
                        "launch-agent",
                        "Launch",
                        Message::RunCommand(CommandAction::LaunchAgent(agent)),
                        SettingsButtonKind::Secondary,
                    ),
                    remove,
                ]
            } else if repair_needed {
                vec![
                    (
                        "repair-hooks",
                        "Repair hooks",
                        Message::ManageHooks(agent, HookAction::ReAdd),
                        SettingsButtonKind::Secondary,
                    ),
                    remove,
                ]
            } else {
                vec![(
                    "add-hooks",
                    "Add integration",
                    Message::ManageHooks(agent, HookAction::Add),
                    SettingsButtonKind::Secondary,
                )]
            };
        let mut actions = div().flex().flex_row().items_center().gap(px(8.));
        if app.integration_refreshing {
            actions = actions.child(
                div()
                    .text_size(ui(9.0))
                    .text_color(color(tokens.muted))
                    .child("Updating…"),
            );
        }
        for (id, label, message, kind) in offered {
            actions =
                actions.child(self.settings_action_button(id, label, message, kind, tokens, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(14.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_grow(1.0)
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(ui(13.0))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(match agent {
                                        Agent::Codex => "Codex",
                                        Agent::Claude => "Claude Code",
                                        Agent::Pi => "Oh My Pi",
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(ui(10.0))
                                    .text_color(color(if installed {
                                        tokens.success
                                    } else if repair_needed {
                                        tokens.warning
                                    } else {
                                        tokens.muted
                                    }))
                                    .child(detail),
                            ),
                    )
                    .child(actions),
            )
            .child(div().h(px(30.)).child(Input::new(command).small()))
            .into_any_element()
    }

    /// A section on the settings page: heading and description above a
    /// bordered panel, rows inside ruled apart.
    fn settings_section(
        &self,
        title: &'static str,
        description: &'static str,
        rows: Vec<AnyElement>,
        tokens: DesignTokens,
    ) -> AnyElement {
        let draft = &self.app().settings_draft;
        let mut panel = div()
            .flex()
            .flex_col()
            .w_full()
            .rounded(px(10.))
            .bg(color(tokens.panel))
            .border_1()
            .border_color(color(tokens.line));
        let count = rows.len();
        for (index, row) in rows.into_iter().enumerate() {
            panel = panel.child(row);
            if index + 1 < count {
                panel = panel.child(div().h(px(1.)).w_full().bg(color(tokens.line)));
            }
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(draft.ui_pixels(13.0)))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(color(tokens.text))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(draft.ui_pixels(9.0)))
                            .text_color(color(tokens.muted))
                            .child(description),
                    ),
            )
            .child(panel)
            .into_any_element()
    }

    /// One row: label and description at left, the control at right.
    fn settings_row(
        &self,
        label: &'static str,
        description: &'static str,
        control: AnyElement,
        tokens: DesignTokens,
    ) -> AnyElement {
        let draft = &self.app().settings_draft;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(18.))
            .w_full()
            .py(px(12.))
            .px(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_grow(1.0)
                    .min_w(px(0.))
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(draft.ui_pixels(11.0)))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(color(tokens.text))
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(draft.ui_pixels(9.0)))
                            .text_color(color(tokens.muted))
                            .child(description),
                    ),
            )
            .child(div().flex_shrink_0().child(control))
            .into_any_element()
    }

    /// Full-screen theme browser: every preset rendered as the same live
    /// terminal preview the settings page shows for the current theme —
    /// sample output, selection, cursor, and the full ANSI strip — two per
    /// row, clickable, Esc or Back to return.
    fn theme_gallery(&self, tokens: DesignTokens, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let draft = &app.settings_draft;
        let chosen = draft.terminal_theme;
        let count_nudge = (draft.ui_pixels(18.0) - draft.ui_pixels(9.5)) * 0.6;
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.))
            .child(self.settings_action_button(
                "gallery-back",
                "← Settings",
                Message::CloseThemeGallery,
                SettingsButtonKind::Secondary,
                tokens,
                cx,
            ))
            .child(
                div()
                    .text_size(px(draft.ui_pixels(18.0)))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(color(tokens.text))
                    .child("Theme gallery"),
            )
            .child(div().flex_grow(1.0))
            .child(
                div()
                    .pt(px(count_nudge))
                    .text_size(px(draft.ui_pixels(9.5)))
                    .text_color(color(tokens.faint))
                    .child(format!(
                        "Current: {} · {} themes",
                        chosen.preset().name,
                        crate::themes::TerminalThemeId::ALL.len()
                    )),
            );

        // One column keeps the preview truthful when cards would squeeze
        // below the width the sample line and ANSI strip need.
        let columns: usize = if app.window_size.width < 980.0 { 1 } else { 2 };
        let mut grid = div().flex().flex_col().gap(px(12.));
        let mut cards: Vec<AnyElement> = Vec::new();
        let flush = |grid: gpui::Div, cards: &mut Vec<AnyElement>| -> gpui::Div {
            if cards.is_empty() {
                return grid;
            }
            while cards.len() < columns {
                cards.push(div().flex_1().into_any_element());
            }
            grid.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(12.))
                    .children(std::mem::take(cards)),
            )
        };
        let mut current_group: Option<bool> = None;
        // Dark first, then light: grouped so the mode chip on every card
        // becomes redundant and a named theme is findable by section.
        let ordered = crate::themes::TerminalThemeId::ALL
            .into_iter()
            .filter(|id| !id.preset().is_light)
            .chain(
                crate::themes::TerminalThemeId::ALL
                    .into_iter()
                    .filter(|id| id.preset().is_light),
            );
        for id in ordered {
            let preset = id.preset();
            if current_group != Some(preset.is_light) {
                grid = flush(grid, &mut cards);
                current_group = Some(preset.is_light);
                grid = grid.child(
                    div()
                        .pt(px(8.))
                        .pl(px(3.))
                        .text_size(px(draft.ui_pixels(8.5)))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(color(tokens.faint))
                        .child(if preset.is_light { "LIGHT" } else { "DARK" }),
                );
            }
            let selected = id == chosen;
            cards.push(
                div()
                    .id(SharedString::from(preset.name))
                    .flex_1()
                    .min_w(px(0.))
                    .p(px(3.))
                    .rounded(px(12.))
                    .border_2()
                    .border_color(color(if selected {
                        tokens.accent
                    } else {
                        iced::Color::TRANSPARENT
                    }))
                    .when(!selected, |card| {
                        card.hover(move |style| style.border_color(color(tokens.line_strong)))
                    })
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::GalleryThemeChosen(id), window, cx);
                        }),
                    )
                    .child(theme_preview_card(preset, draft, false))
                    .into_any_element(),
            );
            if cards.len() == columns {
                grid = flush(grid, &mut cards);
            }
        }
        grid = flush(grid, &mut cards);

        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .size_full()
            .p(px(28.))
            .bg(color(tokens.app))
            .child(header)
            .child(
                div()
                    .id("gallery-scroll")
                    .flex_grow(1.0)
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scrolls.settings)
                    .child(grid.pt(px(4.)).pr(px(14.)).pb(px(24.))),
            )
            .into_any_element()
    }

    /// A settings-page action: the same three kinds the iced page draws.
    fn settings_action_button(
        &self,
        id: &'static str,
        label: &'static str,
        message: Message,
        kind: SettingsButtonKind,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let (fill, hover, text, edge, hover_edge) = match kind {
            SettingsButtonKind::Primary => {
                let mut hover = color(tokens.accent);
                hover.a = 0.86;
                (
                    color(tokens.accent),
                    hover,
                    tokens.app,
                    tokens.accent,
                    tokens.accent,
                )
            }
            SettingsButtonKind::Secondary => (
                color(tokens.panel),
                color(tokens.panel_raised),
                tokens.text,
                tokens.line_strong,
                tokens.line_strong,
            ),
            SettingsButtonKind::Danger => {
                let mut fill = color(tokens.danger);
                fill.a = 0.05;
                let mut hover = color(tokens.danger);
                hover.a = 0.12;
                // tokens.line rendered these borders invisible.
                let mut edge = tokens.danger;
                edge.a = 0.45;
                (fill, hover, tokens.danger, edge, tokens.danger)
            }
            SettingsButtonKind::Quiet => (
                color(iced::Color::TRANSPARENT),
                color(tokens.panel_raised),
                tokens.muted,
                iced::Color::TRANSPARENT,
                tokens.line_strong,
            ),
        };
        div()
            .id(id)
            .h(px(30.))
            .px(px(11.))
            .flex()
            .items_center()
            .rounded(px(5.))
            .cursor_pointer()
            .bg(fill)
            .border_1()
            .border_color(color(edge))
            .text_size(px(app.settings_draft.ui_pixels(9.0)))
            .text_color(color(text))
            .whitespace_nowrap()
            .hover(move |style| style.bg(hover).border_color(color(hover_edge)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(message.clone(), window, cx);
                }),
            )
            .child(label)
            .into_any_element()
    }

    /// Every worktree this repository has, with what is holding each one.
    fn worktree_manager(&self, tokens: DesignTokens, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let mut rows = div().flex().flex_col().gap(px(2.)).p(px(20.));
        let manager = app.worktree_manager.as_ref();
        if let Some(failure) = manager.and_then(|manager| manager.failure.as_deref()) {
            rows = rows.child(
                div()
                    .text_size(px(app.settings.ui_pixels(10.0)))
                    .text_color(color(tokens.muted))
                    .child(failure.to_owned()),
            );
        }
        for (index, entry) in manager
            .map(|manager| manager.entries.as_slice())
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            // A worktree in use, or the primary one, cannot be removed; the
            // row says which rather than offering an action that would fail.
            let blocker = entry.deletion_blocker.clone().or_else(|| {
                entry
                    .used_by
                    .clone()
                    .map(|title| format!("in use by {title}"))
            });
            rows = rows.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .h(px(44.))
                    .px(px(12.))
                    .rounded(px(6.))
                    .bg(color(tokens.panel))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_grow(1.0)
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(10.0)))
                                    .text_color(color(tokens.text))
                                    .truncate()
                                    .child(entry.path.file_name().map_or_else(
                                        || entry.path.display().to_string(),
                                        |name| name.to_string_lossy().into_owned(),
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(8.0)))
                                    .text_color(color(tokens.faint))
                                    .truncate()
                                    .child(
                                        entry.branch.clone().unwrap_or_else(|| "detached".into()),
                                    ),
                            ),
                    )
                    .children((entry.unpushed_commits > 0).then(|| {
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.warning))
                            .child(format!("{} unpushed", entry.unpushed_commits))
                    }))
                    .child(match blocker {
                        Some(reason) => div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.faint))
                            .child(reason)
                            .into_any_element(),
                        None => div()
                            .id(("worktree-delete", index as u64))
                            .h(px(24.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .bg(color(tokens.panel_raised))
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.danger))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                    root.dispatch(
                                        Message::WorktreeManagerDelete(index),
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .child("Delete")
                            .into_any_element(),
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(color(tokens.app))
            .child(self.screen_bar("Worktrees", Message::CloseWorktreeManager, tokens, cx))
            .child(
                div()
                    .id("worktrees-scroll")
                    .flex_grow(1.0)
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scrolls.settings)
                    .child(rows),
            )
            .into_any_element()
    }

    /// A screen's top bar: a title and the way back.
    fn screen_bar(
        &self,
        title: &str,
        back: Message,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .h(px(43.))
            .px(px(10.))
            .bg(color(tokens.rail))
            .border_b(px(1.))
            .border_color(color(tokens.line))
            .child(
                icon_button(
                    gpui::ElementId::from("screen-back"),
                    IconKind::Back,
                    tokens,
                    false,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(back.clone(), window, cx);
                    }),
                ),
            )
            .child(
                div()
                    .text_size(px(self.app().settings.ui_pixels(13.0)))
                    .text_color(color(tokens.text))
                    .child(title.to_owned()),
            )
            .into_any_element()
    }
}

/// The live terminal preview for a theme: sample output, a selection, the
/// cursor, and the full ANSI strip, on the theme's own background. Built from
/// the same colours the real grid uses, so what the card shows is what the
/// terminal will render.
pub(crate) fn theme_preview_card(
    preset: crate::themes::TerminalThemePreset,
    settings: &crate::settings::AppSettings,
    caption: bool,
) -> AnyElement {
    use crate::terminal::runs::rgb;
    let tokens = DesignTokens::for_appearance(settings.appearance);
    let font = crate::terminal::element::terminal_font(settings);
    let preview_size = settings.terminal_font_pixels().clamp(13.0, 18.0);
    let mode = if preset.is_light { "Light" } else { "Dark" };

    // The sample line, as one styled run per coloured span.
    let spans: [(&str, muxtrix_terminal::Rgb, Option<muxtrix_terminal::Rgb>); 10] = [
        ("❯ ", preset.ansi[10], None),
        ("cargo test ", preset.foreground, None),
        ("--workspace\n", preset.ansi[12], None),
        ("   Compiling ", preset.ansi[3], None),
        ("muxtrix\n", preset.foreground, None),
        ("   Finished ", preset.ansi[2], None),
        ("95 tests passed  ", preset.foreground, None),
        (
            " selected ",
            preset.selection_foreground,
            Some(preset.selection_background),
        ),
        ("  ", preset.foreground, None),
        (" C ", preset.cursor_text, Some(preset.cursor)),
    ];
    let mut text = String::new();
    let mut runs = Vec::with_capacity(spans.len());
    for (piece, foreground, background) in spans {
        text.push_str(piece);
        runs.push(gpui::TextRun {
            len: piece.len(),
            font: font.clone(),
            color: color(rgb(foreground)).into(),
            background_color: background.map(|hue| color(rgb(hue)).into()),
            underline: None,
            strikethrough: None,
        });
    }
    let sample = div()
        .text_size(px(preview_size))
        .line_height(px(preview_size * 1.35))
        .child(gpui::StyledText::new(text).with_runs(runs));

    let strip = |colors: &[muxtrix_terminal::Rgb]| {
        let mut row = div().flex().flex_row().gap(px(5.));
        for hue in colors {
            // The iced strip asks for a 24 px cap on a fill-width swatch and
            // gets the fill: the cap never applies. The rendered width is
            // what the gallery is compared against, so match that.
            row = row.child(
                div()
                    .flex_1()
                    .h(px(12.))
                    .rounded(px(2.))
                    .bg(color(rgb(*hue))),
            );
        }
        row
    };

    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .w_full()
        .py(px(14.))
        .px(px(16.))
        .rounded(px(6.))
        .bg(color(rgb(preset.background)))
        .border_1()
        .border_color(color(tokens.line_strong))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_grow(1.0)
                        .min_w(px(0.))
                        .text_size(px(settings.ui_pixels(11.0)))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(color(rgb(preset.foreground)))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(preset.name),
                )
                .when(caption, |head| {
                    // In the gallery, section headings already say the mode;
                    // repeating a pill on every card is noise.
                    head.child(
                        div()
                            .py(px(2.))
                            .px(px(7.))
                            .rounded(px(4.))
                            .bg(color(rgb(preset.foreground)))
                            .text_size(px(settings.ui_pixels(8.0)))
                            .text_color(color(rgb(preset.background)))
                            .child(mode),
                    )
                }),
        )
        .child(sample)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(strip(&preset.ansi[..8]))
                .child(strip(&preset.ansi[8..])),
        );
    if caption {
        card = card.child(
            div()
                .text_size(px(settings.ui_pixels(8.0)))
                .text_color(color(rgb(preset.ansi[8])))
                .child(
                    "Theme colors set defaults · direct RGB and application OSC colors stay intact",
                ),
        );
    }
    card.into_any_element()
}
