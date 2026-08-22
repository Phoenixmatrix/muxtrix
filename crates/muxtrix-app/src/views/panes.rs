//! Pane rendering: the split tree, stacked-pane sheets, and pane chrome.
//!
//! [`Muxtrix::view_tree`] walks the [`muxtrix_domain::PaneTree`] and emits a
//! nested row/column with a draggable divider at every branch. Leaves become
//! either a full pane or, when the layout is stacked, a header sheet.

use iced::widget::column;

use crate::views::prelude::*;

use crate::layout::expanded_stack_pane;
use crate::popover::Popover;
use crate::settings::font_with_style;
use crate::terminal::runs::rgb;
use crate::{
    PANE_HEADER_CHIP_PADDING, PANE_HEADER_CONTROL_SPACING, PANE_HEADER_DIVIDER,
    PANE_HEADER_FIXED_WIDTH, PANE_HEADER_ICON_BUTTON, PANE_HEADER_LABEL_PADDING,
    PANE_TITLE_MIN_WIDTH, PANE_TITLE_UNMEASURED_WIDTH, SPLIT_HANDLE_SIZE, SplitBranch, SplitKey,
    TerminalLaunchState, UI_TEXT_ADVANCE_RATIO, app_tooltip, centered_button_label, commands,
    ellipsize, pane_header_is_compact, pane_menu_divider, pane_menu_entry, pane_title_char_budget,
    quiet_button_style, styled_terminal, terminal_empty_state_copy, terminal_link_modifiers,
    terminal_mouse_interaction, terminal_scrollbar, terminal_surface_background,
};
use iced::Font;
use iced::mouse;
use iced::widget::sensor;
use muxtrix_control::AgentState;
use muxtrix_domain::{PaneId, PaneTree, SplitAxis, Workspace, WorkspaceTab};
use muxtrix_terminal::TerminalMouseButton;
use std::collections::BTreeMap;

