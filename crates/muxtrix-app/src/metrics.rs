//! Measured terminal cell metrics and glyph fallback selection.
//!
//! The terminal grid places every run at a fixed multiple of one advance width.
//! That width must come from the face actually being rendered: a guessed ratio
//! drifts against the shaped text and clips the tail of every run. Faces are
//! measured once through `fontdb` and cached for the process.

use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

use fontdb::{Database, Family, Query, Stretch, Style, Weight};

use crate::settings::intern_font_family;

/// Advance ratio assumed when no face can be measured. Most monospace faces sit
/// near this value, so it degrades gracefully rather than collapsing the grid.
pub(crate) const FALLBACK_ADVANCE_RATIO: f32 = 0.6;

/// Ratios outside this range indicate a proportional or broken face rather than
/// a usable terminal cell.
const MIN_ADVANCE_RATIO: f32 = 0.3;
const MAX_ADVANCE_RATIO: f32 = 1.2;

/// Probed in order; the first glyph present decides the advance width.
const PROBE_CHARS: [char; 3] = ['M', '0', 'x'];

/// Bounds on substitution scaling, so a pathological face cannot collapse a
/// glyph to nothing or blow it far past its cell.
const MIN_FALLBACK_SCALE: f32 = 0.1;
const MAX_FALLBACK_SCALE: f32 = 2.0;

/// Line height must stay positive for the run to lay out at all.
const MIN_LINE_HEIGHT_EM: f32 = 0.05;

/// How much taller than the configured font's capitals a colour glyph is drawn.
///
/// Emoji read as a mark rather than a letter, so matching cap height exactly
/// leaves them looking undersized next to the text. Ghostty and Windows
/// Terminal both draw them slightly proud of the capitals.
const COLOR_CAP_MULTIPLE: f32 = 1.15;

/// How far a colour glyph may exceed its cell, as a multiple of the cell width.
///
/// Colour emoji are drawn on a square canvas, so matching the height of the
/// text beside them needs slightly more width than one cell. The allowance is
/// bounded so a wide glyph is still reined in rather than painting over its
/// neighbours.
const COLOR_WIDTH_ALLOWANCE: f32 = 1.4;

/// Cap height assumed when a face does not publish one.
const FALLBACK_CAP_RATIO: f32 = 0.7;

/// A face substituted for one character the configured font does not cover.
///
/// A substitute is drawn from a different design at a different scale, so it is
/// normalized twice: sized to fill the cell, and placed on the configured
/// font's cap band. Without both, the glyph reads as a small sunken icon rather
/// than as part of the line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GlyphFallback {
    /// Family to shape this character with.
    pub(crate) family: &'static str,
    /// Multiplier applied to the terminal font size so the substituted glyph
    /// fills one cell without exceeding it.
    pub(crate) size_scale: f32,
    /// Weight of the matched face.
    ///
    /// A substitute must be requested at a weight it actually ships. Shaping
    /// drops the family entirely rather than relaxing the weight, so asking a
    /// single-weight face such as a colour emoji font for bold silently falls
    /// through to some other family and draws the wrong symbol.
    pub(crate) weight: u16,
    /// Whether the substitute draws a colour glyph.
    ///
    /// A colour glyph is allowed to exceed its cell slightly so it can match the
    /// height of the text beside it, so its run must not be clipped.
    pub(crate) color: bool,
    /// Line height for this run, as a multiple of the terminal font size.
    ///
    /// Shaping places the baseline at `line_height / 2 + (ascent + descent) / 2`
    /// using the run's own face, so a substitute with different vertical
    /// metrics lands on a different baseline than the text beside it. Choosing
    /// the line height puts it back, and is the only vertical lever the text
    /// API exposes. Requires the run to be laid out top-aligned; centering the
    /// paragraph in the cell cancels the term out.
    pub(crate) line_height_em: f32,
}

static FONT_DATABASE: OnceLock<Database> = OnceLock::new();

/// Publishes the system font database for later measurement. The first call
/// wins, so repeated discovery stays cheap.
///
/// Installing is deliberately explicit rather than a side effect of font
/// discovery: unit tests measure against their own database and must not have
/// process-wide metrics change under them.
pub(crate) fn install_database(database: Database) {
    let _ = FONT_DATABASE.set(database);
}

fn database() -> Option<&'static Database> {
    FONT_DATABASE.get()
}

type RatioCache = RwLock<HashMap<(String, u16), Option<f32>>>;
type FallbackCache = RwLock<HashMap<(String, u16, char, bool), Option<GlyphFallback>>>;

/// U+FE0E, which asks for the text rendering of an emoji-capable codepoint.
const TEXT_PRESENTATION_SELECTOR: char = '\u{FE0E}';

