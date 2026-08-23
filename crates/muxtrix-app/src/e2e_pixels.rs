//! Pixel assertions on a captured frame.
//!
//! Shared by the application and the e2e harness, because which process holds
//! the pixels depends on the renderer: iced photographs its own window, and
//! GPUI cannot outside its test platform, so the harness grabs the frame from
//! the X server instead. The checks themselves are the same either way, and
//! they are the ones that would catch a terminal drawing glyphs that no longer
//! join across cells.

use std::collections::VecDeque;

/// A block glyph run this wide is a solid block rather than glyph noise.
///
/// Kept in step with the scenario's own threshold; both describe the same
/// fixture.
const TERMINAL_BLOCK_CONTINUITY_PIXELS: usize = 120;

/// A captured frame: 8-bit RGBA, rows top to bottom, no padding.
pub struct Frame<'a> {
    pub rgba: &'a [u8],
    pub width: usize,
    pub height: usize,
}

pub fn light_horizontal_continuity(frame: &Frame<'_>) -> (usize, usize) {
    let row_bytes = frame.width * 4;
    let row_runs = frame
        .rgba
        .chunks_exact(row_bytes)
        .map(|row| {
            row.chunks_exact(4)
                .fold((0_usize, 0_usize), |(current, longest), pixel| {
                    let light =
                        pixel[3] == 255 && pixel[0] >= 128 && pixel[1] >= 128 && pixel[2] >= 128;
                    let current = if light { current + 1 } else { 0 };
                    (current, longest.max(current))
                })
                .1
        })
        .collect::<Vec<_>>();
    (
        row_runs.iter().copied().max().unwrap_or(0),
        row_runs
            .iter()
            .filter(|run| **run >= TERMINAL_BLOCK_CONTINUITY_PIXELS)
            .count(),
    )
}

/// Finds the bright-magenta rounded-border fixture and verifies that its top,
/// bottom, left, and right edges all belong to one 8-connected component.
pub fn magenta_rounded_box_continuity(frame: &Frame<'_>) -> (bool, usize, usize) {
    colored_box_continuity(frame, |pixel| {
        pixel[3] == 255 && pixel[0] >= 128 && pixel[1] <= 96 && pixel[2] >= 128
    })
}

/// Finds the bright-cyan heavy-border fixture and verifies all four edges.
pub fn cyan_heavy_box_continuity(frame: &Frame<'_>) -> (bool, usize, usize) {
    colored_box_continuity(frame, |pixel| {
        pixel[3] == 255 && pixel[0] <= 96 && pixel[1] >= 128 && pixel[2] >= 128
    })
}

pub fn colored_box_continuity(
    frame: &Frame<'_>,
    is_border_pixel: impl Fn(&[u8]) -> bool,
) -> (bool, usize, usize) {
    let width = frame.width;
    let height = frame.height;
    let is_border = frame
        .rgba
        .chunks_exact(4)
        .map(is_border_pixel)
        .collect::<Vec<_>>();
    let mut visited = vec![false; is_border.len()];
    let mut largest = Vec::new();

    for start in 0..is_border.len() {
        if !is_border[start] || visited[start] {
            continue;
        }
        visited[start] = true;
        let mut pending = VecDeque::from([start]);
        let mut component = Vec::new();
        while let Some(index) = pending.pop_front() {
            component.push(index);
            let x = index % width;
            let y = index / width;
            for next_y in y.saturating_sub(1)..=(y + 1).min(height.saturating_sub(1)) {
                for next_x in x.saturating_sub(1)..=(x + 1).min(width.saturating_sub(1)) {
                    let next = next_y * width + next_x;
                    if is_border[next] && !visited[next] {
                        visited[next] = true;
                        pending.push_back(next);
                    }
                }
            }
        }
        if component.len() > largest.len() {
            largest = component;
        }
    }

    let Some(min_x) = largest.iter().map(|index| index % width).min() else {
        return (false, 0, 0);
    };
    let max_x = largest
        .iter()
        .map(|index| index % width)
        .max()
        .unwrap_or(min_x);
    let min_y = largest.iter().map(|index| index / width).min().unwrap_or(0);
    let max_y = largest
        .iter()
        .map(|index| index / width)
        .max()
        .unwrap_or(min_y);
    let component_width = max_x - min_x + 1;
    let component_height = max_y - min_y + 1;
    let middle_x = (min_x + max_x) / 2;
    let middle_y = (min_y + max_y) / 2;
    let near = |index: &usize, target_x: usize, target_y: usize| {
        (index % width).abs_diff(target_x) <= 4 && (index / width).abs_diff(target_y) <= 4
    };
    let connected = [
        (middle_x, min_y),
        (middle_x, max_y),
        (min_x, middle_y),
        (max_x, middle_y),
    ]
    .into_iter()
    .all(|(x, y)| largest.iter().any(|index| near(index, x, y)));

    (connected, component_width, component_height)
}
