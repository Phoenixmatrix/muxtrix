//! The GitHub panel shell: tabs, loading and empty states, local view.
//!
//! The panel docks at 372 px on wide windows and floats over the workspace on
//! narrow ones; both share this body. Data arrives through
//! [`crate::GitHubPanelState`] — nothing here talks to the network.

use iced::widget::column;

use crate::views::prelude::*;

use crate::{
    GITHUB_LOADING_DOT_COUNT, GITHUB_PANEL_WIDTH, GITHUB_PULL_REQUEST_LIST_CHROME_HEIGHT,
    GitHubPanelKeyboardFocus, GitHubPanelState, GitHubPanelTab, SettingsButtonKind, app_tooltip,
    centered_button_label, fleet_toggle_style, github, quiet_button_style, settings_button_style,
    signal_dot,
};
use iced::Font;

impl Muxtrix {
    pub(crate) fn github_side_panel_view(
        &self,
        tokens: DesignTokens,
        floating: bool,
    ) -> Element<'_, Message> {
        self.github_panel_view(tokens, floating)
    }

    pub(crate) fn github_panel_view(
        &self,
        tokens: DesignTokens,
        floating: bool,
    ) -> Element<'_, Message> {
        let panel = self
            .github_panel
            .as_ref()
            .expect("GitHub panel view requires panel state");
        let repo_label = panel
            .repository
            .owner_and_name
            .as_deref()
            .unwrap_or(&panel.repository.name);
        let close = app_tooltip(
            button(icon(IconKind::Close, tokens.muted, 13.0))
                .on_press(Message::CloseGitHubPanel)
                .width(30)
                .height(30)
                .padding(8)
                .style(move |_, status| quiet_button_style(tokens, false, status)),
            "Close GitHub panel",
            tooltip::Position::Bottom,
            tokens,
            self.settings.ui_pixels(9.0),
        );
        let mut header_actions = row![].spacing(4).align_y(Alignment::Center);
        if !panel.active_loading() {
            header_actions = header_actions.push(app_tooltip(
                button(icon(IconKind::Refresh, tokens.muted, 14.0))
                    .on_press(Message::RefreshGitHubPanel)
                    .width(30)
                    .height(30)
                    .padding(7)
                    .style(move |_, status| quiet_button_style(tokens, false, status)),
                match panel.active_tab {
                    GitHubPanelTab::Local => "Refresh local changes",
                    GitHubPanelTab::PullRequests
                        if panel.selected_pull_request_number.is_some() =>
                    {
                        "Refresh pull request"
                    }
                    GitHubPanelTab::PullRequests => "Refresh pull requests",
                },
                tooltip::Position::Bottom,
                tokens,
                self.settings.ui_pixels(9.0),
            ));
        }
        header_actions = header_actions.push(close);
        let header_identity: Element<'_, Message> =
            if panel.repository.name.is_empty() && panel.context_loading {
                column![
                    text("GitHub")
                        .size(self.settings.ui_pixels(10.0))
                        .font(Font {
                            weight: font::Weight::Semibold,
                            ..Font::DEFAULT
                        })
                        .color(tokens.text),
                    text("Reading focused pane…")
                        .size(self.settings.ui_pixels(8.0))
                        .color(tokens.faint),
                ]
                .spacing(2)
                .width(Fill)
                .into()
            } else {
                column![
                    text(repo_label)
                        .size(self.settings.ui_pixels(10.0))
                        .font(Font {
                            weight: font::Weight::Semibold,
                            ..Font::DEFAULT
                        })
                        .color(tokens.text)
                        .width(Fill)
                        .wrapping(iced::widget::text::Wrapping::None),
                    row![
                        icon(IconKind::Branch, tokens.faint, 11.0),
                        text(&panel.repository.branch)
                            .size(self.settings.ui_pixels(8.0))
                            .color(tokens.faint)
                            .width(Fill)
                            .wrapping(iced::widget::text::Wrapping::None),
                    ]
                    .spacing(5)
                    .align_y(Alignment::Center),
                ]
                .spacing(2)
                .width(Fill)
                .into()
            };
        let current_pull_request: Element<'_, Message> = panel
            .data
            .as_ref()
            .and_then(|data| data.current_pull_request.as_ref())
            .map_or_else(
                || container("").width(0).into(),
                |pull_request| {
                    let (state_label, color) = match pull_request.state {
                        github::CurrentPullRequestState::Open => ("Open", tokens.accent),
                        github::CurrentPullRequestState::Draft => ("Draft", tokens.muted),
                        github::CurrentPullRequestState::Closed => ("Closed", tokens.faint),
                        github::CurrentPullRequestState::Merged => ("Merged", tokens.github_merged),
                    };
                    app_tooltip(
                        button(
                            text(format!("#{}", pull_request.number))
                                .size(self.settings.ui_pixels(8.5))
                                .font(self.settings.terminal_font.iced())
                                .color(color),
                        )
                        .on_press(Message::OpenGitHubPullRequest(pull_request.url.clone()))
                        .padding([4, 6])
                        .style(move |_, status| quiet_button_style(tokens, false, status)),
                        format!(
                            "Current branch pull request #{} · {state_label}\nOpen in GitHub",
                            pull_request.number
                        ),
                        tooltip::Position::Bottom,
                        tokens,
                        self.settings.ui_pixels(9.0),
                    )
                },
            );
        let header = container(
            row![
                icon(IconKind::GitHub, tokens.text, 17.0),
                header_identity,
                current_pull_request,
                header_actions,
            ]
            .spacing(9)
            .align_y(Alignment::Center),
        )
        .height(54)
        .padding([7, 10]);

        let tab = |label: &'static str, target: GitHubPanelTab| {
            let selected = panel.active_tab == target;
            // Keep the active segment focusable. A disabled selected tab is
            // skipped by keyboard traversal and hides the current location
            // from assistive technology; selecting it again is a safe no-op.
            let message =
                (!panel.active_loading()).then_some(Message::SelectGitHubPanelTab(target));
            button(
                container(
                    text(label)
                        .size(self.settings.ui_pixels(9.0))
                        .wrapping(iced::widget::text::Wrapping::None),
                )
                .width(Fill)
                .height(Fill)
                .center_x(Fill)
                .center_y(Fill),
            )
            .on_press_maybe(message)
            .height(28)
            .width(Fill)
            .padding([0, 10])
            .style(move |_, status| fleet_toggle_style(tokens, selected, status))
        };
        let tabs = container(row![
            tab("Local", GitHubPanelTab::Local),
            tab("Pull requests", GitHubPanelTab::PullRequests),
        ])
        .padding(2)
        .width(Fill)
        .style(move |_| {
            container::Style::default()
                .background(tokens.app)
                .border(Border {
                    color: tokens.line,
                    width: 1.0,
                    radius: 7.0.into(),
                })
        });
        let tabs = container(tabs).padding([7, 10]);

        let body: Element<'_, Message> = if panel.context_loading {
            self.github_panel_loading_state(panel, tokens)
        } else {
            match panel.active_tab {
            GitHubPanelTab::Local => {
                if panel.loading {
                    self.github_panel_loading_state(panel, tokens)
                } else if let Some(error) = panel.error.as_deref() {
                    self.github_centered_state(
                        IconKind::GitHub,
                        "Repository unavailable",
                        error,
                        Some(("Try again", Message::RefreshGitHubPanel)),
                        tokens,
                    )
                } else if let Some(data) = panel.data.as_ref() {
                    self.github_local_view(panel, data, tokens)
                } else {
                    self.github_centered_state(
                        IconKind::GitHub,
                        "Repository unavailable",
                        panel
                            .error
                            .as_deref()
                            .unwrap_or("Refresh to try loading this repository again."),
                        Some(("Try again", Message::RefreshGitHubPanel)),
                        tokens,
                    )
                }
            }
                GitHubPanelTab::PullRequests => match &self.github_auth {
                github::AuthStatus::Authenticated { .. } => {
                    self.github_pull_requests_view(panel, tokens)
                }
                github::AuthStatus::Checking => self.github_centered_state(
                IconKind::GitHub,
                "Checking GitHub…",
                "Muxtrix is checking for an authenticated GitHub account.",
                None,
                tokens,
            ),
                github::AuthStatus::NeedsAuthentication => self.github_centered_state(
                IconKind::GitHub,
                if self.github_auth_busy {
                    "Finish in your browser"
                } else {
                    "Connect GitHub"
                },
                if self.github_auth_busy {
                    "Complete the GitHub sign-in, then this panel will refresh automatically."
                } else {
                    "Authenticate to see pull request details, merge readiness, checks, and merge controls."
                },
                (!self.github_auth_busy)
                    .then_some(("Authenticate with GitHub", Message::BeginGitHubAuth)),
                tokens,
            ),
                github::AuthStatus::Unavailable { reason } => self.github_centered_state(
                IconKind::GitHub,
                "GitHub CLI required",
                reason,
                (!self.github_auth_busy)
                    .then_some(("Try connecting again", Message::BeginGitHubAuth)),
                tokens,
            ),
                },
            }
        };

        row![
            container("")
                .width(1)
                .height(Fill)
                .style(move |_| container::Style::default().background(tokens.line_strong)),
            container(column![
                header,
                container("")
                    .height(1)
                    .width(Fill)
                    .style(move |_| container::Style::default().background(tokens.line)),
                tabs,
                container("")
                    .height(1)
                    .width(Fill)
                    .style(move |_| container::Style::default().background(tokens.line)),
                body,
            ])
            .width(GITHUB_PANEL_WIDTH - 1.0)
            .height(Fill)
            .style(move |_| container::Style {
                background: Some(tokens.rail.into()),
                shadow: if floating {
                    Shadow {
                        color: Color::from_rgba8(0, 0, 0, 0.38),
                        offset: Vector::new(-7.0, 0.0),
                        blur_radius: 22.0,
                    }
                } else {
                    Shadow::default()
                },
                ..container::Style::default()
            }),
        ]
        .width(GITHUB_PANEL_WIDTH)
        .height(Fill)
        .into()
    }

    pub(crate) fn github_panel_loading_state<'a>(
        &'a self,
        panel: &'a GitHubPanelState,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let (title, detail) = if panel.context_loading {
            (
                "Reading focused pane…".to_owned(),
                "Updating repository context and local changes.".to_owned(),
            )
        } else {
            match panel.active_tab {
                GitHubPanelTab::Local if panel.data.is_some() => (
                    "Refreshing local changes…".to_owned(),
                    "Reading the focused pane's working tree and staged changes.".to_owned(),
                ),
                GitHubPanelTab::Local => (
                    "Reading local changes…".to_owned(),
                    "Collecting the focused pane's working tree changes.".to_owned(),
                ),
                GitHubPanelTab::PullRequests
                    if panel.selected_pull_request_number.is_some()
                        && panel.selected_pull_request.is_some() =>
                {
                    let number = panel.selected_pull_request_number.unwrap_or_default();
                    (
                        format!("Refreshing pull request #{number}…"),
                        "Checking its current metadata, readiness, and changed files.".to_owned(),
                    )
                }
                GitHubPanelTab::PullRequests if panel.selected_pull_request_number.is_some() => {
                    let number = panel.selected_pull_request_number.unwrap_or_default();
                    (
                        format!("Reading pull request #{number}…"),
                        "Collecting its metadata, readiness, and changed files.".to_owned(),
                    )
                }
                GitHubPanelTab::PullRequests if panel.pull_requests.is_some() => (
                    "Refreshing pull requests…".to_owned(),
                    "Checking the repository's open pull requests.".to_owned(),
                ),
                GitHubPanelTab::PullRequests => (
                    "Reading pull requests…".to_owned(),
                    "Collecting the repository's open pull requests.".to_owned(),
                ),
            }
        };
        self.github_loading_state(panel.loading_phase, title, detail, tokens)
    }

    pub(crate) fn github_loading_state(
        &self,
        loading_phase: u8,
        title: String,
        detail: String,
        tokens: DesignTokens,
    ) -> Element<'_, Message> {
        let mut dots = column![].spacing(5).align_x(Alignment::Center);
        for row_index in 0..3 {
            let mut dot_row = row![].spacing(5).align_y(Alignment::Center);
            for column_index in 0..3 {
                let index = row_index * 3 + column_index;
                let distance = (loading_phase + GITHUB_LOADING_DOT_COUNT - index as u8)
                    % GITHUB_LOADING_DOT_COUNT;
                let color = match distance {
                    0 => tokens.accent,
                    1 => Color {
                        a: 0.68,
                        ..tokens.accent
                    },
                    2 => Color {
                        a: 0.38,
                        ..tokens.accent
                    },
                    _ => tokens.line_strong,
                };
                dot_row = dot_row.push(signal_dot(color, 6.0));
            }
            dots = dots.push(dot_row);
        }
        let indicator = container(dots)
            .width(52)
            .height(52)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center)
            .style(move |_| container::Style {
                background: Some(tokens.panel_raised.into()),
                border: Border {
                    color: tokens.line_strong,
                    width: 1.0,
                    radius: 12.0.into(),
                },
                ..container::Style::default()
            });
        container(
            column![
                indicator,
                text(title)
                    .size(self.settings.ui_pixels(13.0))
                    .font(Font {
                        weight: font::Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    .color(tokens.text),
                text(detail)
                    .size(self.settings.ui_pixels(9.0))
                    .color(tokens.muted)
                    .center()
                    .width(280),
            ]
            .spacing(10)
            .align_x(Alignment::Center),
        )
        .width(Fill)
        .height(Fill)
        .center_x(Fill)
        .center_y(Fill)
        .padding(24)
        .into()
    }

    pub(crate) fn github_centered_state<'a>(
        &'a self,
        kind: IconKind,
        title: &'a str,
        detail: &'a str,
        action: Option<(&'static str, Message)>,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let mut content = column![
            container(icon(kind, tokens.muted, 28.0))
                .padding(12)
                .style(move |_| container::Style {
                    background: Some(tokens.panel_raised.into()),
                    border: Border {
                        color: tokens.line_strong,
                        width: 1.0,
                        radius: 12.0.into(),
                    },
                    ..container::Style::default()
                }),
            text(title)
                .size(self.settings.ui_pixels(13.0))
                .font(Font {
                    weight: font::Weight::Semibold,
                    ..Font::DEFAULT
                })
                .color(tokens.text),
            text(detail)
                .size(self.settings.ui_pixels(9.0))
                .color(tokens.muted)
                .center()
                .width(280),
        ]
        .spacing(10)
        .align_x(Alignment::Center);
        if let Some((label, message)) = action {
            content = content.push(
                button(centered_button_label(label, self.settings.ui_pixels(9.0)))
                    .on_press(message)
                    .height(34)
                    .padding([0, 14])
                    .style(move |_, status| {
                        settings_button_style(tokens, SettingsButtonKind::Primary, status)
                    }),
            );
        }
        container(content)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .padding(24)
            .into()
    }

    pub(crate) fn github_local_view<'a>(
        &'a self,
        panel: &'a GitHubPanelState,
        data: &'a github::PanelData,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let changes_header = container(
            row![
                text("LOCAL CHANGES")
                    .size(self.settings.ui_pixels(8.0))
                    .font(Font {
                        weight: font::Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .color(tokens.faint),
                text(data.files.len().to_string())
                    .size(self.settings.ui_pixels(8.0))
                    .color(tokens.muted),
                container("").width(Fill),
                text(format!("+{}", data.additions))
                    .size(self.settings.ui_pixels(8.0))
                    .font(self.settings.terminal_font.iced())
                    .color(tokens.success),
                text(format!("−{}", data.deletions))
                    .size(self.settings.ui_pixels(8.0))
                    .font(self.settings.terminal_font.iced())
                    .color(tokens.danger),
            ]
            .spacing(7)
            .align_y(Alignment::Center),
        )
        .height(38)
        .padding([8, 12])
        .style(move |_| container::Style::default().background(tokens.panel));
        let files = self.github_file_list(
            &data.files,
            panel.file_scroll_offset,
            "Working tree is clean",
            "Local file changes will appear here.",
            tokens,
        );
        column![
            changes_header,
            container("")
                .height(1)
                .width(Fill)
                .style(move |_| container::Style::default().background(tokens.line)),
            files,
        ]
        .height(Fill)
        .into()
    }

    pub(crate) fn github_pull_requests_view<'a>(
        &'a self,
        panel: &'a GitHubPanelState,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        if let Some(number) = panel.selected_pull_request_number {
            let back = button(
                container(
                    row![
                        icon(IconKind::Back, tokens.muted, 11.0),
                        text("Pull requests")
                            .size(self.settings.ui_pixels(8.5))
                            .color(tokens.muted),
                        container("").width(Fill),
                        text(format!("#{number}"))
                            .size(self.settings.ui_pixels(8.0))
                            .font(self.settings.terminal_font.iced())
                            .color(tokens.faint),
                    ]
                    .spacing(7)
                    .align_y(Alignment::Center),
                )
                .width(Fill)
                .height(Fill)
                .center_y(Fill),
            )
            .on_press_maybe(
                (!panel.selected_pull_request_loading
                    && !panel.merging
                    && !panel.draft_state_updating)
                    .then_some(Message::CloseGitHubPullRequest),
            )
            .height(36)
            .width(Fill)
            .padding([0, 12])
            .style(move |_, status| {
                quiet_button_style(
                    tokens,
                    panel.keyboard_focus == Some(GitHubPanelKeyboardFocus::Back),
                    status,
                )
            });
            let body = if panel.selected_pull_request_loading {
                self.github_panel_loading_state(panel, tokens)
            } else if let Some(error) = panel.selected_pull_request_error.as_deref() {
                self.github_centered_state(
                    IconKind::GitHub,
                    "Pull request unavailable",
                    error,
                    Some(("Try again", Message::RefreshGitHubPanel)),
                    tokens,
                )
            } else if let Some(details) = panel.selected_pull_request.as_ref() {
                self.github_pull_request_details(panel, details, tokens)
            } else {
                self.github_centered_state(
                    IconKind::GitHub,
                    "Pull request unavailable",
                    "Return to the list and choose this pull request again.",
                    None,
                    tokens,
                )
            };
            return column![
                back,
                container("")
                    .height(1)
                    .width(Fill)
                    .style(move |_| container::Style::default().background(tokens.line)),
                body,
            ]
            .height(Fill)
            .into();
        }

        if panel.pull_requests_loading {
            return column![
                container("").height(GITHUB_PULL_REQUEST_LIST_CHROME_HEIGHT),
                self.github_panel_loading_state(panel, tokens),
            ]
            .height(Fill)
            .into();
        }
        if let Some(error) = panel.pull_requests_error.as_deref() {
            return self.github_centered_state(
                IconKind::GitHub,
                "Pull requests unavailable",
                error,
                Some(("Try again", Message::RefreshGitHubPanel)),
                tokens,
            );
        }
        let Some(pull_requests) = panel.pull_requests.as_ref() else {
            return self.github_centered_state(
                IconKind::GitHub,
                "Pull requests unavailable",
                "Refresh to load this repository's open pull requests.",
                Some(("Try again", Message::RefreshGitHubPanel)),
                tokens,
            );
        };
        self.github_pull_request_list(panel, pull_requests, tokens)
    }
}