fn ratio_cache() -> &'static RatioCache {
    static CACHE: OnceLock<RatioCache> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn fallback_cache() -> &'static FallbackCache {
    static CACHE: OnceLock<FallbackCache> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn cache_key(family: Option<&str>, weight: u16) -> (String, u16) {
    (family.map(str::to_lowercase).unwrap_or_default(), weight)
}

/// Measured advance width of `family` as a fraction of the em square.
///
/// Returns `None` when the face cannot be resolved or measured, leaving the
/// caller to apply [`FALLBACK_ADVANCE_RATIO`].
pub(crate) fn advance_ratio(family: Option<&str>, weight: u16) -> Option<f32> {
    let key = cache_key(family, weight);
    if let Some(cached) = ratio_cache()
        .read()
        .expect("font metric cache should be readable")
        .get(&key)
    {
        return *cached;
    }

    let measured = database().and_then(|database| measure_advance_ratio(database, family, weight));
    ratio_cache()
        .write()
        .expect("font metric cache should be writable")
        .insert(key, measured);
    measured
}

/// Concrete family selected by the platform for the generic monospace role.
///
/// Iced's shaper assigns its own cross-platform default to `Font::MONOSPACE`,
/// which can differ from the family fontconfig selected in this database. The
/// terminal must shape the same face it measures or a fixed-width run can lose
/// its final glyph at the clip boundary.
pub(crate) fn system_monospace_family() -> Option<&'static str> {
    resolved_family_name(database()?, None, 400)
}

/// Chooses a face for `character` when the configured font does not cover it.
///
/// Returns `None` in the common case where the configured font has the glyph.
/// Otherwise the returned family is preferred over whatever the shaper would
/// pick on its own, because an arbitrary proportional fallback overflows the
/// cell and is then clipped into an unreadable sliver.
pub(crate) fn glyph_fallback(
    family: Option<&str>,
    weight: u16,
    grapheme: &str,
    cell_ratio: f32,
    line_height_ratio: f32,
) -> Option<GlyphFallback> {
    let character = grapheme.chars().next()?;
    // U+FE0E asks for the text rendering of an emoji-capable codepoint. Nothing
    // else in the grapheme changes which face should draw it.
    let prefer_color = !grapheme.contains(TEXT_PRESENTATION_SELECTOR);

    let (family_key, weight_key) = cache_key(family, weight);
    let key = (family_key, weight_key, character, prefer_color);
    if let Some(cached) = fallback_cache()
        .read()
        .expect("glyph fallback cache should be readable")
        .get(&key)
    {
        return *cached;
    }

    let resolved = database().and_then(|database| {
        resolve_glyph_fallback(
            database,
            family,
            weight,
            character,
            prefer_color,
            cell_ratio,
            line_height_ratio,
        )
    });
    fallback_cache()
        .write()
        .expect("glyph fallback cache should be writable")
        .insert(key, resolved);
    resolved
}

fn measure_advance_ratio(database: &Database, family: Option<&str>, weight: u16) -> Option<f32> {
    let id = resolve_face(database, family, weight)?;
    database.with_face_data(id, face_advance_ratio)?
}

fn face_advance_ratio(data: &[u8], index: u32) -> Option<f32> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    let units_per_em = f32::from(face.units_per_em());
    if units_per_em <= 0.0 {
        return None;
    }

    let advance = PROBE_CHARS.iter().find_map(|probe| {
        let glyph = face.glyph_index(*probe)?;
        face.glyph_hor_advance(glyph).filter(|advance| *advance > 0)
    })?;

    let ratio = f32::from(advance) / units_per_em;
    (MIN_ADVANCE_RATIO..=MAX_ADVANCE_RATIO)
        .contains(&ratio)
        .then_some(ratio)
}

fn resolve_face(database: &Database, family: Option<&str>, weight: u16) -> Option<fontdb::ID> {
    let requested = [family.map_or(Family::Monospace, Family::Name)];
    let query = Query {
        families: &requested,
        weight: Weight(weight),
        stretch: Stretch::Normal,
        style: Style::Normal,
    };
    if let Some(id) = database.query(&query) {
        return Some(id);
    }

    // `Family::Monospace` only resolves when the platform declared a generic
    // monospace family. Fall back to the first face that reports being one.
    if family.is_none() {
        return database
            .faces()
            .find(|face| face.monospaced)
            .map(|face| face.id);
    }
    None
}

fn resolved_family_name<'a>(
    database: &'a Database,
    family: Option<&str>,
    weight: u16,
) -> Option<&'a str> {
    database
        .face(resolve_face(database, family, weight)?)
        .map(primary_family)
}

