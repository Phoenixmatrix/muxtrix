//! Design tokens: the single source of truth for every chrome surface colour.
//!
//! Terminal palettes live in [`crate::themes`]; this module covers the app
//! shell only. Both light and dark ramps are defined here so the appearance
//! switch is a pure function of [`Appearance`].

use crate::settings::Appearance;

/// A straight-alpha sRGB colour, the shape every chrome token is written in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Color {
    pub(crate) r: f32,
    pub(crate) g: f32,
    pub(crate) b: f32,
    pub(crate) a: f32,
}

impl Color {
    pub(crate) const TRANSPARENT: Self = Self::from_rgba(0.0, 0.0, 0.0, 0.0);
    pub(crate) const WHITE: Self = Self::from_rgb(1.0, 1.0, 1.0);

    pub(crate) const fn from_rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub(crate) const fn from_rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub(crate) fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::from_rgba8(r, g, b, 1.0)
    }

    pub(crate) fn from_rgba8(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DesignTokens {
    pub(crate) app: Color,
    pub(crate) rail: Color,
    pub(crate) panel: Color,
    pub(crate) panel_raised: Color,
    /// Floating surfaces — menus, palette, dialogs, tooltips — reuse the
    /// chrome surface and gain depth from their strong border and shadow.
    pub(crate) overlay: Color,
    pub(crate) line: Color,
    pub(crate) line_strong: Color,
    /// Backdrop dim behind modal dialogs; the only translucent surface
    /// token, appearance-aware where the old literal was not.
    pub(crate) scrim: Color,
    pub(crate) text: Color,
    pub(crate) muted: Color,
    pub(crate) faint: Color,
    pub(crate) accent: Color,
    pub(crate) success: Color,
    pub(crate) warning: Color,
    pub(crate) danger: Color,
    pub(crate) github_open: Color,
    pub(crate) github_merged: Color,
}

impl DesignTokens {
    pub(crate) fn for_appearance(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Light => Self {
                app: Color::from_rgb8(241, 243, 246),
                rail: Color::from_rgb8(249, 250, 252),
                panel: Color::WHITE,
                panel_raised: Color::from_rgb8(233, 236, 241),
                overlay: Color::WHITE,
                line: Color::from_rgb8(205, 210, 219),
                line_strong: Color::from_rgb8(162, 170, 184),
                scrim: Color::from_rgba8(24, 28, 38, 0.42),
                text: Color::from_rgb8(30, 34, 42),
                muted: Color::from_rgb8(91, 99, 113),
                faint: Color::from_rgb8(126, 134, 148),
                accent: Color::from_rgb8(27, 111, 214),
                success: Color::from_rgb8(42, 145, 78),
                warning: Color::from_rgb8(196, 126, 0),
                danger: Color::from_rgb8(194, 54, 59),
                github_open: Color::from_rgb8(31, 136, 61),
                github_merged: Color::from_rgb8(130, 80, 223),
            },
            // Warm graphite chrome surrounds a near-black work surface, with
            // restrained warm-neutral copy.
            Appearance::System | Appearance::Dark => Self {
                app: Color::from_rgb8(13, 16, 22),
                rail: Color::from_rgb8(31, 33, 39),
                panel: Color::from_rgb8(13, 16, 22),
                panel_raised: Color::from_rgb8(45, 47, 52),
                overlay: Color::from_rgb8(31, 33, 39),
                line: Color::from_rgb8(45, 47, 52),
                line_strong: Color::from_rgb8(63, 64, 67),
                scrim: Color::from_rgba8(4, 5, 10, 0.72),
                text: Color::from_rgb8(191, 189, 182),
                muted: Color::from_rgb8(161, 159, 153),
                faint: Color::from_rgb8(152, 150, 145),
                accent: Color::from_rgb8(90, 193, 254),
                success: Color::from_rgb8(170, 216, 76),
                warning: Color::from_rgb8(254, 180, 84),
                danger: Color::from_rgb8(239, 113, 119),
                github_open: Color::from_rgb8(170, 216, 76),
                github_merged: Color::from_rgb8(210, 166, 254),
            },
        }
    }
}
