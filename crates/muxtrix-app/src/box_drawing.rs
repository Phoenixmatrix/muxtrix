//! Font-independent rendering for the Unicode Box Drawing block.
//!
//! Ghostty resolves U+2500..=U+257F to a built-in sprite face before it
//! considers configured fonts. Muxtrix follows the same renderer-level rule:
//! every arm terminates on the terminal cell edge, so adjacent cells share
//! coordinates instead of relying on unrelated font outlines and bearings.
//!
//! The character specifications and drawing algorithms are adapted from
//! Ghostty's MIT-licensed `src/font/sprite/draw/box.zig`.

/// What drawing box glyphs needs from a renderer.
///
/// The geometry in this module is the whole point — arms end exactly on cell
/// edges so adjacent cells join into one unbroken line — and it must not be
/// rewritten per framework. This is the entire surface it draws through, in
/// cell-local coordinates with the origin at the glyph's top-left.
pub(crate) trait BoxPainter {
    /// A filled axis-aligned rectangle. Used for every solid arm and block.
    fn fill_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32);

    /// A stroked polyline through `points`, for diagonals.
    fn stroke_polyline(&mut self, points: &[(f32, f32)], width: f32);

    /// A stroked path that starts at `start`, runs to the corner arc, sweeps
    /// it as a cubic, and ends at `end`. Rounded corners are the only curve
    /// the set needs.
    fn stroke_rounded_corner(
        &mut self,
        start: (f32, f32),
        arc: [(f32, f32); 4],
        end: (f32, f32),
        width: f32,
    );

    /// Move the origin to the given cell, so each glyph draws at (0, 0).
    fn with_cell<F: FnOnce(&mut Self)>(&mut self, offset_x: f32, draw: F);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    None,
    Light,
    Heavy,
    Double,
}

use Style::{Double as D, Heavy as H, Light as L, None as N};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Lines {
    up: Style,
    right: Style,
    down: Style,
    left: Style,
}

