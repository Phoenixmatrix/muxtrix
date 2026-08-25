//! The sidebar rail: workspaces above, the fleet below.
//!
//! Two shapes of the same list. Expanded (272 px) shows names and context;
//! collapsed (46 px) keeps only the markers and signal dots, so the rail still
//! carries pane state at a glance.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, px, svg,
};
use muxtrix_domain::{PaneId, Workspace, WorkspaceId};

use crate::app::{
    COLLAPSED_SIDEBAR_WIDTH, FleetGroupLevel, GITHUB_STATUS_DOT_SIZE, GITHUB_STATUS_ICON_SIZE,
    GITHUB_STATUS_LABEL_WIDTH, GITHUB_STATUS_ROW_SPACING, IconKind, Message, PaneSignalKind,
    RailTarget, SIDEBAR_WIDTH, ellipsize, ellipsize_start,
};
use crate::github;
use crate::layout::pane_ids_in_layout;
use crate::runtime::gpui::{Root, color};
use crate::settings::{FleetScope, FleetView};
use crate::theme::DesignTokens;
use crate::views::{TOP_CHROME_HEIGHT, icon_button, pane_key, rail_marker, terminal_family};

impl Root {
    pub(crate) fn view_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        if app.sidebar_is_compact() {
            return self.collapsed_sidebar(tokens, cx);
        }

