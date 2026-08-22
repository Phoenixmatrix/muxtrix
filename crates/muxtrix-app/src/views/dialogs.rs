//! Modal dialogs: create, rename, worktree, session picker, close confirm.
//!
//! Every dialog is a centred card over the scrim. The caller stacks it; these
//! functions only build the card, so dismissal and focus stay in one place in
//! [`crate::Muxtrix::view`].

use iced::widget::column;

use crate::views::prelude::*;

use crate::{
    RENAME_INPUT_ID, RenameTarget, SettingsButtonKind, WORKSPACE_CREATE_INPUT_ID,
    WORKTREE_INPUT_ID, WorktreeManagerMode, WorktreePromptTarget, age_label, agent_display_name,
    centered_button_label, commands, ellipsize, ellipsize_start, modal_surface,
    settings_action_button, settings_button_style, settings_notice, status_pill,
    worktree_display_name, worktree_name,
};
use iced::Font;
use muxtrix_control::Agent;
use muxtrix_domain::{SplitAxis, WorkspaceId};

impl Muxtrix {
    pub(crate) fn workspace_create_dialog(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        container(
            column![
                text("New workspace")
                    .size(self.settings.ui_pixels(18.0))
                    .font(Font {
                        weight: font::Weight::Bold,
                        ..Font::DEFAULT
                    }),
                text("Name the workspace before its first tab and terminal are created.")
                    .size(self.settings.ui_pixels(10.0))
                    .color(tokens.muted),
                text_input("Workspace name", &self.workspace_name_draft)
                    .id(iced::widget::Id::new(WORKSPACE_CREATE_INPUT_ID))
                    .on_input(Message::WorkspaceNameChanged)
                    .on_submit(Message::CreateWorkspace)
                    .padding(10)
                    .size(self.settings.ui_pixels(11.0)),
                row![
                    container("").width(Fill),
                    settings_action_button(
                        "Cancel",
                        Message::CancelWorkspaceCreate,
                        SettingsButtonKind::Secondary,
                        &self.settings,
                    ),
                    settings_action_button(
                        "Create workspace",
                        Message::CreateWorkspace,
                        SettingsButtonKind::Primary,
                        &self.settings,
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(14),
        )
        .padding(18)
        .width(460)
        .style(move |_| modal_surface(tokens))
        .into()
    }

    pub(crate) fn default_agent_dialog(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let configured_agents = Agent::ALL
            .into_iter()
            .filter(|agent| self.agent_is_configured(*agent))
            .count();
        let (title, body, action_label) = match self.settings.default_agent {
            Some(agent) if !self.agent_is_configured(agent) => {
                let name = agent_display_name(&agent.to_string()).to_owned();
                (
                    "Repair your default agent",
                    format!(
                        "{name} is selected, but its lifecycle hooks or launch command are not ready. Repair that integration or choose another default, then apply to continue your worktree command."
                    ),
                    "Repair agent settings",
                )
            }
            _ if configured_agents > 0 => (
                "Choose an agent for worktrees",
                "One or more agent integrations are ready. Choose which agent Muxtrix should start by default, then apply to continue your worktree command."
                    .to_owned(),
                "Choose default in Settings",
            ),
            _ => (
                "Set up a worktree agent",
                "Before Muxtrix can start an agent in a worktree, add its lifecycle integration and choose it as the default. Apply when ready to continue your worktree command."
                    .to_owned(),
                "Configure an agent",
            ),
        };
        container(
            column![
                text(title).size(self.settings.ui_pixels(18.0)).font(Font {
                    weight: font::Weight::Bold,
                    ..Font::DEFAULT
                }),
                text(body)
                    .size(self.settings.ui_pixels(10.0))
                    .color(tokens.muted)
                    .width(420),
                row![
                    container("").width(Fill),
                    settings_action_button(
                        "Cancel command",
                        Message::CloseDefaultAgentPrompt,
                        SettingsButtonKind::Secondary,
                        &self.settings,
                    ),
                    settings_action_button(
                        action_label,
                        Message::OpenDefaultAgentSettings,
                        SettingsButtonKind::Primary,
                        &self.settings,
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(14),
        )
        .padding(20)
        .width(480)
        .style(move |_| modal_surface(tokens))
        .into()
    }

    pub(crate) fn rename_dialog(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let (title, hint) = match self.rename_prompt {
            Some(RenameTarget::Workspace(_)) => ("Rename workspace", "The new workspace name."),
            Some(RenameTarget::Tab(..)) => ("Rename tab", "The new tab name."),
            Some(RenameTarget::Pane(_)) => (
                "Rename pane",
                "A custom pane name. Leave it empty to restore the automatic title.",
            ),
            None => ("Rename", ""),
        };
        container(
            column![
                text(title).size(self.settings.ui_pixels(18.0)).font(Font {
                    weight: font::Weight::Bold,
                    ..Font::DEFAULT
                }),
                text(hint)
                    .size(self.settings.ui_pixels(10.0))
                    .color(tokens.muted),
                text_input("Name", &self.rename_draft)
                    .id(iced::widget::Id::new(RENAME_INPUT_ID))
                    .on_input(Message::RenameDraftChanged)
                    .on_submit(Message::ConfirmRename)
                    .padding(10)
                    .size(self.settings.ui_pixels(11.0)),
                row![
                    container("").width(Fill),
                    settings_action_button(
                        "Cancel",
                        Message::CancelRename,
                        SettingsButtonKind::Secondary,
                        &self.settings,
                    ),
                    settings_action_button(
                        "Rename",
                        Message::ConfirmRename,
                        SettingsButtonKind::Primary,
                        &self.settings,
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(14),
        )
        .padding(18)
        .width(460)
        .style(move |_| modal_surface(tokens))
        .into()
    }

    pub(crate) fn worktree_dialog(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let Some(prompt) = &self.worktree_prompt else {
            return container("").into();
        };
        let title = if prompt.repo_root.is_none() {
            // The dialog exists to explain a failure; titling it with the
            // action it cannot perform poses a false promise.
            "Can't create a worktree"
        } else {
            match prompt.target {
                WorktreePromptTarget::Open(commands::WorktreeKind::Pane(SplitAxis::Horizontal)) => {
                    "New worktree pane right"
                }
                WorktreePromptTarget::OpenWithAgent(
                    commands::WorktreeKind::Pane(SplitAxis::Horizontal),
                    _,
                ) => "New worktree with agent pane right",
                WorktreePromptTarget::Open(commands::WorktreeKind::Pane(SplitAxis::Vertical)) => {
                    "New worktree pane down"
                }
                WorktreePromptTarget::OpenWithAgent(
                    commands::WorktreeKind::Pane(SplitAxis::Vertical),
                    _,
                ) => "New worktree with agent pane down",
                WorktreePromptTarget::Open(commands::WorktreeKind::Tab) => "New worktree tab",
                WorktreePromptTarget::OpenWithAgent(commands::WorktreeKind::Tab, _) => {
                    "New worktree tab with agent"
                }
                WorktreePromptTarget::RestartPane(_) => "Restart in new worktree",
                WorktreePromptTarget::RestartPaneWithAgent(_, _) => {
                    "Restart with agent in new worktree"
                }
            }
        };
        let mut body = column![text(title).size(self.settings.ui_pixels(18.0)).font(Font {
            weight: font::Weight::Bold,
            ..Font::DEFAULT
        })]
        .spacing(14);
        let Some(repo_root) = &prompt.repo_root else {
            // Still a dialog, not an invisible status line: the operation
            // cannot run and the user needs to see why.
            body = body
                .push(
                    text(prompt.failure.clone().unwrap_or_else(|| {
                        "The focused pane is not inside a git repository, so a worktree cannot be created from it.".to_owned()
                    }))
                    .size(self.settings.ui_pixels(10.0))
                    .color(tokens.muted),
                )
                .push(
                    row![
                        container("").width(Fill),
                        settings_action_button(
                            "Close",
                            Message::CancelWorktree,
                            SettingsButtonKind::Secondary,
                            &self.settings,
                        ),
                    ]
                    .align_y(Alignment::Center),
                );
            return container(body)
                .padding(18)
                .width(460)
                .style(move |_| modal_surface(tokens))
                .into();
        };
        // Live conflict check so the collision is visible before submitting.
        // Names were listed when the dialog opened; no filesystem probing
        // per keystroke (through WSL that would launch wsl.exe every frame).
        let name = worktree_name(&self.worktree_name_draft);
        let conflict = !name.is_empty() && prompt.taken_names.contains(&name);
        let inline_error = prompt
            .error
            .clone()
            .or_else(|| conflict.then(|| format!("{name} already exists for this repository")));
        let can_create = !prompt.busy && !name.is_empty() && !conflict;
        let confirm_label = if prompt.busy {
            "Creating…"
        } else {
            "Create worktree"
        };
        let mut confirm = button(centered_button_label(
            confirm_label,
            self.settings.ui_pixels(9.0),
        ))
        .height(30)
        .padding([0, 11])
        .style({
            let settings_tokens = tokens;
            move |_, status| {
                settings_button_style(settings_tokens, SettingsButtonKind::Primary, status)
            }
        });
        if can_create {
            confirm = confirm.on_press(Message::ConfirmWorktree);
        }
        let worktree_path_size = self.settings.ui_pixels(9.0);
        let worktree_path_prefix = "Created under ";
        let created_under = prompt
            .base_directory
            .as_deref()
            .map_or_else(String::new, |base| {
                let line_budget = (424.0 / (worktree_path_size * 0.62)) as usize;
                let path_budget = line_budget.saturating_sub(worktree_path_prefix.chars().count());
                format!(
                    "{worktree_path_prefix}{}",
                    ellipsize_start(&base.display().to_string(), path_budget)
                )
            });
        body = body
            .push(
                text(format!("Branch and worktree from {}", repo_root.display()))
                    .size(self.settings.ui_pixels(10.0))
                    .color(tokens.muted),
            )
            .push(
                text(created_under)
                    .width(Fill)
                    .size(worktree_path_size)
                    .color(tokens.faint)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .push(
                text_input("Worktree name", &self.worktree_name_draft)
                    .id(iced::widget::Id::new(WORKTREE_INPUT_ID))
                    .on_input(Message::WorktreeNameChanged)
                    .on_submit(Message::ConfirmWorktree)
                    .padding(10)
                    .size(self.settings.ui_pixels(11.0)),
            );
        if let Some(error) = inline_error {
            body = body.push(
                text(error)
                    .size(self.settings.ui_pixels(9.0))
                    .color(tokens.danger),
            );
        }
        body = body.push(
            row![
                text("Enter creates · Esc cancels")
                    .size(self.settings.ui_pixels(8.0))
                    .color(tokens.faint),
                container("").width(Fill),
                settings_action_button(
                    "Cancel",
                    Message::CancelWorktree,
                    SettingsButtonKind::Secondary,
                    &self.settings,
                ),
                confirm,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
        container(body)
            .padding(18)
            .width(460)
            .style(move |_| modal_surface(tokens))
            .into()
    }

    pub(crate) fn session_picker_dialog(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let Some(picker) = &self.session_picker else {
            return container("").into();
        };
        let title = if picker.startup {
            "Resume a session?"
        } else {
            "Sessions"
        };
        let mut body = column![text(title).size(self.settings.ui_pixels(18.0)).font(Font {
            weight: font::Weight::Bold,
            ..Font::DEFAULT
        })]
        .spacing(14);
        if picker.startup {
            body = body.push(
                text("Background sessions are still running. Attach to one, or start fresh.")
                    .size(self.settings.ui_pixels(10.0))
                    .color(tokens.muted),
            );
        }
        if picker.entries.is_empty() {
            body = body.push(
                text(
                    "No background sessions. Close a window and its running session appears here.",
                )
                .size(self.settings.ui_pixels(9.5))
                .color(tokens.faint),
            );
        } else {
            let mut list = column![].spacing(2);
            for (index, entry) in picker.entries.iter().enumerate() {
                let selected = index == picker.selected;
                let age = age_label(entry.record.created_unix);
                let status = if entry.alive {
                    status_pill("Running", tokens.success, &self.settings)
                } else {
                    status_pill("Dead", tokens.faint, &self.settings)
                };
                // Dead sessions cannot be resumed; offering a primary
                // button that silently no-ops would be a lie. They get
                // Remove and nothing else.
                let resume: Element<'_, Message> = if entry.alive {
                    button(centered_button_label(
                        "Resume",
                        self.settings.ui_pixels(9.0),
                    ))
                    .height(28)
                    .padding([0, 14])
                    .style({
                        let settings_tokens = tokens;
                        move |_, status| {
                            settings_button_style(
                                settings_tokens,
                                SettingsButtonKind::Primary,
                                status,
                            )
                        }
                    })
                    .on_press(Message::SessionPickerResume(index))
                    .into()
                } else {
                    container("").width(0).into()
                };
                let kill = button(centered_button_label(
                    if entry.alive { "Kill" } else { "Remove" },
                    self.settings.ui_pixels(9.0),
                ))
                .height(28)
                .padding([0, 14])
                .style({
                    let settings_tokens = tokens;
                    move |_, status| {
                        settings_button_style(settings_tokens, SettingsButtonKind::Danger, status)
                    }
                })
                .on_press(Message::SessionPickerKill(index));
                let details = column![
                    row![
                        text(entry.record.name.clone())
                            .size(self.settings.ui_pixels(11.0))
                            .font(Font {
                                weight: self.default_family_weight(FontWeight::Medium),
                                ..Font::DEFAULT
                            })
                            .color(tokens.text)
                            .wrapping(iced::widget::text::Wrapping::None),
                        status,
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                    text(format!(
                        "{} pane{} · started {age}{}",
                        entry.pane_count,
                        if entry.pane_count == 1 { "" } else { "s" },
                        // A daemon can outlive app updates; surface skew so
                        // "kill and start fresh" is an informed choice.
                        if entry.record.version.is_empty()
                            || entry.record.version == env!("CARGO_PKG_VERSION")
                        {
                            String::new()
                        } else {
                            format!(" · daemon v{}", entry.record.version)
                        }
                    ))
                    .font(self.settings.terminal_font.iced())
                    .size(self.settings.ui_pixels(8.5))
                    .color(tokens.faint)
                    .wrapping(iced::widget::text::Wrapping::None),
                ]
                .spacing(3)
                .width(Fill);
                list = list.push(
                    container(
                        row![details, resume, kill]
                            .spacing(8)
                            .align_y(Alignment::Center),
                    )
                    .padding([9, 12])
                    .width(Fill)
                    .style(move |_| iced::widget::container::Style {
                        // Accent tint like the command palette: the row Enter
                        // and Del act on must clear non-text contrast, which
                        // panel_raised (1.07:1 on the modal) never did.
                        background: selected.then(|| {
                            Color {
                                a: 0.16,
                                ..tokens.accent
                            }
                            .into()
                        }),
                        border: Border {
                            color: if selected {
                                Color {
                                    a: 0.55,
                                    ..tokens.accent
                                }
                            } else {
                                Color::TRANSPARENT
                            },
                            width: 1.0,
                            radius: 8.0.into(),
                        },
                        ..iced::widget::container::Style::default()
                    }),
                );
            }
            body = body.push(scrollable(list).height(iced::Length::Shrink));
        }
        if let Some(error) = &picker.error {
            body = body.push(
                text(error.clone())
                    .size(self.settings.ui_pixels(9.0))
                    .color(tokens.danger),
            );
        }
        let mut footer = row![].spacing(8).align_y(Alignment::Center);
        if !picker.entries.is_empty() {
            footer = footer.push(
                text("↑↓ select · Enter resumes · Del kills · Esc closes")
                    .size(self.settings.ui_pixels(8.0))
                    .color(tokens.faint),
            );
        }
        if !picker.entries.is_empty() {
            footer = footer.push(settings_action_button(
                "Kill all",
                Message::SessionPickerKillAll,
                SettingsButtonKind::Danger,
                &self.settings,
            ));
        }
        footer = footer.push(container("").width(Fill));
        // The selected row's Resume owns the accent; declining the dialog's
        // question is never the primary action.
        footer = footer.push(settings_action_button(
            if picker.startup {
                "Start new session"
            } else {
                "Close"
            },
            Message::CloseSessionPicker,
            SettingsButtonKind::Secondary,
            &self.settings,
        ));
        body = body.push(footer);
        container(body)
            .padding(18)
            .width(600)
            .style(move |_| modal_surface(tokens))
            .into()
    }

    pub(crate) fn worktree_restart_dialog(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let Some(manager) = &self.worktree_manager else {
            return container("").into();
        };
        let restart_agent = match manager.mode {
            WorktreeManagerMode::RestartPaneWithAgent(_, agent) => Some(agent),
            WorktreeManagerMode::RestartPane(_) => None,
            WorktreeManagerMode::Manage => return container("").into(),
        };
        if let Some(entry) = manager
            .restart_target
            .and_then(|index| manager.entries.get(index))
        {
            let name = entry.branch.as_deref().unwrap_or_else(|| {
                entry
                    .path
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .unwrap_or("selected worktree")
            });
            let body = column![
                text(if restart_agent.is_some() {
                    "Restart pane and launch agent?"
                } else {
                    "Restart pane?"
                })
                .size(self.settings.ui_pixels(18.0))
                .font(Font {
                    weight: font::Weight::Bold,
                    ..Font::DEFAULT
                }),
                text(restart_agent.map_or_else(
                    || format!("Open a fresh terminal in {name}?"),
                    |agent| format!(
                        "Open a fresh terminal in {name} and launch {}?",
                        agent_display_name(&agent.to_string())
                    ),
                ))
                .size(self.settings.ui_pixels(10.5))
                .color(tokens.text),
                text("The current process and terminal history will close. The pane stays in its present tab and position.")
                    .size(self.settings.ui_pixels(9.5))
                    .color(tokens.muted),
                container(
                    text(entry.path.display().to_string())
                        .font(self.settings.terminal_font.iced())
                        .size(self.settings.ui_pixels(9.0))
                        .color(tokens.text)
                        .wrapping(iced::widget::text::Wrapping::None),
                )
                .padding([9, 12])
                .width(Fill)
                .style(move |_| container::Style::default()
                    .background(tokens.panel)
                    .border(Border {
                        color: tokens.line,
                        width: 1.0,
                        radius: 7.0.into(),
                    })),
                row![
                    text("Enter restarts · Esc goes back")
                        .size(self.settings.ui_pixels(8.0))
                        .color(tokens.faint),
                    container("").width(Fill),
                    settings_action_button(
                        "Cancel",
                        Message::CancelWorktreeManagerRestart,
                        SettingsButtonKind::Secondary,
                        &self.settings,
                    ),
                    settings_action_button(
                        if restart_agent.is_some() {
                            "Restart and launch"
                        } else {
                            "Restart pane"
                        },
                        Message::ConfirmWorktreeManagerRestart,
                        SettingsButtonKind::Danger,
                        &self.settings,
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(14);
            return container(body)
                .padding(18)
                .width(600)
                .style(move |_| modal_surface(tokens))
                .into();
        }

        let title = text(if restart_agent.is_some() {
            "Restart pane with agent in worktree"
        } else {
            "Restart pane in worktree"
        })
        .size(self.settings.ui_pixels(18.0))
        .font(Font {
            weight: font::Weight::Bold,
            ..Font::DEFAULT
        });
        let header: Element<'_, Message> = title.into();
        let mut body = column![header].spacing(14);
        if manager.loading {
            body = body.push(settings_notice(
                "Loading worktrees",
                "Reading registered checkouts and local commit status in the background.",
                "The terminal remains responsive while this finishes.",
                tokens.accent,
                &self.settings,
            ));
        } else if let Some(failure) = &manager.failure {
            body = body.push(
                text(failure.clone())
                    .size(self.settings.ui_pixels(10.0))
                    .color(tokens.danger),
            );
        } else if manager.repo_root.is_some() {
            body = body.push(
                text("Choose another checkout. You’ll confirm before the current terminal closes.")
                    .size(self.settings.ui_pixels(10.0))
                    .color(tokens.muted),
            );
            if manager.entries.is_empty() {
                body = body.push(
                    text("No other worktrees are registered for this repository.")
                        .size(self.settings.ui_pixels(9.5))
                        .color(tokens.faint),
                );
            } else {
                let mut list = column![].spacing(2);
                for (index, entry) in manager.entries.iter().enumerate() {
                    let selected = index == manager.selected;
                    let name = worktree_display_name(&entry.path);
                    let branch = entry
                        .branch
                        .as_deref()
                        .map_or_else(|| "Detached HEAD".to_owned(), |branch| branch.to_owned());
                    let status = match &entry.used_by {
                        Some(pane) => status_pill(
                            &format!(
                                "In use · {}",
                                ellipsize(pane, self.settings.ui_char_budget(18))
                            ),
                            tokens.warning,
                            &self.settings,
                        ),
                        None => status_pill("Idle", tokens.faint, &self.settings),
                    };
                    let action = settings_action_button(
                        "Restart",
                        Message::WorktreeManagerRestart(index),
                        if selected {
                            SettingsButtonKind::Primary
                        } else {
                            SettingsButtonKind::Secondary
                        },
                        &self.settings,
                    );
                    let push_status: Element<'_, Message> = if entry.unpushed_commits > 0 {
                        status_pill(
                            &format!(
                                "{} unpushed {}",
                                entry.unpushed_commits,
                                if entry.unpushed_commits == 1 {
                                    "commit"
                                } else {
                                    "commits"
                                }
                            ),
                            tokens.warning,
                            &self.settings,
                        )
                    } else {
                        container("").width(0).into()
                    };
                    let details = column![
                        row![
                            text(ellipsize(&name, self.settings.ui_char_budget(34)))
                                .size(self.settings.ui_pixels(11.0))
                                .font(Font {
                                    weight: self.default_family_weight(FontWeight::Medium),
                                    ..Font::DEFAULT
                                })
                                .color(tokens.text)
                                .width(Fill)
                                .wrapping(iced::widget::text::Wrapping::None),
                            status,
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                        row![
                            text(ellipsize(&branch, self.settings.ui_char_budget(34)))
                                .font(self.settings.terminal_font.iced())
                                .size(self.settings.ui_pixels(8.5))
                                .color(tokens.faint)
                                .width(Fill)
                                .wrapping(iced::widget::text::Wrapping::None),
                            push_status,
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    ]
                    .spacing(3)
                    .width(Fill);
                    list = list.push(
                        container(
                            row![
                                container(details).width(Fill).clip(true),
                                container(action)
                                    .width(92)
                                    .align_x(iced::alignment::Horizontal::Right),
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center),
                        )
                        .padding([9, 12])
                        .width(Fill)
                        .style(move |_| iced::widget::container::Style {
                            background: selected.then(|| {
                                Color {
                                    a: 0.16,
                                    ..tokens.accent
                                }
                                .into()
                            }),
                            border: iced::Border {
                                color: if selected {
                                    Color {
                                        a: 0.55,
                                        ..tokens.accent
                                    }
                                } else {
                                    Color::TRANSPARENT
                                },
                                width: 1.0,
                                radius: 8.0.into(),
                            },
                            ..iced::widget::container::Style::default()
                        }),
                    );
                }
                body = body.push(scrollable(list).height(iced::Length::Shrink));
            }
            if let Some(error) = &manager.error {
                body = body.push(
                    text(error.clone())
                        .size(self.settings.ui_pixels(9.0))
                        .color(tokens.danger),
                );
            }
        }
        let hint = if manager.failure.is_none() && !manager.entries.is_empty() {
            "↑↓ select · Enter reviews · Esc closes"
        } else {
            ""
        };
        body = body.push(
            row![
                text(hint)
                    .size(self.settings.ui_pixels(8.0))
                    .color(tokens.faint),
                container("").width(Fill),
                settings_action_button(
                    "Close",
                    Message::CloseWorktreeManager,
                    SettingsButtonKind::Secondary,
                    &self.settings,
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
        container(body)
            .padding(18)
            .width(600)
            .style(move |_| modal_surface(tokens))
            .into()
    }

    pub(crate) fn close_workspace_dialog(&self, workspace_id: WorkspaceId) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let workspace_name = self
            .session
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .map_or("this workspace", |workspace| workspace.name.as_str());
        let can_close = self.session.workspaces.len() > 1;
        let actions: Element<'_, Message> = if can_close {
            row![
                text("Enter closes · Esc keeps")
                    .size(self.settings.ui_pixels(8.0))
                    .color(tokens.faint),
                container("").width(Fill),
                settings_action_button(
                    "Cancel",
                    Message::CancelCloseWorkspace,
                    SettingsButtonKind::Secondary,
                    &self.settings,
                ),
                settings_action_button(
                    "Close workspace",
                    Message::ConfirmCloseWorkspace(workspace_id),
                    SettingsButtonKind::Danger,
                    &self.settings,
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        } else {
            row![
                text("Esc dismisses")
                    .size(self.settings.ui_pixels(8.0))
                    .color(tokens.faint),
                container("").width(Fill),
                settings_action_button(
                    "Keep workspace",
                    Message::CancelCloseWorkspace,
                    SettingsButtonKind::Secondary,
                    &self.settings,
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        };
        container(
            column![
                text(if can_close {
                    "Close workspace?"
                } else {
                    "This is the last workspace"
                })
                .size(self.settings.ui_pixels(18.0))
                    .font(Font {
                        weight: font::Weight::Bold,
                        ..Font::DEFAULT
                    }),
                text(if can_close {
                    format!(
                        "That was the last tab in {workspace_name}. Closing it will also close the workspace and its terminal."
                    )
                } else {
                    format!(
                        "{workspace_name} is the only workspace. Create another workspace before closing its last tab."
                    )
                })
                .size(self.settings.ui_pixels(10.0))
                .color(tokens.muted),
                actions,
            ]
            .spacing(14),
        )
        .padding(18)
        .width(460)
        .style(move |_| modal_surface(tokens))
        .into()
    }
}
