//! Runtimes: the part of the program that owns the window and the event loop.
//!
//! The application core is framework-agnostic — `update` returns
//! [`crate::effect::Effect`] values rather than performing anything — so a
//! runtime is a comparatively small adapter. Exactly one is compiled in.

#[cfg(feature = "gpui")]
pub(crate) mod gpui;
#[cfg(not(feature = "gpui"))]
pub(crate) mod iced;

#[cfg(feature = "gpui")]
pub(crate) use gpui::run;
#[cfg(not(feature = "gpui"))]
pub(crate) use iced::run;
