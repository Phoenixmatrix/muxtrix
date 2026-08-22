//! The preferences page: appearance, fonts, terminal, agents, integrations.
//!
//! Every control edits the settings *draft*, never the saved settings, so the
//! page can be abandoned without effect until Save writes it through.

use iced::widget::column;

use crate::views::prelude::*;

use crate::settings;

use crate::app::{
    FONT_FAMILY_MENU_MAX_HEIGHT, SETTINGS_PAGE_PADDING_X, SETTINGS_SCROLL_ID, SettingsButtonKind,
    ruled_surface, settings_action_button, settings_action_button_maybe, settings_divider,
    settings_row, settings_section, terminal_theme_preview,
};
use crate::settings::{Appearance, FleetScope, font_with_style};
use crate::themes::TerminalThemeId;
use iced::Font;
use muxtrix_control::Agent;

impl Muxtrix {
    pub(crate) fn preferences_settings_view(&self, changed: bool) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings_draft.appearance);
        let can_continue_pending_command = self.pending_default_agent_command.is_some()
            && self
                .settings_draft
                .default_agent
                .is_some_and(|agent| self.agent_is_configured_for(agent, &self.settings_draft));
        let title = column![
            text("Preferences")
                .size(self.settings_draft.ui_pixels(22.0))
                .font(Font {
                    weight: font::Weight::Bold,
                    ..Font::DEFAULT
                }),
            text("Tune the interface, terminal, and developer integrations.")
                .size(self.settings_draft.ui_pixels(11.0))
                .color(tokens.muted),
        ]
        .spacing(4);

        let ui_scale = row![
            slider(
                12.0..=20.0,
                self.settings_draft.ui_font_size,
                Message::SettingsUiFontSize,
            )
            .step(1.0_f32)
            .width(220),
            text(format!("{:.0} pt", self.settings_draft.ui_font_size))
                .size(self.settings_draft.ui_pixels(10.0))
                .color(tokens.muted)
                .width(52),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let interface = settings_section(
            "Appearance",
            "Theme and interface chrome",
            column![
                settings_row(
                    "Theme",
                    "Color scheme for the application",
                    pick_list(
                        Appearance::ALL,
                        Some(self.settings_draft.appearance),
                        Message::SettingsAppearance,
                    )
                    .width(220),
                    &self.settings_draft
                ),
                settings_divider(tokens),
                settings_row(
                    "Interface font",
                    "Installed font used by application chrome",
                    pick_list(
                        self.available_ui_fonts.clone(),
                        Some(self.settings_draft.ui_font.clone()),
                        Message::SettingsUiFont,
                    )
                    .menu_height(Length::Fixed(FONT_FAMILY_MENU_MAX_HEIGHT))
                    .width(280),
                    &self.settings_draft,
                ),
                settings_divider(tokens),
                settings_row(
                    "Interface font weight",
                    "Weights installed for the selected family",
                    pick_list(
                        self.available_ui_font_weights.clone(),
                        Some(self.settings_draft.ui_font_weight),
                        Message::SettingsUiFontWeight,
                    )
                    .width(220),
                    &self.settings_draft,
                ),
                settings_divider(tokens),
                settings_row(
                    "Interface text size",
                    "Scales labels, controls, and workspace chrome",
                    ui_scale,
                    &self.settings_draft
                ),
                settings_divider(tokens),
                settings_row(
                    "Workspace status bar",
                    "Show process messages and pane count at the bottom",
                    toggler(self.settings_draft.show_status_bar)
                        .on_toggle(Message::SettingsShowStatusBar)
                        .size(18),
                    &self.settings_draft
                ),
                settings_divider(tokens),
                settings_row(
                    "Show all workspaces in Fleet",
                    "Include panes from every workspace; when off, show only the current workspace",
                    toggler(self.settings_draft.fleet_scope == FleetScope::AllWorkspaces)
                        .on_toggle(Message::SettingsShowAllWorkspaces)
                        .size(18),
                    &self.settings_draft
                ),
            ],
            &self.settings_draft,
        );

        let font_size = row![
            slider(
                10.0..=28.0,
                self.settings_draft.terminal_font_size,
                Message::SettingsTerminalFontSize,
            )
            .step(1.0_f32)
            .width(220),
            text(format!("{:.0} pt", self.settings_draft.terminal_font_size))
                .size(self.settings_draft.ui_pixels(10.0))
                .color(tokens.muted)
                .width(52),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let line_height = row![
            slider(
                1.0..=1.6,
                self.settings_draft.terminal_line_height,
                Message::SettingsLineHeight,
            )
            .step(0.05_f32)
            .width(220),
            text(format!("{:.2}", self.settings_draft.terminal_line_height))
                .size(self.settings_draft.ui_pixels(10.0))
                .color(tokens.muted)
                .width(52),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let scrollback_validation =
            settings::parse_terminal_scrollback_lines(&self.settings_scrollback_lines_input);
        let scrollback_valid = scrollback_validation.is_ok();
        let mut scrollback_limit = column![
            row![
                text_input("10000", &self.settings_scrollback_lines_input)
                    .on_input(Message::SettingsScrollbackLimit)
                    .line_height(Pixels(30.0))
                    .padding([0, 9])
                    .size(self.settings_draft.ui_pixels(11.0))
                    .width(180),
                text("lines")
                    .size(self.settings_draft.ui_pixels(10.0))
                    .color(tokens.muted),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        ]
        .spacing(4);
        if let Err(error) = scrollback_validation {
            scrollback_limit = scrollback_limit.push(
                text(error)
                    .size(self.settings_draft.ui_pixels(9.0))
                    .color(tokens.danger)
                    .width(260),
            );
        }
        let typography_preview = container(
            text("$ cargo test --workspace\n✓ all checks passed")
                .font(font_with_style(
                    self.settings_draft.terminal_font.iced(),
                    self.settings_draft.terminal_font_weight.iced(),
                    font::Style::Normal,
                ))
                .size(self.settings_draft.terminal_font_pixels())
                .line_height(Pixels(self.settings_draft.terminal_cell_height())),
        )
        .padding([10, 12])
        .width(Fill)
        .style(move |_| {
            container::Style::default()
                .background(tokens.app)
                .border(Border {
                    color: tokens.line,
                    width: 1.0,
                    radius: 5.0.into(),
                })
        });
        let terminal_appearance = settings_section(
            "Terminal appearance",
            "Ghostty-compatible color presets and ANSI palette",
            column![
                settings_row(
                    "Color theme",
                    "Sets terminal defaults while applications keep explicit colors",
                    pick_list(
                        TerminalThemeId::ALL,
                        Some(self.settings_draft.terminal_theme),
                        Message::SettingsTerminalTheme,
                    )
                    .width(280),
                    &self.settings_draft,
                ),
                settings_divider(tokens),
                settings_row(
                    "Theme gallery",
                    "Browse every preset with live terminal previews",
                    settings_action_button(
                        "Browse gallery",
                        Message::OpenThemeGallery,
                        SettingsButtonKind::Secondary,
                        &self.settings_draft,
                    ),
                    &self.settings_draft,
                ),
                settings_divider(tokens),
                container(terminal_theme_preview(
                    self.settings_draft.terminal_theme.preset(),
                    &self.settings_draft,
                ))
                .padding(14),
            ],
            &self.settings_draft,
        );
        let terminal = settings_section(
            "Terminal text and history",
            "Fonts, grid metrics, and scrollback history",
            column![
                settings_row(
                    "Font family",
                    "Only installed monospaced families are listed",
                    pick_list(
                        self.available_terminal_fonts.clone(),
                        Some(self.settings_draft.terminal_font.clone()),
                        Message::SettingsTerminalFont,
                    )
                    .menu_height(Length::Fixed(FONT_FAMILY_MENU_MAX_HEIGHT))
                    .width(340),
                    &self.settings_draft,
                ),
                settings_divider(tokens),
                settings_row(
                    "Font weight",
                    "Weights installed for the selected family",
                    pick_list(
                        self.available_terminal_font_weights.clone(),
                        Some(self.settings_draft.terminal_font_weight),
                        Message::SettingsTerminalFontWeight,
                    )
                    .width(220),
                    &self.settings_draft,
                ),
                settings_divider(tokens),
                settings_row(
                    "Font size",
                    "Point size used for terminal glyphs",
                    font_size,
                    &self.settings_draft
                ),
                settings_divider(tokens),
                settings_row(
                    "Line height",
                    "Vertical spacing between terminal rows",
                    line_height,
                    &self.settings_draft
                ),
                settings_divider(tokens),
                settings_row(
                    "Scrollback history",
                    "Lines kept by new and restarted panes (1,000–100,000)",
                    scrollback_limit,
                    &self.settings_draft,
                ),
                settings_divider(tokens),
                settings_row(
                    "Preview",
                    "Updates before changes are applied",
                    typography_preview,
                    &self.settings_draft
                ),
            ],
            &self.settings_draft,
        );
        let github_host_validation =
            settings::normalize_github_host(&self.settings_draft.github_host);
        let github_host_valid = github_host_validation.is_ok();
        let mut github_host_control = column![
            text_input("github.com", &self.settings_draft.github_host)
                .on_input(Message::SettingsGitHubHost)
                .line_height(Pixels(30.0))
                .padding([0, 9])
                .size(self.settings_draft.ui_pixels(11.0))
                .width(320)
        ]
        .spacing(4);
        if let Err(error) = github_host_validation {
            github_host_control = github_host_control.push(
                text(error)
                    .size(self.settings_draft.ui_pixels(9.0))
                    .color(tokens.danger)
                    .width(320),
            );
        }
        let github = settings_section(
            "GitHub",
            "Public GitHub and Enterprise Server",
            column![settings_row(
                "GitHub host",
                "Use github.com or your Enterprise Server hostname; no API path",
                github_host_control,
                &self.settings_draft,
            )],
            &self.settings_draft,
        );

        let integrations = settings_section(
            "Agent lifecycle hooks",
            "Reversible Codex, Claude Code, and Oh My Pi integration",
            column![
                settings_row(
                    "Default worktree agent",
                    "Used when a worktree command opens or restarts a pane with an agent",
                    pick_list(
                        self.default_agent_choices(),
                        Some(self.default_agent_choice()),
                        Message::SettingsDefaultAgent,
                    )
                    .width(220),
                    &self.settings_draft,
                ),
                settings_divider(tokens),
                self.agent_hook_row(Agent::Codex),
                settings_divider(tokens),
                self.agent_hook_row(Agent::Claude),
                settings_divider(tokens),
                self.agent_hook_row(Agent::Pi),
                settings_divider(tokens),
                row![
                    text("Hook changes apply immediately. Muxtrix updates only its tagged entries. Project hooks remain available in muxtrixctl.")
                        .size(self.settings_draft.ui_pixels(10.0))
                        .color(tokens.muted)
                        .width(Fill)
                        .wrapping(iced::widget::text::Wrapping::Word),
                    settings_action_button(
                        "Refresh",
                        Message::RefreshHookStatus,
                        SettingsButtonKind::Secondary,
                        &self.settings_draft,
                    ),
                ]
                .spacing(18)
                .padding(14)
                .align_y(Alignment::Center),
            ],
            &self.settings_draft,
        );
        let versions = self.version_settings_section();

        let content = column![title, interface, terminal_appearance, terminal];
        #[cfg(target_os = "windows")]
        let content = content.push(settings_section(
            "Default terminal shell",
            "Choose where new Windows terminal panes run",
            column![
                settings_row(
                    "Shell backend",
                    "Existing panes keep running until restarted",
                    pick_list(
                        WindowsShellBackend::ALL,
                        Some(self.settings_draft.windows_shell_backend),
                        Message::SettingsWindowsShellBackend,
                    )
                    .width(220),
                    &self.settings_draft,
                ),
                settings_divider(tokens),
                settings_row(
                    "WSL distribution",
                    "Distributions are discovered from wsl.exe",
                    row![
                        pick_list(
                            self.available_wsl_distributions.clone(),
                            Some(WslDistributionChoice(
                                (!self.settings_draft.wsl_distribution.is_empty())
                                    .then(|| self.settings_draft.wsl_distribution.clone())
                            )),
                            Message::SettingsWslDistribution,
                        )
                        .width(220),
                        settings_action_button(
                            "Refresh",
                            Message::RefreshWslDistributions,
                            SettingsButtonKind::Secondary,
                            &self.settings_draft,
                        ),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    &self.settings_draft,
                ),
            ],
            &self.settings_draft,
        ));
        let content = content
            .push(github)
            .push(integrations)
            .push(versions)
            .spacing(22)
            .max_width(860);
        let font_restart = if self.settings_draft.ui_font != self.settings.ui_font
            || self.settings_draft.ui_font_weight != self.settings.ui_font_weight
        {
            "Interface typography changes after restarting Muxtrix. "
        } else {
            ""
        };
        let footer = container(
            row![
                text(format!(
                    "{font_restart}Preferences apply when saved; shell and scrollback affect new and restarted panes; hook actions apply immediately"
                ))
                .size(self.settings_draft.ui_pixels(9.0))
                .color(tokens.faint)
                .width(Fill),
                settings_action_button(
                    "Cancel",
                    Message::CancelSettings,
                    SettingsButtonKind::Secondary,
                    &self.settings_draft,
                ),
                settings_action_button_maybe(
                    if can_continue_pending_command {
                        "Apply and continue"
                    } else {
                        "Apply changes"
                    },
                    ((changed || can_continue_pending_command)
                        && github_host_valid
                        && scrollback_valid)
                        .then_some(Message::SaveSettings),
                    SettingsButtonKind::Primary,
                    &self.settings_draft,
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .height(58)
        .padding([10.0, SETTINGS_PAGE_PADDING_X])
        .style(move |_| ruled_surface(tokens.rail, tokens.line));
        container(
            column![
                scrollable(
                    container(content)
                        .padding([24.0, SETTINGS_PAGE_PADDING_X])
                        .center_x(Fill)
                )
                .id(iced::widget::Id::new(SETTINGS_SCROLL_ID))
                .height(Fill),
                footer,
            ]
            .height(Fill),
        )
        .width(Fill)
        .height(Fill)
        .style(move |_| {
            container::Style::default()
                .background(tokens.app)
                .color(tokens.text)
        })
        .into()
    }
}