fn resolve_glyph_fallback(
    database: &Database,
    family: Option<&str>,
    weight: u16,
    character: char,
    prefer_color: bool,
    cell_ratio: f32,
    line_height_ratio: f32,
) -> Option<GlyphFallback> {
    let configured = resolve_face(database, family, weight);
    if let Some(id) = configured
        && database
            .with_face_data(id, |data, index| face_covers(data, index, character))
            .unwrap_or(false)
    {
        return None;
    }

    let covering = covering_face(database, character, weight, prefer_color)?;
    let primary = configured
        .and_then(|id| database.with_face_data(id, face_vertical_metrics).flatten())
        .unwrap_or_default();

    let size_scale = fit_scale(
        cell_ratio,
        primary.cap_height,
        covering.ink,
        covering.advance,
        covering.color,
    );
    Some(GlyphFallback {
        family: intern_font_family(&covering.family),
        weight: covering.weight,
        color: covering.color,
        size_scale,
        line_height_em: cap_band_line_height(
            line_height_ratio,
            primary,
            covering.vertical,
            covering.ink,
            size_scale,
        ),
    })
}

/// Scales a substitute so its ink fills one cell without exceeding it, and
/// never grows taller than the configured font's capitals.
fn fit_scale(
    cell_ratio: f32,
    cap_height: f32,
    ink: GlyphInk,
    advance: Option<f32>,
    color: bool,
) -> f32 {
    let height = ink.y_max - ink.y_min;
    let allowance = if color { COLOR_WIDTH_ALLOWANCE } else { 1.0 };
    let target_height = cap_height * if color { COLOR_CAP_MULTIPLE } else { 1.0 };
    // The shaper lays a run out by advance, not by ink, and cuts the glyph at
    // the edge of that box. A face whose ink is narrow inside a wide advance —
    // a symbol carried by a text face — therefore scales far past the cell when
    // only the ink is fitted, and loses its tail to the cut. Bounding by the
    // advance keeps the whole glyph inside the box it is drawn in.
    let by_advance = advance
        .filter(|advance| *advance > 0.0)
        .map(|advance| cell_ratio * allowance / advance);
    let by_width = (ink.width > 0.0).then(|| cell_ratio * allowance / ink.width);
    let by_height = (height > 0.0).then(|| target_height / height);
    let fitted = [by_advance, by_width, by_height]
        .into_iter()
        .flatten()
        .fold(f32::INFINITY, f32::min);
    if fitted.is_finite() { fitted } else { 1.0 }.clamp(MIN_FALLBACK_SCALE, MAX_FALLBACK_SCALE)
}

/// Advance of one glyph in a face, in em units.
fn glyph_advance(data: &[u8], index: u32, character: char) -> Option<f32> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    let units_per_em = f32::from(face.units_per_em());
    if units_per_em <= 0.0 {
        return None;
    }
    let glyph = face.glyph_index(character)?;
    Some(f32::from(face.glyph_hor_advance(glyph)?) / units_per_em)
}

/// Line height that lands a substitute's ink centered on the configured font's
/// cap band.
///
/// Shaping puts the baseline at `line_height / 2 + (ascent + descent) / 2`,
/// scaled by the run's own size, so solving that for the line height is what
/// moves the glyph. Everything is expressed in multiples of the terminal font
/// size.
fn cap_band_line_height(
    line_height_ratio: f32,
    primary: FaceVerticalMetrics,
    fallback: FaceVerticalMetrics,
    ink: GlyphInk,
    size_scale: f32,
) -> f32 {
    let primary_baseline = line_height_ratio / 2.0 + primary.half_vertical_span();
    let ink_center = (ink.y_max + ink.y_min) / 2.0 * size_scale;
    let target_baseline = primary_baseline - primary.cap_height / 2.0 + ink_center;
    let line_height = 2.0 * (target_baseline - fallback.half_vertical_span() * size_scale);
    line_height.max(MIN_LINE_HEIGHT_EM)
}

struct CoveringFace {
    family: String,
    weight: u16,
    vertical: FaceVerticalMetrics,
    ink: GlyphInk,
    /// Advance of this glyph in the covering face, in em units.
    advance: Option<f32>,
    color: bool,
}

fn covering_face(
    database: &Database,
    character: char,
    weight: u16,
    prefer_color: bool,
) -> Option<CoveringFace> {
    let order = substitution_order(database, weight);
    // A color face wins outright when it has this character. Ghostty and Windows
    // Terminal both draw an emoji-capable codepoint from the platform's color
    // font even with no variation selector present, and an outline substitute
    // next to that reads as the wrong symbol rather than the same one restyled.
    if prefer_color
        && let Some(found) = order
            .iter()
            .find_map(|id| face_substitute(database, *id, character, true))
    {
        return Some(found);
    }

    order
        .iter()
        .find_map(|id| face_substitute(database, *id, character, false))
}

