//! The GitHub panel: local changes and pull requests.
//!
//! Docks at 372 px on wide windows and floats over the workspace on narrow
//! ones; both share this body. Everything it shows comes from
//! [`crate::app::GitHubPanelState`] — nothing here talks to the network.
//!
//! The layout is the iced panel's: a 54 px identity header, the Local / Pull
//! requests well, then the tab's body. Lists scroll through handles the root
//! owns and reports back, so the application's own offsets stay the record.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, px, svg,
};

use crate::app::{
    GITHUB_FILE_ROW_HEIGHT, GITHUB_LOADING_DOT_COUNT, GITHUB_PANEL_WIDTH,
    GITHUB_PULL_REQUEST_ROW_HEIGHT, GITHUB_PULL_REQUEST_SEARCH_HEIGHT,
    GITHUB_PULL_REQUEST_SUMMARY_HEIGHT, GitHubDiffSource, GitHubPanelKeyboardFocus,
    GitHubPanelState, GitHubPanelTab, IconKind, Message, github_pull_request_summary_copy,
    github_readiness_copy, github_readiness_icon, single_line_ellipsize,
};
use crate::github;
use crate::runtime::gpui::{Root, color};
use crate::theme::DesignTokens;
use crate::views::gpui::{icon_button, terminal_family};

impl Root {
    /// The panel, when one is open.
    pub(crate) fn github_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let app = self.app();
        let panel = app.github_panel.as_ref()?;
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        let floating =
            app.active_view == crate::app::ActiveView::Workspace && app.window_size.width < 1_080.0;

        let body = if panel.context_loading {
            self.github_loading(panel, tokens)
        } else {
            match panel.active_tab {
                GitHubPanelTab::Local => {
                    if panel.loading {
                        self.github_loading(panel, tokens)
                    } else if let Some(error) = panel.error.as_deref() {
                        self.github_centered_state(
                            IconKind::GitHub,
                            "Repository unavailable",
                            error,
                            Some(("Try again", Message::RefreshGitHubPanel)),
                            tokens,
                            cx,
                        )
                    } else if let Some(data) = panel.data.as_ref() {
                        self.github_local(panel, data, tokens, cx)
                    } else {
                        self.github_centered_state(
                            IconKind::GitHub,
                            "Repository unavailable",
                            "Refresh to try loading this repository again.",
                            Some(("Try again", Message::RefreshGitHubPanel)),
                            tokens,
                            cx,
                        )
                    }
                }
                GitHubPanelTab::PullRequests => match &app.github_auth {
                    github::AuthStatus::Authenticated { .. } => {
                        self.github_pull_requests(panel, tokens, cx)
                    }
                    github::AuthStatus::Checking => self.github_centered_state(
                        IconKind::GitHub,
                        "Checking GitHub…",
                        "Muxtrix is checking for an authenticated GitHub account.",
                        None,
                        tokens,
                        cx,
                    ),
                    github::AuthStatus::NeedsAuthentication => self.github_centered_state(
                        IconKind::GitHub,
                        if app.github_auth_busy {
                            "Finish in your browser"
                        } else {
                            "Connect GitHub"
                        },
                        if app.github_auth_busy {
                            "Complete the GitHub sign-in, then this panel will refresh automatically."
                        } else {
                            "Authenticate to see pull request details, merge readiness, checks, and merge controls."
                        },
                        (!app.github_auth_busy)
                            .then_some(("Authenticate with GitHub", Message::BeginGitHubAuth)),
                        tokens,
                        cx,
                    ),
                    github::AuthStatus::Unavailable { reason } => self.github_centered_state(
                        IconKind::GitHub,
                        "GitHub CLI required",
                        reason,
                        (!app.github_auth_busy)
                            .then_some(("Try connecting again", Message::BeginGitHubAuth)),
                        tokens,
                        cx,
                    ),
                },
            }
        };

