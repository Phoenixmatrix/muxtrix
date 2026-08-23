//! The settings screen.
//!
//! Every control edits the settings *draft*, never the saved settings, so the
//! page can be abandoned without effect until Save writes it through. That is
//! also what makes the Save button's enabled state meaningful: it compares the
//! two.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, px,
};

use crate::app::{IconKind, Message, settings_have_changes};
use crate::runtime::gpui::{Root, color};
use crate::settings::{Appearance, FleetView, FontWeight, TerminalFont, UiFont};
use crate::theme::DesignTokens;
use crate::views::gpui::icon_button;

impl Root {
    pub(crate) fn view_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let tokens = DesignTokens::for_appearance(app.settings.appearance);
        if app.active_view == crate::app::ActiveView::ThemeGallery {
            return self.theme_gallery(tokens, cx);
        }
        if app.settings_page == crate::app::SettingsPage::Worktrees {
            return self.worktree_manager(tokens, cx);
        }
        let draft = &app.settings_draft;
        let changed = settings_have_changes(&app.settings, draft);

        let body = div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .p(px(28.))
            .max_w(px(760.))
            .child(self.section(
                "Appearance",
                vec![
                    self.choice_row(
                        "Theme",
                        [
                            ("System", Appearance::System),
                            ("Light", Appearance::Light),
                            ("Dark", Appearance::Dark),
                        ],
                        draft.appearance,
                        Message::SettingsAppearance,
                        tokens,
                        cx,
                    ),
                    self.toggle_row(
                        "Status bar",
                        draft.show_status_bar,
                        Message::SettingsShowStatusBar(!draft.show_status_bar),
                        tokens,
                        cx,
                    ),
                    self.choice_row(
                        "Fleet view",
                        [
                            ("Tabs", FleetView::Tabs),
                            ("Agents", FleetView::Agents),
                            ("Repos", FleetView::Repos),
                        ],
                        draft.fleet_view,
                        Message::SetFleetView,
                        tokens,
                        cx,
                    ),
                ],
                tokens,
            ))
            .child(self.section(
                "Interface",
                vec![
                    self.choice_row(
                        "Font",
                        [("System sans", UiFont::SystemSans)],
                        draft.ui_font.clone(),
                        Message::SettingsUiFont,
                        tokens,
                        cx,
                    ),
                    self.choice_row(
                        "Weight",
                        [
                            ("Light", FontWeight::Light),
                            ("Normal", FontWeight::Normal),
                            ("Medium", FontWeight::Medium),
                            ("Semibold", FontWeight::Semibold),
                            ("Bold", FontWeight::Bold),
                        ],
                        draft.ui_font_weight,
                        Message::SettingsUiFontWeight,
                        tokens,
                        cx,
                    ),
                    self.step_row(
                        "Size",
                        &format!("{:.0} px", draft.ui_font_size),
                        Message::SettingsUiFontSize(draft.ui_font_size - 1.0),
                        Message::SettingsUiFontSize(draft.ui_font_size + 1.0),
                        tokens,
                        cx,
                    ),
                ],
                tokens,
            ))
            .child(self.section(
                "Terminal",
                vec![
                    self.choice_row(
                        "Font",
                        [("System monospace", TerminalFont::SystemMonospace)],
                        draft.terminal_font.clone(),
                        Message::SettingsTerminalFont,
                        tokens,
                        cx,
                    ),
                    self.choice_row(
                        "Weight",
                        [
                            ("Light", FontWeight::Light),
                            ("Normal", FontWeight::Normal),
                            ("Medium", FontWeight::Medium),
                            ("Semibold", FontWeight::Semibold),
                            ("Bold", FontWeight::Bold),
                        ],
                        draft.terminal_font_weight,
                        Message::SettingsTerminalFontWeight,
                        tokens,
                        cx,
                    ),
                    self.step_row(
                        "Size",
                        &format!("{:.0} px", draft.terminal_font_size),
                        Message::SettingsTerminalFontSize(draft.terminal_font_size - 1.0),
                        Message::SettingsTerminalFontSize(draft.terminal_font_size + 1.0),
                        tokens,
                        cx,
                    ),
                    self.step_row(
                        "Line height",
                        &format!("{:.2}", draft.terminal_line_height),
                        Message::SettingsLineHeight(draft.terminal_line_height - 0.05),
                        Message::SettingsLineHeight(draft.terminal_line_height + 0.05),
                        tokens,
                        cx,
                    ),
                    self.link_row(
                        "Palette",
                        draft.terminal_theme.preset().name,
                        Message::OpenThemeGallery,
                        tokens,
                        cx,
                    ),
                ],
                tokens,
            ))
            .child(self.section(
                "Repository",
                vec![self.link_row(
                    "Worktrees",
                    "Manage",
                    Message::OpenSettingsPage(crate::app::SettingsPage::Worktrees),
                    tokens,
                    cx,
                )],
                tokens,
            ));

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(color(tokens.app))
            .child(self.settings_bar(changed, tokens, cx))
            .child(
                div()
                    .id("settings-scroll")
                    .flex_grow(1.0)
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scrolls.settings)
                    .child(body),
            )
            .into_any_element()
    }

    /// Every terminal palette, two per row, each showing its own colours.
    ///
    /// The swatch is the preview: a palette is its colours, and a sample of
    /// them says more than the name does.
    fn theme_gallery(&self, tokens: DesignTokens, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let chosen = app.settings_draft.terminal_theme;
        let mut grid = div().flex().flex_row().flex_wrap().gap(px(10.)).p(px(20.));
        for id in crate::themes::TerminalThemeId::ALL {
            let preset = id.preset();
            let selected = id == chosen;
            let mut swatches = div().flex().flex_row().gap(px(3.));
            for hue in [
                preset.ansi[1],
                preset.ansi[2],
                preset.ansi[3],
                preset.ansi[4],
                preset.ansi[5],
                preset.ansi[6],
            ] {
                swatches = swatches.child(
                    div()
                        .size(px(12.))
                        .rounded(px(2.))
                        .bg(color(crate::terminal::runs::rgb(hue))),
                );
            }
            grid = grid.child(
                div()
                    .id(SharedString::from(preset.name))
                    .w(px(320.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .bg(color(crate::terminal::runs::rgb(preset.background)))
                    .border_1()
                    .border_color(color(if selected { tokens.accent } else { tokens.line }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::GalleryThemeChosen(id), window, cx);
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(app.settings.ui_pixels(11.0)))
                            .text_color(color(crate::terminal::runs::rgb(preset.foreground)))
                            .child(preset.name),
                    )
                    .child(swatches),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(color(tokens.app))
            .child(self.screen_bar("Terminal palettes", Message::CloseThemeGallery, tokens, cx))
            .child(
                div()
                    .id("gallery-scroll")
                    .flex_grow(1.0)
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scrolls.settings)
                    .child(grid),
            )
            .into_any_element()
    }

    /// Every worktree this repository has, with what is holding each one.
    fn worktree_manager(&self, tokens: DesignTokens, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let mut rows = div().flex().flex_col().gap(px(2.)).p(px(20.));
        let manager = app.worktree_manager.as_ref();
        if let Some(failure) = manager.and_then(|manager| manager.failure.as_deref()) {
            rows = rows.child(
                div()
                    .text_size(px(app.settings.ui_pixels(10.0)))
                    .text_color(color(tokens.muted))
                    .child(failure.to_owned()),
            );
        }
        for (index, entry) in manager
            .map(|manager| manager.entries.as_slice())
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            // A worktree in use, or the primary one, cannot be removed; the
            // row says which rather than offering an action that would fail.
            let blocker = entry.deletion_blocker.clone().or_else(|| {
                entry
                    .used_by
                    .clone()
                    .map(|title| format!("in use by {title}"))
            });
            rows = rows.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .h(px(44.))
                    .px(px(12.))
                    .rounded(px(6.))
                    .bg(color(tokens.panel))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_grow(1.0)
                            .min_w(px(0.))
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(10.0)))
                                    .text_color(color(tokens.text))
                                    .truncate()
                                    .child(entry.path.file_name().map_or_else(
                                        || entry.path.display().to_string(),
                                        |name| name.to_string_lossy().into_owned(),
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(app.settings.ui_pixels(8.0)))
                                    .text_color(color(tokens.faint))
                                    .truncate()
                                    .child(
                                        entry.branch.clone().unwrap_or_else(|| "detached".into()),
                                    ),
                            ),
                    )
                    .children((entry.unpushed_commits > 0).then(|| {
                        div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.warning))
                            .child(format!("{} unpushed", entry.unpushed_commits))
                    }))
                    .child(match blocker {
                        Some(reason) => div()
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.faint))
                            .child(reason)
                            .into_any_element(),
                        None => div()
                            .id(("worktree-delete", index as u64))
                            .h(px(24.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .bg(color(tokens.panel_raised))
                            .text_size(px(app.settings.ui_pixels(9.0)))
                            .text_color(color(tokens.danger))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                                    root.dispatch(
                                        Message::WorktreeManagerDelete(index),
                                        window,
                                        cx,
                                    );
                                }),
                            )
                            .child("Delete")
                            .into_any_element(),
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(color(tokens.app))
            .child(self.screen_bar("Worktrees", Message::CloseWorktreeManager, tokens, cx))
            .child(
                div()
                    .id("worktrees-scroll")
                    .flex_grow(1.0)
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scrolls.settings)
                    .child(rows),
            )
            .into_any_element()
    }

    /// A screen's top bar: a title and the way back.
    fn screen_bar(
        &self,
        title: &str,
        back: Message,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .h(px(43.))
            .px(px(10.))
            .bg(color(tokens.rail))
            .border_b(px(1.))
            .border_color(color(tokens.line))
            .child(
                icon_button(
                    gpui::ElementId::from("screen-back"),
                    IconKind::Back,
                    tokens,
                    false,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(back.clone(), window, cx);
                    }),
                ),
            )
            .child(
                div()
                    .text_size(px(self.app().settings.ui_pixels(13.0)))
                    .text_color(color(tokens.text))
                    .child(title.to_owned()),
            )
            .into_any_element()
    }

    /// The top bar: back out, or save what has changed.
    fn settings_bar(
        &self,
        changed: bool,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .h(px(43.))
            .px(px(10.))
            .bg(color(tokens.rail))
            .border_b(px(1.))
            .border_color(color(tokens.line))
            .child(
                icon_button(
                    gpui::ElementId::from("settings-back"),
                    IconKind::Back,
                    tokens,
                    false,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, _: &MouseDownEvent, window, cx| {
                        root.dispatch(Message::CancelSettings, window, cx);
                    }),
                ),
            )
            .child(
                div()
                    .flex_grow(1.0)
                    .text_size(px(app.settings.ui_pixels(13.0)))
                    .text_color(color(tokens.text))
                    .child("Settings"),
            )
            .children(changed.then(|| {
                div()
                    .id("settings-save")
                    .h(px(28.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .bg(color(tokens.accent))
                    .text_size(px(app.settings.ui_pixels(11.0)))
                    .text_color(color(tokens.app))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::SaveSettings, window, cx);
                        }),
                    )
                    .child("Save")
            }))
            .into_any_element()
    }

    fn section(&self, title: &str, rows: Vec<AnyElement>, tokens: DesignTokens) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .pb(px(6.))
                    .text_size(px(self.app().settings.ui_pixels(9.0)))
                    .text_color(color(tokens.faint))
                    .child(title.to_uppercase()),
            )
            .children(rows)
            .into_any_element()
    }

    /// A labelled row with its control on the right.
    fn row(&self, label: &str, control: AnyElement, tokens: DesignTokens) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .h(px(38.))
            .px(px(12.))
            .rounded(px(6.))
            .bg(color(tokens.panel))
            .child(
                div()
                    .text_size(px(self.app().settings.ui_pixels(11.0)))
                    .text_color(color(tokens.text))
                    .child(label.to_owned()),
            )
            .child(control)
            .into_any_element()
    }

    /// A segmented control: every option visible, the current one filled.
    fn choice_row<T, const N: usize>(
        &self,
        label: &str,
        options: [(&'static str, T); N],
        current: T,
        message: fn(T) -> Message,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        T: PartialEq + Clone + 'static,
    {
        let mut group = div().flex().flex_row().gap(px(4.));
        for (name, value) in options {
            let selected = value == current;
            let chosen = value.clone();
            group = group.child(
                div()
                    .id(SharedString::from(format!("{label}-{name}")))
                    .h(px(26.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .bg(color(if selected {
                        tokens.accent
                    } else {
                        tokens.panel_raised
                    }))
                    .text_size(px(self.app().settings.ui_pixels(10.0)))
                    .text_color(color(if selected { tokens.app } else { tokens.muted }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(message(chosen.clone()), window, cx);
                        }),
                    )
                    .child(name),
            );
        }
        self.row(label, group.into_any_element(), tokens)
    }

    fn toggle_row(
        &self,
        label: &str,
        on: bool,
        message: Message,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let control = div()
            .id(SharedString::from(format!("{label}-toggle")))
            .w(px(38.))
            .h(px(22.))
            .p(px(3.))
            .rounded(px(11.))
            .cursor_pointer()
            .bg(color(if on {
                tokens.accent
            } else {
                tokens.panel_raised
            }))
            .flex()
            .when(on, |style| style.justify_end())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(message.clone(), window, cx);
                }),
            )
            .child(div().size(px(16.)).rounded_full().bg(color(tokens.app)))
            .into_any_element();
        self.row(label, control, tokens)
    }

    /// A numeric row with a decrement and increment either side of the value.
    fn step_row(
        &self,
        label: &str,
        value: &str,
        decrement: Message,
        increment: Message,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let control = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .child(self.step_button(label, "−", decrement, tokens, cx))
            .child(
                div()
                    .min_w(px(56.))
                    .text_size(px(self.app().settings.ui_pixels(10.0)))
                    .text_color(color(tokens.muted))
                    .child(value.to_owned()),
            )
            .child(self.step_button(label, "+", increment, tokens, cx))
            .into_any_element();
        self.row(label, control, tokens)
    }

    fn step_button(
        &self,
        label: &str,
        glyph: &'static str,
        message: Message,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(SharedString::from(format!("{label}-{glyph}")))
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .cursor_pointer()
            .bg(color(tokens.panel_raised))
            .text_color(color(tokens.muted))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(message.clone(), window, cx);
                }),
            )
            .child(glyph)
            .into_any_element()
    }

    /// A row whose value opens another screen.
    fn link_row(
        &self,
        label: &str,
        value: &str,
        message: Message,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let control = div()
            .id(SharedString::from(format!("{label}-link")))
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(5.))
            .cursor_pointer()
            .bg(color(tokens.panel_raised))
            .text_size(px(self.app().settings.ui_pixels(10.0)))
            .text_color(color(tokens.text))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(message.clone(), window, cx);
                }),
            )
            .child(value.to_owned())
            .into_any_element();
        self.row(label, control, tokens)
    }
}
