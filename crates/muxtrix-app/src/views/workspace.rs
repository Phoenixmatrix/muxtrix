//! The workspace surface: the tab strip and the pane tree beneath it.
//!
//! Everything to the right of the sidebar. Phase 3 adds the sidebar and status
//! bar around this; for now it is the whole window.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Styled, Window, div, px, svg,
};

use crate::app::{IconKind, Message, ellipsize};
use crate::runtime::gpui::{Root, color};
use crate::theme::DesignTokens;
use crate::views::{icon_button, icon_path, tab_key};

/// The application bar's fixed height.
const APP_BAR_HEIGHT: f32 = 43.0;

impl Root {
    /// The active workspace: tab strip above, pane tree below.
    pub(crate) fn view_workspace(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let tokens = DesignTokens::for_appearance(app.settings.appearance);

        let Ok(workspace) = app.active_workspace() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(color(tokens.muted))
                .child("No workspace")
                .into_any_element();
        };
        let Some(tab) = workspace.active_tab() else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(color(tokens.muted))
                .child("No tab")
                .into_any_element();
        };

        // A maximized pane replaces the tree rather than being drawn over it,
        // so the other panes cost nothing while it is up.
        let tree = match app.maximized_pane {
            Some(pane_id) if tab.panes.contains_key(&pane_id) => self.view_tree(
                workspace,
                tab,
                &muxtrix_domain::PaneTree::Leaf { pane_id },
                Vec::new(),
                window,
                cx,
            ),
            _ => self.view_tree(workspace, tab, &tab.root, Vec::new(), window, cx),
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.tab_strip(cx))
            .child(div().flex_grow(1.0).overflow_hidden().p(px(8.)).child(tree))
            .into_any_element()
    }

    /// The tab strip: one chip per tab in the active workspace, plus new-tab.
    fn tab_strip(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        let Ok(workspace) = app.active_workspace() else {
            return div().h(px(APP_BAR_HEIGHT)).into_any_element();
        };

        let mut strip = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3.))
            .flex_grow(1.0)
            .min_w(px(0.));

        let workspace_id = workspace.id;
        for (index, tab) in workspace.tabs.iter().enumerate() {
            let tab_id = tab.id;
            let selected = tab.id == workspace.active_tab_id;
            let drop_target = app.tab_drag.is_some_and(|drag| {
                drag.target_workspace_id == workspace.id && drag.target_index == index
            });
            let signal = app.tab_signal_kind(tab).color(tokens);
            let name = ellipsize(&tab.name, app.settings.ui_char_budget(20));
            // The chip carries fill, border and radius; its label and close
            // action remain transparent children so the chip reads as one
            // control.
            let (fill, edge) = if selected {
                (0.08, tokens.line_strong)
            } else {
                (0.03, tokens.line)
            };
            let mut fill_color = color(tokens.text);
            fill_color.a = fill;
            let mut close_hover = color(tokens.text);
            close_hover.a = 0.10;
            strip = strip.child(
                div()
                    .id(("tab", tab_key(tab_id)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(29.))
                    .pr(px(5.))
                    .rounded(px(7.))
                    .bg(fill_color)
                    .border_1()
                    .border_color(color(if drop_target { tokens.accent } else { edge }))
                    .cursor_grab()
                    .on_mouse_move(cx.listener(
                        move |root, _: &gpui::MouseMoveEvent, window, cx| {
                            if root.app.tab_drag.is_some() {
                                root.dispatch(
                                    Message::TabDragOver(workspace_id, index),
                                    window,
                                    cx,
                                );
                            }
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(7.))
                            .h(px(27.))
                            .pl(px(11.))
                            .pr(px(4.))
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                            .text_color(color(if selected { tokens.text } else { tokens.muted }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                    // A click is a zero-distance drag; movement
                                    // turns the same gesture into reordering.
                                    root.dispatch(
                                        Message::BeginTabDrag(workspace_id, tab_id, index),
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .child(div().size(px(6.)).rounded_full().bg(color(signal)))
                            .child(name),
                    )
                    .child(
                        div()
                            .id(("close-tab", tab_key(tab_id)))
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(move |style| style.bg(close_hover))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    root.dispatch(
                                        Message::CloseTab(workspace_id, tab_id),
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .child(
                                svg()
                                    .path(icon_path(IconKind::Close))
                                    .size(px(11.))
                                    .text_color(color(tokens.muted)),
                            ),
                    ),
            );
        }

        strip = strip.child(
            icon_button(
                gpui::ElementId::from("new-tab"),
                IconKind::Add,
                tokens,
                false,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::NewTab, window, cx);
                }),
            ),
        );

        // The Commands entry: icon, label, and the real keycap for the
        // palette; then the settings control behind a rule.
        let mut keycap_fill = color(tokens.text);
        keycap_fill.a = 0.05;
        let mut pill_fill = color(tokens.text);
        pill_fill.a = 0.04;
        let mut pill_hover = color(tokens.text);
        pill_hover.a = 0.07;
        let commands = div()
            .id("commands-pill")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.))
            .h(px(29.))
            .px(px(9.))
            .rounded(px(7.))
            .bg(pill_fill)
            .border_1()
            .border_color(color(tokens.line_strong))
            .cursor_pointer()
            .hover(move |style| style.bg(pill_hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::ToggleCommandPalette, window, cx);
                }),
            )
            .child(
                svg()
                    .path(icon_path(IconKind::Command))
                    .size(px(13.))
                    .text_color(color(tokens.muted)),
            )
            .child(
                div()
                    .text_size(px(app.settings.ui_pixels(9.0)))
                    .line_height(px(app.settings.ui_pixels(9.0) * 1.3))
                    .text_color(color(tokens.muted))
                    .child("Commands"),
            )
            .child(
                div()
                    .py(px(1.))
                    .px(px(5.))
                    .rounded(px(4.))
                    .bg(keycap_fill)
                    .border_1()
                    .border_color(color(tokens.line_strong))
                    .font_family(crate::views::terminal_family(&app.settings))
                    .text_size(px(app.settings.ui_pixels(7.5)))
                    .line_height(px(app.settings.ui_pixels(7.5) * 1.3))
                    .text_color(color(tokens.muted))
                    .child(if cfg!(target_os = "macos") {
                        "Cmd+P"
                    } else {
                        "Ctrl+P"
                    }),
            );

        let tab_count = workspace.tabs.len();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .h(px(APP_BAR_HEIGHT))
            .px(px(10.))
            .bg(color(tokens.rail))
            .border_b(px(1.))
            .border_color(color(tokens.line))
            // Dragging a tab past the last chip drops it at the end.
            .on_mouse_move(
                cx.listener(move |root, _: &gpui::MouseMoveEvent, window, cx| {
                    if root.app.tab_drag.is_some() {
                        root.dispatch(Message::TabDragOver(workspace_id, tab_count), window, cx);
                    }
                }),
            )
            .child(strip)
            .child(commands)
            .child(div().w(px(1.)).h(px(16.)).bg(color(tokens.line_strong)))
            .child(
                icon_button(
                    gpui::ElementId::from("open-settings"),
                    IconKind::Settings,
                    tokens,
                    false,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::OpenSettings, window, cx);
                    }),
                ),
            )
            .into_any_element()
    }
}
