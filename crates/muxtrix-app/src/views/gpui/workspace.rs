//! The workspace surface: the tab strip and the pane tree beneath it.
//!
//! Everything to the right of the sidebar. Phase 3 adds the sidebar and status
//! bar around this; for now it is the whole window.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Styled, Window, div, px,
};

use crate::app::{IconKind, Message};
use crate::runtime::gpui::{Root, color};
use crate::theme::DesignTokens;
use crate::views::gpui::{icon_button, tab_key};

/// The app bar's height, matching the iced build.
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
            .child(div().flex_grow(1.0).overflow_hidden().child(tree))
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
            .gap(px(4.))
            .h(px(APP_BAR_HEIGHT))
            .px(px(8.))
            .bg(color(tokens.rail))
            .border_b(px(1.))
            .border_color(color(tokens.line));

        let workspace_id = workspace.id;
        for (index, tab) in workspace.tabs.iter().enumerate() {
            let tab_id = tab.id;
            let active = tab.id == workspace.active_tab_id;
            let title = app.pane_title(workspace, tab.focused_pane_id).to_owned();
            strip = strip.child(
                div()
                    .id(("tab", tab_key(tab_id)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .h(px(27.))
                    .px(px(10.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .bg(color(if active {
                        tokens.panel_raised
                    } else {
                        tokens.panel
                    }))
                    .text_size(px(app.settings.ui_pixels(11.0)))
                    .text_color(color(if active { tokens.text } else { tokens.muted }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            // The same message the iced strip sends: a click
                            // is a zero-distance drag, and dragging a tab is
                            // also how it is reordered.
                            root.dispatch(
                                Message::BeginTabDrag(workspace_id, tab_id, index),
                                window,
                                cx,
                            );
                        }),
                    )
                    .child(title),
            );
        }

        strip
            .child(
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
            )
            .into_any_element()
    }
}
