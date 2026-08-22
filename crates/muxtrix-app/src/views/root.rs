//! The application shell: what wraps every screen.
//!
//! [`Muxtrix::view`] picks the active screen, docks or floats the GitHub
//! panel, and layers the scrim, the modal dialog, the palette and the toast
//! above it. Screen bodies live in the sibling modules.

use crate::views::prelude::*;

use crate::app::{ActiveView, WorktreeManagerMode};

impl Muxtrix {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let content = match self.active_view {
            ActiveView::Workspace => self.workspace_view(),
            ActiveView::Settings => self.settings_view(),
            ActiveView::ThemeGallery => self.theme_gallery_view(),
            ActiveView::GitHubDiff => self.github_diff_view(tokens),
        };
        let shell: Element<'_, Message> = if self.active_view == ActiveView::GitHubDiff {
            let panel = self.github_side_panel_view(tokens, false);
            container(row![content, panel].height(Fill).width(Fill))
                .style(move |_| container::Style::default().background(tokens.app))
                .into()
        } else if self.active_view == ActiveView::Workspace {
            let workspace_shell = row![self.sidebar(), content].height(Fill).width(Fill);
            if self.github_panel_visible() && self.window_size.width >= 1_080.0 {
                container(workspace_shell.push(self.github_side_panel_view(tokens, false)))
                    .style(move |_| container::Style::default().background(tokens.app))
                    .into()
            } else if self.github_panel_visible() {
                let base: Element<'_, Message> = container(workspace_shell)
                    .style(move |_| container::Style::default().background(tokens.app))
                    .into();
                let panel = container(self.github_side_panel_view(tokens, true))
                    .width(Fill)
                    .height(Fill)
                    .align_x(iced::alignment::Horizontal::Right);
                stack([base, panel.into()]).into()
            } else {
                container(workspace_shell)
                    .style(move |_| container::Style::default().background(tokens.app))
                    .into()
            }
        } else {
            container(content)
                .style(move |_| container::Style::default().background(tokens.app))
                .into()
        };

        let overlay = if self.workspace_create_visible {
            Some((
                Message::CancelWorkspaceCreate,
                container(opaque(self.workspace_create_dialog()))
                    .center_x(Fill)
                    .center_y(Fill),
            ))
        } else if self.rename_prompt.is_some() {
            Some((
                Message::CancelRename,
                container(opaque(self.rename_dialog()))
                    .center_x(Fill)
                    .center_y(Fill),
            ))
        } else if self.default_agent_prompt {
            Some((
                Message::CloseDefaultAgentPrompt,
                container(opaque(self.default_agent_dialog()))
                    .center_x(Fill)
                    .center_y(Fill),
            ))
        } else if self.session_picker.is_some() {
            Some((
                Message::CloseSessionPicker,
                container(opaque(self.session_picker_dialog()))
                    .center_x(Fill)
                    .center_y(Fill),
            ))
        } else if self.worktree_manager.as_ref().is_some_and(|manager| {
            matches!(
                manager.mode,
                WorktreeManagerMode::RestartPane(_)
                    | WorktreeManagerMode::RestartPaneWithAgent(_, _)
            )
        }) {
            Some((
                Message::CloseWorktreeManager,
                container(opaque(self.worktree_restart_dialog()))
                    .center_x(Fill)
                    .center_y(Fill),
            ))
        } else if self.worktree_prompt.is_some() {
            Some((
                Message::CancelWorktree,
                container(opaque(self.worktree_dialog()))
                    .center_x(Fill)
                    .center_y(Fill),
            ))
        } else if let Some(workspace_id) = self.close_workspace_prompt {
            Some((
                Message::CancelCloseWorkspace,
                container(opaque(self.close_workspace_dialog(workspace_id)))
                    .center_x(Fill)
                    .center_y(Fill),
            ))
        } else if self.palette.visible {
            Some((
                Message::CloseCommandPalette,
                container(opaque(self.command_palette()))
                    .center_x(Fill)
                    .align_y(iced::alignment::Vertical::Top)
                    .padding([72, 0]),
            ))
        } else {
            None
        };
        let base: Element<'_, Message> = if let Some((dismiss_message, overlay)) = overlay {
            let dismiss = mouse_area(
                container("")
                    .width(Fill)
                    .height(Fill)
                    .style(move |_| container::Style::default().background(tokens.scrim)),
            )
            .on_press(dismiss_message);
            stack([shell, dismiss.into(), overlay.into()]).into()
        } else {
            shell
        };
        let Some((message, keyboard_mode)) = self.feedback_message() else {
            return base;
        };
        // A live keyboard mode is a state the user is standing in until they
        // leave it, so it is drawn in the accent the rest of the chrome already
        // uses for "this is where you are". A toast keeps the quiet neutral
        // surface: it leaves on its own and must not pull the eye off the
        // terminal to do it.
        let border_color = if keyboard_mode {
            tokens.accent
        } else {
            tokens.line_strong
        };
        // Plain text and containers never capture pointer events, so the
        // pill cannot steal clicks from the pane beneath it.
        let pill = container(text(message).size(self.settings.ui_pixels(9.5)).color(
            if keyboard_mode {
                tokens.accent
            } else {
                tokens.text
            },
        ))
        .padding([7, 14])
        .style(move |_| container::Style {
            background: Some(tokens.overlay.into()),
            border: iced::Border {
                color: border_color,
                width: if keyboard_mode { 1.5 } else { 1.0 },
                radius: 999.0.into(),
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 16.0,
            },
            ..container::Style::default()
        });
        stack([
            base,
            container(pill)
                .center_x(Fill)
                .align_y(iced::alignment::Vertical::Bottom)
                .height(Fill)
                .padding(iced::Padding {
                    top: 0.0,
                    right: 0.0,
                    bottom: 26.0,
                    left: 0.0,
                })
                .into(),
        ])
        .into()
    }

    pub(crate) fn github_panel_visible(&self) -> bool {
        self.github_panel.is_some()
    }
}
