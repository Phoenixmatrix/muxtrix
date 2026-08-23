//! What floats above the shell: the command palette and the toast.
//!
//! Both are transient and neither participates in the shell's layout, so they
//! are drawn last and positioned against the window rather than against
//! whatever they happen to sit over.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, StatefulInteractiveElement, Styled, div, px,
};

use crate::app::{Message, agent_display_name};
use crate::commands;
use crate::runtime::gpui::{Root, color};
use crate::theme::DesignTokens;
use crate::views::gpui::terminal_family;

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
        let settings = &app.settings;
        let ui = |points: f32| px(settings.ui_pixels(points));
        let commands = commands::filtered(&app.palette.query);
        let configured_agent = app
            .configured_default_agent()
            .map(|agent| agent_display_name(&agent.to_string()).to_owned());

        let mut rows = div().flex().flex_col().gap(px(2.));
        for (index, command) in commands.iter().enumerate() {
            let enabled = app.command_enabled(command.action);
            let selected = enabled && index == app.palette.selected;
            let title = if command.action.requires_default_agent() {
                configured_agent.as_ref().map_or_else(
                    || command.title.to_owned(),
                    |agent| {
                        command
                            .title
                            .replace("with agent", &format!("with {agent}"))
                    },
                )
            } else {
                command.title.to_owned()
            };
            let subtitle = if enabled {
                if command.action.requires_default_agent() && configured_agent.is_none() {
                    "Choose a configured default agent in Settings to use this command".to_owned()
                } else if command.action.requires_default_agent() {
                    command.subtitle.replace(
                        "the default agent",
                        configured_agent.as_deref().unwrap_or("your agent"),
                    )
                } else {
                    command.subtitle.to_owned()
                }
            } else {
                "Restore panes to use this command".to_owned()
            };
            let mut selected_fill = color(tokens.accent);
            selected_fill.a = 0.14;
            let mut row = div()
                .id(("palette", index as u64))
                .flex()
                .flex_row()
                .items_center()
                .flex_grow(1.0)
                .min_w(px(0.))
                .pt(px(7.))
                .pb(px(7.))
                .pl(px(12.))
                // Extra right inset keeps the chord hints clear of the
                // overlay scrollbar.
                .pr(px(24.))
                .rounded(px(5.))
                .when(selected, |row| row.bg(selected_fill))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .flex_grow(1.0)
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(ui(11.0))
                                .line_height(px(settings.ui_pixels(11.0) * 1.3))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(color(if enabled { tokens.text } else { tokens.faint }))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(ui(9.0))
                                .line_height(px(settings.ui_pixels(9.0) * 1.3))
                                .text_color(color(if enabled {
                                    tokens.muted
                                } else {
                                    tokens.faint
                                }))
                                .child(subtitle),
                        ),
                )
                .when(!command.shortcut.is_empty(), |row| {
                    row.child(
                        div()
                            .font_family(terminal_family(settings))
                            .text_size(ui(7.5))
                            .line_height((ui(7.5)) * 1.3)
                            .text_color(color(tokens.faint))
                            .whitespace_nowrap()
                            .child(command.shortcut),
                    )
                });
            if enabled {
                row = row
                    .cursor_pointer()
                    .when(!selected, |row| {
                        row.hover(move |style| style.bg(color(tokens.panel)))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::CommandSelected(index), window, cx);
                        }),
                    );
            }
            rows = rows.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .child(
                        div()
                            .w(px(3.))
                            .h_full()
                            .flex_shrink_0()
                            .bg(color(if selected {
                                tokens.accent
                            } else {
                                iced::Color::TRANSPARENT
                            })),
                    )
                    .child(row),
            );
        }
        let results = if commands.is_empty() {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(18.))
                .child(
                    div()
                        .text_size(ui(11.0))
                        .line_height((ui(11.0)) * 1.3)
                        .child("No matching commands"),
                )
                .child(
                    div()
                        .text_size(ui(9.0))
                        .line_height((ui(9.0)) * 1.3)
                        .text_color(color(tokens.muted))
                        .child("Try searching for split, terminal, or settings."),
                )
                .into_any_element()
        } else {
            // Bounded so the keyboard-hint footer below always renders.
            div()
                .id("palette-list")
                .max_h(px(400.))
                .overflow_y_scroll()
                .track_scroll(&self.scrolls.palette)
                .child(rows)
                .into_any_element()
        };

        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(72.))
                .bg(color(tokens.scrim))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::ToggleCommandPalette, window, cx);
                    }),
                )
                .child(
                    div()
                        .w(px(620.))
                        .max_h(px(520.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .pt(px(12.))
                        .pb(px(10.))
                        .rounded(px(10.))
                        .bg(color(tokens.overlay))
                        .border_1()
                        .border_color(color(tokens.line_strong))
                        .shadow(vec![gpui::BoxShadow {
                            color: gpui::Rgba {
                                r: 0.,
                                g: 0.,
                                b: 0.,
                                a: 0.45,
                            }
                            .into(),
                            offset: gpui::point(px(0.), px(10.)),
                            blur_radius: px(28.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        // Clicking inside must not reach the scrim's dismiss.
                        .occlude()
                        .child(
                            div()
                                .px(px(12.))
                                .child(gpui_component::input::Input::new(&self.inputs.palette)),
                        )
                        .child(results)
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(12.))
                                .px(px(12.))
                                .child(
                                    div()
                                        .flex_grow(1.0)
                                        .text_size(ui(8.5))
                                        .line_height((ui(8.5)) * 1.3)
                                        .text_color(color(tokens.muted))
                                        .child("↑/↓ or Tab Navigate  ·  Enter Run  ·  Esc Close"),
                                )
                                .child(
                                    div()
                                        .font_family(terminal_family(settings))
                                        .text_size(ui(8.0))
                                        .line_height((ui(8.0)) * 1.3)
                                        .text_color(color(tokens.accent))
                                        .child(if cfg!(target_os = "macos") {
                                            "Cmd+P"
                                        } else {
                                            "Ctrl+P"
                                        }),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The bottom toast, while it is still fresh.
    ///
    /// A live keyboard mode is a state the user is standing in until they
    /// leave it, so it is drawn in the accent the rest of the chrome already
    /// uses for "this is where you are". A toast keeps the quiet neutral
    /// surface: it leaves on its own and must not pull the eye off the
    /// terminal to do it.
    pub(crate) fn toast(&self) -> Option<AnyElement> {
        let app = self.app();
        let (message, keyboard_mode) = app.feedback_message()?;
        // A mode stays up for as long as the user is in it; a toast fades.
        if !keyboard_mode
            && app
                .toast
                .as_ref()
                .is_some_and(|(_, raised)| raised.elapsed() > TOAST_LIFETIME)
        {
            return None;
        }
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        let (edge, text, edge_width) = if keyboard_mode {
            (tokens.accent, tokens.accent, 1.5)
        } else {
            (tokens.line_strong, tokens.text, 1.0)
        };
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
                        .mb(px(26.))
                        .py(px(7.))
                        .px(px(14.))
                        .rounded(px(999.))
                        .bg(color(tokens.overlay))
                        .border(px(edge_width))
                        .border_color(color(edge))
                        .shadow(vec![gpui::BoxShadow {
                            color: gpui::Rgba {
                                r: 0.,
                                g: 0.,
                                b: 0.,
                                a: 0.45,
                            }
                            .into(),
                            offset: gpui::point(px(0.), px(4.)),
                            blur_radius: px(16.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .text_size(px(app.settings.ui_pixels(9.5)))
                        .line_height((px(app.settings.ui_pixels(9.5))) * 1.3)
                        .text_color(color(text))
                        .child(message.to_owned()),
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
                        .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                        .text_color(color(tokens.muted))
                        .truncate()
                        .child(app.status.clone()),
                )
                .child(
                    div()
                        .text_size(px(app.settings.ui_pixels(9.0)))
                        .line_height((px(app.settings.ui_pixels(9.0))) * 1.3)
                        .text_color(color(tokens.faint))
                        .child(format!("{panes} pane{}", if panes == 1 { "" } else { "s" })),
                )
                .into_any_element(),
        )
    }
}
