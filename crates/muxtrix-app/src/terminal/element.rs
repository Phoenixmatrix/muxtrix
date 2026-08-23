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
use muxtrix_terminal::{ImageLayer, TerminalMouseButton};

use crate::app::{
    Message, TERMINAL_PADDING, TerminalLink, terminal_link_modifiers, terminal_scrollbar_geometry,
};
use crate::geom::{Point as AppPoint, ScrollDelta, Size};
use crate::runtime::gpui::Root;
use crate::runtime::gpui::color;
use crate::settings::AppSettings;
use crate::terminal::box_painter::GpuiBoxPainter;
use crate::terminal::runs::{
    TerminalRunGeometry, TerminalStyleRun, TerminalUnderlineDecoration, rgb,
    terminal_row_style_runs, terminal_run_geometry, terminal_underline_decoration,
};
use crate::theme::DesignTokens;
use crate::themes::TerminalThemePreset;
use muxtrix_domain::PaneId;
use muxtrix_terminal::GridSnapshot;

/// The scrollbar's grab lane down the right edge, and the thumb inside it.
///
/// The lane is wider than the thumb on purpose: a 3 px target is not something
/// a pointer can reasonably hit, and the iced pane reserved the same strip.
const SCROLLBAR_LANE: f32 = 24.0;
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
    /// The grid the PTY currently has, when the runtime knows its viewport.
    ///
    /// `None` means the runtime has no viewport yet — a replacement terminal
    /// in an unchanged layout — and must be told its size even though the grid
    /// would come out the same.
    reported_grid: Option<muxtrix_platform::PtySize>,
    pane_id: PaneId,
    /// How the element talks back. Elements are rebuilt every frame and have
    /// no `Context`, so the handle is what turns a click into a message.
    root: Entity<Root>,
    /// Whether the modifiers that turn a hovered link into a clickable one are
    /// currently held, which decides the cursor over a link.
    link_modifiers: bool,
    /// The scrollback position, when there is scrollback to show.
    scrollbar: Option<muxtrix_terminal::ScrollbarSnapshot>,
    /// Inline images, already decoded, keyed the way the emulator keys them.
    images: std::collections::BTreeMap<u64, std::sync::Arc<gpui::RenderImage>>,
    /// Whether something is open above the grid — a menu, a dialog, the
    /// palette. These handlers are registered on the window rather than on the
    /// element, so nothing else can swallow a press for them; without this a
    /// click that dismisses a menu also starts a selection in the terminal
    /// underneath, and the drag it leaves behind captures the pointer motion
    /// a mouse-reporting program should have received.
    obscured: bool,
}

/// What `prepaint` worked out and `paint` consumes.
pub(crate) struct PreparedGrid {
    lines: Vec<PreparedRow>,
    cell_width: Pixels,
    line_height: Pixels,
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
        images: std::collections::BTreeMap<u64, std::sync::Arc<gpui::RenderImage>>,
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
            reported_grid: runtime.viewport.map(|_| runtime.size),
            pane_id,
            root,
            link_modifiers: terminal_link_modifiers(app.keyboard_modifiers),
            scrollbar: runtime
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.scrollbar)
                .filter(|scrollbar| scrollbar.is_scrollable()),
            images,
            obscured: app.pane_menu.is_some()
                || app.palette.visible
                || app.workspace_create_visible
                || app.rename_prompt.is_some()
                || app.worktree_prompt.is_some()
                || app.session_picker.is_some()
                || app.close_workspace_prompt.is_some()
                || app.default_agent_prompt,
        })
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
        // Absolutely positioned and pinned to its parent's box. A grid that
        // took part in flex sizing would feed its own measurements back into
        // the layout that produced them, and the PTY would oscillate: each
        // resize changes the content, which changes the size, which resizes
        // the PTY again.
        let style = Style {
            position: gpui::Position::Absolute,
            inset: gpui::Edges {
                top: px(0.).into(),
                right: px(0.).into(),
                bottom: px(0.).into(),
                left: px(0.).into(),
            },
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
        // Shaping uses the advance of the face GPUI actually resolved, because
        // that is what the glyphs will be drawn with; measuring it any other
        // way spreads or crowds the row.
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
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
        .filter(|size| *size != self.reported)
        // Resizing a PTY reflows its whole grid, so it is only worth doing
        // when the grid actually changes. Dragging a window edge crosses many
        // pixel widths per column; without this the reflow runs every frame of
        // the drag and starves everything else on the main thread.
        .filter(|size| {
            self.reported_grid
                .is_none_or(|grid| crate::app::pty_size_for_pane(*size, &self.settings) != grid)
        });
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

        let bold_weight = self.settings.terminal_font_weight.bold_variant();
        let mut lines = Vec::with_capacity(rows.len());
        for row in &rows {
            lines.push(prepare_row(
                row,
                &font,
                bold_weight,
                font_size,
                cell_width,
                window,
            ));
        }
        let _ = usable;

        PreparedGrid {
            lines,
            cell_width,
            line_height,
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
            self.paint_images(origin, prepared, ImageLayer::BelowBackground, window);
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
            self.paint_images(origin, prepared, ImageLayer::BelowText, window);
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
            self.paint_images(origin, prepared, ImageLayer::AboveText, window);
        });

        self.paint_scrollbar(bounds, window);
        #[cfg(feature = "e2e")]
        {
            let obscured = self.obscured;
            self.root.update(cx, |root, _| {
                root.app.e2e_paint_trace.0 += 1;
                if !obscured {
                    root.app.e2e_paint_trace.1 += 1;
                }
            });
        }
        self.paint_mouse(bounds, origin, prepared, window);
    }
}