fn face_substitute(
    database: &Database,
    id: fontdb::ID,
    character: char,
    color_only: bool,
) -> Option<CoveringFace> {
    let vertical = database
        .with_face_data(id, face_vertical_metrics)
        .flatten()?;
    let (ink, color) = database
        .with_face_data(id, |data, index| {
            glyph_ink(data, index, character, color_only)
        })
        .flatten()?;
    let advance = database
        .with_face_data(id, |data, index| glyph_advance(data, index, character))
        .flatten();
    let info = database.face(id)?;
    Some(CoveringFace {
        family: primary_family(info).to_owned(),
        weight: info.weight.0,
        vertical,
        ink,
        advance,
        color,
    })
}

/// Faces to try when substituting, most preferred first.
///
/// The platform's sans and monospace families lead, so a substitution matches
/// what other applications on the same system show. Serif is deliberately not
/// preferred: a serif symbol reads as foreign next to terminal text, and these
/// generics do not always agree with the platform's own fallback order, so it
/// would win purely by being third. Every remaining face follows in name order,
/// so a system whose generics miss the character still resolves the same way
/// every run.
fn substitution_order(database: &Database, weight: u16) -> Vec<fontdb::ID> {
    let mut seen = HashSet::new();
    let mut order = Vec::new();
    for generic in [Family::SansSerif, Family::Monospace] {
        let query = Query {
            families: &[generic],
            weight: Weight(weight),
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        if let Some(id) = database.query(&query)
            && seen.insert(id)
        {
            order.push(id);
        }
    }

    let mut remaining: Vec<_> = database
        .faces()
        .filter(|face| face.style == Style::Normal)
        .collect();
    remaining.sort_by_key(|face| primary_family(face).to_lowercase());
    for face in remaining {
        if seen.insert(face.id) {
            order.push(face.id);
        }
    }
    order
}

/// Vertical metrics of a face, in em units.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FaceVerticalMetrics {
    ascent: f32,
    descent: f32,
    cap_height: f32,
}

impl FaceVerticalMetrics {
    /// The `(ascent + descent) / 2` term of the baseline placement. Descent is
    /// negative, so this is the offset of the baseline from the line's middle.
    fn half_vertical_span(self) -> f32 {
        (self.ascent + self.descent) / 2.0
    }
}

impl Default for FaceVerticalMetrics {
    fn default() -> Self {
        Self {
            ascent: 1.0,
            descent: -0.25,
            cap_height: FALLBACK_CAP_RATIO,
        }
    }
}

/// Ink box of one glyph, in em units, measured from the baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
struct GlyphInk {
    width: f32,
    y_min: f32,
    y_max: f32,
}

fn face_vertical_metrics(data: &[u8], index: u32) -> Option<FaceVerticalMetrics> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    let units_per_em = f32::from(face.units_per_em());
    if units_per_em <= 0.0 {
        return None;
    }

    // Shaping uses hhea, not the OS/2 typographic metrics.
    let cap_height = face
        .capital_height()
        .map(|height| f32::from(height) / units_per_em)
        .filter(|height| *height > 0.0)
        .or_else(|| {
            let glyph = face.glyph_index('M')?;
            let bounds = face.glyph_bounding_box(glyph)?;
            Some(f32::from(bounds.y_max) / units_per_em)
        })
        .filter(|height| *height > 0.0)
        .unwrap_or(FALLBACK_CAP_RATIO);

    Some(FaceVerticalMetrics {
        ascent: f32::from(face.ascender()) / units_per_em,
        descent: f32::from(face.descender()) / units_per_em,
        cap_height,
    })
}

/// Ink box of one glyph, from its outline or, for a color bitmap face, from its
/// raster strike.
///
/// `color_only` restricts the answer to color glyphs, which is how a color face
/// is distinguished from an outline one. The outline pass deliberately skips a
/// raster strike even when the same face ships both forms; otherwise U+FE0E
/// text presentation can resolve right back to the color glyph.
fn glyph_ink(
    data: &[u8],
    index: u32,
    character: char,
    color_only: bool,
) -> Option<(GlyphInk, bool)> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    let units_per_em = f32::from(face.units_per_em());
    if units_per_em <= 0.0 {
        return None;
    }

    let glyph = face.glyph_index(character)?;
    if color_only {
        return raster_ink(&face, glyph).map(|raster| (raster, true));
    }

    let bounds = face.glyph_bounding_box(glyph)?;
    Some((
        GlyphInk {
            width: (f32::from(bounds.x_max) - f32::from(bounds.x_min)) / units_per_em,
            y_min: f32::from(bounds.y_min) / units_per_em,
            y_max: f32::from(bounds.y_max) / units_per_em,
        },
        false,
    ))
}

