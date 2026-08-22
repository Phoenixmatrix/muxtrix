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
    App, Bounds, CursorStyle, Element, ElementId, Entity, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, ScrollWheelEvent, ShapedLine, Style, TextRun, Window, fill, point, px,
    relative, size,
};
use muxtrix_terminal::TerminalMouseButton;

use crate::app::{
    Message, TERMINAL_PADDING, TerminalLink, terminal_link_modifiers, terminal_scrollbar_geometry,
};
use crate::geom::{Point as AppPoint, ScrollDelta, Size};
use crate::runtime::gpui::Root;
use crate::runtime::gpui::color;
use crate::settings::AppSettings;
use crate::terminal::box_painter::GpuiBoxPainter;
use crate::terminal::runs::{
    TerminalRunGeometry, TerminalStyleRun, rgb, terminal_row_style_runs, terminal_run_geometry,
};
use crate::theme::DesignTokens;
use crate::themes::TerminalThemePreset;
use muxtrix_domain::PaneId;
use muxtrix_terminal::GridSnapshot;

/// The scrollbar's lane down the right edge, and the thumb inside it.
const SCROLLBAR_WIDTH: f32 = 12.0;
const SCROLLBAR_INSET: f32 = 3.0;
const SCROLLBAR_THUMB_WIDTH: f32 = 3.0;

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
    pane_id: PaneId,
    /// How the element talks back. Elements are rebuilt every frame and have
    /// no `Context`, so the handle is what turns a click into a message.
    root: Entity<Root>,
    /// Whether the modifiers that turn a hovered link into a clickable one are
    /// currently held, which decides the cursor over a link.
    link_modifiers: bool,
    /// The scrollback position, when there is scrollback to show.
    scrollbar: Option<muxtrix_terminal::ScrollbarSnapshot>,
}

/// What `prepaint` worked out and `paint` consumes.
pub(crate) struct PreparedGrid {
    lines: Vec<PreparedRow>,
    cell_width: Pixels,
    line_height: Pixels,
    /// Set when the bounds imply a different number of rows or columns than
    /// the runtime last reported.
    resized_to: Option<Size>,
    /// The grid's share of the window, which scopes the pointer cursor.
    hitbox: gpui::Hitbox,
}

struct PreparedRow {
    shaped: ShapedLine,
    /// Background spans, already in columns, so paint does no measuring.
    backgrounds: Vec<(usize, usize, Hsla)>,
    /// Runs drawn as geometry rather than glyphs: box drawing, whose arms have
    /// to meet exactly on cell edges, and the full block, which has to fill
    /// its cell completely. A font gives neither reliably across faces.
    geometry: Vec<GeometryRun>,
}

/// A run the element draws itself instead of shaping.
struct GeometryRun {
    start_column: usize,
    columns: usize,
    color: Hsla,
    kind: GeometryKind,
}

enum GeometryKind {
    BoxDrawing(String),
    FullBlock,
}

