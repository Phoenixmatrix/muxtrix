use std::collections::BTreeMap;

use iced::advanced::image::{FilterMethod, Handle, Image};
use iced::mouse;
use iced::widget::canvas::{self, Geometry};
use iced::{Element, Length, Rectangle, Renderer, Theme};
use muxtrix_terminal::{GridSnapshot, ImageLayer, ImageSourceRect};

use crate::Message;

pub(crate) fn layer(
    snapshot: &GridSnapshot,
    handles: &BTreeMap<u64, Handle>,
    image_layer: ImageLayer,
    cell_width: f32,
    cell_height: f32,
) -> Element<'static, Message> {
    let placements = snapshot
        .images
        .iter()
        .filter(|placement| placement.layer == image_layer)
        .filter_map(|placement| {
            let handle = handles.get(&placement.image.generation)?.clone();
            let destination = scaled_destination(
                (placement.column, placement.row),
                (placement.width, placement.height),
                (placement.x_offset, placement.y_offset),
                (cell_width, cell_height),
            );
            let geometry = placement_geometry(
                destination,
                placement.source,
                placement.image.width,
                placement.image.height,
            )?;
            Some(DrawPlacement { handle, geometry })
        })
        .collect();
    let width = snapshot
        .cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| usize::from(cell.columns))
                .sum::<usize>()
        })
        .max()
        .unwrap_or_default() as f32
        * cell_width;
    let height = snapshot.cells.len() as f32 * cell_height;

    canvas::Canvas::new(TerminalImageCanvas { placements })
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .into()
}

#[derive(Debug, Clone)]
struct TerminalImageCanvas {
    placements: Vec<DrawPlacement>,
}

#[derive(Debug, Clone)]
struct DrawPlacement {
    handle: Handle,
    geometry: PlacementGeometry,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PlacementGeometry {
    destination: Rectangle,
    full_image: Rectangle,
}

fn scaled_destination(
    grid: (i32, i32),
    pixels: (u32, u32),
    offsets: (u32, u32),
    cell: (f32, f32),
) -> Rectangle {
    let cell_scale_x = cell.0 / cell.0.round().max(1.0);
    let cell_scale_y = cell.1 / cell.1.round().max(1.0);
    Rectangle {
        x: grid.0 as f32 * cell.0 + offsets.0 as f32 * cell_scale_x,
        y: grid.1 as f32 * cell.1 + offsets.1 as f32 * cell_scale_y,
        width: pixels.0 as f32 * cell_scale_x,
        height: pixels.1 as f32 * cell_scale_y,
    }
}

fn placement_geometry(
    destination: Rectangle,
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
        full_image: Rectangle {
            x: destination.x - source.x as f32 * scale_x,
            y: destination.y - source.y as f32 * scale_y,
            width: image_width as f32 * scale_x,
            height: image_height as f32 * scale_y,
        },
    })
}

impl canvas::Program<Message> for TerminalImageCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        for placement in &self.placements {
            frame.with_clip(placement.geometry.destination, |frame| {
                frame.draw_image(
                    placement.geometry.full_image,
                    Image::new(placement.handle.clone()).filter_method(FilterMethod::Linear),
                );
            });
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_crop_scales_the_full_texture_behind_its_destination_clip() {
        let geometry = placement_geometry(
            Rectangle {
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

        assert_eq!(
            geometry.destination.position(),
            iced::Point::new(20.0, 30.0)
        );
        assert_eq!(
            geometry.full_image.position(),
            iced::Point::new(-20.0, 10.0)
        );
        assert_eq!(geometry.full_image.size(), iced::Size::new(160.0, 80.0));
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