impl Lines {
    const fn new(up: Style, right: Style, down: Style, left: Style) -> Self {
        Self {
            up,
            right,
            down,
            left,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashGap {
    Wide,
    Light,
    Heavy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Diagonal {
    Rising,
    Falling,
    Cross,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Glyph {
    Lines(Lines),
    Dashes {
        axis: Axis,
        count: u8,
        style: Style,
        gap: DashGap,
    },
    Arc(Corner),
    Diagonal(Diagonal),
}

/// Maps the complete Unicode Box Drawing block to semantic cell geometry.
/// Edge order for `Lines::new` is up, right, down, left.
fn glyph(character: char) -> Option<Glyph> {
    let lines = |up, right, down, left| Glyph::Lines(Lines::new(up, right, down, left));
    Some(match character as u32 {
        0x2500 => lines(N, L, N, L),
        0x2501 => lines(N, H, N, H),
        0x2502 => lines(L, N, L, N),
        0x2503 => lines(H, N, H, N),
        0x2504 => Glyph::Dashes {
            axis: Axis::Horizontal,
            count: 3,
            style: L,
            gap: DashGap::Wide,
        },
        0x2505 => Glyph::Dashes {
            axis: Axis::Horizontal,
            count: 3,
            style: H,
            gap: DashGap::Wide,
        },
        0x2506 => Glyph::Dashes {
            axis: Axis::Vertical,
            count: 3,
            style: L,
            gap: DashGap::Wide,
        },
        0x2507 => Glyph::Dashes {
            axis: Axis::Vertical,
            count: 3,
            style: H,
            gap: DashGap::Wide,
        },
        0x2508 => Glyph::Dashes {
            axis: Axis::Horizontal,
            count: 4,
            style: L,
            gap: DashGap::Wide,
        },
        0x2509 => Glyph::Dashes {
            axis: Axis::Horizontal,
            count: 4,
            style: H,
            gap: DashGap::Wide,
        },
        0x250a => Glyph::Dashes {
            axis: Axis::Vertical,
            count: 4,
            style: L,
            gap: DashGap::Wide,
        },
        0x250b => Glyph::Dashes {
            axis: Axis::Vertical,
            count: 4,
            style: H,
            gap: DashGap::Wide,
        },
        0x250c => lines(N, L, L, N),
        0x250d => lines(N, H, L, N),
        0x250e => lines(N, L, H, N),
        0x250f => lines(N, H, H, N),
        0x2510 => lines(N, N, L, L),
        0x2511 => lines(N, N, L, H),
        0x2512 => lines(N, N, H, L),
        0x2513 => lines(N, N, H, H),
        0x2514 => lines(L, L, N, N),
        0x2515 => lines(L, H, N, N),
        0x2516 => lines(H, L, N, N),
        0x2517 => lines(H, H, N, N),
        0x2518 => lines(L, N, N, L),
        0x2519 => lines(L, N, N, H),
        0x251a => lines(H, N, N, L),
        0x251b => lines(H, N, N, H),
        0x251c => lines(L, L, L, N),
        0x251d => lines(L, H, L, N),
        0x251e => lines(H, L, L, N),
        0x251f => lines(L, L, H, N),
        0x2520 => lines(H, L, H, N),
        0x2521 => lines(H, H, L, N),
        0x2522 => lines(L, H, H, N),
        0x2523 => lines(H, H, H, N),
        0x2524 => lines(L, N, L, L),
        0x2525 => lines(L, N, L, H),
        0x2526 => lines(H, N, L, L),
        0x2527 => lines(L, N, H, L),
        0x2528 => lines(H, N, H, L),
        0x2529 => lines(H, N, L, H),
        0x252a => lines(L, N, H, H),
        0x252b => lines(H, N, H, H),
        0x252c => lines(N, L, L, L),
        0x252d => lines(N, L, L, H),
        0x252e => lines(N, H, L, L),
        0x252f => lines(N, H, L, H),
        0x2530 => lines(N, L, H, L),
        0x2531 => lines(N, L, H, H),
        0x2532 => lines(N, H, H, L),
        0x2533 => lines(N, H, H, H),
        0x2534 => lines(L, L, N, L),
        0x2535 => lines(L, L, N, H),
        0x2536 => lines(L, H, N, L),
        0x2537 => lines(L, H, N, H),
        0x2538 => lines(H, L, N, L),
        0x2539 => lines(H, L, N, H),
        0x253a => lines(H, H, N, L),
        0x253b => lines(H, H, N, H),
        0x253c => lines(L, L, L, L),
        0x253d => lines(L, L, L, H),
        0x253e => lines(L, H, L, L),
        0x253f => lines(L, H, L, H),
        0x2540 => lines(H, L, L, L),
        0x2541 => lines(L, L, H, L),
        0x2542 => lines(H, L, H, L),
        0x2543 => lines(H, L, L, H),
        0x2544 => lines(H, H, L, L),
        0x2545 => lines(L, L, H, H),
        0x2546 => lines(L, H, H, L),
        0x2547 => lines(H, H, L, H),
        0x2548 => lines(L, H, H, H),
        0x2549 => lines(H, L, H, H),
        0x254a => lines(H, H, H, L),
        0x254b => lines(H, H, H, H),
        0x254c => Glyph::Dashes {
            axis: Axis::Horizontal,
            count: 2,
            style: L,
            gap: DashGap::Light,
        },
        0x254d => Glyph::Dashes {
            axis: Axis::Horizontal,
            count: 2,
            style: H,
            gap: DashGap::Heavy,
        },
        0x254e => Glyph::Dashes {
            axis: Axis::Vertical,
            count: 2,
            style: L,
            gap: DashGap::Heavy,
        },
        0x254f => Glyph::Dashes {
            axis: Axis::Vertical,
            count: 2,
            style: H,
            gap: DashGap::Heavy,
        },
        0x2550 => lines(N, D, N, D),
        0x2551 => lines(D, N, D, N),
        0x2552 => lines(N, D, L, N),
        0x2553 => lines(N, L, D, N),
        0x2554 => lines(N, D, D, N),
        0x2555 => lines(N, N, L, D),
        0x2556 => lines(N, N, D, L),
        0x2557 => lines(N, N, D, D),
        0x2558 => lines(L, D, N, N),
        0x2559 => lines(D, L, N, N),
        0x255a => lines(D, D, N, N),
        0x255b => lines(L, N, N, D),
        0x255c => lines(D, N, N, L),
        0x255d => lines(D, N, N, D),
        0x255e => lines(L, D, L, N),
        0x255f => lines(D, L, D, N),
        0x2560 => lines(D, D, D, N),
        0x2561 => lines(L, N, L, D),
        0x2562 => lines(D, N, D, L),
        0x2563 => lines(D, N, D, D),
        0x2564 => lines(N, D, L, D),
        0x2565 => lines(N, L, D, L),
        0x2566 => lines(N, D, D, D),
        0x2567 => lines(L, D, N, D),
        0x2568 => lines(D, L, N, L),
        0x2569 => lines(D, D, N, D),
        0x256a => lines(L, D, L, D),
        0x256b => lines(D, L, D, L),
        0x256c => lines(D, D, D, D),
        0x256d => Glyph::Arc(Corner::TopLeft),
        0x256e => Glyph::Arc(Corner::TopRight),
        0x256f => Glyph::Arc(Corner::BottomRight),
        0x2570 => Glyph::Arc(Corner::BottomLeft),
        0x2571 => Glyph::Diagonal(Diagonal::Rising),
        0x2572 => Glyph::Diagonal(Diagonal::Falling),
        0x2573 => Glyph::Diagonal(Diagonal::Cross),
        0x2574 => lines(N, N, N, L),
        0x2575 => lines(L, N, N, N),
        0x2576 => lines(N, L, N, N),
        0x2577 => lines(N, N, L, N),
        0x2578 => lines(N, N, N, H),
        0x2579 => lines(H, N, N, N),
        0x257a => lines(N, H, N, N),
        0x257b => lines(N, N, H, N),
        0x257c => lines(N, H, N, L),
        0x257d => lines(L, N, H, N),
        0x257e => lines(N, L, N, H),
        0x257f => lines(H, N, L, N),
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BoxMetrics {
    width: f32,
    height: f32,
    light: f32,
    heavy: f32,
}

impl BoxMetrics {
    pub(crate) fn new(width: f32, height: f32) -> Self {
        // Both renderers work in logical pixels, so one logical pixel scales
        // with the display and matches Ghostty's one-pixel base sprite
        // thickness.
        Self {
            width,
            height,
            light: 1.0,
            heavy: 2.0,
        }
    }

    const fn thickness(self, style: Style) -> f32 {
        match style {
            Style::Heavy => self.heavy,
            Style::None | Style::Light | Style::Double => self.light,
        }
    }
}

fn draw_character(painter: &mut impl BoxPainter, metrics: BoxMetrics, character: char) {
    let Some(glyph) = glyph(character) else {
        return;
    };
    match glyph {
        Glyph::Lines(lines) => draw_lines(painter, metrics, lines),
        Glyph::Dashes {
            axis,
            count,
            style,
            gap,
        } => draw_dashes(painter, metrics, axis, count, style, gap),
        Glyph::Arc(corner) => draw_arc(painter, metrics, corner),
        Glyph::Diagonal(diagonal) => draw_diagonal(painter, metrics, diagonal),
    }
}

fn draw_lines(painter: &mut impl BoxPainter, metrics: BoxMetrics, lines: Lines) {
    let light = metrics.light;
    let heavy = metrics.heavy;
    let h_light_top = (metrics.height - light).max(0.0) / 2.0;
    let h_light_bottom = h_light_top + light;
    let h_heavy_top = (metrics.height - heavy).max(0.0) / 2.0;
    let h_heavy_bottom = h_heavy_top + heavy;
    let h_double_top = (h_light_top - light).max(0.0);
    let h_double_bottom = (h_light_bottom + light).min(metrics.height);
    let v_light_left = (metrics.width - light).max(0.0) / 2.0;
    let v_light_right = v_light_left + light;
    let v_heavy_left = (metrics.width - heavy).max(0.0) / 2.0;
    let v_heavy_right = v_heavy_left + heavy;
    let v_double_left = (v_light_left - light).max(0.0);
    let v_double_right = (v_light_right + light).min(metrics.width);

    let up_bottom = if lines.left == H || lines.right == H {
        h_heavy_bottom
    } else if lines.left != lines.right || lines.down == lines.up {
        if lines.left == D || lines.right == D {
            h_double_bottom
        } else {
            h_light_bottom
        }
    } else if lines.left == N && lines.right == N {
        h_light_bottom
    } else {
        h_light_top
    };
    let down_top = if lines.left == H || lines.right == H {
        h_heavy_top
    } else if lines.left != lines.right || lines.up == lines.down {
        if lines.left == D || lines.right == D {
            h_double_top
        } else {
            h_light_top
        }
    } else if lines.left == N && lines.right == N {
        h_light_top
    } else {
        h_light_bottom
    };
    let left_right = if lines.up == H || lines.down == H {
        v_heavy_right
    } else if lines.up != lines.down || lines.left == lines.right {
        if lines.up == D || lines.down == D {
            v_double_right
        } else {
            v_light_right
        }
    } else if lines.up == N && lines.down == N {
        v_light_right
    } else {
        v_light_left
    };
    let right_left = if lines.up == H || lines.down == H {
        v_heavy_left
    } else if lines.up != lines.down || lines.right == lines.left {
        if lines.up == D || lines.down == D {
            v_double_left
        } else {
            v_light_left
        }
    } else if lines.up == N && lines.down == N {
        v_light_left
    } else {
        v_light_right
    };

    match lines.up {
        N => {}
        L => fill_box(painter, v_light_left, 0.0, v_light_right, up_bottom),
        H => fill_box(painter, v_heavy_left, 0.0, v_heavy_right, up_bottom),
        D => {
            let left_bottom = if lines.left == D {
                h_light_top
            } else {
                up_bottom
            };
            let right_bottom = if lines.right == D {
                h_light_top
            } else {
                up_bottom
            };
            fill_box(painter, v_double_left, 0.0, v_light_left, left_bottom);
            fill_box(painter, v_light_right, 0.0, v_double_right, right_bottom);
        }
    }
    match lines.right {
        N => {}
        L => fill_box(
            painter,
            right_left,
            h_light_top,
            metrics.width,
            h_light_bottom,
        ),
        H => fill_box(
            painter,
            right_left,
            h_heavy_top,
            metrics.width,
            h_heavy_bottom,
        ),
        D => {
            let top_left = if lines.up == D {
                v_light_right
            } else {
                right_left
            };
            let bottom_left = if lines.down == D {
                v_light_right
            } else {
                right_left
            };
            fill_box(painter, top_left, h_double_top, metrics.width, h_light_top);
            fill_box(
                painter,
                bottom_left,
                h_light_bottom,
                metrics.width,
                h_double_bottom,
            );
        }
    }
    match lines.down {
        N => {}
        L => fill_box(
            painter,
            v_light_left,
            down_top,
            v_light_right,
            metrics.height,
        ),
        H => fill_box(
            painter,
            v_heavy_left,
            down_top,
            v_heavy_right,
            metrics.height,
        ),
        D => {
            let left_top = if lines.left == D {
                h_light_bottom
            } else {
                down_top
            };
            let right_top = if lines.right == D {
                h_light_bottom
            } else {
                down_top
            };
            fill_box(
                painter,
                v_double_left,
                left_top,
                v_light_left,
                metrics.height,
            );
            fill_box(
                painter,
                v_light_right,
                right_top,
                v_double_right,
                metrics.height,
            );
        }
    }
    match lines.left {
        N => {}
        L => fill_box(painter, 0.0, h_light_top, left_right, h_light_bottom),
        H => fill_box(painter, 0.0, h_heavy_top, left_right, h_heavy_bottom),
        D => {
            let top_right = if lines.up == D {
                v_light_left
            } else {
                left_right
            };
            let bottom_right = if lines.down == D {
                v_light_left
            } else {
                left_right
            };
            fill_box(painter, 0.0, h_double_top, top_right, h_light_top);
            fill_box(painter, 0.0, h_light_bottom, bottom_right, h_double_bottom);
        }
    }
}

fn draw_dashes(
    painter: &mut impl BoxPainter,
    metrics: BoxMetrics,
    axis: Axis,
    count: u8,
    style: Style,
    gap: DashGap,
) {
    let count = f32::from(count);
    let thickness = metrics.thickness(style);
    let desired_gap = match gap {
        DashGap::Wide => 4.0_f32.max(metrics.light),
        DashGap::Light => metrics.light,
        DashGap::Heavy => metrics.heavy,
    };
    let span = match axis {
        Axis::Horizontal => metrics.width,
        Axis::Vertical => metrics.height,
    };
    if span < count * 2.0 {
        draw_solid_axis(painter, metrics, axis, metrics.light);
        return;
    }
    let gap = desired_gap.min(span / (2.0 * count));
    let dash = (span - count * gap) / count;

    match axis {
        Axis::Horizontal => {
            let y = (metrics.height - thickness).max(0.0) / 2.0;
            let mut x = gap / 2.0;
            for _ in 0..count as usize {
                fill_box(painter, x, y, x + dash, y + thickness);
                x += dash + gap;
            }
        }
        Axis::Vertical => {
            let x = (metrics.width - thickness).max(0.0) / 2.0;
            let mut y = 0.0;
            for _ in 0..count as usize {
                fill_box(painter, x, y, x + thickness, y + dash);
                y += dash + gap;
            }
        }
    }
}

fn draw_solid_axis(painter: &mut impl BoxPainter, metrics: BoxMetrics, axis: Axis, thickness: f32) {
    match axis {
        Axis::Horizontal => {
            let y = (metrics.height - thickness).max(0.0) / 2.0;
            fill_box(painter, 0.0, y, metrics.width, y + thickness);
        }
        Axis::Vertical => {
            let x = (metrics.width - thickness).max(0.0) / 2.0;
            fill_box(painter, x, 0.0, x + thickness, metrics.height);
        }
    }
}

fn draw_arc(painter: &mut impl BoxPainter, metrics: BoxMetrics, corner: Corner) {
    let center_x = metrics.width / 2.0;
    let center_y = metrics.height / 2.0;
    let radius = metrics.width.min(metrics.height) / 2.0;
    let control = radius * 0.25;
    // Each corner is: come in from one cell edge to where the curve starts,
    // sweep a cubic through the centre, leave to the other edge. The entry and
    // exit both land exactly on an edge midpoint, which is what lets a rounded
    // corner meet a straight arm from the neighbouring cell without a seam.
    let (start, arc, end) = match corner {
        Corner::TopLeft => (
            (center_x, metrics.height),
            [
                (center_x, center_y + radius),
                (center_x, center_y + control),
                (center_x + control, center_y),
                (center_x + radius, center_y),
            ],
            (metrics.width, center_y),
        ),
        Corner::TopRight => (
            (center_x, metrics.height),
            [
                (center_x, center_y + radius),
                (center_x, center_y + control),
                (center_x - control, center_y),
                (center_x - radius, center_y),
            ],
            (0.0, center_y),
        ),
        Corner::BottomLeft => (
            (center_x, 0.0),
            [
                (center_x, center_y - radius),
                (center_x, center_y - control),
                (center_x + control, center_y),
                (center_x + radius, center_y),
            ],
            (metrics.width, center_y),
        ),
        Corner::BottomRight => (
            (center_x, 0.0),
            [
                (center_x, center_y - radius),
                (center_x, center_y - control),
                (center_x - control, center_y),
                (center_x - radius, center_y),
            ],
            (0.0, center_y),
        ),
    };
    painter.stroke_rounded_corner(start, arc, end, metrics.light);
}

fn draw_diagonal(painter: &mut impl BoxPainter, metrics: BoxMetrics, diagonal: Diagonal) {
    // Overshooting the cell by half a slope on each end is what makes a run of
    // diagonals read as one unbroken line instead of a dashed one.
    let slope_x = (metrics.width / metrics.height).min(1.0);
    let slope_y = (metrics.height / metrics.width).min(1.0);
    let rising = [
        (metrics.width + 0.5 * slope_x, -0.5 * slope_y),
        (-0.5 * slope_x, metrics.height + 0.5 * slope_y),
    ];
    let falling = [
        (-0.5 * slope_x, -0.5 * slope_y),
        (
            metrics.width + 0.5 * slope_x,
            metrics.height + 0.5 * slope_y,
        ),
    ];
    if matches!(diagonal, Diagonal::Rising | Diagonal::Cross) {
        painter.stroke_polyline(&rising, metrics.light);
    }
    if matches!(diagonal, Diagonal::Falling | Diagonal::Cross) {
        painter.stroke_polyline(&falling, metrics.light);
    }
}

fn fill_box(painter: &mut impl BoxPainter, x0: f32, y0: f32, x1: f32, y1: f32) {
    if x1 > x0 && y1 > y0 {
        painter.fill_rect(x0, y0, x1, y1);
    }
}

/// Draw one box-drawing character at `offset_x` within the run.
///
/// The entry point for a renderer's own painter.
pub(crate) fn draw_cell(
    painter: &mut impl BoxPainter,
    metrics: BoxMetrics,
    character: char,
    offset_x: f32,
) {
    painter.with_cell(offset_x, |painter| {
        draw_character(painter, metrics, character);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_box_drawing_codepoint_has_semantic_geometry() {
        for codepoint in 0x2500..=0x257f {
            let character = char::from_u32(codepoint).expect("box-drawing codepoint is valid");
            assert!(glyph(character).is_some(), "missing U+{codepoint:04X}");
        }
        assert_eq!(glyph('\u{24ff}'), None);
        assert_eq!(glyph('\u{2580}'), None);
    }

    #[test]
    fn representative_glyphs_keep_their_terminal_semantics() {
        assert_eq!(glyph('─'), Some(Glyph::Lines(Lines::new(N, L, N, L))));
        assert_eq!(glyph('┏'), Some(Glyph::Lines(Lines::new(N, H, H, N))));
        assert_eq!(glyph('╬'), Some(Glyph::Lines(Lines::new(D, D, D, D))));
        assert_eq!(glyph('╭'), Some(Glyph::Arc(Corner::TopLeft)));
        assert_eq!(
            glyph('╎'),
            Some(Glyph::Dashes {
                axis: Axis::Vertical,
                count: 2,
                style: L,
                gap: DashGap::Heavy,
            })
        );
        assert_eq!(glyph('╳'), Some(Glyph::Diagonal(Diagonal::Cross)));
    }
}
