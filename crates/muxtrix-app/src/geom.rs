//! App-owned geometry: the shapes [`crate::Message`] and `update` speak in.
//!
//! These are the app's own rather than the framework's. State and message
//! payloads are application facts, not rendering facts, so they should not
//! change type when the renderer does — and unit tests can build them without
//! standing up a UI framework. Conversions at the boundary keep the view layer
//! talking to the framework in its own terms.

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

/// One wheel or trackpad movement.
///
/// The two variants are not interchangeable: line deltas are discrete notches
/// and pixel deltas are continuous, so scroll handling scales them differently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ScrollDelta {
    Lines { x: f32, y: f32 },
    Pixels { x: f32, y: f32 },
}

/// An axis-aligned rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Rect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

#[cfg(test)]
impl Rect {
    pub(crate) const fn position(&self) -> Point {
        Point::new(self.x, self.y)
    }

    pub(crate) const fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}