impl Muxtrix {
    pub(crate) fn view_tree<'a>(
        &'a self,
        workspace: &'a Workspace,
        tab: &'a WorkspaceTab,
        tree: &'a PaneTree,
        path: Vec<SplitBranch>,
    ) -> Element<'a, Message> {
        match tree {
            PaneTree::Leaf { pane_id } => self.view_pane(workspace, tab, *pane_id),
            PaneTree::Stack { pane_ids } => self.view_pane_stack(workspace, tab, pane_ids),
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
                let first = container(self.view_tree(workspace, tab, first, first_path));
                let second_ratio = 1_000 - ratio.permille();
                let second = container(self.view_tree(workspace, tab, second, second_path));
                let dragging = self.split_drag.as_ref().is_some_and(|drag| drag.key == key);
                let handle_color = if dragging {
                    DesignTokens::for_appearance(self.settings.appearance).accent
                } else {
                    Color::TRANSPARENT
                };
                let content: Element<'_, Message> = match axis {
                    SplitAxis::Horizontal => {
                        let handle = mouse_area(
                            container(
                                container("")
                                    .width(if dragging { 2.0 } else { 1.0 })
                                    .height(Fill)
                                    .style(move |_| {
                                        container::Style::default().background(handle_color)
                                    }),
                            )
                            .width(SPLIT_HANDLE_SIZE)
                            .height(Fill)
                            .align_x(iced::alignment::Horizontal::Center),
                        )
                        .on_press(Message::BeginSplitDrag(key.clone(), *axis))
                        .interaction(mouse::Interaction::ResizingHorizontally);
                        row![
                            first.width(Length::FillPortion(ratio.permille())),
                            handle,
                            second.width(Length::FillPortion(second_ratio)),
                        ]
                        .into()
                    }
                    SplitAxis::Vertical => {
                        let handle = mouse_area(
                            container(
                                container("")
                                    .width(Fill)
                                    .height(if dragging { 2.0 } else { 1.0 })
                                    .style(move |_| {
                                        container::Style::default().background(handle_color)
                                    }),
                            )
                            .width(Fill)
                            .height(SPLIT_HANDLE_SIZE)
                            .align_y(iced::alignment::Vertical::Center),
                        )
                        .on_press(Message::BeginSplitDrag(key.clone(), *axis))
                        .interaction(mouse::Interaction::ResizingVertically);
                        column![
                            first.height(Length::FillPortion(ratio.permille())),
                            handle,
                            second.height(Length::FillPortion(second_ratio)),
                        ]
                        .into()
                    }
                };
                sensor(content)
                    .key(key.clone())
                    .on_resize(move |size| Message::ResizeSplit(key.clone(), size))
                    .into()
            }
        }
    }

    pub(crate) fn view_pane_stack<'a>(
        &'a self,
        workspace: &'a Workspace,
        tab: &'a WorkspaceTab,
        pane_ids: &'a [PaneId],
    ) -> Element<'a, Message> {
        let mut sheets = column![].spacing(3).height(Fill);
        let expanded_pane_id = expanded_stack_pane(pane_ids, tab.focused_pane_id);
        for (index, pane_id) in pane_ids.iter().copied().enumerate() {
            if Some(pane_id) == expanded_pane_id {
                sheets = sheets.push(self.view_pane(workspace, tab, pane_id));
            } else {
                sheets = sheets.push(self.view_stacked_pane_header(workspace, tab, pane_id, index));
            }
        }
        sheets.into()
    }

    pub(crate) fn view_stacked_pane_header<'a>(
        &'a self,
        workspace: &'a Workspace,
        tab: &'a WorkspaceTab,
        pane_id: PaneId,
        index: usize,
    ) -> Element<'a, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let Some(pane) = tab.panes.get(&pane_id) else {
            return container(text("Missing pane")).height(31).into();
        };
        let needs_attention = self.pane_needs_attention(pane_id, pane.attention.unread_count);
        let signal_kind = self.pane_signal_kind(pane_id, needs_attention);
        let title = self.pane_title(workspace, pane_id);
        let state = self.pane_state_label(pane_id);
        // A three-step inset keeps large stacks from tapering indefinitely,
        // while making the exposed title sheets read as physical layers.
        let inset = (index % 3) as f32 * 2.0;
        container(
            button(
                row![
                    self.pane_pip(pane_id, signal_kind.color(tokens), 6.0),
                    text(ellipsize(title, self.settings.ui_char_budget(32)))
                        .size(self.settings.ui_pixels(9.0))
                        .font(Font {
                            weight: self.default_family_weight(FontWeight::Medium),
                            ..Font::DEFAULT
                        })
                        .color(tokens.muted)
                        .wrapping(iced::widget::text::Wrapping::None),
                    container("").width(Fill),
                    text(state)
                        .size(self.settings.ui_pixels(8.5))
                        .color(signal_kind.label_color(tokens))
                        .wrapping(iced::widget::text::Wrapping::None),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .height(Fill),
            )
            .on_press(Message::Focus(pane_id))
            .height(31)
            .padding(Padding {
                top: 0.0,
                bottom: 0.0,
                left: 11.0,
                right: 11.0,
            })
            .width(Fill)
            .style(move |_, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(iced::Background::Color(if hovered {
                        Color {
                            a: 0.08,
                            ..tokens.text
                        }
                    } else {
                        tokens.panel_raised
                    })),
                    text_color: tokens.text,
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: 7.0.into(),
                    },
                    shadow: Shadow {
                        color: Color::from_rgba8(0, 0, 0, 0.32),
                        offset: Vector::new(0.0, 4.0),
                        blur_radius: 10.0,
                    },
                    snap: true,
                }
            }),
        )
        .padding(Padding {
            top: 0.0,
            bottom: 0.0,
            left: inset,
            right: 4.0 - inset.min(4.0),
        })
        .height(31)
        .width(Fill)
        .into()
    }

    pub(crate) fn view_pane<'a>(
        &'a self,
        workspace: &'a Workspace,
        tab: &'a WorkspaceTab,
        pane_id: PaneId,
    ) -> Element<'a, Message> {
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let Some(pane) = tab.panes.get(&pane_id) else {
            return container(text("Missing pane"))
                .width(Fill)
                .height(Fill)
                .into();
        };
        let focused = workspace.id == self.session.active_workspace_id
            && workspace.active_tab_id == tab.id
            && tab.focused_pane_id == pane_id;
        let needs_attention = self.pane_needs_attention(pane_id, pane.attention.unread_count);
        let runtime = self.terminals.get(&pane_id);
        let process_exited = runtime
            .is_some_and(|runtime| matches!(runtime.launch_state, TerminalLaunchState::Exited));
        let launch_failed = runtime
            .is_some_and(|runtime| matches!(runtime.launch_state, TerminalLaunchState::Failed(_)));
        let launch_pending = runtime.is_some_and(|runtime| {
            matches!(
                runtime.launch_state,
                TerminalLaunchState::PreparingHost | TerminalLaunchState::Starting { .. }
            )
        });
        let launch_suppressed = runtime
            .is_some_and(|runtime| matches!(runtime.launch_state, TerminalLaunchState::Suppressed));
        let title = self.pane_title(workspace, pane_id);
        let state = self.pane_state_label(pane_id);
        let compact_header = pane_header_is_compact(self.window_size.width, tab.panes.len());
        let signal_kind = self.pane_signal_kind(pane_id, needs_attention);
        let signal = signal_kind.color(tokens);
        let hovered_link = terminal_link_modifiers(self.keyboard_modifiers)
            .then(|| self.hovered_terminal_link(pane_id))
            .flatten();
        let no_image_handles = BTreeMap::new();
        let image_handles = runtime.map_or(&no_image_handles, |runtime| &runtime.image_handles);
        let snapshot = runtime.and_then(|runtime| runtime.snapshot.as_ref());
        let terminal_content: Element<'_, Message> = match snapshot {
            Some(snapshot) => styled_terminal(
                snapshot,
                image_handles,
                focused,
                self.cursor_phase_visible,
                hovered_link.as_ref(),
                &self.settings,
            ),
            None => terminal_empty_state_copy(runtime).map_or_else(
                || container(row![]).into(),
                |copy| {
                    text(copy)
                        .font(font_with_style(
                            self.settings.terminal_font.iced(),
                            self.settings.terminal_font_weight.iced(),
                            font::Style::Normal,
                        ))
                        .size(self.settings.terminal_font_pixels())
                        .into()
                },
            ),
        };
        let terminal_background = rgb(terminal_surface_background(
            snapshot,
            self.settings.terminal_theme.preset(),
        ));
        let terminal_canvas: Element<'_, Message> = container(terminal_content)
            .padding(8)
            .width(Fill)
            .height(Fill)
            .clip(true)
            .style(move |_| {
                iced::widget::container::Style::default()
                    .background(terminal_background)
                    .border(Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: iced::border::Radius {
                            top_left: 0.0,
                            top_right: 0.0,
                            bottom_right: 9.0,
                            bottom_left: 9.0,
                        },
                    })
            })
            .into();
        let scrollbar: Element<'_, Message> = runtime
            .and_then(|runtime| runtime.snapshot.as_ref())
            .filter(|snapshot| {
                self.hovered_terminal == Some(pane_id) && snapshot.scrollbar.is_scrollable()
            })
            .map_or_else(
                || container("").width(0).into(),
                |snapshot| {
                    terminal_scrollbar(
                        pane_id,
                        snapshot.scrollbar,
                        runtime.and_then(|runtime| runtime.viewport).map_or(
                            f32::from(runtime.map_or(24, |runtime| runtime.size.rows))
                                * self.settings.terminal_cell_height()
                                + 16.0,
                            |viewport| viewport.height,
                        ),
                        tokens,
                    )
                },
            );
        let terminal_view = mouse_area(stack([terminal_canvas, scrollbar]))
            .on_enter(Message::EnterTerminal(pane_id))
            .on_exit(Message::LeaveTerminal(pane_id))
            .on_move(move |position| Message::TerminalPointerMoved(pane_id, position))
            .on_press(Message::TerminalMousePressed(
                pane_id,
                TerminalMouseButton::Left,
            ))
            .on_release(Message::TerminalMouseReleased(
                pane_id,
                TerminalMouseButton::Left,
            ))
            .on_middle_press(Message::TerminalMousePressed(
                pane_id,
                TerminalMouseButton::Middle,
            ))
            .on_middle_release(Message::TerminalMouseReleased(
                pane_id,
                TerminalMouseButton::Middle,
            ))
            .on_right_press(Message::TerminalMousePressed(
                pane_id,
                TerminalMouseButton::Right,
            ))
            .on_right_release(Message::TerminalMouseReleased(
                pane_id,
                TerminalMouseButton::Right,
            ))
            .on_scroll(move |delta| Message::ScrollTerminal(pane_id, delta))
            .interaction(terminal_mouse_interaction(hovered_link.is_some()));
        // A restored session can replace a terminal at the same tree position
        // without changing its bounds. `on_resize` alone retains the previous
        // sensor size and emits nothing; `on_show` makes a pane-key change
        // replay that unchanged viewport into the replacement runtime.
        let terminal_view = sensor(terminal_view)
            .key(pane_id)
            .on_show(move |size| Message::ResizePane(pane_id, size))
            .on_resize(move |size| Message::ResizePane(pane_id, size));
        let header_button = |kind: IconKind,
                             label: &'static str,
                             message: Message,
                             danger: bool|
         -> Element<'static, Message> {
            app_tooltip(
                button(
                    container(icon(kind, tokens.muted, 12.0))
                        .width(Fill)
                        .height(Fill)
                        .center_x(Fill)
                        .align_y(iced::alignment::Vertical::Center),
                )
                .on_press(message)
                .width(24)
                .height(24)
                .padding(0)
                .style(move |_, status| {
                    let hovered =
                        matches!(status, button::Status::Hovered | button::Status::Pressed);
                    let background = if !hovered {
                        None
                    } else if danger {
                        Some(Color {
                            a: 0.14,
                            ..tokens.danger
                        })
                    } else {
                        Some(Color {
                            a: 0.07,
                            ..tokens.text
                        })
                    };
                    button::Style {
                        background: background.map(iced::Background::Color),
                        text_color: tokens.text,
                        border: Border::default().rounded(5.0),
                        shadow: Shadow::default(),
                        snap: true,
                    }
                }),
                label,
                tooltip::Position::Bottom,
                tokens,
                12.0,
            )
        };
        // The header row never clips, so every element right of the title has
        // to be paid for out of the title's width budget below.
        let (command_chip, command_chip_width): (Element<'_, Message>, f32) = {
            match self.pane_program(pane_id).filter(|_| !compact_header) {
                None => (container("").width(0).into(), 0.0),
                Some(command) => {
                    let chip_size = self.settings.ui_pixels(7.5);
                    let width = command.chars().count() as f32
                        * chip_size
                        * self.settings.terminal_advance_ratio()
                        + PANE_HEADER_CHIP_PADDING;
                    let chip = container(
                        text(command)
                            .font(self.settings.terminal_font.iced())
                            .size(chip_size)
                            .color(tokens.muted),
                    )
                    .padding([1, 6])
                    .style(move |_| {
                        container::Style::default()
                            .background(Color {
                                a: 0.05,
                                ..tokens.text
                            })
                            .border(Border::default().rounded(4.0))
                    })
                    .into();
                    (chip, width)
                }
            }
        };
        let (state_label, state_label_width): (Element<'_, Message>, f32) = if compact_header
            || state == "Shell"
        {
            (container("").width(0).into(), 0.0)
        } else {
            let width =
                state.chars().count() as f32 * self.settings.ui_pixels(9.0) * UI_TEXT_ADVANCE_RATIO;
            (
                text(state)
                    .size(self.settings.ui_pixels(9.0))
                    .color(signal_kind.label_color(tokens))
                    .wrapping(iced::widget::text::Wrapping::None)
                    .into(),
                width,
            )
        };
        // Every control is a fixed-size icon button or a labelled button whose
        // width follows its copy; tally them as they are pushed so the title
        // budget cannot drift from what the row actually renders.
        let mut controls_width = 0.0_f32;
        let mut push_control_width = |width: f32| {
            controls_width += width + PANE_HEADER_CONTROL_SPACING;
        };
        let labelled_control_width = |label: &str| {
            label.chars().count() as f32 * self.settings.ui_pixels(9.0) * UI_TEXT_ADVANCE_RATIO
                + PANE_HEADER_LABEL_PADDING
        };
        let mut controls = row![].spacing(2).align_y(Alignment::Center);
        if !compact_header {
            if self.maximized_pane.is_none() {
                for _ in 0..2 {
                    push_control_width(PANE_HEADER_ICON_BUTTON);
                }
                controls = controls
                    .push(header_button(
                        IconKind::SplitRight,
                        "Split right",
                        Message::SplitFrom(pane_id, SplitAxis::Horizontal),
                        false,
                    ))
                    .push(header_button(
                        IconKind::SplitDown,
                        "Split down",
                        Message::SplitFrom(pane_id, SplitAxis::Vertical),
                        false,
                    ));
            }
            push_control_width(PANE_HEADER_ICON_BUTTON);
            controls = controls.push(header_button(
                if self.maximized_pane == Some(pane_id) {
                    IconKind::Restore
                } else {
                    IconKind::Maximize
                },
                if self.maximized_pane == Some(pane_id) {
                    "Restore panes"
                } else {
                    "Maximize pane"
                },
                Message::ToggleMaximize(pane_id),
                false,
            ));
        }
        if (process_exited || launch_failed) && !compact_header {
            push_control_width(labelled_control_width("Restart"));
            controls = controls.push(
                button(centered_button_label(
                    "Restart",
                    self.settings.ui_pixels(9.0),
                ))
                .on_press(Message::RestartPane(pane_id))
                .height(24)
                .padding([0, 8])
                .style(move |_, status| quiet_button_style(tokens, false, status)),
            );
        }
        if launch_pending && !compact_header {
            push_control_width(labelled_control_width("Cancel"));
            controls = controls.push(
                button(centered_button_label(
                    "Cancel",
                    self.settings.ui_pixels(9.0),
                ))
                .on_press(Message::CancelTerminalLaunch(pane_id))
                .height(24)
                .padding([0, 8])
                .style(move |_, status| quiet_button_style(tokens, false, status)),
            );
        }
        if launch_suppressed && !compact_header {
            push_control_width(labelled_control_width("Start terminal"));
            controls = controls.push(
                button(centered_button_label(
                    "Start terminal",
                    self.settings.ui_pixels(9.0),
                ))
                .on_press(Message::StartTerminal(pane_id))
                .height(24)
                .padding([0, 8])
                .style(move |_, status| quiet_button_style(tokens, false, status)),
            );
        }
        if !compact_header {
            push_control_width(PANE_HEADER_DIVIDER);
            controls = controls.push(
                container("")
                    .width(1)
                    .height(14)
                    .style(move |_| container::Style::default().background(tokens.line_strong)),
            );
        }
        push_control_width(PANE_HEADER_ICON_BUTTON);
        push_control_width(PANE_HEADER_ICON_BUTTON);
        controls = controls
            .push(header_button(
                IconKind::Overflow,
                "More pane actions",
                Message::TogglePaneMenu(pane_id),
                false,
            ))
            .push(header_button(
                IconKind::Close,
                if tab.panes.len() == 1 {
                    "Close pane and tab"
                } else {
                    "Close pane"
                },
                Message::ClosePane(pane_id),
                true,
            ));
        // A fixed character budget cannot know how much room the trailing
        // chrome actually left, so it has to assume the worst and truncate the
        // title far sooner than the card requires. The measured card width
        // knows better: spend everything the chrome does not claim. Panes that
        // have not reported a size yet keep the conservative budget.
        let title_space = runtime
            .and_then(|runtime| runtime.viewport)
            .map(|viewport| {
                (viewport.width
                    - PANE_HEADER_FIXED_WIDTH
                    - command_chip_width
                    - state_label_width
                    - controls_width)
                    .max(PANE_TITLE_MIN_WIDTH)
            });
        let title_budget = title_space.map_or_else(
            || self.settings.ui_char_budget(24),
            |space| {
                pane_title_char_budget(space, self.settings.ui_pixels(9.0) * UI_TEXT_ADVANCE_RATIO)
            },
        );
        // The card's header: rounded top corners carry the card radius, and
        // the whole band shares one fill so it can never render two-toned.
        let pane_bar = column![
            container(
                row![
                    self.pane_pip(pane_id, signal, 6.0),
                    // The ellipsis is placed from an averaged advance, so a
                    // title of unusually wide glyphs can still outrun its
                    // budget. Bounding the title to the same space it was
                    // budgeted keeps that error inside the title instead of
                    // shoving the state label and controls off the card.
                    container(
                        text(ellipsize(title, title_budget))
                            .size(self.settings.ui_pixels(9.0))
                            .font(Font {
                                weight: self.default_family_weight(FontWeight::Medium),
                                ..Font::DEFAULT
                            })
                            .color(if focused { tokens.text } else { tokens.muted })
                            .wrapping(iced::widget::text::Wrapping::None)
                    )
                    .max_width(title_space.unwrap_or(PANE_TITLE_UNMEASURED_WIDTH))
                    .clip(true),
                    command_chip,
                    container("").width(Fill),
                    state_label,
                    controls,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .height(34)
            .align_y(iced::alignment::Vertical::Center)
            .padding(Padding {
                top: 0.0,
                bottom: 0.0,
                left: 12.0,
                right: 6.0,
            })
            .style(move |_| {
                let style = container::Style::default().border(Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: iced::border::Radius {
                        top_left: 9.0,
                        top_right: 9.0,
                        bottom_right: 0.0,
                        bottom_left: 0.0,
                    },
                });
                if focused {
                    style.background(tokens.panel_raised)
                } else {
                    style.background(Color {
                        a: 0.03,
                        ..tokens.text
                    })
                }
            }),
            container("")
                .height(1)
                .width(Fill)
                .style(move |_| container::Style::default().background(tokens.line)),
        ]
        .height(35);
        let pane_menu: Option<Element<'_, Message>> = if self.pane_menu == Some(pane_id) {
            // Copy and Paste stay present but disabled when they cannot act,
            // so every row keeps a fixed position regardless of invisible
            // selection or process state.
            // Asking the emulator for the text would mean a round trip to the
            // session thread on every frame the menu is open; the flag tracks
            // the same answer from this side of the channel.
            let can_copy = runtime.is_some_and(|runtime| runtime.has_selection);
            let can_paste = runtime.is_some_and(|runtime| runtime.session.is_some());
            let mut entries = column![
                pane_menu_entry(
                    "Copy",
                    commands::COPY_SHORTCUT,
                    can_copy.then_some(Message::CopyTerminalSelection(pane_id)),
                    false,
                    tokens,
                    &self.settings,
                ),
                pane_menu_entry(
                    "Paste",
                    commands::PASTE_SHORTCUT,
                    can_paste.then_some(Message::PastePane(pane_id)),
                    false,
                    tokens,
                    &self.settings,
                ),
                pane_menu_divider(tokens),
            ]
            .spacing(2);
            if self.maximized_pane.is_none() {
                entries = entries
                    .push(pane_menu_entry(
                        "Split right",
                        "Ctrl+Shift+E",
                        Some(Message::SplitFrom(pane_id, SplitAxis::Horizontal)),
                        false,
                        tokens,
                        &self.settings,
                    ))
                    .push(pane_menu_entry(
                        "Split down",
                        "Ctrl+Shift+O",
                        Some(Message::SplitFrom(pane_id, SplitAxis::Vertical)),
                        false,
                        tokens,
                        &self.settings,
                    ));
            }
            entries = entries
                .push(pane_menu_entry(
                    if self.maximized_pane == Some(pane_id) {
                        "Restore panes"
                    } else {
                        "Maximize pane"
                    },
                    "Ctrl+Shift+M",
                    Some(Message::ToggleMaximizeFromPaneMenu(pane_id)),
                    false,
                    tokens,
                    &self.settings,
                ))
                .push(pane_menu_divider(tokens))
                .push(pane_menu_entry(
                    "Restart in worktree…",
                    "",
                    Some(Message::OpenPaneWorktreePrompt(pane_id)),
                    false,
                    tokens,
                    &self.settings,
                ))
                .push(pane_menu_entry(
                    "Restart terminal",
                    "",
                    Some(Message::RestartPane(pane_id)),
                    false,
                    tokens,
                    &self.settings,
                ))
                .push(pane_menu_entry(
                    if tab.panes.len() == 1 {
                        "Close pane and tab"
                    } else {
                        "Close pane"
                    },
                    "",
                    Some(Message::ClosePane(pane_id)),
                    true,
                    tokens,
                    &self.settings,
                ));
            Some(
                container(entries)
                    .width(236)
                    .padding(5)
                    .style(move |_| {
                        container::Style::default()
                            .background(tokens.overlay)
                            .border(Border {
                                color: tokens.line_strong,
                                width: 1.0,
                                radius: 10.0.into(),
                            })
                            .shadow(Shadow {
                                color: Color::from_rgba8(0, 0, 0, 0.5),
                                offset: Vector::new(0.0, 16.0),
                                blur_radius: 40.0,
                            })
                    })
                    .into(),
            )
        } else {
            None
        };
        let terminal: Element<'_, Message> = Popover::new(
            column![pane_bar, terminal_view].height(Fill),
            pane_menu,
            Message::DismissPaneMenu,
        )
        .into();

        // Any pane that requires the user — an agent waiting on input or a
        // terminal with unread attention — gets the full amber ring so it is
        // findable at a glance even while visible on screen.
        let awaiting_input = self
            .agent_statuses
            .get(&pane_id)
            .is_some_and(|status| status.state == AgentState::Waiting)
            || needs_attention;
        // Panes are rounded cards. A pane that needs a person carries a full
        // amber border and glow — the whole card, not just an edge — with
        // focus as the accent-blue equivalent beneath it in priority.
        let (edge, glow) = if awaiting_input {
            (
                Color {
                    a: 0.75,
                    ..tokens.warning
                },
                Shadow {
                    color: Color {
                        a: 0.22,
                        ..tokens.warning
                    },
                    offset: Vector::new(0.0, 0.0),
                    blur_radius: 18.0,
                },
            )
        } else if focused {
            (
                Color {
                    a: 0.62,
                    ..tokens.accent
                },
                Shadow {
                    color: Color {
                        a: 0.14,
                        ..tokens.accent
                    },
                    offset: Vector::new(0.0, 5.0),
                    blur_radius: 16.0,
                },
            )
        } else {
            (tokens.line, Shadow::default())
        };
        mouse_area(
            container(terminal)
                .width(Fill)
                .height(Fill)
                .padding(1)
                .clip(true)
                .style(move |_| {
                    container::Style::default()
                        .background(tokens.panel)
                        .border(Border {
                            color: edge,
                            width: 1.0,
                            radius: 10.0.into(),
                        })
                        .shadow(glow)
                }),
        )
        .on_press(Message::Focus(pane_id))
        .on_right_press(Message::OpenPaneContextMenu(pane_id))
        .into()
    }
}
