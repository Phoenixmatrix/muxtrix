//! Settings: the page shell shared by every settings surface.
//!
//! The two pages (preferences and worktrees) live in submodules; this module
//! owns the frame around them — page toggle, dirty tracking, save/cancel — and
//! the version section.

pub(crate) mod preferences;
pub(crate) mod theme_gallery;
pub(crate) mod worktrees;

use iced::widget::column;

use crate::views::prelude::*;

use crate::commands::CommandAction;
use crate::{
    InstalledVersionsState, SETTINGS_NAV_LABEL_POINTS, SETTINGS_NAV_QUIET_PADDING_X,
    SETTINGS_NAV_RULE_GAP, SETTINGS_PAGE_PADDING_X, SettingsButtonKind, SettingsPage, app_tooltip,
    centered_button_content, installed_version_restart_copy, ruled_surface, settings_action_button,
    settings_button_style, settings_divider, settings_have_changes, settings_hook_button,
    settings_nav_is_crowded, settings_page_toggle, settings_row, settings_section,
    settings_version_value, signal_dot,
};
use iced::Font;
use muxtrix_control::{Agent, HookAction, HookScope};

impl Muxtrix {
    pub(crate) fn settings_view(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings_draft.appearance);
        let changed = settings_have_changes(&self.settings, &self.settings_draft)
            || self.settings_scrollback_lines_input
                != self.settings.terminal_scrollback_lines.to_string();
        let full_terminal_label = if changed {
            "Discard changes and return"
        } else {
            "Back to terminal"
        };
        // Everything the bar holds — this label, the title, the switch — is
        // typeset from one size, so what the bar can hold is a width measured
        // in that size, not in pixels. A window that clears the threshold at
        // the default type size stops clearing it once the interface type is
        // scaled up, which is exactly when the sentence has to give way to the
        // word; the arrow already carries the direction, and the tooltip keeps
        // the sentence one hover away.
        let crowded = settings_nav_is_crowded(self.window_size.width, &self.settings_draft);
        let terminal_label = match (crowded, changed) {
            (true, true) => "Discard changes",
            (true, false) => "Terminal",
            (false, _) => full_terminal_label,
        };
        // Returning to the terminal is navigation, not one of the page's
        // actions, so it wears the quiet role: no fill competing with Refresh,
        // Apply, or Remove, and a real surface only under the pointer.
        let back = button(centered_button_content(
            row![
                icon(IconKind::Back, tokens.muted, 12.0),
                text(terminal_label)
                    .size(self.settings_draft.ui_pixels(SETTINGS_NAV_LABEL_POINTS))
                    .wrapping(iced::widget::text::Wrapping::None),
            ]
            .spacing(7)
            .align_y(Alignment::Center),
        ))
        .on_press(Message::CancelSettings)
        .height(30)
        .padding([0.0, SETTINGS_NAV_QUIET_PADDING_X])
        .style(move |_, status| settings_button_style(tokens, SettingsButtonKind::Quiet, status));
        // Only the shortened label needs explaining, and only it gets a
        // tooltip: repeating a label the eye can already read is noise.
        let back: Element<'_, Message> = if crowded {
            app_tooltip(
                back,
                full_terminal_label,
                tooltip::Position::Bottom,
                tokens,
                self.settings_draft.ui_pixels(9.0),
            )
        } else {
            back.into()
        };
        // The rule owns the gap on both of its sides rather than inheriting one
        // row spacing, because its neighbours are not the same kind of thing:
        // the button carries its own trailing padding and the title carries
        // none, so a single spacing pushed the rule visibly off centre.
        let nav_rule = container(
            container("")
                .width(1)
                .height(16)
                .style(move |_| container::Style::default().background(tokens.line_strong)),
        )
        .padding(Padding {
            top: 0.0,
            bottom: 0.0,
            left: SETTINGS_NAV_RULE_GAP - SETTINGS_NAV_QUIET_PADDING_X,
            right: SETTINGS_NAV_RULE_GAP,
        });
        let nav = container(
            row![
                back,
                nav_rule,
                // The window's own title while settings owns it. The page name
                // belongs to the toggle and the page heading, not here.
                //
                // It shares one type size with the return label beside it and
                // separates itself by weight and colour instead. Iced centres
                // each label's line box rather than aligning baselines, so a
                // second size on this line would sit its words a fraction of a
                // pixel off its neighbour's baseline — a drift that grows with
                // the interface type-size setting and changes with the chosen
                // interface family, and one the rule between them makes plain.
                text("Settings")
                    .size(self.settings_draft.ui_pixels(SETTINGS_NAV_LABEL_POINTS))
                    .font(Font {
                        weight: font::Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    .color(tokens.text)
                    .wrapping(iced::widget::text::Wrapping::None),
                container("").width(Fill),
                settings_page_toggle(self.settings_page, &self.settings_draft),
            ]
            .spacing(0)
            .align_y(Alignment::Center),
        )
        .height(52)
        .align_y(iced::alignment::Vertical::Center)
        // The left inset is short by the back button's own padding so its
        // glyph, not its hit area, lands on the page's content margin; the
        // toggle's well has almost none, so the right inset is the margin.
        .padding(Padding {
            top: 0.0,
            bottom: 0.0,
            left: SETTINGS_PAGE_PADDING_X - SETTINGS_NAV_QUIET_PADDING_X,
            right: SETTINGS_PAGE_PADDING_X,
        })
        .style(move |_| ruled_surface(tokens.rail, tokens.line));
        let content = match self.settings_page {
            SettingsPage::Preferences => self.preferences_settings_view(changed),
            SettingsPage::Worktrees => self.worktree_settings_view(),
        };
        container(column![nav, content].height(Fill))
            .width(Fill)
            .height(Fill)
            .style(move |_| {
                container::Style::default()
                    .background(tokens.app)
                    .color(tokens.text)
            })
            .into()
    }

    pub(crate) fn version_settings_section(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings_draft.appearance);
        let fallback = match &self.installed_versions {
            InstalledVersionsState::Unchecked => "Running",
            InstalledVersionsState::Checking => "Running · checking installed build…",
            InstalledVersionsState::Unavailable => "Running · installed check unavailable",
            InstalledVersionsState::Ready(_) => "Running",
        };
        let (installed_muxtrix, installed_muxtrixctl) = match &self.installed_versions {
            InstalledVersionsState::Ready(versions) => {
                (Some(&versions.muxtrix), Some(&versions.muxtrixctl))
            }
            _ => (None, None),
        };
        let mut rows = column![];
        if let Some(copy) = installed_version_restart_copy(&self.installed_versions) {
            rows = rows
                .push(
                    container(
                        row![
                            container(signal_dot(tokens.warning, 8.0))
                                .height(self.settings_draft.ui_pixels(13.0))
                                .align_y(iced::alignment::Vertical::Center),
                            column![
                                text("Restart to use the installed build")
                                    .size(self.settings_draft.ui_pixels(10.5))
                                    .font(Font {
                                        weight: font::Weight::Semibold,
                                        ..Font::DEFAULT
                                    })
                                    .color(tokens.text),
                                text(copy)
                                    .size(self.settings_draft.ui_pixels(9.0))
                                    .color(tokens.warning),
                            ]
                            .spacing(2),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .padding([11, 14])
                    .width(Fill)
                    .style(move |_| {
                        container::Style::default().background(Color {
                            a: 0.07,
                            ..tokens.warning
                        })
                    }),
                )
                .push(settings_divider(tokens));
        }
        rows = rows
            .push(settings_row(
                "Muxtrix",
                "Desktop application",
                settings_version_value(
                    env!("CARGO_PKG_VERSION"),
                    installed_muxtrix,
                    fallback,
                    &self.settings_draft,
                ),
                &self.settings_draft,
            ))
            .push(settings_divider(tokens))
            .push(settings_row(
                "Muxtrix Control",
                "Local control service and muxtrixctl command",
                settings_version_value(
                    muxtrix_control::VERSION,
                    installed_muxtrixctl,
                    fallback,
                    &self.settings_draft,
                ),
                &self.settings_draft,
            ));
        settings_section(
            "Versions",
            "Builds active in this window and installed on disk",
            rows,
            &self.settings_draft,
        )
    }

    pub(crate) fn agent_hook_row(&self, agent: Agent) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings_draft.appearance);
        let status = self
            .hook_statuses
            .iter()
            .find(|status| status.agent == agent && status.scope == HookScope::User);
        let installed = status.is_some_and(|status| status.installed);
        let repair_needed =
            status.is_some_and(|status| !status.installed && status.managed_entries > 0);
        let detail = status.map_or_else(
            || {
                if self.integration_refreshing {
                    "Checking…".into()
                } else {
                    "Not installed".into()
                }
            },
            |status| {
                if installed {
                    format!("Installed · {} managed entries", status.managed_entries)
                } else if status.unreachable_entries > 0 {
                    // The distinct failure: the hooks read as installed by
                    // their own text, but the binary they call is gone, so the
                    // agent has been reporting nothing at all.
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
                    "Not installed".into()
                }
            },
        );
        let command_input = match agent {
            Agent::Codex => text_input("codex", &self.settings_draft.codex_command)
                .on_input(Message::SettingsCodexCommand),
            Agent::Claude => text_input("claude", &self.settings_draft.claude_command)
                .on_input(Message::SettingsClaudeCommand),
            Agent::Pi => text_input("omp", &self.settings_draft.pi_command)
                .on_input(Message::SettingsPiCommand),
        }
        .line_height(Pixels(30.0))
        .padding([0, 9])
        .size(self.settings_draft.ui_pixels(11.0));
        let mut actions = row![].spacing(8).align_y(Alignment::Center);
        if self.integration_refreshing {
            actions = actions.push(
                text("Updating…")
                    .size(self.settings_draft.ui_pixels(9.0))
                    .color(tokens.muted),
            );
        } else if installed {
            actions = actions
                .push(settings_action_button(
                    "Launch",
                    Message::RunCommand(CommandAction::LaunchAgent(agent)),
                    SettingsButtonKind::Secondary,
                    &self.settings_draft,
                ))
                .push(settings_hook_button(
                    "Remove hooks",
                    agent,
                    HookAction::Remove,
                    SettingsButtonKind::Danger,
                    &self.settings_draft,
                ));
        } else if repair_needed {
            actions = actions
                .push(settings_hook_button(
                    "Repair hooks",
                    agent,
                    HookAction::ReAdd,
                    SettingsButtonKind::Secondary,
                    &self.settings_draft,
                ))
                .push(settings_hook_button(
                    "Remove hooks",
                    agent,
                    HookAction::Remove,
                    SettingsButtonKind::Danger,
                    &self.settings_draft,
                ));
        } else {
            actions = actions.push(settings_hook_button(
                "Add integration",
                agent,
                HookAction::Add,
                SettingsButtonKind::Secondary,
                &self.settings_draft,
            ));
        }
        container(
            column![
                row![
                    column![
                        text(match agent {
                            Agent::Codex => "Codex",
                            Agent::Claude => "Claude Code",
                            Agent::Pi => "Oh My Pi",
                        })
                        .size(self.settings_draft.ui_pixels(13.0))
                        .font(Font {
                            weight: font::Weight::Bold,
                            ..Font::DEFAULT
                        }),
                        text(detail)
                            .size(self.settings_draft.ui_pixels(10.0))
                            .color(if installed {
                                tokens.success
                            } else if repair_needed {
                                tokens.warning
                            } else {
                                tokens.muted
                            }),
                    ]
                    .spacing(2)
                    .width(Fill),
                    actions,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                command_input,
            ]
            .spacing(8),
        )
        .padding(14)
        .into()
    }
}
