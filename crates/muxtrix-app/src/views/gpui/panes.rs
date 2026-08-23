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
    terminal_surface_background,
};
use crate::commands;
use crate::layout::expanded_stack_pane;
use crate::runtime::gpui::{Root, color};
use crate::terminal::element::TerminalElement;
use crate::terminal::runs::rgb;
use crate::theme::DesignTokens;
use crate::views::gpui::{icon_button, pane_key, terminal_family};

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
        let needs_attention = tab
            .panes
            .get(&pane_id)
            .is_some_and(|pane| app.pane_needs_attention(pane_id, pane.attention.unread_count));

        // The surface takes the terminal's own background, not the chrome's:
        // a program that sets one (OSC 11) is honoured, and otherwise the
        // theme's, which is what makes a light appearance still host a dark
        // terminal. The grid only paints cells that carry a colour of their
        // own, so this is what shows through everywhere else.
        let surface = color(rgb(terminal_surface_background(
            runtime.and_then(|runtime| runtime.snapshot.as_ref()),
            app.settings.terminal_theme.preset(),
        )));
        let body = match TerminalElement::for_pane(
            app,
            pane_id,
            focused,
            cx.entity(),
            self.images().clone(),
        ) {
            // The relative box the grid pins itself to.
            Some(terminal) => div()
                .relative()
                .size_full()
                .overflow_hidden()
                .bg(surface)
                .child(terminal)
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

        // Panes are rounded cards. A pane that needs a person carries a full
        // amber border and glow — the whole card, not just an edge — with
        // focus as the accent-blue equivalent beneath it in priority.
        let awaiting_input = app
            .agent_statuses
            .get(&pane_id)
            .is_some_and(|status| status.state == muxtrix_control::AgentState::Waiting)
            || needs_attention;
        let (edge, glow) = if awaiting_input {
            let mut edge = color(tokens.warning);
            edge.a = 0.75;
            let mut glow = color(tokens.warning);
            glow.a = 0.22;
            (edge, Some((glow, 0.0, 18.0)))
        } else if focused {
            let mut edge = color(tokens.accent);
            edge.a = 0.62;
            let mut glow = color(tokens.accent);
            glow.a = 0.14;
            (edge, Some((glow, 5.0, 16.0)))
        } else {
            (color(tokens.line), None)
        };
        let mut card = div()
            .flex()
            .flex_col()
            .size_full()
            .p(px(1.))
            .rounded(px(10.))
            .border_1()
            .border_color(edge)
            .bg(color(tokens.panel))
            .overflow_hidden();
        if let Some((glow, offset_y, blur)) = glow {
            card = card.shadow(vec![gpui::BoxShadow {
                color: glow.into(),
                offset: gpui::point(px(0.), px(offset_y)),
                blur_radius: px(blur),
                spread_radius: px(0.),
                inset: false,
            }]);
        }
        card.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                root.dispatch(Message::Focus(pane_id), window, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                root.dispatch(Message::OpenPaneContextMenu(pane_id), window, cx);
            }),
        )
        .child(self.pane_header(workspace, tab, pane_id, focused, cx))
        .child(
            div()
                .flex_grow(1.0)
                .overflow_hidden()
                .rounded_b(px(9.))
                .child(body),
        )
        .into_any_element()
    }

    /// The card's header: rounded top corners carry the card radius, and the
    /// whole band shares one fill so it can never render two-toned. Geometry
    /// follows the iced header to the pixel: a 34 px band over a 1 px rule.
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
        let needs_attention = tab
            .panes
            .get(&pane_id)
            .is_some_and(|pane| app.pane_needs_attention(pane_id, pane.attention.unread_count));
        let signal_kind = app.pane_signal_kind(pane_id, needs_attention);
        let signal = signal_kind.color(tokens);
        let state = app.pane_state_label(pane_id);
        let compact = crate::app::pane_header_is_compact(app.window_size.width, tab.panes.len());
        let runtime = app.terminals.get(&pane_id);
        let process_exited = runtime.is_some_and(|runtime| {
            matches!(
                runtime.launch_state,
                crate::app::TerminalLaunchState::Exited
            )
        });
        let launch_failed = runtime.is_some_and(|runtime| {
            matches!(
                runtime.launch_state,
                crate::app::TerminalLaunchState::Failed(_)
            )
        });
        let launch_pending = runtime.is_some_and(|runtime| {
            matches!(
                runtime.launch_state,
                crate::app::TerminalLaunchState::PreparingHost
                    | crate::app::TerminalLaunchState::Starting { .. }
            )
        });
        let launch_suppressed = runtime.is_some_and(|runtime| {
            matches!(
                runtime.launch_state,
                crate::app::TerminalLaunchState::Suppressed
            )
        });
        let ui_size = px(app.settings.ui_pixels(9.0));

        let mut controls = div().flex().flex_row().items_center().gap(px(2.));
        if !compact {
            if app.maximized_pane.is_none() {
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
            controls = controls.child(self.header_button(
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
            ));
        }
        let quiet = |label: &'static str, message: Message| {
            let mut hover = color(tokens.text);
            hover.a = 0.07;
            div()
                .id((label, pane_key(pane_id)))
                .h(px(24.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(ui_size)
                .text_color(color(tokens.text))
                .hover(move |style| style.bg(hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(message.clone(), window, cx);
                    }),
                )
                .child(label)
        };
        if (process_exited || launch_failed) && !compact {
            controls = controls.child(quiet("Restart", Message::RestartPane(pane_id)));
        }
        if launch_pending && !compact {
            controls = controls.child(quiet("Cancel", Message::CancelTerminalLaunch(pane_id)));
        }
        if launch_suppressed && !compact {
            controls = controls.child(quiet("Start terminal", Message::StartTerminal(pane_id)));
        }
        if !compact {
            controls = controls.child(div().w(px(1.)).h(px(14.)).bg(color(tokens.line_strong)));
        }
        controls = controls
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

        // What is actually running, when there is room to say so: the
        // program's name in the terminal face, on a faint chip.
        let program_chip = app
            .pane_program(pane_id)
            .filter(|_| !compact)
            .map(|program| {
                let mut chip = color(tokens.text);
                chip.a = 0.05;
                div()
                    .py(px(1.))
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(chip)
                    .font_family(terminal_family(&app.settings))
                    .text_size(px(app.settings.ui_pixels(7.5)))
                    .text_color(color(tokens.muted))
                    .child(program)
            });
        let state_label = (!compact && state != "Shell").then(|| {
            div()
                .text_size(ui_size)
                .text_color(color(signal_kind.label_color(tokens)))
                .whitespace_nowrap()
                .child(state)
        });

        let mut unfocused = color(tokens.text);
        unfocused.a = 0.03;
        div()
            .flex()
            .flex_col()
            .h(px(35.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .h(px(34.))
                    .pl(px(12.))
                    .pr(px(6.))
                    .rounded_t(px(9.))
                    .bg(if focused {
                        color(tokens.panel_raised)
                    } else {
                        unfocused
                    })
                    .child(div().size(px(6.)).rounded_full().bg(color(signal)))
                    .child(
                        div()
                            .min_w(px(0.))
                            .text_size(ui_size)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(color(if focused { tokens.text } else { tokens.muted }))
                            .truncate()
                            .child(title),
                    )
                    .children(program_chip)
                    .child(div().flex_grow(1.0))
                    .children(state_label)
                    .child(controls),
            )
            .child(div().h(px(1.)).w_full().bg(color(tokens.line)))
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

/// The pane menu: the actions that do not fit in the header's five buttons.
///
/// Anchored under the overflow button and dismissed by any press outside it,
/// which is the behaviour the iced `Popover` had and the e2e suite asserts.
impl Root {
    pub(crate) fn pane_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let app = self.app();
        let pane_id = app.pane_menu?;
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        let runtime = app.terminals.get(&pane_id);
        let tab = app
            .active_workspace()
            .ok()
            .and_then(Workspace::active_tab)?;

        // Asking the emulator for the selection would mean a round trip to the
        // session thread every frame the menu is open; the flags track the
        // same answers from this side of the channel.
        let can_copy = runtime.is_some_and(|runtime| runtime.has_selection);
        let can_paste = runtime.is_some_and(|runtime| runtime.session.is_some());
        let maximized = app.maximized_pane == Some(pane_id);

        let mut entries: Vec<MenuEntry> = vec![
            MenuEntry::action(
                "Copy",
                commands::COPY_SHORTCUT,
                can_copy.then_some(Message::CopyTerminalSelection(pane_id)),
                false,
            ),
            MenuEntry::action(
                "Paste",
                commands::PASTE_SHORTCUT,
                can_paste.then_some(Message::PastePane(pane_id)),
                false,
            ),
            MenuEntry::Divider,
        ];
        if app.maximized_pane.is_none() {
            entries.push(MenuEntry::action(
                "Split right",
                "Ctrl+Shift+E",
                Some(Message::SplitFrom(pane_id, SplitAxis::Horizontal)),
                false,
            ));
            entries.push(MenuEntry::action(
                "Split down",
                "Ctrl+Shift+O",
                Some(Message::SplitFrom(pane_id, SplitAxis::Vertical)),
                false,
            ));
        }
        entries.push(MenuEntry::action(
            if maximized {
                "Restore panes"
            } else {
                "Maximize pane"
            },
            "Ctrl+Shift+M",
            Some(Message::ToggleMaximizeFromPaneMenu(pane_id)),
            false,
        ));
        entries.push(MenuEntry::Divider);
        entries.push(MenuEntry::action(
            "Restart in worktree…",
            "",
            Some(Message::OpenPaneWorktreePrompt(pane_id)),
            false,
        ));
        entries.push(MenuEntry::action(
            "Restart terminal",
            "",
            Some(Message::RestartPane(pane_id)),
            false,
        ));
        entries.push(MenuEntry::action(
            if tab.panes.len() == 1 {
                "Close pane and tab"
            } else {
                "Close pane"
            },
            "",
            Some(Message::ClosePane(pane_id)),
            true,
        ));

        let mut menu = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .w(px(232.))
            .p(px(4.))
            .rounded(px(8.))
            .bg(color(tokens.overlay))
            .border_1()
            .border_color(color(tokens.line))
            .shadow_lg();
        for (index, entry) in entries.into_iter().enumerate() {
            menu = menu.child(entry.render(index, pane_id, tokens, self.app(), cx));
        }
        Some(menu.into_any_element())
    }
}

/// One row of the pane menu.
enum MenuEntry {
    Action {
        label: &'static str,
        shortcut: &'static str,
        /// `None` renders the row present but dimmed, so the menu's height and
        /// every row's position stay fixed regardless of what is available.
        message: Option<Message>,
        danger: bool,
    },
    Divider,
}

impl MenuEntry {
    const fn action(
        label: &'static str,
        shortcut: &'static str,
        message: Option<Message>,
        danger: bool,
    ) -> Self {
        Self::Action {
            label,
            shortcut,
            message,
            danger,
        }
    }

    fn render(
        self,
        index: usize,
        pane_id: PaneId,
        tokens: DesignTokens,
        app: &crate::app::Muxtrix,
        cx: &mut Context<Root>,
    ) -> AnyElement {
        match self {
            Self::Divider => div()
                .h(px(1.))
                .mx(px(6.))
                .my(px(3.))
                .bg(color(tokens.line))
                .into_any_element(),
            Self::Action {
                label,
                shortcut,
                message,
                danger,
            } => {
                let enabled = message.is_some();
                let foreground = if !enabled {
                    tokens.faint
                } else if danger {
                    tokens.danger
                } else {
                    tokens.text
                };
                let mut hover = color(tokens.line_strong);
                hover.a = 0.14;
                let mut row = div()
                    .id(("pane-menu", pane_key(pane_id).wrapping_add(index as u64)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .text_size(px(app.settings.ui_pixels(11.0)))
                    .text_color(color(foreground))
                    .child(label)
                    .child(
                        div()
                            .text_color(color(tokens.faint))
                            .child(shortcut.to_owned()),
                    );
                if let Some(message) = message {
                    row = row
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                root.dispatch(message.clone(), window, cx);
                            }),
                        );
                }
                row.into_any_element()
            }
        }
    }
}
