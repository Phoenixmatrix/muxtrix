use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use iced::{Font, font};
use muxtrix_control::Agent;
use serde::{Deserialize, Serialize};

use crate::metrics;
use crate::themes::TerminalThemeId;

const CONFIG_OVERRIDE: &str = "MUXTRIX_CONFIG_PATH";
#[cfg(not(target_os = "macos"))]
const POINTS_TO_PIXELS: f32 = 96.0 / 72.0;
#[cfg(target_os = "macos")]
const POINTS_TO_PIXELS: f32 = 1.0;
/// Size the relative type scale is anchored to. Interface base point sizes are
/// expressed against this reference, not against the current setting, so every
/// size moves together when the setting changes.
const UI_TYPE_SCALE_REFERENCE: f32 = 14.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Appearance {
    #[default]
    System,
    Dark,
    Light,
}

impl Appearance {
    pub(crate) const ALL: [Self; 3] = [Self::System, Self::Dark, Self::Light];
}

impl fmt::Display for Appearance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::System => "System",
            Self::Dark => "Dark",
            Self::Light => "Light",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum WindowsShellBackend {
    #[default]
    Native,
    Wsl,
}

/// How the fleet projects the selected workspace's panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FleetView {
    #[default]
    Tabs,
    Agents,
    Repos,
}

impl fmt::Display for FleetView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Tabs => "Tabs",
            Self::Agents => "Agents",
            Self::Repos => "Repos",
        })
    }
}

impl WindowsShellBackend {
    #[cfg(target_os = "windows")]
    pub(crate) const ALL: [Self; 2] = [Self::Native, Self::Wsl];
}

impl fmt::Display for WindowsShellBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Native => "Windows PowerShell",
            Self::Wsl => "WSL",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FontWeight {
    Thin,
    ExtraLight,
    Light,
    #[default]
    Normal,
    Medium,
    Semibold,
    Bold,
    ExtraBold,
    Black,
}

impl FontWeight {
    pub(crate) const ALL: [Self; 9] = [
        Self::Thin,
        Self::ExtraLight,
        Self::Light,
        Self::Normal,
        Self::Medium,
        Self::Semibold,
        Self::Bold,
        Self::ExtraBold,
        Self::Black,
    ];

    pub(crate) const fn iced(self) -> font::Weight {
        match self {
            Self::Thin => font::Weight::Thin,
            Self::ExtraLight => font::Weight::ExtraLight,
            Self::Light => font::Weight::Light,
            Self::Normal => font::Weight::Normal,
            Self::Medium => font::Weight::Medium,
            Self::Semibold => font::Weight::Semibold,
            Self::Bold => font::Weight::Bold,
            Self::ExtraBold => font::Weight::ExtraBold,
            Self::Black => font::Weight::Black,
        }
    }

    pub(crate) const fn bold_variant(self) -> font::Weight {
        match self {
            Self::Thin | Self::ExtraLight | Self::Light | Self::Normal => font::Weight::Bold,
            Self::Medium | Self::Semibold => font::Weight::ExtraBold,
            Self::Bold | Self::ExtraBold | Self::Black => font::Weight::Black,
        }
    }

    pub(crate) const fn numeric(self) -> u16 {
        match self {
            Self::Thin => 100,
            Self::ExtraLight => 200,
            Self::Light => 300,
            Self::Normal => 400,
            Self::Medium => 500,
            Self::Semibold => 600,
            Self::Bold => 700,
            Self::ExtraBold => 800,
            Self::Black => 900,
        }
    }

    const fn from_numeric(weight: u16) -> Self {
        match weight {
            0..=149 => Self::Thin,
            150..=249 => Self::ExtraLight,
            250..=349 => Self::Light,
            350..=449 => Self::Normal,
            450..=549 => Self::Medium,
            550..=649 => Self::Semibold,
            650..=749 => Self::Bold,
            750..=849 => Self::ExtraBold,
            _ => Self::Black,
        }
    }
}

