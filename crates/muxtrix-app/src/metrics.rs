//! Measured terminal cell metrics and glyph fallback selection.
//!
//! The terminal grid places every run at a fixed multiple of one advance width.
//! That width must come from the face actually being rendered: a guessed ratio
//! drifts against the shaped text and clips the tail of every run. Faces are
//! measured once through `fontdb` and cached for the process.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use fontdb::{Database, Family, Query, Stretch, Style, Weight};

/// Advance ratio assumed when no face can be measured. Most monospace faces sit
/// near this value, so it degrades gracefully rather than collapsing the grid.
pub(crate) const FALLBACK_ADVANCE_RATIO: f32 = 0.6;

/// Ratios outside this range indicate a proportional or broken face rather than
/// a usable terminal cell.
const MIN_ADVANCE_RATIO: f32 = 0.3;
const MAX_ADVANCE_RATIO: f32 = 1.2;

/// Probed in order; the first glyph present decides the advance width.
const PROBE_CHARS: [char; 3] = ['M', '0', 'x'];

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

fn ratio_cache() -> &'static RatioCache {
    static CACHE: OnceLock<RatioCache> = OnceLock::new();
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
/// Given only the generic name, a shaper may answer with a default of its own
/// that differs from the family fontconfig selected in this database. The
/// terminal must shape the same face it measures or a fixed-width run can lose
/// its final glyph at the clip boundary.
pub(crate) fn system_monospace_family() -> Option<&'static str> {
    resolved_family_name(database()?, None, 400)
}

/// Concrete family selected by the platform for the generic sans-serif role.
///
/// Iced's `Font::DEFAULT` resolves through fontconfig to this family; GPUI,
/// given no family at all, falls back to a face of its own choosing, and the
/// two runtimes then set the same copy at visibly different widths. Naming
/// the family keeps the chrome's type identical across both.
pub(crate) fn system_sans_family() -> Option<&'static str> {
    let database = database()?;
    let query = Query {
        families: &[Family::SansSerif],
        weight: Weight(400),
        stretch: Stretch::Normal,
        style: Style::Normal,
    };
    database.face(database.query(&query)?).map(primary_family)
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

fn primary_family(face: &fontdb::FaceInfo) -> &str {
    face.families
        .first()
        .map_or(face.post_script_name.as_str(), |(family, _)| {
            family.as_str()
        })
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
}
