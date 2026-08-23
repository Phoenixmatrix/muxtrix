//! The GPUI view layer.
//!
//! Sibling of the iced views, built surface by surface as the port advances.
//! The state they read is the same `Muxtrix` value; only the element tree
//! differs. Phase 7 deletes the iced views and flattens these up a level.

pub(crate) mod dialogs;
pub(crate) mod github;
pub(crate) mod inputs;
pub(crate) mod overlays;
pub(crate) mod panes;
pub(crate) mod settings;
pub(crate) mod settings_widgets;
pub(crate) mod sidebar;
pub(crate) mod workspace;

use gpui::{Div, Hsla, InteractiveElement, ParentElement, Stateful, Styled, div, px, svg};

use crate::app::IconKind;
use crate::assets::icon_path;
use crate::runtime::gpui::color;
use crate::theme::DesignTokens;

/// A 24 px square icon button, the pane header's unit of chrome.
///
/// `danger` tints the hover state toward the danger token rather than the
/// neutral one, which is how close reads as different from the rest without
/// being red at rest.
pub(crate) fn icon_button(
    id: impl Into<gpui::ElementId>,
    kind: IconKind,
    tokens: DesignTokens,
    danger: bool,
) -> Stateful<Div> {
    let hover: Hsla = if danger {
        let mut hue = color(tokens.danger);
        hue.a = 0.14;
        hue.into()
    } else {
        let mut hue = color(tokens.line_strong);
        hue.a = 0.14;
        hue.into()
    };
    div()
        .id(id.into())
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(
            svg()
                .path(icon_path(kind))
                .size(px(12.))
                .text_color(color(tokens.muted)),
        )
}

/// A stable, hashable key for a pane, for GPUI element ids.
///
/// Element ids must be unique within a frame and stable across frames; the
/// pane's own identity is both.
pub(crate) fn pane_key(pane_id: muxtrix_domain::PaneId) -> u64 {
    pane_id.as_uuid().as_u128() as u64
}

/// The same, for a tab.
pub(crate) fn tab_key(tab_id: muxtrix_domain::TabId) -> u64 {
    tab_id.as_uuid().as_u128() as u64
}

/// The terminal face, resolved the way the grid resolves it, for chrome that
/// shows a program name in monospace.
pub(crate) fn terminal_family(settings: &crate::settings::AppSettings) -> gpui::SharedString {
    settings
        .terminal_font
        .family_name()
        .map_or_else(
            || {
                crate::metrics::system_monospace_family()
                    .unwrap_or("monospace")
                    .to_owned()
            },
            ToOwned::to_owned,
        )
        .into()
}

/// The leading mark in a rail row's gutter. Where you already are is one
/// solid bar; where the keyboard cursor stands is a ladder of accent rungs,
/// so the two never read as the same thing.
pub(crate) fn rail_marker(selected: bool, targeted: bool, tokens: DesignTokens) -> Div {
    let mut marker = div().flex().flex_col().w(px(3.)).flex_shrink_0();
    if !targeted {
        return marker.bg(color(if selected {
            tokens.accent
        } else {
            iced::Color::TRANSPARENT
        }));
    }
    for rung in 0..crate::app::RAIL_CURSOR_RUNGS {
        let filled = rung % 2 == 0;
        marker = marker.child(div().w(px(3.)).flex_grow(1.0).bg(color(if filled {
            tokens.accent
        } else {
            iced::Color::TRANSPARENT
        })));
    }
    marker
}
