//! Does GPUI render Muxtrix's two hard cases on every platform we ship to?
//!
//! Phase 1 of the port plan is a measurement, not a feature. Two things carry
//! real risk: the stock widget set from `gpui-component`, and a dense
//! monospace grid drawn as one shaped line per row. This program puts both on
//! screen at Muxtrix's window size and reports what adapter drew them and how
//! long a frame took, so the numbers in `docs/GPU.md` come from a run rather
//! than an assumption.
//!
//! It is deliberately not Muxtrix. No settings, no PTY, no state.

use std::time::Instant;

use gpui::{
    App, AppContext, Bounds, Context, Font, FontFeatures, FontStyle, FontWeight,
    Hsla, IntoElement, ParentElement, Pixels, Render, SharedString, Styled, TextAlign, TextRun,
    Window, WindowBounds, WindowOptions, div, point, px, rgb, size,
};

/// Matches the terminal Muxtrix opens by default, which is the size the grid
/// has to stay cheap at.
const GRID_COLUMNS: usize = 200;
const GRID_ROWS: usize = 60;

fn main() {
    // The platform crate picks the backend for the host: Metal, DirectX, or
    // wgpu over Vulkan/GL on Linux.
    gpui_platform::application().run(|cx: &mut App| {
        gpui_component::init(cx);

        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Spike::new()),
        )
        .expect("the spike could not open a window");
    });
}

struct Spike {
    /// One row of text per grid row, generated once. The point of the spike is
    /// the cost of shaping and painting, not of inventing content.
    rows: Vec<SharedString>,
    frames: u32,
    started: Instant,
}

impl Spike {
    fn new() -> Self {
        // A deterministic mix of ASCII, box drawing and wide glyphs — the three
        // things the real grid has to shape.
        let alphabet: Vec<char> =
            "abcdefghijklmnopqrstuvwxyz0123456789 ─│┌┐└┘├┤┬┴┼━┃╭╮╰╯█▓▒░"
                .chars()
                .collect();
        let rows = (0..GRID_ROWS)
            .map(|row| {
                let text: String = (0..GRID_COLUMNS)
                    .map(|column| alphabet[(row * 7 + column * 13) % alphabet.len()])
                    .collect();
                SharedString::from(text)
            })
            .collect();
        Self {
            rows,
            frames: 0,
            started: Instant::now(),
        }
    }
}

impl Render for Spike {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let font_size = px(14.);
        let line_height = px(16.);
        let font = Font {
            family: "monospace".into(),
            features: FontFeatures::default(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            fallbacks: None,
        };

        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        // The advance of one cell. Passing it to `shape_line` is what forces a
        // uniform grid instead of proportional spacing.
        let cell_width = text_system
            .advance(font_id, font_size, 'M')
            .map(|advance| advance.width)
            .unwrap_or(px(8.));

        let paint_started = Instant::now();
        let mut lines = Vec::with_capacity(self.rows.len());
        for (index, text) in self.rows.iter().enumerate() {
            let color = if index % 3 == 0 {
                Hsla::from(rgb(0x9cdcfe))
            } else {
                Hsla::from(rgb(0xd4d4d4))
            };
            let runs = [TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            lines.push(text_system.shape_line(text.clone(), font_size, &runs, Some(cell_width)));
        }
        let shape_micros = paint_started.elapsed().as_micros();

        self.frames += 1;
        let elapsed = self.started.elapsed().as_secs_f64().max(0.001);
        let fps = f64::from(self.frames) / elapsed;

        // Keep repainting: a single static frame would not show whether the
        // grid stays cheap, which is the number worth reporting.
        cx.notify();

        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(rgb(0x0b0e14))
            .text_color(rgb(0xe8ecf4))
            .child(
                // The stock widget side of the risk.
                div()
                    .w(px(272.))
                    .h_full()
                    .bg(rgb(0x12161f))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child("muxtrix-gpui-spike")
                    .child(SharedString::from(format!("cols x rows: {GRID_COLUMNS} x {GRID_ROWS}")))
                    .child(SharedString::from(format!("cell width: {:?}", cell_width)))
                    .child(SharedString::from(format!("shape: {shape_micros} us/frame")))
                    .child(SharedString::from(format!("fps: {fps:.0}")))
                    .child(gpui_component::button::Button::new("primary").label("Primary"))
                    .child(gpui_component::checkbox::Checkbox::new("check").label("Checkbox"))
                    .child(gpui_component::switch::Switch::new("switch").label("Switch")),
            )
            .child(GridElement {
                lines,
                line_height,
                origin_x: px(8.),
                origin_y: px(8.),
            })
    }
}

/// Paints already-shaped rows at a fixed line height, the way the real
/// terminal element will.
struct GridElement {
    lines: Vec<gpui::ShapedLine>,
    line_height: Pixels,
    origin_x: Pixels,
    origin_y: Pixels,
}

impl IntoElement for GridElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl gpui::Element for GridElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.paint_layer(bounds, |window| {
            for (index, line) in self.lines.iter().enumerate() {
                let origin = point(
                    bounds.origin.x + self.origin_x,
                    bounds.origin.y + self.origin_y + self.line_height * index,
                );
                let _ = line.paint(origin, self.line_height, TextAlign::Left, None, window, cx);
            }
        });
    }
}
