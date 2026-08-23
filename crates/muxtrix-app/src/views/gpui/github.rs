//! The GitHub panel: local changes and pull requests.
//!
//! Docks at 372 px on wide windows and floats over the workspace on narrow
//! ones; both share this body. Everything it shows comes from
//! [`crate::app::GitHubPanelState`] — nothing here talks to the network.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, px,
};

use crate::app::{GITHUB_PANEL_WIDTH, GitHubPanelState, GitHubPanelTab, IconKind, Message};
use crate::github;
use crate::runtime::gpui::{Root, color};
use crate::theme::DesignTokens;
use crate::views::gpui::icon_button;

impl Root {
    /// The panel, when one is open.
    pub(crate) fn github_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let app = self.app();
        let panel = app.github_panel.as_ref()?;
        let tokens = DesignTokens::for_appearance(app.settings.appearance);

        let body = match panel.active_tab {
            GitHubPanelTab::Local => self.github_local(panel, tokens, cx),
            // A chosen pull request replaces the list with its own files,
            // which is where the file list lives on this tab.
            GitHubPanelTab::PullRequests if panel.selected_pull_request_number.is_some() => {
                self.github_pull_request(panel, tokens, cx)
            }
            GitHubPanelTab::PullRequests => self.github_pull_requests(panel, tokens, cx),
        };

