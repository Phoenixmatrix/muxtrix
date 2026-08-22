//! The terminal grid as a GPUI element.
//!
//! This is where the port earns or loses its performance. A grid is up to
//! twelve thousand cells, redrawn on every keystroke, so the work is arranged
//! to be per-*run* rather than per-cell: [`terminal_row_style_runs`] has
//! already collapsed each row into maximal spans of one style, and each span
//! becomes one shaped line and at most one background quad.
//!
//! Passing the cell advance to `shape_line` is what keeps the result a grid.
//! Without it the shaper applies the font's natural advances and proportional
//! glyphs drift out of their columns.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, Hsla, InspectorElementId, IntoElement,
    LayoutId, Pixels, ShapedLine, Style, TextRun, Window, fill, point, px, relative, size,
};

use crate::app::{TERMINAL_PADDING, TerminalLink};
use crate::geom::Size;
use crate::runtime::gpui::color;
use crate::settings::AppSettings;
use crate::terminal::runs::{TerminalStyleRun, rgb, terminal_row_style_runs};
use crate::themes::TerminalThemePreset;
use muxtrix_terminal::GridSnapshot;

/// What the element needs to draw one pane, gathered before layout.
pub(crate) struct TerminalElement {
    snapshot: GridSnapshot,
    settings: AppSettings,
    theme: TerminalThemePreset,
    focused: bool,
    cursor_phase_visible: bool,
    hovered_link: Option<TerminalLink>,
    /// The grid size the runtime currently believes this pane has. When layout
    /// disagrees, the element reports the difference rather than resizing
    /// itself — the PTY is the thing that has to agree, and it lives in the
    /// application.
    reported: Size,
}

/// What `prepaint` worked out and `paint` consumes.
pub(crate) struct PreparedGrid {
    lines: Vec<PreparedRow>,
    cell_width: Pixels,
    line_height: Pixels,
    /// Set when the bounds imply a different number of rows or columns than
    /// the runtime last reported.
    resized_to: Option<Size>,
}

struct PreparedRow {
    shaped: ShapedLine,
    /// Background spans, already in columns, so paint does no measuring.
    backgrounds: Vec<(usize, usize, Hsla)>,
}

impl TerminalElement {
    pub(crate) fn new(
        snapshot: GridSnapshot,
        settings: AppSettings,
        theme: TerminalThemePreset,
        focused: bool,
        cursor_phase_visible: bool,
        hovered_link: Option<TerminalLink>,
        reported: Size,
    ) -> Self {
        Self {
            snapshot,
            settings,
            theme,
            focused,
            cursor_phase_visible,
            hovered_link,
            reported,
        }
    }

    /// The size in pixels the grid occupies, which the caller compares against
    /// the pane to decide whether the PTY needs resizing.
    pub(crate) fn resized_to(prepared: &PreparedGrid) -> Option<Size> {
        prepared.resized_to
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = PreparedGrid;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            size: size(relative(1.).into(), relative(1.).into()),
            ..Default::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        let font = crate::terminal::element::terminal_font(&self.settings);
        let font_size = px(self.settings.terminal_font_pixels());
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        // Measured through GPUI rather than from font metrics, so the grid and
        // the shaper can never disagree about a column's width.
        let cell_width = text_system
            .advance(font_id, font_size, 'M')
            .map_or_else(|_| px(self.settings.terminal_cell_width()), |a| a.width);
        let line_height = px(self.settings.terminal_cell_height());

        let usable = size(
            (bounds.size.width - px(TERMINAL_PADDING)).max(px(0.)),
            (bounds.size.height - px(TERMINAL_PADDING)).max(px(0.)),
        );
        let resized_to = Some(Size::new(
            bounds.size.width.into(),
            bounds.size.height.into(),
        ))
        .filter(|size| *size != self.reported);

        let rows = terminal_row_style_runs(
            &self.snapshot,
            self.focused,
            self.cursor_phase_visible,
            self.hovered_link.as_ref(),
            self.theme,
        );

        let mut lines = Vec::with_capacity(rows.len());
        for row in &rows {
            lines.push(prepare_row(row, &font, font_size, cell_width, window));
        }
        let _ = usable;

        PreparedGrid {
            lines,
            cell_width,
            line_height,
            resized_to,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepared: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let origin = point(
            bounds.origin.x + px(TERMINAL_PADDING / 2.0),
            bounds.origin.y + px(TERMINAL_PADDING / 2.0),
        );
        // Clipped to the pane: a grid whose last row half-fits should be cut,
        // not spill over the pane's chrome.
        window.paint_layer(bounds, |window| {
            // Backgrounds first, as one quad per run, then all the text. Two
            // passes rather than interleaving so the shaped lines paint in one
            // uninterrupted sequence.
            for (index, row) in prepared.lines.iter().enumerate() {
                let top = origin.y + prepared.line_height * index;
                for (start, columns, color) in &row.backgrounds {
                    let left = origin.x + prepared.cell_width * *start;
                    let width = prepared.cell_width * *columns;
                    window.paint_quad(fill(
                        Bounds {
                            origin: point(left, top),
                            size: size(width, prepared.line_height),
                        },
                        *color,
                    ));
                }
            }
            for (index, row) in prepared.lines.iter().enumerate() {
                let top = origin.y + prepared.line_height * index;
                let _ = row.shaped.paint(
                    point(origin.x, top),
                    prepared.line_height,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        });
    }
}

/// Shape one row and record where its background runs sit.
fn prepare_row(
    row: &[TerminalStyleRun],
    font: &gpui::Font,
    font_size: Pixels,
    cell_width: Pixels,
    window: &mut Window,
) -> PreparedRow {
    let mut text = String::new();
    let mut runs = Vec::with_capacity(row.len());
    let mut backgrounds = Vec::new();
    let mut column = 0usize;

    for run in row {
        // A background is drawn as a quad rather than handed to the shaper:
        // the shaper would size it to the glyphs, and a terminal cell's
        // background must fill the whole cell even where the glyph is narrow.
        if let Some(background) = run.style.overlay_background.or(run.style.background) {
            backgrounds.push((column, run.columns, color(rgb(background)).into()));
        }
        let mut face = font.clone();
        face.weight = if run.style.bold {
            gpui::FontWeight::BOLD
        } else {
            font.weight
        };
        face.style = if run.style.italic {
            gpui::FontStyle::Italic
        } else {
            gpui::FontStyle::Normal
        };
        runs.push(TextRun {
            len: run.text.len(),
            font: face,
            color: color(rgb(run.style.foreground)).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        });
        text.push_str(&run.text);
        column += run.columns;
    }

    PreparedRow {
        shaped: window
            .text_system()
            .shape_line(text.into(), font_size, &runs, Some(cell_width)),
        backgrounds,
    }
}

/// The configured terminal face, in GPUI's terms.
///
/// No `&'static str` interning: GPUI takes an owned family name, so the leak
/// the iced runtime needed does not carry over. An unset family means "the
/// system monospace", which GPUI resolves itself from the generic name.
pub(crate) fn terminal_font(settings: &AppSettings) -> gpui::Font {
    let family = settings
        .terminal_font
        .family_name()
        .map_or_else(|| "monospace".to_owned(), ToOwned::to_owned);
    gpui::Font {
        family: family.into(),
        features: gpui::FontFeatures::default(),
        weight: gpui::FontWeight(f32::from(crate::settings::weight_numeric(
            settings.terminal_font_weight.iced(),
        ))),
        style: gpui::FontStyle::Normal,
        fallbacks: None,
    }
}
