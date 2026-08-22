//! The command palette: fuzzy-free, ordered list of every command.
//!
//! Ordering and enablement come from [`crate::commands`]; this module only
//! draws the query field and the rows, and keeps the selected row visible.

use iced::widget::column;

use crate::views::prelude::*;

use crate::{
    PALETTE_INPUT_ID, PALETTE_SCROLL_ID, agent_display_name, commands, palette_button_style,
    selection_bar,
};
use iced::Font;

impl Muxtrix {
    pub(crate) fn command_palette(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let commands = commands::filtered(&self.palette.query);
        let results =
            commands
                .iter()
                .enumerate()
                .fold(column![].spacing(2), |list, (index, command)| {
                    let enabled = self.command_enabled(command.action);
                    let selected = enabled && index == self.palette.selected;
                    let shortcut: Element<'_, Message> = if command.shortcut.is_empty() {
                        container("").width(0).into()
                    } else {
                        text(command.shortcut)
                            .font(self.settings.terminal_font.iced())
                            .size(self.settings.ui_pixels(7.5))
                            .color(tokens.faint)
                            .into()
                    };
                    let title_color = if enabled { tokens.text } else { tokens.faint };
                    let configured_agent = self
                        .configured_default_agent()
                        .map(|agent| agent_display_name(&agent.to_string()).to_owned());
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
                            "Choose a configured default agent in Settings to use this command"
                                .to_owned()
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
                    let mut command_button = button(
                        row![
                            column![
                                text(title)
                                    .size(self.settings.ui_pixels(11.0))
                                    .font(Font {
                                        weight: self.default_family_weight(FontWeight::Medium),
                                        ..Font::DEFAULT
                                    })
                                    .color(title_color),
                                text(subtitle)
                                    .size(self.settings.ui_pixels(9.0))
                                    .color(if enabled { tokens.muted } else { tokens.faint }),
                            ]
                            .spacing(2)
                            .width(Fill),
                            shortcut,
                        ]
                        .align_y(Alignment::Center),
                    )
                    // Extra right inset keeps the chord hints clear
                    // of the overlay scrollbar.
                    .padding(Padding {
                        top: 7.0,
                        bottom: 7.0,
                        left: 12.0,
                        right: 24.0,
                    })
                    .width(Fill)
                    .style(move |_, status| {
                        palette_button_style(tokens, selected, enabled, status)
                    });
                    if enabled {
                        command_button = command_button.on_press(Message::CommandSelected(index));
                    }
                    list.push(
                        row![selection_bar(selected, tokens), command_button,]
                            .align_y(Alignment::Center),
                    )
                });
        let results: Element<'_, Message> = if commands.is_empty() {
            container(
                column![
                    text("No matching commands").size(self.settings.ui_pixels(11.0)),
                    text("Try searching for split, terminal, or settings.")
                        .size(self.settings.ui_pixels(9.0))
                        .color(tokens.muted),
                ]
                .spacing(4),
            )
            .padding(18)
            .into()
        } else {
            // Bounded so the keyboard-hint footer below always renders.
            container(scrollable(results).id(iced::widget::Id::new(PALETTE_SCROLL_ID)))
                .max_height(400)
                .into()
        };

        container(
            column![
                container(
                    text_input("Search commands…", &self.palette.query)
                        .id(iced::widget::Id::new(PALETTE_INPUT_ID))
                        .on_input(Message::CommandQueryChanged)
                        .padding(11)
                        .size(self.settings.ui_pixels(12.0))
                        .style(move |_, _| text_input::Style {
                            background: iced::Background::Color(tokens.app),
                            border: Border {
                                color: tokens.line_strong,
                                width: 1.0,
                                radius: 7.0.into(),
                            },
                            icon: tokens.muted,
                            placeholder: tokens.faint,
                            value: tokens.text,
                            selection: Color {
                                a: 0.35,
                                ..tokens.accent
                            },
                        }),
                )
                .padding(Padding {
                    top: 0.0,
                    bottom: 0.0,
                    left: 12.0,
                    right: 12.0,
                }),
                results,
                container(
                    row![
                        text("↑/↓ or Tab Navigate  ·  Enter Run  ·  Esc Close")
                            .size(self.settings.ui_pixels(8.5))
                            .color(tokens.muted)
                            .width(Fill),
                        text(if cfg!(target_os = "macos") {
                            "Cmd+P"
                        } else {
                            "Ctrl+P"
                        })
                        .font(self.settings.terminal_font.iced())
                        .size(self.settings.ui_pixels(8.0))
                        .color(tokens.accent),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .padding(Padding {
                    top: 0.0,
                    bottom: 0.0,
                    left: 12.0,
                    right: 12.0,
                }),
            ]
            .spacing(8),
        )
        .padding(Padding {
            top: 12.0,
            bottom: 10.0,
            left: 0.0,
            right: 0.0,
        })
        .width(620)
        .max_height(520)
        .style(move |_| {
            container::Style::default()
                .background(tokens.overlay)
                .border(Border {
                    color: tokens.line_strong,
                    width: 1.0,
                    radius: 10.0.into(),
                })
                .shadow(Shadow {
                    color: Color::from_rgba8(0, 0, 0, 0.45),
                    offset: Vector::new(0.0, 12.0),
                    blur_radius: 32.0,
                })
        })
        .into()
    }
}
