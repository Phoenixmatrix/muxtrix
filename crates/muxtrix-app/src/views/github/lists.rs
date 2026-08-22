//! The pull-request and changed-file lists.
//!
//! Both are virtualised: rows are a fixed height (58 px for pull requests,
//! 42 px for files) so only the visible window plus a small overscan is built,
//! which keeps a thousand-row repository scrolling smoothly.

use iced::widget::column;

use crate::views::prelude::*;

use crate::github;

use crate::app::{
    GITHUB_FILE_ROW_HEIGHT, GITHUB_FILE_SCROLL_ID, GITHUB_PULL_REQUEST_QUERY_ID,
    GITHUB_PULL_REQUEST_ROW_HEIGHT, GITHUB_PULL_REQUEST_SCROLL_ID,
    GITHUB_PULL_REQUEST_SEARCH_HEIGHT, GITHUB_PULL_REQUEST_SUMMARY_HEIGHT, GitHubDiffSource,
    GitHubPanelKeyboardFocus, GitHubPanelState, GitHubPanelTab, SettingsButtonKind, app_tooltip,
    centered_button_content, centered_button_label, github_action_button_style,
    github_file_viewport_height, github_merge_button_style, github_pull_request_summary_copy,
    github_pull_request_viewport_height, github_readiness_copy, github_readiness_icon,
    github_virtual_window, quiet_button_style, settings_button_style, signal_dot,
    single_line_ellipsize, status_pill,
};
use iced::Font;

