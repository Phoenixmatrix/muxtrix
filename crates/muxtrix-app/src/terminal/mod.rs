//! Terminal rendering: everything between a VT grid snapshot and pixels.
//!
//! The VT state machine itself lives in `muxtrix-terminal`; nothing here
//! interprets escape sequences. These modules only decide how an already
//! parsed grid is drawn.

pub(crate) mod box_painter;
pub(crate) mod element;
pub(crate) mod runs;
