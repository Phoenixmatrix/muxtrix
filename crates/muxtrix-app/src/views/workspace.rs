//! The workspace surface: app bar, tab strip, commands pill, status bar.
//!
//! Everything to the right of the sidebar and above the pane tree. The tab
//! strip owns drag-reorder; the rest is read-only chrome over the active
//! workspace.

use iced::widget::column;

use crate::views::prelude::*;

use crate::{
    add_tab_button_style, app_tooltip, centered_button_content, ellipsize, pane_icon_button,
    ruled_surface, signal_dot,
};
use iced::mouse;
use muxtrix_domain::Workspace;

impl Muxtrix {
    pub(crate) fn workspace_view(&self) -> Element<'_, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let content = match self.active_workspace() {
            Ok(workspace) => workspace.active_tab().map_or_else(
                || {
                    container(text("Workspace has no tabs"))
                        .width(Fill)
                        .height(Fill)
                        .into()
                },
                |tab| {
                    self.maximized_pane.map_or_else(
                        || self.view_tree(workspace, tab, &tab.root, Vec::new()),
                        |pane_id| self.view_pane(workspace, tab, pane_id),
                    )
                },
            ),
            Err(error) => container(text(error)).width(Fill).height(Fill).into(),
        };
        // One 44px bar: tab chips on the left, command and settings actions on
        // the right. Pane actions stay with their pane headers.
        let divider = || {
            container("")
                .width(1)
                .height(16)
                .style(move |_| container::Style::default().background(tokens.line_strong))
        };
        let toolbar = row![
            self.app_bar_tabs(tokens),
            self.commands_pill(tokens),
            divider(),
        ]
        .push(pane_icon_button(
            IconKind::Settings,
            "Settings",
            Message::OpenSettings,
            tokens,
        ))
        .padding([0, 10])
        .spacing(8)
        .align_y(Alignment::Center);
        let workspace_id = self.active_workspace().map(|workspace| workspace.id).ok();
        let tab_count = self
            .active_workspace()
            .map(|workspace| workspace.tabs.len())
            .unwrap_or_default();
        let mut bar = mouse_area(
            container(toolbar)
                .height(43)
                .align_y(iced::alignment::Vertical::Center)
                .style(move |_| container::Style::default().background(tokens.rail)),
        );
        if let Some(workspace_id) = workspace_id {
            bar = bar.on_enter(Message::TabDragOver(workspace_id, tab_count));
        }
        let mut layout = column![
            bar,
            container("")
                .height(1)
                .width(Fill)
                .style(move |_| container::Style::default().background(tokens.line)),
            container(content).padding(8),
        ]
        .height(Fill);
        if self.settings.show_status_bar {
            let panes = self
                .active_workspace()
                .ok()
                .and_then(Workspace::active_tab)
                .map_or(0, |tab| tab.panes.len());
            layout = layout.push(
                container(
                    row![
                        text(&self.status)
                            .size(self.settings.ui_pixels(9.0))
                            .color(tokens.muted)
                            .width(Fill),
                        text(format!("{panes} pane{}", if panes == 1 { "" } else { "s" }))
                            .size(self.settings.ui_pixels(9.0))
                            .color(tokens.faint),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .height(26)
                .padding([0, 10])
                .style(move |_| ruled_surface(tokens.rail, tokens.line)),
            );
        }
        container(layout)
            .width(Fill)
            .height(Fill)
            .style(move |_| container::Style::default().background(tokens.app))
            .into()
    }

    /// The Commands entry: icon, label, and the real keycap for the palette.
    pub(crate) fn commands_pill(&self, tokens: DesignTokens) -> Element<'_, Message> {
        let keycap = container(
            text(if cfg!(target_os = "macos") {
                "Cmd+P"
            } else {
                "Ctrl+P"
            })
            .font(self.settings.terminal_font.iced())
            .size(self.settings.ui_pixels(7.5))
            .color(tokens.muted),
        )
        .padding([1, 5])
        .style(move |_| {
            container::Style::default()
                .background(Color {
                    a: 0.05,
                    ..tokens.text
                })
                .border(Border {
                    color: tokens.line_strong,
                    width: 1.0,
                    radius: 4.0.into(),
                })
        });
        button(centered_button_content(
            row![
                icon(IconKind::Command, tokens.muted, 13.0),
                text("Commands")
                    .size(self.settings.ui_pixels(9.0))
                    .color(tokens.muted),
                keycap,
            ]
            .spacing(7)
            .align_y(Alignment::Center),
        ))
        .on_press(Message::ToggleCommandPalette)
        .height(29)
        .padding([0, 9])
        .style(move |_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(iced::Background::Color(Color {
                    a: if hovered { 0.07 } else { 0.04 },
                    ..tokens.text
                })),
                text_color: tokens.muted,
                border: Border {
                    color: tokens.line_strong,
                    width: 1.0,
                    radius: 7.0.into(),
                },
                shadow: Shadow::default(),
                snap: true,
            }
        })
        .into()
    }

    /// Tab chips inside the app bar: dot, name, and close inside one rounded
    /// chip, with the trailing add action.
    pub(crate) fn app_bar_tabs(&self, tokens: DesignTokens) -> Element<'_, Message> {
        let Ok(workspace) = self.active_workspace() else {
            return container("").width(Fill).into();
        };
        let mut tabs = row![].spacing(3).align_y(Alignment::Center);
        for (index, tab) in workspace.tabs.iter().enumerate() {
            let selected = tab.id == workspace.active_tab_id;
            let drop_target = self.tab_drag.is_some_and(|drag| {
                drag.target_workspace_id == workspace.id && drag.target_index == index
            });
            let signal_kind = self.tab_signal_kind(tab);
            let label = button(centered_button_content(
                row![
                    signal_dot(signal_kind.color(tokens), 6.0),
                    text(ellipsize(&tab.name, self.settings.ui_char_budget(20)))
                        .size(self.settings.ui_pixels(9.0))
                        .color(if selected { tokens.text } else { tokens.muted })
                        .wrapping(iced::widget::text::Wrapping::None),
                ]
                .spacing(7)
                .align_y(Alignment::Center),
            ))
            .on_press(Message::BeginTabDrag(workspace.id, tab.id, index))
            .height(27)
            .padding(Padding {
                top: 0.0,
                bottom: 0.0,
                left: 11.0,
                right: 4.0,
            })
            .style(move |_, _| button::Style {
                background: None,
                text_color: tokens.text,
                border: Border::default(),
                shadow: Shadow::default(),
                snap: true,
            });
            let close = app_tooltip(
                button(
                    container(icon(IconKind::Close, tokens.muted, 11.0))
                        .width(Fill)
                        .height(Fill)
                        .center_x(Fill)
                        .align_y(iced::alignment::Vertical::Center),
                )
                .on_press(Message::CloseTab(workspace.id, tab.id))
                .width(18)
                .height(18)
                .padding(0)
                .style(move |_, status| {
                    let hovered =
                        matches!(status, button::Status::Hovered | button::Status::Pressed);
                    button::Style {
                        background: hovered.then_some(iced::Background::Color(Color {
                            a: 0.10,
                            ..tokens.text
                        })),
                        text_color: tokens.text,
                        border: Border::default().rounded(4.0),
                        shadow: Shadow::default(),
                        snap: true,
                    }
                }),
                "Close tab",
                tooltip::Position::Bottom,
                tokens,
                self.settings.ui_pixels(9.0),
            );
            // The chip carries fill, border, and radius; label and close are
            // transparent children so the whole chip reads as one control.
            let chip = container(
                row![label, close]
                    .spacing(0)
                    .align_y(Alignment::Center)
                    .padding(Padding {
                        top: 0.0,
                        bottom: 0.0,
                        left: 0.0,
                        right: 5.0,
                    }),
            )
            .height(29)
            .align_y(iced::alignment::Vertical::Center)
            .style(move |_| {
                let (fill, edge) = if selected {
                    (0.08, tokens.line_strong)
                } else {
                    (0.03, tokens.line)
                };
                container::Style::default()
                    .background(Color {
                        a: fill,
                        ..tokens.text
                    })
                    .border(Border {
                        color: if drop_target { tokens.accent } else { edge },
                        width: 1.0,
                        radius: 7.0.into(),
                    })
            });
            tabs = tabs.push(
                mouse_area(chip)
                    .on_enter(Message::TabDragOver(workspace.id, index))
                    .interaction(mouse::Interaction::Grab),
            );
        }
        let add_tab = app_tooltip(
            button(
                container(icon(IconKind::Add, tokens.muted, 13.0))
                    .width(Fill)
                    .height(Fill)
                    .center_x(Fill)
                    .align_y(iced::alignment::Vertical::Center),
            )
            .on_press(Message::NewTab)
            .width(26)
            .height(26)
            .padding(0)
            .style(move |_, status| add_tab_button_style(tokens, status)),
            "New tab",
            tooltip::Position::Bottom,
            tokens,
            self.settings.ui_pixels(9.0),
        );
        container(
            scrollable(
                row![tabs, add_tab]
                    .spacing(3)
                    .align_y(Alignment::Center)
                    .padding(Padding {
                        top: 0.0,
                        bottom: 0.0,
                        left: 0.0,
                        right: 8.0,
                    }),
            )
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::hidden(),
            )),
        )
        .width(Fill)
        .align_y(iced::alignment::Vertical::Center)
        .into()
    }
}