        Some(
            div()
                .flex()
                .flex_col()
                .w(px(GITHUB_PANEL_WIDTH))
                .h_full()
                .bg(color(tokens.panel))
                .border_l(px(1.))
                .border_color(color(tokens.line))
                .child(self.github_header(panel, tokens, cx))
                .child(
                    div()
                        .id("github-body")
                        .flex_grow(1.0)
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        // The file lists — local changes, and a chosen pull
                        // request's — share one handle, because only one of
                        // them is ever on screen.
                        .track_scroll(
                            if panel.active_tab == GitHubPanelTab::PullRequests
                                && panel.selected_pull_request_number.is_none()
                            {
                                &self.scrolls.github_pull_requests
                            } else {
                                &self.scrolls.github_files
                            },
                        )
                        .child(body),
                )
                .into_any_element(),
        )
    }

    fn github_header(
        &self,
        panel: &GitHubPanelState,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let mut tabs = div().flex().flex_row().gap(px(2.));
        for (label, tab) in [
            ("Local", GitHubPanelTab::Local),
            ("Pull requests", GitHubPanelTab::PullRequests),
        ] {
            let selected = panel.active_tab == tab;
            tabs = tabs.child(
                div()
                    .id(SharedString::from(format!("github-tab-{label}")))
                    .h(px(22.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .bg(color(if selected {
                        tokens.panel_raised
                    } else {
                        tokens.panel
                    }))
                    .text_size(px(app.settings.ui_pixels(9.0)))
                    .text_color(color(if selected { tokens.text } else { tokens.faint }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::SelectGitHubPanelTab(tab), window, cx);
                        }),
                    )
                    .child(label),
            );
        }
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .h(px(43.))
            .px(px(10.))
            .bg(color(tokens.rail))
            .border_b(px(1.))
            .border_color(color(tokens.line))
            .child(tabs)
            .child(div().flex_grow(1.0))
            .child(
                icon_button(
                    gpui::ElementId::from("github-refresh"),
                    IconKind::Refresh,
                    tokens,
                    false,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::RefreshGitHubPanel, window, cx);
                    }),
                ),
            )
            .child(
                icon_button(
                    gpui::ElementId::from("github-close"),
                    IconKind::Close,
                    tokens,
                    false,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::CloseGitHubPanel, window, cx);
                    }),
                ),
            )
            .into_any_element()
    }

    /// The working tree: branch, totals, and the files that changed.
    fn github_local(
        &self,
        panel: &GitHubPanelState,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        if panel.loading || panel.context_loading {
            return self.github_notice("Loading…", tokens);
        }
        if let Some(error) = panel.error.as_deref() {
            return self.github_notice(error, tokens);
        }
        let Some(data) = panel.data.as_ref() else {
            return self.github_notice("No local changes.", tokens);
        };

        let mut files = div().flex().flex_col();
        for (index, file) in data.files.iter().enumerate() {
            let path = file.path.clone();
            files = files.child(
                div()
                    .id(("github-file", index as u64))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .h(px(42.))
                    .px(px(10.))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::OpenGitHubDiff(path.clone()), window, cx);
                        }),
                    )
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
                                    .child(file.path.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(8.0)))
                                    .text_color(color(tokens.faint))
                                    .child(file.status.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.success))
                            .child(format!("+{}", file.additions)),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.danger))
                            .child(format!("−{}", file.deletions)),
                    ),
            );
        }

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .p(px(10.))
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(11.0)))
                            .text_color(color(tokens.text))
                            .truncate()
                            .child(data.branch.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.faint))
                            .child(format!(
                                "{} file{} · +{} −{}",
                                data.files.len(),
                                if data.files.len() == 1 { "" } else { "s" },
                                data.additions,
                                data.deletions
                            )),
                    ),
            )
            .child(files)
            .into_any_element()
    }

    /// The repository's open pull requests.
    fn github_pull_requests(
        &self,
        panel: &GitHubPanelState,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        if panel.pull_requests_loading {
            return self.github_notice("Loading pull requests…", tokens);
        }
        if let Some(error) = panel.pull_requests_error.as_deref() {
            return self.github_notice(error, tokens);
        }
        let Some(requests) = panel.pull_requests.as_ref() else {
            return self.github_notice("No pull requests.", tokens);
        };

        let mut rows = div().flex().flex_col();
        for request in requests
            .iter()
            .filter(|request| request.matches(&panel.pull_request_query))
        {
            let number = request.number;
            let selected = panel.selected_pull_request_number == Some(number);
            rows = rows.child(
                div()
                    .id(("github-pr", number))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .h(px(58.))
                    .px(px(10.))
                    .cursor_pointer()
                    .bg(color(if selected {
                        tokens.panel_raised
                    } else {
                        tokens.panel
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::SelectGitHubPullRequest(number), window, cx);
                        }),
                    )
                    .child(
                        gpui::svg()
                            .path(crate::assets::icon_path(status_icon(request.status)))
                            .size(px(14.))
                            .text_color(color(status_hue(request.status, tokens))),
                    )
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
                                    .child(request.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(8.0)))
                                    .text_color(color(tokens.faint))
                                    .truncate()
                                    .child(format!("#{number} · {}", request.author)),
                            ),
                    ),
            );
        }

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(34.))
                    .m(px(8.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .bg(color(tokens.panel_raised))
                    .child(gpui_component::input::Input::new(&self.inputs.github_query)),
            )
            .child(rows)
            .into_any_element()
    }

    /// A chosen pull request: what it is, and the files it changes.
    fn github_pull_request(
        &self,
        panel: &GitHubPanelState,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        if panel.selected_pull_request_loading {
            return self.github_notice("Loading pull request…", tokens);
        }
        if let Some(error) = panel.selected_pull_request_error.as_deref() {
            return self.github_notice(error, tokens);
        }
        let Some(details) = panel.selected_pull_request.as_ref() else {
            return self.github_notice("No pull request selected.", tokens);
        };
        let request = &details.pull_request;

        let mut files = div().flex().flex_col();
        for (index, file) in details.files.iter().enumerate() {
            let path = file.path.clone();
            files = files.child(
                div()
                    .id(("pr-file", index as u64))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .h(px(42.))
                    .px(px(10.))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::OpenGitHubDiff(path.clone()), window, cx);
                        }),
                    )
                    .child(
                        div()
                            .flex_grow(1.0)
                            .min_w(px(0.))
                            .text_size(px(app.settings.ui_pixels(10.0)))
                            .text_color(color(tokens.text))
                            .truncate()
                            .child(file.path.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.success))
                            .child(format!("+{}", file.additions)),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.danger))
                            .child(format!("−{}", file.deletions)),
                    ),
            );
        }

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .p(px(10.))
                    .child(
                        icon_button(
                            gpui::ElementId::from("pr-back"),
                            IconKind::Back,
                            tokens,
                            false,
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|root, _: &MouseDownEvent, window, cx| {
                                root.dispatch(Message::CloseGitHubPullRequest, window, cx);
                            }),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_grow(1.0)
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(11.0)))
                                    .text_color(color(tokens.text))
                                    .truncate()
                                    .child(request.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(9.0)))
                                    .text_color(color(tokens.faint))
                                    .child(format!(
                                        "#{} · {} file{} · +{} −{}",
                                        request.number,
                                        request.changed_files,
                                        if request.changed_files == 1 { "" } else { "s" },
                                        request.additions,
                                        request.deletions
                                    )),
                            ),
                    ),
            )
            .child(files)
            .into_any_element()
    }

    /// One centred line, for every state that has nothing to list.
    fn github_notice(&self, message: &str, tokens: DesignTokens) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .h(px(120.))
            .px(px(16.))
            .text_size(px(self.app().settings.ui_pixels(10.0)))
            .text_color(color(tokens.muted))
            .child(message.to_owned())
            .into_any_element()
    }
}

const fn status_icon(status: github::PullRequestSummaryStatus) -> IconKind {
    match status {
        github::PullRequestSummaryStatus::Open => IconKind::PullRequestOpen,
        github::PullRequestSummaryStatus::Draft => IconKind::PullRequestDraft,
        github::PullRequestSummaryStatus::Merged => IconKind::PullRequestMerged,
    }
}

const fn status_hue(status: github::PullRequestSummaryStatus, tokens: DesignTokens) -> iced::Color {
    match status {
        github::PullRequestSummaryStatus::Open => tokens.github_open,
        github::PullRequestSummaryStatus::Merged => tokens.github_merged,
        github::PullRequestSummaryStatus::Draft => tokens.muted,
    }
}
