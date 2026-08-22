//! The sidebar rail: workspaces, fleet, alerts, and the GitHub status chip.
//!
//! Two shapes of the same list. Expanded (272 px) shows names and context;
//! collapsed (46 px) keeps only the markers and signal dots, so the rail still
//! carries pane state at a glance.

use iced::widget::column;

use crate::views::prelude::*;

use crate::ellipsized_text::EllipsizedText;
use crate::layout::pane_ids_in_layout;
use crate::settings::{FleetScope, FleetView, font_with_style};
use crate::{
    COLLAPSED_SIDEBAR_WIDTH, FLEET_ENTRY_TEXT_WIDTH, FleetGroupLevel, GITHUB_STATUS_DOT_SIZE,
    GITHUB_STATUS_ICON_SIZE, GITHUB_STATUS_LABEL_WIDTH, GITHUB_STATUS_ROW_SPACING, GlobalAlert,
    PaneSignalKind, RailTarget, SIDEBAR_WIDTH, app_tooltip, centered_button_content, ellipsize,
    ellipsize_start, fleet_group_label, fleet_toggle_style, github, quiet_button_style,
    rail_marker, rail_row_style, roster_ring, section_label, signal_dot, single_line_ellipsize,
};
use iced::Font;
use muxtrix_domain::{PaneId, Workspace};