impl Muxtrix {
    pub(crate) fn github_pull_request_list<'a>(
        &'a self,
        panel: &'a GitHubPanelState,
        pull_requests: &'a [github::PullRequestSummary],
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let filtered = pull_requests
            .iter()
            .filter(|pull_request| pull_request.matches(&panel.pull_request_query))
            .collect::<Vec<_>>();
        let search = container(
            column![
                text("SEARCH")
                    .size(self.settings.ui_pixels(7.2))
                    .font(Font {
                        weight: font::Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .color(tokens.faint),
                text_input(
                    "Title, number, author, or branch",
                    &panel.pull_request_query
                )
                .id(iced::widget::Id::new(GITHUB_PULL_REQUEST_QUERY_ID))
                .on_input(Message::GitHubPullRequestQueryChanged)
                .padding([7, 10])
                .size(self.settings.ui_pixels(9.0))
                .style(move |_, _| text_input::Style {
                    background: iced::Background::Color(tokens.app),
                    border: Border {
                        color: tokens.line_strong,
                        width: 1.0,
                        radius: 7.0.into(),
                    },
                    icon: tokens.muted,
                    placeholder: tokens.faint,
                    value: tokens.text,
                    selection: Color {
                        a: 0.35,
                        ..tokens.accent
                    },
                }),
            ]
            .spacing(5),
        )
        .padding([8, 10])
        .height(GITHUB_PULL_REQUEST_SEARCH_HEIGHT);
        let summary_label = if pull_requests
            .iter()
            .any(|pull_request| pull_request.status == github::PullRequestSummaryStatus::Merged)
        {
            "PULL REQUESTS"
        } else {
            "OPEN PULL REQUESTS"
        };
        let summary = container(
            row![
                text(summary_label)
                    .size(self.settings.ui_pixels(8.0))
                    .font(Font {
                        weight: font::Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .color(tokens.faint),
                text(if filtered.len() == pull_requests.len() {
                    filtered.len().to_string()
                } else {
                    format!("{} of {}", filtered.len(), pull_requests.len())
                })
                .size(self.settings.ui_pixels(8.0))
                .color(tokens.muted),
            ]
            .spacing(7)
            .align_y(Alignment::Center),
        )
        .height(GITHUB_PULL_REQUEST_SUMMARY_HEIGHT)
        .padding([7, 12])
        .width(Fill)
        .style(move |_| container::Style::default().background(tokens.panel));
        let list: Element<'_, Message> = if filtered.is_empty() {
            self.github_centered_state(
                IconKind::GitHub,
                if pull_requests.is_empty() {
                    "No open pull requests"
                } else {
                    "No matching pull requests"
                },
                if pull_requests.is_empty() {
                    "Open pull requests for this repository will appear here."
                } else {
                    "Try a title, number, author, or branch name."
                },
                None,
                tokens,
            )
        } else {
            let viewport_height = github_pull_request_viewport_height(self.window_size);
            let (first, last) = github_virtual_window(
                filtered.len(),
                panel.pull_request_scroll_offset,
                viewport_height,
                GITHUB_PULL_REQUEST_ROW_HEIGHT,
            );
            let mut rows =
                column![container("").height(first as f32 * GITHUB_PULL_REQUEST_ROW_HEIGHT)]
                    .spacing(0);
            for (index, pull_request) in filtered[first..last].iter().enumerate() {
                rows = rows.push(self.github_pull_request_row(
                    pull_request,
                    panel.pull_request_keyboard_cursor == Some(first + index),
                    tokens,
                ));
            }
            rows = rows.push(
                container("")
                    .height((filtered.len() - last) as f32 * GITHUB_PULL_REQUEST_ROW_HEIGHT),
            );
            scrollable(rows)
                .id(iced::widget::Id::new(GITHUB_PULL_REQUEST_SCROLL_ID))
                .height(Fill)
                .on_scroll(|viewport| {
                    Message::GitHubPullRequestScrolled(viewport.absolute_offset().y)
                })
                .into()
        };
        column![
            search,
            summary,
            container("")
                .height(1)
                .width(Fill)
                .style(move |_| container::Style::default().background(tokens.line)),
            list,
        ]
        .height(Fill)
        .into()
    }

    pub(crate) fn github_pull_request_row<'a>(
        &'a self,
        pull_request: &'a github::PullRequestSummary,
        keyboard_selected: bool,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let (status_label, status_color) = match pull_request.status {
            github::PullRequestSummaryStatus::Open => ("Open", tokens.github_open),
            github::PullRequestSummaryStatus::Draft => ("Draft", tokens.muted),
            github::PullRequestSummaryStatus::Merged => ("Merged", tokens.github_merged),
        };
        let state: Element<'_, Message> = status_pill(status_label, status_color, &self.settings);
        let (readiness_label, readiness_detail, readiness_color) =
            github_pull_request_summary_copy(pull_request, tokens);
        let readiness = app_tooltip(
            container(icon(
                github_readiness_icon(pull_request.readiness),
                readiness_color,
                14.0,
            ))
            .width(18)
            .height(18)
            .center_x(18)
            .align_y(iced::alignment::Vertical::Center),
            format!("{readiness_label} — {readiness_detail}"),
            tooltip::Position::Left,
            tokens,
            self.settings.ui_pixels(8.0),
        );
        button(
            row![
                icon(IconKind::GitHub, tokens.faint, 13.0),
                column![
                    row![
                        text(single_line_ellipsize(
                            &pull_request.title,
                            self.settings.ui_char_budget(33),
                        ))
                        .size(self.settings.ui_pixels(8.8))
                        .font(Font {
                            weight: font::Weight::Semibold,
                            ..Font::DEFAULT
                        })
                        .color(tokens.text)
                        .width(Fill)
                        .wrapping(iced::widget::text::Wrapping::None),
                        readiness,
                        state,
                    ]
                    .spacing(7)
                    .align_y(Alignment::Center),
                    text(format!(
                        "#{} by {}  ·  {} → {}",
                        pull_request.number,
                        pull_request.author,
                        pull_request.head,
                        pull_request.base
                    ))
                    .size(self.settings.ui_pixels(7.4))
                    .color(tokens.faint)
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None),
                ]
                .spacing(4)
                .width(Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .height(GITHUB_PULL_REQUEST_ROW_HEIGHT)
        .padding([6, 11])
        .width(Fill)
        .on_press(Message::SelectGitHubPullRequest(pull_request.number))
        .style(move |_, status| quiet_button_style(tokens, keyboard_selected, status))
        .into()
    }

    pub(crate) fn github_pull_request_details<'a>(
        &'a self,
        panel: &'a GitHubPanelState,
        details: &'a github::PullRequestDetails,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let pull_request = self.github_pull_request(panel, &details.pull_request, tokens);
        let changes_header = container(
            row![
                text("CHANGED FILES")
                    .size(self.settings.ui_pixels(8.0))
                    .font(Font {
                        weight: font::Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .color(tokens.faint),
                text(details.files.len().to_string())
                    .size(self.settings.ui_pixels(8.0))
                    .color(tokens.muted),
                container("").width(Fill),
                text(format!("+{}", details.pull_request.additions))
                    .size(self.settings.ui_pixels(8.0))
                    .font(self.settings.terminal_font.iced())
                    .color(tokens.success),
                text(format!("−{}", details.pull_request.deletions))
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
            &details.files,
            panel.selected_pull_request_file_scroll_offset,
            "No changed files",
            "GitHub did not report any files for this pull request.",
            tokens,
        );
        column![
            pull_request,
            container("")
                .height(1)
                .width(Fill)
                .style(move |_| container::Style::default().background(tokens.line)),
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

    pub(crate) fn github_pull_request<'a>(
        &'a self,
        panel: &'a GitHubPanelState,
        pull_request: &'a github::PullRequest,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let readiness = pull_request.readiness();
        let (readiness_label, readiness_detail, readiness_color) = if panel.merging {
            (
                "Merge in progress",
                "Waiting for GitHub to finish the merge",
                tokens.accent,
            )
        } else if panel.draft_state_updating {
            (
                "Updating pull request",
                if pull_request.draft {
                    "Marking it ready for review on GitHub"
                } else {
                    "Converting it to a draft on GitHub"
                },
                tokens.accent,
            )
        } else {
            github_readiness_copy(readiness, tokens)
        };
        let busy = panel.merging || panel.draft_state_updating;
        let draft_label = if panel.draft_state_updating {
            "Updating…"
        } else if pull_request.draft {
            "Mark ready"
        } else {
            "Convert to draft"
        };
        let draft_action = button(centered_button_label(
            draft_label,
            self.settings.ui_pixels(8.5),
        ))
        .on_press_maybe(
            (!busy && !panel.merge_confirmation).then_some(Message::ToggleGitHubPullRequestDraft),
        )
        .height(30)
        .padding([0, 12])
        .style(move |_, status| {
            github_action_button_style(
                tokens,
                panel.keyboard_focus == Some(GitHubPanelKeyboardFocus::DraftAction),
                status,
            )
        });
        let merge_label: Element<'_, Message> = if panel.merging {
            centered_button_content(
                row![
                    signal_dot(tokens.accent, 4.5),
                    text("Merging…").size(self.settings.ui_pixels(8.5)),
                ]
                .spacing(5)
                .align_y(Alignment::Center),
            )
        } else {
            centered_button_label("Merge", self.settings.ui_pixels(8.5))
        };
        let mut merge = button(merge_label)
            .height(30)
            .padding([0, 12])
            .style(move |_, status| {
                github_merge_button_style(
                    tokens,
                    panel.keyboard_focus == Some(GitHubPanelKeyboardFocus::MergeAction),
                    status,
                )
            });
        if readiness == github::MergeReadiness::Ready && !busy && !panel.merge_confirmation {
            merge = merge.on_press(Message::RequestGitHubMerge);
        }
        let readiness_row = row![
            signal_dot(readiness_color, 8.0),
            column![
                text(readiness_label)
                    .size(self.settings.ui_pixels(9.0))
                    .font(Font {
                        weight: font::Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    .color(readiness_color),
                text(readiness_detail)
                    .size(self.settings.ui_pixels(7.5))
                    .color(tokens.muted),
            ]
            .spacing(1)
            .width(Fill),
        ]
        .spacing(9)
        .align_y(Alignment::Center);
        let actions = row![draft_action, container("").width(Fill), merge]
            .spacing(8)
            .align_y(Alignment::Center);

        let title = button(
            column![
                text(&pull_request.title)
                    .size(self.settings.ui_pixels(11.0))
                    .font(Font {
                        weight: font::Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    .color(tokens.text)
                    .width(Fill),
                text(format!(
                    "#{} by {}  ·  {} into {}",
                    pull_request.number, pull_request.author, pull_request.head, pull_request.base
                ))
                .size(self.settings.ui_pixels(8.0))
                .color(tokens.faint)
                .width(Fill)
                .wrapping(iced::widget::text::Wrapping::None),
            ]
            .spacing(4),
        )
        .on_press(Message::OpenGitHubPullRequest(pull_request.url.clone()))
        .padding(0)
        .width(Fill)
        .style(move |_, status| quiet_button_style(tokens, false, status));
        let checks_color = if pull_request.checks.failed > 0 {
            tokens.danger
        } else if pull_request.checks.pending > 0 {
            tokens.warning
        } else {
            tokens.success
        };
        let stats = row![
            text(format!("{} files", pull_request.changed_files))
                .size(self.settings.ui_pixels(8.0))
                .color(tokens.muted),
            text(format!("+{}", pull_request.additions))
                .size(self.settings.ui_pixels(8.0))
                .font(self.settings.terminal_font.iced())
                .color(tokens.success),
            text(format!("−{}", pull_request.deletions))
                .size(self.settings.ui_pixels(8.0))
                .font(self.settings.terminal_font.iced())
                .color(tokens.danger),
            container("").width(Fill),
            signal_dot(checks_color, 6.0),
            text(format!(
                "{} passed · {} pending · {} failed",
                pull_request.checks.passed, pull_request.checks.pending, pull_request.checks.failed
            ))
            .size(self.settings.ui_pixels(7.5))
            .color(tokens.muted),
        ]
        .spacing(6)
        .align_y(Alignment::Center);

        let merge_progress: Element<'_, Message> = if panel.merging {
            let active_dot = (panel.loading_phase / 3) % 3;
            let mut dots = row![].spacing(4).align_y(Alignment::Center);
            for index in 0..3 {
                let color = if index == active_dot {
                    tokens.accent
                } else if (index + 1) % 3 == active_dot {
                    Color {
                        a: 0.52,
                        ..tokens.accent
                    }
                } else {
                    tokens.line_strong
                };
                dots = dots.push(signal_dot(color, 5.0));
            }
            container(
                row![
                    container(dots)
                        .width(38)
                        .height(30)
                        .align_x(iced::alignment::Horizontal::Center)
                        .align_y(iced::alignment::Vertical::Center)
                        .style(move |_| {
                            container::Style::default()
                                .background(tokens.panel_raised)
                                .border(Border {
                                    color: Color {
                                        a: 0.42,
                                        ..tokens.accent
                                    },
                                    width: 1.0,
                                    radius: 7.0.into(),
                                })
                        }),
                    column![
                        text(format!("Merging pull request #{}…", pull_request.number))
                            .size(self.settings.ui_pixels(9.0))
                            .font(Font {
                                weight: font::Weight::Semibold,
                                ..Font::DEFAULT
                            })
                            .color(tokens.accent),
                        text("GitHub is creating the merge commit. The branch is kept.")
                            .size(self.settings.ui_pixels(8.0))
                            .color(tokens.muted),
                    ]
                    .spacing(2)
                    .width(Fill),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .padding(10)
            .width(Fill)
            .style(move |_| container::Style {
                background: Some(
                    Color {
                        a: 0.09,
                        ..tokens.accent
                    }
                    .into(),
                ),
                border: Border {
                    color: Color {
                        a: 0.38,
                        ..tokens.accent
                    },
                    width: 1.0,
                    radius: 6.0.into(),
                },
                shadow: Shadow {
                    color: Color::from_rgba8(0, 0, 0, 0.18),
                    offset: Vector::new(0.0, 2.0),
                    blur_radius: 10.0,
                },
                ..container::Style::default()
            })
            .into()
        } else {
            container("").height(0).into()
        };

        let confirmation: Element<'_, Message> = if panel.merge_confirmation && !panel.merging {
            container(
                column![
                    text(format!("Merge pull request #{}?", pull_request.number))
                        .size(self.settings.ui_pixels(9.0))
                        .font(Font {
                            weight: font::Weight::Semibold,
                            ..Font::DEFAULT
                        })
                        .color(tokens.text),
                    text("This creates a merge commit on GitHub. The branch is kept.")
                        .size(self.settings.ui_pixels(8.0))
                        .color(tokens.muted),
                    row![
                        button(centered_button_label(
                            "Cancel",
                            self.settings.ui_pixels(8.5),
                        ))
                        .on_press(Message::CancelGitHubMerge)
                        .height(30)
                        .padding([0, 12])
                        .style(move |_, status| {
                            settings_button_style(tokens, SettingsButtonKind::Secondary, status)
                        }),
                        button(centered_button_label(
                            "Merge pull request",
                            self.settings.ui_pixels(8.5),
                        ))
                        .on_press(Message::ConfirmGitHubMerge)
                        .height(30)
                        .padding([0, 12])
                        .style(move |_, status| {
                            github_merge_button_style(
                                tokens,
                                panel.keyboard_focus == Some(GitHubPanelKeyboardFocus::MergeAction),
                                status,
                            )
                        }),
                    ]
                    .spacing(8),
                ]
                .spacing(8),
            )
            .padding(10)
            .width(Fill)
            .style(move |_| {
                container::Style::default()
                    .background(Color {
                        a: 0.07,
                        ..tokens.success
                    })
                    .border(Border {
                        color: Color {
                            a: 0.35,
                            ..tokens.success
                        },
                        width: 1.0,
                        radius: 6.0.into(),
                    })
            })
            .into()
        } else {
            container("").height(0).into()
        };
        let action_error: Element<'_, Message> =
            if let Some(error) = panel.pull_request_action_error.as_deref() {
                container(
                    row![
                        signal_dot(tokens.danger, 6.0),
                        text(error)
                            .size(self.settings.ui_pixels(8.0))
                            .color(tokens.danger),
                    ]
                    .spacing(7)
                    .align_y(Alignment::Center),
                )
                .padding([7, 9])
                .width(Fill)
                .style(move |_| {
                    container::Style::default()
                        .background(Color {
                            a: 0.06,
                            ..tokens.danger
                        })
                        .border(Border {
                            color: Color {
                                a: 0.32,
                                ..tokens.danger
                            },
                            width: 1.0,
                            radius: 6.0.into(),
                        })
                })
                .into()
            } else {
                container("").height(0).into()
            };

        container(
            column![
                readiness_row,
                actions,
                action_error,
                title,
                stats,
                merge_progress,
                confirmation
            ]
            .spacing(10),
        )
        .padding(12)
        .width(Fill)
        .into()
    }

    pub(crate) fn github_file_list<'a>(
        &'a self,
        files: &'a [github::FileChange],
        offset: f32,
        empty_title: &'static str,
        empty_detail: &'static str,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        if files.is_empty() {
            return container(
                column![
                    icon(IconKind::File, tokens.faint, 22.0),
                    text(empty_title)
                        .size(self.settings.ui_pixels(10.0))
                        .color(tokens.text),
                    text(empty_detail)
                        .size(self.settings.ui_pixels(8.5))
                        .color(tokens.muted),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into();
        }
        let pull_request_detail = self.github_panel.as_ref().is_some_and(|panel| {
            panel.active_tab == GitHubPanelTab::PullRequests
                && panel.selected_pull_request_number.is_some()
        });
        let viewport_height = github_file_viewport_height(self.window_size, pull_request_detail);
        let (first, last) =
            github_virtual_window(files.len(), offset, viewport_height, GITHUB_FILE_ROW_HEIGHT);
        let mut rows =
            column![container("").height(first as f32 * GITHUB_FILE_ROW_HEIGHT)].spacing(0);
        let keyboard_cursor = self
            .github_panel
            .as_ref()
            .and_then(|panel| panel.file_keyboard_cursor);
        for (index, file) in files[first..last].iter().enumerate() {
            rows = rows.push(self.github_file_row(
                file,
                keyboard_cursor == Some(first + index),
                tokens,
            ));
        }
        rows =
            rows.push(container("").height((files.len() - last) as f32 * GITHUB_FILE_ROW_HEIGHT));
        scrollable(rows)
            .id(iced::widget::Id::new(GITHUB_FILE_SCROLL_ID))
            .height(Fill)
            .on_scroll(|viewport| Message::GitHubFileScrolled(viewport.absolute_offset().y))
            .into()
    }

    pub(crate) fn github_file_row<'a>(
        &'a self,
        file: &'a github::FileChange,
        keyboard_selected: bool,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let status_color = if file.status == "Conflict" || file.status == "Deleted" {
            tokens.danger
        } else if file.status == "Untracked" || file.status == "Added" {
            tokens.success
        } else {
            tokens.muted
        };
        let selected = self.github_diff.as_ref().is_some_and(|diff| {
            let matching_source = self.github_panel.as_ref().is_some_and(|panel| {
                match (panel.active_tab, diff.source) {
                    (GitHubPanelTab::Local, GitHubDiffSource::Local) => true,
                    (GitHubPanelTab::PullRequests, GitHubDiffSource::PullRequest(number)) => {
                        panel.selected_pull_request_number == Some(number)
                    }
                    _ => false,
                }
            });
            matching_source && diff.path == file.path
        });
        button(
            row![
                icon(IconKind::File, tokens.faint, 13.0),
                column![
                    text(single_line_ellipsize(
                        &file.path,
                        self.settings.ui_char_budget(36),
                    ))
                    .size(self.settings.ui_pixels(8.8))
                    .color(tokens.text)
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None),
                    text(&file.status)
                        .size(self.settings.ui_pixels(7.2))
                        .color(status_color),
                ]
                .spacing(1)
                .width(Fill),
                text(format!("+{}", file.additions))
                    .size(self.settings.ui_pixels(7.5))
                    .font(self.settings.terminal_font.iced())
                    .color(tokens.success),
                text(format!("−{}", file.deletions))
                    .size(self.settings.ui_pixels(7.5))
                    .font(self.settings.terminal_font.iced())
                    .color(tokens.danger),
            ]
            .spacing(7)
            .align_y(Alignment::Center),
        )
        .height(GITHUB_FILE_ROW_HEIGHT)
        .padding([4, 11])
        .width(Fill)
        .on_press(Message::OpenGitHubDiff(file.path.clone()))
        .style(move |_, status| quiet_button_style(tokens, selected || keyboard_selected, status))
        .into()
    }
}