impl fmt::Display for FontWeight {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Thin => "Thin",
            Self::ExtraLight => "Extra light",
            Self::Light => "Light",
            Self::Normal => "Regular",
            Self::Medium => "Medium",
            Self::Semibold => "Semibold",
            Self::Bold => "Bold",
            Self::ExtraBold => "Extra bold",
            Self::Black => "Black",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalFont {
    SystemMonospace,
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UiFont {
    SystemSans,
    Named(String),
}

impl UiFont {
    pub(crate) fn named(family: impl Into<String>) -> Self {
        Self::Named(family.into())
    }

    pub(crate) fn iced(&self) -> Font {
        match self {
            Self::SystemSans => Font::DEFAULT,
            Self::Named(family) => Font::with_name(intern_font_family(family)),
        }
    }

    pub(crate) fn family_name(&self) -> Option<&str> {
        match self {
            Self::SystemSans => None,
            Self::Named(family) => Some(family),
        }
    }
}

impl fmt::Display for UiFont {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.family_name().unwrap_or("System sans serif"))
    }
}

impl Serialize for UiFont {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.family_name().unwrap_or("system-sans"))
    }
}

impl<'de> Deserialize<'de> for UiFont {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let family = String::deserialize(deserializer)?;
        Ok(if family == "system-sans" {
            Self::SystemSans
        } else {
            Self::named(family)
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InstalledFontCatalog {
    database: fontdb::Database,
    ui_fonts: Vec<UiFont>,
    ui_weights: BTreeMap<String, Vec<FontWeight>>,
    /// Family that "System sans serif" actually resolves to, so its real
    /// weights can be looked up like any named family's.
    system_ui_family: String,
    terminal_fonts: Vec<TerminalFont>,
    terminal_weights: BTreeMap<String, Vec<FontWeight>>,
}

impl InstalledFontCatalog {
    pub(crate) fn discover() -> Self {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();

        let mut ui_families = BTreeMap::new();
        let mut ui_weight_values: BTreeMap<String, Vec<u16>> = BTreeMap::new();
        let mut terminal_families = BTreeMap::new();
        let mut terminal_weight_values: BTreeMap<String, Vec<u16>> = BTreeMap::new();
        for face in database.faces() {
            for (family, _) in &face.families {
                let family = family.trim();
                if family.is_empty() {
                    continue;
                }
                let key = family.to_lowercase();
                ui_families
                    .entry(key.clone())
                    .or_insert_with(|| family.to_owned());
                if face.style == fontdb::Style::Normal {
                    ui_weight_values
                        .entry(key.clone())
                        .or_default()
                        .push(face.weight.0);
                }
                if face.monospaced {
                    terminal_families
                        .entry(key.clone())
                        .or_insert_with(|| family.to_owned());
                    if face.style == fontdb::Style::Normal {
                        terminal_weight_values
                            .entry(key)
                            .or_default()
                            .push(face.weight.0);
                    }
                }
            }
        }

        let ui_fonts = std::iter::once(UiFont::SystemSans)
            .chain(ui_families.into_values().map(UiFont::named))
            .collect();
        let ui_weights = ui_weight_values
            .into_iter()
            .map(|(family, weights)| (family, font_weight_choices(weights)))
            .collect();
        let terminal_fonts = std::iter::once(TerminalFont::SystemMonospace)
            .chain(terminal_families.into_values().map(TerminalFont::named))
            .collect();
        let terminal_weights = terminal_weight_values
            .into_iter()
            .map(|(family, weights)| (family, font_weight_choices(weights)))
            .collect();

        let system_ui_family = database.family_name(&fontdb::Family::SansSerif).to_owned();

        Self {
            database,
            ui_fonts,
            ui_weights,
            system_ui_family,
            terminal_fonts,
            terminal_weights,
        }
    }

    /// Publishes the discovered faces for cell measurement.
    ///
    /// Kept separate from discovery so tests can enumerate fonts without
    /// changing the metrics the rest of the process sees.
    pub(crate) fn install_metrics(&self) {
        metrics::install_database(self.database.clone());
    }

    pub(crate) fn ui_fonts(&self) -> Vec<UiFont> {
        self.ui_fonts.clone()
    }

    pub(crate) fn ui_weights(&self, font: &UiFont) -> Vec<FontWeight> {
        // "System sans serif" is a real installed family once fontconfig has
        // resolved it, so its weights are looked up like any other. Assuming a
        // full ramp here offered weights the face does not ship.
        let family = match font {
            UiFont::Named(family) => family.clone(),
            UiFont::SystemSans => self.system_ui_family.clone(),
        };
        self.ui_weights
            .get(&family.to_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    /// The installed weight closest to `desired` for `font`'s family.
    ///
    /// Shaping a run at a weight its family does not install does not fall back
    /// to that family's nearest face — it falls back to a *different family*
    /// that does have the weight. The run then changes typeface, and glyphs
    /// like `…` move off the baseline mid-rail. Snapping first keeps every
    /// emphasis level inside one family.
    pub(crate) fn nearest_ui_weight(&self, font: &UiFont, desired: FontWeight) -> FontWeight {
        nearest_weight(&self.ui_weights(font), desired)
    }

    pub(crate) fn terminal_fonts(&self) -> Vec<TerminalFont> {
        self.terminal_fonts.clone()
    }

    pub(crate) fn terminal_weights(&self, font: &TerminalFont) -> Vec<FontWeight> {
        let TerminalFont::Named(family) = font else {
            return vec![FontWeight::Normal, FontWeight::Bold];
        };
        self.terminal_weights
            .get(&family.to_lowercase())
            .cloned()
            .unwrap_or_default()
    }
}

impl TerminalFont {
    pub(crate) fn named(family: impl Into<String>) -> Self {
        Self::Named(family.into())
    }

    pub(crate) fn iced(&self) -> Font {
        self.iced_with_system_family(metrics::system_monospace_family())
    }

    fn iced_with_system_family(&self, system_family: Option<&str>) -> Font {
        match self {
            Self::SystemMonospace => system_family.map_or(Font::MONOSPACE, |family| {
                Font::with_name(intern_font_family(family))
            }),
            Self::Named(family) => Font::with_name(intern_font_family(family)),
        }
    }

    pub(crate) fn family_name(&self) -> Option<&str> {
        match self {
            Self::SystemMonospace => None,
            Self::Named(family) => Some(family),
        }
    }
}

/// Nearest Iced weight for a face's numeric weight.
pub(crate) const fn weight_from_numeric(weight: u16) -> font::Weight {
    match weight {
        0..=149 => font::Weight::Thin,
        150..=249 => font::Weight::ExtraLight,
        250..=349 => font::Weight::Light,
        350..=449 => font::Weight::Normal,
        450..=549 => font::Weight::Medium,
        550..=649 => font::Weight::Semibold,
        650..=749 => font::Weight::Bold,
        750..=849 => font::Weight::ExtraBold,
        _ => font::Weight::Black,
    }
}

/// Numeric weight for a resolved Iced weight, used when measuring a face.
pub(crate) const fn weight_numeric(weight: font::Weight) -> u16 {
    match weight {
        font::Weight::Thin => 100,
        font::Weight::ExtraLight => 200,
        font::Weight::Light => 300,
        font::Weight::Normal => 400,
        font::Weight::Medium => 500,
        font::Weight::Semibold => 600,
        font::Weight::Bold => 700,
        font::Weight::ExtraBold => 800,
        font::Weight::Black => 900,
    }
}

/// The entry of `installed` closest to `desired`, or `desired` when nothing is
/// known about the family.
///
/// Ties round *down*. A family with only two faces has to collapse one step of
/// the emphasis ladder either way; rounding down collapses Medium into Regular,
/// which two adjacent levels already distinguish by size and colour. Rounding
/// up would instead collapse Medium into Semibold — the weight that marks the
/// focused row — and focus is the signal that has nothing else to fall back on.
fn nearest_weight(installed: &[FontWeight], desired: FontWeight) -> FontWeight {
    if installed.is_empty() || installed.contains(&desired) {
        return desired;
    }
    installed
        .iter()
        .copied()
        .min_by_key(|weight| {
            (
                weight.numeric().abs_diff(desired.numeric()),
                u16::from(weight.numeric() > desired.numeric()),
            )
        })
        .unwrap_or(desired)
}

fn font_weight_choices(weights: impl IntoIterator<Item = u16>) -> Vec<FontWeight> {
    let weights: Vec<_> = weights.into_iter().collect();
    FontWeight::ALL
        .into_iter()
        .filter(|choice| {
            weights
                .iter()
                .copied()
                .any(|weight| FontWeight::from_numeric(weight) == *choice)
        })
        .collect()
}

impl fmt::Display for TerminalFont {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.family_name().unwrap_or("System monospace"))
    }
}

impl Serialize for TerminalFont {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.family_name().unwrap_or("system-monospace"))
    }
}

impl<'de> Deserialize<'de> for TerminalFont {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let family = String::deserialize(deserializer)?;
        Ok(match family.as_str() {
            "system-monospace" => Self::SystemMonospace,
            // Preserve settings written by the original fixed font list.
            "jet-brains-mono" => Self::named("JetBrains Mono"),
            "cascadia-mono" => Self::named("Cascadia Mono"),
            "fira-code" => Self::named("Fira Code"),
            "consolas" => Self::named("Consolas"),
            "sf-mono" => Self::named("SF Mono"),
            _ => Self::named(family),
        })
    }
}

