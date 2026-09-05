//! A thread-confined Ghostty VT engine exposed through a Send-safe actor.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Cursor, Read};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use libghostty_vt::alloc::{Allocator, Bytes};
use libghostty_vt::fmt::Format;
use libghostty_vt::key::Mods as GhosttyMods;
use libghostty_vt::kitty::graphics::{self, ImageFormat, PlacementIterator};
use libghostty_vt::mouse::{
    Action as GhosttyMouseAction, Button as GhosttyMouseButton, Encoder as MouseEncoder,
    EncoderSize as MouseEncoderSize, Event as GhosttyMouseEvent, Position as GhosttyMousePosition,
};
use libghostty_vt::render::{CellIterator, Dirty, RowIterator};
use libghostty_vt::screen::CellWide;
use libghostty_vt::screen::TrackedGridRef;
use libghostty_vt::selection::{FormatOptions, Selection};
use libghostty_vt::style::{Palette, RgbColor, Underline};
use libghostty_vt::terminal::{Mode, Point, PointCoordinate, ScrollViewport};
use libghostty_vt::{RenderState, Terminal, TerminalOptions, paste};
use muxtrix_platform::{LaunchPlan, PtyOutput, PtySession, PtySize};
use thiserror::Error;

/// Ghostty's full terminal host resets a stuck synchronized-output frame after
/// one second. libghostty-vt deliberately leaves that host policy to embedders.
const SYNC_OUTPUT_RESET_AFTER: Duration = Duration::from_secs(1);
const KITTY_IMAGE_STORAGE_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Default)]
struct PngDecoder {
    buffer: Vec<u8>,
}

impl graphics::DecodePng for PngDecoder {
    fn decode_png<'alloc>(
        &mut self,
        alloc: &'alloc Allocator<'_>,
        data: &[u8],
    ) -> Option<graphics::DecodedImage<'alloc>> {
        let mut decoder = png::Decoder::new(Cursor::new(data));
        decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        self.buffer.resize(reader.output_buffer_size(), 0);
        let info = reader.next_frame(&mut self.buffer).ok()?;
        let pixel_count = usize::try_from(info.width)
            .ok()?
            .checked_mul(usize::try_from(info.height).ok()?)?;
        let mut rgba = Bytes::new_with_alloc(alloc, pixel_count.checked_mul(4)?).ok()?;
        let source = &self.buffer[..info.buffer_size()];

        match info.color_type {
            png::ColorType::Rgba => rgba.copy_from_slice(source),
            png::ColorType::Rgb => {
                for (source, target) in source
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .zip(rgba.as_chunks_mut::<4>().0.iter_mut())
                {
                    target.copy_from_slice(&[source[0], source[1], source[2], u8::MAX]);
                }
            }
            png::ColorType::GrayscaleAlpha => {
                for (source, target) in source
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .zip(rgba.as_chunks_mut::<4>().0.iter_mut())
                {
                    target.copy_from_slice(&[source[0], source[0], source[0], source[1]]);
                }
            }
            png::ColorType::Grayscale => {
                for (source, target) in source.iter().zip(rgba.as_chunks_mut::<4>().0.iter_mut()) {
                    target.copy_from_slice(&[*source, *source, *source, u8::MAX]);
                }
            }
            png::ColorType::Indexed => return None,
        }

        Some(graphics::DecodedImage {
            width: info.width,
            height: info.height,
            data: rgba,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridSnapshot {
    pub rows: Vec<Arc<str>>,
    pub cells: Vec<Arc<[CellSnapshot]>>,
    pub images: Vec<ImagePlacementSnapshot>,
    pub default_foreground: Rgb,
    pub default_background: Rgb,
    pub cursor_color: Option<Rgb>,
    pub cursor: Option<CursorSnapshot>,
    pub title: Option<String>,
    /// The working directory the shell reported via OSC 7/9/1337, verbatim —
    /// typically a file:// URI for OSC 7. Works across every backend,
    /// including Windows-hosted WSL panes where /proc is unreachable.
    pub pwd: Option<String>,
    pub scrollbar: ScrollbarSnapshot,
    /// Whether the running program answers wheel gestures itself — a
    /// mouse-reporting or alternate-screen application. Such a program may
    /// repaint its own content in place instead of moving this viewport.
    pub application_scroll: bool,
    /// Whether the running program enabled terminal mouse tracking. Pointer
    /// buttons and motion belong to the program while this is true; terminal
    /// hosts conventionally reserve Shift for local text selection.
    pub mouse_reporting: bool,
    /// The selected columns of each viewport row, as the emulator resolved
    /// them for this frame. The selection is terminal state anchored to
    /// tracked references, so these ranges already account for whatever the
    /// terminal scrolled between frames.
    pub selection: Vec<Option<SelectedColumns>>,
}

/// Decoded pixels retained across frames for every visible Kitty image.
#[derive(Debug, Clone)]
pub struct ImageSnapshot {
    pub id: u32,
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl PartialEq for ImageSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.generation == other.generation
            && self.width == other.width
            && self.height == other.height
    }
}

impl Eq for ImageSnapshot {}

/// Source pixels selected from a Kitty image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSourceRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Ghostty's three image planes around terminal cell backgrounds and text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageLayer {
    BelowBackground,
    BelowText,
    AboveText,
}

/// One viewport-relative Kitty image placement ready for the GPU host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlacementSnapshot {
    pub image: Arc<ImageSnapshot>,
    pub source: ImageSourceRect,
    pub column: i32,
    pub row: i32,
    pub width: u32,
    pub height: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub z: i32,
    pub layer: ImageLayer,
}

/// An inclusive run of selected columns within one viewport row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedColumns {
    pub start: usize,
    pub end: usize,
}

impl SelectedColumns {
    #[must_use]
    pub const fn contains(self, column: usize) -> bool {
        column >= self.start && column <= self.end
    }
}

/// Terminal color defaults supplied by the emulator theme.
///
/// Ghostty keeps these defaults distinct from colors selected by terminal
/// programs. Applying a new theme therefore preserves direct RGB cell colors
/// and any active OSC color overrides while changing ordinary ANSI colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalTheme {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub ansi: [Rgb; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScrollbarSnapshot {
    pub total: u64,
    pub offset: u64,
    pub visible: u64,
}

impl ScrollbarSnapshot {
    #[must_use]
    pub const fn is_scrollable(self) -> bool {
        self.total > self.visible
    }
}

