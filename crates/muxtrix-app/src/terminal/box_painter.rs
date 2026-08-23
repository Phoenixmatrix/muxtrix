//! Painting box-drawing geometry with GPUI.
//!
//! The geometry itself lives in [`crate::box_drawing`] and is shared with the
//! iced runtime: arms end exactly on cell edges so adjacent cells join into
//! one unbroken line, and that arithmetic must not be reimplemented per
//! framework. This module only knows how to put those shapes on screen.

use gpui::{Bounds, Hsla, PathBuilder, Pixels, Window, fill, point, px, size};

use crate::box_drawing::BoxPainter;

/// Collects box geometry for one run and paints it into a window.
pub(crate) struct GpuiBoxPainter<'a, 'w> {
    window: &'a mut Window,
    /// The run's top-left in window coordinates.
    origin: gpui::Point<Pixels>,
    color: Hsla,
    /// The current cell's left edge, in cell-local terms.
    offset_x: f32,
    _marker: std::marker::PhantomData<&'w ()>,
}

impl<'a> GpuiBoxPainter<'a, '_> {
    pub(crate) fn new(window: &'a mut Window, origin: gpui::Point<Pixels>, color: Hsla) -> Self {
        Self {
            window,
            origin,
            color,
            offset_x: 0.0,
            _marker: std::marker::PhantomData,
        }
    }

    fn at(&self, x: f32, y: f32) -> gpui::Point<Pixels> {
        point(self.origin.x + px(x + self.offset_x), self.origin.y + px(y))
    }
}

impl BoxPainter for GpuiBoxPainter<'_, '_> {
    fn fill_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        // A quad rather than a filled path: these are axis-aligned and by far
        // the most common shape in the set, and a quad avoids tessellation.
        let origin = self.at(x0, y0);
        self.window.paint_quad(fill(
            Bounds {
                origin,
                size: size(px(x1 - x0), px(y1 - y0)),
            },
            self.color,
        ));
    }

    fn stroke_polyline(&mut self, points: &[(f32, f32)], width: f32) {
        let Some((first, rest)) = points.split_first() else {
            return;
        };
        let mut builder = PathBuilder::stroke(px(width));
        builder.move_to(self.at(first.0, first.1));
        for point in rest {
            builder.line_to(self.at(point.0, point.1));
        }
        if let Ok(path) = builder.build() {
            self.window.paint_path(path, self.color);
        }
    }

    fn stroke_rounded_corner(
        &mut self,
        start: (f32, f32),
        arc: [(f32, f32); 4],
        end: (f32, f32),
        width: f32,
    ) {
        let mut builder = PathBuilder::stroke(px(width));
        builder.move_to(self.at(start.0, start.1));
        builder.line_to(self.at(arc[0].0, arc[0].1));
        builder.cubic_bezier_to(
            self.at(arc[3].0, arc[3].1),
            self.at(arc[1].0, arc[1].1),
            self.at(arc[2].0, arc[2].1),
        );
        builder.line_to(self.at(end.0, end.1));
        if let Ok(path) = builder.build() {
            self.window.paint_path(path, self.color);
        }
    }

    fn with_cell<F: FnOnce(&mut Self)>(&mut self, offset_x: f32, draw: F) {
        let previous = self.offset_x;
        self.offset_x = offset_x;
        draw(self);
        self.offset_x = previous;
    }
}