#[cfg(test)]
fn terminal_font_choices<'a>(families: impl IntoIterator<Item = &'a str>) -> Vec<TerminalFont> {
    let mut installed = BTreeMap::new();
    for family in families {
        let family = family.trim();
        if !family.is_empty() {
            installed
                .entry(family.to_lowercase())
                .or_insert_with(|| family.to_owned());
        }
    }

    std::iter::once(TerminalFont::SystemMonospace)
        .chain(installed.into_values().map(TerminalFont::named))
        .collect()
}

pub(crate) fn intern_font_family(family: &str) -> &'static str {
    static FAMILIES: OnceLock<RwLock<HashMap<String, &'static str>>> = OnceLock::new();
    let families = FAMILIES.get_or_init(|| RwLock::new(HashMap::new()));
    if let Some(interned) = families
        .read()
        .expect("font family cache should be readable")
        .get(family)
    {
        return interned;
    }

    let mut families = families
        .write()
        .expect("font family cache should be writable");
    if let Some(interned) = families.get(family) {
        return interned;
    }
    let interned = Box::leak(family.to_owned().into_boxed_str());
    families.insert(family.to_owned(), interned);
    interned
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AppSettings {
    pub(crate) appearance: Appearance,
    pub(crate) show_status_bar: bool,
    pub(crate) ui_font: UiFont,
    pub(crate) ui_font_weight: FontWeight,
    pub(crate) ui_font_size: f32,
    pub(crate) fleet_view: FleetView,
    pub(crate) terminal_theme: TerminalThemeId,
    pub(crate) terminal_font: TerminalFont,
    pub(crate) terminal_font_weight: FontWeight,
    pub(crate) terminal_font_size: f32,
    pub(crate) terminal_line_height: f32,
    pub(crate) windows_shell_backend: WindowsShellBackend,
    pub(crate) wsl_distribution: String,
    pub(crate) default_agent: Option<Agent>,
    pub(crate) codex_command: String,
    pub(crate) claude_command: String,
    pub(crate) pi_command: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            show_status_bar: false,
            ui_font: UiFont::SystemSans,
            ui_font_weight: FontWeight::Normal,
            ui_font_size: 16.0,
            fleet_view: FleetView::Tabs,
            terminal_theme: TerminalThemeId::MuxtrixDark,
            terminal_font: TerminalFont::SystemMonospace,
            terminal_font_weight: FontWeight::Normal,
            terminal_font_size: 14.0,
            terminal_line_height: 1.15,
            windows_shell_backend: WindowsShellBackend::Native,
            wsl_distribution: String::new(),
            default_agent: None,
            codex_command: "codex".into(),
            claude_command: "claude".into(),
            pi_command: "omp".into(),
        }
    }
}