/// Ink box of a color bitmap glyph, converted from its strike's pixel grid.
///
/// Emoji strikes are padded with transparent margins, so the stored bitmap is
/// measurably larger than the mark it draws. Fitting the padded box leaves the
/// visible glyph short of the text beside it, so the opaque region is measured
/// instead when the image can be decoded.
fn raster_ink(face: &ttf_parser::Face<'_>, glyph: ttf_parser::GlyphId) -> Option<GlyphInk> {
    // Ask for an oversized strike so the face returns its largest.
    let image = face.glyph_raster_image(glyph, u16::MAX)?;
    let pixels_per_em = f32::from(image.pixels_per_em);
    if pixels_per_em <= 0.0 {
        return None;
    }

    let (width, height) = (u32::from(image.width), u32::from(image.height));
    let opaque = (image.format == ttf_parser::RasterImageFormat::PNG)
        .then(|| opaque_bounds(image.data, width, height))
        .flatten()
        .unwrap_or((0, 0, width, height));
    let (left, top, right, bottom) = opaque;

    // Bitmap rows run downward from the top of the image; the glyph's own
    // origin sits at the bottom edge.
    let bottom_edge = f32::from(image.y) / pixels_per_em;
    let scale = |value: u32| value as f32 / pixels_per_em;
    Some(GlyphInk {
        width: scale(right.saturating_sub(left)),
        y_min: bottom_edge + scale(height.saturating_sub(bottom)),
        y_max: bottom_edge + scale(height.saturating_sub(top)),
    })
}

