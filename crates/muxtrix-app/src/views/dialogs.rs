//! Modal dialogs: create, rename, worktree, close confirm.
//!
//! Every dialog is a centred card over the scrim. This module builds the card;
//! the root stacks it and owns dismissal, so escape and outside-press behave
//! the same whichever dialog is up.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Styled, div, px,
};

use crate::app::{
    DialogButton, Message, RenameTarget, SettingsButtonKind, WorktreeManagerMode,
    WorktreeManagerState, agent_display_name, ellipsize, worktree_display_name,
};
use crate::runtime::gpui::{Root, color};
use crate::theme::DesignTokens;
use crate::views::terminal_family;

impl Root {
    /// The active dialog and the message that dismisses it.
    ///
    /// One place decides precedence so two dialogs can never be open at once.
    pub(crate) fn dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let app = self.app();
        let tokens = DesignTokens::for_appearance(app.settings.appearance);

        let (dismiss, card) = if app.workspace_create_visible {
            (
                Message::CancelWorkspaceCreate,
                self.text_prompt(
                    "New workspace",
                    "Name the workspace before its first tab and terminal are created.",
                    &self.inputs.workspace_create,
                    ("Cancel", Message::CancelWorkspaceCreate),
                    ("Create", Message::CreateWorkspace),
                    tokens,
                    cx,
                ),
            )
        } else if let Some(target) = app.rename_prompt {
            let title = match target {
                RenameTarget::Workspace(_) => "Rename workspace",
                RenameTarget::Tab(_, _) => "Rename tab",
                RenameTarget::Pane(_) => "Rename pane",
            };
            (
                Message::CancelRename,
                self.text_prompt(
                    title,
                    "The new name applies immediately.",
                    &self.inputs.rename,
                    ("Cancel", Message::CancelRename),
                    ("Rename", Message::ConfirmRename),
                    tokens,
                    cx,
                ),
            )
        } else if let Some(prompt) = app.worktree_prompt.as_ref() {
            let body = prompt.failure.clone().unwrap_or_else(|| {
                prompt.base_directory.as_ref().map_or_else(
                    || "A new git worktree will be created.".to_owned(),
                    |base| format!("Created under {}", base.display()),
                )
            });
            (
                Message::CancelWorktree,
                self.text_prompt(
                    "New worktree",
                    &body,
                    &self.inputs.worktree,
                    ("Cancel", Message::CancelWorktree),
                    ("Create", Message::ConfirmWorktree),
                    tokens,
                    cx,
                ),
            )
        } else if app.default_agent_prompt {
            (
                Message::CloseDefaultAgentPrompt,
                self.confirm(
                    "No default agent",
                    "This command starts an agent, and none is configured yet.",
                    ("Not now", Message::CloseDefaultAgentPrompt),
                    ("Open settings", Message::OpenDefaultAgentSettings),
                    false,
                    tokens,
                    cx,
                ),
            )
        } else if let Some(picker) = app.session_picker.as_ref() {
            (
                Message::CloseSessionPicker,
                self.session_picker(picker, tokens, cx),
            )
        } else if let Some(manager) = app
            .worktree_manager
            .as_ref()
            .filter(|manager| manager.mode != WorktreeManagerMode::Manage)
        {
            // Choosing a worktree and confirming the restart are two steps of
            // one dialog; Esc from the second goes back to the first.
            let dismiss = if manager.restart_target.is_some() {
                Message::CancelWorktreeManagerRestart
            } else {
                Message::CloseWorktreeManager
            };
            (dismiss, self.worktree_restart_dialog(manager, tokens, cx))
        } else {
            let workspace_id = app.close_workspace_prompt?;
            let name = app
                .session
                .workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id)
                .map_or_else(|| "this workspace".to_owned(), |w| ellipsize(&w.name, 32));
            (
                Message::CancelCloseWorkspace,
                self.confirm(
                    "Close workspace",
                    &format!("{name} and every terminal in it will be closed."),
                    ("Cancel", Message::CancelCloseWorkspace),
                    ("Close", Message::ConfirmCloseWorkspace(workspace_id)),
                    true,
                    tokens,
                    cx,
                ),
            )
        };

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(color(tokens.scrim))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(dismiss.clone(), window, cx);
                    }),
                )
                .child(card)
                .into_any_element(),
        )
    }

    /// The sessions this machine still has running, offered for resuming.
    ///
    /// Shown before a new daemon is started when unattached sessions exist;
    /// declining it is what explicitly starts a fresh one.
    fn session_picker(
        &self,
        picker: &crate::app::SessionPickerState,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let mut rows = div().flex().flex_col().gap(px(2.)).max_h(px(320.));
        for (index, entry) in picker.entries.iter().enumerate() {
            let selected = index == picker.selected;
            let panes = entry.pane_count;
            rows = rows.child(
                div()
                    .id(("session", index as u64))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .h(px(38.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .bg(color(if selected {
                        tokens.panel_raised
                    } else {
                        tokens.panel
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::SessionPickerResume(index), window, cx);
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
                                    .text_size(px(app.settings.ui_pixels(11.0)))
                                    .line_height((px(app.settings.ui_pixels(11.0))) * 1.3)
                                    .text_color(color(tokens.text))
                                    .truncate()
                                    .child(entry.record.id.to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(9.0)))
                                    .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                                    .text_color(color(tokens.faint))
                                    .child(format!(
                                        "{panes} pane{}{}",
                                        if panes == 1 { "" } else { "s" },
                                        if entry.alive { "" } else { " · not running" }
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .id(("session-kill", index as u64))
                            .h(px(24.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .bg(color(tokens.panel_raised))
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                            .text_color(color(tokens.danger))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                    root.dispatch(Message::SessionPickerKill(index), window, cx);
                                }),
                            )
                            .child("Kill"),
                    ),
            );
        }
        self.card(
            "Resume a session",
            picker
                .error
                .as_deref()
                .unwrap_or("These sessions are still running. Resume one, or start fresh."),
            vec![rows.into_any_element()],
            ("Kill all", Message::SessionPickerKillAll),
            ("Start fresh", Message::CloseSessionPicker),
            false,
            tokens,
            cx,
        )
    }

    /// A dialog with a single text field.
    #[allow(clippy::too_many_arguments, reason = "a dialog is its parts")]
    fn text_prompt(
        &self,
        title: &str,
        body: &str,
        field: &gpui::Entity<gpui_component::input::InputState>,
        cancel: (&str, Message),
        confirm: (&str, Message),
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.card(
            title,
            body,
            vec![
                div()
                    .h(px(32.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .bg(color(tokens.panel))
                    .border_1()
                    .border_color(color(tokens.line))
                    .child(gpui_component::input::Input::new(field))
                    .into_any_element(),
            ],
            cancel,
            confirm,
            false,
            tokens,
            cx,
        )
    }

    /// A dialog that only asks a question.
    #[allow(clippy::too_many_arguments, reason = "a dialog is its parts")]
    fn confirm(
        &self,
        title: &str,
        body: &str,
        cancel: (&str, Message),
        confirm: (&str, Message),
        danger: bool,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.card(title, body, Vec::new(), cancel, confirm, danger, tokens, cx)
    }

    #[allow(clippy::too_many_arguments, reason = "a dialog is its parts")]
    fn card(
        &self,
        title: &str,
        body: &str,
        middle: Vec<AnyElement>,
        cancel: (&str, Message),
        confirm: (&str, Message),
        danger: bool,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        div()
            .w(px(460.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .rounded(px(10.))
            .bg(color(tokens.overlay))
            .border_1()
            .border_color(color(tokens.line))
            .shadow_lg()
            // Clicking inside must not reach the scrim's dismiss handler.
            .occlude()
            .child(
                div()
                    .text_size(px(app.settings.ui_pixels(15.0)))
                    .line_height((px(app.settings.ui_pixels(15.0))) * 1.3)
                    .text_color(color(tokens.text))
                    .child(title.to_owned()),
            )
            .child(
                div()
                    .text_size(px(app.settings.ui_pixels(10.0)))
                    .line_height((px(app.settings.ui_pixels(10.0))) * 1.3)
                    .text_color(color(tokens.muted))
                    .child(body.to_owned()),
            )
            .children(middle)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.))
                    .child(self.dialog_button(cancel.0, cancel.1, false, false, tokens, cx))
                    .child(self.dialog_button(confirm.0, confirm.1, true, danger, tokens, cx)),
            )
            .into_any_element()
    }

    fn dialog_button(
        &self,
        label: &str,
        message: Message,
        primary: bool,
        danger: bool,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let background = if danger {
            tokens.danger
        } else if primary {
            tokens.accent
        } else {
            tokens.panel
        };
        let foreground = if primary || danger {
            tokens.app
        } else {
            tokens.text
        };
        let button = if primary {
            DialogButton::Confirm
        } else {
            DialogButton::Cancel
        };
        let selected = self.app().dialog_button == Some(button);
        let border = if selected && primary {
            tokens.text
        } else if selected {
            tokens.accent
        } else {
            tokens.line
        };
        div()
            .id(gpui::ElementId::from(gpui::SharedString::from(
                label.to_owned(),
            )))
            .h(px(30.))
            .px(px(14.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .border_1()
            .border_color(color(border))
            .cursor_pointer()
            .bg(color(background))
            .text_size(px(self.app().settings.ui_pixels(11.0)))
            .text_color(color(foreground))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(message.clone(), window, cx);
                }),
            )
            .child(label.to_owned())
            .into_any_element()
    }
}

impl Root {
    /// Restart a pane in another worktree: pick the checkout, then confirm.
    fn worktree_restart_dialog(
        &self,
        manager: &WorktreeManagerState,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let settings = &app.settings;
        let ui = |points: f32| px(settings.ui_pixels(points));
        let restart_agent = match manager.mode {
            WorktreeManagerMode::RestartPaneWithAgent(_, agent) => Some(agent),
            WorktreeManagerMode::RestartPane(_) | WorktreeManagerMode::Manage => None,
        };
        let card = || {
            div()
                .w(px(600.))
                .flex()
                .flex_col()
                .gap(px(14.))
                .p(px(18.))
                .rounded(px(8.))
                .bg(color(tokens.overlay))
                .border_1()
                .border_color(color(tokens.line_strong))
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::Rgba {
                        r: 0.,
                        g: 0.,
                        b: 0.,
                        a: 0.45,
                    }
                    .into(),
                    offset: gpui::point(px(0.), px(10.)),
                    blur_radius: px(28.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                // Clicking inside must not reach the scrim's dismiss handler.
                .occlude()
        };
        let title = |copy: &'static str| {
            div()
                .text_size(ui(18.0))
                .line_height((ui(18.0)) * 1.3)
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(color(tokens.text))
                .child(copy)
        };

        // Step two: the confirmation for a chosen worktree.
        if let Some(entry) = manager
            .restart_target
            .and_then(|index| manager.entries.get(index))
        {
            let name = entry.branch.clone().unwrap_or_else(|| {
                entry
                    .path
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .unwrap_or("selected worktree")
                    .to_owned()
            });
            let question = restart_agent.map_or_else(
                || format!("Open a fresh terminal in {name}?"),
                |agent| {
                    format!(
                        "Open a fresh terminal in {name} and launch {}?",
                        agent_display_name(&agent.to_string())
                    )
                },
            );
            return card()
                .child(title(if restart_agent.is_some() {
                    "Restart pane and launch agent?"
                } else {
                    "Restart pane?"
                }))
                .child(div().text_size(ui(10.5)).line_height((ui(10.5)) * 1.3).text_color(color(tokens.text)).child(question))
                .child(
                    div()
                        .text_size(ui(9.5)).line_height((ui(9.5)) * 1.3)
                        .text_color(color(tokens.muted))
                        .child("The current process and terminal history will close. The pane stays in its present tab and position."),
                )
                .child(
                    div()
                        .w_full()
                        .py(px(9.))
                        .px(px(12.))
                        .rounded(px(7.))
                        .bg(color(tokens.panel))
                        .border_1()
                        .border_color(color(tokens.line))
                        .font_family(terminal_family(settings))
                        .text_size(ui(9.0)).line_height((ui(9.0)) * 1.3)
                        .text_color(color(tokens.text))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(entry.path.display().to_string()),
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
                                .text_size(ui(8.0)).line_height((ui(8.0)) * 1.3)
                                .text_color(color(tokens.faint))
                                .child("Enter restarts · Esc goes back"),
                        )
                        .child(
                            div()
                                .rounded(px(7.))
                                .border_1()
                                .border_color(color(
                                    if app.dialog_button == Some(DialogButton::Cancel) {
                                        tokens.accent
                                    } else {
                                        tokens.line
                                    },
                                ))
                                .child(self.settings_action_button(
                                    "worktree-restart-cancel",
                                    "Cancel",
                                    Message::CancelWorktreeManagerRestart,
                                    SettingsButtonKind::Secondary,
                                    tokens,
                                    cx,
                                )),
                        )
                        .child(
                            div()
                                .rounded(px(7.))
                                .border_1()
                                .border_color(color(
                                    if app.dialog_button == Some(DialogButton::Confirm) {
                                        tokens.text
                                    } else {
                                        tokens.line
                                    },
                                ))
                                .child(self.settings_action_button(
                                    "worktree-restart-confirm",
                                    if restart_agent.is_some() {
                                        "Restart and launch"
                                    } else {
                                        "Restart pane"
                                    },
                                    Message::ConfirmWorktreeManagerRestart,
                                    SettingsButtonKind::Danger,
                                    tokens,
                                    cx,
                                )),
                        ),
                )
                .into_any_element();
        }

        // Step one: the list.
        let mut body = card().child(title(if restart_agent.is_some() {
            "Restart pane with agent in worktree"
        } else {
            "Restart pane in worktree"
        }));
        if manager.loading {
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(11.))
                    .w_full()
                    .py(px(12.))
                    .px(px(14.))
                    .rounded(px(7.))
                    .bg(color(tokens.panel))
                    .border_1()
                    .border_color(color(tokens.line))
                    .child(
                        div()
                            .h(px(settings.ui_pixels(11.0) * 1.3))
                            .flex()
                            .items_center()
                            .child(div().size(px(7.)).rounded_full().bg(color(tokens.accent))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .flex_grow(1.0)
                            .child(
                                div()
                                    .text_size(ui(11.0)).line_height((ui(11.0)) * 1.3)
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(color(tokens.text))
                                    .child("Loading worktrees"),
                            )
                            .child(
                                div()
                                    .text_size(ui(9.5)).line_height((ui(9.5)) * 1.3)
                                    .text_color(color(tokens.muted))
                                    .child("Reading registered checkouts and local commit status in the background."),
                            )
                            .child(
                                div()
                                    .text_size(ui(8.5)).line_height((ui(8.5)) * 1.3)
                                    .text_color(color(tokens.accent))
                                    .child("The terminal remains responsive while this finishes."),
                            ),
                    ),
            );
        } else if let Some(failure) = &manager.failure {
            body = body.child(
                div()
                    .text_size(ui(10.0))
                    .line_height((ui(10.0)) * 1.3)
                    .text_color(color(tokens.danger))
                    .child(failure.clone()),
            );
        } else if manager.repo_root.is_some() {
            body = body.child(
                div()
                    .text_size(ui(10.0)).line_height((ui(10.0)) * 1.3)
                    .text_color(color(tokens.muted))
                    .child("Choose another checkout. You’ll confirm before the current terminal closes."),
            );
            if manager.entries.is_empty() {
                body = body.child(
                    div()
                        .text_size(ui(9.5))
                        .line_height((ui(9.5)) * 1.3)
                        .text_color(color(tokens.faint))
                        .child("No other worktrees are registered for this repository."),
                );
            } else {
                let pill = |label: String, hue: crate::theme::Color| {
                    let mut fill = color(hue);
                    fill.a = 0.12;
                    div()
                        .py(px(2.))
                        .px(px(8.))
                        .rounded(px(999.))
                        .bg(fill)
                        .text_size(ui(7.5))
                        .line_height((ui(7.5)) * 1.3)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(color(hue))
                        .whitespace_nowrap()
                        .child(label)
                };
                let mut list = div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .max_h(px(320.))
                    .overflow_hidden();
                for (index, entry) in manager.entries.iter().enumerate() {
                    let selected = index == manager.selected;
                    let name = worktree_display_name(&entry.path);
                    let branch = entry
                        .branch
                        .clone()
                        .unwrap_or_else(|| "Detached HEAD".to_owned());
                    let status = match &entry.used_by {
                        Some(pane) => pill(
                            format!("In use · {}", ellipsize(pane, settings.ui_char_budget(18))),
                            tokens.warning,
                        ),
                        None => pill("Idle".to_owned(), tokens.faint),
                    };
                    let push_status = (entry.unpushed_commits > 0).then(|| {
                        pill(
                            format!(
                                "{} unpushed {}",
                                entry.unpushed_commits,
                                if entry.unpushed_commits == 1 {
                                    "commit"
                                } else {
                                    "commits"
                                }
                            ),
                            tokens.warning,
                        )
                    });
                    let mut selected_fill = color(tokens.accent);
                    selected_fill.a = 0.16;
                    let mut selected_edge = color(tokens.accent);
                    selected_edge.a = 0.55;
                    list = list.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.))
                            .w_full()
                            .py(px(9.))
                            .px(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(if selected {
                                selected_edge
                            } else {
                                color(crate::theme::Color::TRANSPARENT)
                            })
                            .bg(if selected {
                                selected_fill
                            } else {
                                color(crate::theme::Color::TRANSPARENT)
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .flex_grow(1.0)
                                    .min_w(px(0.))
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(10.))
                                            .child(
                                                div()
                                                    .flex_grow(1.0)
                                                    .min_w(px(0.))
                                                    .text_size(ui(11.0))
                                                    .line_height((ui(11.0)) * 1.3)
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .text_color(color(tokens.text))
                                                    .truncate()
                                                    .child(ellipsize(
                                                        &name,
                                                        settings.ui_char_budget(34),
                                                    )),
                                            )
                                            .child(status),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(10.))
                                            .child(
                                                div()
                                                    .flex_grow(1.0)
                                                    .min_w(px(0.))
                                                    .font_family(terminal_family(settings))
                                                    .text_size(ui(8.5))
                                                    .line_height((ui(8.5)) * 1.3)
                                                    .text_color(color(tokens.faint))
                                                    .truncate()
                                                    .child(ellipsize(
                                                        &branch,
                                                        settings.ui_char_budget(34),
                                                    )),
                                            )
                                            .children(push_status),
                                    ),
                            )
                            .child(div().w(px(92.)).flex().justify_end().child(
                                self.settings_action_button(
                                    "worktree-restart-pick",
                                    "Restart",
                                    Message::WorktreeManagerRestart(index),
                                    if selected {
                                        SettingsButtonKind::Primary
                                    } else {
                                        SettingsButtonKind::Secondary
                                    },
                                    tokens,
                                    cx,
                                ),
                            )),
                    );
                }
                body = body.child(list);
            }
            if let Some(error) = &manager.error {
                body = body.child(
                    div()
                        .text_size(ui(9.0))
                        .line_height((ui(9.0)) * 1.3)
                        .text_color(color(tokens.danger))
                        .child(error.clone()),
                );
            }
        }
        let hint = if manager.failure.is_none() && !manager.entries.is_empty() {
            "↑↓ select · Enter reviews · Esc closes"
        } else {
            ""
        };
        body.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_grow(1.0)
                        .text_size(ui(8.0))
                        .line_height((ui(8.0)) * 1.3)
                        .text_color(color(tokens.faint))
                        .child(hint),
                )
                .child(self.settings_action_button(
                    "worktree-restart-close",
                    "Close",
                    Message::CloseWorktreeManager,
                    SettingsButtonKind::Secondary,
                    tokens,
                    cx,
                )),
        )
        .into_any_element()
    }
}