impl TerminalElement {
    /// Inline images on one of the three planes the emulator places them on.
    ///
    /// A placement can be a crop of a larger image, so the whole image is
    /// painted into the rectangle it would occupy and clipped to the part the
    /// placement actually asked for — the same source-crop-by-clip the iced
    /// canvas did.
    fn paint_images(
        &self,
        origin: gpui::Point<Pixels>,
        prepared: &PreparedGrid,
        layer: ImageLayer,
        window: &mut Window,
    ) {
        for placement in self
            .snapshot
            .images
            .iter()
            .filter(|placement| placement.layer == layer)
        {
            let Some(image) = self.images.get(&placement.image.generation) else {
                continue;
            };
            let cell = (
                f32::from(prepared.cell_width),
                f32::from(prepared.line_height),
            );
            let destination = crate::terminal_image::scaled_destination(
                (placement.column, placement.row),
                (placement.width, placement.height),
                (placement.x_offset, placement.y_offset),
                cell,
            );
            let Some(geometry) = crate::terminal_image::placement_geometry(
                destination,
                placement.source,
                placement.image.width,
                placement.image.height,
            ) else {
                continue;
            };
            let clip = to_bounds(origin, geometry.destination);
            window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
                let _ = window.paint_image(
                    to_bounds(origin, geometry.full_image),
                    clip,
                    gpui::Corners::default(),
                    std::sync::Arc::clone(image),
                    0,
                    false,
                );
            });
        }
    }

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
        if self.obscured {
            return;
        }
        let pane_id = self.pane_id;
        let root = self.root.clone();
        let cell_width = prepared.cell_width;
        let line_height = prepared.line_height;

        // A hovered link only becomes clickable while the modifiers are held,
        // and the cursor is what says so.
        if self.hovered_link.is_some() && self.link_modifiers {
            window.set_cursor_style(CursorStyle::PointingHand, &prepared.hitbox);
        }

        // The scrollbar's lane down the right edge. A press here grabs the
        // thumb rather than starting a selection, so it is tested first.
        let scrollbar_lane = Bounds {
            origin: point(
                bounds.origin.x + bounds.size.width - px(SCROLLBAR_LANE),
                bounds.origin.y,
            ),
            size: size(px(SCROLLBAR_LANE), bounds.size.height),
        };
        let position_in_pane = move |window_point: gpui::Point<Pixels>| {
            AppPoint::new(
                f32::from(window_point.x - bounds.origin.x),
                f32::from(window_point.y - bounds.origin.y),
            )
        };

        let position_in_grid = move |window_point: gpui::Point<Pixels>| {
            AppPoint::new(
                f32::from(window_point.x - origin.x) + TERMINAL_PADDING / 2.0,
                f32::from(window_point.y - origin.y) + TERMINAL_PADDING / 2.0,
            )
        };

        {
            let root = root.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
                #[cfg(feature = "e2e")]
                root.update(cx, |root, _| {
                    root.app.e2e_phase_trace.0 += 1;
                    if phase.bubble() {
                        root.app.e2e_phase_trace.1 += 1;
                    }
                });
                if !phase.bubble() {
                    return;
                }
                let inside = bounds.contains(&event.position);
                let grid_point = position_in_grid(event.position);
                let pane_point = position_in_pane(event.position);
                root.update(cx, |root, cx| {
                    // Whether this motion belongs to the program or to a
                    // selection depends on a mode the PTY may have set since
                    // the last frame, so read what is pending before deciding.
                    root.drain_terminals(cx);
                    if inside {
                        root.dispatch_detached(Message::EnterTerminal(pane_id), cx);
                    } else if root.app.hovered_terminal == Some(pane_id) {
                        root.dispatch_detached(Message::LeaveTerminal(pane_id), cx);
                    }
                    // A drag that began on the thumb keeps steering it even
                    // once the pointer leaves the lane, which is what makes
                    // dragging usable.
                    root.dispatch_detached(
                        Message::TerminalScrollbarMoved(pane_id, pane_point),
                        cx,
                    );
                    root.dispatch_detached(Message::TerminalPointerMoved(pane_id, grid_point), cx);
                });
            });
        }

        {
            let root = root.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, _window, cx| {
                if !phase.bubble() || !bounds.contains(&event.position) {
                    return;
                }
                if scrollbar_lane.contains(&event.position) {
                    let point = position_in_pane(event.position);
                    root.update(cx, |root, cx| {
                        // Position first: grabbing the thumb is defined
                        // relative to where in the lane the press landed.
                        root.dispatch_detached(Message::TerminalScrollbarMoved(pane_id, point), cx);
                        root.dispatch_detached(Message::BeginTerminalScroll(pane_id), cx);
                    });
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
                    root.dispatch_detached(Message::EndPointerInteraction, cx);
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
    bold_weight: crate::settings::FontWeight,
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
        // Bold steps up from the configured weight rather than to one fixed
        // face, so a family set to Medium still has somewhere to go.
        face.weight = if run.style.bold {
            gpui::FontWeight(f32::from(bold_weight.numeric()))
        } else {
            font.weight
        };
        face.style = if run.style.italic {
            gpui::FontStyle::Italic
        } else {
            gpui::FontStyle::Normal
        };
        // Links own their underline: dotted until the modifiers make them
        // clickable, then solid — a printed URL the program itself underlined
        // must not look live until it is.
        let foreground: gpui::Hsla = color(rgb(run.style.foreground)).into();
        let underline = match terminal_underline_decoration(run.style) {
            TerminalUnderlineDecoration::None => None,
            TerminalUnderlineDecoration::Dotted => Some(gpui::UnderlineStyle {
                thickness: px(1.),
                color: Some(foreground),
                wavy: true,
            }),
            TerminalUnderlineDecoration::Solid => Some(gpui::UnderlineStyle {
                thickness: px(1.),
                color: Some(foreground),
                wavy: false,
            }),
        };
        let strikethrough = run.style.strikethrough.then_some(gpui::StrikethroughStyle {
            thickness: px(1.),
            color: Some(foreground),
        });
        runs.push(TextRun {
            len: run.text.len(),
            font: face,
            color: foreground,
            background_color: None,
            underline,
            strikethrough,
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
    // Resolve the generic name through fontconfig rather than handing GPUI
    // the word "monospace": GPUI can answer that with a proportional fallback,
    // and a proportional face in a fixed grid spreads every row — an 'M' is
    // far wider than a digit, and the cell is sized from one of them.
    let family = settings.terminal_font.family_name().map_or_else(
        || {
            crate::metrics::system_monospace_family()
                .unwrap_or("monospace")
                .to_owned()
        },
        ToOwned::to_owned,
    );
    gpui::Font {
        family: family.into(),
        features: gpui::FontFeatures::default(),
        weight: gpui::FontWeight(f32::from(settings.terminal_font_weight.numeric())),
        style: gpui::FontStyle::Normal,
        fallbacks: None,
    }
}

/// A grid-local rectangle in window coordinates.
fn to_bounds(origin: gpui::Point<Pixels>, rect: crate::geom::Rect) -> Bounds<Pixels> {
    Bounds {
        origin: point(origin.x + px(rect.x), origin.y + px(rect.y)),
        size: size(px(rect.width), px(rect.height)),
    }
}
