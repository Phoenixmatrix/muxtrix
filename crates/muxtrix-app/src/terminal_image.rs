//! Where an inline image lands on the grid.
//!
//! Ghostty reports placements in integer pixels on its own idea of the cell;
//! the renderer's cells are fractional, so the placement is scaled onto the
//! render grid and the source crop is expressed as the full image's rectangle
//! behind a clip. Renderers draw from [`PlacementGeometry`].

use muxtrix_terminal::ImageSourceRect;

use crate::geom::Rect;
#[cfg(test)]
use crate::geom::{Point, Size};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlacementGeometry {
    pub(crate) destination: Rect,
    pub(crate) full_image: Rect,
}

pub(crate) fn scaled_destination(
    grid: (i32, i32),
    pixels: (u32, u32),
    offsets: (u32, u32),
    cell: (f32, f32),
) -> Rect {
    let cell_scale_x = cell.0 / cell.0.round().max(1.0);
    let cell_scale_y = cell.1 / cell.1.round().max(1.0);
    Rect {
        x: grid.0 as f32 * cell.0 + offsets.0 as f32 * cell_scale_x,
        y: grid.1 as f32 * cell.1 + offsets.1 as f32 * cell_scale_y,
        width: pixels.0 as f32 * cell_scale_x,
        height: pixels.1 as f32 * cell_scale_y,
    }
}

pub(crate) fn placement_geometry(
    destination: Rect,
    source: ImageSourceRect,
    image_width: u32,
    image_height: u32,
) -> Option<PlacementGeometry> {
    if destination.width <= 0.0
        || destination.height <= 0.0
        || source.width == 0
        || source.height == 0
        || image_width == 0
        || image_height == 0
    {
        return None;
    }
    let scale_x = destination.width / source.width as f32;
    let scale_y = destination.height / source.height as f32;
    Some(PlacementGeometry {
        destination,
        full_image: Rect {
            x: destination.x - source.x as f32 * scale_x,
            y: destination.y - source.y as f32 * scale_y,
            width: image_width as f32 * scale_x,
            height: image_height as f32 * scale_y,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_crop_scales_the_full_texture_behind_its_destination_clip() {
        let geometry = placement_geometry(
            Rect {
                x: 20.0,
                y: 30.0,
                width: 80.0,
                height: 40.0,
            },
            ImageSourceRect {
                x: 50,
                y: 25,
                width: 100,
                height: 50,
            },
            200,
            100,
        )
        .expect("placement geometry");

        assert_eq!(geometry.destination.position(), Point::new(20.0, 30.0));
        assert_eq!(geometry.full_image.position(), Point::new(-20.0, 10.0));
        assert_eq!(geometry.full_image.size(), Size::new(160.0, 80.0));
    }

    #[test]
    fn ghostty_integer_pixel_extents_scale_to_the_fractional_render_grid() {
        let destination = scaled_destination((4, 3), (264, 252), (5, 7), (11.2, 21.0));

        assert!((destination.x - 49.890_91).abs() < 0.001);
        assert_eq!(destination.y, 70.0);
        assert!((destination.width - 268.8).abs() < 0.001);
        assert_eq!(destination.height, 252.0);
    }
}
