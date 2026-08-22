//! App-owned geometry: the shapes [`crate::Message`] and `update` speak in.
//!
//! These deliberately duplicate `iced::Point`, `iced::Size` and
//! `iced::mouse::ScrollDelta` rather than re-using them. State and message
//! payloads are application facts, not rendering facts, so they should not
//! change type when the renderer does — and unit tests can build them without
//! standing up a UI framework. Conversions at the boundary keep the view layer
//! talking to iced in iced's own terms.

/// A position in logical pixels, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Point {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

impl Point {
    pub(crate) const ORIGIN: Self = Self { x: 0.0, y: 0.0 };

    pub(crate) const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl From<iced::Point> for Point {
    fn from(value: iced::Point) -> Self {
        Self::new(value.x, value.y)
    }
}

impl From<Point> for iced::Point {
    fn from(value: Point) -> Self {
        Self::new(value.x, value.y)
    }
}

/// An extent in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Size {
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl Size {
    pub(crate) const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

impl From<iced::Size> for Size {
    fn from(value: iced::Size) -> Self {
        Self::new(value.width, value.height)
    }
}

impl From<Size> for iced::Size {
    fn from(value: Size) -> Self {
        Self::new(value.width, value.height)
    }
}

/// One wheel or trackpad movement.
///
/// The two variants are not interchangeable: line deltas are discrete notches
/// and pixel deltas are continuous, so scroll handling scales them differently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ScrollDelta {
    Lines { x: f32, y: f32 },
    Pixels { x: f32, y: f32 },
}

impl From<iced::mouse::ScrollDelta> for ScrollDelta {
    fn from(value: iced::mouse::ScrollDelta) -> Self {
        match value {
            iced::mouse::ScrollDelta::Lines { x, y } => Self::Lines { x, y },
            iced::mouse::ScrollDelta::Pixels { x, y } => Self::Pixels { x, y },
        }
    }
}