impl TerminalElement {
    /// Build the element for one pane, or `None` when it has no grid yet.
    ///
    /// The extraction lives here rather than in the view because what the
    /// terminal needs from the application is the terminal's business, and
    /// gathering it at the call site made for a ten-argument constructor.
    pub(crate) fn for_pane(
        app: &crate::app::Muxtrix,
        pane_id: PaneId,
        focused: bool,
        root: Entity<Root>,
    ) -> Option<Self> {
        let runtime = app.terminals.get(&pane_id)?;
        Some(Self {
            snapshot: runtime.snapshot.clone()?,
            settings: app.settings.clone(),
            theme: app.settings.terminal_theme.preset(),
            focused: focused && app.window_focused,
            cursor_phase_visible: app.cursor_phase_visible,
            hovered_link: app.hovered_terminal_link(pane_id),
            reported: runtime.viewport.unwrap_or_default(),
            pane_id,
            root,
            link_modifiers: terminal_link_modifiers(app.keyboard_modifiers),
            scrollbar: runtime
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.scrollbar)
                .filter(|scrollbar| scrollbar.is_scrollable()),
        })
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
        cx: &mut App,
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
        if let Some(size) = resized_to {
            // The PTY is what has to agree with the layout, and it lives in
            // the application. Reporting rather than resizing here is what
            // keeps this settling: the runtime records the new viewport, so
            // the next frame's comparison matches.
            let pane_id = self.pane_id;
            self.root.update(cx, |root, cx| {
                root.dispatch_detached(Message::ResizePane(pane_id, size), cx);
            });
        }

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
            hitbox: window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
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
                for run in &row.geometry {
                    let left = origin.x + prepared.cell_width * run.start_column;
                    match &run.kind {
                        GeometryKind::FullBlock => window.paint_quad(fill(
                            Bounds {
                                origin: point(left, top),
                                size: size(prepared.cell_width * run.columns, prepared.line_height),
                            },
                            run.color,
                        )),
                        GeometryKind::BoxDrawing(text) => {
                            let mut painter =
                                GpuiBoxPainter::new(window, point(left, top), run.color);
                            let metrics = crate::box_drawing::BoxMetrics::new(
                                f32::from(prepared.cell_width),
                                f32::from(prepared.line_height),
                            );
                            for (offset, character) in text.chars().enumerate() {
                                let cell = offset as f32 * f32::from(prepared.cell_width);
                                crate::box_drawing::draw_cell(
                                    &mut painter,
                                    metrics,
                                    character,
                                    cell,
                                );
                            }
                        }
                    }
                }
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

        self.paint_scrollbar(bounds, window);
        self.paint_mouse(bounds, origin, prepared, window);
    }
}

impl TerminalElement {
    /// The scrollback indicator down the right edge.
    ///
    /// Drawn only when there is scrollback, matching the iced pane: a track
    /// that is always empty is noise on a terminal that has never scrolled.
    fn paint_scrollbar(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let Some(scrollbar) = self.scrollbar else {
            return;
        };
        let tokens = DesignTokens::for_appearance(self.settings.appearance);
        let geometry = terminal_scrollbar_geometry(scrollbar, f32::from(bounds.size.height));
        let thumb = Bounds {
            origin: point(
                bounds.origin.x + bounds.size.width - px(SCROLLBAR_WIDTH - SCROLLBAR_INSET),
                bounds.origin.y + px(geometry.track_top + geometry.thumb_top),
            ),
            size: size(px(SCROLLBAR_THUMB_WIDTH), px(geometry.thumb_height)),
        };
        window.paint_quad(gpui::quad(
            thumb,
            px(2.),
            color(tokens.line_strong),
            px(0.),
            gpui::transparent_black(),
            gpui::BorderStyle::default(),
        ));
    }

