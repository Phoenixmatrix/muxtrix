//! Modal dialogs: create, rename, worktree, close confirm.
//!
//! Every dialog is a centred card over the scrim. This module builds the card;
//! the root stacks it and owns dismissal, so escape and outside-press behave
//! the same whichever dialog is up.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Styled, div, px,
};

use crate::app::{Message, RenameTarget, ellipsize};
use crate::runtime::gpui::{Root, color};
use crate::theme::DesignTokens;

impl Root {
    /// The dialog that is up, with the message that dismisses it.
    ///
    /// One place decides, in the same order the iced shell used, so two
    /// dialogs can never be open at once.
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
        } else if app.worktree_manager.as_ref().is_some_and(restart_is_open) {
            (
                Message::CancelWorktreeManagerRestart,
                self.confirm(
                    "Restart in worktree",
                    "The pane's terminal is replaced, and anything running in it ends.",
                    ("Cancel", Message::CancelWorktreeManagerRestart),
                    ("Restart", Message::ConfirmWorktreeManagerRestart),
                    true,
                    tokens,
                    cx,
                ),
            )
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
                                    .text_color(color(tokens.text))
                                    .truncate()
                                    .child(entry.record.id.to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(9.0)))
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
                    .text_color(color(tokens.text))
                    .child(title.to_owned()),
            )
            .child(
                div()
                    .text_size(px(app.settings.ui_pixels(10.0)))
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
        div()
            .id(gpui::ElementId::from(gpui::SharedString::from(
                label.to_owned(),
            )))
            .h(px(30.))
            .px(px(14.))
            .flex()
            .items_center()
            .rounded(px(6.))
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

/// Whether a restart confirmation is standing over the worktree manager.
fn restart_is_open(manager: &crate::app::WorktreeManagerState) -> bool {
    manager.restart_target.is_some()
}