impl GridSnapshot {
    #[must_use]
    pub fn text(&self) -> String {
        self.rows
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl From<RgbColor> for Rgb {
    fn from(color: RgbColor) -> Self {
        Self {
            red: color.r,
            green: color.g,
            blue: color.b,
        }
    }
}

impl From<Rgb> for RgbColor {
    fn from(color: Rgb) -> Self {
        Self {
            r: color.red,
            g: color.green,
            b: color.blue,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSnapshot {
    pub text: String,
    /// Number of terminal grid columns owned by this render cell.
    /// Wide-character tails own zero columns because their leading cell owns both.
    pub columns: u8,
    pub foreground: Rgb,
    pub background: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// Explicit OSC 8 hyperlink destination carried by this cell.
    pub hyperlink: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorSnapshot {
    pub column: u16,
    pub row: u16,
    pub visible: bool,
    pub blinking: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalMouseAction {
    Press,
    Release,
    Motion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalMouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalMouseEvent {
    pub action: TerminalMouseAction,
    pub button: Option<TerminalMouseButton>,
    /// Pointer position in terminal-content pixels, excluding host padding.
    pub x: f32,
    pub y: f32,
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

struct TerminalCore {
    terminal: Terminal<'static, 'static>,
    mouse_encoder: MouseEncoder<'static>,
    /// The host's exact cell metrics in pixels. The encoder only takes whole
    /// pixels per cell, so pointer positions are rescaled from these into its
    /// integer grid before encoding; otherwise every row of fractional cell
    /// height drifts the reported cell a little further from the pointer.
    mouse_cell_width: f32,
    mouse_cell_height: f32,
    /// The whole-pixel cell the encoder was configured with, and its screen
    /// in those cells.
    mouse_encoder_cell_width: u32,
    mouse_encoder_cell_height: u32,
    mouse_encoder_screen_width: u32,
    mouse_encoder_screen_height: u32,
    render_state: RenderState<'static>,
    row_iterator: RowIterator<'static>,
    cell_iterator: CellIterator<'static>,
    image_iterator: PlacementIterator<'static>,
    image_cache: HashMap<u64, Arc<ImageSnapshot>>,
    pty_responses: Rc<RefCell<Vec<Vec<u8>>>>,
    last_snapshot: Option<GridSnapshot>,
    /// Start of a DEC mode 2026 frame. While this is set, `last_snapshot` is
    /// the last complete frame and must remain the presented frame.
    sync_output_started_at: Option<Instant>,
    /// Where a drag began, as a tracked reference rather than a coordinate.
    /// The emulator moves it with the row it points at, so an anchor survives
    /// everything that shifts rows underneath it — scrollback, a pager
    /// scrolling the alternate screen, reflow on resize.
    selection_anchor: Option<TrackedGridRef>,
}

impl TerminalCore {
    fn new(options: TerminalOptions) -> Result<Self, TerminalActorError> {
        graphics::set_png_decoder(Some(Box::new(PngDecoder::default()))).map_err(ghostty_error)?;
        let mut terminal = Terminal::new(options).map_err(ghostty_error)?;
        terminal
            .set_kitty_image_storage_limit(KITTY_IMAGE_STORAGE_LIMIT)
            .map_err(ghostty_error)?;
        let mut mouse_encoder = MouseEncoder::new().map_err(ghostty_error)?;
        mouse_encoder
            .set_options_from_terminal(&terminal)
            .set_size(MouseEncoderSize {
                screen_width: u32::from(options.cols),
                screen_height: u32::from(options.rows),
                cell_width: 1,
                cell_height: 1,
                padding_top: 0,
                padding_bottom: 0,
                padding_right: 0,
                padding_left: 0,
            })
            .set_track_last_cell(true);
        let pty_responses = Rc::new(RefCell::new(Vec::new()));
        let callback_responses = Rc::clone(&pty_responses);
        terminal
            .on_pty_write(move |_terminal, data| {
                callback_responses.borrow_mut().push(data.to_vec());
            })
            .map_err(ghostty_error)?;

        Ok(Self {
            terminal,
            mouse_encoder,
            mouse_cell_width: 1.0,
            mouse_cell_height: 1.0,
            mouse_encoder_cell_width: 1,
            mouse_encoder_cell_height: 1,
            mouse_encoder_screen_width: u32::from(options.cols),
            mouse_encoder_screen_height: u32::from(options.rows),
            render_state: RenderState::new().map_err(ghostty_error)?,
            row_iterator: RowIterator::new().map_err(ghostty_error)?,
            cell_iterator: CellIterator::new().map_err(ghostty_error)?,
            image_iterator: PlacementIterator::new().map_err(ghostty_error)?,
            image_cache: HashMap::new(),
            pty_responses,
            last_snapshot: None,
            sync_output_started_at: None,
            selection_anchor: None,
        })
    }

    /// Begins a selection at a viewport cell, anchoring it to a tracked
    /// reference. Nothing is selected until the drag extends it.
    fn selection_start(&mut self, column: u16, row: u16) -> Result<(), TerminalActorError> {
        self.selection_anchor = Some(
            self.terminal
                .track_grid_ref(viewport_point(column, row))
                .map_err(ghostty_error)?,
        );
        self.terminal.set_selection(None).map_err(ghostty_error)?;
        Ok(())
    }

    /// Extends the active selection to a viewport cell and installs it. The
    /// emulator takes ownership and tracks both endpoints from here on.
    fn selection_extend(&mut self, column: u16, row: u16) -> Result<(), TerminalActorError> {
        let Some(anchor) = self.selection_anchor.as_ref() else {
            return Ok(());
        };
        let Some(start) = anchor.snapshot(&self.terminal).map_err(ghostty_error)? else {
            // The anchored row has left the terminal entirely.
            return self.selection_clear();
        };
        let end = self
            .terminal
            .grid_ref(viewport_point(column, row))
            .map_err(ghostty_error)?;
        let selection = Selection::new(start, end, false);
        self.terminal
            .set_selection(Some(&selection))
            .map_err(ghostty_error)?;
        Ok(())
    }

    fn selection_clear(&mut self) -> Result<(), TerminalActorError> {
        self.selection_anchor = None;
        self.terminal.set_selection(None).map_err(ghostty_error)?;
        Ok(())
    }

    /// The selected text, formatted the way Ghostty formats a copy: plain,
    /// unwrapped, and trimmed.
    fn selection_text(&self) -> Result<Option<String>, TerminalActorError> {
        self.active_selection_text()
    }

    fn active_selection_text(&self) -> Result<Option<String>, TerminalActorError> {
        let options = FormatOptions::new()
            .with_emit_format(Format::Plain)
            .with_unwrap(true)
            .with_trim(true);
        let bytes = self
            .terminal
            .format_selection_alloc(None, options)
            .map_err(ghostty_error)?;
        Ok(bytes.map(|bytes| String::from_utf8_lossy(bytes.as_ref()).into_owned()))
    }

    fn feed(&mut self, bytes: &[u8]) {
        let was_synchronized = self.synchronized_output_active();
        let previous_mouse_protocol = self.mouse_protocol_state();
        self.terminal.vt_write(bytes);
        if self.mouse_protocol_state() != previous_mouse_protocol {
            self.mouse_encoder.set_options_from_terminal(&self.terminal);
        }
        let is_synchronized = self.synchronized_output_active();
        self.sync_output_started_at = match (was_synchronized, is_synchronized) {
            (false, true) => Some(Instant::now()),
            (_, false) => None,
            (true, true) => self.sync_output_started_at,
        };
    }

    fn synchronized_output_active(&self) -> bool {
        self.terminal.mode(Mode::SYNC_OUTPUT).unwrap_or(false)
    }

    fn synchronized_output_timeout_remaining(&self) -> Option<Duration> {
        self.synchronized_output_active().then(|| {
            SYNC_OUTPUT_RESET_AFTER.saturating_sub(
                self.sync_output_started_at
                    .map_or(Duration::ZERO, |started| started.elapsed()),
            )
        })
    }

    /// Releases a mode-2026 frame whose producer disappeared before sending
    /// the closing sequence. Returns true when a new frame should be shown.
    fn expire_synchronized_output(&mut self) -> Result<bool, TerminalActorError> {
        let Some(remaining) = self.synchronized_output_timeout_remaining() else {
            return Ok(false);
        };
        if !remaining.is_zero() {
            return Ok(false);
        }
        self.finish_synchronized_output()?;
        Ok(true)
    }

    fn finish_synchronized_output(&mut self) -> Result<(), TerminalActorError> {
        self.terminal
            .set_mode(Mode::SYNC_OUTPUT, false)
            .map_err(ghostty_error)?;
        self.sync_output_started_at = None;
        Ok(())
    }

    /// Translates a wheel gesture into what the running application actually
    /// expects, the way real terminals do. A mouse-reporting application gets
    /// wheel button events at the pointer cell; an alternate-screen
    /// application without mouse reporting gets arrow keys when DEC mode 1007
    /// enables xterm alternate scroll; otherwise the caller scrolls the
    /// viewport.
    ///
    /// `lines` is negative to scroll toward history (wheel up); `cell` is the
    /// 1-based pointer column/row when known.
    /// Whether the running program consumes wheel gestures itself, rather than
    /// letting them scroll the terminal's viewport. Such a program may answer
    /// by repainting its own content in place, which requires selection
    /// reconciliation after the next complete frame.
    fn application_owns_wheel(&self) -> bool {
        let mode = |mode: Mode| self.terminal.mode(mode).unwrap_or(false);
        let mouse_reporting =
            mode(Mode::NORMAL_MOUSE) || mode(Mode::BUTTON_MOUSE) || mode(Mode::ANY_MOUSE);
        let alternate_screen =
            mode(Mode::ALT_SCREEN) || mode(Mode::ALT_SCREEN_SAVE) || mode(Mode::ALT_SCREEN_LEGACY);
        mouse_reporting || (alternate_screen && mode(Mode::ALT_SCROLL))
    }

    fn mouse_reporting(&self) -> bool {
        let mode = |mode: Mode| self.terminal.mode(mode).unwrap_or(false);
        mode(Mode::X10_MOUSE)
            || mode(Mode::NORMAL_MOUSE)
            || mode(Mode::BUTTON_MOUSE)
            || mode(Mode::ANY_MOUSE)
    }

    fn mouse_protocol_state(&self) -> [bool; 8] {
        let mode = |mode: Mode| self.terminal.mode(mode).unwrap_or(false);
        [
            mode(Mode::X10_MOUSE),
            mode(Mode::NORMAL_MOUSE),
            mode(Mode::BUTTON_MOUSE),
            mode(Mode::ANY_MOUSE),
            mode(Mode::UTF8_MOUSE),
            mode(Mode::SGR_MOUSE),
            mode(Mode::URXVT_MOUSE),
            mode(Mode::SGR_PIXELS_MOUSE),
        ]
    }

    /// Configures pointer-to-cell mapping from the grid and the host's exact
    /// cell metrics. The encoder's screen is exactly `cols × rows` of its
    /// whole-pixel cells, so its clamping agrees with the grid rather than
    /// with a content area that may end in a partial row.
    fn set_mouse_geometry(&mut self, cols: u16, rows: u16, cell_width: f32, cell_height: f32) {
        let cell_width = if cell_width.is_finite() && cell_width > 0.0 {
            cell_width
        } else {
            1.0
        };
        let cell_height = if cell_height.is_finite() && cell_height > 0.0 {
            cell_height
        } else {
            1.0
        };
        let encoder_cell_width = (cell_width.round() as u32).max(1);
        let encoder_cell_height = (cell_height.round() as u32).max(1);
        self.mouse_cell_width = cell_width;
        self.mouse_cell_height = cell_height;
        self.mouse_encoder_cell_width = encoder_cell_width;
        self.mouse_encoder_cell_height = encoder_cell_height;
        self.mouse_encoder_screen_width = u32::from(cols.max(1)) * encoder_cell_width;
        self.mouse_encoder_screen_height = u32::from(rows.max(1)) * encoder_cell_height;
        self.mouse_encoder.set_size(MouseEncoderSize {
            screen_width: self.mouse_encoder_screen_width,
            screen_height: self.mouse_encoder_screen_height,
            cell_width: encoder_cell_width,
            cell_height: encoder_cell_height,
            padding_top: 0,
            padding_bottom: 0,
            padding_right: 0,
            padding_left: 0,
        });
    }

    /// A host pointer position in the encoder's whole-pixel grid: the same
    /// fractional cell, so the encoder names the cell under the pointer. The
    /// encoder drops positions outside its screen, so a pointer in the host's
    /// padding — or dragged past the grid — is held at the nearest edge cell.
    fn mouse_encoder_position(&self, x: f32, y: f32) -> GhosttyMousePosition {
        let x = x / self.mouse_cell_width * self.mouse_encoder_cell_width as f32;
        let y = y / self.mouse_cell_height * self.mouse_encoder_cell_height as f32;
        GhosttyMousePosition {
            x: x.clamp(0.0, (self.mouse_encoder_screen_width as f32 - 1.0).max(0.0)),
            y: y.clamp(
                0.0,
                (self.mouse_encoder_screen_height as f32 - 1.0).max(0.0),
            ),
        }
    }

    fn encode_mouse(&mut self, event: TerminalMouseEvent) -> Result<Vec<u8>, TerminalActorError> {
        let action = match event.action {
            TerminalMouseAction::Press => GhosttyMouseAction::Press,
            TerminalMouseAction::Release => GhosttyMouseAction::Release,
            TerminalMouseAction::Motion => GhosttyMouseAction::Motion,
        };
        let button = event.button.map(|button| match button {
            TerminalMouseButton::Left => GhosttyMouseButton::Left,
            TerminalMouseButton::Middle => GhosttyMouseButton::Middle,
            TerminalMouseButton::Right => GhosttyMouseButton::Right,
        });
        let mut modifiers = GhosttyMods::empty();
        modifiers.set(GhosttyMods::SHIFT, event.shift);
        modifiers.set(GhosttyMods::ALT, event.alt);
        modifiers.set(GhosttyMods::CTRL, event.control);

        let mut encoded_event = GhosttyMouseEvent::new().map_err(ghostty_error)?;
        encoded_event
            .set_action(action)
            .set_button(button)
            .set_mods(modifiers)
            .set_position(self.mouse_encoder_position(event.x, event.y));
        self.mouse_encoder.set_any_button_pressed(
            event.action != TerminalMouseAction::Release && event.button.is_some(),
        );
        let mut bytes = Vec::new();
        self.mouse_encoder
            .encode_to_vec(&encoded_event, &mut bytes)
            .map_err(ghostty_error)?;
        Ok(bytes)
    }

    fn encode_wheel(&self, lines: isize, cell: Option<(u16, u16)>) -> Option<Vec<u8>> {
        if lines == 0 {
            return None;
        }
        let mode = |mode: Mode| self.terminal.mode(mode).unwrap_or(false);
        let steps = lines.unsigned_abs();
        let up = lines < 0;
        if mode(Mode::NORMAL_MOUSE) || mode(Mode::BUTTON_MOUSE) || mode(Mode::ANY_MOUSE) {
            let (column, row) = cell.unwrap_or((1, 1));
            let button = if up { 64 } else { 65 };
            let mut bytes = Vec::new();
            for _ in 0..steps {
                if mode(Mode::SGR_MOUSE) {
                    bytes.extend_from_slice(format!("\x1b[<{button};{column};{row}M").as_bytes());
                } else {
                    // Legacy X10 encoding; coordinates saturate at its limit.
                    bytes.extend_from_slice(&[
                        0x1b,
                        b'[',
                        b'M',
                        32 + button,
                        32 + u8::try_from(column.min(223)).unwrap_or(223),
                        32 + u8::try_from(row.min(223)).unwrap_or(223),
                    ]);
                }
            }
            return Some(bytes);
        }
        let alternate_screen =
            mode(Mode::ALT_SCREEN) || mode(Mode::ALT_SCREEN_SAVE) || mode(Mode::ALT_SCREEN_LEGACY);
        if alternate_screen && mode(Mode::ALT_SCROLL) {
            let arrow: &[u8] = match (mode(Mode::DECCKM), up) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1bOB",
                (false, true) => b"\x1b[A",
                (false, false) => b"\x1b[B",
            };
            return Some(arrow.repeat(steps));
        }
        None
    }

    /// Encodes pasted text exactly as Ghostty would: unsafe control bytes are
    /// stripped, and the result is wrapped in bracketed-paste markers when the
    /// application enabled mode 2004, or has its newlines converted to
    /// carriage returns when it did not.
    fn encode_paste(&self, text: &str) -> Result<Vec<u8>, TerminalActorError> {
        let bracketed = self
            .terminal
            .mode(Mode::BRACKETED_PASTE)
            .map_err(ghostty_error)?;
        encode_paste_bytes(text, bracketed)
    }

    fn apply_theme(&mut self, theme: TerminalTheme) -> Result<(), TerminalActorError> {
        let mut palette = Palette::default();
        for (index, color) in theme.ansi.into_iter().enumerate() {
            palette.0[index] = color.into();
        }
        self.terminal
            .set_default_fg_color(Some(theme.foreground.into()))
            .and_then(|terminal| terminal.set_default_bg_color(Some(theme.background.into())))
            .and_then(|terminal| terminal.set_default_cursor_color(Some(theme.cursor.into())))
            .and_then(|terminal| terminal.set_default_color_palette(Some(palette)))
            .map_err(ghostty_error)?;
        self.last_snapshot = None;
        Ok(())
    }

    fn snapshot(&mut self) -> Result<GridSnapshot, TerminalActorError> {
        // DEC mode 2026 is a presentation barrier: the VT keeps parsing the
        // application's redraw, but the user continues seeing the last whole
        // frame. Returning the in-progress grid here is the terminal tearing
        // that Codex exposes as transient extra prompt rows.
        if self.synchronized_output_active()
            && let Some(snapshot) = &self.last_snapshot
        {
            return Ok(snapshot.clone());
        }
        let application_scroll = self.application_owns_wheel();
        let mouse_reporting = self.mouse_reporting();
        let snapshot = self
            .render_state
            .update(&self.terminal)
            .map_err(ghostty_error)?;
        let colors = snapshot.colors().map_err(ghostty_error)?;
        let default_foreground = Rgb::from(colors.foreground);
        let default_background = Rgb::from(colors.background);
        let cursor_color = colors.cursor.map(Rgb::from);
        let row_count = usize::from(snapshot.rows().map_err(ghostty_error)?);
        let can_reuse = snapshot.dirty().map_err(ghostty_error)? != Dirty::Full
            && self.last_snapshot.as_ref().is_some_and(|previous| {
                previous.cells.len() == row_count
                    && previous.default_foreground == default_foreground
                    && previous.default_background == default_background
            });
        let cursor = snapshot
            .cursor_viewport()
            .map_err(ghostty_error)?
            .map(|position| CursorSnapshot {
                column: position.x,
                row: position.y,
                visible: snapshot.cursor_visible().unwrap_or(false),
                blinking: snapshot.cursor_blinking().unwrap_or(false),
            });
        let (mut rows, mut cell_rows) = if can_reuse {
            let previous = self
                .last_snapshot
                .as_ref()
                .expect("reusable terminal snapshot should exist");
            (previous.rows.clone(), previous.cells.clone())
        } else {
            (
                vec![Arc::<str>::from(""); row_count],
                vec![Arc::<[CellSnapshot]>::from([]); row_count],
            )
        };
        let mut row_iterator = self.row_iterator.update(&snapshot).map_err(ghostty_error)?;
        let mut row_index = 0;
        let mut selection = vec![None; row_count];

        while let Some(row) = row_iterator.next() {
            // Read for every row, dirty or not. A row's selected range changes
            // when the selection moves over otherwise untouched text, which is
            // exactly what happens when the terminal scrolls under an anchor.
            if let Some(slot) = selection.get_mut(row_index) {
                *slot = row
                    .selection()
                    .map_err(ghostty_error)?
                    .map(|range| SelectedColumns {
                        start: usize::from(range.start_x),
                        end: usize::from(range.end_x),
                    });
            }
            if !can_reuse || row.dirty().map_err(ghostty_error)? {
                let mut cell_iterator = self.cell_iterator.update(row).map_err(ghostty_error)?;
                let mut text = String::new();
                let mut cells = Vec::new();
                while let Some(cell) = cell_iterator.next() {
                    let wide = cell
                        .raw_cell()
                        .map_err(ghostty_error)?
                        .wide()
                        .map_err(ghostty_error)?;
                    let graphemes = cell.graphemes().map_err(ghostty_error)?;
                    let cell_text = if matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead) {
                        String::new()
                    } else if graphemes.is_empty() {
                        " ".to_owned()
                    } else {
                        graphemes.into_iter().collect()
                    };
                    text.push_str(&cell_text);
                    let style = cell.style().map_err(ghostty_error)?;
                    let hyperlink = if cell
                        .raw_cell()
                        .and_then(|cell| cell.has_hyperlink())
                        .map_err(ghostty_error)?
                    {
                        cell_hyperlink(&self.terminal, row_index, cells.len())?
                    } else {
                        None
                    };
                    let mut foreground = cell
                        .fg_color()
                        .map_err(ghostty_error)?
                        .map_or(default_foreground, Rgb::from);
                    let mut background = cell
                        .bg_color()
                        .map_err(ghostty_error)?
                        .map_or(default_background, Rgb::from);
                    if style.inverse {
                        std::mem::swap(&mut foreground, &mut background);
                    }
                    cells.push(CellSnapshot {
                        text: if style.invisible {
                            " ".into()
                        } else {
                            cell_text
                        },
                        columns: match wide {
                            CellWide::Wide => 2,
                            CellWide::SpacerTail => 0,
                            CellWide::Narrow | CellWide::SpacerHead => 1,
                        },
                        foreground,
                        background,
                        bold: style.bold,
                        italic: style.italic,
                        faint: style.faint,
                        underline: style.underline != Underline::None,
                        strikethrough: style.strikethrough,
                        hyperlink,
                    });
                }
                rows[row_index] = Arc::from(text.trim_end());
                cell_rows[row_index] = cells.into();
            }
            row.set_dirty(false).map_err(ghostty_error)?;
            row_index += 1;
        }
        snapshot.set_dirty(Dirty::Clean).map_err(ghostty_error)?;
        let images = image_snapshots(
            &self.terminal,
            &mut self.image_iterator,
            &mut self.image_cache,
        )?;
        let scrollbar = self.terminal.scrollbar().map_err(ghostty_error)?;

        let result = GridSnapshot {
            rows,
            cells: cell_rows,
            images,
            default_foreground,
            default_background,
            cursor_color,
            cursor,
            title: normalize_terminal_title(self.terminal.title().unwrap_or_default()),
            pwd: self
                .terminal
                .pwd()
                .ok()
                .filter(|pwd| !pwd.is_empty())
                .map(str::to_owned),
            scrollbar: ScrollbarSnapshot {
                total: scrollbar.total,
                offset: scrollbar.offset,
                visible: scrollbar.len,
            },
            application_scroll,
            mouse_reporting,
            selection,
        };
        self.last_snapshot = Some(result.clone());
        Ok(result)
    }

    fn take_pty_responses(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.pty_responses.borrow_mut())
    }

    fn scroll_viewport(&mut self, lines: isize) {
        self.terminal.scroll_viewport(ScrollViewport::Delta(lines));
    }
}
fn image_snapshots(
    terminal: &Terminal<'_, '_>,
    iterator: &mut PlacementIterator<'_>,
    cache: &mut HashMap<u64, Arc<ImageSnapshot>>,
) -> Result<Vec<ImagePlacementSnapshot>, TerminalActorError> {
    let graphics = terminal.kitty_graphics().map_err(ghostty_error)?;
    let mut iteration = iterator.update(&graphics).map_err(ghostty_error)?;
    let mut active_generations = HashSet::new();
    let mut placements = Vec::new();

    while let Some(placement) = iteration.next() {
        // libghostty-vt 0.2.1 reports virtual placements but does not expose
        // Ghostty's Unicode-placeholder resolver, so it cannot supply a safe
        // viewport position for this renderer.
        if placement.is_virtual().map_err(ghostty_error)? {
            continue;
        }
        let image_id = placement.image_id().map_err(ghostty_error)?;
        let Some(image) = graphics.image(image_id) else {
            continue;
        };
        let generation = image.generation().map_err(ghostty_error)?;
        active_generations.insert(generation);
        let info = placement
            .placement_render_info(&image, terminal)
            .map_err(ghostty_error)?;
        if !info.viewport_visible || info.pixel_width == 0 || info.pixel_height == 0 {
            continue;
        }
        let image = if let Some(image) = cache.get(&generation) {
            Arc::clone(image)
        } else {
            let width = image.width().map_err(ghostty_error)?;
            let height = image.height().map_err(ghostty_error)?;
            let Some(rgba) = image_rgba(
                width,
                height,
                image.format().map_err(ghostty_error)?,
                image.data().map_err(ghostty_error)?,
            ) else {
                continue;
            };
            let image = Arc::new(ImageSnapshot {
                id: image_id,
                generation,
                width,
                height,
                rgba,
            });
            cache.insert(generation, Arc::clone(&image));
            image
        };
        let z = placement.z().map_err(ghostty_error)?;
        placements.push(ImagePlacementSnapshot {
            image,
            source: ImageSourceRect {
                x: info.source_x,
                y: info.source_y,
                width: info.source_width,
                height: info.source_height,
            },
            column: info.viewport_col,
            row: info.viewport_row,
            width: info.pixel_width,
            height: info.pixel_height,
            x_offset: placement.x_offset().map_err(ghostty_error)?,
            y_offset: placement.y_offset().map_err(ghostty_error)?,
            z,
            layer: if z < i32::MIN / 2 {
                ImageLayer::BelowBackground
            } else if z < 0 {
                ImageLayer::BelowText
            } else {
                ImageLayer::AboveText
            },
        });
    }

    cache.retain(|generation, _| active_generations.contains(generation));
    placements.sort_unstable_by_key(|placement| (placement.z, placement.image.id));
    Ok(placements)
}

fn image_rgba(width: u32, height: u32, format: ImageFormat, data: &[u8]) -> Option<Arc<[u8]>> {
    let pixels = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    let channels = match format {
        ImageFormat::Rgba => 4,
        ImageFormat::Rgb => 3,
        ImageFormat::GrayAlpha => 2,
        ImageFormat::Gray => 1,
        ImageFormat::Png => return None,
        _ => return None,
    };
    let data = data.get(..pixels.checked_mul(channels)?)?;
    if channels == 4 {
        return Some(Arc::from(data));
    }

    let mut rgba = Vec::with_capacity(pixels.checked_mul(4)?);
    match format {
        ImageFormat::Rgb => {
            for pixel in data.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], u8::MAX]);
            }
        }
        ImageFormat::GrayAlpha => {
            for pixel in data.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        ImageFormat::Gray => {
            for value in data {
                rgba.extend_from_slice(&[*value, *value, *value, u8::MAX]);
            }
        }
        ImageFormat::Rgba | ImageFormat::Png => unreachable!("handled before allocation"),
        _ => return None,
    }
    Some(rgba.into())
}

/// A point in the visible grid, which is the only space a pointer knows.
fn viewport_point(column: u16, row: u16) -> Point {
    Point::Viewport(PointCoordinate {
        x: column,
        y: u32::from(row),
    })
}

fn cell_hyperlink(
    terminal: &Terminal<'_, '_>,
    row: usize,
    column: usize,
) -> Result<Option<String>, TerminalActorError> {
    let cell = terminal
        .grid_ref(Point::Viewport(PointCoordinate {
            x: u16::try_from(column)
                .map_err(|_| TerminalActorError::Ghostty("hyperlink column is too large".into()))?,
            y: u32::try_from(row)
                .map_err(|_| TerminalActorError::Ghostty("hyperlink row is too large".into()))?,
        }))
        .map_err(ghostty_error)?;
    let mut bytes = Vec::new();
    let required = match cell.hyperlink_uri(&mut bytes) {
        Ok(0) => return Ok(None),
        Ok(length) => length,
        Err(libghostty_vt::Error::OutOfSpace { required }) => required,
        Err(error) => return Err(ghostty_error(error)),
    };
    bytes.resize(required, 0);
    let length = cell.hyperlink_uri(&mut bytes).map_err(ghostty_error)?;
    bytes.truncate(length);
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| TerminalActorError::Ghostty(format!("invalid OSC 8 hyperlink: {error}")))
}

fn normalize_terminal_title(title: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut previous_was_space = false;
    for character in title.chars().take(256) {
        if character.is_control() || character.is_whitespace() {
            if !previous_was_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            previous_was_space = true;
        } else {
            normalized.push(character);
            previous_was_space = false;
        }
    }
    let normalized = normalized.trim();
    (!normalized.is_empty()).then(|| normalized.to_owned())
}

fn ghostty_error(error: libghostty_vt::Error) -> TerminalActorError {
    TerminalActorError::Ghostty(error.to_string())
}

enum Command {
    Feed(Vec<u8>),
    ApplyTheme(TerminalTheme, Sender<Result<(), TerminalActorError>>),
    Snapshot(Sender<Result<GridSnapshot, TerminalActorError>>),
    TakePtyResponses(Sender<Vec<Vec<u8>>>),
    Select {
        column: u16,
        row: u16,
        extend: bool,
        response: Sender<Result<(), TerminalActorError>>,
    },
    SelectionText(Sender<Result<Option<String>, TerminalActorError>>),
    Shutdown,
}

/// Send-safe handle for a terminal whose Ghostty objects stay on one thread.
pub struct TerminalActor {
    sender: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl TerminalActor {
    pub fn spawn(options: TerminalOptions) -> Result<Self, TerminalActorError> {
        let (sender, receiver) = mpsc::channel();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("muxtrix-terminal".into())
            .spawn(move || run_actor(options, receiver, ready_sender))
            .map_err(|error| TerminalActorError::Spawn(error.to_string()))?;

        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                sender,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(error) => {
                let _ = thread.join();
                Err(TerminalActorError::Channel(error.to_string()))
            }
        }
    }

    pub fn feed(&self, bytes: impl Into<Vec<u8>>) -> Result<(), TerminalActorError> {
        self.sender
            .send(Command::Feed(bytes.into()))
            .map_err(|error| TerminalActorError::Channel(error.to_string()))
    }

    pub fn apply_theme(&self, theme: TerminalTheme) -> Result<(), TerminalActorError> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .send(Command::ApplyTheme(theme, sender))
            .map_err(|error| TerminalActorError::Channel(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| TerminalActorError::Channel(error.to_string()))?
    }

    pub fn snapshot(&self) -> Result<GridSnapshot, TerminalActorError> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .send(Command::Snapshot(sender))
            .map_err(|error| TerminalActorError::Channel(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| TerminalActorError::Channel(error.to_string()))?
    }

    /// Anchors a selection at a viewport cell; the emulator tracks it there.
    pub fn selection_start(&self, column: u16, row: u16) -> Result<(), TerminalActorError> {
        self.select(column, row, false)
    }

    /// Extends the anchored selection to a viewport cell.
    pub fn selection_extend(&self, column: u16, row: u16) -> Result<(), TerminalActorError> {
        self.select(column, row, true)
    }

    fn select(&self, column: u16, row: u16, extend: bool) -> Result<(), TerminalActorError> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .send(Command::Select {
                column,
                row,
                extend,
                response: sender,
            })
            .map_err(|error| TerminalActorError::Channel(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| TerminalActorError::Channel(error.to_string()))?
    }

    pub fn selection_text(&self) -> Result<Option<String>, TerminalActorError> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .send(Command::SelectionText(sender))
            .map_err(|error| TerminalActorError::Channel(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| TerminalActorError::Channel(error.to_string()))?
    }

    pub fn take_pty_responses(&self) -> Result<Vec<Vec<u8>>, TerminalActorError> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .send(Command::TakePtyResponses(sender))
            .map_err(|error| TerminalActorError::Channel(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| TerminalActorError::Channel(error.to_string()))
    }

    pub fn shutdown(mut self) -> Result<(), TerminalActorError> {
        self.stop()
    }

    fn stop(&mut self) -> Result<(), TerminalActorError> {
        let send_result = self.sender.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| TerminalActorError::ThreadPanicked)?;
        }
        send_result.map_err(|error| TerminalActorError::Channel(error.to_string()))
    }
}