    /// Register the mouse handlers for this pane.
    ///
    /// Hit-tested against the pane's own bounds rather than relying on
    /// element containment, because these are window-level handlers: a drag
    /// that starts inside the grid has to keep reporting after the pointer
    /// leaves it, which is how selection past the edge works.
    fn paint_mouse(
        &self,
        bounds: Bounds<Pixels>,
        origin: gpui::Point<Pixels>,
        prepared: &PreparedGrid,
        window: &mut Window,
    ) {
        let pane_id = self.pane_id;
        let root = self.root.clone();
        let cell_width = prepared.cell_width;
        let line_height = prepared.line_height;

        // A hovered link only becomes clickable while the modifiers are held,
        // and the cursor is what says so.
        if self.hovered_link.is_some() && self.link_modifiers {
            window.set_cursor_style(CursorStyle::PointingHand, &prepared.hitbox);
        }

        let position_in_grid = move |window_point: gpui::Point<Pixels>| {
            AppPoint::new(
                f32::from(window_point.x - origin.x) + TERMINAL_PADDING / 2.0,
                f32::from(window_point.y - origin.y) + TERMINAL_PADDING / 2.0,
            )
        };

        {
            let root = root.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
                if !phase.bubble() {
                    return;
                }
                let inside = bounds.contains(&event.position);
                let point = position_in_grid(event.position);
                root.update(cx, |root, cx| {
                    if inside {
                        root.dispatch_detached(Message::EnterTerminal(pane_id), cx);
                    }
                    root.dispatch_detached(Message::TerminalPointerMoved(pane_id, point), cx);
                });
            });
        }

        {
            let root = root.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, cx| {
                if !phase.bubble() || !bounds.contains(&event.position) {
                    return;
                }
                let Some(button) = terminal_button(event.button) else {
                    return;
                };
                let point = position_in_grid(event.position);
                root.update(cx, |root, cx| {
                    root.dispatch_detached(Message::Focus(pane_id), cx);
                    root.dispatch_detached(Message::TerminalPointerMoved(pane_id, point), cx);
                    root.dispatch_detached(Message::TerminalMousePressed(pane_id, button), cx);
                });
            });
        }

        {
            let root = root.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
                if !phase.bubble() {
                    return;
                }
                let Some(button) = terminal_button(event.button) else {
                    return;
                };
                root.update(cx, |root, cx| {
                    root.dispatch_detached(Message::TerminalMouseReleased(pane_id, button), cx);
                });
            });
        }

        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _window, cx| {
            if !phase.bubble() || !bounds.contains(&event.position) {
                return;
            }
            let delta = match event.delta {
                gpui::ScrollDelta::Lines(lines) => ScrollDelta::Lines {
                    x: lines.x,
                    y: lines.y,
                },
                gpui::ScrollDelta::Pixels(pixels) => ScrollDelta::Pixels {
                    x: pixels.x.into(),
                    y: pixels.y.into(),
                },
            };
            let _ = (cell_width, line_height);
            root.update(cx, |root, cx| {
                root.dispatch_detached(Message::ScrollTerminal(pane_id, delta), cx);
            });
        });
    }
}

/// The emulator's name for a mouse button, for the ones it reports.
fn terminal_button(button: MouseButton) -> Option<TerminalMouseButton> {
    match button {
        MouseButton::Left => Some(TerminalMouseButton::Left),
        MouseButton::Middle => Some(TerminalMouseButton::Middle),
        MouseButton::Right => Some(TerminalMouseButton::Right),
        _ => None,
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
    let mut backgrounds_geometry: Vec<GeometryRun> = Vec::new();
    let mut column = 0usize;

    for run in row {
        // A background is drawn as a quad rather than handed to the shaper:
        // the shaper would size it to the glyphs, and a terminal cell's
        // background must fill the whole cell even where the glyph is narrow.
        if let Some(background) = run.style.overlay_background.or(run.style.background) {
            backgrounds.push((column, run.columns, color(rgb(background)).into()));
        }
        if run.kind == crate::terminal::runs::TerminalRunKind::BoxDrawing {
            backgrounds_geometry.push(GeometryRun {
                start_column: column,
                columns: run.columns,
                color: color(rgb(run.style.foreground)).into(),
                kind: GeometryKind::BoxDrawing(run.text.clone()),
            });
            // The shaper still needs the columns accounted for, so the run is
            // replaced by spaces rather than dropped.
            text.push_str(&" ".repeat(run.text.chars().count()));
            runs.push(TextRun {
                len: run.text.chars().count(),
                font: font.clone(),
                color: color(rgb(run.style.foreground)).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            });
            column += run.columns;
            continue;
        }
        if terminal_run_geometry(run) == Some(TerminalRunGeometry::FullBlock) {
            backgrounds_geometry.push(GeometryRun {
                start_column: column,
                columns: run.columns,
                color: color(rgb(run.style.foreground)).into(),
                kind: GeometryKind::FullBlock,
            });
            text.push_str(&" ".repeat(run.text.chars().count()));
            runs.push(TextRun {
                len: run.text.chars().count(),
                font: font.clone(),
                color: color(rgb(run.style.foreground)).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            });
            column += run.columns;
            continue;
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
        geometry: backgrounds_geometry,
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
