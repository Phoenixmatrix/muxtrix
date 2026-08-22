//! Design tokens: the single source of truth for every chrome surface colour.
//!
//! Terminal palettes live in [`crate::themes`]; this module covers the app
//! shell only. Both light and dark ramps are defined here so the appearance
//! switch is a pure function of [`Appearance`].

use iced::Color;

use crate::settings::Appearance;

#[derive(Debug, Clone, Copy)]
pub(crate) struct DesignTokens {
    pub(crate) app: Color,
    pub(crate) rail: Color,
    pub(crate) panel: Color,
    pub(crate) panel_raised: Color,
    /// Floating surfaces — menus, palette, dialogs, tooltips — sit above the
    /// terminal, so this is the lightest step of the dark ramp.
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
            // The "Muxtrix Polished" world: chrome sits on a slate rail,
            // terminal panes are darker cards floating on the app field, and
            // hairlines are translucent white so they read on any surface.
            Appearance::System | Appearance::Dark => Self {
                app: Color::from_rgb8(11, 14, 20),
                rail: Color::from_rgb8(18, 22, 31),
                panel: Color::from_rgb8(12, 15, 21),
                panel_raised: Color::from_rgb8(20, 26, 38),
                overlay: Color::from_rgb8(27, 32, 41),
                line: Color::from_rgba8(255, 255, 255, 0.06),
                line_strong: Color::from_rgba8(255, 255, 255, 0.12),
                scrim: Color::from_rgba8(4, 5, 10, 0.72),
                text: Color::from_rgb8(232, 236, 244),
                muted: Color::from_rgb8(152, 161, 184),
                faint: Color::from_rgb8(132, 142, 164),
                accent: Color::from_rgb8(92, 157, 255),
                success: Color::from_rgb8(85, 199, 126),
                warning: Color::from_rgb8(242, 177, 78),
                danger: Color::from_rgb8(240, 122, 110),
                github_open: Color::from_rgb8(63, 185, 80),
                github_merged: Color::from_rgb8(163, 113, 247),
            },
        }
    }
}
