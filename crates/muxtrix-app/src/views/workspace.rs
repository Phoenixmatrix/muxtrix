//! The workspace surface: the tab strip and the pane tree beneath it.
//!
//! Everything to the right of the sidebar. Phase 3 adds the sidebar and status
//! bar around this; for now it is the whole window.

use gpui::{
    Anchor, AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, StatefulInteractiveElement, Styled, Window, div, point, px, svg,
};
use gpui_component::Sizable as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::tab::{Tab, TabBar};

use crate::app::{IconKind, Message, PaneSignalKind};
use crate::runtime::gpui::{Root, color};
use crate::theme::{Color, DesignTokens};
use crate::views::{TOP_CHROME_HEIGHT, icon_button, icon_path, tab_key};

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
            .child(self.tab_strip(window, cx))
            .child(div().flex_grow(1.0).overflow_hidden().p(px(8.)).child(tree))
            .into_any_element()
    }

    /// The active workspace's component-backed tab bar.
    fn tab_strip(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        let Ok(workspace) = app.active_workspace() else {
            return div().h(px(TOP_CHROME_HEIGHT)).into_any_element();
        };

        let workspace_id = workspace.id;
        let active_index = workspace
            .tabs
            .iter()
            .position(|tab| tab.id == workspace.active_tab_id)
            .unwrap_or_default();
        let scroll_target = (workspace_id, workspace.active_tab_id);
        if self.tab_scroll_target.get() != Some(scroll_target) {
            self.tab_scroll_target.set(Some(scroll_target));
            self.scrolls.tabs.scroll_to_item(active_index);
        }
        let label_characters = workspace
            .tabs
            .iter()
            .map(|tab| tab.name.chars().count())
            .sum();
        let overflow_layout = (
            workspace.tabs.len(),
            label_characters,
            window.bounds().size.width,
            app.sidebar_collapsed,
            app.github_panel.is_some(),
        );
        let needs_overflow_measurement = self.tab_overflow_layout.get() != Some(overflow_layout);
        if needs_overflow_measurement {
            // Measure first without the menu so the menu never creates the
            // overflow used to justify its own presence.
            self.tab_overflow_visible.set(false);
            self.tab_overflow_reveal.set(None);
            if self.tab_overflow_probe.get() != Some(overflow_layout) {
                self.tab_overflow_probe.set(Some(overflow_layout));
                let root = cx.entity();
                let scroll = self.scrolls.tabs.clone();
                window.defer(cx, move |_, cx| {
                    let overflow = scroll.max_offset().x > px(14.);
                    let offset = scroll.offset();
                    let reset_scroll = !overflow && offset.x != px(0.);
                    if reset_scroll {
                        scroll.set_offset(point(px(0.), offset.y));
                    }
                    root.update(cx, |root, cx| {
                        root.tab_overflow_layout.set(Some(overflow_layout));
                        root.tab_overflow_probe.set(None);
                        if root.tab_overflow_visible.replace(overflow) != overflow || reset_scroll {
                            cx.notify();
                        }
                    });
                });
            }
        }
        let tabs_overflow = self.tab_overflow_visible.get();
        if tabs_overflow {
            let reveal_key = (
                workspace_id,
                workspace.active_tab_id,
                workspace.tabs.len(),
                window.bounds().size.width,
                app.sidebar_collapsed,
                app.github_panel.is_some(),
            );
            if self.tab_overflow_reveal.get() != Some(reveal_key) {
                self.tab_overflow_reveal.set(Some(reveal_key));
                let root = cx.entity();
                let scroll = self.scrolls.tabs.clone();
                window.defer(cx, move |_, cx| {
                    let Some(bounds) = scroll.bounds_for_item(active_index) else {
                        return;
                    };
                    let viewport = scroll.bounds();
                    let offset = scroll.offset();
                    let mut next_x = offset.x;
                    let left = bounds.left() + offset.x;
                    let right = bounds.right() + offset.x;
                    if left < viewport.left() {
                        next_x += viewport.left() - left;
                    } else if right > viewport.right() {
                        next_x -= right - viewport.right();
                    }
                    let limit = -scroll.max_offset().x;
                    next_x = if next_x < limit {
                        limit
                    } else if next_x > px(0.) {
                        px(0.)
                    } else {
                        next_x
                    };
                    if next_x != offset.x {
                        scroll.set_offset(point(next_x, offset.y));
                        root.update(cx, |_, cx| cx.notify());
                    }
                });
            }
        }

        let close_hover = color(tokens.element_hover);
        let mut tabs = Vec::with_capacity(workspace.tabs.len());
        for (index, workspace_tab) in workspace.tabs.iter().enumerate() {
            let tab_id = workspace_tab.id;
            let drop_target = app.tab_drag.is_some_and(|drag| {
                drag.target_workspace_id == workspace_id && drag.target_index == index
            });
            let signal_kind = app.tab_signal_kind(workspace_tab);
            let signal_label = match signal_kind {
                PaneSignalKind::Subtle => "idle",
                PaneSignalKind::Neutral => "ready",
                PaneSignalKind::Warning => "needs input",
                PaneSignalKind::Active => "working",
                PaneSignalKind::Danger => "failed",
            };
            let signal = signal_kind.color(tokens);
            let tab_hover_group = format!("workspace-tab-{}", tab_key(tab_id));
            let close = div()
                .id(("close-tab", tab_key(tab_id)))
                .role(gpui::accesskit::Role::Button)
                .aria_label(format!("Close {} tab", workspace_tab.name))
                .invisible()
                .group_hover(tab_hover_group.clone(), |close| close.visible())
                .size(px(18.))
                .mr(px(2.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(move |style| style.bg(close_hover))
                // The tab begins a drag on mouse-down and selects on click.
                // Stop both phases so closing cannot trigger either.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()),
                )
                .on_click(cx.listener(move |root, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    root.dispatch(Message::CloseTab(workspace_id, tab_id), window, cx);
                }))
                .child(
                    svg()
                        .path(icon_path(IconKind::Close))
                        .size(px(14.))
                        .text_color(color(tokens.muted)),
                );
            let mut tab = Tab::new()
                .label(workspace_tab.name.clone())
                .aria_label(format!("{}, {signal_label}", workspace_tab.name))
                .group(tab_hover_group)
                .prefix(
                    div()
                        .size(px(6.))
                        // Reallocate the component's leading whitespace so the
                        // state dot groups with its own label, not the previous
                        // tab's close control.
                        .ml(px(8.))
                        .mr(px(-8.))
                        .flex_none()
                        .rounded_full()
                        .bg(color(signal)),
                )
                .suffix(close)
                .cursor_grab()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(
                            Message::BeginTabDrag(workspace_id, tab_id, index),
                            window,
                            cx,
                        );
                    }),
                )
                .on_mouse_move(
                    cx.listener(move |root, _: &gpui::MouseMoveEvent, window, cx| {
                        if root.app.tab_drag.is_some() {
                            root.dispatch(Message::TabDragOver(workspace_id, index), window, cx);
                        }
                    }),
                )
                .on_aux_click(cx.listener(move |root, event: &ClickEvent, window, cx| {
                    if event.is_middle_click() {
                        cx.stop_propagation();
                        root.dispatch(Message::CloseTab(workspace_id, tab_id), window, cx);
                    }
                }));
            if drop_target {
                tab = tab.border_l(px(2.)).border_color(color(tokens.accent));
            }
            tabs.push(tab);
        }

        let tab_count = workspace.tabs.len();
        let can_navigate_tabs = tab_count > 1;
        let previous_tab_id = can_navigate_tabs
            .then(|| workspace.tabs[(active_index + tab_count - 1) % tab_count].id);
        let next_tab_id =
            can_navigate_tabs.then(|| workspace.tabs[(active_index + 1) % tab_count].id);
        let navigation_hover = color(tokens.element_hover);
        let tab_navigation_button =
            |id: &'static str,
             kind: IconKind,
             label: &'static str,
             target: Option<muxtrix_domain::TabId>| {
                let navigation_root = cx.entity();
                let mut button = div()
                    .id(id)
                    .role(gpui::accesskit::Role::Button)
                    .aria_label(label)
                    .size(px(24.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .child(svg().path(icon_path(kind)).size(px(12.)).text_color(color(
                        if target.is_some() {
                            tokens.muted
                        } else {
                            tokens.faint
                        },
                    )));
                if let Some(tab_id) = target {
                    button = button
                        .cursor_pointer()
                        .hover(move |style| style.bg(navigation_hover))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                root.dispatch(Message::ActivateTab(tab_id), window, cx);
                            }),
                        )
                        .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                            navigation_root.update(cx, |root, cx| {
                                root.dispatch(Message::ActivateTab(tab_id), window, cx);
                            });
                        });
                } else {
                    button = button.cursor_default();
                }
                button
            };
        let previous_tab = tab_navigation_button(
            "previous-tab",
            IconKind::Back,
            "Previous tab",
            previous_tab_id,
        );
        let next_tab =
            tab_navigation_button("next-tab", IconKind::Forward, "Next tab", next_tab_id);
        let tab_bar_prefix = div()
            .h_full()
            .flex()
            .flex_none()
            .items_center()
            .px(px(4.))
            // Collapse the prefix and first-tab border slots onto one pixel.
            // Exactly one is visible in either selection state, so both the
            // geometry and the painted seam stay fixed.
            .border_r(px(1.))
            .mr(px(-1.))
            .border_color(color(if active_index == 0 {
                Color::TRANSPARENT
            } else {
                tokens.line
            }))
            .child(previous_tab)
            .child(next_tab);
        let new_tab = svg()
            .path(icon_path(IconKind::Add))
            .size(px(12.))
            .text_color(color(tokens.muted));
        let overflow_tabs = workspace
            .tabs
            .iter()
            .map(|tab| (tab.id, tab.name.clone()))
            .collect::<Vec<_>>();
        let selected_tab_id = workspace.active_tab_id;
        let overflow_root = cx.entity();
        let overflow_menu = tabs_overflow.then(|| {
            Button::new("tab-overflow")
                .xsmall()
                .ghost()
                .label("Tabs")
                .dropdown_caret(true)
                .dropdown_menu(move |mut menu, _, _| {
                    menu = menu.scrollable(true);
                    for (tab_id, label) in &overflow_tabs {
                        let tab_id = *tab_id;
                        let root = overflow_root.clone();
                        menu = menu.item(
                            PopupMenuItem::new(label.clone())
                                .checked(tab_id == selected_tab_id)
                                .on_click(move |_, window, cx| {
                                    root.update(cx, |root, cx| {
                                        root.dispatch(Message::ActivateTab(tab_id), window, cx);
                                    });
                                }),
                        );
                    }
                    menu
                })
                .anchor(Anchor::TopRight)
        });
        let tab_bar_suffix = div().flex().items_center().children(overflow_menu);
        let last_empty_space = div()
            .min_w(px(12.))
            .h_full()
            .flex_grow(1.0)
            .on_mouse_move(
                cx.listener(move |root, _: &gpui::MouseMoveEvent, window, cx| {
                    if root.app.tab_drag.is_some() {
                        root.dispatch(Message::TabDragOver(workspace_id, tab_count), window, cx);
                    }
                }),
            )
            .child("");
        let tab_ids = workspace.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>();
        let root = cx.entity();
        let tab_bar = TabBar::new("workspace-tabs")
            .selected_index(active_index)
            .max_width(px(app.settings.ui_pixels(120.0)))
            .track_scroll(&self.scrolls.tabs)
            .prefix(tab_bar_prefix)
            .children(tabs)
            .last_empty_space(last_empty_space)
            // The group callback keeps direct tab activation in one path.
            // Close stops click propagation before it reaches this handler.
            .on_click(move |index, window, cx| {
                if let Some(tab_id) = tab_ids.get(*index).copied() {
                    root.update(cx, |root, cx| {
                        root.dispatch(Message::ActivateTab(tab_id), window, cx);
                    });
                }
            })
            .suffix(tab_bar_suffix)
            .size_full()
            .min_w(px(0.))
            .bg(color(tokens.rail));
        let strip = div()
            .h_full()
            .flex()
            .flex_grow(1.0)
            .min_w(px(0.))
            .child(tab_bar);
        // One quiet toolbar follows the scrolling tabs. Its single leading
        // rule and shared bottom edge make it part of the tab strip rather
        // than a row of boxed controls.
        let mut keycap_fill = color(tokens.text);
        keycap_fill.a = 0.05;
        let action_hover = color(tokens.element_hover);
        let commands_root = cx.entity();
        let commands = div()
            .id("commands-action")
            .role(gpui::accesskit::Role::Button)
            .aria_label("Commands")
            .aria_keyshortcuts(if cfg!(target_os = "macos") {
                "Meta+P"
            } else {
                "Control+P"
            })
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .h(px(24.))
            .px(px(6.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(move |style| style.bg(action_hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::ToggleCommandPalette, window, cx);
                }),
            )
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                commands_root.update(cx, |root, cx| {
                    root.dispatch(Message::ToggleCommandPalette, window, cx);
                });
            })
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

        let settings_root = cx.entity();
        let settings = icon_button(
            gpui::ElementId::from("open-settings"),
            IconKind::Settings,
            tokens,
            false,
        )
        .role(gpui::accesskit::Role::Button)
        .aria_label("Settings")
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|root, _: &MouseDownEvent, window, cx| {
                root.dispatch(Message::OpenSettings, window, cx);
            }),
        )
        .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
            settings_root.update(cx, |root, cx| {
                root.dispatch(Message::OpenSettings, window, cx);
            });
        });
        let new_tab_root = cx.entity();
        let new_tab_action = div()
            .id("new-tab-action")
            .role(gpui::accesskit::Role::Button)
            .aria_label("New tab")
            .aria_keyshortcuts(if cfg!(target_os = "macos") {
                "Meta+T"
            } else {
                "Control+T"
            })
            .size(px(24.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .hover(move |style| style.bg(action_hover))
            .cursor_pointer()
            // This fixed action cell remains an always-reachable end drop
            // target when the scroller's last empty space is offscreen.
            .on_mouse_move(
                cx.listener(move |root, _: &gpui::MouseMoveEvent, window, cx| {
                    if root.app.tab_drag.is_some() {
                        root.dispatch(Message::TabDragOver(workspace_id, tab_count), window, cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(Message::NewTab, window, cx);
                }),
            )
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                new_tab_root.update(cx, |root, cx| {
                    root.dispatch(Message::NewTab, window, cx);
                });
            })
            .child(new_tab);
        let app_actions = div()
            .h_full()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .border_l(px(1.))
            .border_b(px(1.))
            .border_color(color(tokens.line))
            .child(new_tab_action)
            .child(commands)
            .child(settings);

        div()
            .flex()
            .flex_row()
            .h(px(TOP_CHROME_HEIGHT))
            .bg(color(tokens.rail))
            .child(strip)
            .child(app_actions)
            .into_any_element()
    }
}
