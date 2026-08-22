//! Pane rendering: the split tree, stacked-pane sheets, and pane chrome.
//!
//! Walks the [`PaneTree`] and emits a nested flex layout with a draggable
//! divider at every branch. Leaves become a full pane — header plus terminal —
//! or, when the layout is stacked and the pane is not the expanded one, a
//! collapsed header sheet.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Styled, Window, div, px,
};
use muxtrix_domain::{PaneId, PaneTree, SplitAxis, Workspace, WorkspaceTab};

use crate::app::{
    IconKind, Message, SPLIT_HANDLE_SIZE, SplitBranch, SplitKey, terminal_empty_state_copy,
};
use crate::layout::expanded_stack_pane;
use crate::runtime::gpui::{Root, color};
use crate::terminal::element::TerminalElement;
use crate::theme::DesignTokens;
use crate::views::gpui::{icon_button, pane_key};

/// The pane header's height, matching the iced build so layout comparisons
/// against the capture gallery stay meaningful.
const HEADER_HEIGHT: f32 = 31.0;

impl Root {
    /// One level of the split tree.
    pub(crate) fn view_tree(
        &self,
        workspace: &Workspace,
        tab: &WorkspaceTab,
        tree: &PaneTree,
        path: Vec<SplitBranch>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match tree {
            PaneTree::Leaf { pane_id } => self.view_pane(workspace, tab, *pane_id, window, cx),
            PaneTree::Stack { pane_ids } => {
                let expanded = expanded_stack_pane(pane_ids, tab.focused_pane_id);
                let mut sheets = div().flex().flex_col().gap(px(3.)).size_full();
                for pane_id in pane_ids.iter().copied() {
                    sheets = sheets.child(if Some(pane_id) == expanded {
                        self.view_pane(workspace, tab, pane_id, window, cx)
                    } else {
                        self.view_stacked_header(workspace, tab, pane_id, cx)
                    });
                }
                sheets.into_any_element()
            }
            PaneTree::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let key = SplitKey {
                    workspace_id: workspace.id,
                    tab_id: tab.id,
                    path: path.clone(),
                };
                let mut first_path = path.clone();
                first_path.push(SplitBranch::First);
                let mut second_path = path;
                second_path.push(SplitBranch::Second);

                let tokens = DesignTokens::for_appearance(self.app().settings.appearance);
                let dragging = self
                    .app()
                    .split_drag
                    .as_ref()
                    .is_some_and(|drag| drag.key == key);
                // The handle is a wide invisible grab target with a hairline
                // down the middle; only the hairline is visible, and only
                // while dragging does it take the accent.
                let rule = if dragging {
                    color(tokens.accent)
                } else {
                    color(tokens.line)
                };
                let thickness = px(if dragging { 2. } else { 1. });

                let first_share = f32::from(ratio.permille());
                let second_share = 1000.0 - first_share;
                let first = self.view_tree(workspace, tab, first, first_path, window, cx);
                let second = self.view_tree(workspace, tab, second, second_path, window, cx);

                let drag_key = key.clone();
                let drag_axis = *axis;
                let handle = div()
                    .id(("split", split_id(&key)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(
                                Message::BeginSplitDrag(drag_key.clone(), drag_axis),
                                window,
                                cx,
                            );
                        }),
                    );

                match axis {
                    SplitAxis::Horizontal => div()
                        .flex()
                        .flex_row()
                        .size_full()
                        .child(
                            div()
                                .flex_grow(1.0)
                                .flex_basis(gpui::relative(first_share))
                                .child(first),
                        )
                        .child(
                            handle
                                .w(px(SPLIT_HANDLE_SIZE))
                                .h_full()
                                .child(div().w(thickness).h_full().bg(rule)),
                        )
                        .child(
                            div()
                                .flex_grow(1.0)
                                .flex_basis(gpui::relative(second_share))
                                .child(second),
                        )
                        .into_any_element(),
                    SplitAxis::Vertical => div()
                        .flex()
                        .flex_col()
                        .size_full()
                        .child(
                            div()
                                .flex_grow(1.0)
                                .flex_basis(gpui::relative(first_share))
                                .child(first),
                        )
                        .child(
                            handle
                                .h(px(SPLIT_HANDLE_SIZE))
                                .w_full()
                                .child(div().h(thickness).w_full().bg(rule)),
                        )
                        .child(
                            div()
                                .flex_grow(1.0)
                                .flex_basis(gpui::relative(second_share))
                                .child(second),
                        )
                        .into_any_element(),
                }
            }
        }
    }

    /// A full pane: header chrome above the terminal surface.
    fn view_pane(
        &self,
        workspace: &Workspace,
        tab: &WorkspaceTab,
        pane_id: PaneId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        let focused = workspace.id == app.session.active_workspace_id
            && workspace.active_tab_id == tab.id
            && tab.focused_pane_id == pane_id;
        let runtime = app.terminals.get(&pane_id);
        let theme = app.settings.terminal_theme.preset();

        let body = match runtime.and_then(|runtime| runtime.snapshot.clone()) {
            Some(snapshot) => div()
                .size_full()
                .child(TerminalElement::new(
                    snapshot,
                    app.settings.clone(),
                    theme,
                    focused && app.window_focused,
                    app.cursor_phase_visible,
                    app.hovered_terminal_link(pane_id),
                    runtime
                        .and_then(|runtime| runtime.viewport)
                        .unwrap_or_default(),
                ))
                .into_any_element(),
            // No grid yet: either the shell has not spoken or the launch
            // failed, and the preview text says which.
            None => div()
                .size_full()
                .p(px(8.))
                .text_color(color(tokens.muted))
                .children(terminal_empty_state_copy(runtime).map(ToOwned::to_owned))
                .into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(color(tokens.panel))
            .child(self.pane_header(workspace, tab, pane_id, focused, cx))
            .child(div().flex_grow(1.0).overflow_hidden().child(body))
            .into_any_element()
    }

    /// The 31 px chrome strip above a pane.
    fn pane_header(
        &self,
        workspace: &Workspace,
        tab: &WorkspaceTab,
        pane_id: PaneId,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        let title = app.pane_title(workspace, pane_id).to_owned();
        let maximized = app.maximized_pane == Some(pane_id);
        let single_pane = tab.panes.len() == 1;
        let signal = app.pane_signal_kind(pane_id, false).color(tokens);

        let mut controls = div().flex().flex_row().items_center().gap(px(2.));
        if !single_pane || !maximized {
            controls = controls
                .child(self.header_button(
                    ("split-right", pane_key(pane_id)),
                    IconKind::SplitRight,
                    Message::SplitFrom(pane_id, SplitAxis::Horizontal),
                    false,
                    tokens,
                    cx,
                ))
                .child(self.header_button(
                    ("split-down", pane_key(pane_id)),
                    IconKind::SplitDown,
                    Message::SplitFrom(pane_id, SplitAxis::Vertical),
                    false,
                    tokens,
                    cx,
                ));
        }
        controls = controls
            .child(self.header_button(
                ("maximize", pane_key(pane_id)),
                if maximized {
                    IconKind::Restore
                } else {
                    IconKind::Maximize
                },
                Message::ToggleMaximize(pane_id),
                false,
                tokens,
                cx,
            ))
            .child(self.header_button(
                ("overflow", pane_key(pane_id)),
                IconKind::Overflow,
                Message::TogglePaneMenu(pane_id),
                false,
                tokens,
                cx,
            ))
            .child(self.header_button(
                ("close", pane_key(pane_id)),
                IconKind::Close,
                Message::ClosePane(pane_id),
                true,
                tokens,
                cx,
            ));

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(HEADER_HEIGHT))
            .px(px(12.))
            .bg(color(if focused {
                tokens.panel_raised
            } else {
                tokens.rail
            }))
            .border_b(px(1.))
            .border_color(color(tokens.line))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .min_w(px(0.))
                    .child(div().size(px(7.)).rounded_full().bg(color(signal)))
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(11.0)))
                            .text_color(color(if focused { tokens.text } else { tokens.muted }))
                            .truncate()
                            .child(title),
                    ),
            )
            .child(controls)
            .into_any_element()
    }

    /// A collapsed pane in a stacked layout: header only, click to expand.
    fn view_stacked_header(
        &self,
        workspace: &Workspace,
        tab: &WorkspaceTab,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(("stacked", pane_key(pane_id)))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::Focus(pane_id), window, cx);
                }),
            )
            .child(self.pane_header(workspace, tab, pane_id, false, cx))
            .into_any_element()
    }

    fn header_button(
        &self,
        id: (&'static str, u64),
        kind: IconKind,
        message: Message,
        danger: bool,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        icon_button(gpui::ElementId::from(id), kind, tokens, danger)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(message.clone(), window, cx);
                }),
            )
            .into_any_element()
    }
}

/// A stable element id for a split handle.
///
/// Element ids have to be unique and stable across frames; the branch path is
/// what distinguishes two handles in the same tab.
fn split_id(key: &SplitKey) -> u64 {
    let mut id = 0u64;
    for branch in &key.path {
        id = id * 2 + u64::from(matches!(branch, SplitBranch::Second));
        id += 1;
    }
    id
}
