//! The terminal theme gallery: every preset, two per row, live-previewed.
//!
//! Previews are built from the same run model the real grid uses, so what the
//! gallery shows is what the terminal will render.

use iced::widget::column;

use crate::views::prelude::*;

use crate::app::{SettingsButtonKind, settings_action_button, terminal_theme_preview_with_caption};
use crate::themes::TerminalThemeId;
use iced::Font;

impl Muxtrix {
    /// Full-screen theme browser: every preset rendered as the same live
    /// terminal preview the settings page shows for the current theme —
    /// sample output, selection, cursor, and the full ANSI strip — two per
    /// row, clickable, Esc or Back to return. The catalog is bounded, so
    /// rendering all cards in the scrollable stays cheap.
    pub(crate) fn theme_gallery_view(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings_draft.appearance);
        let count_nudge = iced::Padding {
            top: (self.settings_draft.ui_pixels(18.0) - self.settings_draft.ui_pixels(9.5)) * 0.6,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        };
        let header = row![
            settings_action_button(
                "← Settings",
                Message::CloseThemeGallery,
                SettingsButtonKind::Secondary,
                &self.settings_draft,
            ),
            text("Theme gallery")
                .size(self.settings_draft.ui_pixels(18.0))
                .font(Font {
                    weight: font::Weight::Bold,
                    ..Font::DEFAULT
                })
                .color(tokens.text),
            container("").width(Fill),
            container(
                text(format!(
                    "Current: {} · {} themes",
                    self.settings_draft.terminal_theme.preset().name,
                    TerminalThemeId::ALL.len()
                ))
                .size(self.settings_draft.ui_pixels(9.5))
                .color(tokens.faint),
            )
            .padding(count_nudge),
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        // One column keeps the preview truthful when cards would squeeze
        // below the width the sample line and ANSI strip need.
        let columns: usize = if self.window_size.width < 980.0 { 1 } else { 2 };
        let mut grid = column![].spacing(12);
        let mut cards: Vec<Element<'_, Message>> = Vec::new();
        fn flush_row<'a>(
            mut grid: iced::widget::Column<'a, Message>,
            cards: &mut Vec<Element<'a, Message>>,
            columns: usize,
        ) -> iced::widget::Column<'a, Message> {
            if !cards.is_empty() {
                while cards.len() < columns {
                    cards.push(container("").width(Fill).into());
                }
                grid =
                    grid.push(iced::widget::Row::with_children(std::mem::take(cards)).spacing(12));
            }
            grid
        }
        let mut current_group: Option<bool> = None;
        // Dark first, then light: grouped so the mode chip on every card
        // becomes redundant and a named theme is findable by section.
        let ordered = TerminalThemeId::ALL
            .into_iter()
            .filter(|id| !id.preset().is_light)
            .chain(
                TerminalThemeId::ALL
                    .into_iter()
                    .filter(|id| id.preset().is_light),
            );
        for id in ordered {
            let preset = id.preset();
            if current_group != Some(preset.is_light) {
                grid = flush_row(grid, &mut cards, columns);
                current_group = Some(preset.is_light);
                grid = grid.push(
                    container(
                        text(if preset.is_light { "LIGHT" } else { "DARK" })
                            .size(self.settings_draft.ui_pixels(8.5))
                            .font(Font {
                                weight: font::Weight::Semibold,
                                ..Font::DEFAULT
                            })
                            .color(tokens.faint),
                    )
                    .padding(iced::Padding {
                        top: 8.0,
                        right: 0.0,
                        bottom: 0.0,
                        left: 3.0,
                    }),
                );
            }
            let selected = id == self.settings_draft.terminal_theme;
            let card = button(terminal_theme_preview_with_caption(
                preset,
                &self.settings_draft,
                false,
            ))
            .on_press(Message::GalleryThemeChosen(id))
            .padding(3)
            .width(Fill)
            .style(move |_, status| iced::widget::button::Style {
                background: None,
                border: Border {
                    color: if selected {
                        tokens.accent
                    } else if matches!(status, iced::widget::button::Status::Hovered) {
                        tokens.line_strong
                    } else {
                        Color::TRANSPARENT
                    },
                    width: 2.0,
                    radius: 12.0.into(),
                },
                ..iced::widget::button::Style::default()
            });
            cards.push(card.into());
            if cards.len() == columns {
                grid = grid
                    .push(iced::widget::Row::with_children(std::mem::take(&mut cards)).spacing(12));
            }
        }
        grid = flush_row(grid, &mut cards, columns);
        container(
            column![
                header,
                scrollable(container(grid).padding(iced::Padding {
                    top: 4.0,
                    right: 14.0,
                    bottom: 24.0,
                    left: 0.0,
                }))
                .height(Fill),
            ]
            .spacing(18),
        )
        .padding(28)
        .width(Fill)
        .height(Fill)
        .style(move |_| container::Style::default().background(tokens.app))
        .into()
    }
}
