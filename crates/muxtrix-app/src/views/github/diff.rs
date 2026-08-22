//! The unified diff view.
//!
//! Lines are laid out on a fixed 24 px grid and soft-wrapped to the available
//! width, so the visible window can be computed rather than measured.

use iced::widget::column;

use crate::views::prelude::*;

use crate::github;

use crate::app::{
    GITHUB_DIFF_CHROME_WIDTH, GITHUB_DIFF_LINE_HEIGHT, GITHUB_PANEL_WIDTH, GitHubDiffState,
    SettingsButtonKind, centered_button_content, github_diff_header_height, github_diff_window,
    ruled_surface, settings_button_style, single_line_ellipsize,
};
use iced::Font;

impl Muxtrix {
    pub(crate) fn github_diff_view(&self, tokens: DesignTokens) -> Element<'_, Message> {
        let Some(diff) = self.github_diff.as_ref() else {
            return container("").width(Fill).height(Fill).into();
        };
        let back = || {
            button(centered_button_content(
                row![
                    icon(IconKind::Back, tokens.muted, 12.0),
                    text("Back to workspace")
                        .size(self.settings.ui_pixels(9.5))
                        .wrapping(iced::widget::text::Wrapping::None),
                ]
                .spacing(7)
                .align_y(Alignment::Center),
            ))
            .on_press(Message::CloseGitHubDiff)
            .height(30)
            .padding([0, 10])
            .style(move |_, status| {
                settings_button_style(tokens, SettingsButtonKind::Quiet, status)
            })
        };
        let compact_header = github_diff_header_height(self.window_size.width) > 52.0;
        let header: Element<'_, Message> = if compact_header {
            let available = (self.window_size.width - GITHUB_PANEL_WIDTH - 24.0).max(120.0);
            let path = single_line_ellipsize(
                &diff.path,
                self.settings
                    .ui_char_budget((available / 8.0).floor() as usize),
            );
            container(
                column![
                    row![
                        back(),
                        container("").width(Fill),
                        text(format!("+{}", diff.additions))
                            .size(self.settings.ui_pixels(8.5))
                            .font(self.settings.terminal_font.iced())
                            .color(tokens.success),
                        text(format!("−{}", diff.deletions))
                            .size(self.settings.ui_pixels(8.5))
                            .font(self.settings.terminal_font.iced())
                            .color(tokens.danger),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                    row![
                        text(path)
                            .size(self.settings.ui_pixels(9.5))
                            .font(Font {
                                weight: font::Weight::Semibold,
                                ..Font::DEFAULT
                            })
                            .color(tokens.text)
                            .width(Fill)
                            .wrapping(iced::widget::text::Wrapping::None),
                        text(&diff.status)
                            .size(self.settings.ui_pixels(7.5))
                            .color(tokens.faint),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ]
                .spacing(4),
            )
            .height(github_diff_header_height(self.window_size.width))
            .padding([7, 12])
            .clip(true)
            .style(move |_| ruled_surface(tokens.rail, tokens.line))
            .into()
        } else {
            container(
                row![
                    back(),
                    container("")
                        .width(1)
                        .height(16)
                        .style(move |_| container::Style::default().background(tokens.line_strong)),
                    column![
                        text(&diff.path)
                            .size(self.settings.ui_pixels(10.5))
                            .font(Font {
                                weight: font::Weight::Semibold,
                                ..Font::DEFAULT
                            })
                            .color(tokens.text)
                            .width(Fill)
                            .wrapping(iced::widget::text::Wrapping::None),
                        text(&diff.status)
                            .size(self.settings.ui_pixels(7.5))
                            .color(tokens.faint),
                    ]
                    .spacing(1)
                    .width(Fill),
                    text(format!("+{}", diff.additions))
                        .size(self.settings.ui_pixels(8.5))
                        .font(self.settings.terminal_font.iced())
                        .color(tokens.success),
                    text(format!("−{}", diff.deletions))
                        .size(self.settings.ui_pixels(8.5))
                        .font(self.settings.terminal_font.iced())
                        .color(tokens.danger),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .height(github_diff_header_height(self.window_size.width))
            .padding([0, 18])
            .clip(true)
            .style(move |_| ruled_surface(tokens.rail, tokens.line))
            .into()
        };

        let body: Element<'_, Message> = if diff.loading {
            self.github_centered_state(
                IconKind::Refresh,
                "Loading diff…",
                "Reading the selected file without blocking the workspace.",
                None,
                tokens,
            )
        } else if let Some(error) = diff.error.as_deref() {
            self.github_centered_state(
                IconKind::File,
                "Diff unavailable",
                error,
                Some(("Try again", Message::RetryGitHubDiff)),
                tokens,
            )
        } else if let Some(document) = diff.document.as_ref() {
            self.github_diff_document_view(diff, document, tokens)
        } else {
            container("").width(Fill).height(Fill).into()
        };

        container(column![header, body].width(Fill).height(Fill))
            .width(Fill)
            .height(Fill)
            .style(move |_| container::Style::default().background(tokens.panel))
            .into()
    }

    pub(crate) fn github_diff_document_view<'a>(
        &'a self,
        diff: &'a GitHubDiffState,
        document: &'a github::DiffDocument,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        if document.lines.is_empty() {
            return self.github_centered_state(
                IconKind::File,
                "No textual diff",
                document
                    .notice
                    .as_deref()
                    .unwrap_or("This file has no textual changes to display."),
                None,
                tokens,
            );
        }
        let notice_height = document
            .notice
            .as_ref()
            .map_or(0.0, |_| GITHUB_DIFF_LINE_HEIGHT);
        let viewport_height = (self.window_size.height
            - github_diff_header_height(self.window_size.width)
            - notice_height)
            .max(120.0);
        let (first, last, top_rows, bottom_rows) =
            github_diff_window(&diff.line_starts, diff.scroll_offset, viewport_height);
        let wrapped = diff.wrap_columns.is_some();
        let content_width = if wrapped {
            Fill
        } else {
            Length::Fixed(
                (document.max_columns as f32 * self.settings.terminal_cell_width()
                    + GITHUB_DIFF_CHROME_WIDTH)
                    .max((self.window_size.width - GITHUB_PANEL_WIDTH).max(320.0)),
            )
        };
        let mut rows = column![container("").height(top_rows as f32 * GITHUB_DIFF_LINE_HEIGHT)]
            .spacing(0)
            .width(content_width);
        for (line_index, line) in document.lines[first..last].iter().enumerate() {
            let line_index = first + line_index;
            let visual_rows = diff.line_starts[line_index + 1] - diff.line_starts[line_index];
            rows = rows.push(self.github_diff_line(line, visual_rows, wrapped, tokens));
        }
        rows = rows.push(container("").height(bottom_rows as f32 * GITHUB_DIFF_LINE_HEIGHT));
        let direction = if wrapped {
            scrollable::Direction::Vertical(scrollable::Scrollbar::default())
        } else {
            scrollable::Direction::Both {
                vertical: scrollable::Scrollbar::default(),
                horizontal: scrollable::Scrollbar::default(),
            }
        };
        let viewer: Element<'_, Message> = scrollable(rows)
            .id(iced::widget::Id::new("muxtrix-github-diff"))
            .width(Fill)
            .height(Fill)
            .direction(direction)
            .on_scroll(|viewport| Message::GitHubDiffScrolled(viewport.absolute_offset().y))
            .into();
        if let Some(notice) = document.notice.as_deref() {
            let notice = container(
                text(notice)
                    .size(self.settings.ui_pixels(8.0))
                    .color(tokens.warning),
            )
            .height(GITHUB_DIFF_LINE_HEIGHT)
            .padding([4, 12])
            .width(Fill)
            .style(move |_| {
                container::Style::default().background(Color {
                    a: 0.08,
                    ..tokens.warning
                })
            });
            column![notice, viewer].width(Fill).height(Fill).into()
        } else {
            viewer
        }
    }

    pub(crate) fn github_diff_line<'a>(
        &'a self,
        line: &'a github::DiffLine,
        visual_rows: usize,
        wrapped: bool,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let (foreground, background) = match line.kind {
            github::DiffLineKind::Addition => (
                tokens.success,
                Some(Color {
                    a: 0.10,
                    ..tokens.success
                }),
            ),
            github::DiffLineKind::Deletion => (
                tokens.danger,
                Some(Color {
                    a: 0.10,
                    ..tokens.danger
                }),
            ),
            github::DiffLineKind::Hunk => (
                tokens.accent,
                Some(Color {
                    a: 0.10,
                    ..tokens.accent
                }),
            ),
            github::DiffLineKind::Metadata => (tokens.muted, Some(tokens.rail)),
            github::DiffLineKind::Context => (tokens.text, None),
        };
        let number = |value: Option<usize>| {
            text(value.map_or_else(String::new, |line| line.to_string()))
                .size(self.settings.ui_pixels(7.5))
                .font(self.settings.terminal_font.iced())
                .color(tokens.faint)
                .align_x(iced::alignment::Horizontal::Right)
                .line_height(Pixels(GITHUB_DIFF_LINE_HEIGHT))
                .width(42)
        };
        let code = text(&line.text)
            .size(self.settings.terminal_font_pixels())
            .font(self.settings.terminal_font.iced())
            .color(foreground)
            .line_height(Pixels(GITHUB_DIFF_LINE_HEIGHT))
            .width(if wrapped { Fill } else { Length::Shrink })
            .wrapping(if wrapped {
                iced::widget::text::Wrapping::Glyph
            } else {
                iced::widget::text::Wrapping::None
            });
        container(
            row![
                number(line.old_line),
                number(line.new_line),
                container("")
                    .width(1)
                    .height(Fill)
                    .style(move |_| container::Style::default().background(tokens.line)),
                code,
            ]
            .spacing(7)
            .align_y(Alignment::Start),
        )
        .height(visual_rows.max(1) as f32 * GITHUB_DIFF_LINE_HEIGHT)
        .padding([0, 8])
        .width(Fill)
        .style(move |_| container::Style {
            background: background.map(iced::Background::Color),
            ..container::Style::default()
        })
        .into()
    }
}