impl Drop for TerminalActor {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn run_actor(
    options: TerminalOptions,
    receiver: Receiver<Command>,
    ready: mpsc::SyncSender<Result<(), TerminalActorError>>,
) {
    let mut terminal = match TerminalCore::new(options) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }

    while let Ok(command) = receiver.recv() {
        match command {
            Command::Feed(bytes) => terminal.feed(&bytes),
            Command::ApplyTheme(theme, response) => {
                let _ = response.send(terminal.apply_theme(theme));
            }
            Command::Snapshot(response) => {
                let _ = response.send(terminal.snapshot());
            }
            Command::TakePtyResponses(response) => {
                let _ = response.send(terminal.take_pty_responses());
            }
            Command::Select {
                column,
                row,
                extend,
                response,
            } => {
                let result = if extend {
                    terminal.selection_extend(column, row)
                } else {
                    terminal.selection_start(column, row)
                };
                let _ = response.send(result);
            }
            Command::SelectionText(response) => {
                let _ = response.send(terminal.selection_text());
            }
            Command::Shutdown => break,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TerminalActorError {
    #[error("Ghostty VT operation failed: {0}")]
    Ghostty(String),
    #[error("failed to spawn terminal actor: {0}")]
    Spawn(String),
    #[error("terminal actor channel failed: {0}")]
    Channel(String),
    #[error("terminal actor thread panicked")]
    ThreadPanicked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveSessionEvent {
    Frame(GridSnapshot),
    Notification(TerminalNotification),
    /// The child ended. `clean` is true only for a confirmed zero exit
    /// status; unknown or failed statuses report false so callers keep the
    /// pane (and its error output) around.
    Exited {
        clean: bool,
    },
    Error(String),
}

pub type EventNotifier = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct LiveEventQueue {
    events: Mutex<VecDeque<LiveSessionEvent>>,
    ready: Condvar,
    notifier: Option<EventNotifier>,
}

impl LiveEventQueue {
    fn new(notifier: Option<EventNotifier>) -> Self {
        Self {
            notifier,
            ..Self::default()
        }
    }

    fn push(&self, event: LiveSessionEvent) {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if matches!(event, LiveSessionEvent::Frame(_)) {
            events.retain(|queued| !matches!(queued, LiveSessionEvent::Frame(_)));
        }
        events.push_back(event);
        drop(events);
        self.ready.notify_one();
        if let Some(notifier) = &self.notifier {
            notifier();
        }
    }

    fn try_recv(&self) -> Result<LiveSessionEvent, TryRecvError> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
            .ok_or(TryRecvError::Empty)
    }

    fn recv_timeout(&self, timeout: Duration) -> Result<LiveSessionEvent, RecvTimeoutError> {
        let events = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut events, wait) = self
            .ready
            .wait_timeout_while(events, timeout, |events| events.is_empty())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        events.pop_front().ok_or(if wait.timed_out() {
            RecvTimeoutError::Timeout
        } else {
            RecvTimeoutError::Disconnected
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalNotification {
    pub title: String,
    pub body: String,
}

#[derive(Default)]
struct OscNotificationScanner {
    buffer: Vec<u8>,
}

impl OscNotificationScanner {
    fn push(&mut self, bytes: &[u8]) -> Vec<TerminalNotification> {
        self.buffer.extend_from_slice(bytes);
        let mut notifications = Vec::new();
        loop {
            let Some(start) = self.buffer.windows(2).position(|bytes| bytes == b"\x1b]") else {
                if self.buffer.last() == Some(&0x1b) {
                    self.buffer.drain(..self.buffer.len().saturating_sub(1));
                } else {
                    self.buffer.clear();
                }
                break;
            };
            if start > 0 {
                self.buffer.drain(..start);
            }
            let content_start = 2;
            let terminator = self.buffer[content_start..]
                .iter()
                .position(|byte| *byte == 0x07)
                .map(|offset| (content_start + offset, 1))
                .or_else(|| {
                    self.buffer[content_start..]
                        .windows(2)
                        .position(|bytes| bytes == b"\x1b\\")
                        .map(|offset| (content_start + offset, 2))
                });
            let Some((end, terminator_len)) = terminator else {
                if self.buffer.len() > 64 * 1_024 {
                    self.buffer.clear();
                }
                break;
            };
            if let Some(notification) = parse_notification_osc(&self.buffer[content_start..end]) {
                notifications.push(notification);
            }
            self.buffer.drain(..end + terminator_len);
        }
        notifications
    }
}

fn parse_notification_osc(payload: &[u8]) -> Option<TerminalNotification> {
    let payload = String::from_utf8_lossy(payload);
    if let Some(body) = payload.strip_prefix("9;") {
        return notification("Terminal", body);
    }
    if let Some(body) = payload.strip_prefix("99;") {
        let body = body.rsplit_once(';').map_or(body, |(_, body)| body);
        return notification("Terminal", body);
    }
    let payload = payload.strip_prefix("777;notify;")?;
    let (title, body) = payload.split_once(';').unwrap_or(("Terminal", payload));
    notification(title, body)
}

fn notification(title: &str, body: &str) -> Option<TerminalNotification> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    Some(TerminalNotification {
        title: if title.trim().is_empty() {
            "Terminal".into()
        } else {
            title.trim().into()
        },
        body: body.into(),
    })
}

enum LiveCommand {
    Terminate,
    PtyOutput(PtyOutput),
    PtyEof,
    PtyReadFailed(String),
    Input(Vec<u8>),
    Paste(String),
    ApplyTheme(TerminalTheme),
    Resize {
        size: PtySize,
        cell_width_px: f32,
        cell_height_px: f32,
    },
    ScrollViewport(isize),
    ScrollViewportTo(usize),
    Wheel {
        lines: isize,
        cell: Option<(u16, u16)>,
    },
    Mouse(TerminalMouseEvent),
    Snapshot(Sender<Result<GridSnapshot, LiveSessionError>>),
    SelectionStart {
        column: u16,
        row: u16,
    },
    SelectionExtend {
        column: u16,
        row: u16,
    },
    SelectionClear,
    SelectionText(Sender<Result<Option<String>, LiveSessionError>>),
    Shutdown,
}

fn encode_paste_bytes(text: &str, bracketed: bool) -> Result<Vec<u8>, TerminalActorError> {
    let mut data = text.as_bytes().to_vec();
    // Bracketed wrapping adds twelve bytes; stripping never grows the data.
    let mut buffer = vec![0_u8; data.len() + 16];
    loop {
        match paste::encode(&mut data, bracketed, &mut buffer) {
            Ok(written) => {
                buffer.truncate(written);
                return Ok(buffer);
            }
            Err(libghostty_vt::error::Error::OutOfSpace { required }) => {
                buffer = vec![0_u8; required.max(buffer.len() + 16)];
            }
            Err(error) => return Err(ghostty_error(error)),
        }
    }
}

fn exit_was_clean(session: &mut PtySession) -> bool {
    for _ in 0..20 {
        match session.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => return false,
        }
    }
    false
}

/// An unframed local PTY or remote output retaining its replay provenance.
pub enum SessionReader {
    Pty(Box<dyn Read + Send>),
    Channel(Receiver<PtyOutput>),
}

/// Where a pane's bytes come from and where its control operations go:
/// an in-process PTY, or a session daemon owning the PTY remotely so the
/// process survives this GUI closing.
pub trait SessionBackend: Send + 'static {
    fn take_reader(&mut self) -> Result<SessionReader, String>;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn resize(&self, size: PtySize) -> Result<(), String>;
    fn kill(&mut self) -> Result<(), String>;
    fn process_id(&self) -> Option<u32>;
    /// Exit status polled without blocking; `Some(clean)` once known.
    fn poll_exit(&mut self) -> Result<Option<bool>, String>;
    /// Exit status once the byte stream has ended, waiting briefly.
    fn exit_clean(&mut self) -> bool;
    /// Whether dropping the session should kill the process. In-process
    /// PTYs die with the GUI; daemon-owned panes must survive it — that
    /// survival is the whole point of the session daemon.
    fn kill_on_detach(&self) -> bool {
        true
    }
}

struct LocalBackend(PtySession);

impl SessionBackend for LocalBackend {
    fn take_reader(&mut self) -> Result<SessionReader, String> {
        self.0
            .take_reader()
            .map(SessionReader::Pty)
            .map_err(|error| error.to_string())
    }
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.0.write_all(bytes).map_err(|error| error.to_string())
    }
    fn resize(&self, size: PtySize) -> Result<(), String> {
        self.0.resize(size).map_err(|error| error.to_string())
    }
    fn kill(&mut self) -> Result<(), String> {
        self.0.kill().map_err(|error| error.to_string())
    }
    fn process_id(&self) -> Option<u32> {
        self.0.process_id()
    }
    fn poll_exit(&mut self) -> Result<Option<bool>, String> {
        self.0
            .try_wait()
            .map(|status| status.map(|status| status.success()))
            .map_err(|error| error.to_string())
    }
    fn exit_clean(&mut self) -> bool {
        exit_was_clean(&mut self.0)
    }
}

enum SessionSource {
    Local(LaunchPlan),
    Remote(Box<dyn SessionBackend>),
}

struct LiveSessionInit {
    source: SessionSource,
    size: PtySize,
    options: TerminalOptions,
    theme: Option<TerminalTheme>,
}

/// Owns a native PTY/ConPTY and Ghostty core on a session thread.
pub struct LiveSession {
    sender: Sender<LiveCommand>,
    events: Arc<LiveEventQueue>,
    thread: Option<JoinHandle<()>>,
    process_id: Option<u32>,
}

impl LiveSession {
    pub fn spawn(
        plan: LaunchPlan,
        size: PtySize,
        options: TerminalOptions,
    ) -> Result<Self, LiveSessionError> {
        Self::spawn_inner(plan, size, options, None, None)
    }

    pub fn spawn_with_notifier_and_theme(
        plan: LaunchPlan,
        size: PtySize,
        options: TerminalOptions,
        theme: TerminalTheme,
        notifier: Option<EventNotifier>,
    ) -> Result<Self, LiveSessionError> {
        Self::spawn_inner(plan, size, options, Some(theme), notifier)
    }

    /// A live session whose PTY lives elsewhere (the session daemon).
    pub fn spawn_remote(
        backend: Box<dyn SessionBackend>,
        size: PtySize,
        options: TerminalOptions,
        theme: TerminalTheme,
        notifier: Option<EventNotifier>,
    ) -> Result<Self, LiveSessionError> {
        Self::spawn_source(
            SessionSource::Remote(backend),
            size,
            options,
            Some(theme),
            notifier,
        )
    }

    fn spawn_inner(
        plan: LaunchPlan,
        size: PtySize,
        options: TerminalOptions,
        theme: Option<TerminalTheme>,
        notifier: Option<EventNotifier>,
    ) -> Result<Self, LiveSessionError> {
        Self::spawn_source(SessionSource::Local(plan), size, options, theme, notifier)
    }

    fn spawn_source(
        source: SessionSource,
        size: PtySize,
        options: TerminalOptions,
        theme: Option<TerminalTheme>,
        notifier: Option<EventNotifier>,
    ) -> Result<Self, LiveSessionError> {
        let (sender, receiver) = mpsc::channel();
        let actor_sender = sender.clone();
        let events = Arc::new(LiveEventQueue::new(notifier));
        let session_events = Arc::clone(&events);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("muxtrix-session".into())
            .spawn(move || {
                run_live_session(
                    LiveSessionInit {
                        source,
                        size,
                        options,
                        theme,
                    },
                    actor_sender,
                    receiver,
                    session_events,
                    ready_sender,
                );
            })
            .map_err(|error| LiveSessionError::Spawn(error.to_string()))?;

        match ready_receiver.recv() {
            Ok(Ok(process_id)) => Ok(Self {
                sender,
                events,
                thread: Some(thread),
                process_id,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(error) => {
                let _ = thread.join();
                Err(LiveSessionError::Channel(error.to_string()))
            }
        }
    }

    /// The operating-system process id of the shell attached to this session,
    /// when the platform reported one at spawn time.
    #[must_use]
    pub fn process_id(&self) -> Option<u32> {
        self.process_id
    }

    pub fn input(&self, bytes: impl Into<Vec<u8>>) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::Input(bytes.into()))
    }

    /// Sends clipboard text to the child, encoded against the terminal's own
    /// bracketed-paste state on the session thread.
    pub fn paste(&self, text: impl Into<String>) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::Paste(text.into()))
    }