/// Bounding box of the non-transparent pixels of a PNG, as `(left, top, right,
/// bottom)`. `None` when the image cannot be decoded or carries no alpha.
fn opaque_bounds(data: &[u8], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let mut decoder = png::Decoder::new(data);
    // Colour strikes are commonly palette images with a transparency chunk, so
    // the palette has to be expanded before there is an alpha channel to read.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    if info.width != width || info.height != height {
        return None;
    }
    let channels = info.color_type.samples();
    if !matches!(
        info.color_type,
        png::ColorType::Rgba | png::ColorType::GrayscaleAlpha
    ) {
        return None;
    }
    let bytes_per_sample = usize::from(info.bit_depth as u8).div_ceil(8);
    let alpha_offset = (channels - 1) * bytes_per_sample;

    let (mut left, mut top) = (width, height);
    let (mut right, mut bottom) = (0, 0);
    for y in 0..height {
        for x in 0..width {
            let pixel = (y as usize * width as usize + x as usize) * channels * bytes_per_sample;
            let alpha = buffer.get(pixel + alpha_offset).copied().unwrap_or(0);
            if alpha > 0 {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }

    (right > left && bottom > top).then_some((left, top, right, bottom))
}

fn primary_family(face: &fontdb::FaceInfo) -> &str {
    face.families
        .first()
        .map_or(face.post_script_name.as_str(), |(family, _)| {
            family.as_str()
        })
}

fn face_covers(data: &[u8], index: u32, character: char) -> bool {
    ttf_parser::Face::parse(data, index)
        .ok()
        .and_then(|face| face.glyph_index(character))
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a private database so measurement is exercised without publishing
    /// process-wide metrics that other tests depend on.
    fn system_database() -> Database {
        let mut database = Database::new();
        database.load_system_fonts();
        database
    }

    fn first_monospace_family(database: &Database) -> Option<String> {
        let mut families: Vec<_> = database
            .faces()
            .filter(|face| face.monospaced && face.style == Style::Normal)
            .map(|face| primary_family(face).to_owned())
            .collect();
        families.sort();
        families.into_iter().next()
    }

    /// FiraCode Nerd Font Mono Bold, the configured terminal face.
    const PRIMARY: FaceVerticalMetrics = FaceVerticalMetrics {
        ascent: 0.923_076_9,
        descent: -0.307_692_3,
        cap_height: 0.710_769_2,
    };
    /// DejaVu Sans Bold, the substitute chosen for U+26A0.
    const SUBSTITUTE: FaceVerticalMetrics = FaceVerticalMetrics {
        ascent: 0.928,
        descent: -0.236,
        cap_height: 0.729,
    };
    const WARNING_INK: GlyphInk = GlyphInk {
        width: 0.798_828_1,
        y_min: 0.0,
        y_max: 0.728_515_6,
    };
    const CELL_RATIO: f32 = 0.615_384_6;
    const LINE_HEIGHT_RATIO: f32 = 1.15;

    /// Where shaping puts the baseline, in multiples of the terminal font size.
    /// Verified against cosmic-text, which Iced shapes through.
    fn shaped_baseline(line_height_em: f32, face: FaceVerticalMetrics, size_scale: f32) -> f32 {
        line_height_em / 2.0 + face.half_vertical_span() * size_scale
    }

    #[test]
    fn a_substitute_fills_its_cell_without_exceeding_it() {
        let scale = fit_scale(CELL_RATIO, PRIMARY.cap_height, WARNING_INK, None, false);
        let width = WARNING_INK.width * scale;
        let height = (WARNING_INK.y_max - WARNING_INK.y_min) * scale;
        assert!(
            (width - CELL_RATIO).abs() < 1e-4,
            "ink width {width} should fill the {CELL_RATIO} cell"
        );
        assert!(
            height <= PRIMARY.cap_height + 1e-4,
            "ink height {height} should not exceed cap height {}",
            PRIMARY.cap_height
        );
    }

    #[test]
    fn a_squat_substitute_is_bounded_by_cap_height_not_cell_width() {
        // A wide, short glyph would otherwise be stretched past the capitals.
        let squat = GlyphInk {
            width: 0.2,
            y_min: 0.0,
            y_max: 0.5,
        };
        let scale = fit_scale(CELL_RATIO, PRIMARY.cap_height, squat, None, false);
        assert!((scale - PRIMARY.cap_height / 0.5).abs() < 1e-4);
        assert!(squat.width * scale <= CELL_RATIO);
    }

    #[test]
    fn degenerate_ink_cannot_collapse_or_explode_a_substitute() {
        let empty = GlyphInk {
            width: 0.0,
            y_min: 0.0,
            y_max: 0.0,
        };
        assert_eq!(
            fit_scale(CELL_RATIO, PRIMARY.cap_height, empty, None, false),
            1.0
        );

        let hairline = GlyphInk {
            width: 0.000_01,
            y_min: 0.0,
            y_max: 0.000_01,
        };
        assert_eq!(
            fit_scale(CELL_RATIO, PRIMARY.cap_height, hairline, None, false),
            MAX_FALLBACK_SCALE
        );
    }

    /// U+23F5 in FreeMono, the substitute chosen when a Nerd Font is configured
    /// but does not cover the character: a small triangle inside a full-width
    /// monospace advance. Fitting its ink alone scales it half again past the
    /// cell, and the shaper then cuts the point off.
    #[test]
    fn a_substitute_never_outgrows_the_advance_it_is_drawn_in() {
        let narrow_ink = GlyphInk {
            width: 0.38,
            y_min: 0.0,
            y_max: 0.45,
        };
        let advance = 0.6;
        let scale = fit_scale(
            CELL_RATIO,
            PRIMARY.cap_height,
            narrow_ink,
            Some(advance),
            false,
        );
        assert!(
            advance * scale <= CELL_RATIO + 1e-4,
            "advance {} should stay inside one cell",
            advance * scale
        );

        let unbounded = fit_scale(CELL_RATIO, PRIMARY.cap_height, narrow_ink, None, false);
        assert!(
            scale < unbounded,
            "the advance should bind before the ink does, got {scale} against {unbounded}"
        );
    }

    #[test]
    fn a_substitute_lands_centered_on_the_configured_cap_band() {
        let scale = fit_scale(CELL_RATIO, PRIMARY.cap_height, WARNING_INK, None, false);
        let line_height_em =
            cap_band_line_height(LINE_HEIGHT_RATIO, PRIMARY, SUBSTITUTE, WARNING_INK, scale);

        let primary_baseline = shaped_baseline(LINE_HEIGHT_RATIO, PRIMARY, 1.0);
        let band_center = primary_baseline - PRIMARY.cap_height / 2.0;

        let baseline = shaped_baseline(line_height_em, SUBSTITUTE, scale);
        let ink_center = baseline - (WARNING_INK.y_max + WARNING_INK.y_min) / 2.0 * scale;

        assert!(
            (ink_center - band_center).abs() < 1e-4,
            "ink center {ink_center} should sit on cap band center {band_center}"
        );
    }

    #[test]
    fn a_substitute_with_matching_metrics_keeps_the_cell_line_height() {
        // A face whose vertical metrics match the configured font and whose ink
        // already fills the cap band needs no correction.
        let ink = GlyphInk {
            width: CELL_RATIO,
            y_min: 0.0,
            y_max: PRIMARY.cap_height,
        };
        let line_height_em = cap_band_line_height(LINE_HEIGHT_RATIO, PRIMARY, PRIMARY, ink, 1.0);
        assert!(
            (line_height_em - LINE_HEIGHT_RATIO).abs() < 1e-4,
            "expected {LINE_HEIGHT_RATIO}, got {line_height_em}"
        );
    }

    #[test]
    fn line_height_stays_positive_for_a_pathological_face() {
        let absurd = FaceVerticalMetrics {
            ascent: 40.0,
            descent: 0.0,
            cap_height: 0.7,
        };
        let line_height_em =
            cap_band_line_height(LINE_HEIGHT_RATIO, PRIMARY, absurd, WARNING_INK, 1.0);
        assert!(line_height_em >= MIN_LINE_HEIGHT_EM);
    }

    #[test]
    fn malformed_face_data_yields_no_metrics() {
        assert_eq!(face_advance_ratio(&[0, 1, 2, 3], 0), None);
        assert!(!face_covers(&[0, 1, 2, 3], 0, 'M'));
    }

    #[test]
    fn proportional_ratios_are_rejected_as_cell_widths() {
        assert!(!(MIN_ADVANCE_RATIO..=MAX_ADVANCE_RATIO).contains(&0.25));
        assert!(!(MIN_ADVANCE_RATIO..=MAX_ADVANCE_RATIO).contains(&1.5));
        assert!((MIN_ADVANCE_RATIO..=MAX_ADVANCE_RATIO).contains(&FALLBACK_ADVANCE_RATIO));
    }

    #[test]
    fn cache_keys_ignore_family_casing_but_not_identity() {
        assert_eq!(
            cache_key(Some("Fira Code"), 400),
            cache_key(Some("FIRA CODE"), 400)
        );
        assert_ne!(cache_key(Some("Fira Code"), 400), cache_key(None, 400));
        assert_ne!(
            cache_key(Some("Fira Code"), 400),
            cache_key(Some("Fira Code"), 700)
        );
    }

    #[test]
    fn unknown_families_are_not_measured() {
        let database = system_database();
        assert_eq!(
            measure_advance_ratio(&database, Some("No Such Family Anywhere"), 400),
            None
        );
    }

    #[test]
    fn installed_monospace_faces_measure_within_the_cell_range() {
        let database = system_database();
        let Some(family) = first_monospace_family(&database) else {
            // A font-less build host cannot exercise measurement.
            return;
        };

        let Some(ratio) = measure_advance_ratio(&database, Some(&family), 400) else {
            return;
        };
        assert!(
            (MIN_ADVANCE_RATIO..=MAX_ADVANCE_RATIO).contains(&ratio),
            "{family} measured {ratio}, outside the monospace cell range"
        );
    }

    #[test]
    fn generic_monospace_resolves_to_the_named_face_that_is_measured() {
        let database = system_database();
        let Some(family) = resolved_family_name(&database, None, 400) else {
            // A font-less build host cannot exercise generic resolution.
            return;
        };

        assert_eq!(
            measure_advance_ratio(&database, None, 400),
            measure_advance_ratio(&database, Some(family), 400),
            "the named rendering face must preserve the generic face's cell metrics"
        );
    }

    /// A colour face for U+26A0, if this system has one.
    fn colour_substitute(database: &Database) -> Option<CoveringFace> {
        covering_face(database, '\u{26A0}', 700, true).filter(|found| found.color)
    }

    #[test]
    fn an_emoji_capable_character_prefers_a_colour_face() {
        let database = system_database();
        let Some(colour) = colour_substitute(&database) else {
            // No colour emoji font installed on this build host.
            return;
        };
        // Some platform families ship both raster and outline forms, so the
        // rendering kind is the contract rather than a different family name.
        let outline = covering_face(&database, '\u{26A0}', 700, false)
            .expect("some face should cover the warning sign");
        assert!(
            colour.color,
            "color preference should choose a raster strike"
        );
        assert!(
            !outline.color,
            "outline preference should skip raster strikes"
        );
    }

    #[test]
    fn a_substitute_reports_the_weight_its_own_face_ships() {
        let database = system_database();
        let Some(colour) = colour_substitute(&database) else {
            return;
        };
        // Requesting bold from a single-weight face makes shaping drop the
        // family and silently draw another one, so the face's own weight is
        // what must be reported back.
        assert_ne!(
            colour.weight, 700,
            "{} should report its own weight, not the requested bold",
            colour.family
        );
    }

    #[test]
    fn the_text_presentation_selector_keeps_an_outline_face() {
        let database = system_database();
        if colour_substitute(&database).is_none() {
            return;
        }
        let text = covering_face(&database, '\u{26A0}', 700, false)
            .expect("some face should cover the warning sign");
        assert!(
            !text.color,
            "text presentation should choose an outline glyph"
        );
    }

    #[test]
    fn a_colour_glyph_matches_cap_height_within_its_allowance() {
        // Colour emoji sit on a square canvas, so matching the height of the
        // text beside them needs slightly more than one cell.
        let square = GlyphInk {
            width: 1.0,
            y_min: 0.0,
            y_max: 1.0,
        };
        let scale = fit_scale(CELL_RATIO, PRIMARY.cap_height, square, None, true);
        let height = (square.y_max - square.y_min) * scale;
        let width = square.width * scale;
        assert!(
            height >= PRIMARY.cap_height,
            "colour glyph should reach at least cap height, got {height}"
        );
        assert!(
            width <= CELL_RATIO * COLOR_WIDTH_ALLOWANCE + 1e-4,
            "colour glyph {width} should stay inside its allowance"
        );
        // The same glyph as an outline stays inside one cell.
        let outline = fit_scale(CELL_RATIO, PRIMARY.cap_height, square, None, false);
        assert!(square.width * outline <= CELL_RATIO + 1e-4);
    }

    #[test]
    fn a_colour_glyph_is_drawn_proud_of_the_capitals() {
        let square = GlyphInk {
            width: 1.0,
            y_min: 0.0,
            y_max: 1.0,
        };
        let scale = fit_scale(CELL_RATIO, PRIMARY.cap_height, square, None, true);
        let height = (square.y_max - square.y_min) * scale;
        assert!(
            height > PRIMARY.cap_height,
            "a colour glyph should stand slightly above cap height, got {height}"
        );
        assert!(
            (height - PRIMARY.cap_height * COLOR_CAP_MULTIPLE).abs() < 1e-4,
            "expected {} got {height}",
            PRIMARY.cap_height * COLOR_CAP_MULTIPLE
        );
    }

    #[test]
    fn a_wide_colour_glyph_is_still_reined_in() {
        let wide = GlyphInk {
            width: 4.0,
            y_min: 0.0,
            y_max: 1.0,
        };
        let scale = fit_scale(CELL_RATIO, PRIMARY.cap_height, wide, None, true);
        assert!(wide.width * scale <= CELL_RATIO * COLOR_WIDTH_ALLOWANCE + 1e-4);
    }

    #[test]
    fn transparent_padding_is_cropped_from_a_colour_strike() {
        // A 4x4 image whose only opaque pixels form a 2x2 block at (1,1).
        let mut pixels = vec![0u8; 4 * 4 * 4];
        for (x, y) in [(1, 1), (2, 1), (1, 2), (2, 2)] {
            let i = (y * 4 + x) * 4;
            pixels[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 4, 4);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(&pixels).expect("data");
        }
        assert_eq!(opaque_bounds(&encoded, 4, 4), Some((1, 1, 3, 3)));
    }

    #[test]
    fn a_palette_strike_with_transparency_is_cropped() {
        // Colour strikes are commonly indexed rather than RGBA, so the palette
        // has to be expanded before there is an alpha channel to measure.
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 4, 4);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_palette(vec![0, 0, 0, 255, 255, 255]);
            // Index 0 transparent, index 1 opaque.
            encoder.set_trns(vec![0, 255]);
            let mut writer = encoder.write_header().expect("header");
            #[rustfmt::skip]
            let indices = [
                0, 0, 0, 0,
                0, 1, 1, 0,
                0, 1, 1, 0,
                0, 0, 0, 0,
            ];
            writer.write_image_data(&indices).expect("data");
        }
        assert_eq!(opaque_bounds(&encoded, 4, 4), Some((1, 1, 3, 3)));
    }

    #[test]
    fn a_fully_transparent_strike_has_no_bounds() {
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(&[0; 2 * 2 * 4]).expect("data");
        }
        assert_eq!(opaque_bounds(&encoded, 2, 2), None);
    }

    #[test]
    fn a_covered_character_needs_no_fallback() {
        let database = system_database();
        let Some(family) = first_monospace_family(&database) else {
            return;
        };
        // Every usable terminal face covers ASCII.
        assert_eq!(
            resolve_glyph_fallback(
                &database,
                Some(&family),
                400,
                'M',
                true,
                FALLBACK_ADVANCE_RATIO,
                1.15
            ),
            None
        );
    }

    #[test]
    fn an_uncovered_character_falls_back_within_one_cell() {
        let database = system_database();
        if database.faces().next().is_none() {
            return;
        }
        // A private-use codepoint no ordinary face covers.
        let Some(fallback) = resolve_glyph_fallback(
            &database,
            Some("No Such Family Anywhere"),
            400,
            '\u{2500}',
            true,
            FALLBACK_ADVANCE_RATIO,
            1.15,
        ) else {
            return;
        };
        assert!(
            fallback.size_scale > 0.0 && fallback.size_scale <= 1.0,
            "fallback must fit one cell, got scale {}",
            fallback.size_scale
        );
        assert!(!fallback.family.is_empty());
    }
}
