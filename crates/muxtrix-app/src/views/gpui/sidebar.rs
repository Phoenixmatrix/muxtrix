//! The sidebar rail: workspaces above, the fleet below.
//!
//! Two shapes of the same list. Expanded (272 px) shows names and context;
//! collapsed (46 px) keeps only the markers and signal dots, so the rail still
//! carries pane state at a glance.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Styled, div, px,
};
use muxtrix_domain::{PaneId, Workspace, WorkspaceId};

use crate::app::{
    COLLAPSED_SIDEBAR_WIDTH, IconKind, Message, RailTarget, SIDEBAR_WIDTH, ellipsize,
};
use crate::runtime::gpui::{Root, color};
use crate::theme::DesignTokens;
use crate::views::gpui::{icon_button, pane_key};

/// The rail header's height. Matches the app bar so the two headers' text
/// shares one baseline across the seam.
const HEADER_HEIGHT: f32 = 44.0;

impl Root {
    pub(crate) fn view_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        if app.sidebar_is_compact() {
            return self.collapsed_sidebar(tokens, cx);
        }

        let mut rail = div()
            .flex()
            .flex_col()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .bg(color(tokens.rail))
            .border_r(px(1.))
            .border_color(color(tokens.line))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(HEADER_HEIGHT))
                    .px(px(8.))
                    .child(
                        div()
                            .flex_grow(1.0)
                            .text_size(px(app.settings.ui_pixels(9.0)))
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
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(28.))
                    .px(px(8.))
                    .text_size(px(app.settings.ui_pixels(9.0)))
                    .text_color(color(tokens.faint))
                    .child("FLEET"),
            );

        // One entry order feeds the rows and the keyboard handler, so the
        // visible order and the direct-navigation targets cannot disagree.
        let mut fleet = div().flex().flex_col().flex_grow(1.0).overflow_hidden();
        for (workspace_id, pane_id) in app.fleet_entries() {
            fleet = fleet.child(self.fleet_row(workspace_id, pane_id, tokens, cx));
        }
        rail.child(fleet).into_any_element()
    }

    /// One workspace: name, state, and how much is in it.
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
        let tabs = workspace.tabs.len();
        let panes = workspace.pane_count();

        div()
            .id(("workspace", workspace_key(workspace_id)))
            .flex()
            .flex_row()
            .items_start()
            .gap(px(8.))
            .px(px(8.))
            .py(px(6.))
            .cursor_pointer()
            .bg(color(if selected {
                tokens.panel_raised
            } else {
                tokens.rail
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::SwitchWorkspace(workspace_id), window, cx);
                }),
            )
            // Always present so selection never shifts the row.
            .child(div().w(px(2.)).h(px(28.)).bg(color(if selected {
                tokens.accent
            } else {
                iced::Color::TRANSPARENT
            })))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_grow(1.0)
                    .min_w(px(0.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .size(px(9.))
                                    .rounded_full()
                                    .bg(color(signal_kind.color(tokens))),
                            )
                            .child(
                                div()
                                    .flex_grow(1.0)
                                    .text_size(px(app.settings.ui_pixels(11.0)))
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
                                    .text_color(color(signal_kind.label_color(tokens)))
                                    .child(app.workspace_state_label(workspace)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.faint))
                            .child(format!(
                                "{tabs} tab{} · {panes} pane{}",
                                if tabs == 1 { "" } else { "s" },
                                if panes == 1 { "" } else { "s" }
                            )),
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
        let attention = app.pane_needs_attention(pane_id, 0);
        let signal_kind = app.pane_signal_kind(pane_id, attention);
        let location = app.pane_location_label(pane_id);
        let title = app.fleet_pane_identity_label(workspace, pane_id, &location);
        let state = app.pane_state_label(pane_id);
        let targeted = app.rail_nav == Some(RailTarget::FleetPane(workspace_id, pane_id));
        let focused = app.session.active_workspace_id == workspace_id
            && app.focused_pane_id() == Some(pane_id);

        div()
            .id(("fleet", pane_key(pane_id)))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(5.))
            .cursor_pointer()
            .bg(color(if focused {
                tokens.panel_raised
            } else {
                tokens.rail
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::FocusFleetPane(workspace_id, pane_id), window, cx);
                }),
            )
            .child(
                div()
                    .size(px(7.))
                    .rounded_full()
                    .bg(color(signal_kind.color(tokens))),
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
                            .text_color(color(if targeted { tokens.accent } else { tokens.text }))
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.faint))
                            .truncate()
                            .child(location),
                    ),
            )
            .child(
                div()
                    .text_size(px(app.settings.ui_pixels(9.0)))
                    .text_color(color(signal_kind.label_color(tokens)))
                    .child(state),
            )
            .into_any_element()
    }

    /// The 46 px rail: markers and signal dots only.
    fn collapsed_sidebar(&self, tokens: DesignTokens, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let mut rail = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .w(px(COLLAPSED_SIDEBAR_WIDTH))
            .h_full()
            .py(px(8.))
            .bg(color(tokens.rail))
            .border_r(px(1.))
            .border_color(color(tokens.line));

        for workspace in &app.session.workspaces {
            let workspace_id = workspace.id;
            let selected = workspace_id == app.session.active_workspace_id;
            let signal = app.workspace_signal_kind(workspace).color(tokens);
            rail = rail.child(
                div()
                    .id(("collapsed-workspace", workspace_key(workspace_id)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .bg(color(if selected {
                        tokens.panel_raised
                    } else {
                        tokens.rail
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::SwitchWorkspace(workspace_id), window, cx);
                        }),
                    )
                    .child(div().size(px(9.)).rounded_full().bg(color(signal))),
            );
        }
        rail.into_any_element()
    }
}

/// A stable, hashable key for a workspace, for GPUI element ids.
fn workspace_key(workspace_id: WorkspaceId) -> u64 {
    workspace_id.as_uuid().as_u128() as u64
}
