//! The view layer: `Muxtrix` state in, `Element<Message>` out.
//!
//! Every module here is pure rendering. Nothing mutates state and nothing
//! performs I/O — a view returns an element tree whose interactions produce
//! [`Message`]s, and [`crate::Muxtrix::update`] decides what they mean. The
//! split mirrors the screens: shell, sidebar, workspace, panes, dialogs,
//! palette, GitHub panel and settings.

pub(crate) mod dialogs;
pub(crate) mod github;
pub(crate) mod palette;
pub(crate) mod panes;
pub(crate) mod root;
pub(crate) mod settings;
pub(crate) mod sidebar;
pub(crate) mod workspace;

/// What nearly every view module needs, in one import.
///
/// View code reaches for the same two dozen names constantly (layout widgets,
/// tokens, the message type). Naming them once here keeps each module's head
/// about the module rather than about `use` lines.
pub(crate) mod prelude {
    // `column` is deliberately absent: `column!` would then be a glob-imported
    // macro competing with std's `column!`, which is an ambiguity error. Each
    // view module imports it explicitly instead.
    pub(crate) use iced::widget::{
        button, container, mouse_area, opaque, pick_list, row, scrollable, slider, stack, text,
        text_input, toggler, tooltip,
    };
    pub(crate) use iced::{
        Alignment, Border, Color, Element, Fill, Length, Padding, Pixels, Shadow, Vector, font,
    };

    pub(crate) use crate::app::{IconKind, Message, Muxtrix, icon};
    pub(crate) use crate::settings::FontWeight;
    pub(crate) use crate::theme::DesignTokens;
}
