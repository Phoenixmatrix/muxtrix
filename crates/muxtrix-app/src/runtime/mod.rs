//! Runtimes: the part of the program that owns the window and the event loop.
//!
//! The application core is framework-agnostic — `update` returns
//! [`crate::effect::Effect`] values rather than performing anything — so a
//! runtime is a comparatively small adapter.

pub(crate) mod gpui;

pub(crate) use gpui::run;