        let mut rail = div().flex().flex_col().w_full();
        // The rail and tab bar share one flush top-chrome band.
        rail = rail.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .h(px(TOP_CHROME_HEIGHT))
                .px(px(8.))
                .child(
                    div()
                        .flex_grow(1.0)
                        .text_size(px(app.settings.ui_pixels(9.0)))
                        .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(color(tokens.faint))
                        .child("WORKSPACES"),
                )
                .child(
                    icon_button(
                        gpui::ElementId::from("new-workspace"),
                        IconKind::Add,
                        tokens,
                        false,
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::NewWorkspace, window, cx);
                        }),
                    ),
                ),
        );
        for workspace in &app.session.workspaces {
            rail = rail.child(self.workspace_row(workspace, tokens, cx));
        }
        rail = rail
            .child(div().h(px(10.)))
            .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
            .child(div().h(px(4.)))
            .child(self.fleet_header(tokens, cx));

        // One entry order feeds row rendering and the keyboard handler so the
        // visible order and direct-navigation targets can never disagree.
        let entry_order = app.fleet_entries();
        let show_workspace_groups = app.settings.fleet_scope == FleetScope::AllWorkspaces;
        let warns =
            |kind: PaneSignalKind| matches!(kind, PaneSignalKind::Warning | PaneSignalKind::Danger);
        let pane_warns = |workspace: &Workspace, pane_id: PaneId| {
            workspace.pane(pane_id).is_some_and(|pane| {
                warns(app.pane_signal_kind(
                    pane_id,
                    app.pane_needs_attention(pane_id, pane.attention.unread_count),
                ))
            })
        };
        match app.settings.fleet_view {
            FleetView::Tabs => {
                for workspace in app.fleet_workspaces() {
                    if show_workspace_groups {
                        rail = rail.child(self.fleet_group_label(
                            workspace.name.clone(),
                            FleetGroupLevel::Workspace,
                            warns(app.workspace_signal_kind(workspace)),
                            app.rail_nav == Some(RailTarget::FleetWorkspace(workspace.id)),
                            Some(Message::SwitchWorkspace(workspace.id)),
                            tokens,
                            cx,
                        ));
                    }
                    for tab in &workspace.tabs {
                        if workspace.tabs.len() > 1 {
                            rail =
                                rail.child(self.fleet_group_label(
                                    tab.name.clone(),
                                    FleetGroupLevel::Nested,
                                    warns(app.tab_signal_kind(tab)),
                                    app.rail_nav
                                        == Some(RailTarget::FleetTab(workspace.id, tab.id)),
                                    pane_ids_in_layout(&tab.root).first().map(|pane_id| {
                                        Message::FocusFleetPane(workspace.id, *pane_id)
                                    }),
                                    tokens,
                                    cx,
                                ));
                        }
                        for pane_id in pane_ids_in_layout(&tab.root) {
                            rail = rail.child(self.fleet_row(workspace.id, pane_id, tokens, cx));
                        }
                    }
                }
            }
            FleetView::Agents => {
                if entry_order.is_empty() {
                    rail = rail.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .p(px(8.))
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(9.0)))
                                    .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                                    .text_color(color(tokens.muted))
                                    .child("No agent panes"),
                            )
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(8.0)))
                                    .line_height((px(app.settings.ui_pixels(8.0))) * 1.3)
                                    .text_color(color(tokens.faint))
                                    .child("Launch Codex or Claude Code from the command palette"),
                            ),
                    );
                }
                for workspace in app.fleet_workspaces() {
                    let entries: Vec<PaneId> = entry_order
                        .iter()
                        .filter(|(workspace_id, _)| *workspace_id == workspace.id)
                        .map(|(_, pane_id)| *pane_id)
                        .collect();
                    if entries.is_empty() {
                        continue;
                    }
                    if show_workspace_groups {
                        let warning = entries
                            .iter()
                            .any(|pane_id| pane_warns(workspace, *pane_id));
                        rail = rail.child(self.fleet_group_label(
                            workspace.name.clone(),
                            FleetGroupLevel::Workspace,
                            warning,
                            app.rail_nav == Some(RailTarget::FleetWorkspace(workspace.id)),
                            Some(Message::SwitchWorkspace(workspace.id)),
                            tokens,
                            cx,
                        ));
                    }
                    for pane_id in entries {
                        rail = rail.child(self.fleet_row(workspace.id, pane_id, tokens, cx));
                    }
                }
            }
            FleetView::Repos => {
                for workspace in app.fleet_workspaces() {
                    let groups = app.fleet_repository_groups_for(workspace);
                    if show_workspace_groups && !groups.is_empty() {
                        rail = rail.child(self.fleet_group_label(
                            workspace.name.clone(),
                            FleetGroupLevel::Workspace,
                            warns(app.workspace_signal_kind(workspace)),
                            app.rail_nav == Some(RailTarget::FleetWorkspace(workspace.id)),
                            Some(Message::SwitchWorkspace(workspace.id)),
                            tokens,
                            cx,
                        ));
                    }
                    for group in groups {
                        let Some((workspace_id, first_pane)) = group.entries.first().copied()
                        else {
                            continue;
                        };
                        let warning = group
                            .entries
                            .iter()
                            .any(|(_, pane_id)| pane_warns(workspace, *pane_id));
                        rail = rail.child(self.fleet_group_label(
                            group.name,
                            FleetGroupLevel::Nested,
                            warning,
                            app.rail_nav == Some(RailTarget::FleetGroup(workspace_id, first_pane)),
                            Some(Message::FocusFleetPane(workspace_id, first_pane)),
                            tokens,
                            cx,
                        ));
                        for (_, pane_id) in group.entries {
                            rail = rail.child(self.fleet_row(workspace.id, pane_id, tokens, cx));
                        }
                    }
                }
            }
        }

        // Alerts sit after the fleet, not before the workspaces: they arrive
        // and clear on their own schedule, and at the end of the rail they
        // shove nothing above them out from under the pointer.
        if !app.global_alerts.is_empty() {
            rail = rail
                .child(div().h(px(12.)))
                .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
                .child(
                    div()
                        .p(px(6.))
                        .text_size(px(app.settings.ui_pixels(9.0)))
                        .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(color(tokens.faint))
                        .child("ATTENTION"),
                );
            for (index, alert) in app.global_alerts.iter().enumerate() {
                rail = rail.child(self.global_alert_row(index, alert, tokens, cx));
            }
        }

        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(44.))
            .py(px(4.))
            .px(px(8.))
            .child(self.github_status_button(tokens, false, cx))
            .child(div().flex_grow(1.0))
            .child(
                icon_button(
                    gpui::ElementId::from("collapse-sidebar"),
                    IconKind::Collapse,
                    tokens,
                    false,
                )
                .size(px(31.))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::ToggleSidebar, window, cx);
                    }),
                ),
            );

        // The rail's edge is its own element, so selected-row fills and copy
        // can never paint over it and no border runs along window edges.
        div()
            .flex()
            .flex_row()
            .h_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(SIDEBAR_WIDTH - 1.0))
                    .h_full()
                    .bg(color(tokens.rail))
                    .child(
                        div()
                            .id("sidebar-scroll")
                            .flex_grow(1.0)
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .pb(px(12.))
                            .child(rail),
                    )
                    .child(footer),
            )
            .child(div().w(px(1.)).h_full().bg(color(tokens.line)))
            .into_any_element()
    }

    /// The Tabs/Agents/Repos projection control, in its recessed well.
    fn fleet_header(&self, tokens: DesignTokens, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let mut well = div()
            .flex()
            .flex_row()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(7.))
            .bg(color(tokens.app))
            .border_1()
            .border_color(color(tokens.line));
        for view in [FleetView::Tabs, FleetView::Agents, FleetView::Repos] {
            let selected = app.settings.fleet_view == view;
            let mut hover = color(tokens.text);
            hover.a = 0.05;
            let mut segment = div()
                .id(SharedString::from(format!("fleet-{view}")))
                .h(px(26.))
                .px(px(6.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .cursor_pointer()
                .border_1()
                .text_size(px(app.settings.ui_pixels(10.0)))
                .line_height((px(app.settings.ui_pixels(10.0))) * 1.3)
                .whitespace_nowrap()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::SetFleetView(view), window, cx);
                    }),
                )
                .child(view.to_string());
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
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .h(px(36.))
            .px(px(6.))
            .child(well)
            .into_any_element()
    }

    /// A fleet group band with an amber rollup dot when any pane inside needs
    /// a person. Workspace bands carry stronger type and the rail surface;
    /// nested tab and repository bands stay smaller and recessed on the app
    /// surface.
    #[allow(clippy::too_many_arguments)]
    fn fleet_group_label(
        &self,
        label: String,
        level: FleetGroupLevel,
        warning: bool,
        targeted: bool,
        on_press: Option<Message>,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let workspace = level == FleetGroupLevel::Workspace;
        let mut targeted_fill = color(tokens.accent);
        targeted_fill.a = 0.12;
        let mut hover = color(tokens.text);
        hover.a = 0.04;
        let id = SharedString::from(format!(
            "fleet-group-{}-{label}",
            if workspace { "w" } else { "n" }
        ));
        let mut band = div()
            .id(id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .w_full()
            .h(px(if workspace { 32. } else { 30. }))
            .px(px(if workspace { 12. } else { 16. }))
            .border_1()
            .border_color(color(if targeted {
                tokens.accent
            } else {
                crate::theme::Color::TRANSPARENT
            }))
            .bg(if targeted {
                targeted_fill
            } else if workspace {
                color(tokens.rail)
            } else {
                color(tokens.app)
            })
            .child(
                div()
                    .flex_grow(1.0)
                    .min_w(px(0.))
                    .text_size(px(app.settings.ui_pixels(if workspace {
                        9.0
                    } else {
                        8.0
                    })))
                    .line_height(
                        (px(app.settings.ui_pixels(if workspace { 9.0 } else { 8.0 }))) * 1.3,
                    )
                    .font_weight(if workspace {
                        gpui::FontWeight::BOLD
                    } else {
                        gpui::FontWeight::SEMIBOLD
                    })
                    .text_color(color(if targeted {
                        tokens.accent
                    } else if workspace {
                        tokens.muted
                    } else {
                        tokens.faint
                    }))
                    .truncate()
                    .child(ellipsize(
                        &label.to_uppercase(),
                        app.settings.ui_char_budget(if workspace { 24 } else { 26 }),
                    )),
            )
            .child(div().size(px(6.)).rounded_full().bg(color(if warning {
                tokens.warning
            } else {
                crate::theme::Color::TRANSPARENT
            })));
        if !targeted {
            band = band.hover(move |style| style.bg(hover));
        }
        if let Some(message) = on_press {
            band = band.cursor_pointer().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(message.clone(), window, cx);
                }),
            );
        }
        band.into_any_element()
    }

    fn global_alert_row(
        &self,
        index: usize,
        alert: &crate::app::GlobalAlert,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .py(px(10.))
                    .px(px(6.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .child(div().size(px(8.)).rounded_full().bg(color(tokens.warning)))
                            .child(
                                div()
                                    .flex_grow(1.0)
                                    .min_w(px(0.))
                                    .text_size(px(app.settings.ui_pixels(11.0)))
                                    .line_height((px(app.settings.ui_pixels(11.0))) * 1.3)
                                    .text_color(color(tokens.text))
                                    .truncate()
                                    .child(alert.title.clone()),
                            )
                            .child(
                                icon_button(
                                    gpui::ElementId::from(SharedString::from(format!(
                                        "dismiss-alert-{index}"
                                    ))),
                                    IconKind::Close,
                                    tokens,
                                    false,
                                )
                                .size(px(20.))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                        root.dispatch(
                                            Message::DismissGlobalAlert(index),
                                            window,
                                            cx,
                                        );
                                    }),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                            .text_color(color(tokens.muted))
                            .child(alert.body.clone()),
                    ),
            )
            .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
            .into_any_element()
    }

    /// The GitHub status control in the rail's footer (or the collapsed rail).
    pub(crate) fn github_status_button(
        &self,
        tokens: DesignTokens,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let (label, hue) = if app.github_auth_busy {
            ("Connecting…".to_owned(), tokens.warning)
        } else {
            match &app.github_auth {
                github::AuthStatus::Checking => ("GitHub".to_owned(), tokens.muted),
                github::AuthStatus::Authenticated { login } => {
                    (format!("@{login}"), tokens.success)
                }
                github::AuthStatus::NeedsAuthentication => {
                    ("Connect GitHub".to_owned(), tokens.warning)
                }
                github::AuthStatus::Unavailable { .. } => {
                    ("GitHub unavailable".to_owned(), tokens.danger)
                }
            }
        };
        let busy = app.github_auth_busy;
        let mut hover = color(tokens.text);
        hover.a = 0.04;
        let mut control = div()
            .id("github-status")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(if compact {
                3.
            } else {
                GITHUB_STATUS_ROW_SPACING
            }))
            .rounded(px(5.))
            .hover(move |style| style.bg(hover))
            .child(
                svg()
                    .path(crate::assets::icon_path(IconKind::GitHub))
                    .size(px(GITHUB_STATUS_ICON_SIZE))
                    .text_color(color(tokens.muted)),
            );
        control = if compact {
            control.p(px(8.))
        } else {
            control.py(px(7.)).px(px(8.)).child(
                div()
                    .max_w(px(GITHUB_STATUS_LABEL_WIDTH))
                    .text_size(px(app.settings.ui_pixels(10.0)))
                    .line_height((px(app.settings.ui_pixels(10.0))) * 1.3)
                    .text_color(color(tokens.muted))
                    .truncate()
                    .child(label),
            )
        };
        control = control.child(
            div()
                .size(px(GITHUB_STATUS_DOT_SIZE))
                .rounded_full()
                .bg(color(hue)),
        );
        if !busy {
            control = control.cursor_pointer().on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::GitHubStatusPressed, window, cx);
                }),
            );
        }
        control.into_any_element()
    }

    fn workspace_row(
        &self,
        workspace: &Workspace,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let workspace_id = workspace.id;
        let selected = workspace_id == app.session.active_workspace_id;
        let targeted = app.rail_nav == Some(RailTarget::Workspace(workspace_id));
        let signal_kind = app.workspace_signal_kind(workspace);
        let context = app.workspace_context(workspace);
        let tabs = workspace.tabs.len();
        let panes = workspace.pane_count();
        let tab_count = workspace.tabs.len();

        // Text-derived translucent fills read correctly on every surface in
        // both appearances, which is why the iced rail uses them too.
        let fill = if targeted {
            let mut fill = color(tokens.accent);
            fill.a = 0.18;
            fill
        } else if selected {
            let mut fill = color(tokens.text);
            fill.a = 0.07;
            fill
        } else {
            color(crate::theme::Color::TRANSPARENT)
        };
        let mut hover = color(tokens.text);
        hover.a = 0.04;
        let context_budget =
            (crate::app::FLEET_ENTRY_TEXT_WIDTH / (app.settings.ui_pixels(8.5) * 0.62)) as usize;

        div()
            .id(("workspace", workspace_key(workspace_id)))
            .flex()
            .flex_row()
            .items_stretch()
            .w_full()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::SwitchWorkspace(workspace_id), window, cx);
                }),
            )
            .on_mouse_move(
                cx.listener(move |root, _: &gpui::MouseMoveEvent, window, cx| {
                    if root.app.tab_drag.is_some() {
                        root.dispatch(Message::TabDragOver(workspace_id, tab_count), window, cx);
                    }
                }),
            )
            .child(rail_marker(selected, targeted, tokens))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_grow(1.0)
                    .min_w(px(0.))
                    .gap(px(4.))
                    .py(px(9.))
                    .px(px(13.))
                    .bg(fill)
                    .when(targeted, |row| {
                        row.border_1().border_color(color(tokens.accent))
                    })
                    .when(!selected && !targeted, |row| {
                        row.hover(move |style| style.bg(hover))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            // The font's visible glyphs sit below the centre
                            // of its line box, so the dot is lowered by one
                            // logical pixel to align with the name optically.
                            .child(
                                div()
                                    .size(px(9.))
                                    .mt(px(2.))
                                    .rounded_full()
                                    .bg(color(signal_kind.color(tokens))),
                            )
                            .child(
                                div()
                                    .flex_grow(1.0)
                                    .min_w(px(0.))
                                    .text_size(px(app.settings.ui_pixels(11.0)))
                                    .line_height((px(app.settings.ui_pixels(11.0))) * 1.3)
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    // One accent headline means one thing
                                    // everywhere in the rail: the keyboard
                                    // cursor is standing here.
                                    .text_color(color(if targeted {
                                        tokens.accent
                                    } else {
                                        tokens.text
                                    }))
                                    .truncate()
                                    .child(ellipsize(
                                        &workspace.name,
                                        app.settings.ui_char_budget(24),
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(9.0)))
                                    .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                                    .text_color(color(signal_kind.label_color(tokens)))
                                    .whitespace_nowrap()
                                    .child(app.workspace_state_label(workspace)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                            .text_color(color(tokens.muted))
                            .child(format!(
                                "{tabs} tab{} · {panes} pane{}",
                                if tabs == 1 { "" } else { "s" },
                                if panes == 1 { "" } else { "s" }
                            )),
                    )
                    .child(
                        div()
                            .font_family(terminal_family(&app.settings))
                            .text_size(px(app.settings.ui_pixels(8.5)))
                            .line_height((px(app.settings.ui_pixels(8.5))) * 1.3)
                            .text_color(color(tokens.faint))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(if context.is_empty() {
                                "\u{00a0}".to_owned()
                            } else {
                                ellipsize_start(&context, context_budget)
                            }),
                    ),
            )
            .into_any_element()
    }

    /// One fleet entry: a pane, wherever it lives.
    fn fleet_row(
        &self,
        workspace_id: WorkspaceId,
        pane_id: PaneId,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let Some(workspace) = app
            .session
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
        else {
            return div().into_any_element();
        };
        let Some(pane) = workspace.pane(pane_id) else {
            return div().child("Missing pane").into_any_element();
        };
        let focused = workspace.id == app.session.active_workspace_id
            && workspace
                .active_tab()
                .is_some_and(|tab| tab.focused_pane_id == pane_id);
        let targeted = app.rail_nav == Some(RailTarget::FleetPane(workspace_id, pane_id));
        let attention = app.pane_needs_attention(pane_id, pane.attention.unread_count);
        let signal_kind = app.pane_signal_kind(pane_id, attention);
        let signal = signal_kind.color(tokens);
        let location = app.pane_location_label(pane_id);
        let title = app.fleet_pane_identity_label(workspace, pane_id, &location);
        let is_agent = app.agent_statuses.contains_key(&pane_id);
        let pane_state = app.pane_state_label(pane_id);
        // A text state accompanies every non-neutral pip so colour never
        // stands alone.
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
        let pull_request = app
            .pane_repositories
            .get(&pane_id)
            .and_then(|repository| repository.pull_request.clone());
        let has_pull_request = pull_request.is_some();
        let selected = focused && !has_pull_request;
        let cursor = targeted && !has_pull_request;

        let fill = if cursor {
            let mut fill = color(tokens.accent);
            fill.a = 0.18;
            fill
        } else if selected {
            let mut fill = color(tokens.text);
            fill.a = 0.07;
            fill
        } else {
            color(crate::theme::Color::TRANSPARENT)
        };
        let mut hover = color(tokens.text);
        hover.a = 0.04;

        // Agents carry a roster ring rather than a plain dot.
        let pip = if app.shows_agents_roster(pane_id) {
            let mut ring = color(signal);
            ring.a *= 0.7;
            div()
                .size(px(7.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(ring)
                .child(div().size(px(3.)).rounded_full().bg(color(signal)))
        } else {
            div().size(px(7.)).rounded_full().bg(color(signal))
        };

        div()
            .id(("fleet", pane_key(pane_id)))
            .flex()
            .flex_row()
            .items_stretch()
            .w_full()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::FocusFleetPane(workspace_id, pane_id), window, cx);
                }),
            )
            .child(rail_marker(focused, targeted, tokens))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(3.))
                    .flex_grow(1.0)
                    .min_w(px(0.))
                    .h(px(52.))
                    .py(px(5.))
                    .px(px(8.))
                    .bg(fill)
                    .when(cursor, |row| {
                        row.border_1().border_color(color(tokens.accent))
                    })
                    .when(!selected && !cursor, |row| {
                        row.hover(move |style| style.bg(hover))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .child(pip)
                            .child(
                                div()
                                    .flex_grow(1.0)
                                    .min_w(px(0.))
                                    .text_size(px(app.settings.ui_pixels(10.5)))
                                    .line_height((px(app.settings.ui_pixels(10.5))) * 1.3)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(color(if targeted {
                                        tokens.accent
                                    } else {
                                        tokens.text
                                    }))
                                    .truncate()
                                    .child(title),
                            ),
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
                                    .pl(px(15.))
                                    .text_size(px(app.settings.ui_pixels(9.0)))
                                    .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                                    .text_color(color(if focused || targeted {
                                        tokens.text
                                    } else {
                                        tokens.muted
                                    }))
                                    .truncate()
                                    .child(location),
                            )
                            .when(!has_pull_request, |line| {
                                line.child(
                                    div()
                                        .text_size(px(app.settings.ui_pixels(9.0)))
                                        .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                                        .text_color(color(state_color))
                                        .whitespace_nowrap()
                                        .child(state_label.clone()),
                                )
                            }),
                    ),
            )
            // A linked pull request shares the title's first-line baseline as
            // its marker, with the lifecycle state beneath it.
            .children(pull_request.map(|pull_request| {
                let (icon_kind, hue) = match pull_request.state {
                    github::CurrentPullRequestState::Open => {
                        (IconKind::PullRequestOpen, tokens.github_open)
                    }
                    github::CurrentPullRequestState::Draft => {
                        (IconKind::PullRequestDraft, tokens.muted)
                    }
                    github::CurrentPullRequestState::Closed => {
                        (IconKind::PullRequestClosed, tokens.faint)
                    }
                    github::CurrentPullRequestState::Merged => {
                        (IconKind::PullRequestMerged, tokens.github_merged)
                    }
                };
                let url = pull_request.url.clone();
                let mut marker_hover = color(tokens.text);
                marker_hover.a = 0.04;
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .h(px(52.))
                    .py(px(5.))
                    .px(px(2.))
                    .child(
                        div()
                            .id(("fleet-pr", pane_key(pane_id)))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(3.))
                            .h(px(30.))
                            .px(px(3.))
                            .rounded(px(5.))
                            .cursor_pointer()
                            .hover(move |style| style.bg(marker_hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    root.dispatch(
                                        Message::OpenGitHubPullRequest(url.clone()),
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .child(
                                svg()
                                    .path(crate::assets::icon_path(icon_kind))
                                    .size(px(app.settings.ui_pixels(9.0)))
                                    .text_color(color(hue)),
                            )
                            .child(
                                div()
                                    .font_family(terminal_family(&app.settings))
                                    .text_size(px(app.settings.ui_pixels(8.5)))
                                    .line_height(px(app.settings.ui_pixels(8.5) * 1.3))
                                    .text_color(color(hue))
                                    .child(format!("#{}", pull_request.number)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .line_height(px(app.settings.ui_pixels(9.0) * 1.3))
                            .text_color(color(state_color))
                            .whitespace_nowrap()
                            .child(state_label),
                    )
            }))
            .into_any_element()
    }

    /// The 46 px rail: numbered workspaces and panes with their signal dots,
    /// a new-workspace control above and GitHub and expand below — the iced
    /// collapsed rail.
    fn collapsed_sidebar(&self, tokens: DesignTokens, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let ui = |points: f32| px(app.settings.ui_pixels(points));
        let mut fill_selected = color(tokens.text);
        fill_selected.a = 0.07;
        let mut fill_targeted = color(tokens.accent);
        fill_targeted.a = 0.18;
        let mut hover = color(tokens.text);
        hover.a = 0.04;
        let row_fill = |selected: bool, targeted: bool| {
            if targeted {
                fill_targeted
            } else if selected {
                fill_selected
            } else {
                color(crate::theme::Color::TRANSPARENT)
            }
        };

        let mut items = div().flex().flex_col().items_center().w_full();
        for (index, workspace) in app.session.workspaces.iter().enumerate() {
            let workspace_id = workspace.id;
            let selected = workspace_id == app.session.active_workspace_id;
            let targeted = app.rail_nav == Some(RailTarget::Workspace(workspace_id));
            let signal = app.workspace_signal_kind(workspace).color(tokens);
            items = items
                .child(
                    div()
                        .id(("collapsed-workspace", workspace_key(workspace_id)))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .gap(px(6.))
                        .w(px(COLLAPSED_SIDEBAR_WIDTH - 2.0))
                        .h(px(43.))
                        .cursor_pointer()
                        .bg(row_fill(selected, targeted))
                        .when(targeted, |row| {
                            row.border_1().border_color(color(tokens.accent))
                        })
                        .when(!selected && !targeted, |row| {
                            row.hover(move |style| style.bg(hover))
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                root.dispatch(Message::SwitchWorkspace(workspace_id), window, cx);
                            }),
                        )
                        .child(
                            div()
                                .text_size(ui(10.0))
                                .line_height(px(app.settings.ui_pixels(10.0) * 1.3))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                // The collapsed rail has no room for a rung
                                // bar, so identity carries the cursor here.
                                .text_color(color(if targeted {
                                    tokens.accent
                                } else {
                                    tokens.text
                                }))
                                .child((index + 1).to_string()),
                        )
                        .child(div().size(px(7.)).rounded_full().bg(color(signal))),
                )
                .child(div().h(px(1.)).w_full().bg(color(tokens.line)));
        }
        // Workspaces and fleet panes are both numbered from one, so a rule in
        // the strong line colour separates the two ledgers.
        let entries = app.fleet_entries();
        if !entries.is_empty() {
            items = items
                .child(div().h(px(7.)))
                .child(
                    div()
                        .h(px(1.))
                        .w(px(COLLAPSED_SIDEBAR_WIDTH - 18.0))
                        .bg(color(tokens.line_strong)),
                )
                .child(div().h(px(7.)));
        }
        for (index, (workspace_id, pane_id)) in entries.into_iter().enumerate() {
            let Some(workspace) = app
                .session
                .workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id)
            else {
                continue;
            };
            let Some(pane) = workspace.pane(pane_id) else {
                continue;
            };
            let focused = workspace.id == app.session.active_workspace_id
                && workspace
                    .active_tab()
                    .is_some_and(|tab| tab.focused_pane_id == pane_id);
            let targeted = app.rail_nav == Some(RailTarget::FleetPane(workspace_id, pane_id));
            let attention = app.pane_needs_attention(pane_id, pane.attention.unread_count);
            let signal = app.pane_signal_kind(pane_id, attention).color(tokens);
            // The keyboard cursor outranks attention: it marks where the user
            // is looking right now, and it moves away again.
            let identity_color = if targeted {
                tokens.accent
            } else if attention {
                tokens.warning
            } else {
                tokens.text
            };
            let pip = if app.shows_agents_roster(pane_id) {
                let mut ring = color(signal);
                ring.a *= 0.7;
                div()
                    .size(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(ring)
                    .child(div().size(px(3.)).rounded_full().bg(color(signal)))
            } else {
                div().size(px(7.)).rounded_full().bg(color(signal))
            };
            items = items
                .child(
                    div()
                        .id(("collapsed-pane", pane_key(pane_id)))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .gap(px(6.))
                        .w(px(COLLAPSED_SIDEBAR_WIDTH - 2.0))
                        .h(px(43.))
                        .cursor_pointer()
                        .bg(row_fill(focused, targeted))
                        .when(targeted, |row| {
                            row.border_1().border_color(color(tokens.accent))
                        })
                        .when(!focused && !targeted, |row| {
                            row.hover(move |style| style.bg(hover))
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                root.dispatch(
                                    Message::FocusFleetPane(workspace_id, pane_id),
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .child(
                            div()
                                .text_size(ui(10.0))
                                .line_height(px(app.settings.ui_pixels(10.0) * 1.3))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(color(identity_color))
                                .child((index + 1).to_string()),
                        )
                        .child(pip),
                )
                .child(div().h(px(1.)).w_full().bg(color(tokens.line)));
        }

        let centred = |child: AnyElement| {
            div()
                .h(px(44.))
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .child(child)
        };
        div()
            .flex()
            .flex_row()
            .h_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(COLLAPSED_SIDEBAR_WIDTH - 1.0))
                    .h_full()
                    .bg(color(tokens.rail))
                    .child(
                        div()
                            .h(px(TOP_CHROME_HEIGHT))
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon_button(
                                    gpui::ElementId::from("collapsed-new-workspace"),
                                    IconKind::Add,
                                    tokens,
                                    false,
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                                        root.dispatch(Message::NewWorkspace, window, cx);
                                    }),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .id("collapsed-scroll")
                            .flex_grow(1.0)
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .child(items),
                    )
                    .child(centred(self.github_status_button(tokens, true, cx)))
                    .child(centred(
                        icon_button(
                            gpui::ElementId::from("expand-sidebar"),
                            IconKind::Expand,
                            tokens,
                            false,
                        )
                        .size(px(31.))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|root, _: &MouseDownEvent, window, cx| {
                                root.dispatch(Message::ToggleSidebar, window, cx);
                            }),
                        )
                        .into_any_element(),
                    )),
            )
            .child(div().w(px(1.)).h_full().bg(color(tokens.line)))
            .into_any_element()
    }
}

/// A stable, hashable key for a workspace, for GPUI element ids.
fn workspace_key(workspace_id: WorkspaceId) -> u64 {
    workspace_id.as_uuid().as_u128() as u64
}