impl AppSettings {
    pub(crate) fn load() -> (Self, Option<String>) {
        // Unit tests describe the defaults, so they must not inherit whatever
        // the developer running them happens to have configured — a saved
        // `fleet_view` of Repos silently failed three fleet assertions that
        // pass everywhere else. An explicit override still wins, which is what
        // the e2e harness and the round-trip test rely on.
        #[cfg(test)]
        if std::env::var_os(CONFIG_OVERRIDE).is_none() {
            return (Self::default(), None);
        }
        let path = config_path();
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Self>(&bytes) {
                Ok(settings) => (settings.sanitized(), None),
                Err(error) => (
                    Self::default(),
                    Some(format!("Could not read {}: {error}", path.display())),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(error) => (
                Self::default(),
                Some(format!("Could not read {}: {error}", path.display())),
            ),
        }
    }

    pub(crate) fn save(&self) -> Result<PathBuf, String> {
        let path = config_path();
        save_to(&path, &self.clone().sanitized())?;
        Ok(path)
    }

    /// Advance width of one terminal cell, measured from the configured face.
    ///
    /// The grid positions every run at a multiple of this value, so an assumed
    /// ratio drifts against the shaped text and clips the tail of each run.
    pub(crate) fn terminal_cell_width(&self) -> f32 {
        (self.terminal_font_pixels() * self.terminal_advance_ratio() * 100.0).round() / 100.0
    }

    /// Measured advance ratio of the configured face, or the assumed ratio when
    /// the face cannot be resolved.
    pub(crate) fn terminal_advance_ratio(&self) -> f32 {
        metrics::advance_ratio(
            self.terminal_font.family_name(),
            self.terminal_font_weight.numeric(),
        )
        .unwrap_or(metrics::FALLBACK_ADVANCE_RATIO)
    }

    pub(crate) fn terminal_cell_height(&self) -> f32 {
        (self.terminal_font_pixels() * self.terminal_line_height * 100.0).round() / 100.0
    }

    pub(crate) fn terminal_font_pixels(&self) -> f32 {
        (self.terminal_font_size * POINTS_TO_PIXELS * 100.0).round() / 100.0
    }

    pub(crate) fn ui_font(&self) -> Font {
        font_with_style(
            self.ui_font.iced(),
            self.ui_font_weight.iced(),
            font::Style::Normal,
        )
    }

    /// Characters that occupy the space `base_characters` occupy at the
    /// reference type size.
    ///
    /// Secondary rail copy that is not bound by measured layout still uses
    /// character budgets, so those budgets have to shrink as type grows.
    /// Without this a larger type size would demand a wider rail instead of
    /// truncating sooner, which is what makes an entry taller than its fixed
    /// anatomy allows.
    pub(crate) fn ui_char_budget(&self, base_characters: usize) -> usize {
        let scaled = base_characters as f32 * UI_TYPE_SCALE_REFERENCE / self.ui_font_size;
        (scaled.round().max(1.0) as usize).max(1)
    }

    pub(crate) fn ui_pixels(&self, base_points: f32) -> f32 {
        let relative_size = self.ui_font_size / UI_TYPE_SCALE_REFERENCE;
        (base_points * relative_size * POINTS_TO_PIXELS * 100.0).round() / 100.0
    }

    fn sanitized(mut self) -> Self {
        self.ui_font_size = self.ui_font_size.clamp(12.0, 20.0);
        self.terminal_font_size = self.terminal_font_size.clamp(10.0, 28.0);
        self.terminal_line_height = self.terminal_line_height.clamp(1.0, 1.6);
        if self.codex_command.trim().is_empty() {
            self.codex_command = "codex".into();
        }
        if self.claude_command.trim().is_empty() {
            self.claude_command = "claude".into();
        }
        if self.pi_command.trim().is_empty() {
            self.pi_command = "omp".into();
        }
        self
    }
}

fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os(CONFIG_OVERRIDE) {
        return PathBuf::from(path);
    }