impl Muxtrix {
    pub(crate) fn github_status_button(
        &self,
        tokens: DesignTokens,
        compact: bool,
    ) -> Element<'_, Message> {
        let (label, color, tooltip_label) = if self.github_auth_busy {
            (
                "Connecting…".to_owned(),
                tokens.warning,
                "Finish connecting GitHub in your browser".to_owned(),
            )
        } else {
            match &self.github_auth {
                github::AuthStatus::Checking => (
                    "GitHub".into(),
                    tokens.muted,
                    "Checking GitHub authentication".into(),
                ),
                github::AuthStatus::Authenticated { login } => (
                    format!("@{login}"),
                    tokens.success,
                    "GitHub connected — open repository panel".into(),
                ),
                github::AuthStatus::NeedsAuthentication => (
                    "Connect GitHub".into(),
                    tokens.warning,
                    "Open local changes; connect for pull requests".into(),
                ),
                github::AuthStatus::Unavailable { reason } => (
                    "GitHub unavailable".into(),
                    tokens.danger,
                    format!("{reason} Local changes remain available."),
                ),
            }
        };
        let content: Element<'_, Message> = if compact {
            row![
                icon(IconKind::GitHub, tokens.muted, GITHUB_STATUS_ICON_SIZE),
                signal_dot(color, GITHUB_STATUS_DOT_SIZE),
            ]
            .spacing(3)
            .align_y(Alignment::Center)
            .into()
        } else {
            row![
                icon(IconKind::GitHub, tokens.muted, GITHUB_STATUS_ICON_SIZE),
                // The dot belongs to the account, not to the rail's edge, so
                // the name hugs its own copy and the dot follows it. That
                // makes the cap the only thing deciding where a long login
                // ends: it is measured against this lane, never run under the
                // dot or allowed to shove collapse off the rail.
                container(
                    EllipsizedText::owned(
                        label,
                        self.settings.ui_pixels(10.0),
                        self.settings.ui_font(),
                        tokens.muted,
                    )
                    .width(Length::Shrink)
                )
                .max_width(GITHUB_STATUS_LABEL_WIDTH),
                signal_dot(color, GITHUB_STATUS_DOT_SIZE),
            ]
            .spacing(GITHUB_STATUS_ROW_SPACING)
            .align_y(Alignment::Center)
            .into()
        };
        let mut control = button(content)
            .padding(if compact {
                Padding::from(8)
            } else {
                Padding::from([7, 8])
            })
            .style(move |_, status| quiet_button_style(tokens, false, status));
        if !self.github_auth_busy {
            control = control.on_press(Message::GitHubStatusPressed);
        }
        app_tooltip(
            control,
            tooltip_label,
            if compact {
                tooltip::Position::Right
            } else {
                tooltip::Position::Top
            },
            tokens,
            self.settings.ui_pixels(9.0),
        )
    }

    pub(crate) fn sidebar(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        if self.sidebar_is_compact() {
            return self.collapsed_sidebar(tokens);
        }

        let mut rail = column![].spacing(0);
        rail = rail.push(
            container(
                row![
                    text("WORKSPACES")
                        .size(self.settings.ui_pixels(9.0))
                        .font(Font {
                            weight: font::Weight::Bold,
                            ..Font::DEFAULT
                        })
                        .color(tokens.faint)
                        .width(Fill),
                    app_tooltip(
                        button(icon(IconKind::Add, tokens.muted, 14.0))
                            .on_press(Message::NewWorkspace)
                            .width(28)
                            .height(28)
                            .padding(0)
                            .style(move |_, status| { quiet_button_style(tokens, false, status) }),
                        "New workspace",
                        tooltip::Position::Bottom,
                        tokens,
                        self.settings.ui_pixels(9.0),
                    ),
                ]
                .align_y(Alignment::Center),
            )
            // Same height as the app bar so the two headers' text shares one
            // baseline across the rail/content seam.
            .height(44)
            .align_y(iced::alignment::Vertical::Center)
            .padding([4, 8]),
        );
        for workspace in &self.session.workspaces {
            rail = rail.push(self.workspace_row(workspace, tokens));
        }
        rail = rail.push(container("").height(10));
        rail = rail.push(
            container("")
                .height(1)
                .width(Fill)
                .style(move |_| container::Style::default().background(tokens.line)),
        );
        rail = rail.push(container("").height(4));
        rail = rail.push(self.fleet_header(tokens));
        // One entry order feeds row rendering and the keyboard handler so the
        // visible order and direct-navigation targets can never disagree.
        let entry_order = self.fleet_entries();
        let show_workspace_groups = self.settings.fleet_scope == FleetScope::AllWorkspaces;
        match self.settings.fleet_view {
            FleetView::Tabs => {
                for workspace in self.fleet_workspaces() {
                    if show_workspace_groups {
                        rail = rail.push(self.fleet_workspace_group(
                            workspace,
                            matches!(
                                self.workspace_signal_kind(workspace),
                                PaneSignalKind::Warning | PaneSignalKind::Danger
                            ),
                            tokens,
                        ));
                    }
                    for tab in &workspace.tabs {
                        if workspace.tabs.len() > 1 {
                            rail = rail.push(fleet_group_label(
                                tab.name.clone(),
                                FleetGroupLevel::Nested,
                                matches!(
                                    self.tab_signal_kind(tab),
                                    PaneSignalKind::Warning | PaneSignalKind::Danger
                                ),
                                self.rail_nav == Some(RailTarget::FleetTab(workspace.id, tab.id)),
                                pane_ids_in_layout(&tab.root)
                                    .first()
                                    .map(|pane_id| Message::FocusFleetPane(workspace.id, *pane_id)),
                                &self.settings,
                                tokens,
                            ));
                        }
                        for pane_id in pane_ids_in_layout(&tab.root) {
                            rail = rail.push(self.fleet_row(workspace, pane_id, tokens));
                        }
                    }
                }
            }
            FleetView::Agents => {
                if entry_order.is_empty() {
                    rail = rail.push(
                        container(
                            column![
                                text("No agent panes")
                                    .size(self.settings.ui_pixels(9.0))
                                    .color(tokens.muted),
                                text("Launch Codex or Claude Code from the command palette")
                                    .size(self.settings.ui_pixels(8.0))
                                    .color(tokens.faint),
                            ]
                            .spacing(3),
                        )
                        .padding([8, 8]),
                    );
                }
                for workspace in self.fleet_workspaces() {
                    let has_entries = entry_order
                        .iter()
                        .any(|(workspace_id, _)| *workspace_id == workspace.id);
                    if !has_entries {
                        continue;
                    }
                    if show_workspace_groups {
                        let warning = entry_order.iter().any(|(workspace_id, pane_id)| {
                            *workspace_id == workspace.id
                                && workspace.pane(*pane_id).is_some_and(|pane| {
                                    matches!(
                                        self.pane_signal_kind(
                                            *pane_id,
                                            self.pane_needs_attention(
                                                *pane_id,
                                                pane.attention.unread_count,
                                            ),
                                        ),
                                        PaneSignalKind::Warning | PaneSignalKind::Danger
                                    )
                                })
                        });
                        rail = rail.push(self.fleet_workspace_group(workspace, warning, tokens));
                    }
                    for (_, pane_id) in entry_order
                        .iter()
                        .copied()
                        .filter(|(workspace_id, _)| *workspace_id == workspace.id)
                    {
                        rail = rail.push(self.fleet_row(workspace, pane_id, tokens));
                    }
                }
            }
            FleetView::Repos => {
                for workspace in self.fleet_workspaces() {
                    let groups = self.fleet_repository_groups_for(workspace);
                    if show_workspace_groups && !groups.is_empty() {
                        rail = rail.push(self.fleet_workspace_group(
                            workspace,
                            matches!(
                                self.workspace_signal_kind(workspace),
                                PaneSignalKind::Warning | PaneSignalKind::Danger
                            ),
                            tokens,
                        ));
                    }
                    for group in groups {
                        let Some((workspace_id, first_pane)) = group.entries.first().copied()
                        else {
                            continue;
                        };
                        let warning = group.entries.iter().any(|(_, pane_id)| {
                            workspace.pane(*pane_id).is_some_and(|pane| {
                                matches!(
                                    self.pane_signal_kind(
                                        *pane_id,
                                        self.pane_needs_attention(
                                            *pane_id,
                                            pane.attention.unread_count,
                                        ),
                                    ),
                                    PaneSignalKind::Warning | PaneSignalKind::Danger
                                )
                            })
                        });
                        rail = rail.push(fleet_group_label(
                            group.name,
                            FleetGroupLevel::Nested,
                            warning,
                            self.rail_nav == Some(RailTarget::FleetGroup(workspace_id, first_pane)),
                            Some(Message::FocusFleetPane(workspace_id, first_pane)),
                            &self.settings,
                            tokens,
                        ));
                        for (_, pane_id) in group.entries {
                            rail = rail.push(self.fleet_row(workspace, pane_id, tokens));
                        }
                    }
                }
            }
        }

        // Alerts sit after the fleet, not before the workspaces. They arrive and
        // clear on their own schedule, and from the top they shoved every
        // workspace and pane row down the rail the moment one appeared — the
        // row a user was reaching for moved out from under the pointer. At the
        // end of the rail they cost nothing above them, and the rail's own
        // scrollbar carries them when they do not fit.
        if !self.global_alerts.is_empty() {
            rail = rail.push(container("").height(12));
            rail = rail.push(
                container("")
                    .height(1)
                    .width(Fill)
                    .style(move |_| container::Style::default().background(tokens.line)),
            );
            rail = rail.push(section_label("ATTENTION", &self.settings, tokens));
            for (index, alert) in self.global_alerts.iter().enumerate() {
                rail = rail.push(self.global_alert_row(index, alert, tokens));
            }
        }

        let collapse = app_tooltip(
            button(icon(IconKind::Collapse, tokens.muted, 15.0))
                .on_press(Message::ToggleSidebar)
                .padding(8)
                .style(move |_, status| quiet_button_style(tokens, false, status)),
            "Collapse fleet",
            tooltip::Position::Top,
            tokens,
            self.settings.ui_pixels(9.0),
        );
        let footer = row![
            self.github_status_button(tokens, false),
            container("").width(Fill),
            collapse
        ]
        .align_y(Alignment::Center);
        let surface = container(column![
            scrollable(container(rail).padding(Padding {
                top: 0.0,
                bottom: 12.0,
                left: 0.0,
                right: 0.0,
            }))
            .height(Fill),
            container(footer).height(44).padding([4, 8]),
        ])
        .width(SIDEBAR_WIDTH - 1.0)
        .height(Fill)
        .style(move |_| container::Style::default().background(tokens.rail));
        // The rail's edge is its own element, so selected-row fills and copy
        // can never paint over it and no border runs along window edges.
        row![
            surface,
            container("")
                .width(1)
                .height(Fill)
                .style(move |_| container::Style::default().background(tokens.line)),
        ]
        .into()
    }

    pub(crate) fn workspace_row<'a>(
        &'a self,
        workspace: &'a Workspace,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let selected = workspace.id == self.session.active_workspace_id;
        let targeted = self.rail_nav == Some(RailTarget::Workspace(workspace.id));
        let signal_kind = self.workspace_signal_kind(workspace);
        let context = self.workspace_context(workspace);
        let tab_count = workspace.tabs.len();
        let pane_count = workspace.pane_count();
        let details = column![
            row![
                // The font's visible glyphs sit below the center of its line
                // box, so lower the geometric dot by one logical pixel to
                // align it with the workspace name optically.
                container(signal_dot(signal_kind.color(tokens), 9.0)).padding(Padding {
                    top: 2.0,
                    bottom: 0.0,
                    left: 0.0,
                    right: 0.0,
                }),
                text(ellipsize(&workspace.name, self.settings.ui_char_budget(24)))
                    .size(self.settings.ui_pixels(11.0))
                    .font(Font {
                        weight: font::Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    // One accent headline means one thing everywhere in the
                    // rail: the keyboard cursor is standing here.
                    .color(if targeted { tokens.accent } else { tokens.text })
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None),
                text(self.workspace_state_label(workspace))
                    // Same size as the fleet row's state label: both report a
                    // pane's condition, so they belong to one type step.
                    .size(self.settings.ui_pixels(9.0))
                    .color(signal_kind.label_color(tokens))
                    .wrapping(iced::widget::text::Wrapping::None),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text(format!(
                "{tab_count} tab{} · {pane_count} pane{}",
                if tab_count == 1 { "" } else { "s" },
                if pane_count == 1 { "" } else { "s" }
            ))
            .size(self.settings.ui_pixels(9.0))
            .color(tokens.muted),
            text(if context.is_empty() {
                "\u{00a0}".to_owned()
            } else {
                let budget =
                    (FLEET_ENTRY_TEXT_WIDTH / (self.settings.ui_pixels(8.5) * 0.62)) as usize;
                ellipsize_start(&context, budget)
            })
            .font(self.settings.terminal_font.iced())
            .size(self.settings.ui_pixels(8.5))
            .color(tokens.faint)
            .wrapping(iced::widget::text::Wrapping::None),
        ]
        .spacing(4);
        let row = row![
            rail_marker(selected, targeted, tokens),
            button(details)
                .on_press(Message::SwitchWorkspace(workspace.id))
                .padding([9, 13])
                .width(Fill)
                .style(move |_, status| rail_row_style(tokens, selected, targeted, status)),
        ];
        mouse_area(row)
            .on_enter(Message::TabDragOver(workspace.id, workspace.tabs.len()))
            .into()
    }

    pub(crate) fn fleet_workspace_group<'a>(
        &'a self,
        workspace: &Workspace,
        warning: bool,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        fleet_group_label(
            workspace.name.clone(),
            FleetGroupLevel::Workspace,
            warning,
            self.rail_nav == Some(RailTarget::FleetWorkspace(workspace.id)),
            Some(Message::SwitchWorkspace(workspace.id)),
            &self.settings,
            tokens,
        )
    }

    /// The Tabs/Agents/Repos projection control.
    pub(crate) fn fleet_header(&self, tokens: DesignTokens) -> Element<'_, Message> {
        let view_segment = |view: FleetView| -> Element<'_, Message> {
            let selected = self.settings.fleet_view == view;
            button(centered_button_content(
                text(view.to_string())
                    .size(self.settings.ui_pixels(10.0))
                    .color(if selected { tokens.text } else { tokens.muted })
                    .wrapping(iced::widget::text::Wrapping::None),
            ))
            .on_press(Message::SetFleetView(view))
            .height(24)
            .padding([0, 6])
            .style(move |_, status| fleet_toggle_style(tokens, selected, status))
            .into()
        };
        let toggle_well = move |content| {
            container(content).padding(2).style(move |_| {
                container::Style::default()
                    .background(tokens.app)
                    .border(Border {
                        color: tokens.line,
                        width: 1.0,
                        radius: 7.0.into(),
                    })
            })
        };
        container(
            row![
                iced::widget::Space::new().width(Fill),
                toggle_well(
                    row![
                        view_segment(FleetView::Tabs),
                        view_segment(FleetView::Agents),
                        view_segment(FleetView::Repos)
                    ]
                    .spacing(2)
                ),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .height(36)
        .align_y(iced::alignment::Vertical::Center)
        .padding([0, 6])
        .into()
    }

    pub(crate) fn collapsed_sidebar(&self, tokens: DesignTokens) -> Element<'_, Message> {
        let mut items = column![].spacing(0).align_x(Alignment::Center);
        for (index, workspace) in self.session.workspaces.iter().enumerate() {
            let selected = workspace.id == self.session.active_workspace_id;
            let targeted = self.rail_nav == Some(RailTarget::Workspace(workspace.id));
            let signal_kind = self.workspace_signal_kind(workspace);
            let hint = format!(
                "{}\n{} · {} tabs · {} panes",
                workspace.name,
                self.workspace_state_label(workspace),
                workspace.tabs.len(),
                workspace.pane_count()
            );
            items = items.push(app_tooltip(
                button(
                    container(
                        row![
                            text((index + 1).to_string())
                                .size(self.settings.ui_pixels(10.0))
                                .font(Font {
                                    weight: font::Weight::Semibold,
                                    ..Font::DEFAULT
                                })
                                // The collapsed rail has no room for a rung bar,
                                // so identity carries the cursor here, the same
                                // accent the expanded rows put on their headline.
                                .color(if targeted { tokens.accent } else { tokens.text }),
                            signal_dot(signal_kind.color(tokens), 7.0),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .width(Fill)
                    .height(Fill)
                    .center_x(Fill)
                    .align_y(iced::alignment::Vertical::Center),
                )
                .on_press(Message::SwitchWorkspace(workspace.id))
                .width(COLLAPSED_SIDEBAR_WIDTH - 2.0)
                .height(43)
                .padding(0)
                .style(move |_, status| rail_row_style(tokens, selected, targeted, status)),
                hint,
                tooltip::Position::Right,
                tokens,
                self.settings.ui_pixels(9.0),
            ));
            items = items.push(
                container("")
                    .height(1)
                    .width(Fill)
                    .style(move |_| container::Style::default().background(tokens.line)),
            );
        }
        // Workspaces and fleet panes are both numbered from one, so an 8px gap
        // was the only thing telling a user which "1" they were looking at. A
        // rule in the strong line colour separates the two ledgers the way the
        // expanded rail's own divider does.
        if !self.fleet_entries().is_empty() {
            items = items.push(container("").height(7));
            items = items.push(
                container("")
                    .height(1)
                    .width(COLLAPSED_SIDEBAR_WIDTH - 18.0)
                    .style(move |_| container::Style::default().background(tokens.line_strong)),
            );
            items = items.push(container("").height(7));
        }
        for (index, (workspace_id, pane_id)) in self.fleet_entries().into_iter().enumerate() {
            if let Some(workspace) = self
                .session
                .workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id)
            {
                let focused = workspace.id == self.session.active_workspace_id
                    && workspace
                        .active_tab()
                        .is_some_and(|tab| tab.focused_pane_id == pane_id);
                let targeted = self.rail_nav == Some(RailTarget::FleetPane(workspace_id, pane_id));
                let Some(pane) = workspace.pane(pane_id) else {
                    continue;
                };
                let attention = self.pane_needs_attention(pane_id, pane.attention.unread_count);
                let color = self.pane_signal_color(pane_id, attention, tokens);
                let title = self.pane_title(workspace, pane_id);
                // Agent tooltips carry lifecycle; terminal tooltips carry the
                // same truthful command/folder copy as their expanded rows.
                let hint = if self.agent_statuses.contains_key(&pane_id) {
                    let state = self.pane_state_label(pane_id);
                    let activity = single_line_ellipsize(
                        &self.pane_activity(pane_id, pane.attention.message.as_deref()),
                        self.settings.ui_char_budget(72),
                    );
                    format!("{title}\n{state} · {activity}")
                } else {
                    let mut line = self.pane_command(pane_id);
                    let state = self.pane_state_label(pane_id);
                    if state != "Shell" {
                        line = format!("{state} · {line}");
                    }
                    let context = self.pane_context(pane_id);
                    if !context.is_empty() {
                        line = format!("{line} · {context}");
                    }
                    format!(
                        "{title}\n{}",
                        single_line_ellipsize(&line, self.settings.ui_char_budget(72))
                    )
                };
                // Attention is carried by colour, not by a tally: how many
                // notifications an agent emitted while the user was elsewhere
                // says nothing about what it wants, and the number competes
                // with the row index for the same few pixels.
                let identity = (index + 1).to_string();
                // The keyboard cursor outranks attention: it marks where the
                // user is looking right now, and it moves away again. Attention
                // still has the pip, which the cursor does not touch.
                let identity_color = if targeted {
                    tokens.accent
                } else if attention {
                    tokens.warning
                } else {
                    tokens.text
                };
                items = items.push(app_tooltip(
                    button(
                        container(
                            row![
                                text(identity)
                                    .size(self.settings.ui_pixels(10.0))
                                    .font(Font {
                                        weight: font::Weight::Semibold,
                                        ..Font::DEFAULT
                                    })
                                    .color(identity_color),
                                self.pane_pip(pane_id, color, 7.0),
                            ]
                            .spacing(6)
                            .align_y(Alignment::Center),
                        )
                        .width(Fill)
                        .height(Fill)
                        .center_x(Fill)
                        .align_y(iced::alignment::Vertical::Center),
                    )
                    .on_press(Message::FocusFleetPane(workspace_id, pane_id))
                    .width(COLLAPSED_SIDEBAR_WIDTH - 2.0)
                    .height(43)
                    .padding(0)
                    .style(move |_, status| rail_row_style(tokens, focused, targeted, status)),
                    hint,
                    tooltip::Position::Right,
                    tokens,
                    self.settings.ui_pixels(9.0),
                ));
                items = items.push(
                    container("")
                        .height(1)
                        .width(Fill)
                        .style(move |_| container::Style::default().background(tokens.line)),
                );
            }
        }
        let expand = app_tooltip(
            button(icon(IconKind::Expand, tokens.muted, 15.0))
                .padding(8)
                .on_press(Message::ToggleSidebar)
                .style(move |_, status| quiet_button_style(tokens, false, status)),
            "Expand fleet",
            tooltip::Position::Right,
            tokens,
            self.settings.ui_pixels(9.0),
        );
        let surface = container(column![
            container(app_tooltip(
                button(icon(IconKind::Add, tokens.text, 16.0))
                    .on_press(Message::NewWorkspace)
                    .padding(8)
                    .style(move |_, status| quiet_button_style(tokens, false, status)),
                "New workspace",
                tooltip::Position::Right,
                tokens,
                self.settings.ui_pixels(9.0),
            ))
            .height(44)
            .center_x(Fill)
            .align_y(iced::alignment::Vertical::Center),
            scrollable(container(items).padding([0, 0])).height(Fill),
            container(self.github_status_button(tokens, true))
                .height(44)
                .center_x(Fill)
                .align_y(iced::alignment::Vertical::Center),
            container(expand)
                .height(44)
                .center_x(Fill)
                .align_y(iced::alignment::Vertical::Center),
        ])
        .width(COLLAPSED_SIDEBAR_WIDTH - 1.0)
        .height(Fill)
        .style(move |_| container::Style::default().background(tokens.rail));
        row![
            surface,
            container("")
                .width(1)
                .height(Fill)
                .style(move |_| container::Style::default().background(tokens.line)),
        ]
        .into()
    }

    pub(crate) fn global_alert_row<'a>(
        &'a self,
        index: usize,
        alert: &'a GlobalAlert,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        column![
            container(
                column![
                    row![
                        signal_dot(tokens.warning, 8.0),
                        text(&alert.title)
                            .size(self.settings.ui_pixels(11.0))
                            .color(tokens.text)
                            .width(Fill),
                        button(icon(IconKind::Close, tokens.faint, 12.0))
                            .on_press(Message::DismissGlobalAlert(index))
                            .padding(4)
                            .style(move |_, status| quiet_button_style(tokens, false, status)),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    text(&alert.body)
                        .size(self.settings.ui_pixels(9.0))
                        .color(tokens.muted),
                ]
                .spacing(4)
            )
            .padding([10, 6]),
            container("")
                .height(1)
                .width(Fill)
                .style(move |_| container::Style::default().background(tokens.line)),
        ]
        .into()
    }

    pub(crate) fn fleet_row<'a>(
        &'a self,
        workspace: &'a Workspace,
        pane_id: PaneId,
        tokens: DesignTokens,
    ) -> Element<'a, Message> {
        let Some(pane) = workspace.pane(pane_id) else {
            return container(text("Missing pane")).into();
        };
        let focused = workspace.id == self.session.active_workspace_id
            && workspace
                .active_tab()
                .is_some_and(|tab| tab.focused_pane_id == pane_id);
        let targeted = self.rail_nav == Some(RailTarget::FleetPane(workspace.id, pane_id));
        let attention = self.pane_needs_attention(pane_id, pane.attention.unread_count);
        let signal_kind = self.pane_signal_kind(pane_id, attention);
        let signal = signal_kind.color(tokens);
        let location = self.pane_location_label(pane_id);
        let title = self.fleet_pane_identity_label(workspace, pane_id, &location);
        let is_agent = self.agent_statuses.contains_key(&pane_id);
        let pane_state = self.pane_state_label(pane_id);
        // Agents report a real lifecycle, so their rows carry state, activity,
        // and branch context. Plain terminals have none of that: their rows
        // show only what is true — name, command, and folder. A text state
        // still accompanies every non-neutral pip so color never stands alone.
        let state_label = if attention && pane_state == "Shell" {
            "Needs input".to_owned()
        } else {
            pane_state
        };
        let state_color = if is_agent || state_label != "Needs input" {
            signal_kind.label_color(tokens)
        } else {
            tokens.warning
        };
        // The title is the row's primary identity, so linked PR metadata shares
        // its first-line baseline. Context and lifecycle state form the second
        // line, with state directly beneath the PR marker when one is present.
        let dot = self.pane_pip(pane_id, signal, 7.0);
        // Weight is the one property here that changes text metrics, so it never
        // varies with state: a row that thickened under focus or the cursor
        // re-fitted its own ellipsis and slid every glyph sideways as the
        // selection moved. Emphasis rides on colour and the leading marker
        // instead, neither of which costs a single pixel of layout.
        let title_font = font_with_style(
            self.settings.ui_font(),
            self.ui_weight(FontWeight::Medium),
            font::Style::Normal,
        );
        let title_line = row![
            dot,
            EllipsizedText::owned(
                title,
                self.settings.ui_pixels(10.5),
                title_font,
                if targeted { tokens.accent } else { tokens.text },
            ),
        ]
        .spacing(8)
        .width(Fill)
        .clip(true)
        .align_y(Alignment::Center);
        let context_font = font_with_style(
            self.settings.ui_font(),
            self.ui_weight(FontWeight::Normal),
            font::Style::Normal,
        );
        let context_line = row![
            container("").width(15),
            EllipsizedText::owned(
                location,
                self.settings.ui_pixels(9.0),
                context_font,
                if focused || targeted {
                    tokens.text
                } else {
                    tokens.muted
                },
            ),
        ]
        .spacing(0)
        .width(Fill)
        .clip(true)
        .align_y(Alignment::Center);
        // No unread tally. The pip and the state label already say a pane wants
        // the user; the count only said how many times it said so while they
        // were elsewhere, which never changes what they do next.
        let pull_request = self
            .pane_repositories
            .get(&pane_id)
            .and_then(|repository| repository.pull_request.clone());
        let has_pull_request = pull_request.is_some();
        let context_line: Element<'_, Message> = if has_pull_request {
            context_line.into()
        } else {
            let state: Element<'_, Message> = text(state_label.clone())
                .size(self.settings.ui_pixels(9.0))
                .color(state_color)
                .wrapping(iced::widget::text::Wrapping::None)
                .into();
            row![context_line, state]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
        };
        let details = column![title_line, context_line].spacing(3).width(Fill);
        let mut row = row![
            rail_marker(focused, targeted, tokens),
            button(centered_button_content(details))
                .on_press(Message::FocusFleetPane(workspace.id, pane_id))
                .height(52)
                .padding([5, 8])
                .width(Fill)
                .style(move |_, status| {
                    rail_row_style(
                        tokens,
                        focused && !has_pull_request,
                        targeted && !has_pull_request,
                        status,
                    )
                }),
        ]
        .align_y(Alignment::Center);
        if let Some(pull_request) = pull_request {
            let (pull_request_state, icon_kind, color) = match pull_request.state {
                github::CurrentPullRequestState::Open => {
                    ("Open", IconKind::PullRequestOpen, tokens.github_open)
                }
                github::CurrentPullRequestState::Draft => {
                    ("Draft", IconKind::PullRequestDraft, tokens.muted)
                }
                github::CurrentPullRequestState::Closed => {
                    ("Closed", IconKind::PullRequestClosed, tokens.faint)
                }
                github::CurrentPullRequestState::Merged => {
                    ("Merged", IconKind::PullRequestMerged, tokens.github_merged)
                }
            };
            let marker = app_tooltip(
                button(
                    row![
                        icon(icon_kind, color, self.settings.ui_pixels(9.0)),
                        text(format!("#{}", pull_request.number))
                            .size(self.settings.ui_pixels(8.5))
                            .font(self.settings.terminal_font.iced())
                            .color(color),
                    ]
                    .spacing(3)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::OpenGitHubPullRequest(pull_request.url.clone()))
                .height(30)
                .padding([0, 3])
                .style(move |_, status| quiet_button_style(tokens, false, status)),
                format!(
                    "Pull request #{} · {pull_request_state}\nOpen in GitHub",
                    pull_request.number
                ),
                tooltip::Position::Right,
                tokens,
                self.settings.ui_pixels(9.0),
            );
            let state = text(state_label)
                .size(self.settings.ui_pixels(9.0))
                .color(state_color)
                .wrapping(iced::widget::text::Wrapping::None);
            let marker_layer = container(marker).height(52).padding(Padding {
                top: 5.0,
                right: 2.0,
                bottom: 17.0,
                left: 2.0,
            });
            let state_layer = container(state)
                .height(52)
                .padding(Padding {
                    top: 29.0,
                    right: 2.0,
                    bottom: 5.0,
                    left: 2.0,
                })
                .align_x(iced::alignment::Horizontal::Right);
            row = row.push(stack([marker_layer.into(), state_layer.into()]));
            return container(row)
                .width(Fill)
                .style(move |_| {
                    let background = if targeted {
                        Some(
                            Color {
                                a: 0.18,
                                ..tokens.accent
                            }
                            .into(),
                        )
                    } else if focused {
                        Some(
                            Color {
                                a: 0.07,
                                ..tokens.text
                            }
                            .into(),
                        )
                    } else {
                        None
                    };
                    container::Style {
                        background,
                        border: if targeted {
                            Border {
                                color: tokens.accent,
                                width: 1.0,
                                radius: 0.0.into(),
                            }
                        } else {
                            Border::default()
                        },
                        ..container::Style::default()
                    }
                })
                .into();
        }
        row.into()
    }

    /// Every surface that stands for one pane draws the same pip, so a pane
    /// projecting the roster reads identically in the rail, its header, and its
    /// stacked title sheet.
    pub(crate) fn pane_pip(
        &self,
        pane_id: PaneId,
        color: Color,
        size: f32,
    ) -> Element<'static, Message> {
        if self.shows_agents_roster(pane_id) {
            roster_ring(color, size)
        } else {
            signal_dot(color, size)
        }
    }
}