    pub fn apply_theme(&self, theme: TerminalTheme) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::ApplyTheme(theme))
    }

    /// Resizes the grid. The cell metrics are the host's exact pixel cell —
    /// they size the terminal's pixel reports and map pointer positions to
    /// cells, so a fractional value must reach here unrounded.
    pub fn resize(
        &self,
        size: PtySize,
        cell_width_px: f32,
        cell_height_px: f32,
    ) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::Resize {
            size,
            cell_width_px,
            cell_height_px,
        })
    }

    pub fn scroll_viewport(&self, lines: isize) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::ScrollViewport(lines))
    }

    /// Routes a wheel gesture by the terminal's own state on the session
    /// thread: mouse events to mouse-reporting applications, arrow keys to
    /// alternate-screen applications, and a viewport scroll otherwise.
    pub fn wheel(&self, lines: isize, cell: Option<(u16, u16)>) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::Wheel { lines, cell })
    }

    /// Routes a normalized pointer event through the terminal's active mouse
    /// tracking mode and encoding format on the session thread.
    pub fn mouse(&self, event: TerminalMouseEvent) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::Mouse(event))
    }

    /// Scroll the viewport to an absolute row in the scrollback buffer.
    pub fn scroll_viewport_to(&self, row: usize) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::ScrollViewportTo(row))
    }

    pub fn snapshot(&self) -> Result<GridSnapshot, LiveSessionError> {
        let (sender, receiver) = mpsc::channel();
        self.send(LiveCommand::Snapshot(sender))?;
        receiver
            .recv()
            .map_err(|error| LiveSessionError::Channel(error.to_string()))?
    }

    /// Anchors a selection at a viewport cell. The emulator owns the selection
    /// from here, tracking it through anything that moves the rows beneath it.
    pub fn selection_start(&self, column: u16, row: u16) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::SelectionStart { column, row })
    }

    pub fn selection_extend(&self, column: u16, row: u16) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::SelectionExtend { column, row })
    }

    pub fn selection_clear(&self) -> Result<(), LiveSessionError> {
        self.send(LiveCommand::SelectionClear)
    }

    pub fn selection_text(&self) -> Result<Option<String>, LiveSessionError> {
        let (sender, receiver) = mpsc::channel();
        self.send(LiveCommand::SelectionText(sender))?;
        receiver
            .recv()
            .map_err(|error| LiveSessionError::Channel(error.to_string()))?
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<LiveSessionEvent, RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<LiveSessionEvent, TryRecvError> {
        self.events.try_recv()
    }

    pub fn shutdown(mut self) -> Result<(), LiveSessionError> {
        self.stop()
    }

    fn send(&self, command: LiveCommand) -> Result<(), LiveSessionError> {
        self.sender
            .send(command)
            .map_err(|error| LiveSessionError::Channel(error.to_string()))
    }

    /// Deliberately ends the pane's process — the explicit close path, as
    /// opposed to Drop, which merely detaches daemon-owned panes.
    pub fn terminate(&self) {
        let _ = self.sender.send(LiveCommand::Terminate);
    }

    fn stop(&mut self) -> Result<(), LiveSessionError> {
        // A short-lived child may have already closed the command channel after
        // reporting Exited. Joining its owner thread is still a clean shutdown.
        let _ = self.sender.send(LiveCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| LiveSessionError::ThreadPanicked)?;
        }
        Ok(())
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn run_live_session(
    init: LiveSessionInit,
    actor_sender: Sender<LiveCommand>,
    receiver: Receiver<LiveCommand>,
    events: Arc<LiveEventQueue>,
    ready: mpsc::SyncSender<Result<Option<u32>, LiveSessionError>>,
) {
    let LiveSessionInit {
        source,
        size,
        options,
        theme,
    } = init;
    let mut terminal = match TerminalCore::new(options) {
        Ok(mut terminal) => {
            terminal.set_mouse_geometry(
                size.cols,
                size.rows,
                f32::from(size.pixel_width) / f32::from(size.cols.max(1)),
                f32::from(size.pixel_height) / f32::from(size.rows.max(1)),
            );
            if let Some(theme) = theme
                && let Err(error) = terminal.apply_theme(theme)
            {
                let _ = ready.send(Err(LiveSessionError::Terminal(error.to_string())));
                return;
            }
            terminal
        }
        Err(error) => {
            let _ = ready.send(Err(LiveSessionError::Terminal(error.to_string())));
            return;
        }
    };
    let mut notification_scanner = OscNotificationScanner::default();
    let mut session: Box<dyn SessionBackend> = match source {
        SessionSource::Local(plan) => match PtySession::spawn(&plan, size) {
            Ok(session) => Box::new(LocalBackend(session)),
            Err(error) => {
                let _ = ready.send(Err(LiveSessionError::Pty(error.to_string())));
                return;
            }
        },
        SessionSource::Remote(backend) => backend,
    };
    let reader = match session.take_reader() {
        Ok(reader) => reader,
        Err(error) => {
            let _ = ready.send(Err(LiveSessionError::Pty(error)));
            return;
        }
    };
    if spawn_pty_reader(reader, actor_sender).is_err() {
        let _ = ready.send(Err(LiveSessionError::Spawn(
            "failed to spawn PTY reader".into(),
        )));
        return;
    }
    if ready.send(Ok(session.process_id())).is_err() {
        let _ = session.kill();
        return;
    }

    let mut pending = None;
    loop {
        let command = match pending.take() {
            Some(command) => command,
            None => {
                let sync_timeout = terminal.synchronized_output_timeout_remaining();
                #[cfg(windows)]
                let receive_timeout = Some(
                    sync_timeout
                        .unwrap_or(Duration::from_millis(50))
                        .min(Duration::from_millis(50)),
                );
                #[cfg(not(windows))]
                let receive_timeout = sync_timeout;

                let received = match receive_timeout {
                    Some(timeout) => receiver.recv_timeout(timeout),
                    None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
                };
                match received {
                    Ok(command) => command,
                    Err(RecvTimeoutError::Timeout) => {
                        match terminal.expire_synchronized_output() {
                            Ok(true) => match terminal.snapshot() {
                                Ok(snapshot) => events.push(LiveSessionEvent::Frame(snapshot)),
                                Err(error) => {
                                    events.push(LiveSessionEvent::Error(error.to_string()));
                                }
                            },
                            Ok(false) => {}
                            Err(error) => {
                                events.push(LiveSessionEvent::Error(error.to_string()));
                            }
                        }
                        #[cfg(windows)]
                        match session.poll_exit() {
                            Ok(Some(clean)) => {
                                if let Ok(snapshot) = terminal.snapshot() {
                                    events.push(LiveSessionEvent::Frame(snapshot));
                                }
                                events.push(LiveSessionEvent::Exited { clean });
                                break;
                            }
                            Ok(None) => {}
                            Err(error) => {
                                events.push(LiveSessionEvent::Error(error));
                                events.push(LiveSessionEvent::Exited { clean: false });
                                break;
                            }
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        };
        match command {
            LiveCommand::PtyOutput(output) => {
                process_pty_output(
                    &mut terminal,
                    &mut notification_scanner,
                    &mut session,
                    &events,
                    &output,
                );
                loop {
                    match receiver.try_recv() {
                        Ok(LiveCommand::PtyOutput(output)) => process_pty_output(
                            &mut terminal,
                            &mut notification_scanner,
                            &mut session,
                            &events,
                            &output,
                        ),
                        Ok(command) => {
                            pending = Some(command);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                match terminal.snapshot() {
                    Ok(snapshot) => events.push(LiveSessionEvent::Frame(snapshot)),
                    Err(error) => events.push(LiveSessionEvent::Error(error.to_string())),
                }
            }
            LiveCommand::PtyEof => {
                if terminal.synchronized_output_active()
                    && let Err(error) = terminal.finish_synchronized_output()
                {
                    events.push(LiveSessionEvent::Error(error.to_string()));
                }
                if let Ok(snapshot) = terminal.snapshot() {
                    events.push(LiveSessionEvent::Frame(snapshot));
                }
                events.push(LiveSessionEvent::Exited {
                    clean: session.exit_clean(),
                });
                break;
            }
            LiveCommand::PtyReadFailed(error) => {
                events.push(LiveSessionEvent::Error(error));
                events.push(LiveSessionEvent::Exited { clean: false });
                break;
            }
            LiveCommand::Input(bytes) => {
                if let Err(error) = session.write_all(&bytes) {
                    events.push(LiveSessionEvent::Error(error.to_string()));
                }
            }
            LiveCommand::Paste(text) => match terminal.encode_paste(&text) {
                Ok(bytes) => {
                    if let Err(error) = session.write_all(&bytes) {
                        events.push(LiveSessionEvent::Error(error.to_string()));
                    }
                }
                Err(error) => events.push(LiveSessionEvent::Error(error.to_string())),
            },
            LiveCommand::ApplyTheme(theme) => {
                match terminal
                    .apply_theme(theme)
                    .and_then(|()| terminal.snapshot())
                {
                    Ok(snapshot) => events.push(LiveSessionEvent::Frame(snapshot)),
                    Err(error) => events.push(LiveSessionEvent::Error(error.to_string())),
                }
            }
            LiveCommand::Resize {
                size,
                cell_width_px,
                cell_height_px,
            } => {
                if let Err(error) = session.resize(size) {
                    events.push(LiveSessionEvent::Error(error.to_string()));
                }
                if let Err(error) = terminal.terminal.resize(
                    size.cols,
                    size.rows,
                    (cell_width_px.round() as u32).max(1),
                    (cell_height_px.round() as u32).max(1),
                ) {
                    events.push(LiveSessionEvent::Error(error.to_string()));
                }
                terminal.set_mouse_geometry(size.cols, size.rows, cell_width_px, cell_height_px);
                terminal.sync_output_started_at = None;
                match terminal.snapshot() {
                    Ok(snapshot) => events.push(LiveSessionEvent::Frame(snapshot)),
                    Err(error) => events.push(LiveSessionEvent::Error(error.to_string())),
                }
            }
            LiveCommand::ScrollViewport(lines) => {
                terminal.scroll_viewport(lines);
                match terminal.snapshot() {
                    Ok(snapshot) => events.push(LiveSessionEvent::Frame(snapshot)),
                    Err(error) => events.push(LiveSessionEvent::Error(error.to_string())),
                }
            }
            LiveCommand::ScrollViewportTo(row) => {
                terminal.terminal.scroll_viewport(ScrollViewport::Row(row));
                match terminal.snapshot() {
                    Ok(snapshot) => events.push(LiveSessionEvent::Frame(snapshot)),
                    Err(error) => events.push(LiveSessionEvent::Error(error.to_string())),
                }
            }
            LiveCommand::Wheel { lines, cell } => {
                if let Some(bytes) = terminal.encode_wheel(lines, cell) {
                    if let Err(error) = session.write_all(&bytes) {
                        events.push(LiveSessionEvent::Error(error.to_string()));
                    }
                } else {
                    terminal.scroll_viewport(lines);
                    match terminal.snapshot() {
                        Ok(snapshot) => events.push(LiveSessionEvent::Frame(snapshot)),
                        Err(error) => events.push(LiveSessionEvent::Error(error.to_string())),
                    }
                }
            }
            LiveCommand::Mouse(event) => match terminal.encode_mouse(event) {
                Ok(bytes) if !bytes.is_empty() => {
                    if let Err(error) = session.write_all(&bytes) {
                        events.push(LiveSessionEvent::Error(error.to_string()));
                    }
                }
                Ok(_) => {}
                Err(error) => events.push(LiveSessionEvent::Error(error.to_string())),
            },
            LiveCommand::Snapshot(response) => {
                let result = terminal
                    .snapshot()
                    .map_err(|error| LiveSessionError::Terminal(error.to_string()));
                let _ = response.send(result);
            }
            // Selection is emulator state: it is anchored to tracked
            // references there, so it follows whatever the terminal scrolls
            // without this side having to model rows at all.
            LiveCommand::SelectionStart { column, row } => {
                if let Err(error) = terminal.selection_start(column, row) {
                    events.push(LiveSessionEvent::Error(error.to_string()));
                }
            }
            LiveCommand::SelectionExtend { column, row } => {
                match terminal
                    .selection_extend(column, row)
                    .and_then(|()| terminal.snapshot())
                {
                    Ok(snapshot) => events.push(LiveSessionEvent::Frame(snapshot)),
                    Err(error) => events.push(LiveSessionEvent::Error(error.to_string())),
                }
            }
            LiveCommand::SelectionClear => {
                match terminal
                    .selection_clear()
                    .and_then(|()| terminal.snapshot())
                {
                    Ok(snapshot) => events.push(LiveSessionEvent::Frame(snapshot)),
                    Err(error) => events.push(LiveSessionEvent::Error(error.to_string())),
                }
            }
            LiveCommand::SelectionText(response) => {
                let result = terminal
                    .selection_text()
                    .map_err(|error| LiveSessionError::Terminal(error.to_string()));
                let _ = response.send(result);
            }
            LiveCommand::Shutdown => {
                // Drop-driven: a detach, not a verdict on the process.
                if session.kill_on_detach() {
                    let _ = session.kill();
                }
                break;
            }
            LiveCommand::Terminate => {
                let _ = session.kill();
                break;
            }
        }
    }
}

fn process_pty_output(
    terminal: &mut TerminalCore,
    notification_scanner: &mut OscNotificationScanner,
    session: &mut Box<dyn SessionBackend>,
    events: &LiveEventQueue,
    output: &PtyOutput,
) {
    let (bytes, replaying) = match output {
        PtyOutput::Live(bytes) => (bytes, false),
        PtyOutput::Backlog(bytes) => (bytes, true),
    };
    for notification in notification_scanner.push(bytes) {
        events.push(LiveSessionEvent::Notification(notification));
    }
    terminal.feed(bytes);
    for response in terminal.take_pty_responses() {
        if replaying {
            continue;
        }
        if let Err(error) = session.write_all(&response) {
            events.push(LiveSessionEvent::Error(error));
        }
    }
}

fn spawn_pty_reader(
    reader: SessionReader,
    sender: Sender<LiveCommand>,
) -> Result<(), LiveSessionError> {
    thread::Builder::new()
        .name("muxtrix-pty-reader".into())
        .spawn(move || match reader {
            SessionReader::Pty(mut reader) => {
                let mut buffer = vec![0_u8; 16 * 1_024];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => {
                            let _ = sender.send(LiveCommand::PtyEof);
                            break;
                        }
                        Ok(read) => {
                            if sender
                                .send(LiveCommand::PtyOutput(PtyOutput::Live(
                                    buffer[..read].to_vec(),
                                )))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(LiveCommand::PtyReadFailed(error.to_string()));
                            break;
                        }
                    }
                }
            }
            SessionReader::Channel(receiver) => {
                while let Ok(output) = receiver.recv() {
                    let bytes = match &output {
                        PtyOutput::Live(bytes) | PtyOutput::Backlog(bytes) => bytes,
                    };
                    if bytes.is_empty() {
                        continue;
                    }
                    if sender.send(LiveCommand::PtyOutput(output)).is_err() {
                        return;
                    }
                }
                let _ = sender.send(LiveCommand::PtyEof);
            }
        })
        .map(|_| ())
        .map_err(|error| LiveSessionError::Spawn(error.to_string()))
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LiveSessionError {
    #[error("failed to spawn live terminal session: {0}")]
    Spawn(String),
    #[error("terminal session channel failed: {0}")]
    Channel(String),
    #[error("PTY session failed: {0}")]
    Pty(String),
    #[error("terminal emulator failed: {0}")]
    Terminal(String),
    #[error("live terminal session thread panicked")]
    ThreadPanicked,
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn wheel_routes_by_terminal_state() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(options())?;

        // Primary screen: no bytes — the caller scrolls the viewport.
        assert_eq!(terminal.encode_wheel(-3, None), None);

        // Alternate screen with DEC mode 1007 enabled: arrow keys, one per
        // line, honoring application cursor keys.
        terminal.feed(b"\x1b[?1049h");
        assert_eq!(
            terminal.encode_wheel(-2, None).as_deref(),
            Some(b"\x1b[A\x1b[A".as_slice())
        );
        assert_eq!(
            terminal.encode_wheel(1, None).as_deref(),
            Some(b"\x1b[B".as_slice())
        );
        terminal.feed(b"\x1b[?1h");
        assert_eq!(
            terminal.encode_wheel(-1, None).as_deref(),
            Some(b"\x1bOA".as_slice())
        );
        terminal.feed(b"\x1b[?1l");

        // An application can disable alternate scroll. Ghostty then scrolls
        // the viewport instead of synthesizing cursor keys.
        terminal.feed(b"\x1b[?1007l");
        assert_eq!(terminal.encode_wheel(-1, None), None);
        assert!(!terminal.application_owns_wheel());

        // Mouse reporting takes priority: SGR wheel events at the pointer.
        terminal.feed(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(
            terminal.encode_wheel(-1, Some((12, 7))).as_deref(),
            Some(b"\x1b[<64;12;7M".as_slice())
        );
        assert_eq!(
            terminal.encode_wheel(2, Some((3, 4))).as_deref(),
            Some(b"\x1b[<65;3;4M\x1b[<65;3;4M".as_slice())
        );

        // Legacy encoding when SGR is off.
        terminal.feed(b"\x1b[?1006l");
        assert_eq!(
            terminal.encode_wheel(-1, Some((2, 3))).as_deref(),
            Some([0x1b, b'[', b'M', 32 + 64, 34, 35].as_slice())
        );

        // Leaving the alternate screen restores viewport scrolling.
        terminal.feed(b"\x1b[?1000l\x1b[?1049l");
        assert_eq!(terminal.encode_wheel(-3, None), None);
        Ok(())
    }

    #[test]
    fn mouse_events_route_by_the_programs_tracking_mode() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(options())?;
        let event = |action, button| TerminalMouseEvent {
            action,
            button,
            x: 4.2,
            y: 2.2,
            shift: false,
            alt: false,
            control: false,
        };

        assert!(!terminal.mouse_reporting());
        assert!(
            terminal
                .encode_mouse(event(
                    TerminalMouseAction::Press,
                    Some(TerminalMouseButton::Left)
                ))?
                .is_empty(),
            "a shell that did not enable mouse tracking keeps host-side selection"
        );

        terminal.feed(b"\x1b[?1003h\x1b[?1006h");
        assert!(terminal.mouse_reporting());
        assert_eq!(
            terminal.encode_mouse(event(
                TerminalMouseAction::Press,
                Some(TerminalMouseButton::Left)
            ))?,
            b"\x1b[<0;5;3M"
        );
        assert_eq!(
            terminal.encode_mouse(TerminalMouseEvent {
                x: 5.2,
                ..event(TerminalMouseAction::Motion, None)
            })?,
            b"\x1b[<35;6;3M",
            "mode 1003 must receive hover motion even with no button down"
        );
        assert_eq!(
            terminal.encode_mouse(event(
                TerminalMouseAction::Release,
                Some(TerminalMouseButton::Left)
            ))?,
            b"\x1b[<0;5;3m"
        );

        terminal.feed(b"\x1b[?1003l\x1b[?1000h");
        assert!(
            terminal
                .encode_mouse(event(TerminalMouseAction::Motion, None))?
                .is_empty(),
            "mode 1000 reports buttons, not hover motion"
        );

        terminal.feed(b"\x1b[?1000l\x1b[?9h\x1b[?1006l");
        assert!(
            terminal.mouse_reporting(),
            "legacy X10 mode still owns clicks"
        );
        assert_eq!(
            terminal.encode_mouse(event(
                TerminalMouseAction::Press,
                Some(TerminalMouseButton::Right)
            ))?,
            [0x1b, b'[', b'M', 34, 37, 35]
        );
        Ok(())
    }

    /// The host lays the grid out with fractional cell metrics — 14pt at a
    /// 1.15 line height is a 21.47px row — while the encoder only takes whole
    /// pixels per cell. Feeding it raw positions against a rounded cell made
    /// the reported row drift below the pointer by nearly half a pixel per
    /// row: two thirds of a row by the middle of a tall pane, a whole row at
    /// its foot. Claude Code selects its own text from these reports, so its
    /// selection landed under the pointer instead of on it.
    #[test]
    fn reported_mouse_cells_follow_fractional_cell_metrics() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(TerminalOptions {
            cols: 120,
            rows: 60,
            max_scrollback: 100,
        })?;
        terminal.feed(b"\x1b[?1000h\x1b[?1006h");
        let (cell_width, cell_height) = (11.2_f32, 21.47_f32);
        terminal.set_mouse_geometry(120, 60, cell_width, cell_height);
        let press = |column: u16, row: u16| TerminalMouseEvent {
            action: TerminalMouseAction::Press,
            button: Some(TerminalMouseButton::Left),
            // The middle of the cell the pointer is over.
            x: (f32::from(column) + 0.5) * cell_width,
            y: (f32::from(row) + 0.5) * cell_height,
            shift: false,
            alt: false,
            control: false,
        };
        for (column, row) in [(0, 0), (40, 30), (100, 45), (119, 59)] {
            assert_eq!(
                String::from_utf8_lossy(&terminal.encode_mouse(press(column, row))?),
                format!("\x1b[<0;{};{}M", column + 1, row + 1),
                "pointer over column {column}, row {row}"
            );
        }
        // Positions past the grid clamp to its last cell rather than to a
        // content area that ends in a partial row.
        assert_eq!(
            String::from_utf8_lossy(&terminal.encode_mouse(TerminalMouseEvent {
                x: 200.0 * cell_width,
                y: 90.0 * cell_height,
                ..press(0, 0)
            })?),
            "\x1b[<0;120;60M"
        );
        Ok(())
    }

    /// The `less` case: a pager scrolls the alternate screen by writing one
    /// line and letting the terminal move the rest. The selection is anchored
    /// to a tracked reference, so it must move with the text it covers —
    /// which row numbering alone could never do here, since the alternate
    /// screen has no scrollback for the viewport offset to report.
    #[test]
    fn a_selection_follows_text_the_terminal_scrolls() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(TerminalOptions {
            cols: 20,
            rows: 4,
            max_scrollback: 100,
        })?;
        terminal.feed(b"\x1b[?1049h\x1b[2J\x1b[H");
        terminal.feed(b"alpha\r\nbravo\r\ncharlie\r\ndelta");

        // Select "bravo", on the second row.
        terminal.selection_start(0, 1)?;
        terminal.selection_extend(4, 1)?;
        assert_eq!(terminal.selection_text()?.as_deref(), Some("bravo"));
        let selected_rows = |snapshot: &GridSnapshot| {
            snapshot
                .selection
                .iter()
                .enumerate()
                .filter_map(|(row, range)| range.map(|range| (row, range.start, range.end)))
                .collect::<Vec<_>>()
        };
        assert_eq!(selected_rows(&terminal.snapshot()?), vec![(1, 0, 4)]);

        // The pager scrolls one line: one new line at the bottom, and the
        // terminal moves everything else up.
        terminal.feed(b"\r\necho");
        let snapshot = terminal.snapshot()?;
        assert_eq!(
            snapshot.rows.first().map(AsRef::as_ref),
            Some("bravo"),
            "the text should have moved up a row"
        );
        assert_eq!(
            selected_rows(&snapshot),
            vec![(0, 0, 4)],
            "the selection should have followed it"
        );
        assert_eq!(
            terminal.selection_text()?.as_deref(),
            Some("bravo"),
            "and it should still describe the text that was picked"
        );

        terminal.selection_clear()?;
        assert_eq!(terminal.selection_text()?, None);
        assert!(selected_rows(&terminal.snapshot()?).is_empty());
        Ok(())
    }

    /// Full-screen applications repaint different text into the same grid
    /// rows as autonomous output arrives. The selection stays anchored to the
    /// cells the user dragged over — the universal emulator contract — so it
    /// covers whatever the program paints there next, and a copy takes the
    /// text those cells now hold rather than a remembered string.
    #[test]
    fn a_selection_keeps_its_cells_through_an_application_repaint() -> Result<(), TerminalActorError>
    {
        let mut terminal = TerminalCore::new(TerminalOptions {
            cols: 24,
            rows: 4,
            max_scrollback: 100,
        })?;
        terminal.feed(b"\x1b[?1049h\x1b[2J\x1b[Halpha\r\nbravo\r\ncharlie\r\ndelta");
        terminal.selection_start(0, 1)?;
        terminal.selection_extend(4, 1)?;
        let selected_rows = |snapshot: &GridSnapshot| {
            snapshot
                .selection
                .iter()
                .enumerate()
                .filter_map(|(row, range)| range.map(|range| (row, range.start, range.end)))
                .collect::<Vec<_>>()
        };
        assert_eq!(selected_rows(&terminal.snapshot()?), vec![(1, 0, 4)]);

        // The application repaints different content into the same rows. The
        // highlight neither jumps to a lookalike elsewhere on screen nor
        // abandons the dragged cells.
        terminal.feed(b"\x1b[2J\x1b[Hone\r\nbravo\r\nthree\r\nbravo");
        let repainted = terminal.snapshot()?;
        assert_eq!(selected_rows(&repainted), vec![(1, 0, 4)]);
        assert_eq!(
            terminal.selection_text()?.as_deref(),
            Some("bravo"),
            "the copy describes the cells as they are now"
        );

        // A repaint that changes the covered text changes the copy with it:
        // the selection never reports text it does not paint.
        terminal.feed(b"\x1b[2J\x1b[Hone\r\ntwo\r\nthree\r\nbravo");
        let changed = terminal.snapshot()?;
        assert_eq!(selected_rows(&changed), vec![(1, 0, 4)]);
        assert_eq!(terminal.selection_text()?.as_deref(), Some("two"));
        Ok(())
    }

    #[test]
    fn a_selection_follows_scrollback_on_the_primary_screen() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(TerminalOptions {
            cols: 20,
            rows: 3,
            max_scrollback: 100,
        })?;
        terminal.feed(b"one\r\ntwo\r\nthree");
        terminal.selection_start(0, 1)?;
        terminal.selection_extend(2, 1)?;
        assert_eq!(terminal.selection_text()?.as_deref(), Some("two"));

        // Push the selected line up into the scrollback.
        terminal.feed(b"\r\nfour\r\nfive");
        assert_eq!(
            terminal.selection_text()?.as_deref(),
            Some("two"),
            "a selection scrolled out of view still describes its text"
        );
        assert!(
            terminal
                .snapshot()?
                .selection
                .iter()
                .all(std::option::Option::is_none),
            "and paints nothing while it is off screen"
        );

        terminal.scroll_viewport(-2);
        let snapshot = terminal.snapshot()?;
        let selected: Vec<_> = snapshot
            .selection
            .iter()
            .enumerate()
            .filter_map(|(row, range)| range.map(|range| (row, range.start, range.end)))
            .collect();
        assert_eq!(
            selected,
            vec![(1, 0, 2)],
            "scrolling back to it paints it again, on the row it now occupies"
        );
        Ok(())
    }

    #[test]
    fn a_snapshot_reports_who_answers_the_wheel() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(options())?;
        assert!(
            !terminal.snapshot()?.application_scroll,
            "the primary screen scrolls its own viewport"
        );

        // The alternate screen: the program redraws its content in place.
        terminal.feed(b"\x1b[?1049h");
        assert!(terminal.snapshot()?.application_scroll);
        terminal.feed(b"\x1b[?1007l");
        assert!(
            !terminal.snapshot()?.application_scroll,
            "alternate screens only own the wheel while mode 1007 is enabled"
        );
        terminal.feed(b"\x1b[?1007h");
        terminal.feed(b"\x1b[?1049l");
        assert!(!terminal.snapshot()?.application_scroll);

        // Mouse reporting alone routes the wheel to the program too, even on
        // the primary screen.
        terminal.feed(b"\x1b[?1000h");
        let snapshot = terminal.snapshot()?;
        assert!(snapshot.application_scroll);
        assert!(snapshot.mouse_reporting);
        terminal.feed(b"\x1b[?1000l");
        let snapshot = terminal.snapshot()?;
        assert!(!snapshot.application_scroll);
        assert!(!snapshot.mouse_reporting);

        // The flag must agree with where the wheel actually goes.
        for sequence in [
            b"\x1b[?1049h".as_slice(),
            b"\x1b[?1000h".as_slice(),
            b"\x1b[?1003h".as_slice(),
        ] {
            terminal.feed(sequence);
            assert_eq!(
                terminal.snapshot()?.application_scroll,
                terminal.encode_wheel(-1, Some((1, 1))).is_some(),
                "the snapshot flag disagreed with the wheel routing"
            );
        }
        Ok(())
    }

    #[test]
    fn paste_encoding_follows_the_terminal_bracketed_paste_mode() -> Result<(), TerminalActorError>
    {
        let mut terminal = TerminalCore::new(options())?;
        assert_eq!(terminal.encode_paste("one\ntwo")?, b"one\rtwo".to_vec());

        terminal.feed(b"\x1b[?2004h");
        assert_eq!(
            terminal.encode_paste("one\ntwo")?,
            b"\x1b[200~one\ntwo\x1b[201~".to_vec()
        );

        terminal.feed(b"\x1b[?2004l");
        assert_eq!(terminal.encode_paste("one\ntwo")?, b"one\rtwo".to_vec());
        Ok(())
    }

    #[test]
    fn paste_encoding_strips_bytes_that_could_inject_sequences() {
        let encoded =
            encode_paste_bytes("safe\x1b[201~text", false).expect("paste encoding should succeed");
        assert!(
            !encoded.contains(&0x1b),
            "escape bytes must never survive an unbracketed paste"
        );
    }

    fn options() -> TerminalOptions {
        TerminalOptions {
            cols: 24,
            rows: 4,
            max_scrollback: 100,
        }
    }

    #[test]
    fn ghostty_parses_vt_sequences_on_its_actor_thread() -> Result<(), TerminalActorError> {
        let actor = TerminalActor::spawn(options())?;
        actor.feed(b"plain \x1b[31mred\x1b[0m\r\nsecond".to_vec())?;
        let snapshot = actor.snapshot()?;

        assert!(snapshot.text().contains("plain red"));
        assert!(snapshot.text().contains("second"));
        let red_cell = snapshot
            .cells
            .iter()
            .flat_map(|row| row.iter())
            .find(|cell| cell.text == "r")
            .expect("styled red cell should be present");
        assert_ne!(red_cell.foreground, snapshot.default_foreground);
        assert!(snapshot.cursor.is_some());
        actor.shutdown()
    }

    #[test]
    fn kitty_png_placements_are_exposed_as_reusable_rgba_frames() -> Result<(), TerminalActorError>
    {
        let mut terminal = TerminalCore::new(options())?;
        terminal
            .terminal
            .resize(options().cols, options().rows, 8, 16)
            .map_err(ghostty_error)?;
        terminal.feed(
            b"\x1b_Gi=7,a=T,f=100,q=2,z=-1;iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==\x1b\\",
        );

        let first = terminal.snapshot()?;
        let placement = first.images.first().expect("Kitty placement");
        assert_eq!(placement.image.id, 7);
        assert_eq!(&*placement.image.rgba, &[255, 0, 0, 255]);
        assert_eq!(placement.source.width, 1);
        assert_eq!(placement.source.height, 1);
        assert_eq!(placement.layer, ImageLayer::BelowText);

        let second = terminal.snapshot()?;
        assert!(Arc::ptr_eq(
            &placement.image,
            &second.images.first().expect("retained placement").image,
        ));
        Ok(())
    }

    #[test]
    fn themes_change_defaults_and_ansi_without_recoloring_direct_rgb()
    -> Result<(), TerminalActorError> {
        let actor = TerminalActor::spawn(options())?;
        let mut ansi = [Rgb {
            red: 32,
            green: 32,
            blue: 32,
        }; 16];
        ansi[1] = Rgb {
            red: 240,
            green: 80,
            blue: 96,
        };
        actor.apply_theme(TerminalTheme {
            foreground: Rgb {
                red: 224,
                green: 225,
                blue: 226,
            },
            background: Rgb {
                red: 18,
                green: 19,
                blue: 20,
            },
            cursor: Rgb {
                red: 200,
                green: 201,
                blue: 202,
            },
            ansi,
        })?;
        actor.feed(b"\x1b[31mA\x1b[38;2;1;2;3mB".to_vec())?;
        let snapshot = actor.snapshot()?;
        let cells: Vec<_> = snapshot.cells.iter().flat_map(|row| row.iter()).collect();

        assert_eq!(snapshot.default_foreground.red, 224);
        assert_eq!(snapshot.default_background.red, 18);
        assert_eq!(snapshot.cursor_color.expect("cursor theme").red, 200);
        assert_eq!(cells[0].foreground, ansi[1]);
        assert_eq!(
            cells[1].foreground,
            Rgb {
                red: 1,
                green: 2,
                blue: 3,
            }
        );
        actor.shutdown()
    }

    #[test]
    fn runtime_osc_palette_override_survives_theme_changes() -> Result<(), TerminalActorError> {
        let actor = TerminalActor::spawn(options())?;
        let mut first_ansi = [Rgb {
            red: 20,
            green: 20,
            blue: 20,
        }; 16];
        first_ansi[1] = Rgb {
            red: 200,
            green: 10,
            blue: 10,
        };
        let first = TerminalTheme {
            foreground: first_ansi[7],
            background: first_ansi[0],
            cursor: first_ansi[7],
            ansi: first_ansi,
        };
        actor.apply_theme(first)?;
        actor.feed(b"\x1b]4;1;#0a141e\x07\x1b[31mX".to_vec())?;

        let mut second = first;
        second.ansi[1] = Rgb {
            red: 240,
            green: 30,
            blue: 30,
        };
        actor.apply_theme(second)?;
        let snapshot = actor.snapshot()?;
        assert_eq!(
            snapshot.cells[0][0].foreground,
            Rgb {
                red: 10,
                green: 20,
                blue: 30,
            }
        );
        actor.shutdown()
    }

    #[test]
    fn ghostty_accepts_split_unicode_output() -> Result<(), TerminalActorError> {
        let actor = TerminalActor::spawn(options())?;
        actor.feed(vec![0xc6, 0xb0, b'm'])?;
        let snapshot = actor.snapshot()?;
        assert!(snapshot.text().contains("ưm"));
        actor.shutdown()
    }

    #[test]
    fn terminal_query_responses_are_captured_for_the_pty_writer() -> Result<(), TerminalActorError>
    {
        let actor = TerminalActor::spawn(options())?;
        actor.feed(b"\x1b[6n".to_vec())?;
        let responses = actor.take_pty_responses()?;

        assert!(!responses.is_empty());
        actor.shutdown()
    }

    #[test]
    fn configured_scrollback_retains_lines_across_widths_and_resize()
    -> Result<(), TerminalActorError> {
        for cols in [40, 80, 240] {
            let mut terminal = TerminalCore::new(TerminalOptions {
                cols,
                rows: 24,
                max_scrollback: 20_000,
            })?;
            // Use real VT output rather than merely checking the launch option.
            for line in 0..20_000 {
                terminal.feed(format!("line-{line:05}\r\n").as_bytes());
            }
            terminal.terminal.scroll_viewport(ScrollViewport::Top);
            let top = terminal.snapshot()?;
            assert!(
                top.text().contains("line-00000"),
                "cols={cols}: {:?}",
                top.scrollbar
            );
            assert_eq!(
                terminal.terminal.scrollback_rows().map_err(ghostty_error)?,
                19_977
            );

            // Widening a pane must not turn its line limit into a smaller
            // effective byte budget. Short lines avoid reflow in this check.
            terminal
                .terminal
                .resize(300, 24, 8, 16)
                .map_err(ghostty_error)?;
            for line in 20_000..60_000 {
                terminal.feed(format!("\x1b[32mline-{line:05}\x1b[0m\r\n").as_bytes());
            }
            let history = terminal.terminal.scrollback_rows().map_err(ghostty_error)?;
            // Ghostty recycles whole pages, so allow a small page-sized excess.
            assert!(
                (20_000..20_512).contains(&history),
                "cols={cols}: {history}"
            );
            terminal.terminal.scroll_viewport(ScrollViewport::Top);
            let top = terminal.snapshot()?.text();
            assert!(!top.contains("line-00000"));
            terminal.terminal.scroll_viewport(ScrollViewport::Bottom);
            assert!(terminal.snapshot()?.text().contains("line-59999"));
        }
        Ok(())
    }

    #[test]
    fn zero_scrollback_and_alternate_screen_keep_no_history() -> Result<(), TerminalActorError> {
        for limit in [0, 20_000] {
            let mut terminal = TerminalCore::new(TerminalOptions {
                cols: 80,
                rows: 24,
                max_scrollback: limit,
            })?;
            if limit != 0 {
                terminal.feed(b"\x1b[?1049h");
            }
            for _ in 0..1_000 {
                terminal.feed(b"output\r\n");
            }
            assert_eq!(
                terminal.terminal.scrollback_rows().map_err(ghostty_error)?,
                0
            );
        }
        Ok(())
    }

    #[test]
    fn viewport_scroll_uses_ghostty_scrollback() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(options())?;
        terminal.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
        let bottom = terminal.snapshot()?;
        assert!(bottom.scrollbar.is_scrollable());
        assert_eq!(bottom.scrollbar.visible, 4);

        terminal.scroll_viewport(-2);
        let scrolled = terminal.snapshot()?;
        assert_ne!(scrolled.text(), bottom.text());
        assert!(scrolled.text().contains("three"));
        assert!(scrolled.scrollbar.offset < bottom.scrollbar.offset);

        terminal.terminal.scroll_viewport(ScrollViewport::Bottom);
        assert_eq!(terminal.snapshot()?.text(), bottom.text());
        Ok(())
    }

    #[test]
    fn incremental_snapshots_reuse_clean_terminal_rows() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(options())?;
        terminal.feed(b"first row\r\nsecond row");
        let first = terminal.snapshot()?;

        terminal.feed(b"!");
        let second = terminal.snapshot()?;

        assert!(Arc::ptr_eq(&first.cells[0], &second.cells[0]));
        assert!(!Arc::ptr_eq(&first.cells[1], &second.cells[1]));
        assert!(second.text().contains("second row!"));
        Ok(())
    }

    #[test]
    fn synchronized_output_keeps_the_last_complete_frame_until_commit()
    -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(options())?;
        terminal.feed(b"complete frame");
        let complete = terminal.snapshot()?;

        terminal.feed(b"\x1b[?2026h\x1b[2J\x1b[Hpartial redraw");
        assert!(terminal.synchronized_output_active());
        assert_eq!(
            terminal.snapshot()?,
            complete,
            "an in-progress synchronized redraw must never be presented"
        );

        terminal.feed(b"\x1b[?2026l");
        let committed = terminal.snapshot()?;
        assert!(!terminal.synchronized_output_active());
        assert!(committed.text().contains("partial redraw"));
        assert_ne!(committed, complete);
        Ok(())
    }

    #[test]
    fn stuck_synchronized_output_expires_like_ghostty() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(options())?;
        terminal.feed(b"complete frame");
        let complete = terminal.snapshot()?;
        terminal.feed(b"\x1b[?2026h\x1b[2J\x1b[Hunfinished redraw");
        terminal.sync_output_started_at = Some(Instant::now() - SYNC_OUTPUT_RESET_AFTER);

        assert!(terminal.expire_synchronized_output()?);
        let recovered = terminal.snapshot()?;
        assert!(!terminal.synchronized_output_active());
        assert!(recovered.text().contains("unfinished redraw"));
        assert_ne!(recovered, complete);
        Ok(())
    }

    #[test]
    fn wide_glyph_spacer_cells_do_not_add_rendered_width() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(options())?;
        terminal.feed("界".as_bytes());
        let snapshot = terminal.snapshot()?;

        assert_eq!(snapshot.cells[0][0].text, "界");
        assert_eq!(snapshot.cells[0][0].columns, 2);
        assert_eq!(snapshot.cells[0][1].text, "");
        assert_eq!(snapshot.cells[0][1].columns, 0);
        assert_eq!(snapshot.text().lines().next(), Some("界"));
        Ok(())
    }

    #[test]
    fn osc_window_title_is_sanitized_and_exposed_in_snapshots() -> Result<(), TerminalActorError> {
        let mut terminal = TerminalCore::new(options())?;
        terminal.feed(b"\x1b]2; cargo   test \n workspace \x07");
        let snapshot = terminal.snapshot()?;

        assert_eq!(snapshot.title.as_deref(), Some("cargo test workspace"));
        assert_eq!(normalize_terminal_title("\n\t"), None);
        Ok(())
    }

    #[test]
    fn consecutive_osc7_reports_keep_updating_pwd() {
        let actor = TerminalActor::spawn(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 100,
        })
        .expect("actor");
        actor
            .feed(b"one\x1b]7;file://h/home/x\x1b\\two".to_vec())
            .expect("feed 1");
        let first = actor.snapshot().expect("snap 1").pwd;
        actor
            .feed(b"three\x1b]7;file://h/tmp\x1b\\four".to_vec())
            .expect("feed 2");
        let second = actor.snapshot().expect("snap 2").pwd;
        actor
            .feed(b"five\x1b]7;file://h/var\x07six".to_vec())
            .expect("feed 3");
        let third = actor.snapshot().expect("snap 3").pwd;
        actor.shutdown().expect("shutdown");
        assert_eq!(
            (first.as_deref(), second.as_deref(), third.as_deref()),
            (
                Some("file://h/home/x"),
                Some("file://h/tmp"),
                Some("file://h/var")
            )
        );
    }

    /// Blocks until its sender half drops — an instantly-EOF reader would
    /// let the session loop exit before commands arrive, making the tests
    /// race their own setup.
    struct BlockingReader(mpsc::Receiver<Vec<u8>>);

    impl std::io::Read for BlockingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            let _ = self.0.recv();
            Ok(0)
        }
    }

    struct FlagBackend {
        killed: Arc<std::sync::atomic::AtomicBool>,
        // Held so the paired reader blocks for the backend's lifetime.
        _keep_reader_open: mpsc::Sender<Vec<u8>>,
        reader: Option<mpsc::Receiver<Vec<u8>>>,
    }

    impl SessionBackend for FlagBackend {
        fn take_reader(&mut self) -> Result<SessionReader, String> {
            self.reader
                .take()
                .map(|receiver| SessionReader::Pty(Box::new(BlockingReader(receiver))))
                .ok_or_else(|| "reader taken twice".to_owned())
        }
        fn write_all(&mut self, _bytes: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn resize(&self, _size: PtySize) -> Result<(), String> {
            Ok(())
        }
        fn kill(&mut self) -> Result<(), String> {
            self.killed
                .store(true, std::sync::atomic::Ordering::Release);
            Ok(())
        }
        fn process_id(&self) -> Option<u32> {
            None
        }
        fn poll_exit(&mut self) -> Result<Option<bool>, String> {
            Ok(None)
        }
        fn exit_clean(&mut self) -> bool {
            true
        }
        fn kill_on_detach(&self) -> bool {
            false
        }
    }

    fn spawn_flag_session(killed: &Arc<std::sync::atomic::AtomicBool>) -> LiveSession {
        let (sender, receiver) = mpsc::channel();
        LiveSession::spawn_remote(
            Box::new(FlagBackend {
                killed: Arc::clone(killed),
                _keep_reader_open: sender,
                reader: Some(receiver),
            }),
            PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
            TerminalOptions {
                cols: 80,
                rows: 24,
                max_scrollback: 100,
            },
            TerminalTheme {
                foreground: Rgb {
                    red: 255,
                    green: 255,
                    blue: 255,
                },
                background: Rgb {
                    red: 0,
                    green: 0,
                    blue: 0,
                },
                cursor: Rgb {
                    red: 255,
                    green: 255,
                    blue: 255,
                },
                ansi: [Rgb {
                    red: 128,
                    green: 128,
                    blue: 128,
                }; 16],
            },
            None,
        )
        .expect("remote session should spawn")
    }

    #[test]
    fn dropping_a_remote_session_detaches_without_killing_the_pane() {
        // GUI exit must leave daemon-owned processes running — that
        // survival is the entire point of the session daemon.
        let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        drop(spawn_flag_session(&killed));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!killed.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn terminating_a_remote_session_kills_the_pane() {
        let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let session = spawn_flag_session(&killed);
        session.terminate();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline
            && !killed.load(std::sync::atomic::Ordering::Acquire)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(killed.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn live_event_queue_keeps_only_the_latest_frame_without_dropping_events() {
        fn frame(text: &str) -> GridSnapshot {
            GridSnapshot {
                rows: vec![text.into()],
                cells: Vec::new(),
                images: Vec::new(),
                default_foreground: Rgb {
                    red: 255,
                    green: 255,
                    blue: 255,
                },
                default_background: Rgb {
                    red: 0,
                    green: 0,
                    blue: 0,
                },
                cursor_color: None,
                cursor: None,
                pwd: None,
                scrollbar: ScrollbarSnapshot::default(),
                title: None,
                application_scroll: false,
                mouse_reporting: false,
                selection: Vec::new(),
            }
        }

        let queue = LiveEventQueue::default();
        queue.push(LiveSessionEvent::Frame(frame("old")));
        queue.push(LiveSessionEvent::Notification(TerminalNotification {
            title: "Codex".into(),
            body: "Needs input".into(),
        }));
        queue.push(LiveSessionEvent::Frame(frame("latest")));

        assert!(matches!(
            queue.try_recv(),
            Ok(LiveSessionEvent::Notification(_))
        ));
        let latest = match queue.try_recv() {
            Ok(LiveSessionEvent::Frame(snapshot)) => snapshot,
            event => panic!("expected latest frame, got {event:?}"),
        };
        assert_eq!(latest.text(), "latest");
        assert_eq!(queue.try_recv(), Err(TryRecvError::Empty));
    }

    #[cfg(unix)]
    #[test]
    fn live_session_streams_a_real_pty_through_ghostty() -> Result<(), Box<dyn std::error::Error>> {
        let plan = LaunchPlan {
            executable: "/bin/sh".into(),
            arguments: vec![
                "-c".into(),
                "printf '\\033[32mheadless-pty\\033[0m\\n'".into(),
            ],
            working_directory: Some(PathBuf::from("/tmp")),
            environment: vec![("TERM".into(), "xterm-256color".into())],
        };
        let size = PtySize {
            rows: 4,
            cols: 32,
            pixel_width: 320,
            pixel_height: 80,
        };
        let session = LiveSession::spawn(plan, size, options())?;
        let mut observed = String::new();

        loop {
            match session.recv_timeout(Duration::from_secs(2))? {
                LiveSessionEvent::Frame(snapshot) => observed = snapshot.text(),
                LiveSessionEvent::Notification(_) => {}
                LiveSessionEvent::Exited { .. } => break,
                LiveSessionEvent::Error(error) => return Err(error.into()),
            }
        }

        assert!(observed.contains("headless-pty"));
        session.shutdown()?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn live_session_forwards_input_to_the_pty() -> Result<(), Box<dyn std::error::Error>> {
        let plan = LaunchPlan {
            executable: "/bin/sh".into(),
            arguments: vec![
                "-c".into(),
                "stty -echo; IFS= read -r line; printf 'received:%s\\n' \"$line\"".into(),
            ],
            working_directory: Some(PathBuf::from("/tmp")),
            environment: vec![("TERM".into(), "xterm-256color".into())],
        };
        let size = PtySize {
            rows: 4,
            cols: 32,
            pixel_width: 320,
            pixel_height: 80,
        };
        let session = LiveSession::spawn(plan, size, options())?;
        session.input(b"keyboard-path\r".to_vec())?;
        let mut observed = String::new();

        loop {
            match session.recv_timeout(Duration::from_secs(2))? {
                LiveSessionEvent::Frame(snapshot) => observed = snapshot.text(),
                LiveSessionEvent::Notification(_) => {}
                LiveSessionEvent::Exited { .. } => break,
                LiveSessionEvent::Error(error) => return Err(error.into()),
            }
        }

        assert!(observed.contains("received:keyboard-path"));
        session.shutdown()?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn live_session_publishes_resized_grid_immediately() -> Result<(), Box<dyn std::error::Error>> {
        let plan = LaunchPlan {
            executable: "/bin/sh".into(),
            arguments: vec!["-c".into(), "sleep 5".into()],
            working_directory: Some(PathBuf::from("/tmp")),
            environment: vec![("TERM".into(), "xterm-256color".into())],
        };
        let initial_size = PtySize {
            rows: 4,
            cols: 24,
            pixel_width: 240,
            pixel_height: 80,
        };
        let session = LiveSession::spawn(plan, initial_size, options())?;
        let resized = PtySize {
            rows: 7,
            cols: 31,
            pixel_width: 310,
            pixel_height: 140,
        };
        session.resize(resized, 10.0, 20.0)?;

        let snapshot = match session.recv_timeout(Duration::from_secs(2))? {
            LiveSessionEvent::Frame(snapshot) => snapshot,
            LiveSessionEvent::Notification(_) => {
                return Err("unexpected notification before resize frame".into());
            }
            LiveSessionEvent::Exited { .. } => {
                return Err("shell exited before resize frame".into());
            }
            LiveSessionEvent::Error(error) => return Err(error.into()),
        };

        assert_eq!(snapshot.cells.len(), usize::from(resized.rows));
        assert!(
            snapshot
                .cells
                .iter()
                .all(|row| row.len() == usize::from(resized.cols))
        );
        session.shutdown()?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn live_session_keeps_input_aligned_after_resize() -> Result<(), Box<dyn std::error::Error>> {
        let plan = LaunchPlan {
            executable: "/bin/sh".into(),
            arguments: vec![
                "-c".into(),
                "stty -echo; IFS= read -r line; size=$(stty size); printf 'received:%s size:%s\\n' \"$line\" \"$size\"".into(),
            ],
            working_directory: Some(PathBuf::from("/tmp")),
            environment: vec![("TERM".into(), "xterm-256color".into())],
        };
        let initial_size = PtySize {
            rows: 4,
            cols: 24,
            pixel_width: 240,
            pixel_height: 80,
        };
        let resized = PtySize {
            rows: 7,
            cols: 31,
            pixel_width: 310,
            pixel_height: 140,
        };
        let session = LiveSession::spawn(plan, initial_size, options())?;
        session.resize(resized, 10.0, 20.0)?;
        session.input(b"after-resize\r".to_vec())?;
        let mut observed = String::new();

        loop {
            match session.recv_timeout(Duration::from_secs(2))? {
                LiveSessionEvent::Frame(snapshot) => observed = snapshot.text(),
                LiveSessionEvent::Notification(_) => {}
                LiveSessionEvent::Exited { .. } => break,
                LiveSessionEvent::Error(error) => return Err(error.into()),
            }
        }

        assert!(observed.contains("received:after-resize"));
        assert!(observed.contains("size:7 31"));
        session.shutdown()?;
        Ok(())
    }

    #[test]
    fn notification_scanner_handles_supported_osc_sequences_and_chunk_boundaries() {
        let mut scanner = OscNotificationScanner::default();
        assert!(
            scanner
                .push(b"prefix\x1b]777;notify;Codex;Needs ")
                .is_empty()
        );
        assert_eq!(
            scanner.push(b"input\x07tail\x1b]9;Build complete\x1b\\"),
            vec![
                TerminalNotification {
                    title: "Codex".into(),
                    body: "Needs input".into(),
                },
                TerminalNotification {
                    title: "Terminal".into(),
                    body: "Build complete".into(),
                },
            ]
        );
        assert_eq!(
            scanner.push(b"\x1b]99;i=1:p=body;Agent waiting\x07"),
            vec![TerminalNotification {
                title: "Terminal".into(),
                body: "Agent waiting".into(),
            }]
        );
    }
}
