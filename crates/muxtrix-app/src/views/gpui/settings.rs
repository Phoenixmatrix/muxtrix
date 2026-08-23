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

use crate::app::{IconKind, Message, SettingsButtonKind, settings_have_changes};
use crate::runtime::gpui::{Root, color};
use crate::settings::{Appearance, FleetView, FontWeight, TerminalFont, UiFont};
use crate::theme::DesignTokens;
use crate::views::gpui::icon_button;

impl Root {
    pub(crate) fn view_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        // The draft's appearance, not the saved one: flipping Light/Dark on
        // the page previews it on the page, as the iced screen does.
        let tokens = DesignTokens::for_appearance(app.settings_draft.appearance);
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

    /// Full-screen theme browser: every preset rendered as the same live
    /// terminal preview the settings page shows for the current theme —
    /// sample output, selection, cursor, and the full ANSI strip — two per
    /// row, clickable, Esc or Back to return.
    fn theme_gallery(&self, tokens: DesignTokens, cx: &mut Context<Self>) -> AnyElement {
        let app = self.app();
        let draft = &app.settings_draft;
        let chosen = draft.terminal_theme;
        let count_nudge = (draft.ui_pixels(18.0) - draft.ui_pixels(9.5)) * 0.6;
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.))
            .child(self.settings_action_button(
                "gallery-back",
                "← Settings",
                Message::CloseThemeGallery,
                SettingsButtonKind::Secondary,
                tokens,
                cx,
            ))
            .child(
                div()
                    .text_size(px(draft.ui_pixels(18.0)))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(color(tokens.text))
                    .child("Theme gallery"),
            )
            .child(div().flex_grow(1.0))
            .child(
                div()
                    .pt(px(count_nudge))
                    .text_size(px(draft.ui_pixels(9.5)))
                    .text_color(color(tokens.faint))
                    .child(format!(
                        "Current: {} · {} themes",
                        chosen.preset().name,
                        crate::themes::TerminalThemeId::ALL.len()
                    )),
            );

        // One column keeps the preview truthful when cards would squeeze
        // below the width the sample line and ANSI strip need.
        let columns: usize = if app.window_size.width < 980.0 { 1 } else { 2 };
        let mut grid = div().flex().flex_col().gap(px(12.));
        let mut cards: Vec<AnyElement> = Vec::new();
        let flush = |grid: gpui::Div, cards: &mut Vec<AnyElement>| -> gpui::Div {
            if cards.is_empty() {
                return grid;
            }
            while cards.len() < columns {
                cards.push(div().flex_1().into_any_element());
            }
            grid.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(12.))
                    .children(std::mem::take(cards)),
            )
        };
        let mut current_group: Option<bool> = None;
        // Dark first, then light: grouped so the mode chip on every card
        // becomes redundant and a named theme is findable by section.
        let ordered = crate::themes::TerminalThemeId::ALL
            .into_iter()
            .filter(|id| !id.preset().is_light)
            .chain(
                crate::themes::TerminalThemeId::ALL
                    .into_iter()
                    .filter(|id| id.preset().is_light),
            );
        for id in ordered {
            let preset = id.preset();
            if current_group != Some(preset.is_light) {
                grid = flush(grid, &mut cards);
                current_group = Some(preset.is_light);
                grid = grid.child(
                    div()
                        .pt(px(8.))
                        .pl(px(3.))
                        .text_size(px(draft.ui_pixels(8.5)))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(color(tokens.faint))
                        .child(if preset.is_light { "LIGHT" } else { "DARK" }),
                );
            }
            let selected = id == chosen;
            cards.push(
                div()
                    .id(SharedString::from(preset.name))
                    .flex_1()
                    .min_w(px(0.))
                    .p(px(3.))
                    .rounded(px(12.))
                    .border_2()
                    .border_color(color(if selected {
                        tokens.accent
                    } else {
                        iced::Color::TRANSPARENT
                    }))
                    .when(!selected, |card| {
                        card.hover(move |style| style.border_color(color(tokens.line_strong)))
                    })
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                            root.dispatch(Message::GalleryThemeChosen(id), window, cx);
                        }),
                    )
                    .child(theme_preview_card(preset, draft, false))
                    .into_any_element(),
            );
            if cards.len() == columns {
                grid = flush(grid, &mut cards);
            }
        }
        grid = flush(grid, &mut cards);

        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .size_full()
            .p(px(28.))
            .bg(color(tokens.app))
            .child(header)
            .child(
                div()
                    .id("gallery-scroll")
                    .flex_grow(1.0)
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scrolls.settings)
                    .child(grid.pt(px(4.)).pr(px(14.)).pb(px(24.))),
            )
            .into_any_element()
    }

    /// A settings-page action: the same three kinds the iced page draws.
    fn settings_action_button(
        &self,
        id: &'static str,
        label: &'static str,
        message: Message,
        kind: SettingsButtonKind,
        tokens: DesignTokens,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = self.app();
        let (fill, hover, text, edge, hover_edge) = match kind {
            SettingsButtonKind::Primary => {
                let mut hover = color(tokens.accent);
                hover.a = 0.86;
                (
                    color(tokens.accent),
                    hover,
                    tokens.app,
                    tokens.accent,
                    tokens.accent,
                )
            }
            SettingsButtonKind::Secondary => (
                color(tokens.panel),
                color(tokens.panel_raised),
                tokens.text,
                tokens.line_strong,
                tokens.line_strong,
            ),
            SettingsButtonKind::Danger => {
                let mut fill = color(tokens.danger);
                fill.a = 0.05;
                let mut hover = color(tokens.danger);
                hover.a = 0.12;
                // tokens.line rendered these borders invisible.
                let mut edge = tokens.danger;
                edge.a = 0.45;
                (fill, hover, tokens.danger, edge, tokens.danger)
            }
            SettingsButtonKind::Quiet => (
                color(iced::Color::TRANSPARENT),
                color(tokens.panel_raised),
                tokens.muted,
                iced::Color::TRANSPARENT,
                tokens.line_strong,
            ),
        };
        div()
            .id(id)
            .h(px(30.))
            .px(px(11.))
            .flex()
            .items_center()
            .rounded(px(5.))
            .cursor_pointer()
            .bg(fill)
            .border_1()
            .border_color(color(edge))
            .text_size(px(app.settings_draft.ui_pixels(9.0)))
            .text_color(color(text))
            .whitespace_nowrap()
            .hover(move |style| style.bg(hover).border_color(color(hover_edge)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _: &MouseDownEvent, window, cx| {
                    root.dispatch(message.clone(), window, cx);
                }),
            )
            .child(label)
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

/// The live terminal preview for a theme: sample output, a selection, the
/// cursor, and the full ANSI strip, on the theme's own background. Built from
/// the same colours the real grid uses, so what the card shows is what the
/// terminal will render.
pub(crate) fn theme_preview_card(
    preset: crate::themes::TerminalThemePreset,
    settings: &crate::settings::AppSettings,
    caption: bool,
) -> AnyElement {
    use crate::terminal::runs::rgb;
    let tokens = DesignTokens::for_appearance(settings.appearance);
    let font = crate::terminal::element::terminal_font(settings);
    let preview_size = settings.terminal_font_pixels().clamp(13.0, 18.0);
    let mode = if preset.is_light { "Light" } else { "Dark" };

    // The sample line, as one styled run per coloured span.
    let spans: [(&str, muxtrix_terminal::Rgb, Option<muxtrix_terminal::Rgb>); 10] = [
        ("❯ ", preset.ansi[10], None),
        ("cargo test ", preset.foreground, None),
        ("--workspace\n", preset.ansi[12], None),
        ("   Compiling ", preset.ansi[3], None),
        ("muxtrix\n", preset.foreground, None),
        ("   Finished ", preset.ansi[2], None),
        ("95 tests passed  ", preset.foreground, None),
        (
            " selected ",
            preset.selection_foreground,
            Some(preset.selection_background),
        ),
        ("  ", preset.foreground, None),
        (" C ", preset.cursor_text, Some(preset.cursor)),
    ];
    let mut text = String::new();
    let mut runs = Vec::with_capacity(spans.len());
    for (piece, foreground, background) in spans {
        text.push_str(piece);
        runs.push(gpui::TextRun {
            len: piece.len(),
            font: font.clone(),
            color: color(rgb(foreground)).into(),
            background_color: background.map(|hue| color(rgb(hue)).into()),
            underline: None,
            strikethrough: None,
        });
    }
    let sample = div()
        .text_size(px(preview_size))
        .line_height(px(preview_size * 1.35))
        .child(gpui::StyledText::new(text).with_runs(runs));

    let strip = |colors: &[muxtrix_terminal::Rgb]| {
        let mut row = div().flex().flex_row().gap(px(5.));
        for hue in colors {
            // The iced strip asks for a 24 px cap on a fill-width swatch and
            // gets the fill: the cap never applies. The rendered width is
            // what the gallery is compared against, so match that.
            row = row.child(
                div()
                    .flex_1()
                    .h(px(12.))
                    .rounded(px(2.))
                    .bg(color(rgb(*hue))),
            );
        }
        row
    };

    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .w_full()
        .py(px(14.))
        .px(px(16.))
        .rounded(px(6.))
        .bg(color(rgb(preset.background)))
        .border_1()
        .border_color(color(tokens.line_strong))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_grow(1.0)
                        .min_w(px(0.))
                        .text_size(px(settings.ui_pixels(11.0)))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(color(rgb(preset.foreground)))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(preset.name),
                )
                .when(caption, |head| {
                    // In the gallery, section headings already say the mode;
                    // repeating a pill on every card is noise.
                    head.child(
                        div()
                            .py(px(2.))
                            .px(px(7.))
                            .rounded(px(4.))
                            .bg(color(rgb(preset.foreground)))
                            .text_size(px(settings.ui_pixels(8.0)))
                            .text_color(color(rgb(preset.background)))
                            .child(mode),
                    )
                }),
        )
        .child(sample)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(strip(&preset.ansi[..8]))
                .child(strip(&preset.ansi[8..])),
        );
    if caption {
        card = card.child(
            div()
                .text_size(px(settings.ui_pixels(8.0)))
                .text_color(color(rgb(preset.ansi[8])))
                .child(
                    "Theme colors set defaults · direct RGB and application OSC colors stay intact",
                ),
        );
    }
    card.into_any_element()
}
