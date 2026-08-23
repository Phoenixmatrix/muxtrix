//! What floats above the shell: the command palette and the toast.
//!
//! Both are transient and neither participates in the shell's layout, so they
//! are drawn last and positioned against the window rather than against
//! whatever they happen to sit over.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, StatefulInteractiveElement, Styled, div, px,
};

use crate::app::Message;
use crate::commands;
use crate::runtime::gpui::{Root, color};
use crate::theme::DesignTokens;

/// How long a toast stays up. Matches the iced shell.
const TOAST_LIFETIME: std::time::Duration = std::time::Duration::from_secs(4);

impl Root {
    /// The command palette, when it is open.
    pub(crate) fn command_palette(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let app = self.app();
        if !app.palette.visible {
            return None;
        }
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        let commands = commands::filtered(&app.palette.query);

        let mut rows = div().flex().flex_col().gap(px(1.));
        for (index, command) in commands.iter().enumerate() {
            let enabled = app.command_enabled(command.action);
            let selected = index == app.palette.selected;
            let action = command.action;
            let mut hover = color(tokens.line_strong);
            hover.a = 0.14;
            let mut row = div()
                .id(("palette", index as u64))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .h(px(34.))
                .px(px(10.))
                .rounded(px(5.))
                .bg(color(if selected {
                    tokens.panel_raised
                } else {
                    tokens.overlay
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_grow(1.0)
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(app.settings.ui_pixels(11.0)))
                                .text_color(color(if enabled { tokens.text } else { tokens.faint }))
                                .truncate()
                                .child(command.title),
                        )
                        .child(
                            div()
                                .text_size(px(app.settings.ui_pixels(9.0)))
                                .text_color(color(tokens.faint))
                                .truncate()
                                .child(command.subtitle),
                        ),
                )
                .child(
                    div()
                        .text_size(px(app.settings.ui_pixels(9.0)))
                        .text_color(color(tokens.faint))
                        .child(command.shortcut),
                );
            if enabled {
                row = row
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::RunCommand(action), window, cx);
                        }),
                    );
            }
            rows = rows.child(row);
        }

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .bg(color(tokens.scrim))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::ToggleCommandPalette, window, cx);
                    }),
                )
                .child(
                    div()
                        .mt(px(96.))
                        .w(px(560.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .p(px(8.))
                        .rounded(px(10.))
                        .bg(color(tokens.overlay))
                        .border_1()
                        .border_color(color(tokens.line))
                        .shadow_lg()
                        .child(
                            div()
                                .h(px(34.))
                                .px(px(6.))
                                .flex()
                                .items_center()
                                .rounded(px(6.))
                                .bg(color(tokens.panel))
                                .child(gpui_component::input::Input::new(&self.inputs.palette)),
                        )
                        .child(
                            div()
                                .id("palette-list")
                                .max_h(px(420.))
                                .overflow_y_scroll()
                                .track_scroll(&self.scrolls.palette)
                                .child(rows),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The bottom toast, while it is still fresh.
    pub(crate) fn toast(&self) -> Option<AnyElement> {
        let app = self.app();
        let (message, raised) = app.toast.as_ref()?;
        if raised.elapsed() > TOAST_LIFETIME {
            return None;
        }
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_end()
                .child(
                    div()
                        .mb(px(28.))
                        .px(px(14.))
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .rounded(px(15.))
                        .bg(color(tokens.overlay))
                        .border_1()
                        .border_color(color(tokens.line))
                        .text_size(px(app.settings.ui_pixels(10.0)))
                        .text_color(color(tokens.text))
                        .child(message.clone()),
                )
                .into_any_element(),
        )
    }

    /// The status bar, when the setting asks for one.
    pub(crate) fn status_bar(&self) -> Option<AnyElement> {
        let app = self.app();
        if !app.settings.show_status_bar {
            return None;
        }
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        let panes = app
            .active_workspace()
            .ok()
            .and_then(muxtrix_domain::Workspace::active_tab)
            .map_or(0, |tab| tab.panes.len());
        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .h(px(26.))
                .px(px(10.))
                .bg(color(tokens.rail))
                .border_t(px(1.))
                .border_color(color(tokens.line))
                .child(
                    div()
                        .flex_grow(1.0)
                        .text_size(px(app.settings.ui_pixels(9.0)))
                        .text_color(color(tokens.muted))
                        .truncate()
                        .child(app.status.clone()),
                )
                .child(
                    div()
                        .text_size(px(app.settings.ui_pixels(9.0)))
                        .text_color(color(tokens.faint))
                        .child(format!("{panes} pane{}", if panes == 1 { "" } else { "s" })),
                )
                .into_any_element(),
        )
    }
}
