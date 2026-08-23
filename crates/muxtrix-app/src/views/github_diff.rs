//! The unified diff view: a full-window screen with the GitHub panel docked.
//!
//! Lines sit on a fixed 24 px grid and soft-wrap to the available width, so
//! the visible window is computed from the scroll offset rather than
//! measured — the same arithmetic the iced view uses, in
//! [`crate::app::github_diff_window`].

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, ScrollWheelEvent, SharedString, Styled, div, px, svg,
};

use crate::app::{
    GITHUB_DIFF_CHROME_WIDTH, GITHUB_DIFF_LINE_HEIGHT, GITHUB_PANEL_WIDTH, GitHubDiffState,
    IconKind, Message, SettingsButtonKind, github_diff_header_height, github_diff_window,
    single_line_ellipsize,
};
use crate::github;
use crate::runtime::gpui::{Root, color};
use crate::theme::DesignTokens;
use crate::views::terminal_family;

impl Root {
    /// The diff screen, when one is open.
    pub(crate) fn github_diff_view(
        &self,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let Some(diff) = app.github_diff.as_ref() else {
            return div().size_full().into_any_element();
        };
        let settings = &app.settings;
        let ui = |points: f32| px(settings.ui_pixels(points));
        let header_height = github_diff_header_height(app.window_size.width);
        let compact_header = header_height > 52.0;

        let back = self.settings_action_button(
            "diff-back",
            "Back to workspace",
            Message::CloseGitHubDiff,
            SettingsButtonKind::Quiet,
            tokens,
            cx,
        );
        let counts = |size: f32| {
            [
                (format!("+{}", diff.additions), tokens.success),
                (format!("−{}", diff.deletions), tokens.danger),
            ]
            .into_iter()
            .map(move |(copy, hue)| {
                div()
                    .font_family(terminal_family(settings))
                    .text_size(px(settings.ui_pixels(size)))
                    .line_height((px(settings.ui_pixels(size))) * 1.3)
                    .text_color(color(hue))
                    .child(copy)
            })
        };
        let status = |size: f32| {
            div()
                .text_size(ui(size))
                .line_height((ui(size)) * 1.3)
                .text_color(color(tokens.faint))
                .whitespace_nowrap()
                .child(diff.status.clone())
        };

        let header = if compact_header {
            let available = (app.window_size.width - GITHUB_PANEL_WIDTH - 24.0).max(120.0);
            let path = single_line_ellipsize(
                &diff.path,
                settings.ui_char_budget((available / 8.0).floor() as usize),
            );
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .h(px(header_height))
                .py(px(7.))
                .px(px(12.))
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.))
                        .child(back)
                        .child(div().flex_grow(1.0))
                        .children(counts(8.5)),
                )
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
                                .text_size(ui(9.5))
                                .line_height((ui(9.5)) * 1.3)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(color(tokens.text))
                                .truncate()
                                .child(path),
                        )
                        .child(status(7.5)),
                )
        } else {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .h(px(header_height))
                .px(px(18.))
                .overflow_hidden()
                .child(back)
                .child(div().w(px(1.)).h(px(16.)).bg(color(tokens.line_strong)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .flex_grow(1.0)
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(ui(10.5))
                                .line_height((ui(10.5)) * 1.3)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(color(tokens.text))
                                .truncate()
                                .child(diff.path.clone()),
                        )
                        .child(status(7.5)),
                )
                .children(counts(8.5))
        }
        .bg(color(tokens.rail))
        .border_b(px(1.))
        .border_color(color(tokens.line));

        let body = if diff.loading {
            self.github_centered_state(
                IconKind::Refresh,
                "Loading diff…",
                "Reading the selected file without blocking the workspace.",
                None,
                tokens,
                cx,
            )
        } else if let Some(error) = diff.error.as_deref() {
            self.github_centered_state(
                IconKind::File,
                "Diff unavailable",
                error,
                Some(("Try again", Message::RetryGitHubDiff)),
                tokens,
                cx,
            )
        } else if let Some(document) = diff.document.as_ref() {
            self.github_diff_document(diff, document, tokens, cx)
        } else {
            div().size_full().into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(color(tokens.panel))
            .child(header)
            .child(
                div()
                    .flex_grow(1.0)
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(body),
            )
            .into_any_element()
    }

    fn github_diff_document(
        &self,
        diff: &GitHubDiffState,
        document: &github::DiffDocument,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
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
                cx,
            );
        }
        let notice_height = document
            .notice
            .as_ref()
            .map_or(0.0, |_| GITHUB_DIFF_LINE_HEIGHT);
        let viewport_height = (app.window_size.height
            - github_diff_header_height(app.window_size.width)
            - notice_height)
            .max(120.0);
        let (first, last, top_rows, bottom_rows) =
            github_diff_window(&diff.line_starts, diff.scroll_offset, viewport_height);
        let wrapped = diff.wrap_columns.is_some();
        let content_width = (document.max_columns as f32 * app.settings.terminal_cell_width()
            + GITHUB_DIFF_CHROME_WIDTH)
            .max((app.window_size.width - GITHUB_PANEL_WIDTH).max(320.0));

        // The window is virtual: only the lines on screen are built, with
        // spacers standing in for the rest so the scroll extent is right.
        let mut rows = div()
            .flex()
            .flex_col()
            .when(!wrapped, |rows| rows.w(px(content_width)))
            .when(wrapped, |rows| rows.w_full())
            .child(div().h(px(top_rows as f32 * GITHUB_DIFF_LINE_HEIGHT)));
        for (offset, line) in document.lines[first..last].iter().enumerate() {
            let line_index = first + offset;
            let visual_rows = diff.line_starts[line_index + 1] - diff.line_starts[line_index];
            rows = rows.child(self.github_diff_line(line, visual_rows, wrapped, tokens));
        }
        rows = rows.child(div().h(px(bottom_rows as f32 * GITHUB_DIFF_LINE_HEIGHT)));

        // The offset the application holds is the one source of truth; the
        // wheel moves it through the same message the iced scrollable sent,
        // and the window is rebuilt from it on the next frame.
        let offset = diff.scroll_offset;
        let max_offset = (diff.line_starts.last().copied().unwrap_or(0) as f32
            * GITHUB_DIFF_LINE_HEIGHT
            - viewport_height)
            .max(0.0);
        let viewer = div()
            .id("github-diff")
            .size_full()
            .overflow_hidden()
            .on_scroll_wheel(
                cx.listener(move |root, event: &ScrollWheelEvent, window, cx| {
                    let delta = match event.delta {
                        gpui::ScrollDelta::Pixels(delta) => -f32::from(delta.y),
                        gpui::ScrollDelta::Lines(delta) => -delta.y * GITHUB_DIFF_LINE_HEIGHT,
                    };
                    let next = (offset + delta).clamp(0.0, max_offset);
                    if (next - offset).abs() > 0.5 {
                        root.dispatch(Message::GitHubDiffScrolled(next), window, cx);
                    }
                }),
            )
            .child(
                div()
                    .relative()
                    .top(px(-(offset - top_rows as f32 * GITHUB_DIFF_LINE_HEIGHT)))
                    .child(rows),
            );
        match document.notice.as_deref() {
            Some(notice) => {
                let mut fill = color(tokens.warning);
                fill.a = 0.08;
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .child(
                        div()
                            .h(px(GITHUB_DIFF_LINE_HEIGHT))
                            .py(px(4.))
                            .px(px(12.))
                            .bg(fill)
                            .text_size(px(app.settings.ui_pixels(8.0)))
                            .line_height((px(app.settings.ui_pixels(8.0))) * 1.3)
                            .text_color(color(tokens.warning))
                            .child(notice.to_owned()),
                    )
                    .child(div().flex_grow(1.0).min_h(px(0.)).child(viewer))
                    .into_any_element()
            }
            None => viewer.into_any_element(),
        }
    }

    fn github_diff_line(
        &self,
        line: &github::DiffLine,
        visual_rows: usize,
        wrapped: bool,
        tokens: DesignTokens,
    ) -> AnyElement {
        let settings = &self.app().settings;
        let faded = |hue: crate::theme::Color| {
            let mut faded = color(hue);
            faded.a = 0.10;
            faded
        };
        let (foreground, background) = match line.kind {
            github::DiffLineKind::Addition => (tokens.success, Some(faded(tokens.success))),
            github::DiffLineKind::Deletion => (tokens.danger, Some(faded(tokens.danger))),
            github::DiffLineKind::Hunk => (tokens.accent, Some(faded(tokens.accent))),
            github::DiffLineKind::Metadata => (tokens.muted, Some(color(tokens.rail))),
            github::DiffLineKind::Context => (tokens.text, None),
        };
        let number = |value: Option<usize>| {
            div()
                .w(px(42.))
                .flex_shrink_0()
                .font_family(terminal_family(settings))
                .text_size(px(settings.ui_pixels(7.5)))
                .line_height(px(GITHUB_DIFF_LINE_HEIGHT))
                .text_color(color(tokens.faint))
                .text_right()
                .child(value.map_or_else(String::new, |line| line.to_string()))
        };
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(7.))
            .w_full()
            .h(px(visual_rows.max(1) as f32 * GITHUB_DIFF_LINE_HEIGHT))
            .px(px(8.))
            .when_some(background, |row, background| row.bg(background))
            .child(number(line.old_line))
            .child(number(line.new_line))
            .child(div().w(px(1.)).h_full().bg(color(tokens.line)))
            .child(
                div()
                    .when(wrapped, |code| code.flex_grow(1.0).min_w(px(0.)))
                    .when(!wrapped, |code| code.whitespace_nowrap())
                    .font_family(terminal_family(settings))
                    .text_size(px(settings.terminal_font_pixels()))
                    .line_height(px(GITHUB_DIFF_LINE_HEIGHT))
                    .text_color(color(foreground))
                    .child(line.text.clone()),
            )
            .into_any_element()
    }

    /// An icon, a title and a line of detail, centred in whatever is empty.
    pub(crate) fn github_centered_state(
        &self,
        kind: IconKind,
        title: &str,
        detail: &str,
        action: Option<(&'static str, Message)>,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = &self.app().settings;
        let mut content = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .p(px(12.))
                    .rounded(px(12.))
                    .bg(color(tokens.panel_raised))
                    .border_1()
                    .border_color(color(tokens.line_strong))
                    .child(
                        svg()
                            .path(crate::assets::icon_path(kind))
                            .size(px(28.))
                            .text_color(color(tokens.muted)),
                    ),
            )
            .child(
                div()
                    .text_size(px(settings.ui_pixels(13.0)))
                    .line_height((px(settings.ui_pixels(13.0))) * 1.3)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(color(tokens.text))
                    .child(title.to_owned()),
            )
            .child(
                div()
                    .w(px(280.))
                    .text_size(px(settings.ui_pixels(9.0)))
                    .line_height((px(settings.ui_pixels(9.0))) * 1.3)
                    .text_color(color(tokens.muted))
                    .text_center()
                    .child(detail.to_owned()),
            );
        if let Some((label, message)) = action {
            let mut hover = color(tokens.accent);
            hover.a = 0.86;
            content = content.child(
                div()
                    .id(SharedString::from(label))
                    .h(px(34.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .bg(color(tokens.accent))
                    .border_1()
                    .border_color(color(tokens.accent))
                    .text_size(px(settings.ui_pixels(9.0)))
                    .line_height((px(settings.ui_pixels(9.0))) * 1.3)
                    .text_color(color(tokens.app))
                    .hover(move |style| style.bg(hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(message.clone(), window, cx);
                        }),
                    )
                    .child(label),
            );
        }
        div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .p(px(24.))
            .child(content)
            .into_any_element()
    }
}