    #[cfg(target_os = "windows")]
    if let Some(base) = std::env::var_os("APPDATA") {
        return PathBuf::from(base).join("Muxtrix").join("settings.json");
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Muxtrix")
            .join("settings.json");
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(base).join("muxtrix").join("settings.json");
    }

    std::env::var_os("HOME").map_or_else(
        || PathBuf::from("muxtrix-settings.json"),
        |home| {
            PathBuf::from(home)
                .join(".config")
                .join("muxtrix")
                .join("settings.json")
        },
    )
}

fn save_to(path: &Path, settings: &AppSettings) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("settings path {} has no parent", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    if path.exists() {
        std::fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temporary, path).map_err(|error| error.to_string())
}

pub(crate) fn font_with_style(base: Font, weight: font::Weight, style: font::Style) -> Font {
    Font {
        weight,
        style,
        ..base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emphasis_weights_snap_into_the_family_the_user_actually_has() {
        // FreeSans, the sans this project's Linux CI resolves to, ships exactly
        // these two. Medium sits halfway between them.
        let two_face = [FontWeight::Normal, FontWeight::Semibold];
        assert_eq!(
            nearest_weight(&two_face, FontWeight::Medium),
            FontWeight::Normal,
            "a tie must not collapse Medium onto the weight that marks focus"
        );
        assert_eq!(
            nearest_weight(&two_face, FontWeight::Semibold),
            FontWeight::Semibold
        );
        assert_eq!(
            nearest_weight(&two_face, FontWeight::Bold),
            FontWeight::Semibold,
            "a heavier request than the family ships snaps to its heaviest face"
        );
        assert_eq!(
            nearest_weight(&two_face, FontWeight::Thin),
            FontWeight::Normal
        );

        // A family with the requested face is left alone, and an unknown family
        // keeps the caller's intent rather than snapping to nothing.
        assert_eq!(
            nearest_weight(&FontWeight::ALL, FontWeight::Medium),
            FontWeight::Medium
        );
        assert_eq!(nearest_weight(&[], FontWeight::Medium), FontWeight::Medium);
    }

    #[test]
    fn settings_round_trip_and_clamp_unsafe_metrics() {
        let directory = std::env::temp_dir().join(format!(
            "muxtrix-settings-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("settings.json");
        let settings = AppSettings {
            appearance: Appearance::Dark,
            show_status_bar: true,
            ui_font: UiFont::named("Inter"),
            ui_font_weight: FontWeight::Medium,
            ui_font_size: 99.0,
            fleet_view: FleetView::Agents,
            terminal_theme: TerminalThemeId::Dracula,
            terminal_font: TerminalFont::named("Cascadia Mono"),
            terminal_font_weight: FontWeight::Semibold,
            terminal_font_size: 2.0,
            terminal_line_height: 4.0,
            windows_shell_backend: WindowsShellBackend::Wsl,
            wsl_distribution: "Ubuntu-24.04".into(),
            default_agent: Some(Agent::Claude),
            codex_command: String::new(),
            claude_command: "claude --model opus".into(),
            pi_command: String::new(),
        }
        .sanitized();

        save_to(&path, &settings).expect("settings should save");
        save_to(&path, &settings).expect("settings should overwrite");
        let restored: AppSettings = serde_json::from_slice(
            &std::fs::read(&path).expect("saved settings should be readable"),
        )
        .expect("settings should deserialize");

        assert_eq!(restored.ui_font_size, 20.0);
        assert_eq!(restored.ui_font, UiFont::named("Inter"));
        assert_eq!(restored.ui_font_weight, FontWeight::Medium);
        assert_eq!(restored.fleet_view, FleetView::Agents);
        assert_eq!(restored.terminal_theme, TerminalThemeId::Dracula);
        assert_eq!(restored.appearance, Appearance::Dark);
        assert!(restored.show_status_bar);
        assert_eq!(restored.terminal_font_size, 10.0);
        assert_eq!(restored.terminal_line_height, 1.6);
        assert_eq!(restored.terminal_font, TerminalFont::named("Cascadia Mono"));
        assert_eq!(restored.terminal_font_weight, FontWeight::Semibold);
        assert_eq!(restored.windows_shell_backend, WindowsShellBackend::Wsl);
        assert_eq!(restored.default_agent, Some(Agent::Claude));
        assert_eq!(
            restored.terminal_font_pixels(),
            if cfg!(target_os = "macos") {
                10.0
            } else {
                13.33
            }
        );
        assert_eq!(restored.codex_command, "codex");
        assert_eq!(restored.claude_command, "claude --model opus");
        assert_eq!(restored.pi_command, "omp");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(directory);
    }

    #[test]
    fn repos_fleet_view_round_trips_with_its_stable_settings_name() {
        let encoded = serde_json::to_string(&FleetView::Repos).expect("view should serialize");
        assert_eq!(encoded, "\"repos\"");
        assert_eq!(
            serde_json::from_str::<FleetView>(&encoded).expect("view should deserialize"),
            FleetView::Repos
        );
        assert_eq!(FleetView::Repos.to_string(), "Repos");
    }

    #[test]
    fn font_weights_follow_the_faces_available_for_a_family() {
        assert_eq!(
            font_weight_choices([390, 610, 700, 610]),
            vec![FontWeight::Normal, FontWeight::Semibold, FontWeight::Bold,]
        );
    }

    #[test]
    fn interface_font_applies_the_selected_family_and_weight() {
        let settings = AppSettings {
            ui_font: UiFont::named("Inter"),
            ui_font_weight: FontWeight::Semibold,
            ..AppSettings::default()
        };

        assert_eq!(settings.ui_font().family, font::Family::Name("Inter"));
        assert_eq!(settings.ui_font().weight, font::Weight::Semibold);
    }

    #[test]
    fn system_monospace_shapes_with_the_resolved_platform_family() {
        let font =
            TerminalFont::SystemMonospace.iced_with_system_family(Some("Resolved System Mono"));

        assert_eq!(font.family, font::Family::Name("Resolved System Mono"));
    }

    #[test]
    fn old_or_partial_settings_receive_defaults() {
        let restored: AppSettings =
            serde_json::from_str(r#"{"terminal_font_size":18.0,"fleet_scope":"all-workspaces"}"#)
                .expect("partial settings with the retired fleet scope should deserialize");
        assert_eq!(restored.terminal_font_size, 18.0);
        assert_eq!(restored.terminal_font, TerminalFont::SystemMonospace);
        assert_eq!(restored.ui_font_size, 16.0);
        assert_eq!(restored.ui_font, UiFont::SystemSans);
        assert_eq!(restored.ui_font_weight, FontWeight::Normal);
        assert_eq!(restored.terminal_theme, TerminalThemeId::MuxtrixDark);
        assert_eq!(restored.appearance, Appearance::System);
        assert!(!restored.show_status_bar);
        assert_eq!(restored.default_agent, None);
        assert_eq!(restored.codex_command, "codex");
        assert_eq!(restored.pi_command, "omp");
    }

    #[test]
    fn font_settings_use_point_sizing_for_consistent_cross_platform_metrics() {
        let settings = AppSettings::default();
        // No face database is published in unit tests, so cell width follows the
        // assumed ratio. See `cell_width_tracks_the_measured_advance_ratio`.
        assert_eq!(
            settings.terminal_advance_ratio(),
            metrics::FALLBACK_ADVANCE_RATIO
        );
        if cfg!(target_os = "macos") {
            assert_eq!(settings.terminal_font_pixels(), 14.0);
            assert_eq!(settings.terminal_cell_width(), 8.4);
            assert_eq!(settings.terminal_cell_height(), 16.1);
            assert_eq!(settings.ui_pixels(12.0), 13.71);
        } else {
            assert_eq!(settings.terminal_font_pixels(), 18.67);
            assert_eq!(settings.terminal_cell_width(), 11.2);
            assert_eq!(settings.terminal_cell_height(), 21.47);
            assert_eq!(settings.ui_pixels(12.0), 18.29);
        }
    }

    #[test]
    fn interface_type_defaults_to_sixteen_points_and_scales_together() {
        let settings = AppSettings::default();
        assert_eq!(settings.ui_font_size, 16.0);
        // Secondary copy is expressed against the reference size, so it moves
        // with the setting rather than staying at its 14 pt value.
        let smaller = AppSettings {
            ui_font_size: UI_TYPE_SCALE_REFERENCE,
            ..AppSettings::default()
        };
        assert!(
            settings.ui_pixels(9.0) > smaller.ui_pixels(9.0),
            "secondary copy should grow with the interface size"
        );
        assert!(
            (settings.ui_pixels(9.0) / smaller.ui_pixels(9.0)
                - settings.ui_pixels(11.0) / smaller.ui_pixels(11.0))
            .abs()
                < 1e-3,
            "every size should scale by the same factor"
        );
    }

    #[test]
    fn character_budgets_shrink_as_interface_type_grows() {
        // Fleet copy truncates by character budget, so a larger type size has to
        // truncate sooner rather than ask the rail for more width.
        let reference = AppSettings {
            ui_font_size: UI_TYPE_SCALE_REFERENCE,
            ..AppSettings::default()
        };
        assert_eq!(reference.ui_char_budget(24), 24);
        assert_eq!(reference.ui_char_budget(44), 44);

        let larger = AppSettings::default();
        assert!(larger.ui_font_size > reference.ui_font_size);
        assert!(
            larger.ui_char_budget(44) < reference.ui_char_budget(44),
            "a larger interface size should truncate sooner"
        );
        // The budget tracks the inverse of the type scale.
        assert_eq!(larger.ui_char_budget(24), 21);
        assert_eq!(larger.ui_char_budget(44), 39);
    }

    #[test]
    fn a_character_budget_never_reaches_zero() {
        let largest = AppSettings {
            ui_font_size: 20.0,
            ..AppSettings::default()
        };
        assert!(largest.ui_char_budget(1) >= 1);
        assert!(largest.ui_char_budget(0) >= 1);
    }

    #[test]
    fn cell_width_tracks_the_measured_advance_ratio() {
        // Cell width must stay derived from the face being rendered. A constant
        // ratio drifts against the shaped text and clips the tail of every run.
        let settings = AppSettings::default();
        let expected =
            (settings.terminal_font_pixels() * settings.terminal_advance_ratio() * 100.0).round()
                / 100.0;
        assert_eq!(settings.terminal_cell_width(), expected);

        let larger = AppSettings {
            terminal_font_size: 20.0,
            ..AppSettings::default()
        };
        assert!(larger.terminal_cell_width() > settings.terminal_cell_width());
    }

    #[test]
    fn enumerating_fonts_does_not_publish_process_wide_metrics() {
        // Discovery is read-only; only an explicit `install_metrics` may change
        // what the rest of the process measures.
        let before = AppSettings::default().terminal_advance_ratio();
        let _ = InstalledFontCatalog::discover();
        assert_eq!(AppSettings::default().terminal_advance_ratio(), before);
    }

    #[test]
    fn font_picker_never_offers_unavailable_named_families() {
        let available = InstalledFontCatalog::discover().terminal_fonts();
        assert_eq!(available.first(), Some(&TerminalFont::SystemMonospace));
        assert!(
            available
                .iter()
                .skip(1)
                .all(|font| font.family_name().is_some())
        );
    }

    #[test]
    fn font_picker_includes_every_unique_installed_monospace_family() {
        let available = terminal_font_choices([
            "Cascadia Mono",
            "Fira Code",
            "cascadia mono",
            "  JetBrains Mono  ",
            "",
        ]);
        assert_eq!(
            available,
            vec![
                TerminalFont::SystemMonospace,
                TerminalFont::named("Cascadia Mono"),
                TerminalFont::named("Fira Code"),
                TerminalFont::named("JetBrains Mono"),
            ]
        );
    }

    #[test]
    fn font_settings_accept_dynamic_and_legacy_family_names() {
        let dynamic: TerminalFont =
            serde_json::from_str(r#""Iosevka Term""#).expect("dynamic family should deserialize");
        let legacy: TerminalFont =
            serde_json::from_str(r#""jet-brains-mono""#).expect("legacy family should deserialize");
        assert_eq!(dynamic, TerminalFont::named("Iosevka Term"));
        assert_eq!(legacy, TerminalFont::named("JetBrains Mono"));
        assert_eq!(
            serde_json::to_string(&dynamic).expect("dynamic family should serialize"),
            r#""Iosevka Term""#
        );
    }
}
