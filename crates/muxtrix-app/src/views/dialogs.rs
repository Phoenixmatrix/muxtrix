//! Modal dialogs: create, rename, worktree, close confirm.
//!
//! Every dialog is a centred card over the scrim. This module builds the card;
//! the root stacks it and owns dismissal, so escape and outside-press behave
//! the same whichever dialog is up.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, StatefulInteractiveElement, Styled, div, px,
};

use crate::app::{
    DialogButton, Message, RenameTarget, SessionEndTarget, SettingsButtonKind, WorktreeManagerMode,
    WorktreeManagerState, agent_display_name, ellipsize, worktree_display_name,
};
use crate::runtime::gpui::{Root, color, ui_family};
use crate::theme::DesignTokens;
use crate::views::terminal_family;

impl Root {
    /// The active dialog and the message that dismisses it.
    ///
    /// One place decides precedence so two dialogs can never be open at once.
    pub(crate) fn dialog(
        &self,
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
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
                if picker.confirm_end.is_some() {
                    Message::SessionPickerCancelEnd
                } else {
                    Message::CloseSessionPicker
                },
                self.session_picker(picker, tokens, window, cx),
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
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let settings = &app.settings;
        let ui = |points: f32| px(settings.ui_pixels(points));
        // Resume owns the accent. Session identity leads a selectable inventory;
        // lifecycle cleanup stays quiet and always opens a separate confirmation.
        if let Some(target) = picker.confirm_end {
            let entry = match target {
                SessionEndTarget::One(index) => picker.entries.get(index),
                SessionEndTarget::All => None,
            };
            let stopped = entry.is_some_and(|entry| !entry.alive);
            let (title, description, action) = if stopped {
                (
                    "Remove stopped session?".to_owned(),
                    "This session is no longer running. Only its saved record will be removed.",
                    "Remove session",
                )
            } else if let Some(entry) = entry {
                (
                    format!("End {}?", entry.record.name),
                    "Its terminals and running agents will stop. This cannot be undone.",
                    "End session",
                )
            } else if picker.startup {
                (
                    "End all sessions and start fresh?".to_owned(),
                    "All listed sessions will be removed and their terminals and agents will stop. A new session will start only if cleanup succeeds. This cannot be undone.",
                    "End all & start fresh",
                )
            } else {
                (
                    "End all sessions?".to_owned(),
                    "All listed sessions will be removed. Their running terminals and agents will stop. This cannot be undone.",
                    "End all sessions",
                )
            };
            let mut middle = Vec::new();
            if let Some(error) = &picker.error {
                middle.push(
                    div()
                        .text_size(ui(10.))
                        .text_color(color(tokens.danger))
                        .child(error.clone())
                        .into_any_element(),
                );
            }
            return self.card(
                &title,
                description,
                middle,
                ("Cancel", Message::SessionPickerCancelEnd),
                (action, Message::SessionPickerConfirmEnd),
                true,
                tokens,
                cx,
            );
        }

        let empty = picker.entries.is_empty();
        let can_resume = picker
            .entries
            .get(picker.selected)
            .is_some_and(|entry| entry.alive);
        let secondary = if picker.startup {
            "Start fresh"
        } else {
            "Cancel"
        };
        let mut card = div()
            .w(px((app.window_size.width - 48.).min(620.)))
            .max_h(px(app.window_size.height - 32.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .bg(color(tokens.overlay))
            .border_1()
            .border_color(color(tokens.line_strong))
            .shadow_lg()
            .occlude()
            .child(
                div()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_size(ui(18.))
                            .line_height(ui(18.) * 1.3)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(color(tokens.text))
                            .child("Resume a session"),
                    )
                    .child(
                        div()
                            .text_size(ui(10.5))
                            .line_height(ui(10.5) * 1.4)
                            .text_color(color(tokens.muted))
                            .child(if empty && picker.error.is_some() {
                                "The session list could not be loaded."
                            } else if empty && picker.startup {
                                "Start a new session to open a fresh workspace."
                            } else if empty {
                                "There are no other sessions available on this machine."
                            } else {
                                "Pick up where you left off. Choose a session to reconnect."
                            }),
                    ),
            );
        if let Some(error) = &picker.error {
            card = card.child(
                div()
                    .mx(px(20.))
                    .mb(px(12.))
                    .text_size(ui(10.))
                    .line_height(ui(10.) * 1.4)
                    .text_color(color(tokens.danger))
                    .child(error.clone()),
            );
        }
        if empty {
            card = card.child(
                div()
                    .mx(px(20.))
                    .py(px(24.))
                    .border_t_1()
                    .border_color(color(tokens.line))
                    .text_size(ui(11.))
                    .line_height(ui(11.) * 1.4)
                    .text_color(color(tokens.muted))
                    .child(if picker.error.is_some() {
                        "Your running sessions have not been changed."
                    } else {
                        "No background sessions to resume."
                    }),
            );
        } else {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs());
            // Share content-measured columns across rows. Only session names
            // may truncate; status and action widths follow the actual UI font.
            let label_width = |label: &'static str| {
                let run = gpui::TextRun {
                    len: label.len(),
                    font: gpui::Font {
                        family: ui_family(settings),
                        weight: gpui::FontWeight(f32::from(settings.ui_font_weight.numeric())),
                        ..Default::default()
                    },
                    color: color(tokens.muted).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window
                    .text_system()
                    .shape_line(label.into(), ui(9.), &[run], None);
                px(f32::from(line.width).ceil())
            };
            let status_width = label_width("Running").max(label_width("Stopped"));
            let action_width = label_width("End session").max(label_width("Remove")) + px(24.);
            let mut rows = div()
                .id("session-list")
                .flex()
                .flex_col()
                .gap(px(2.))
                .p(px(4.))
                .max_h(px((app.window_size.height - 280.).clamp(80., 320.)))
                .overflow_y_scroll()
                .track_scroll(&self.scrolls.sessions)
                .bg(color(tokens.panel))
                .rounded(px(7.));
            for (index, entry) in picker.entries.iter().enumerate() {
                let selected = index == picker.selected;
                let mut selected_fill = color(tokens.accent);
                selected_fill.a = 0.10;
                let mut selected_edge = color(tokens.accent);
                selected_edge.a = if app.dialog_button.is_none() {
                    0.75
                } else {
                    0.3
                };
                let age = now.saturating_sub(entry.record.created_unix);
                let started = if age < 60 {
                    "Started just now".to_owned()
                } else if age < 3_600 {
                    format!("Started {}m ago", age / 60)
                } else if age < 86_400 {
                    format!("Started {}h ago", age / 3_600)
                } else {
                    format!("Started {}d ago", age / 86_400)
                };
                let short_id = format!("{:08x}", entry.record.id.as_fields().0);
                rows = rows.child(
                    div()
                        .id(("session", index as u64))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .w_full()
                        .flex_shrink_0()
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(5.))
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
                        .cursor_pointer()
                        .hover(move |style| {
                            style.bg(if selected {
                                selected_fill
                            } else {
                                color(tokens.element_hover)
                            })
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                root.dispatch(Message::SessionPickerSelect(index), window, cx);
                            }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .flex_grow(1.)
                                .flex_basis(px(0.))
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .text_size(ui(12.))
                                        .line_height(ui(12.) * 1.3)
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(color(if selected {
                                            tokens.accent
                                        } else {
                                            tokens.text
                                        }))
                                        .truncate()
                                        .child(if entry.record.name.trim().is_empty() {
                                            "Untitled session".to_owned()
                                        } else {
                                            entry.record.name.clone()
                                        }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .text_size(ui(9.))
                                        .line_height(ui(9.) * 1.3)
                                        .text_color(color(tokens.muted))
                                        .child(div().min_w(px(0.)).truncate().child(format!(
                                            "{} pane{} · {started}",
                                            entry.pane_count,
                                            if entry.pane_count == 1 { "" } else { "s" },
                                        )))
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .font_family(terminal_family(settings))
                                                .text_size(ui(8.))
                                                .text_color(color(tokens.faint))
                                                .child(short_id),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(status_width)
                                .whitespace_nowrap()
                                .text_size(ui(9.))
                                .text_color(color(tokens.muted))
                                .child(if entry.alive { "Running" } else { "Stopped" }),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(action_width)
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(self.settings_action_button(
                                    "session-end",
                                    if entry.alive { "End session" } else { "Remove" },
                                    Message::SessionPickerRequestEnd(SessionEndTarget::One(index)),
                                    SettingsButtonKind::Quiet,
                                    tokens,
                                    cx,
                                )),
                        ),
                );
            }
            card = card.child(div().mx(px(16.)).mb(px(16.)).child(rows));
        }
        let mut actions = div().flex().items_center().justify_end().gap(px(8.));
        if picker.entries.len() > 1 || (picker.startup && !empty) {
            actions = actions.child(
                div()
                    .flex()
                    .flex_grow(1.)
                    .child(self.settings_action_button(
                        "session-end-all",
                        if picker.startup {
                            "End all & start fresh"
                        } else {
                            "End all sessions"
                        },
                        Message::SessionPickerRequestEnd(SessionEndTarget::All),
                        SettingsButtonKind::Secondary,
                        tokens,
                        cx,
                    )),
            );
        } else {
            actions = actions.child(div().flex_grow(1.));
        }
        if empty {
            actions = actions.child(self.dialog_button(
                if picker.startup {
                    "Start fresh"
                } else {
                    "Done"
                },
                Message::CloseSessionPicker,
                true,
                false,
                tokens,
                cx,
            ));
        } else {
            actions = actions.child(self.dialog_button(
                secondary,
                Message::CloseSessionPicker,
                false,
                false,
                tokens,
                cx,
            ));
            if can_resume {
                actions = actions.child(self.dialog_button(
                    "Resume session",
                    Message::SessionPickerResume(picker.selected),
                    true,
                    false,
                    tokens,
                    cx,
                ));
            } else {
                actions = actions.child(
                    div()
                        .h(px(30.))
                        .px(px(14.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(color(tokens.line))
                        .bg(color(tokens.panel_raised))
                        .text_size(ui(11.))
                        .text_color(color(tokens.faint))
                        .child("Resume session"),
                );
            }
        }
        let mut footer = div()
            .p(px(16.))
            .border_t_1()
            .border_color(color(tokens.line))
            .flex()
            .flex_col()
            .gap(px(12.));
        if !empty {
            let key = |label: &'static str| {
                div()
                    .px(px(5.))
                    .py(px(1.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(color(tokens.line_strong))
                    .font_family(terminal_family(settings))
                    .text_size(ui(8.))
                    .line_height(ui(8.) * 1.3)
                    .text_color(color(tokens.muted))
                    .child(label)
            };
            footer = footer.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .text_size(ui(9.))
                    .text_color(color(tokens.muted))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(key("↑ ↓"))
                            .child("Select"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(key("Enter"))
                            .child("Resume"),
                    )
                    .child(div().flex_grow(1.))
                    .child(div().text_size(ui(9.)).child(if picker.startup {
                        "Start fresh keeps sessions running"
                    } else {
                        "Other sessions keep running"
                    })),
            );
        }
        card.child(footer.child(actions)).into_any_element()
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