        let mut surface = div()
            .flex()
            .flex_col()
            .w(px(GITHUB_PANEL_WIDTH - 1.0))
            .h_full()
            .bg(color(tokens.rail))
            .child(self.github_header(panel, tokens, cx))
            .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
            .child(self.github_tabs(panel, tokens, cx))
            .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
            .child(
                div()
                    .flex_grow(1.0)
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(body),
            );
        if floating {
            surface = surface.shadow(vec![gpui::BoxShadow {
                color: gpui::Rgba {
                    r: 0.,
                    g: 0.,
                    b: 0.,
                    a: 0.38,
                }
                .into(),
                offset: gpui::point(px(-7.), px(0.)),
                blur_radius: px(22.),
                spread_radius: px(0.),
                inset: false,
            }]);
        }
        let panel_element = div()
            .flex()
            .flex_row()
            .w(px(GITHUB_PANEL_WIDTH))
            .h_full()
            .child(div().w(px(1.)).h_full().bg(color(tokens.line_strong)))
            .child(surface);
        Some(if floating {
            // Over the workspace, hugging the right edge.
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right_0()
                .child(panel_element)
                .into_any_element()
        } else {
            panel_element.into_any_element()
        })
    }

    /// Repository identity, the current branch's pull request, and the
    /// refresh and close controls.
    fn github_header(
        &self,
        panel: &GitHubPanelState,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let ui = |points: f32| px(app.settings.ui_pixels(points));
        let repo_label = panel
            .repository
            .owner_and_name
            .clone()
            .unwrap_or_else(|| panel.repository.name.clone());
        let identity = if panel.repository.name.is_empty() && panel.context_loading {
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .flex_grow(1.0)
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(ui(10.0))
                        .line_height((ui(10.0)) * 1.3)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(color(tokens.text))
                        .child("GitHub"),
                )
                .child(
                    div()
                        .text_size(ui(8.0))
                        .line_height((ui(8.0)) * 1.3)
                        .text_color(color(tokens.faint))
                        .child("Reading focused pane…"),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .flex_grow(1.0)
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(ui(10.0))
                        .line_height((ui(10.0)) * 1.3)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(color(tokens.text))
                        .truncate()
                        .child(repo_label),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.))
                        .child(
                            svg()
                                .path(crate::assets::icon_path(IconKind::Branch))
                                .size(px(11.))
                                .flex_shrink_0()
                                .text_color(color(tokens.faint)),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .text_size(ui(8.0))
                                .line_height((ui(8.0)) * 1.3)
                                .text_color(color(tokens.faint))
                                .truncate()
                                .child(panel.repository.branch.clone()),
                        ),
                )
        };
        let current_pull_request = panel
            .data
            .as_ref()
            .and_then(|data| data.current_pull_request.as_ref())
            .map(|pull_request| {
                let hue = match pull_request.state {
                    github::CurrentPullRequestState::Open => tokens.accent,
                    github::CurrentPullRequestState::Draft => tokens.muted,
                    github::CurrentPullRequestState::Closed => tokens.faint,
                    github::CurrentPullRequestState::Merged => tokens.github_merged,
                };
                let url = pull_request.url.clone();
                let mut hover = color(tokens.text);
                hover.a = 0.04;
                div()
                    .id("github-current-pr")
                    .py(px(4.))
                    .px(px(6.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .font_family(terminal_family(&app.settings))
                    .text_size(ui(8.5))
                    .line_height((ui(8.5)) * 1.3)
                    .text_color(color(hue))
                    .hover(move |style| style.bg(hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::OpenGitHubPullRequest(url.clone()), window, cx);
                        }),
                    )
                    .child(format!("#{}", pull_request.number))
            });
        let mut actions = div().flex().flex_row().items_center().gap(px(4.));
        if !panel.active_loading() {
            actions = actions.child(
                icon_button(
                    gpui::ElementId::from("github-refresh"),
                    IconKind::Refresh,
                    tokens,
                    false,
                )
                .size(px(30.))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::RefreshGitHubPanel, window, cx);
                    }),
                ),
            );
        }
        actions = actions.child(
            icon_button(
                gpui::ElementId::from("github-close"),
                IconKind::Close,
                tokens,
                false,
            )
            .size(px(30.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::CloseGitHubPanel, window, cx);
                }),
            ),
        );
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(9.))
            .h(px(54.))
            .py(px(7.))
            .px(px(10.))
            .child(
                svg()
                    .path(crate::assets::icon_path(IconKind::GitHub))
                    .size(px(17.))
                    .flex_shrink_0()
                    .text_color(color(tokens.text)),
            )
            .child(identity)
            .children(current_pull_request)
            .child(actions)
            .into_any_element()
    }

    /// The Local / Pull requests well.
    fn github_tabs(
        &self,
        panel: &GitHubPanelState,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let mut well = div()
            .flex()
            .flex_row()
            .w_full()
            .p(px(2.))
            .rounded(px(7.))
            .bg(color(tokens.app))
            .border_1()
            .border_color(color(tokens.line));
        let busy = panel.active_loading();
        for (label, tab) in [
            ("Local", GitHubPanelTab::Local),
            ("Pull requests", GitHubPanelTab::PullRequests),
        ] {
            let selected = panel.active_tab == tab;
            let mut hover = color(tokens.text);
            hover.a = 0.05;
            let mut segment = div()
                .id(SharedString::from(format!("github-tab-{label}")))
                .flex_1()
                .h(px(28.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .border_1()
                .text_size(px(app.settings.ui_pixels(9.0)))
                .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                .whitespace_nowrap()
                .child(label);
            if !busy {
                segment = segment.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::SelectGitHubPanelTab(tab), window, cx);
                    }),
                );
            }
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
                    .border_color(color(crate::theme::Color::TRANSPARENT))
                    .text_color(color(tokens.muted))
                    .hover(move |style| style.bg(hover))
            };
            well = well.child(segment);
        }
        div().py(px(7.)).px(px(10.)).child(well).into_any_element()
    }

    /// The working tree: totals, and the files that changed.
    fn github_local(
        &self,
        panel: &GitHubPanelState,
        data: &github::PanelData,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.github_section_header(
                "LOCAL CHANGES",
                data.files.len(),
                data.additions,
                data.deletions,
                tokens,
            ))
            .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
            .child(self.github_file_list(
                panel,
                &data.files,
                &self.scrolls.github_files,
                "Working tree is clean",
                "Local file changes will appear here.",
                tokens,
                cx,
            ))
            .into_any_element()
    }

    /// A list band: "LOCAL CHANGES 4  +12 −3".
    fn github_section_header(
        &self,
        label: &'static str,
        count: usize,
        additions: usize,
        deletions: usize,
        tokens: DesignTokens,
    ) -> AnyElement {
        let settings = &self.app().settings;
        let ui = |points: f32| px(settings.ui_pixels(points));
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.))
            .h(px(38.))
            .py(px(8.))
            .px(px(12.))
            .bg(color(tokens.panel))
            .child(
                div()
                    .text_size(ui(8.0))
                    .line_height((ui(8.0)) * 1.3)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(color(tokens.faint))
                    .child(label),
            )
            .child(
                div()
                    .text_size(ui(8.0))
                    .line_height((ui(8.0)) * 1.3)
                    .text_color(color(tokens.muted))
                    .child(count.to_string()),
            )
            .child(div().flex_grow(1.0))
            .child(
                div()
                    .font_family(terminal_family(settings))
                    .text_size(ui(8.0))
                    .line_height((ui(8.0)) * 1.3)
                    .text_color(color(tokens.success))
                    .child(format!("+{additions}")),
            )
            .child(
                div()
                    .font_family(terminal_family(settings))
                    .text_size(ui(8.0))
                    .line_height((ui(8.0)) * 1.3)
                    .text_color(color(tokens.danger))
                    .child(format!("−{deletions}")),
            )
            .into_any_element()
    }

    /// The pull requests tab: the list, or a chosen pull request.
    fn github_pull_requests(
        &self,
        panel: &GitHubPanelState,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let ui = |points: f32| px(app.settings.ui_pixels(points));
        if let Some(number) = panel.selected_pull_request_number {
            let can_go_back = !panel.selected_pull_request_loading
                && !panel.merging
                && !panel.draft_state_updating;
            let focused = panel.keyboard_focus == Some(GitHubPanelKeyboardFocus::Back);
            let mut selected_fill = color(tokens.text);
            selected_fill.a = 0.07;
            let mut hover = color(tokens.text);
            hover.a = 0.04;
            let mut back = div()
                .id("github-pr-back")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(7.))
                .w_full()
                .h(px(36.))
                .px(px(12.))
                .when(focused, |back| back.bg(selected_fill))
                .when(!focused, |back| back.hover(move |style| style.bg(hover)))
                .child(
                    svg()
                        .path(crate::assets::icon_path(IconKind::Back))
                        .size(px(11.))
                        .text_color(color(tokens.muted)),
                )
                .child(
                    div()
                        .text_size(ui(8.5))
                        .line_height((ui(8.5)) * 1.3)
                        .text_color(color(tokens.muted))
                        .child("Pull requests"),
                )
                .child(div().flex_grow(1.0))
                .child(
                    div()
                        .font_family(terminal_family(&app.settings))
                        .text_size(ui(8.0))
                        .line_height((ui(8.0)) * 1.3)
                        .text_color(color(tokens.faint))
                        .child(format!("#{number}")),
                );
            if can_go_back {
                back = back.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::CloseGitHubPullRequest, window, cx);
                    }),
                );
            }
            let body = if panel.selected_pull_request_loading {
                self.github_loading(panel, tokens)
            } else if let Some(error) = panel.selected_pull_request_error.as_deref() {
                self.github_centered_state(
                    IconKind::GitHub,
                    "Pull request unavailable",
                    error,
                    Some(("Try again", Message::RefreshGitHubPanel)),
                    tokens,
                    cx,
                )
            } else if let Some(details) = panel.selected_pull_request.as_ref() {
                self.github_pull_request_details(panel, details, tokens, cx)
            } else {
                self.github_centered_state(
                    IconKind::GitHub,
                    "Pull request unavailable",
                    "Return to the list and choose this pull request again.",
                    None,
                    tokens,
                    cx,
                )
            };
            return div()
                .flex()
                .flex_col()
                .size_full()
                .child(back)
                .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
                .child(
                    div()
                        .flex_grow(1.0)
                        .min_h(px(0.))
                        .overflow_hidden()
                        .child(body),
                )
                .into_any_element();
        }

        if panel.pull_requests_loading {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .child(div().h(px(GITHUB_PULL_REQUEST_SEARCH_HEIGHT
                    + GITHUB_PULL_REQUEST_SUMMARY_HEIGHT
                    + 1.0)))
                .child(
                    div()
                        .flex_grow(1.0)
                        .min_h(px(0.))
                        .child(self.github_loading(panel, tokens)),
                )
                .into_any_element();
        }
        if let Some(error) = panel.pull_requests_error.as_deref() {
            return self.github_centered_state(
                IconKind::GitHub,
                "Pull requests unavailable",
                error,
                Some(("Try again", Message::RefreshGitHubPanel)),
                tokens,
                cx,
            );
        }
        let Some(pull_requests) = panel.pull_requests.as_ref() else {
            return self.github_centered_state(
                IconKind::GitHub,
                "Pull requests unavailable",
                "Refresh to load this repository's open pull requests.",
                Some(("Try again", Message::RefreshGitHubPanel)),
                tokens,
                cx,
            );
        };
        self.github_pull_request_list(panel, pull_requests, tokens, cx)
    }

    fn github_pull_request_list(
        &self,
        panel: &GitHubPanelState,
        pull_requests: &[github::PullRequestSummary],
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let ui = |points: f32| px(app.settings.ui_pixels(points));
        let filtered: Vec<&github::PullRequestSummary> = pull_requests
            .iter()
            .filter(|pull_request| pull_request.matches(&panel.pull_request_query))
            .collect();
        let search = div()
            .flex()
            .flex_col()
            .gap(px(5.))
            .h(px(GITHUB_PULL_REQUEST_SEARCH_HEIGHT))
            .py(px(8.))
            .px(px(10.))
            .child(
                div()
                    .text_size(ui(7.2))
                    .line_height((ui(7.2)) * 1.3)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(color(tokens.faint))
                    .child("SEARCH"),
            )
            .child(
                div()
                    .h(px(30.))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::FocusGitHubPullRequestQuery, window, cx);
                        }),
                    )
                    .child(gpui_component::input::Input::new(&self.inputs.github_query)),
            );
        let summary_label = if pull_requests
            .iter()
            .any(|pull_request| pull_request.status == github::PullRequestSummaryStatus::Merged)
        {
            "PULL REQUESTS"
        } else {
            "OPEN PULL REQUESTS"
        };
        let summary = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.))
            .h(px(GITHUB_PULL_REQUEST_SUMMARY_HEIGHT))
            .py(px(7.))
            .px(px(12.))
            .bg(color(tokens.panel))
            .child(
                div()
                    .text_size(ui(8.0))
                    .line_height((ui(8.0)) * 1.3)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(color(tokens.faint))
                    .child(summary_label),
            )
            .child(
                div()
                    .text_size(ui(8.0))
                    .line_height((ui(8.0)) * 1.3)
                    .text_color(color(tokens.muted))
                    .child(if filtered.len() == pull_requests.len() {
                        filtered.len().to_string()
                    } else {
                        format!("{} of {}", filtered.len(), pull_requests.len())
                    }),
            );
        let list = if filtered.is_empty() {
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
                cx,
            )
        } else {
            let mut rows = div().flex().flex_col();
            for (index, pull_request) in filtered.iter().enumerate() {
                rows = rows.child(self.github_pull_request_row(
                    pull_request,
                    panel.pull_request_keyboard_cursor == Some(index),
                    tokens,
                    cx,
                ));
            }
            div()
                .id("github-pull-requests")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scrolls.github_pull_requests)
                .child(rows)
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(search)
            .child(summary)
            .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
            .child(
                div()
                    .flex_grow(1.0)
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(list),
            )
            .into_any_element()
    }

    fn github_pull_request_row(
        &self,
        pull_request: &github::PullRequestSummary,
        keyboard_selected: bool,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = &self.app().settings;
        let ui = |points: f32| px(settings.ui_pixels(points));
        let (status_label, status_color) = match pull_request.status {
            github::PullRequestSummaryStatus::Open => ("Open", tokens.github_open),
            github::PullRequestSummaryStatus::Draft => ("Draft", tokens.muted),
            github::PullRequestSummaryStatus::Merged => ("Merged", tokens.github_merged),
        };
        let (_, _, readiness_color) = github_pull_request_summary_copy(pull_request, tokens);
        let number = pull_request.number;
        let mut selected_fill = color(tokens.text);
        selected_fill.a = 0.07;
        let mut hover = color(tokens.text);
        hover.a = 0.04;
        div()
            .id(("github-pr", number))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .w_full()
            .h(px(GITHUB_PULL_REQUEST_ROW_HEIGHT))
            .py(px(6.))
            .px(px(11.))
            .cursor_pointer()
            .when(keyboard_selected, |row| row.bg(selected_fill))
            .when(!keyboard_selected, |row| {
                row.hover(move |style| style.bg(hover))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::SelectGitHubPullRequest(number), window, cx);
                }),
            )
            .child(
                svg()
                    .path(crate::assets::icon_path(IconKind::GitHub))
                    .size(px(13.))
                    .flex_shrink_0()
                    .text_color(color(tokens.faint)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .flex_grow(1.0)
                    .min_w(px(0.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(7.))
                            .child(
                                div()
                                    .flex_grow(1.0)
                                    .min_w(px(0.))
                                    .text_size(ui(8.8))
                                    .line_height((ui(8.8)) * 1.3)
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(color(tokens.text))
                                    .truncate()
                                    .child(single_line_ellipsize(
                                        &pull_request.title,
                                        settings.ui_char_budget(33),
                                    )),
                            )
                            .child(
                                div()
                                    .size(px(18.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        svg()
                                            .path(crate::assets::icon_path(github_readiness_icon(
                                                pull_request.readiness,
                                            )))
                                            .size(px(14.))
                                            .text_color(color(readiness_color)),
                                    ),
                            )
                            .child(status_pill(status_label.to_owned(), status_color, settings)),
                    )
                    .child(
                        div()
                            .text_size(ui(7.4))
                            .line_height((ui(7.4)) * 1.3)
                            .text_color(color(tokens.faint))
                            .truncate()
                            .child(format!(
                                "#{} by {}  ·  {} → {}",
                                pull_request.number,
                                pull_request.author,
                                pull_request.head,
                                pull_request.base
                            )),
                    ),
            )
            .into_any_element()
    }

    fn github_pull_request_details(
        &self,
        panel: &GitHubPanelState,
        details: &github::PullRequestDetails,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.github_pull_request_card(panel, &details.pull_request, tokens, cx))
            .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
            .child(self.github_section_header(
                "CHANGED FILES",
                details.files.len(),
                details.pull_request.additions,
                details.pull_request.deletions,
                tokens,
            ))
            .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
            .child(self.github_file_list(
                panel,
                &details.files,
                &self.scrolls.github_files,
                "No changed files",
                "GitHub did not report any files for this pull request.",
                tokens,
                cx,
            ))
            .into_any_element()
    }

    /// A chosen pull request's readiness, actions, title and statistics.
    fn github_pull_request_card(
        &self,
        panel: &GitHubPanelState,
        pull_request: &github::PullRequest,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = &self.app().settings;
        let ui = |points: f32| px(settings.ui_pixels(points));
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
        let draft_focused = panel.keyboard_focus == Some(GitHubPanelKeyboardFocus::DraftAction);
        let merge_focused = panel.keyboard_focus == Some(GitHubPanelKeyboardFocus::MergeAction);
        let draft_enabled = !busy && !panel.merge_confirmation;
        let draft_action = self.github_button(
            "github-draft",
            draft_label,
            draft_enabled.then_some(Message::ToggleGitHubPullRequestDraft),
            GitHubButton::Secondary,
            draft_focused,
            tokens,
            cx,
        );
        let merge_enabled =
            readiness == github::MergeReadiness::Ready && !busy && !panel.merge_confirmation;
        let merge = self.github_button(
            "github-merge",
            if panel.merging { "Merging…" } else { "Merge" },
            merge_enabled.then_some(Message::RequestGitHubMerge),
            GitHubButton::Merge,
            merge_focused,
            tokens,
            cx,
        );
        let readiness_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(9.))
            .child(
                div()
                    .size(px(8.))
                    .rounded_full()
                    .flex_shrink_0()
                    .bg(color(readiness_color)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .flex_grow(1.0)
                    .min_w(px(0.))
                    .child(
                        div()
                            .text_size(ui(9.0))
                            .line_height((ui(9.0)) * 1.3)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(color(readiness_color))
                            .child(readiness_label),
                    )
                    .child(
                        div()
                            .text_size(ui(7.5))
                            .line_height((ui(7.5)) * 1.3)
                            .text_color(color(tokens.muted))
                            .child(readiness_detail),
                    ),
            );
        let actions = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .child(draft_action)
            .child(div().flex_grow(1.0))
            .child(merge);
        let url = pull_request.url.clone();
        let mut hover = color(tokens.text);
        hover.a = 0.04;
        let title = div()
            .id("github-pr-title")
            .flex()
            .flex_col()
            .gap(px(4.))
            .w_full()
            .rounded(px(5.))
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::OpenGitHubPullRequest(url.clone()), window, cx);
                }),
            )
            .child(
                div()
                    .text_size(ui(11.0))
                    .line_height((ui(11.0)) * 1.3)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(color(tokens.text))
                    .child(pull_request.title.clone()),
            )
            .child(
                div()
                    .text_size(ui(8.0))
                    .line_height((ui(8.0)) * 1.3)
                    .text_color(color(tokens.faint))
                    .truncate()
                    .child(format!(
                        "#{} by {}  ·  {} into {}",
                        pull_request.number,
                        pull_request.author,
                        pull_request.head,
                        pull_request.base
                    )),
            );
        let checks_color = if pull_request.checks.failed > 0 {
            tokens.danger
        } else if pull_request.checks.pending > 0 {
            tokens.warning
        } else {
            tokens.success
        };
        let stats = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .text_size(ui(8.0))
                    .line_height((ui(8.0)) * 1.3)
                    .text_color(color(tokens.muted))
                    .child(format!("{} files", pull_request.changed_files)),
            )
            .child(
                div()
                    .font_family(terminal_family(settings))
                    .text_size(ui(8.0))
                    .line_height((ui(8.0)) * 1.3)
                    .text_color(color(tokens.success))
                    .child(format!("+{}", pull_request.additions)),
            )
            .child(
                div()
                    .font_family(terminal_family(settings))
                    .text_size(ui(8.0))
                    .line_height((ui(8.0)) * 1.3)
                    .text_color(color(tokens.danger))
                    .child(format!("−{}", pull_request.deletions)),
            )
            .child(div().flex_grow(1.0))
            .child(div().size(px(6.)).rounded_full().bg(color(checks_color)))
            .child(
                div()
                    .text_size(ui(7.5))
                    .line_height((ui(7.5)) * 1.3)
                    .text_color(color(tokens.muted))
                    .child(format!(
                        "{} passed · {} pending · {} failed",
                        pull_request.checks.passed,
                        pull_request.checks.pending,
                        pull_request.checks.failed
                    )),
            );

        let mut card = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .w_full()
            .p(px(12.))
            .child(readiness_row)
            .child(actions);
        if let Some(error) = panel.pull_request_action_error.as_deref() {
            let mut fill = color(tokens.danger);
            fill.a = 0.06;
            let mut edge = color(tokens.danger);
            edge.a = 0.32;
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(7.))
                    .w_full()
                    .py(px(7.))
                    .px(px(9.))
                    .rounded(px(6.))
                    .bg(fill)
                    .border_1()
                    .border_color(edge)
                    .child(div().size(px(6.)).rounded_full().bg(color(tokens.danger)))
                    .child(
                        div()
                            .text_size(ui(8.0))
                            .line_height((ui(8.0)) * 1.3)
                            .text_color(color(tokens.danger))
                            .child(error.to_owned()),
                    ),
            );
        }
        card = card.child(title).child(stats);
        if panel.merging {
            let active_dot = (panel.loading_phase / 3) % 3;
            let mut dots = div().flex().flex_row().items_center().gap(px(4.));
            for index in 0..3 {
                let hue = if index == active_dot {
                    color(tokens.accent)
                } else if (index + 1) % 3 == active_dot {
                    let mut hue = color(tokens.accent);
                    hue.a = 0.52;
                    hue
                } else {
                    color(tokens.line_strong)
                };
                dots = dots.child(div().size(px(5.)).rounded_full().bg(hue));
            }
            let mut fill = color(tokens.accent);
            fill.a = 0.09;
            let mut edge = color(tokens.accent);
            edge.a = 0.38;
            let mut dots_edge = color(tokens.accent);
            dots_edge.a = 0.42;
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .w_full()
                    .p(px(10.))
                    .rounded(px(6.))
                    .bg(fill)
                    .border_1()
                    .border_color(edge)
                    .child(
                        div()
                            .w(px(38.))
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(7.))
                            .bg(color(tokens.panel_raised))
                            .border_1()
                            .border_color(dots_edge)
                            .child(dots),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .flex_grow(1.0)
                            .child(
                                div()
                                    .text_size(ui(9.0))
                                    .line_height((ui(9.0)) * 1.3)
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(color(tokens.accent))
                                    .child(format!(
                                        "Merging pull request #{}…",
                                        pull_request.number
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(ui(8.0))
                                    .line_height((ui(8.0)) * 1.3)
                                    .text_color(color(tokens.muted))
                                    .child(
                                        "GitHub is creating the merge commit. The branch is kept.",
                                    ),
                            ),
                    ),
            );
        }
        if panel.merge_confirmation && !panel.merging {
            let mut fill = color(tokens.success);
            fill.a = 0.07;
            let mut edge = color(tokens.success);
            edge.a = 0.35;
            card = card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .w_full()
                    .p(px(10.))
                    .rounded(px(6.))
                    .bg(fill)
                    .border_1()
                    .border_color(edge)
                    .child(
                        div()
                            .text_size(ui(9.0))
                            .line_height((ui(9.0)) * 1.3)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(color(tokens.text))
                            .child(format!("Merge pull request #{}?", pull_request.number)),
                    )
                    .child(
                        div()
                            .text_size(ui(8.0))
                            .line_height((ui(8.0)) * 1.3)
                            .text_color(color(tokens.muted))
                            .child("This creates a merge commit on GitHub. The branch is kept."),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.))
                            .child(self.github_button(
                                "github-merge-cancel",
                                "Cancel",
                                Some(Message::CancelGitHubMerge),
                                GitHubButton::Secondary,
                                false,
                                tokens,
                                cx,
                            ))
                            .child(self.github_button(
                                "github-merge-confirm",
                                "Merge pull request",
                                Some(Message::ConfirmGitHubMerge),
                                GitHubButton::Merge,
                                merge_focused,
                                tokens,
                                cx,
                            )),
                    ),
            );
        }
        card.into_any_element()
    }

    /// A panel action: the secondary settings button, or the green merge
    /// button, with the accent ring when the keyboard is on it.
    #[allow(clippy::too_many_arguments)]
    fn github_button(
        &self,
        id: &'static str,
        label: &'static str,
        message: Option<Message>,
        kind: GitHubButton,
        keyboard_selected: bool,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = &self.app().settings;
        let enabled = message.is_some();
        let (fill, hover, text, edge) = match (kind, enabled) {
            (_, false) => (
                color(tokens.panel),
                color(tokens.panel),
                tokens.faint,
                tokens.line,
            ),
            (GitHubButton::Secondary, true) => (
                color(tokens.panel),
                color(tokens.panel_raised),
                tokens.text,
                tokens.line_strong,
            ),
            (GitHubButton::Merge, true) => {
                let mut hover = color(tokens.success);
                hover.a = 0.88;
                (color(tokens.success), hover, tokens.app, tokens.success)
            }
        };
        let edge = if keyboard_selected && enabled {
            tokens.accent
        } else {
            edge
        };
        let mut button = div()
            .id(id)
            .h(px(30.))
            .px(px(12.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .bg(fill)
            .border_1()
            .border_color(color(edge))
            .text_size(px(settings.ui_pixels(8.5)))
            .line_height((px(settings.ui_pixels(8.5))) * 1.3)
            .text_color(color(text))
            .whitespace_nowrap()
            .child(label);
        if let Some(message) = message {
            button = button
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(message.clone(), window, cx);
                    }),
                );
        }
        button.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn github_file_list(
        &self,
        panel: &GitHubPanelState,
        files: &[github::FileChange],
        handle: &gpui::ScrollHandle,
        empty_title: &'static str,
        empty_detail: &'static str,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = &self.app().settings;
        if files.is_empty() {
            return div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            svg()
                                .path(crate::assets::icon_path(IconKind::File))
                                .size(px(22.))
                                .text_color(color(tokens.faint)),
                        )
                        .child(
                            div()
                                .text_size(px(settings.ui_pixels(10.0)))
                                .line_height((px(settings.ui_pixels(10.0))) * 1.3)
                                .text_color(color(tokens.text))
                                .child(empty_title),
                        )
                        .child(
                            div()
                                .text_size(px(settings.ui_pixels(8.5)))
                                .line_height((px(settings.ui_pixels(8.5))) * 1.3)
                                .text_color(color(tokens.muted))
                                .child(empty_detail),
                        ),
                )
                .into_any_element();
        }
        let mut rows = div().flex().flex_col();
        for (index, file) in files.iter().enumerate() {
            rows = rows.child(self.github_file_row(
                panel,
                file,
                panel.file_keyboard_cursor == Some(index),
                tokens,
                cx,
            ));
        }
        div()
            .id("github-files")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(handle)
            .child(rows)
            .into_any_element()
    }

    fn github_file_row(
        &self,
        panel: &GitHubPanelState,
        file: &github::FileChange,
        keyboard_selected: bool,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let settings = &app.settings;
        let ui = |points: f32| px(settings.ui_pixels(points));
        let status_color = if file.status == "Conflict" || file.status == "Deleted" {
            tokens.danger
        } else if file.status == "Untracked" || file.status == "Added" {
            tokens.success
        } else {
            tokens.muted
        };
        let selected = app.github_diff.as_ref().is_some_and(|diff| {
            let matching_source = match (panel.active_tab, diff.source) {
                (GitHubPanelTab::Local, GitHubDiffSource::Local) => true,
                (GitHubPanelTab::PullRequests, GitHubDiffSource::PullRequest(number)) => {
                    panel.selected_pull_request_number == Some(number)
                }
                _ => false,
            };
            matching_source && diff.path == file.path
        });
        let highlighted = selected || keyboard_selected;
        let path = file.path.clone();
        let mut selected_fill = color(tokens.text);
        selected_fill.a = 0.07;
        let mut hover = color(tokens.text);
        hover.a = 0.04;
        div()
            .id(SharedString::from(format!("github-file-{}", file.path)))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.))
            .w_full()
            .h(px(GITHUB_FILE_ROW_HEIGHT))
            .py(px(4.))
            .px(px(11.))
            .cursor_pointer()
            .when(highlighted, |row| row.bg(selected_fill))
            .when(!highlighted, |row| row.hover(move |style| style.bg(hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::OpenGitHubDiff(path.clone()), window, cx);
                }),
            )
            .child(
                svg()
                    .path(crate::assets::icon_path(IconKind::File))
                    .size(px(13.))
                    .flex_shrink_0()
                    .text_color(color(tokens.faint)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .flex_grow(1.0)
                    .min_w(px(0.))
                    .child(
                        div()
                            .text_size(ui(8.8))
                            .line_height((ui(8.8)) * 1.3)
                            .text_color(color(tokens.text))
                            .truncate()
                            .child(single_line_ellipsize(
                                &file.path,
                                settings.ui_char_budget(36),
                            )),
                    )
                    .child(
                        div()
                            .text_size(ui(7.2))
                            .line_height((ui(7.2)) * 1.3)
                            .text_color(color(status_color))
                            .child(file.status.clone()),
                    ),
            )
            .child(
                div()
                    .font_family(terminal_family(settings))
                    .text_size(ui(7.5))
                    .line_height((ui(7.5)) * 1.3)
                    .text_color(color(tokens.success))
                    .child(format!("+{}", file.additions)),
            )
            .child(
                div()
                    .font_family(terminal_family(settings))
                    .text_size(ui(7.5))
                    .line_height((ui(7.5)) * 1.3)
                    .text_color(color(tokens.danger))
                    .child(format!("−{}", file.deletions)),
            )
            .into_any_element()
    }

    /// The nine-dot loading indicator with the copy for what is being read.
    fn github_loading(&self, panel: &GitHubPanelState, tokens: DesignTokens) -> AnyElement {
        let settings = &self.app().settings;
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
        let mut dots = div().flex().flex_col().items_center().gap(px(5.));
        for row_index in 0..3u8 {
            let mut dot_row = div().flex().flex_row().items_center().gap(px(5.));
            for column_index in 0..3u8 {
                let index = row_index * 3 + column_index;
                let distance = (panel.loading_phase + GITHUB_LOADING_DOT_COUNT - index)
                    % GITHUB_LOADING_DOT_COUNT;
                let hue = match distance {
                    0 => color(tokens.accent),
                    1 => {
                        let mut hue = color(tokens.accent);
                        hue.a = 0.68;
                        hue
                    }
                    2 => {
                        let mut hue = color(tokens.accent);
                        hue.a = 0.38;
                        hue
                    }
                    _ => color(tokens.line_strong),
                };
                dot_row = dot_row.child(div().size(px(6.)).rounded_full().bg(hue));
            }
            dots = dots.child(dot_row);
        }
        div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .p(px(24.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .size(px(52.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(12.))
                            .bg(color(tokens.panel_raised))
                            .border_1()
                            .border_color(color(tokens.line_strong))
                            .child(dots),
                    )
                    .child(
                        div()
                            .text_size(px(settings.ui_pixels(13.0)))
                            .line_height((px(settings.ui_pixels(13.0))) * 1.3)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(color(tokens.text))
                            .child(title),
                    )
                    .child(
                        div()
                            .w(px(280.))
                            .text_size(px(settings.ui_pixels(9.0)))
                            .line_height((px(settings.ui_pixels(9.0))) * 1.3)
                            .text_color(color(tokens.muted))
                            .text_center()
                            .child(detail),
                    ),
            )
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
enum GitHubButton {
    Secondary,
    Merge,
}

/// A small rounded pill with a tinted fill, for a state.
fn status_pill(
    label: String,
    hue: crate::theme::Color,
    settings: &crate::settings::AppSettings,
) -> gpui::Div {
    let mut fill = color(hue);
    fill.a = 0.12;
    div()
        .py(px(2.))
        .px(px(8.))
        .rounded(px(999.))
        .bg(fill)
        .text_size(px(settings.ui_pixels(7.5)))
        .line_height((px(settings.ui_pixels(7.5))) * 1.3)
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(color(hue))
        .whitespace_nowrap()
        .child(label)
}
