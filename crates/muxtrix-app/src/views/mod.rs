//! The view layer: `Muxtrix` state in, elements out.
//!
//! Every module here is pure rendering. Nothing mutates state and nothing
//! performs I/O — a view returns an element tree whose interactions produce
//! [`crate::app::Message`]s, and [`crate::Muxtrix::update`] decides what they
//! mean.

pub(crate) mod gpui;
